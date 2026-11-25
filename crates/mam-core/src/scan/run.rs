//! 스캔 실행의 수명주기: 실행 시작/재개 -> 순회 -> 완료 처리(missing·트리 정리·오류 해결) 또는 일시정지.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use mam_db::Priority;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::IngestError;
use super::errors::{record_error, resolve_stale};
use super::folders::{FolderRow, load_folder, path_text};
use super::guarded::{UNRESPONSIVE_CODE, UNRESPONSIVE_MESSAGE};
use super::io_guard::Guarded;
use super::work::{ScanCounts, ScanEnv, Task, Walk, WalkEnd};
use crate::CoreError;
use crate::jobs::{Job, JobContext, JobError, JobExecutor, JobOutcome};
use crate::keyrange::{join_key, key_range};
use crate::reconcile::{Reconciler, ScanSession, touch_under};

/// 스캔 1회의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanOutcome {
    Completed {
        counts: ScanCounts,
    },
    /// 일시정지·종료로 중단 — 재개 지점이 저장됐다
    Paused,
    /// 폴더가 없거나 제거 중
    Gone,
    /// 볼륨이 연결돼 있지 않다 — 아무것도 바꾸지 않았다
    Offline,
    /// 루트 폴더를 읽을 수 없다(제한 시간 초과 포함) — 아무것도 missing 으로 만들지 않았다
    RootUnreadable,
    /// 순회 도중 볼륨이 응답 없음이 됐다 — 재개 지점을 저장하고 멈췄다 (끝난 것으로 치지 않아 missing 처리 없음)
    Unresponsive,
}

struct Ready {
    folder: FolderRow,
    run_id: i64,
    cursor: Option<String>,
    counts: ScanCounts,
    started_at: i64,
}

enum Setup {
    Ready(Box<Ready>),
    Gone,
    Offline,
}

