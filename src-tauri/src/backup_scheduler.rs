//! 자동 백업 스케줄러 (P2-9 D6) — 앱 실행 중에만 도는 타이머 스레드. 설정 `backup.enabled`·`intervalHours`·`keep`(PL9).
//!
//! 마지막 자동 백업 시각은 백업 폴더의 `mam-auto-<ms>.sqlite` 파일에서 읽는다(별도 상태 없음 -> 재시작해도 주기가 유지된다).
//! 현재 시각은 주입된 시계로만 읽고, 판단은 [`tick`] 한 함수에 모아 시계를 고정해 시험한다.
//! 실패하면 [`RETRY_AFTER_FAILURE_MS`] 동안 다시 시도하지 않는다 (디스크 가득 등에서 매 틱 재시도하지 않도록).

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use mam_core::SharedClock;
use mam_db::{BackupInfo, BackupKind, Clock};

use crate::storage_commands::Storage;

/// 점검 간격
pub const TICK: Duration = Duration::from_secs(60);
/// 앱 시작 직후 첫 점검까지의 유예 (기동 부하와 겹치지 않게)
pub const FIRST_DELAY: Duration = Duration::from_secs(30);
pub const RETRY_AFTER_FAILURE_MS: i64 = 15 * 60_000;
const HOUR_MS: i64 = 3_600_000;

#[derive(Debug, Default)]
pub struct SchedulerState {
    last_failure_at: Option<i64>,
}

/// 한 번 점검한다. 백업을 만들었으면 그 정보를 돌려준다.
pub fn tick(
    storage: &Storage,
    clock: &dyn Clock,
    state: &mut SchedulerState,
) -> Option<BackupInfo> {
    let settings = match storage.get_settings() {
        Ok(s) => s.settings,
        Err(e) => {
            tracing::warn!(code = ?e.code(), "auto backup: cannot read settings");
            return None;
        }
    };
    let b = &settings.backup;
    if !b.enabled {
        return None;
    }
    let now = clock.now_ms();
    if state
        .last_failure_at
        .is_some_and(|t| now >= t && now - t < RETRY_AFTER_FAILURE_MS)
    {
        return None;
    }
    let dir = storage.backup_dir(&settings);
    let last_auto = match mam_db::list_backups(&dir) {
        Ok(list) => list
            .into_iter()
            .find(|i| i.kind == BackupKind::Auto)
            .map(|i| i.created_at),
        Err(e) => {
            tracing::warn!(%e, "auto backup: cannot list backups");
            None
        }
    };
    let interval_ms = i64::from(b.interval_hours) * HOUR_MS;
    // 시스템 시계를 되돌렸다면(마지막 백업이 미래) 기다리지 않고 바로 만든다
    let due = last_auto.is_none_or(|t| t > now || now - t >= interval_ms);
    if !due {
        return None;
    }
    let protect = crate::update_guard::protected_backups(&storage.paths, clock);
    match mam_db::create_backup(
        &storage.paths.db(),
        &dir,
        BackupKind::Auto,
        b.keep as usize,
        &protect,
        clock,
    ) {
        Ok(info) => {
            // PR4: 만든 직후 백업본을 검사한다 — 손상 백업이 쌓여 정상 백업을 밀어내지 않게 지우고 경고한다.
            // (보관 개수 정리는 create_backup 안에서 이미 끝났다. 손상 백업이 정상 백업을 하나 밀어냈더라도 다음 주기에 다시 만든다.)
            if !keep_if_sound(&dir.join(&info.file_name)) {
                state.last_failure_at = Some(now);
                return None;
            }
            crate::restore::hint::record(&storage.paths, &dir);
            state.last_failure_at = None;
            tracing::info!(backup = %info.file_name, bytes = info.size_bytes, "auto backup created");
            Some(info)
        }
        Err(e) => {
            state.last_failure_at = Some(now);
            tracing::warn!(%e, "auto backup failed");
            None
        }
    }
}

/// PR4: 방금 만든 자동 백업에 `quick_check` 를 돌려, 실패하면 파일을 지우고 경고한다 (`false`).
fn keep_if_sound(path: &std::path::Path) -> bool {
    match mam_db::quick_check(path) {
        Ok(()) => true,
        Err(e) => {
            let _ = std::fs::remove_file(path);
            tracing::warn!(%e, "auto backup failed quick_check; deleted it");
            false
        }
    }
}

#[derive(Default)]
struct Stop {
    stopped: Mutex<bool>,
    cv: Condvar,
}

impl Stop {
    fn set(&self) {
        *self.stopped.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.cv.notify_all();
    }

