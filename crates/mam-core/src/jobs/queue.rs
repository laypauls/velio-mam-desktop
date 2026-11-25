//! `jobs` 테이블 위의 대기열. 모든 쓰기는 DB 쓰기 스레드(`Low`)를 거친다.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use mam_db::{DbError, DbPool, Priority};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{Job, JobCounts, JobError, JobKind, JobOutcome, JobsError, MAX_ATTEMPTS};
use crate::SharedClock;

const BACKOFF_BASE_MS: i64 = 30_000;
const BACKOFF_MAX_MS: i64 = 3_600_000;

/// n 번째 실패 뒤 다음 시도까지의 대기 (30s, 60s, 120s, ... 최대 1시간).
pub fn backoff_ms(attempts: i64) -> i64 {
    let exp = u32::try_from(attempts.saturating_sub(1).clamp(0, 20)).unwrap_or(20);
    BACKOFF_BASE_MS
        .saturating_mul(1i64 << exp)
        .min(BACKOFF_MAX_MS)
}

/// 등록할 작업. `asset_id` / `folder_id` 중 해당하는 쪽을 채운다.
#[derive(Debug, Clone, Default)]
pub struct NewJob {
    pub kind: Option<JobKind>,
    pub asset_id: Option<i64>,
    pub folder_id: Option<i64>,
    /// None = 종류별 기본값
    pub priority: Option<i64>,
    /// 이 시각(UTC ms) 이전에는 시작하지 않는다. 0 = 즉시
    pub not_before: i64,
}

impl NewJob {
    pub fn for_folder(kind: JobKind, folder_id: i64) -> Self {
        Self {
            kind: Some(kind),
            folder_id: Some(folder_id),
            ..Self::default()
        }
    }
    pub fn for_asset(kind: JobKind, asset_id: i64) -> Self {
        Self {
            kind: Some(kind),
            asset_id: Some(asset_id),
            ..Self::default()
        }
    }
}

mod inflight;
mod lanes;
mod pause;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// 실행 중인 행 `id` 와 같은 (kind, asset|folder) 의 대기 행이 있으면 `id` 행을 지운다 (`true`).
/// `finish` 가 행을 `pending` 으로 되돌릴 때 `uq_jobs_pending_*` 위반으로 실패해 행이 `running` 으로 굳는 것을 막는다.
fn drop_if_pending_twin(c: &rusqlite::Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "DELETE FROM jobs WHERE job_id = ?1 AND EXISTS (
           SELECT 1 FROM jobs t
           WHERE t.job_id <> jobs.job_id AND t.kind = jobs.kind AND t.status = 'pending'
             AND ((jobs.asset_id IS NOT NULL AND t.asset_id = jobs.asset_id)
                  OR (jobs.folder_id IS NOT NULL AND t.folder_id = jobs.folder_id)))",
        [id],
    )? > 0)
}

pub struct JobQueue {
    pool: Arc<DbPool>,
    clock: SharedClock,
    /// 종류별 동시성 상한 (스캔 칸은 시작값일 뿐 — 실제 상한은 `scan_cap`)
    caps: [usize; JobKind::COUNT],
    /// 스캔 전체 동시성 상한 (같은 볼륨은 항상 1개 — `lanes.rs`). 설정으로 바뀐다
    scan_cap: AtomicUsize,
    /// 실행 중인 스캔이 차지한 볼륨 차선
    lanes: lanes::Lanes,
    paused_all: AtomicBool,
    paused_mask: AtomicU8,
    running: [AtomicUsize; JobKind::COUNT],
    claim_lock: Mutex<()>,
    pause_lock: Mutex<()>,
    signal: Mutex<u64>,
    wake: Condvar,
    generation: AtomicU64,
    shutdown: Arc<AtomicBool>,
    /// 실행 중인 작업과 작업별 취소 플래그 (inflight.rs)
    inflight: inflight::Inflight,
}

impl JobQueue {
    pub fn new(pool: Arc<DbPool>, clock: SharedClock) -> Arc<Self> {
        Self::with_concurrency(pool, clock, JobKind::ALL.map(JobKind::default_concurrency))
    }

