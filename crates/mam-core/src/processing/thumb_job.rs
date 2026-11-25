//! `thumb` 작업 실행기 — 320/1024 WebP 썸네일을 `cache/thumbnails` 에, 영상은 미리보기 프레임 스트립을 `cache/previews` 에 만든다
//! (D3·D4). 성공해도 파일 존재가 아니라 `asset_derivatives` 상태로 기록한다. 음성은 `unsupported`.

use std::path::Path;
use std::sync::Arc;

use mam_media::{
    MediaError, MediaKind, ThumbSize, ThumbnailOptions, create_frame_strip, create_thumbnails,
    strip_path, thumbnail_path,
};
use rusqlite::{Connection, TransactionBehavior};

use super::derivatives::{DerivKind, DerivState, set_state};
use super::source::{AssetSnapshot, Source, file_is_current, load_source};
use super::{ProcError, ProcessingEnv, Verdict, judge};
use crate::jobs::{Job, JobContext, JobError, JobExecutor, JobOutcome};

pub struct ThumbExecutor {
    env: Arc<ProcessingEnv>,
}

impl ThumbExecutor {
    pub fn new(env: Arc<ProcessingEnv>) -> Self {
        Self { env }
    }
}

/// 자산이 아직 작업 시작 시점과 같은가 (활성 + 크기·수정 시각 동일).
fn still_current(
    conn: &Connection,
    asset_id: i64,
    snap: &AssetSnapshot,
) -> Result<bool, ProcError> {
    let current: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT status, file_size, modified_at FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    Ok(current == Some(("active".to_string(), snap.size, snap.modified_at)))
}

/// 상태를 기록한다. 그 사이 자산이 바뀌었으면 기록하지 않고 `false` (호출자가 산출물을 지운다).
fn commit_state(
    conn: &mut Connection,
    now: i64,
    asset_id: i64,
    snap: &AssetSnapshot,
    kind: DerivKind,
    state: DerivState,
    error: Option<&str>,
) -> Result<bool, ProcError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !still_current(&tx, asset_id, snap)? {
        return Ok(false);
    }
    set_state(&tx, now, asset_id, kind, state, error)?;
    tx.commit()?;
    Ok(true)
}

/// 산출물을 지운다 (결과 폐기 시). 실패해도 무시한다 - 캐시는 언제든 재생성된다.
fn remove_outputs(cache_root: &Path, asset_id: i64) {
    for size in [ThumbSize::Small, ThumbSize::Large] {
        let _ = std::fs::remove_file(thumbnail_path(cache_root, asset_id, size));
    }
    let _ = std::fs::remove_file(strip_path(cache_root, asset_id));
}

impl ThumbExecutor {
    fn record(
        &self,
        asset_id: i64,
        snap: &AssetSnapshot,
        kind: DerivKind,
        state: DerivState,
        error: Option<String>,
    ) -> Result<bool, JobError> {
        let (clock, snap) = (Arc::clone(&self.env.clock), snap.clone());
        Ok(self.env.write(move |c| {
            commit_state(
                c,
                clock.now_ms(),
                asset_id,
                &snap,
                kind,
                state,
                error.as_deref(),
            )
        })?)
    }

    /// 영상 프레임 스트립 (썸네일 성공 뒤). 실패해도 작업은 실패가 아니다 - 스트립 상태만 기록한다.
    fn make_strip(
        &self,
        asset_id: i64,
        snap: &AssetSnapshot,
        path: &Path,
        job: &Job,
    ) -> Result<JobOutcome, JobError> {
        let env = &self.env;
        let duration = env.tools.probe(path).ok().and_then(|p| p.duration_secs);
        let (state, error) =
            match create_frame_strip(&env.tools, path, asset_id, &env.cache_root, duration) {
                Ok(_) => (DerivState::Done, None),
                Err(e) => match judge(&e, job.attempts) {
                    Verdict::Gone => return Ok(JobOutcome::Done),
                    Verdict::Interrupted => return Ok(JobOutcome::Yielded),
                    Verdict::Unsupported(why) => (DerivState::Unsupported, Some(why)),
                    // 일시 오류(부하 시간 초과 등)는 작업을 재시도한다 - 영구 failed 로 굳히지 않는다 (썸네일은 멱등 재생성)
                    Verdict::Retry(msg) => return Err(JobError::retryable(msg)),
                    Verdict::Failed(msg) => (DerivState::Failed, Some(msg)),
                },
            };
        if !self.record(asset_id, snap, DerivKind::Strip, state, error)? {
            let _ = std::fs::remove_file(strip_path(&env.cache_root, asset_id));
        }
        Ok(JobOutcome::Done)
    }
}

impl JobExecutor for ThumbExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let asset_id = job
            .asset_id
            .ok_or_else(|| JobError::fatal("thumb job without asset"))?;
        let env = &self.env;
        let (snap, path) = match env.read(|c| Ok(load_source(c, asset_id)?))? {
            Source::Ready(snap, path) => (snap, path),
            Source::Inactive | Source::Unavailable => return Ok(JobOutcome::Done),
        };
        if snap.kind == MediaKind::Audio {
            self.record(
                asset_id,
                &snap,
                DerivKind::Thumb,
                DerivState::Unsupported,
                None,
            )?;
            return Ok(JobOutcome::Done);
        }
        if ctx.should_stop() {
            return Ok(JobOutcome::Yielded);
        }
        if !file_is_current(&path, &snap) {
            return Ok(JobOutcome::Done);
        }
        match create_thumbnails(
            &env.tools,
            &path,
            asset_id,
            &env.cache_root,
            &ThumbnailOptions::default(),
        ) {
            Ok(_) => {
                if !self.record(asset_id, &snap, DerivKind::Thumb, DerivState::Done, None)? {
                    // 처리 도중 자산이 바뀌었다 (T-proc-06) - 결과를 버린다
                    remove_outputs(&env.cache_root, asset_id);
                    return Ok(JobOutcome::Done);
                }
            }
            Err(e) => return self.on_error(job, asset_id, &snap, &e),
        }
        if snap.kind == MediaKind::Video {
            // 일시정지 요청이 와도 진행 중인 작업은 스트립(짧은 프레임 5장)까지 끝낸다 - 썸네일 재계산을 피한다
            if ctx
                .shutdown_flag()
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(JobOutcome::Yielded);
            }
            return self.make_strip(asset_id, &snap, &path, job);
        }
        Ok(JobOutcome::Done)
    }
}

impl ThumbExecutor {
    fn on_error(
        &self,
        job: &Job,
        asset_id: i64,
        snap: &AssetSnapshot,
        e: &MediaError,
    ) -> Result<JobOutcome, JobError> {
        let (state, msg) = match judge(e, job.attempts) {
            Verdict::Gone => return Ok(JobOutcome::Done),
            Verdict::Interrupted => return Ok(JobOutcome::Yielded),
            Verdict::Retry(msg) => return Err(JobError::retryable(msg)),
            Verdict::Unsupported(why) => (DerivState::Unsupported, Some(why)),
            Verdict::Failed(msg) => (DerivState::Failed, Some(msg)),
        };
        self.record(asset_id, snap, DerivKind::Thumb, state, msg)?;
        Ok(JobOutcome::Done)
    }
}