    /// `timeout` 동안 기다린다. 종료 신호가 오면 true.
    fn wait(&self, timeout: Duration) -> bool {
        let g = self.stopped.lock().unwrap_or_else(|p| p.into_inner());
        if *g {
            return true;
        }
        let (g, _) = self
            .cv
            .wait_timeout(g, timeout)
            .unwrap_or_else(|p| p.into_inner());
        *g
    }
}

pub struct BackupScheduler {
    stop: Arc<Stop>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl BackupScheduler {
    pub fn start(
        storage: Arc<Storage>,
        clock: SharedClock,
        first_delay: Duration,
        every: Duration,
    ) -> std::io::Result<Arc<Self>> {
        let stop = Arc::new(Stop::default());
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("mam-auto-backup".into())
            .spawn(move || {
                let mut state = SchedulerState::default();
                let mut wait = first_delay;
                while !thread_stop.wait(wait) {
                    tick(&storage, &*clock, &mut state);
                    wait = every;
                }
            })?;
        Ok(Arc::new(Self {
            stop,
            handle: Mutex::new(Some(handle)),
        }))
    }

    /// 스레드를 멈추고 기다린다. DB 풀 종료보다 **먼저** 호출한다.
    #[cfg(test)]
    pub fn stop(&self) {
        self.stop_within(Duration::MAX);
    }

    /// 종료 신호만 보낸다 (기다리지 않는다).
    pub fn request_stop(&self) {
        self.stop.set();
    }

    /// [`Self::stop`] 이되 `grace` 를 넘기면 기다리지 않는다 (큰 DB 의 백업 복사 중일 수 있다).
    /// 백업은 `.partial` 로 쓰고 끝나야 이름을 바꾸므로 끊겨도 완성본으로 보이지 않고, 다음 백업이 남은 조각을 지운다.
    pub fn stop_within(&self, grace: Duration) -> usize {
        self.stop.set();
        let h = self.handle.lock().unwrap_or_else(|p| p.into_inner()).take();
        let abandoned = mam_core::join::join_within(h.into_iter().collect(), grace);
        if abandoned > 0 {
            tracing::warn!("auto backup still running at shutdown; not waiting");
        }
        abandoned
    }
}

#[cfg(test)]
mod tests {
    // Categories: Time (T-prot-12 주기·보관 개수, 시계 되감기) / Business rule (비활성) / External (실패 후 재시도 유예) /
    // Concurrency (스레드 즉시 정지)
    use std::sync::atomic::{AtomicI64, Ordering};

    use mam_core::storage::DataPaths;
    use mam_db::DbPool;
    use serde_json::json;

    use super::*;

    struct FakeClock(AtomicI64);

    impl FakeClock {
        fn advance(&self, ms: i64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }

    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    const T0: i64 = 1_800_000_000_000;

    fn setup() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = DataPaths::new(dir.path().join("MAMData"));
        paths.ensure_layout().unwrap();
        let mut conn = mam_db::open(&paths.db()).unwrap();
        mam_db::migrate(&mut conn, &paths.backups(), &FakeClock(AtomicI64::new(T0))).unwrap();
        drop(conn);
        let storage = Storage {
            pool: Arc::new(DbPool::open(&paths.db(), 2).unwrap()),
            paths,
            bootstrap_dir: dir.path().join("boot"),
            _lock: None,
        };
        (dir, storage)
    }

    fn set(s: &Storage, patch: serde_json::Value) {
        s.update_settings(
            patch.as_object().unwrap().clone(),
            &mam_core::storage::SystemDriveProbe,
            &[],
        )
        .unwrap();
    }

    fn autos(s: &Storage) -> Vec<i64> {
        mam_db::list_backups(&s.paths.backups())
            .unwrap()
            .into_iter()
            .filter(|b| b.kind == BackupKind::Auto)
            .map(|b| b.created_at)
            .collect()
    }

    #[test]
    fn t_prot_12_creates_on_interval_and_keeps_only_the_configured_count() {
        // Arrange: 2시간마다, 최대 2개
        let (_dir, s) = setup();
        set(&s, json!({"backup": {"intervalHours": 2, "keep": 2}}));
        let clock = FakeClock(AtomicI64::new(T0));
        let mut st = SchedulerState::default();

        // Act + Assert: 첫 점검은 자동 백업이 없으므로 즉시 생성
        assert!(tick(&s, &clock, &mut st).is_some());
        clock.advance(2 * HOUR_MS - 1);
        assert!(tick(&s, &clock, &mut st).is_none(), "interval not elapsed");
        clock.advance(1);
        assert!(
            tick(&s, &clock, &mut st).is_some(),
            "exactly on the interval"
        );
        clock.advance(2 * HOUR_MS);
        assert!(tick(&s, &clock, &mut st).is_some());
        assert_eq!(
            autos(&s),
            [T0 + 4 * HOUR_MS, T0 + 2 * HOUR_MS],
            "oldest pruned beyond keep"
        );
    }

