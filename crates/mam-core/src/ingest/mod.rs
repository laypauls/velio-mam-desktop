//! 수집 서비스 파사드 (P2-2) — 관리 폴더, 스캔 작업, 감시, 주기 재스캔, 볼륨 연결 감지를 묶는다.
//! Tauri command 는 이 타입의 메서드만 부른다 (로직은 여기·하위 모듈에 있어 command 없이 시험할 수 있다).

mod api;
mod detection;
mod device;
mod eject;
mod known;
mod newvol;
pub(crate) mod pending;
mod purge_ops;
pub mod rescan_policy;
mod relocate;
mod remote_api;
mod removal;
mod stall;
mod stop;
mod summary;
mod supervisor;
mod suppress;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::SharedClock;
use crate::hash::ContentHasher;
use crate::identity::FileIdentityProvider;
use crate::jobs::{ExecutorRegistry, JobExecutor, JobKind, JobQueue, JobRunner, NewJob};
use crate::processing::{HashExecutor, MetaExecutor, ProcessingEnv, ThumbExecutor};
use crate::scan::{IngestError, IoGuard, IoGuardConfig, ScanEnv, ScanExecutor, ScanProgressCell, apply_path};
use crate::volume::VolumeProvider;
use crate::secret::{Dpapi, SecretBox};
use crate::watch::remote::hub::{HubDeps, HubRefs};
use crate::watch::remote::{RemoteHub, SshConnector};
use crate::watch::{DdpAudit, DetectorRegistry, FanotifyFeed, WatchManager, WatchMsg};
use mam_db::DbPool;

pub use eject::{EjectHook, EjectPrepared, EjectScope};
pub use known::KnownVolume;
pub use newvol::{PENDING_CHANGED_EVENT, PendingChangedPayload, PendingListener, PendingVolume};
pub use pending::{
    DbCommitter,
    ExclusiveOutcome, MAX_ATTEMPTS, PendingConfig, PendingKind, PendingPassReport, PendingRow, SCAN_RECENT_GUARD_MS,
    SettleGate,
};
pub use purge_ops::LegacyCleanupReport;
pub use remote_api::{RemoteSourceInput, RemoteTestResult};
pub use removal::{
    BackupHook, EmbeddingsChangedHook, FolderRemovalProgress, REMOVAL_PROGRESS_EVENT, RemovalListener, RemovalRun,
};
pub use relocate::RelocateOutcome;
pub use stall::{STALL_LOG_EVERY_MS, StallEvent};
pub use summary::{JobSummary, ScanProgressView};
pub use supervisor::SuperviseReport;
pub use suppress::WatchSuppression;

/// 외부 의존성 (시험에서는 가짜 볼륨·시계를 넣는다).
pub struct IngestDeps {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub volumes: Arc<dyn VolumeProvider + Send + Sync>,
    pub identity: Arc<dyn FileIdentityProvider + Send + Sync>,
    pub hasher: Arc<dyn ContentHasher + Send + Sync>,
}

#[derive(Debug, Clone)]
pub struct IngestConfig {
    /// 항상 스캔에서 제외할 절대 경로 (MAMData, 백업 폴더 — PL8)
    pub auto_excludes: Vec<PathBuf>,
    /// 감시 이벤트 디바운스 (D5: 2초) — 변경 대기열에서 첫 확인까지의 대기
    pub debounce_ms: i64,
    /// 변경 대기열·안정화 게이트 (표본 수·간격, 폴더당 상한, 스캔의 최근 수정 파일 창)
    pub pending: PendingConfig,
    /// 볼륨 연결 확인·주기 재스캔 점검 간격 (D6: 30초)
    pub supervise_every: Duration,
    /// 종류별 동시성 상한 (`JobKind::ALL` 순서). 스캔 칸은 전체 상한의 시작값이다 — 설정 `maxConcurrentScans` 가 덮어쓴다
    pub concurrency: [usize; JobKind::COUNT],
    /// 막힐 수 있는 볼륨의 파일 호출 제한 시간·볼륨별 검사 상한
    pub io_guard: IoGuardConfig,
    /// 변경 감지기 레지스트리 (시험에서는 가짜 감지기를 넣는다)
    pub detectors: Arc<DetectorRegistry>,
    /// 로컬 시간대 (재스캔 일정 정책의 "03:00" 해석용 — 시험에서는 고정 오프셋을 넣는다)
    pub local_zone: Arc<dyn rescan_policy::LocalZone>,
    /// 원격 스토리지 접속 허브 (`ddp_audit` 감지기가 쓴다). `Ingest::new` 가 DB·기록기를 꽂는다. 시험은 가짜 접속기로 만든 허브를 넣는다
    pub remote: Arc<RemoteHub>,
    /// 원격 접속 비밀 보관 (운영 = DPAPI, 시험 = 가짜)
    pub secrets: Arc<dyn SecretBox>,
}