fn setup(c: &mut Connection, folder_id: i64, now: i64) -> Result<Setup, IngestError> {
    let folder = match load_folder(c, folder_id) {
        Ok(f) => f,
        Err(IngestError::NotFound) => return Ok(Setup::Gone),
        Err(e) => return Err(e),
    };
    if !folder.enabled {
        return Ok(Setup::Gone);
    }
    if !folder.online {
        return Ok(Setup::Offline);
    }
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let resumable: Option<(i64, Option<String>, String, i64)> = tx
        .query_row(
            "SELECT run_id, cursor, counts, started_at FROM scan_runs
             WHERE folder_id = ?1 AND status IN ('running', 'paused')
             ORDER BY run_id DESC LIMIT 1",
            [folder_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (run_id, cursor, counts, started_at) = match resumable {
        Some((run_id, cursor, counts, started_at)) => {
            // 더 오래된 미완료 실행은 정리한다 (하나만 이어간다)
            tx.execute(
                "UPDATE scan_runs SET status = 'cancelled', finished_at = ?3
                 WHERE folder_id = ?1 AND run_id <> ?2 AND status IN ('running', 'paused')",
                params![folder_id, run_id, now],
            )?;
            tx.execute(
                "UPDATE scan_runs SET status = 'running' WHERE run_id = ?1",
                [run_id],
            )?;
            (
                run_id,
                cursor,
                serde_json::from_str(&counts).unwrap_or_default(),
                started_at,
            )
        }
        None => {
            let kind = if folder.last_full_scan_at.is_none() {
                "full"
            } else {
                "rescan"
            };
            tx.execute(
                "INSERT INTO scan_runs (folder_id, kind, status, started_at)
                 VALUES (?1, ?2, 'running', ?3)",
                params![folder_id, kind, now],
            )?;
            (tx.last_insert_rowid(), None, ScanCounts::default(), now)
        }
    };
    tx.commit()?;
    Ok(Setup::Ready(Box::new(Ready {
        folder,
        run_id,
        cursor,
        counts,
        started_at,
    })))
}

fn counts_json(c: &ScanCounts) -> String {
    serde_json::to_string(c).unwrap_or_else(|_| "{}".into())
}

/// 폴더 하나를 스캔한다 (처음이면 새 실행, 중단된 실행이 있으면 cursor 이후부터 이어서).
///
/// `stop` 이 true 를 돌려주면 다음 디렉터리 경계에서 멈추고 재개 지점을 저장한다.
pub fn scan_folder(
    env: &ScanEnv,
    folder_id: i64,
    stop: &dyn Fn() -> bool,
    cancel: &AtomicBool,
) -> Result<ScanOutcome, IngestError> {
    let now = env.clock.now_ms();
    let ready = match env.write(Priority::Low, move |c| setup(c, folder_id, now))? {
        Setup::Gone => return Ok(ScanOutcome::Gone),
        Setup::Offline => return Ok(ScanOutcome::Offline),
        Setup::Ready(r) => *r,
    };
    let Ready {
        folder,
        run_id,
        cursor,
        counts,
        started_at,
    } = ready;
    let root = folder.root_path();

    // 루트를 읽을 수 없으면 "전부 삭제됨" 이 아니라 오류다 — missing 처리하지 않는다. 응답이 없는 것(제한 시간 초과)도 같다
    let root_failure: Option<(&'static str, &'static str, String)> =
        match env.root_readable_guarded(&folder) {
            Guarded::Done(Ok(())) => None,
            Guarded::Done(Err(f)) => Some((f.code, "root_unreadable", f.message)),
            Guarded::TimedOut => Some((
                UNRESPONSIVE_CODE,
                UNRESPONSIVE_CODE,
                UNRESPONSIVE_MESSAGE.to_owned(),
            )),
        };
    if let Some((code, error_type, message)) = root_failure {
        let network = folder.volume_kind == crate::VolumeKind::Network;
        let volume_id = folder.volume_id;
        env.write(Priority::Low, move |c| {
            if network {
                // 공유가 응답하지 않는 것으로 본다 — 오프라인 전환, 재연결 때 재스캔된다
                c.execute(
                    "UPDATE volumes SET availability = 'offline' WHERE volume_id = ?1",
                    [volume_id],
                )?;
            }
            record_error(c, folder_id, "", error_type, &message, now)?;
            c.execute(
                "UPDATE scan_runs SET status = 'failed', error_code = ?2, finished_at = ?3 WHERE run_id = ?1",
                params![run_id, code, now],
            )?;
            Ok(())
        })?;
        return Ok(ScanOutcome::RootUnreadable);
    }

    let session = env.write(Priority::Low, move |c| {
        Ok(ScanSession::begin_at(c, started_at, folder.volume_id)?)
    });
    let session = Arc::new(Mutex::new(session?));
    let folder = load_current(env, folder_id)?;
    let Some(folder) = folder else {
        return Ok(ScanOutcome::Gone);
    };
    env.progress.start(
        folder_id,
        path_text(&root),
        counts.processed,
        env.clock.now_ms(),
    );

    let mut walk = Walk::new(
        env,
        &folder,
        Arc::clone(&session),
        started_at,
        Some(run_id),
        counts,
        cancel,
        stop,
    );
    let end = walk.walk(Task::root(root), cursor.as_deref());
    let counts = walk.counts();
    drop(walk);
    let result = match end {
        Ok(WalkEnd::Completed) => finish(env, &folder, run_id, started_at, counts, &session),
        Ok(WalkEnd::Stopped) => pause(env, run_id, counts).map(|()| ScanOutcome::Paused),
        Ok(WalkEnd::FolderGone) => Ok(ScanOutcome::Gone),
        Ok(WalkEnd::Unresponsive) => pause(env, run_id, counts).map(|()| ScanOutcome::Unresponsive),
        // 종료 신호로 해시가 취소됨 — 일시정지와 같다
        Err(IngestError::Core(CoreError::Cancelled)) => {
            pause(env, run_id, counts).map(|()| ScanOutcome::Paused)
        }
        Err(e) => Err(e),
    };
    env.progress.clear(folder_id);
    result
}

fn load_current(env: &ScanEnv, folder_id: i64) -> Result<Option<FolderRow>, IngestError> {
    match env.pool.read(|c| Ok(load_folder(c, folder_id))) {
        Ok(Ok(f)) => Ok(Some(f)),
        Ok(Err(IngestError::NotFound)) => Ok(None),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(e.into()),
    }
}

fn pause(env: &ScanEnv, run_id: i64, counts: ScanCounts) -> Result<(), IngestError> {
    env.write(Priority::Low, move |c| {
        c.execute(
            "UPDATE scan_runs SET status = 'paused', counts = ?2 WHERE run_id = ?1",
            params![run_id, counts_json(&counts)],
        )?;
        Ok(())
    })
}

/// 순회가 끝까지 간 뒤의 마무리. 모두 멱등이라 중간에 죽어도 다음 실행이 같은 결과에 도달한다.
fn finish(
    env: &ScanEnv,
    folder: &FolderRow,
    run_id: i64,
    started_at: i64,
    mut counts: ScanCounts,
    session: &Arc<Mutex<ScanSession>>,
) -> Result<ScanOutcome, IngestError> {
    let clock = Arc::clone(&env.clock);
    let session = Arc::clone(session);
    let (folder_id, volume_id, root_key) =
        (folder.folder_id, folder.volume_id, folder.path_key.clone());
    env.write(Priority::Low, move |c| {
        // 1) 읽지 못한 경로의 하위 위치·트리 행은 "봤다" 로 보호한다 (읽기 오류가 missing/삭제 오판이 되지 않게)
        {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let rels: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT rel_path FROM scan_errors
                     WHERE folder_id = ?1 AND resolved_at IS NULL AND last_at >= ?2",
                )?;
                let rows = stmt.query_map(params![folder_id, started_at], |r| r.get(0))?;
                rows.collect::<Result<_, _>>()?
            };
            for rel in rels {
                let rel_key = mam_db::path_key(&rel).unwrap_or_default();
                touch_under(&tx, volume_id, &join_key(&root_key, &rel_key), started_at)?;
                let (lo, hi) = key_range(&rel_key);
                tx.execute(
                    "UPDATE folder_dirs SET seen_at = ?2
                     WHERE folder_id = ?1 AND seen_at < ?2
                       AND (dir_key = ?3 OR (dir_key >= ?4 AND dir_key < ?5))",
                    params![folder_id, started_at, rel_key, lo, hi],
                )?;
            }
            tx.commit()?;
        }
        // 2) 이번에 못 본 위치 -> missing (볼륨이 online 일 때만, 폴더 범위 안에서만)
        let reconciler = Reconciler {
            clock: &*clock,
            hasher: &crate::hash::Blake3Hasher,
            cancel: &AtomicBool::new(false),
        };
        let missing = {
            let s = session.lock().unwrap_or_else(|p| p.into_inner());
            reconciler.finish_scan_in(c, &s, &root_key)?
        };
        counts.missing = missing as u64;
        // 3) 트리 정리·오류 해결·실행 완료
        let now = clock.now_ms();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM folder_dirs WHERE folder_id = ?1 AND seen_at < ?2",
            params![folder_id, started_at],
        )?;
        resolve_stale(&tx, folder_id, started_at, now)?;
        tx.execute(
            "UPDATE scan_runs SET status = 'done', finished_at = ?2, counts = ?3 WHERE run_id = ?1",
            params![run_id, now, counts_json(&counts)],
        )?;
        tx.execute(
            "UPDATE managed_folders SET last_full_scan_at = ?2 WHERE folder_id = ?1",
            params![folder_id, now],
        )?;
        tx.commit()?;
        Ok(ScanOutcome::Completed { counts })
    })
}

/// `scan` 작업 실행기.
pub struct ScanExecutor {
    pub env: Arc<ScanEnv>,
}

impl JobExecutor for ScanExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let Some(folder_id) = job.folder_id else {
            return Err(JobError::fatal("scan job without folder"));
        };
        let stop = || ctx.should_stop();
        match scan_folder(&self.env, folder_id, &stop, ctx.shutdown_flag()) {
            Ok(ScanOutcome::Paused) => Ok(JobOutcome::Yielded),
            // 응답 없는 볼륨 — 바로 다시 집으면 같은 곳에서 돌기만 하므로 백오프로 미룬다 (재개 지점부터 이어간다)
            Ok(ScanOutcome::Unresponsive) => Err(JobError::retryable("volume is not responding")),
            Ok(_) => Ok(JobOutcome::Done),
            Err(e) => Err(JobError::retryable(e.to_string())),
        }
    }
}
