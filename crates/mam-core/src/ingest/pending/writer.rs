//! 기록기 스레드 — `ChangeSink` 가 mpsc 로 보낸 보고를 모아 0.25초마다 1 트랜잭션으로 `watch_pending` 에 쓴다.
//!
//! 같은 경로의 반복 보고는 메모리에서 먼저 합친다(1000회 -> 1건). 쓰기는 `Priority::Low` 라 사용자 편집(High)을 막지 않는다.
//! 종료하면 mpsc 에 남은 것을 **별도 기한**(`PendingConfig::drain`)까지 마저 쓴다 — 종료 신호로 이미 취소된 상태에서도 쓴다.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use super::store::{Incoming, PendingKind};
use crate::ingest::Ingest;
use crate::watch::WatchMsg;

/// 쓰기가 계속 실패할 때 메모리에 쌓아 두는 최대 보고 수 — 넘기면 버리고 폴더 재스캔으로 만회한다
const MAX_BUFFERED: usize = 100_000;

type Buffer = HashMap<(i64, String), Incoming>;

impl Ingest {
    pub(in crate::ingest) fn writer_loop(&self, rx: &Receiver<WatchMsg>) {
        let every = self.config.pending.write_every;
        let mut buf = Buffer::new();
        let mut last_flush = Instant::now();
        while !self.stop.is_set() {
            match rx.recv_timeout(every) {
                Ok(msg) => self.accept(msg, &mut buf),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if last_flush.elapsed() >= every {
                self.flush_buffer(&mut buf);
                last_flush = Instant::now();
            }
        }
        // 종료: 남은 보고를 기한 안에서 마저 쓴다 (종료 신호와 무관한 별도 기한)
        let deadline = Instant::now() + self.config.pending.drain;
        while Instant::now() < deadline {
            match rx.try_recv() {
                Ok(msg) => self.accept(msg, &mut buf),
                Err(_) => break,
            }
        }
        self.flush_buffer(&mut buf);
    }

    fn accept(&self, msg: WatchMsg, buf: &mut Buffer) {
        let (folder_id, p, kind) = match msg {
            WatchMsg::Path { folder_id, path } => (folder_id, path, PendingKind::Path),
            WatchMsg::Subtree { folder_id, dir } => (folder_id, dir, PendingKind::Subtree),
            // 놓쳤을 수 있다 — 재스캔 작업은 jobs 테이블에 남으므로 대기열을 거치지 않는다
            WatchMsg::Rescan { folder_id } => return self.schedule_rescan_quiet(folder_id),
        };
        // 원본 이동 중인 폴더의 이벤트는 기록하지 않는다 (반영 직전에도 한 번 더 거른다)
        if self.suppressed.covers(&p) {
            return;
        }
        if let Some(mut item) = Incoming::new(folder_id, &p, kind, self.source_of(folder_id)) {
            let key = (folder_id, item.path_key.clone());
            // 같은 경로를 Path 와 Subtree 로 보고했으면 하위 순회(Subtree)가 이긴다 (DB UPSERT 와 같은 규칙)
            if buf.get(&key).is_some_and(|old| old.kind == PendingKind::Subtree) {
                item.kind = PendingKind::Subtree;
            }
            buf.insert(key, item);
        }
    }

    fn flush_buffer(&self, buf: &mut Buffer) {
        if buf.is_empty() {
            return;
        }
        let items: Vec<Incoming> = buf.values().cloned().collect();
        match self.enqueue_incoming(items) {
            Ok(report) => {
                buf.clear();
                self.on_overflow(&report.overflowed);
            }
            Err(e) => {
                tracing::warn!(%e, pending = buf.len(), "cannot record watch changes; will retry");
                if buf.len() > MAX_BUFFERED {
                    let mut folders: Vec<i64> = buf.keys().map(|(f, _)| *f).collect();
                    folders.sort_unstable();
                    folders.dedup();
                    buf.clear();
                    for f in folders {
                        self.schedule_rescan_quiet(f);
                    }
                }
            }
        }
    }
}