impl Default for IngestConfig {
    fn default() -> Self {
        let remote = RemoteHub::new(Arc::new(SshConnector::default()));
        Self {
            auto_excludes: Vec::new(),
            debounce_ms: 2_000,
            pending: PendingConfig::default(),
            supervise_every: Duration::from_secs(30),
            concurrency: JobKind::ALL.map(JobKind::default_concurrency),
            io_guard: IoGuardConfig::default(),
            detectors: Arc::new(
                DetectorRegistry::platform_default_with(vec![Arc::new(DdpAudit::new(Arc::clone(&remote))), Arc::new(FanotifyFeed::new(Arc::clone(&remote)))])
                    .with_ref_resolver(Arc::new(HubRefs(Arc::clone(&remote)))),
            ),
            local_zone: Arc::new(rescan_policy::OsLocalZone),
            remote,
            secrets: Arc::new(Dpapi),
        }
    }
}

use stop::{StopFlag, Tick};

pub struct Ingest {
    env: Arc<ScanEnv>,
    jobs: Arc<JobQueue>,
    volumes: Arc<dyn VolumeProvider + Send + Sync>,
    config: IngestConfig,
    watchers: WatchManager,
    watch_rx: Mutex<Option<Receiver<WatchMsg>>>,
    registry: Mutex<ExecutorRegistry>,
    runner: Mutex<Option<JobRunner>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    stop: Arc<StopFlag>,
    totals: Mutex<Option<(std::time::Instant, u64, i64)>>,
    watch_failed: Mutex<std::collections::HashSet<i64>>,
    /// 감시 시작·교체를 직렬화한다 (폴더 이전 중 점검 스레드가 옛 경로 감시를 되살리지 못하게)
    watch_ctl: Mutex<()>,
    /// 처리 작업 보충을 이미 한 번 했는가 (첫 점검에서만, 이후는 볼륨 연결 때)
    backfilled: std::sync::atomic::AtomicBool,
    /// 점검이 재스캔 정책 판정에 한 번이라도 도달했는가 (`on_start` 첫 점검 판정, 프로세스당 1회)
    on_start_pass: std::sync::atomic::AtomicBool,
    /// 해석할 수 없는 재스캔 정책 경고를 이미 낸 폴더 (폴더·프로세스당 1회)
    policy_warned: Mutex<std::collections::HashSet<i64>>,
    /// 원본 이동 중 감시 이벤트를 버릴 폴더 (P2-9)
    suppressed: Arc<suppress::SuppressSet>,
    /// 변경 대기열 런타임 상태 (pending/)
    pending: pending::PendingState,
    /// 스캔 멈춤 로그 상태 (stall.rs)
    stall_log: stall::StallLog,
    /// 새 디스크 감지 상태·대기 목록 (newvol.rs)
    newvol: newvol::NewVolumes,
    /// 장치 변경 감지기 연결 (device.rs)
    device: device::DeviceHook,
    /// 폴더 제거·연결 데이터 정리 상태 (removal.rs, purge_ops.rs)
    removal: removal::Removals,
    /// USB 꺼내기 준비 상태 (eject.rs)
    eject: eject::Ejects,
    /// 자기 자신 (백그라운드 스레드가 쥘 수 있게)
    me: std::sync::Weak<Ingest>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Ingest {
    /// 스레드는 만들지 않는다 — [`Ingest::start`] 를 호출해야 작업 실행·감시·점검이 시작된다.
    pub fn new(deps: IngestDeps, config: IngestConfig) -> Arc<Self> {
        let watch_clock = Arc::clone(&deps.clock);
        // 원격 접속 허브에 DB·기록기를 꽂는다 (보고 + 커서를 한 트랜잭션에 쓴다)
        config.remote.bind(HubDeps {
            pool: Arc::clone(&deps.pool),
            clock: Arc::clone(&deps.clock),
            committer: Arc::new(pending::DbCommitter::new(
                Arc::clone(&deps.pool),
                Arc::clone(&deps.clock),
                config.debounce_ms,
                config.pending.max_rows_per_folder,
            )),
            secrets: Arc::clone(&config.secrets),
        });
        let jobs = JobQueue::with_concurrency(
            Arc::clone(&deps.pool),
            Arc::clone(&deps.clock),
            config.concurrency,
        );
        let env = Arc::new(ScanEnv {
            pool: deps.pool,
            clock: deps.clock,
            identity: deps.identity,
            hasher: deps.hasher,
            auto_excludes: config.auto_excludes.clone(),
            progress: ScanProgressCell::default(),
            io: IoGuard::new(config.io_guard),
            schedule_processing: std::sync::atomic::AtomicBool::new(false),
            recent_guard_ms: config.pending.recent_guard_ms,
            pending_cap: config.pending.max_rows_per_folder,
            pending_due_ms: config.debounce_ms,
        });
        let (tx, rx) = channel();
        let detectors = Arc::clone(&config.detectors);
        let mut registry = ExecutorRegistry::new();
        registry.register(
            JobKind::Scan,
            Arc::new(ScanExecutor {
                env: Arc::clone(&env),
            }),
        );
        Arc::new_cyclic(|me| Self {
            env,
            jobs,
            volumes: deps.volumes,
            config,
            watchers: WatchManager::with_clock(tx, detectors, watch_clock),
            watch_rx: Mutex::new(Some(rx)),
            registry: Mutex::new(registry),
            runner: Mutex::new(None),
            threads: Mutex::new(Vec::new()),
            stop: Arc::default(),
            totals: Mutex::new(None),
            watch_failed: Mutex::default(),
            watch_ctl: Mutex::default(),
            backfilled: std::sync::atomic::AtomicBool::new(false),
            on_start_pass: std::sync::atomic::AtomicBool::new(false),
            policy_warned: Mutex::default(),
            suppressed: Arc::default(),
            pending: pending::PendingState::default(),
            stall_log: stall::StallLog::default(),
            newvol: newvol::NewVolumes::default(),
            device: device::DeviceHook::default(),
            removal: removal::Removals::default(),
            eject: eject::Ejects::default(),
            me: me.clone(),
        })
    }

    pub fn jobs(&self) -> &Arc<JobQueue> {
        &self.jobs
    }

    /// 동시에 도는 스캔의 전체 상한을 바꾼다 (설정 `maxConcurrentScans`, 허용 범위 밖은 범위로 맞춘다). 같은 볼륨은 항상 하나씩이다.
    pub fn set_scan_concurrency(&self, cap: usize) {
        self.jobs.set_scan_cap(cap);
    }

    /// 응답 없음 상태인 볼륨 (파일 호출이 막혀 검사 상한에 닿았다 — 풀리면 사라진다)
    pub fn unresponsive_volumes(&self) -> Vec<i64> {
        self.env.io.unresponsive_volumes()
    }

    /// 메타·썸네일·해시 등 다른 종류의 실행기를 꽂는다 (P2-3). [`Ingest::start`] 전에 호출한다.
    pub fn register_executor(&self, kind: JobKind, executor: Arc<dyn JobExecutor>) {
        lock(&self.registry).register(kind, executor);
    }

    /// 자산 처리(메타 -> 썸네일 -> 해시, P2-3)를 켠다: 실행기를 꽂고, 이후 스캔이 새·변경 자산마다 처리 작업을 등록하며,
    /// 첫 점검에서 기존 자산의 누락분을 보충한다. [`Ingest::start`] 전에 호출한다.
    pub fn enable_processing(&self, env: Arc<ProcessingEnv>) {
        self.register_executor(JobKind::Meta, Arc::new(MetaExecutor::new(Arc::clone(&env))));
        self.register_executor(
            JobKind::Thumb,
            Arc::new(ThumbExecutor::new(Arc::clone(&env))),
        );
        self.register_executor(JobKind::Hash, Arc::new(HashExecutor::new(env)));
        self.env.schedule_processing.store(true, Ordering::Relaxed);
    }

    /// 작업 복구 -> 실행기 시작 -> 감시 시작 -> 점검 스레드 시작.
    pub fn start(self: &Arc<Self>) -> Result<(), IngestError> {
        self.jobs.recover()?;
        // 실행기 시작 전에 일시정지 상태를 복원한다 - 재시작 직후 멈춰 둔 종류의 작업이 돌지 않게
        self.jobs.restore_pause()?;
        // 스캔 동시성 상한은 저장된 설정으로 — 실행기가 첫 스캔을 집기 전에 맞춘다
        self.apply_scan_cap_setting();
        let registry = lock(&self.registry).clone();
        *lock(&self.runner) = Some(JobRunner::start(Arc::clone(&self.jobs), registry)?);

        // 변경 대기열: 지난 실행이 남긴 행은 표본을 버리고 다시 확인한다. 비정상 종료 표식이 남아 있으면 실시간 감시 폴더를 재스캔한다
        if let Err(e) = self.recover_pending() {
            tracing::warn!(%e, "cannot recover pending watch changes");
        }
        if let Err(e) = self.begin_run() {
            tracing::warn!(%e, "cannot write the run marker");
        }
        let rx = lock(&self.watch_rx).take();
        if let Some(rx) = rx {
            let me = Arc::clone(self);
            lock(&self.threads).push(
                std::thread::Builder::new()
                    .name("mam-watch-writer".into())
                    .spawn(move || me.writer_loop(&rx))?,
            );
        }
        let me = Arc::clone(self);
        lock(&self.threads).push(
            std::thread::Builder::new()
                .name("mam-watch-apply".into())
                .spawn(move || me.processor_loop())?,
        );
        self.start_device_watch();
        // 지난 실행에서 끝내지 못한 폴더 제거를 이어서 하고, 옛 보관 데이터를 1회 정리한다 (둘 다 낮은 우선순위 백그라운드)
        if let Err(e) = self.resume_removals() {
            tracing::warn!(%e, "cannot resume folder removals");
        }
        self.spawn_legacy_cleanup();
        let me = Arc::clone(self);
        lock(&self.threads).push(
            std::thread::Builder::new()
                .name("mam-supervisor".into())
                .spawn(move || {
                    // 앱 시작 직후 한 번, 이후 간격마다 점검
                    loop {
                        if let Err(e) = me.supervise_once() {
                            tracing::warn!(%e, "supervisor tick failed");
                        }
                        if me.stop.wait_tick(me.config.supervise_every) == Tick::Stopped {
                            break;
                        }
                    }
                })?,
        );
        Ok(())
    }

    /// 모든 스레드를 정리한다. DB 풀 종료보다 **먼저** 호출한다.
    pub fn shutdown(&self) {
        self.shutdown_within(Duration::MAX);
    }

    /// 종료 신호만 보낸다 (기다리지 않는다): 감시 중단, 점검·감시 스레드 정지, 작업 실행기에 종료 알림.
    /// 실행 중인 작업은 체크포인트에서 양보하고, 사이드카(FFmpeg 등)는 종료 신호 공유 핸들로 즉시 죽는다.
    pub fn request_shutdown(&self) {
        self.stop.set();
        self.stop_device_watch();
        self.watchers.stop_all();
        self.jobs.request_shutdown();
    }

    /// [`Self::shutdown`] 이되 `grace` 를 넘긴 스레드는 기다리지 않고 떼어 둔다. 떼어 낸 스레드 수를 돌려준다.
    /// 작업 스레드는 DB 의 `running` 행으로 남아 다음 시작의 `recover` 가 이어서 실행한다 (실패·시도 횟수로 세지 않는다).
    pub fn shutdown_within(&self, grace: Duration) -> usize {
        let t = std::time::Instant::now();
        self.request_shutdown();
        let watchers_ms = t.elapsed().as_millis() as u64;
        let mut abandoned = 0;
        if let Some(r) = lock(&self.runner).take() {
            abandoned += r.stop_within(grace);
        }
        let runner_ms = t.elapsed().as_millis() as u64;
        // 감시 기록기·처리기(`threads`)를 먼저 기한 안에 기다린다 — 이 스레드들이 끝나야 표식을 지울 수 있다
        let threads: Vec<_> = std::mem::take(&mut *lock(&self.threads));
        let left = grace.saturating_sub(t.elapsed());
        let threads_abandoned = crate::join::join_within(threads, left);
        let removals: Vec<_> = std::mem::take(&mut *lock(&self.removal.handles));
        let left = grace.saturating_sub(t.elapsed());
        abandoned += threads_abandoned + crate::join::join_within(removals, left);
        // 정상 종료 — 기록기가 마저 쓴 뒤에만 표식을 지운다. 기한 안에 끝나지 않은 스레드가 있으면 표식을 남겨
        // 다음 시작이 비정상 종료로 보고 실시간 감시 폴더를 재스캔하게 한다 (놓친 변경 만회)
        if threads_abandoned == 0 {
            self.end_run();
        } else {
            tracing::warn!(threads_abandoned, "watch threads did not finish in time; run marker kept");
        }
        tracing::info!(
            watchers_ms,
            runner_ms,
            total_ms = t.elapsed().as_millis() as u64,
            abandoned,
            "ingest shutdown"
        );
        abandoned
    }

    /// 가드가 살아 있는 동안 `dir` 아래의 감시 이벤트를 버린다 (원본 이동 중 대상 폴더용, P2-9).
    pub fn suppress_watch(&self, dir: &std::path::Path) -> WatchSuppression {
        self.suppressed.add(dir)
    }

    /// 감시 쪽에서 재스캔을 예약한다 (실패는 로그만).
    fn schedule_rescan_quiet(&self, folder_id: i64) {
        if let Err(e) = self.enqueue_scan(folder_id, false) {
            tracing::warn!(%e, folder_id, "cannot schedule rescan");
        }
    }

    pub(crate) fn enqueue_scan(&self, folder_id: i64, explicit: bool) -> Result<bool, IngestError> {
        if explicit {
            // 수동 재스캔은 실패 기록을 정리하고 새로 시작한다 (supervisor 는 실패 폴더를 자동 재시도하지 않는다)
            self.env.write(mam_db::Priority::Low, move |c| {
                c.execute(
                    "DELETE FROM jobs WHERE kind = 'scan' AND folder_id = ?1 AND status = 'failed'",
                    [folder_id],
                )?;
                Ok(())
            })?;
        }
        Ok(self
            .jobs
            .enqueue(NewJob::for_folder(JobKind::Scan, folder_id))?)
    }

    /// 감시 메시지 1건을 디바운스 없이 처리한다 (시험·진단용): Path·Subtree -> 즉시 반영, Rescan -> 재스캔 예약.
    pub fn handle_watch_now(&self, msg: WatchMsg) -> Result<(), IngestError> {
        match msg {
            WatchMsg::Path { folder_id, path: p } | WatchMsg::Subtree { folder_id, dir: p } => {
                apply_path(&self.env, folder_id, &p)?;
            }
            WatchMsg::Rescan { folder_id } => {
                self.enqueue_scan(folder_id, false)?;
            }
        }
        Ok(())
    }

    pub(crate) fn watchers(&self) -> &WatchManager {
        &self.watchers
    }

    pub fn env(&self) -> &Arc<ScanEnv> {
        &self.env
    }
}