    /// `caps` 는 `JobKind::ALL` 순서의 종류별 동시성 상한 (0 은 1 로 올린다).
    pub fn with_concurrency(
        pool: Arc<DbPool>,
        clock: SharedClock,
        caps: [usize; JobKind::COUNT],
    ) -> Arc<Self> {
        let caps = caps.map(|c| c.max(1));
        Arc::new(Self {
            pool,
            clock,
            scan_cap: AtomicUsize::new(caps[JobKind::Scan.idx()]),
            lanes: lanes::Lanes::default(),
            caps,
            paused_all: AtomicBool::new(false),
            paused_mask: AtomicU8::new(0),
            running: Default::default(),
            claim_lock: Mutex::new(()),
            pause_lock: Mutex::new(()),
            signal: Mutex::new(0),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            shutdown: Arc::new(AtomicBool::new(false)),
            inflight: inflight::Inflight::default(),
        })
    }

    pub fn pool(&self) -> &Arc<DbPool> {
        &self.pool
    }

    // ── 상태 알림 ──────────────────────────────────────────────

    /// 상태가 바뀔 때마다 오르는 값 (진행률 이벤트 발행 여부 판단용).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// 상태가 바뀌었음을 알린다 (generation 증가 + 디스패처 깨움).
    pub fn notify(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        *lock(&self.signal) += 1;
        self.wake.notify_all();
    }

    /// 새 신호가 올 때까지(또는 `timeout`) 기다린다. `seen` 은 마지막으로 본 신호 번호.
    pub fn wait(&self, seen: &mut u64, timeout: Duration) {
        let mut g = lock(&self.signal);
        if *g == *seen {
            g = self
                .wake
                .wait_timeout(g, timeout)
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
        *seen = *g;
    }

    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.inflight_cancel_all();
        self.notify();
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// 종료 신호의 공유 핸들 - 사이드카 실행기가 `request_shutdown` 시 프로세스를 바로 죽이는 데 쓴다.
    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    pub fn running_count(&self, kind: JobKind) -> usize {
        self.running[kind.idx()].load(Ordering::SeqCst)
    }

    // ── 영속 상태 ──────────────────────────────────────────────

    /// 앱 시작 시 호출: 이전 실행이 남긴 `running` 을 `pending` 으로 되돌린다 (중복 실행 없음 — 이 시점엔 실행 중인 작업이 없다).
    pub fn recover(&self) -> Result<usize, JobsError> {
        let now = self.clock.now_ms();
        let n = self.pool.write(Priority::Low, move |c| {
            // 실행 중에 같은 (kind, asset|folder) 의 대기 작업이 다시 등록돼 있을 수 있다(부분 UNIQUE 는 대기 행만 막는다).
            // 그대로 pending 으로 되돌리면 UNIQUE 위반으로 시작이 영구히 실패하므로, 대기 쌍둥이가 있는 running 행은 버린다.
            c.execute(
                "DELETE FROM jobs WHERE status = 'running' AND EXISTS (
                   SELECT 1 FROM jobs t
                   WHERE t.job_id <> jobs.job_id AND t.kind = jobs.kind
                     AND ((jobs.asset_id IS NOT NULL AND t.asset_id = jobs.asset_id)
                          OR (jobs.folder_id IS NOT NULL AND t.folder_id = jobs.folder_id))
                     AND (t.status = 'pending' OR (t.status = 'running' AND t.job_id < jobs.job_id)))",
                [],
            )?;
            Ok(c.execute(
                "UPDATE jobs SET status = 'pending', updated_at = ?1 WHERE status = 'running'",
                [now],
            )?)
        })?;
        self.notify();
        Ok(n)
    }

    /// 작업을 등록한다. 같은 (kind, asset) / (kind, folder) 의 대기 작업이 이미 있으면 등록하지 않고 `false`.
    pub fn enqueue(&self, new: NewJob) -> Result<bool, JobsError> {
        let kind = new.kind.ok_or_else(|| {
            JobsError::Sql(rusqlite::Error::InvalidParameterName("job kind".into()))
        })?;
        let now = self.clock.now_ms();
        let priority = new.priority.unwrap_or_else(|| kind.default_priority());
        let inserted = self.pool.write(Priority::Low, move |c| {
            Ok(c.execute(
                "INSERT OR IGNORE INTO jobs
                   (kind, asset_id, folder_id, priority, not_before, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    kind.as_str(),
                    new.asset_id,
                    new.folder_id,
                    priority,
                    new.not_before,
                    now
                ],
            )?)
        })?;
        if inserted > 0 {
            self.notify();
        }
        Ok(inserted > 0)
    }

