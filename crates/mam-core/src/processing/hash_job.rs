//! `hash` 작업 실행기 — 중복 검사 단계 판정: 크기 -> 앞뒤 4K 체크섬(`edge_sig`) -> 전체 내용 해시(BLAKE3).
//!
//! 1. `edge_sig` 가 없으면 계산·저장하고, 같은 크기·같은 sig 이면서 전체 해시 없는 상대에게 `hash` 작업을 등록한다
//! 2. 같은 크기·같은 sig 의 다른 활성 자산이 **없으면 끝** — 전체 해시를 계산하지 않고 `hash_state = pending` 으로 남긴다
//!    (`pending` + `edge_sig` 있음 = "검사 완료, 전체 해시 불필요"). 앞뒤가 같다고 같은 파일은 아니므로 sig 는 후보 축소용일 뿐이다
//! 3. 있으면 전체 BLAKE3 를 계산해 기록하고 같은 해시의 `missing` 자산이 있으면 병합한다 (D6)
//!
//! 해시 I/O 는 DB 쓰기 스레드 **밖**에서 하고, 반영(해시 기록 + 병합)만 한 트랜잭션으로 쓴다.
//! 해시 도중 파일이 바뀌면 결과를 버린다 - 스캔·감시가 변경을 반영하면서 새 작업을 등록한다.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mam_media::{ThumbSize, strip_path, thumbnail_path};
use rusqlite::TransactionBehavior;

use super::edge::{record_and_check, stored_edge};
use super::merge::merge_after_hash;
use super::source::{AssetSnapshot, Source, load_source};
use super::{ProcError, ProcessingEnv};
use crate::error::CoreError;
use crate::hash::HashedFile;
use crate::jobs::{Job, JobContext, JobError, JobExecutor, JobOutcome, MAX_ATTEMPTS};
use crate::reconcile::{PendingHash, Reconciler};

pub struct HashExecutor {
    env: Arc<ProcessingEnv>,
}

impl HashExecutor {
    pub fn new(env: Arc<ProcessingEnv>) -> Self {
        Self { env }
    }

    /// 해시를 기록하고 병합을 시도한다. 병합돼 사라진 자산의 캐시 산출물은 커밋 뒤에 지운다.
    fn commit(&self, asset_id: i64, hashed: HashedFile) -> Result<(), JobError> {
        let env = &self.env;
        let (clock, hasher) = (Arc::clone(&env.clock), Arc::clone(&env.hasher));
        let merged = env.write(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let never = AtomicBool::new(false);
            let reconciler = Reconciler {
                clock: &*clock,
                hasher: &*hasher,
                cancel: &never,
            };
            let merged = match reconciler.commit_pending_hash(&tx, asset_id, &hashed)? {
                PendingHash::Done => merge_after_hash(&tx, clock.now_ms(), asset_id)?,
                PendingHash::Deferred => None,
            };
            tx.commit()?;
            Ok::<_, ProcError>(merged)
        })?;
        if let Some(m) = merged {
            for size in [ThumbSize::Small, ThumbSize::Large] {
                let _ =
                    std::fs::remove_file(thumbnail_path(&env.cache_root, m.removed_asset_id, size));
            }
            let _ = std::fs::remove_file(strip_path(&env.cache_root, m.removed_asset_id));
        }
        Ok(())
    }

    /// 마지막 시도에서도 실패하면 `hash_state = failed` 로 남긴다 (재시도는 [`super::retry_failed`]).
    fn mark_failed(&self, asset_id: i64) -> Result<(), JobError> {
        let clock = Arc::clone(&self.env.clock);
        self.env.write(move |c| {
            c.execute(
                "UPDATE media_assets SET hash_state = 'failed', updated_at = ?2
                 WHERE asset_id = ?1 AND hash_state = 'pending'",
                rusqlite::params![asset_id, clock.now_ms()],
            )?;
            Ok(())
        })?;
        Ok(())
    }
}

