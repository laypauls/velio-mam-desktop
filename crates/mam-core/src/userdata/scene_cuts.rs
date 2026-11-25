//! 동영상 장면전환점 (`asset_scene_cuts`, V15) — 사용자 편집 경로. 저장소 로직은 `mam_db::scene_cuts` 에 있고
//! 여기서는 자산 검증(활성 영상만)·고우선순위 쓰기 트랜잭션·계약 모양 변환만 한다.
//!
//! 장면전환점은 검색 색인 대상이 아니라 `revision`·`search_outbox` 를 건드리지 않는다. 원본 파일은 읽지도 쓰지도 않는다.

use mam_db::SceneSensitivity;
use mam_db::scene_cuts::{self, CutSource as DbCutSource, SceneError, SceneRow, SceneState as DbState};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::UserData;
use crate::scan::IngestError;

/// 계약 `SceneState` — 행 없음은 `none`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SceneStateView {
    None,
    Pending,
    Done,
    Failed,
    Unsupported,
}

/// 계약 `CutSource`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CutSource {
    Auto,
    User,
}

/// 계약 `SceneFailure` — 고정 코드만 (경로·stderr 는 싣지 않는다)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneFailure {
    ToolNotFound,
    Timeout,
    ToolFailed,
    OutputTooLarge,
    Io,
    Other,
}

impl SceneFailure {
    fn from_code(code: &str) -> Self {
        match code {
            "tool_not_found" => Self::ToolNotFound,
            "timeout" => Self::Timeout,
            "tool_failed" => Self::ToolFailed,
            "output_too_large" => Self::OutputTooLarge,
            "io" => Self::Io,
            _ => Self::Other,
        }
    }
}

/// 계약 `SceneCut`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCut {
    pub ms: i64,
    pub source: CutSource,
}

/// 계약 `SceneCuts` — `cuts` 는 ms 오름차순으로 병합된 자동·사용자 컷
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCuts {
    pub asset_id: i64,
    pub state: SceneStateView,
    pub sensitivity: Option<SceneSensitivity>,
    pub error: Option<SceneFailure>,
    pub cuts: Vec<SceneCut>,
    pub suppressed_count: u32,
    pub updated_at: Option<i64>,
}

impl SceneCuts {
    /// 행이 없는 자산의 빈 상태
    pub fn none(asset_id: i64) -> Self {
        Self {
            asset_id,
            state: SceneStateView::None,
            sensitivity: None,
            error: None,
            cuts: Vec::new(),
            suppressed_count: 0,
            updated_at: None,
        }
    }

    pub fn from_row(asset_id: i64, row: Option<SceneRow>) -> Self {
        let Some(r) = row else {
            return Self::none(asset_id);
        };
        let mut cuts: Vec<SceneCut> = r
            .auto
            .iter()
            .map(|&ms| SceneCut { ms, source: CutSource::Auto })
            .chain(r.user.iter().map(|&ms| SceneCut { ms, source: CutSource::User }))
            .collect();
        // 같은 ms 면 자동이 먼저 (안정 정렬)
        cuts.sort_by_key(|c| c.ms);
        Self {
            asset_id,
            state: match r.state {
                DbState::Pending => SceneStateView::Pending,
                DbState::Done => SceneStateView::Done,
                DbState::Failed => SceneStateView::Failed,
                DbState::Unsupported => SceneStateView::Unsupported,
            },
            sensitivity: r.sensitivity.as_deref().and_then(SceneSensitivity::parse_str),
            error: r.error.as_deref().map(SceneFailure::from_code),
            cuts,
            suppressed_count: u32::try_from(r.suppressed.len()).unwrap_or(u32::MAX),
            updated_at: Some(r.updated_at),
        }
    }
}

impl From<CutSource> for DbCutSource {
    fn from(s: CutSource) -> Self {
        match s {
            CutSource::Auto => Self::Auto,
            CutSource::User => Self::User,
        }
    }
}

impl From<SceneError> for IngestError {
    fn from(e: SceneError) -> Self {
        match e {
            SceneError::InvalidInput(f) => Self::InvalidInput(f),
            SceneError::Sqlite(e) => e.into(),
        }
    }
}

/// 편집 대상 자산의 검증 결과
struct Target {
    duration_ms: Option<i64>,
    fps: Option<f64>,
}

/// 활성 영상만 허용: 없음·비활성 → `NotFound`, 영상 아님 → `InvalidInput("mediaType")`
fn require_active_video(c: &Connection, asset_id: i64) -> Result<Target, IngestError> {
    let row: Option<(String, Option<i64>, Option<f64>)> = c
        .query_row(
            "SELECT a.media_type, m.duration_ms, m.fps FROM media_assets a
             LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
             WHERE a.asset_id = ?1 AND a.status = 'active'",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (media_type, duration_ms, fps) = row.ok_or(IngestError::NotFound)?;
    if media_type != "video" {
        return Err(IngestError::InvalidInput("mediaType"));
    }
    Ok(Target { duration_ms, fps })
}

impl UserData {
    /// 자산의 장면전환점. 없는 자산은 `NotFound`, 행이 없으면 `state: none`.
    pub fn scene_cuts_get(&self, asset_id: i64) -> Result<SceneCuts, IngestError> {
        self.read(|c| {
            let exists: Option<i64> = c
                .query_row(
                    "SELECT asset_id FROM media_assets WHERE asset_id = ?1",
                    [asset_id],
                    |r| r.get(0),
                )
                .optional()?;
            exists.ok_or(IngestError::NotFound)?;
            Ok(SceneCuts::from_row(asset_id, scene_cuts::get(c, asset_id)?))
        })
    }

    /// 사용자 컷 추가 (같은 위치 ±1프레임 안에 이미 있으면 no-op). 갱신된 전체 상태를 돌려준다.
    /// 오류: `not_found`, `invalid_input`("mediaType"·"ms"·"cuts").
    pub fn scene_cut_add(&self, asset_id: i64, ms: i64) -> Result<SceneCuts, IngestError> {
        self.write(move |tx, now| {
            let t = require_active_video(tx, asset_id)?;
            scene_cuts::add_user_tx(tx, asset_id, ms, t.fps, t.duration_ms, now)?;
            Ok(SceneCuts::from_row(asset_id, scene_cuts::get(tx, asset_id)?))
        })
    }

    /// 가장 가까운 컷 1개 제거 (자동 컷은 숨김 목록에 기억). 없으면 no-op. 갱신된 전체 상태를 돌려준다.
    pub fn scene_cut_remove(
        &self,
        asset_id: i64,
        ms: i64,
        source: CutSource,
    ) -> Result<SceneCuts, IngestError> {
        self.write(move |tx, now| {
            let t = require_active_video(tx, asset_id)?;
            scene_cuts::remove_tx(tx, asset_id, ms, source.into(), t.fps, now)?;
            Ok(SceneCuts::from_row(asset_id, scene_cuts::get(tx, asset_id)?))
        })
    }

    /// 자동 컷을 모두 숨김으로 옮긴다 (재추출해도 되살아나지 않는다). 갱신된 전체 상태를 돌려준다.
    pub fn scene_cuts_clear_auto(&self, asset_id: i64) -> Result<SceneCuts, IngestError> {
        self.write(move |tx, now| {
            require_active_video(tx, asset_id)?;
            scene_cuts::clear_auto_tx(tx, asset_id, now)?;
            Ok(SceneCuts::from_row(asset_id, scene_cuts::get(tx, asset_id)?))
        })
    }
}
