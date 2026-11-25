//! `polling` (실험) — 디렉터리 수정 시각으로 가지치기한 폴링. 이벤트가 오지 않는 NAS 등을 위한 감지기.
//!
//! 한 주기마다 알고 있는 디렉터리의 수정 시각만 조회하고, 바뀐 디렉터리만 목록을 읽어 이름 집합을 비교한다
//! (`snapshot.rs`). 추가·삭제·이름 변경은 `Path`, 새 하위 폴더는 `Subtree` 로 보고한다.
//!
//! **한계 (UI 설명에 적는다)**: 디렉터리 수정 시각은 파일 내용만 바뀐 경우에는 바뀌지 않는다 — 그 변경은 감지하지 못하고
//! 주기 재스캔이 잡는다. 시작 때 이미 있던 파일은 기준선으로만 삼고 보고하지 않는다(역시 재스캔의 몫).
//!
//! 응답 없는 볼륨: 네트워크·이동식 볼륨의 호출은 `io_guard` 제한 시간을 건다. 시간 초과·읽기 실패는 `Degraded` 로 알리고
//! 계속 시도하며, 회복하면 `Ok` 로 돌아오고 비어 있던 구간을 덮도록 `Rescan` 을 한 번 보낸다.
//! 자동 선택 순서에는 넣지 않는다 — NAS 실측 전이라 사용자가 고른 폴더에서만 동작한다.

mod fs;
mod snapshot;
mod stop;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::super::detector::{
    ChangeDetector, DetectError, DetectorOptions, OptionField, OptionKind, ProbeResult,
    WatchHandle, WatchTarget,
};
use super::super::sink::ChangeSink;
use crate::SharedClock;
use crate::scan::io_guard::{IoGuard, IoGuardConfig};
use crate::volume::VolumeKind;

pub use fs::{GuardedFs, Listing, PollError, PollFs, StdFs};
pub use snapshot::{Change, Snapshot, TickOutcome, TickStats};
pub use stop::StopSignal;

pub const OPT_INTERVAL: &str = "intervalSec";
/// 보수적인 기본값 — 로컬 NTFS 실측(`docs/requests/2026-11/…multi-scan-modules…` 0단계)과 NAS 미측정을 함께 고려했다
pub const DEFAULT_INTERVAL_SEC: i64 = 300;
pub const MIN_INTERVAL_SEC: i64 = 60;
pub const MAX_INTERVAL_SEC: i64 = 86_400;

pub const HEALTH_TIMEOUT: &str = "watch.health.poll_timeout";
pub const HEALTH_ERROR: &str = "watch.health.poll_error";

/// 한 주기에 보고할 수 있는 최대 변경 수 — 넘으면 나머지는 버리고 `Rescan` 한 번으로 바꾼다 (변경 대기열의 폴더당 상한과 같은 크기)
pub const MAX_CHANGES_PER_TICK: usize = 50_000;
/// 실패한 뒤 다시 시도하기까지의 최대 간격
const RETRY_MAX: Duration = Duration::from_secs(60);
/// `start` 가 첫 기준선을 기다리는 시간 — 작은 폴더는 시작이 끝났을 때 이미 무장되어 있게 한다 (큰 폴더는 백그라운드에서 계속)
const BASELINE_WAIT: Duration = Duration::from_millis(250);
/// 호출 하나의 제한 시간 (네트워크·이동식) — 중지 요청은 이 안에서도 `SLICE` 마다 확인한다
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
const SLICE: Duration = Duration::from_millis(250);

const FIELDS: &[OptionField] = &[OptionField {
    key: OPT_INTERVAL,
    kind: OptionKind::Integer,
    min: MIN_INTERVAL_SEC,
    max: MAX_INTERVAL_SEC,
    default: DEFAULT_INTERVAL_SEC,
}];

pub struct Polling {
    /// 시험용 — 없으면 실제 파일시스템
    fs: Option<Arc<dyn PollFs>>,
    /// 시험용 — 옵션 대신 쓰는 간격
    interval: Option<Duration>,
    io: IoGuardConfig,
    clock: SharedClock,
}

impl Default for Polling {
    fn default() -> Self {
        Self::new()
    }
}

impl Polling {
    pub fn new() -> Self {
        Self {
            fs: None,
            interval: None,
            io: IoGuardConfig {
                timeout: CALL_TIMEOUT,
                ..IoGuardConfig::default()
            },
            clock: Arc::new(mam_db::SystemClock),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(
        fs: Option<Arc<dyn PollFs>>,
        interval: Duration,
        io: IoGuardConfig,
    ) -> Self {
        Self {
            fs,
            interval: Some(interval),
            clock: Arc::new(mam_db::SystemClock),
            io,
        }
    }
}

/// 출하 설정이면 프로세스 공용 가드 하나(감지기를 다시 시작해도 막힌 검사를 이어받는다), 시험 설정이면 새 가드.
/// 폴더별 검사 스레드 상한은 `GuardedFs::lane`(폴더 ID)으로 센다.
fn guard_for(cfg: IoGuardConfig) -> Arc<IoGuard> {
    static SHARED: OnceLock<Arc<IoGuard>> = OnceLock::new();
    let shipped = Polling::new().io;
    if cfg == shipped {
        Arc::clone(SHARED.get_or_init(|| IoGuard::new(shipped)))
    } else {
        IoGuard::new(cfg)
    }
}

struct PollHandle(Arc<StopSignal>);

impl WatchHandle for PollHandle {}

impl Drop for PollHandle {
    fn drop(&mut self) {
        // 기다리지 않는다 — 스레드가 중지 신호를 보고 끝나면 `ReleaseToken` 이 풀려 해제가 관찰된다
        self.0.set();
    }
}

impl ChangeDetector for Polling {
    fn id(&self) -> &'static str {
        "polling"
    }

    fn experimental(&self) -> bool {
        true
    }

    /// 모든 볼륨 종류에서 쓸 수 있다 (자동 선택에는 들어가지 않는다).
    fn probe(&self, _target: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }

    fn option_fields(&self) -> &'static [OptionField] {
        FIELDS
    }

