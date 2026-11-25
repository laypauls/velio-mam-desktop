//! 워커 한 세대(incarnation)와의 연결 — 읽기·쓰기 스레드, 호스트와 공유하는 원자 상태.
//! 규칙(계약 §5.4): 파이프 쓰기는 전용 스레드(유한 큐)에서만, UI 스레드의 `release_slots` 는 `try_send` 로 버린다.
//! 프레임 알림은 읽기 스레드가 감독자를 거치지 않고 곧바로 표면으로 보낸다 (세대·epoch 가 현재일 때만).

use std::io::{BufReader, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use super::proc::{WorkerConn, WorkerProcess};
use super::surface::FrameSurface;
use crate::protocol::wire::MAX_LINE_BYTES;
use crate::protocol::{
    Command, FrameMsgOut, Message, Request, SlotRef, decode_line, encode_line, read_bounded_line,
};

/// 쓰기 큐 길이 — 가득 차면 워커가 멈춘 것이다 (감독자의 침묵 감시가 처리한다)
const WRITE_QUEUE: usize = 256;

/// 감독자 스레드로 들어오는 모든 입력
pub(crate) enum Inbox {
    Req(super::core::Req),
    /// `msg == None` = 이벤트 파이프 종료(워커 종료·크래시)
    Worker {
        inc: u64,
        msg: Option<Message>,
    },
    /// 기다리던 프레임이 도착했다 (`want_frame` 가 켜져 있을 때 한 번)
    FrameSeen,
}

/// 감독자 · 읽기 스레드 · UI 스레드가 함께 보는 상태
pub(crate) struct Shared {
    /// 현재 세대 번호 (0 = 없음)
    pub inc: AtomicU64,
    /// 현재 프레임을 받아들이는 epoch (0 = 없음)
    pub epoch: AtomicU64,
    /// 현재 세대의 쓰기 큐 — `release_slots` 가 쓴다
    pub writer: Mutex<Option<SyncSender<String>>>,
    pub release_dropped: AtomicU64,
    /// 게시한 프레임 수 (현재 세대·epoch 의 것만)
    pub frames: AtomicU64,
    pub want_frame: AtomicBool,
    /// `close_nowait` 가 올리는 세대 — 대기 중인 열기를 취소한다
    pub close_gen: AtomicU64,
    base: Instant,
    last_msg_ms: AtomicU64,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inc: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            writer: Mutex::new(None),
            release_dropped: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            want_frame: AtomicBool::new(false),
            close_gen: AtomicU64::new(0),
            base: Instant::now(),
            last_msg_ms: AtomicU64::new(0),
        })
    }

    pub fn touch(&self) {
        self.last_msg_ms
            .store(self.base.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    pub fn silence(&self) -> std::time::Duration {
        let now = self.base.elapsed().as_millis() as u64;
        std::time::Duration::from_millis(
            now.saturating_sub(self.last_msg_ms.load(Ordering::Relaxed)),
        )
    }

    fn writer_lock(&self) -> std::sync::MutexGuard<'_, Option<SyncSender<String>>> {
        self.writer.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_writer(&self, w: Option<SyncSender<String>>) {
        *self.writer_lock() = w;
    }

    /// UI 스레드에서 부른다 — 절대 막지 않는다. epoch 가 현재가 아니거나 큐가 가득 차면 버린다
    pub fn release(&self, epoch: u64, slots: &[(u32, u64)]) {
        if epoch == 0 || epoch != self.epoch.load(Ordering::Acquire) || slots.is_empty() {
            return;
        }
        let cmd = Command::Release {
            epoch,
            slots: slots
                .iter()
                .map(|&(slot, seq)| SlotRef { slot, seq })
                .collect(),
        };
        let Ok(line) = encode_line(&Request {
            id: 0,
            command: cmd,
        }) else {
            return;
        };
        let guard = self.writer_lock();
        if let Some(tx) = guard.as_ref()
            && let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = tx.try_send(line)
        {
            self.release_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// 감독자가 쥐는 워커 한 세대
pub(crate) struct Inc {
    pub id: u64,
    pub process: Box<dyn WorkerProcess>,
    tx: SyncSender<String>,
    next_cmd: u64,
    threads: Vec<JoinHandle<()>>,
}

impl Inc {
    /// 명령을 쓰기 큐에 넣는다. `reply == false` 면 id 0 (응답 없음). 반환: 쓴 id. 큐가 가득 차면 None
    pub fn send(&mut self, command: Command, reply: bool) -> Option<u64> {
        let id = if reply {
            self.next_cmd += 1;
            self.next_cmd
        } else {
            0
        };
        let line = encode_line(&Request { id, command }).ok()?;
        self.tx.try_send(line).ok().map(|_| id)
    }

    pub fn writer(&self) -> SyncSender<String> {
        self.tx.clone()
    }

    /// 프로세스를 이 핸들로 종료하고 스레드를 정리한다
    pub fn kill(mut self) {
        self.process.kill();
        drop(self.tx);
        // 스레드는 파이프 EOF 로 스스로 끝난다 — join 으로 감독자를 막지 않는다
        self.threads.clear();
    }
}

/// 연결된 `conn` 에서 읽기·쓰기 스레드를 띄운다. 첫 `Ready` 는 `ready` 채널로만 가고, 그 뒤 메시지는 감독자 inbox 로 간다.
/// 반환: (세대, 첫 메시지 수신기). 첫 메시지가 `Ready` 가 아니면 호출자가 실패로 본다
pub(crate) fn start(
    id: u64,
    conn: WorkerConn,
    shared: &Arc<Shared>,
    surface: &Arc<dyn FrameSurface>,
    inbox: Sender<Inbox>,
) -> (Inc, Receiver<Option<Message>>) {
    let (tx, wrx) = sync_channel::<String>(WRITE_QUEUE);
    let (ready_tx, ready_rx) = channel::<Option<Message>>();
    let WorkerConn { process, cmd, evt } = conn;

    let mut cmd = cmd;
    let writer = std::thread::spawn(move || {
        for line in wrx {
            if cmd
                .write_all(line.as_bytes())
                .and_then(|_| cmd.flush())
                .is_err()
            {
                break;
            }
        }
    });

    let (sh, sf) = (Arc::clone(shared), Arc::clone(surface));
    let reader = std::thread::spawn(move || {
        let mut r = BufReader::new(evt);
        let mut first = true;
        let mut line = String::new();
        loop {
            line.clear();
            match read_bounded_line(&mut r, &mut line, MAX_LINE_BYTES) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let Ok(msg) = decode_line::<Message>(&line) else {
                continue;
            };
            if inc_is_current(&sh, id) {
                sh.touch();
            }
            if first {
                first = false;
                if ready_tx.send(Some(msg)).is_err() {
                    break;
                }
                continue;
            }
            match msg {
                Message::FrameReady {
                    epoch,
                    slot,
                    seq,
                    w,
                    h,
                    stride,
                    render_done_ms,
                } => {
                    // 빠른 경로: 현재 세대·epoch 의 프레임만 UI 로 보낸다 (오래된 세대·epoch 는 버린다 — JS 가 받지 않았으니 슬롯은 워커가 Attach 로 정리)
                    if sh.inc.load(Ordering::Acquire) == id
                        && sh.epoch.load(Ordering::Acquire) == epoch
                    {
                        sf.post_frame(&FrameMsgOut::new(
                            epoch,
                            slot,
                            seq,
                            w,
                            h,
                            stride,
                            render_done_ms,
                        ));
                        sh.frames.fetch_add(1, Ordering::AcqRel);
                        if sh.want_frame.swap(false, Ordering::AcqRel) {
                            let _ = inbox.send(Inbox::FrameSeen);
                        }
                    }
                }
                // 하트비트는 침묵 감시용 touch 로 충분하다
                Message::Stats { .. } => {}
                m => {
                    if inbox
                        .send(Inbox::Worker {
                            inc: id,
                            msg: Some(m),
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
        // 첫 메시지 전에 끊겼다면 start 대기자에게, 아니면 감독자에게 알린다
        if first {
            let _ = ready_tx.send(None);
        } else {
            let _ = inbox.send(Inbox::Worker { inc: id, msg: None });
        }
    });

    (
        Inc {
            id,
            process,
            tx,
            next_cmd: 0,
            threads: vec![writer, reader],
        },
        ready_rx,
    )
}

/// 읽기 스레드는 자기 세대가 현재일 때만 침묵 시계를 갱신한다 (이전 세대의 늦은 메시지가 새 워커의 침묵을 가리지 않게). 호출자가 `start` 전에 `shared.inc` 를 세대로 맞춘다
fn inc_is_current(sh: &Shared, id: u64) -> bool {
    sh.inc.load(Ordering::Acquire) == id
}