    #[test]
    fn t_prot_12b_disabled_creates_nothing_and_the_interval_survives_a_restart() {
        // Arrange
        let (_dir, s) = setup();
        let clock = FakeClock(AtomicI64::new(T0));
        set(&s, json!({"backup": {"enabled": false}}));

        // Act + Assert: 비활성
        assert!(tick(&s, &clock, &mut SchedulerState::default()).is_none());
        assert!(autos(&s).is_empty());

        // 활성화 -> 생성, "재시작"(새 상태) 직후에는 백업 파일 때문에 다시 만들지 않는다
        set(&s, json!({"backup": {"enabled": true}}));
        assert!(tick(&s, &clock, &mut SchedulerState::default()).is_some());
        clock.advance(HOUR_MS);
        assert!(tick(&s, &clock, &mut SchedulerState::default()).is_none());
    }

    #[test]
    fn t_prot_12c_clock_set_back_does_not_block_and_failure_backs_off() {
        // Arrange
        let (dir, s) = setup();
        let clock = FakeClock(AtomicI64::new(T0));
        let mut st = SchedulerState::default();
        assert!(tick(&s, &clock, &mut st).is_some());

        // Act: 시계를 되감으면 마지막 백업이 미래 -> 바로 생성
        clock.advance(-HOUR_MS);
        assert!(tick(&s, &clock, &mut st).is_some());

        // 백업 폴더가 파일이라 쓰기 불가 -> 실패, 유예 안에서는 재시도하지 않는다
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        set(&s, json!({"backup": {"dir": blocker.to_str().unwrap()}}));
        clock.advance(48 * HOUR_MS);
        assert!(tick(&s, &clock, &mut st).is_none());
        assert_eq!(st.last_failure_at, Some(T0 + 47 * HOUR_MS));
        std::fs::remove_file(&blocker).unwrap();
        clock.advance(RETRY_AFTER_FAILURE_MS - 1);
        assert!(tick(&s, &clock, &mut st).is_none(), "backing off");
        clock.advance(1);
        assert!(
            tick(&s, &clock, &mut st).is_some(),
            "retries after the delay"
        );
    }

    #[test]
    fn t_bksched_restore_01_auto_backup_is_checked_and_a_damaged_one_is_deleted_and_backed_off() {
        // Arrange: 정상 생성 -> 검사 통과, 그 파일을 망가뜨리면 검사 실패 -> 삭제
        let (_dir, s) = setup();
        let clock = FakeClock(AtomicI64::new(T0));
        let mut st = SchedulerState::default();
        let made = tick(&s, &clock, &mut st).expect("sound backup is kept");
        let path = s.paths.backups().join(&made.file_name);
        assert!(path.is_file() && st.last_failure_at.is_none());
        assert!(
            crate::restore::hint::read(&s.paths).is_some(),
            "location hint recorded"
        );

        // Act + Assert: 손상된 백업은 지워진다 (정상 파일은 건드리지 않는다)
        assert!(keep_if_sound(&path));
        let damaged = s.paths.backups().join("mam-auto-1.sqlite");
        std::fs::write(&damaged, vec![0xAB; 8192]).unwrap();
        assert!(!keep_if_sound(&damaged));
        assert!(!damaged.exists());
        assert!(path.is_file());
    }

    #[test]
    fn t_prot_12d_scheduler_thread_stops_promptly_without_a_backup() {
        // Arrange: 첫 점검 유예가 길어도 stop 이 즉시 깨운다
        let (_dir, s) = setup();
        let sched = BackupScheduler::start(
            Arc::new(s),
            Arc::new(mam_db::SystemClock),
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        )
        .unwrap();

        // Act
        let started = std::time::Instant::now();
        sched.stop();

        // Assert
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn t_exit_sched_01_request_stop_then_bounded_join_returns_without_abandoning() {
        // Arrange
        let (_dir, s) = setup();
        let sched = BackupScheduler::start(
            Arc::new(s),
            Arc::new(mam_db::SystemClock),
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        )
        .unwrap();

        // Act
        let started = std::time::Instant::now();
        sched.request_stop();
        let abandoned = sched.stop_within(Duration::from_secs(5));

        // Assert
        assert_eq!(abandoned, 0);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(
            sched.stop_within(Duration::ZERO),
            0,
            "second stop is a no-op"
        );
    }
}