    fn start(
        &self,
        target: &WatchTarget,
        opts: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        let interval = self.interval.unwrap_or_else(|| {
            Duration::from_secs(opts.get_i64(OPT_INTERVAL).unwrap_or(DEFAULT_INTERVAL_SEC) as u64)
        });
        let stop = Arc::new(StopSignal::default());
        let base: Arc<dyn PollFs> = self.fs.clone().unwrap_or_else(|| Arc::new(StdFs));
        let fs: Arc<dyn PollFs> = match target.volume_kind {
            VolumeKind::Fixed => base,
            _ => Arc::new(GuardedFs {
                inner: base,
                guard: guard_for(self.io),
                lane: target.folder_id,
                timeout: self.io.timeout,
                slice: SLICE.min(self.io.timeout),
                stop: Arc::clone(&stop),
            }),
        };
        let ready = Arc::new(stop::Latch::default());
        let token = sink.release_token();
        let (root, thread_stop, thread_ready, clock) = (
            target.root.clone(),
            Arc::clone(&stop),
            Arc::clone(&ready),
            Arc::clone(&self.clock),
        );
        std::thread::Builder::new()
            .name("mam-poll".into())
            .spawn(move || {
                let _token = token;
                run(
                    &root,
                    &*fs,
                    &sink,
                    &*clock,
                    interval,
                    &thread_stop,
                    &thread_ready,
                );
            })
            .map_err(|e| DetectError::Start(e.to_string()))?;
        ready.wait(BASELINE_WAIT);
        Ok(Box::new(PollHandle(stop)))
    }
}

fn run(
    root: &Path,
    fs: &dyn PollFs,
    sink: &ChangeSink,
    clock: &dyn mam_db::Clock,
    interval: Duration,
    stop: &StopSignal,
    ready: &stop::Latch,
) {
    let mut poller = Poller::default();
    loop {
        poller.step(fs, root, sink, clock.now_ms(), &|| stop.is_set());
        ready.open();
        let wait = if poller.failing {
            interval.min(RETRY_MAX)
        } else {
            interval
        };
        if stop.is_set() || stop.wait(wait) {
            break;
        }
    }
}

/// 감지기 한 대의 상태 — 스냅숏(없으면 기준선 대기)과 "실패 중" 표시. 스레드와 무관해서 시험이 직접 `step` 을 부른다.
#[derive(Default)]
pub(crate) struct Poller {
    snap: Option<Snapshot>,
    pub(crate) failing: bool,
}

impl Poller {
    /// 한 걸음: 기준선이 없으면 조용히 만들고, 있으면 한 주기를 돌려 변경을 보고한다. 실패는 `Degraded`, 회복은 `Ok` + `Rescan` 한 번.
    pub(crate) fn step(
        &mut self,
        fs: &dyn PollFs,
        root: &Path,
        sink: &ChangeSink,
        now_ms: i64,
        stop: &dyn Fn() -> bool,
    ) {
        let (mut changes, failure) = match self.snap.as_mut() {
            None => match Snapshot::baseline(fs, root, now_ms, stop).0 {
                Ok(s) => {
                    self.snap = Some(s);
                    (Vec::new(), None)
                }
                Err(e) => (Vec::new(), Some(e)),
            },
            Some(s) => {
                let o = s.tick(fs, root, now_ms, stop);
                (o.changes, o.failure)
            }
        };
        let overflow = changes.len() > MAX_CHANGES_PER_TICK;
        changes.truncate(MAX_CHANGES_PER_TICK);
        let any = !changes.is_empty();
        for c in changes {
            match c {
                Change::Path(p) => sink.path(p),
                Change::Subtree(d) => sink.subtree(d),
            }
        }
        if any {
            sink.signal();
        }
        if overflow {
            sink.rescan();
        }
        match failure {
            Some(PollError::Stopped) => {}
            Some(e) => {
                tracing::warn!(?e, "polling tick failed; will retry");
                sink.degraded(if e == PollError::TimedOut {
                    HEALTH_TIMEOUT
                } else {
                    HEALTH_ERROR
                });
                self.failing = true;
            }
            None if self.failing => {
                self.failing = false;
                sink.recovered();
                // 응답 없던 동안의 변경을 놓쳤을 수 있다 — 재스캔이 맞춘다 (감지기 불변 조건 1)
                sink.rescan();
            }
            None => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Option<&Snapshot> {
        self.snap.as_ref()
    }
}