impl HashExecutor {
    /// I/O 오류를 작업 결과로 바꾼다 (해시·체크섬 공통).
    fn io_outcome(&self, job: &Job, asset_id: i64, e: CoreError) -> Result<JobOutcome, JobError> {
        match e {
            CoreError::Cancelled => Ok(JobOutcome::Yielded),
            // 계산 도중 바뀜 / 사라짐: 스캔·감시가 변경을 반영하며 새 작업을 등록한다
            CoreError::ChangedDuringHash(_) => Ok(JobOutcome::Done),
            CoreError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
                Ok(JobOutcome::Done)
            }
            e => {
                if job.attempts + 1 >= MAX_ATTEMPTS {
                    self.mark_failed(asset_id)?;
                    return Ok(JobOutcome::Done);
                }
                Err(JobError::retryable(e.to_string()))
            }
        }
    }

    /// 앞뒤 4K 체크섬을 확보(없으면 계산·저장)하고 같은 크기·같은 sig 상대가 있는지 돌려준다.
    /// `Ok(Err(outcome))` = 여기서 작업을 끝낸다.
    fn edge_collision(
        &self,
        job: &Job,
        ctx: &JobContext<'_>,
        asset_id: i64,
        snap: &AssetSnapshot,
        path: &Path,
    ) -> Result<Result<bool, JobOutcome>, JobError> {
        let env = &self.env;
        let stored = env.read(|c| Ok(stored_edge(c, asset_id)?))?;
        let new_sig = if stored.is_some() {
            None
        } else {
            match env.hasher.edge_signature(path, ctx.shutdown_flag()) {
                Ok(e) => {
                    if i64::try_from(e.size) != Ok(snap.size) || e.modified_ms != snap.modified_at {
                        return Ok(Err(JobOutcome::Done)); // 등록 뒤 파일이 바뀜 - 스캔이 새로 등록한다
                    }
                    Some(*e.sig.as_bytes())
                }
                Err(e) => return Ok(Err(self.io_outcome(job, asset_id, e)?)),
            }
        };
        let (clock, size, modified_at) = (Arc::clone(&env.clock), snap.size, snap.modified_at);
        let checked = env.write(move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let r = record_and_check(&tx, clock.now_ms(), asset_id, size, modified_at, new_sig)?;
            tx.commit()?;
            Ok::<_, ProcError>(r)
        })?;
        Ok(checked.ok_or(JobOutcome::Done))
    }
}

impl JobExecutor for HashExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let asset_id = job
            .asset_id
            .ok_or_else(|| JobError::fatal("hash job without asset"))?;
        let env = &self.env;
        let (snap, path) = match env.read(|c| Ok(load_source(c, asset_id)?))? {
            Source::Ready(snap, path) => (snap, path),
            Source::Inactive | Source::Unavailable => return Ok(JobOutcome::Done),
        };
        if !snap.hash_pending && !snap.hash_done {
            return Ok(JobOutcome::Done); // failed - 수동 재시도(retry_failed)가 pending 으로 되돌린다
        }
        if ctx.should_stop() {
            return Ok(JobOutcome::Yielded);
        }
        let collision = match self.edge_collision(job, ctx, asset_id, &snap, &path)? {
            Ok(collision) => collision,
            Err(outcome) => return Ok(outcome),
        };
        // 이미 전체 해시가 있거나(중복 실행은 멱등), 같은 크기·같은 앞뒤 체크섬 상대가 없으면 전체 해시는 필요 없다
        if !snap.hash_pending || !collision {
            return Ok(JobOutcome::Done);
        }
        match env.hasher.hash_file(&path, ctx.shutdown_flag()) {
            Ok(hashed) => {
                self.commit(asset_id, hashed)?;
                Ok(JobOutcome::Done)
            }
            Err(e) => self.io_outcome(job, asset_id, e),
        }
    }
}