    /// 다음에 실행할 작업 1건을 `running` 으로 표시해 돌려준다.
    /// `kinds` 중 일시정지되지 않았고 동시성 여유가 있는 종류만 대상이며, 우선순위 -> 등록 순으로 고른다.
    pub fn claim_next(&self, kinds: &[JobKind]) -> Result<Option<Job>, JobsError> {
        let _guard = lock(&self.claim_lock);
        let eligible: Vec<JobKind> = kinds
            .iter()
            .copied()
            .filter(|k| !self.is_paused(*k) && self.running_count(*k) < self.cap_of(*k))
            .collect();
        if eligible.is_empty() {
            return Ok(None);
        }
        let now = self.clock.now_ms();
        // 장면 검출 게이트: 메타·썸네일이 실행 중이면 사용자 요청(높은 우선순위)만 시작한다 - 대량 수집 중 백필이
        // CPU 를 나눠 먹지 않게 한다. 메모리 카운터만 보므로 DB 접근이 늘지 않고, 이미 실행 중인 작업은 건드리지 않는다
        let busy = self.running_count(JobKind::Meta) + self.running_count(JobKind::Thumb) > 0;
        let eligible: Vec<(JobKind, i64)> = eligible
            .into_iter()
            .map(|k| {
                let cap = if k == JobKind::Scene && busy {
                    mam_db::scene_cuts::SCENE_PRIORITY_USER
                } else {
                    i64::MAX
                };
                (k, cap)
            })
            .collect();
        let busy = self.busy_lanes();
        let claimed = self.pool.write(Priority::Low, move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut best: Option<Job> = None;
            let mut best_lane: Option<i64> = None;
            for (kind, max_priority) in eligible {
                // 스캔은 이미 스캔이 도는 볼륨(차선)의 작업을 건너뛰고 다른 볼륨의 작업을 고른다
                let row = if kind == JobKind::Scan {
                    lanes::next_scan_job(&tx, now, max_priority, &busy)?
                } else {
                    tx.query_row(
                        "SELECT job_id, asset_id, folder_id, priority, attempts FROM jobs
                         WHERE kind = ?1 AND status = 'pending' AND not_before <= ?2 AND priority <= ?3
                         ORDER BY priority, job_id LIMIT 1",
                        params![kind.as_str(), now, max_priority],
                        |r| {
                            Ok(Job {
                                job_id: r.get(0)?,
                                kind,
                                asset_id: r.get(1)?,
                                folder_id: r.get(2)?,
                                priority: r.get(3)?,
                                attempts: r.get(4)?,
                            })
                        },
                    )
                    .optional()?
                    .map(|j| (j, 0))
                };
                if let Some((j, lane)) = row
                    && best
                        .as_ref()
                        .is_none_or(|b| (j.priority, j.job_id) < (b.priority, b.job_id))
                {
                    best_lane = (j.kind == JobKind::Scan).then_some(lane);
                    best = Some(j);
                }
            }
            if let Some(j) = &best {
                tx.execute(
                    "UPDATE jobs SET status = 'running', updated_at = ?2 WHERE job_id = ?1",
                    params![j.job_id, now],
                )?;
            }
            tx.commit()?;
            Ok::<_, DbError>(best.map(|j| (j, best_lane)))
        })?;
        let job = claimed.map(|(j, lane)| {
            if let Some(lane) = lane {
                self.lane_occupy(j.job_id, lane);
            }
            j
        });
        if let Some(j) = &job {
            self.running[j.kind.idx()].fetch_add(1, Ordering::SeqCst);
            self.inflight_register(j);
            self.notify();
        }
        Ok(job)
    }

    /// 실행 결과를 반영한다: 완료는 행 삭제, 중단은 대기로 복귀, 실패는 백오프 재시도 또는 `failed`.
    pub fn finish(&self, job: &Job, result: Result<JobOutcome, JobError>) -> Result<(), JobsError> {
        let now = self.clock.now_ms();
        let id = job.job_id;
        let attempts = job.attempts + 1;
        let written = self.pool.write(Priority::Low, move |c| {
            match result {
                Ok(JobOutcome::Done) => {
                    c.execute("DELETE FROM jobs WHERE job_id = ?1", [id])?;
                }
                Ok(JobOutcome::Yielded) => {
                    // 실행 중에 같은 대상의 대기 작업이 다시 등록됐으면(부분 UNIQUE 는 running 을 막지 않는다) 그쪽이 이어받는다
                    if !drop_if_pending_twin(c, id)? {
                        c.execute(
                            "UPDATE jobs SET status = 'pending', updated_at = ?2 WHERE job_id = ?1",
                            params![id, now],
                        )?;
                    }
                }
                Err(e) if e.retryable && attempts < MAX_ATTEMPTS => {
                    let after = e
                        .retry_after
                        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX / 2));
                    let wait = backoff_ms(attempts).max(after);
                    // 대기 쌍둥이가 있으면 그쪽이 바로 다시 돈다 (이 행의 백오프·시도 횟수는 버려진다)
                    if !drop_if_pending_twin(c, id)? {
                        c.execute(
                            "UPDATE jobs SET status = 'pending', attempts = ?2, not_before = ?3,
                               last_error = ?4, updated_at = ?5 WHERE job_id = ?1",
                            params![id, attempts, now.saturating_add(wait), e.message, now],
                        )?;
                    }
                }
                Err(e) => {
                    c.execute(
                        "UPDATE jobs SET status = 'failed', attempts = ?2, last_error = ?3,
                           updated_at = ?4 WHERE job_id = ?1",
                        params![id, attempts, e.message, now],
                    )?;
                }
            }
            Ok::<_, DbError>(())
        });
        // DB 쓰기가 실패해도 메모리의 실행 중 카운트는 반드시 내린다 (슬롯 누수 방지)
        let slot = &self.running[job.kind.idx()];
        let _ = slot.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
            Some(v.saturating_sub(1))
        });
        self.inflight_remove(job.job_id);
        self.lane_release(job.job_id);
        self.notify();
        written?;
        Ok(())
    }

    /// `failed` 작업을 대기로 되돌린다 (수동 재시도). 되돌린 개수를 반환.
    pub fn retry_failed(&self, kind: Option<JobKind>) -> Result<usize, JobsError> {
        let now = self.clock.now_ms();
        let kind = kind.map(JobKind::as_str);
        let n = self.pool.write(Priority::Low, move |c| {
            Ok(c.execute(
                "UPDATE OR IGNORE jobs SET status = 'pending', attempts = 0, not_before = 0, updated_at = ?2
                 WHERE status = 'failed' AND (?1 IS NULL OR kind = ?1)",
                params![kind, now],
            )?)
        })?;
        self.notify();
        Ok(n)
    }

    /// 종류별 건수 (`failed` 는 수동 재시도 대상). `JobKind::ALL` 순서.
    /// 상태 표시줄용이므로 **숨김 폴더의 스캔 작업은 뺀다** (`folder_id` 없는 스캔 작업·다른 종류는 그대로).
    /// 스캔 외 종류는 기존처럼 `(kind, status)` 인덱스만 훑고, 행이 적은 스캔 종류만 숨김 여부를 본다.
    pub fn counts(&self) -> Result<Vec<(JobKind, JobCounts)>, JobsError> {
        let rows: Vec<(String, String, i64)> = self.pool.read(|c| {
            let mut stmt = c.prepare(
                "SELECT kind, status, COUNT(*) FROM jobs WHERE kind <> 'scan' GROUP BY kind, status
                 UNION ALL
                 SELECT kind, status, COUNT(*) FROM jobs
                 WHERE kind = 'scan'
                   AND (folder_id IS NULL
                        OR folder_id NOT IN (SELECT folder_id FROM managed_folders WHERE hidden = 1))
                 GROUP BY kind, status",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })?;
        let mut out: Vec<(JobKind, JobCounts)> = JobKind::ALL
            .into_iter()
            .map(|k| (k, JobCounts::default()))
            .collect();
        for (kind, status, n) in rows {
            let Some(kind) = JobKind::parse(&kind) else {
                continue;
            };
            let n = u64::try_from(n).unwrap_or(0);
            if let Some((_, c)) = out.iter_mut().find(|(k, _)| *k == kind) {
                match status.as_str() {
                    "pending" => c.pending = n,
                    "running" => c.running = n,
                    "failed" => c.failed = n,
                    _ => {}
                }
            }
        }
        Ok(out)
    }

    /// 폴더의 대기·실패 스캔 작업 상태 (`pending` / `running` / `failed`) — 폴더 상태 표시용.
    pub fn folder_scan_statuses(&self) -> Result<Vec<(i64, String)>, JobsError> {
        Ok(self.pool.read(|c| {
            let mut stmt = c.prepare(
                "SELECT folder_id, status FROM jobs WHERE kind = 'scan' AND folder_id IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })?)
    }
}
