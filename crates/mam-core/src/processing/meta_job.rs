//! `meta` 작업 실행기 — 메타데이터를 추출해 `media_metadata` 에 저장한다 (D2).
//! 값이 실제로 바뀐 경우에만 `revision` 을 올리고 outbox 이벤트를 남긴다 (같은 작업을 다시 돌려도 이벤트가 늘지 않는다).

use std::sync::Arc;

use mam_media::{MediaKind, MediaMetadata, Playback, extract_metadata};
use rusqlite::{Connection, TransactionBehavior, params};

use super::derivatives::{DerivKind, DerivState, set_state};
use super::source::{AssetSnapshot, Source, file_is_current, load_source};
use super::{ProcError, ProcessingEnv, Verdict, judge};
use crate::CoreError;
use crate::jobs::{Job, JobContext, JobError, JobExecutor, JobOutcome};
use crate::reconcile::bump_revision;

pub struct MetaExecutor {
    env: Arc<ProcessingEnv>,
}

impl MetaExecutor {
    pub fn new(env: Arc<ProcessingEnv>) -> Self {
        Self { env }
    }
}

fn playback_text(p: Playback) -> &'static str {
    match p {
        Playback::Playable => "playable",
        Playback::Conditional => "conditional",
        Playback::NotPlayable => "not_playable",
    }
}

/// 처리 결과를 기록한다. 자산이 그 사이 바뀌었으면 폐기하고 `false`.
fn commit(
    conn: &mut Connection,
    now: i64,
    asset_id: i64,
    snap: &AssetSnapshot,
    m: &MediaMetadata,
) -> Result<bool, ProcError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: Option<(String, i64, i64)> = tx
        .query_row(
            "SELECT status, file_size, modified_at FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    if current != Some(("active".to_string(), snap.size, snap.modified_at)) {
        return Ok(false);
    }
    let extra = m
        .container
        .as_ref()
        .map(|c| serde_json::json!({ "container": c }).to_string());
    // 값이 달라진 경우에만 행이 바뀐다 (IS NOT 은 NULL 도 비교한다)
    let changed = tx.execute(
        "INSERT INTO media_metadata
           (asset_id, width, height, duration_ms, codec, captured_at,
            audio_codec, fps, bitrate, camera, orientation, playback, extra)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (asset_id) DO UPDATE SET
           width = excluded.width, height = excluded.height, duration_ms = excluded.duration_ms,
           codec = excluded.codec, captured_at = excluded.captured_at,
           audio_codec = excluded.audio_codec, fps = excluded.fps, bitrate = excluded.bitrate,
           camera = excluded.camera, orientation = excluded.orientation,
           playback = excluded.playback, extra = excluded.extra
         WHERE (width, height, duration_ms, codec, captured_at, audio_codec, fps, bitrate,
                camera, orientation, playback, extra)
           IS NOT (excluded.width, excluded.height, excluded.duration_ms, excluded.codec,
                   excluded.captured_at, excluded.audio_codec, excluded.fps, excluded.bitrate,
                   excluded.camera, excluded.orientation, excluded.playback, excluded.extra)",
        params![
            asset_id,
            m.width,
            m.height,
            m.duration_ms,
            m.codec,
            m.captured_at_ms,
            m.audio_codec,
            m.fps,
            m.bitrate,
            m.camera,
            m.orientation,
            playback_text(m.playback),
            extra,
        ],
    )?;
    set_state(&tx, now, asset_id, DerivKind::Meta, DerivState::Done, None)?;
    // 새 영상 장면전환점 자동 추출 (설정 켬일 때만, 기본 끔) - 메타 완료와 같은 트랜잭션. 행이 이미 있으면 등록하지 않는다
    if snap.kind == MediaKind::Video
        && mam_db::scene_cuts::load_scene_detect_auto(&tx)?
        && let Err(e) = mam_db::scene_cuts::enqueue_auto_tx(&tx, asset_id, now)
    {
        return Err(ProcError::Core(CoreError::Internal(e.to_string())));
    }
    if changed > 0 {
        bump_revision(&tx, now, asset_id)?;
    }
    tx.commit()?;
    Ok(true)
}

impl JobExecutor for MetaExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let asset_id = job
            .asset_id
            .ok_or_else(|| JobError::fatal("meta job without asset"))?;
        let env = &self.env;
        let (snap, path) = match env.read(|c| Ok(load_source(c, asset_id)?))? {
            Source::Ready(snap, path) => (snap, path),
            Source::Inactive | Source::Unavailable => return Ok(JobOutcome::Done),
        };
        if ctx.should_stop() {
            return Ok(JobOutcome::Yielded);
        }
        // DB 의 크기·수정 시각이 낡았으면 스캔이 곧 갱신하고 새 작업을 등록한다 - 지금 결과는 쓰지 않는다
        if !file_is_current(&path, &snap) {
            return Ok(JobOutcome::Done);
        }
        match extract_metadata(&env.tools, &path, snap.kind) {
            Ok(m) => {
                let clock = Arc::clone(&env.clock);
                env.write(move |c| commit(c, clock.now_ms(), asset_id, &snap, &m))?;
                Ok(JobOutcome::Done)
            }
            Err(e) => match judge(&e, job.attempts) {
                Verdict::Gone => Ok(JobOutcome::Done),
                Verdict::Interrupted => Ok(JobOutcome::Yielded),
                Verdict::Retry(msg) => Err(JobError::retryable(msg)),
                Verdict::Unsupported(msg) | Verdict::Failed(msg) => {
                    let (clock, state) = (Arc::clone(&env.clock), DerivState::Failed);
                    env.write(move |c| {
                        set_state(
                            c,
                            clock.now_ms(),
                            asset_id,
                            DerivKind::Meta,
                            state,
                            Some(&msg),
                        )?;
                        Ok(())
                    })?;
                    Ok(JobOutcome::Done)
                }
            },
        }
    }
}
