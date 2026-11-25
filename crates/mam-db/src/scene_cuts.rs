//! 동영상 장면전환점 저장소 (`asset_scene_cuts`, V15) — 자산당 1행, 컷 목록은 u32 LE BLOB (설계 §1·§4.2).
//!
//! 컷은 검색 색인 대상이 아니므로 자산 `revision`·`search_outbox` 를 건드리지 않는다. 컷 배열은 항상 통째로 읽고
//! 쓴다. 자동 컷(`auto_cuts`)·사용자 컷(`user_cuts`)·숨김(`suppressed`, 사용자가 지운 자동 컷의 기억)은 서로
//! 독립 열이라 재추출이 사용자 편집을 지우지 않는다. 모든 `*_tx` 함수는 호출자가 연 트랜잭션 안에서 실행한다.
//! 원본 미디어와 무관한 MAM 메타데이터라 원본 파일은 읽지도 쓰지도 않는다.

mod codec;
mod jobs;

pub use codec::{
    CODEC_MAX_CUTS, CODEC_VERSION, DecodeError, EDGE_MARGIN_MS, MAX_AUTO_CUTS, MAX_SUPPRESSED,
    MAX_USER_CUTS, MIN_GAP_MS, decode, encode, half_frame_ms, select_auto, tolerance_ms,
};
pub use jobs::{
    BACKFILL_BATCH, BACKFILL_COUNT_SQL, BACKFILL_PAGE_SQL, BackfillBatch, REQUEUE_SQL,
    RequestOptions, RequestOutcome, backfill_batch_tx, backfill_count, enqueue_auto_tx,
    request_tx, requeue_pending_tx,
};

use codec::{has_within, nearest_within};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::view_options::SceneSensitivity;

/// `jobs.kind` 문자열 (`JobKind::Scene.as_str()` 과 같아야 한다 — mam-core 쪽 테스트가 고정)
pub const SCENE_JOB_KIND: &str = "scene";
/// 사용자 요청(우클릭·플레이어 버튼) 작업 우선순위 — 썸네일·메타데이터·AI 앞 (D11)
pub const SCENE_PRIORITY_USER: i64 = 45;
/// 백필·자동 등록 우선순위 — AI(50) 뒤
pub const SCENE_PRIORITY_BACKGROUND: i64 = 60;
/// 검출 명령·임계값 대응이 바뀌면 올린다 (결과 지문)
pub const SCENE_DETECTOR_VERSION: i64 = 2;

/// 설정 키 (`settings.key`)
pub const SETTING_DETECT_AUTO: &str = "scene_detect_auto";
pub const SETTING_SENSITIVITY: &str = "scene_detect_sensitivity";

/// 저장된 `error` 고정 코드 — 경로·stderr 는 저장하지 않는다
const ERROR_CODES: [&str; 5] = [
    "tool_not_found",
    "timeout",
    "tool_failed",
    "output_too_large",
    "io",
];

/// 행 상태 (행 없음은 호출자가 `none` 으로 취급)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneState {
    Pending,
    Done,
    Failed,
    Unsupported,
}

impl SceneState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// 컷 출처
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutSource {
    Auto,
    User,
}

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    /// 잘못된 입력 (필드 이름만)
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// `asset_scene_cuts` 한 행 (컷 열은 디코드됨 — 손상 열은 `[]`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneRow {
    pub state: SceneState,
    pub sensitivity: Option<String>,
    pub detector_version: Option<i64>,
    pub requested_sensitivity: Option<String>,
    pub auto: Vec<i64>,
    pub user: Vec<i64>,
    pub suppressed: Vec<i64>,
    pub error: Option<String>,
    pub updated_at: i64,
}

fn decode_col(asset_id: i64, column: &'static str, blob: Option<Vec<u8>>) -> Vec<i64> {
    match decode(blob.as_deref()) {
        Ok(v) => v,
        Err(e) => {
            // 값은 싣지 않는다. 다음 쓰기가 정상 형식으로 덮어쓴다
            tracing::warn!(asset_id, column, error = %e, "corrupt scene cut blob; treating as empty");
            Vec::new()
        }
    }
}

/// 자산의 장면 행 (없으면 `None`). 디코드 실패 열은 `[]` + 경고.
pub fn get(conn: &Connection, asset_id: i64) -> Result<Option<SceneRow>, rusqlite::Error> {
    conn.prepare_cached(
        "SELECT state, sensitivity, detector_version, requested_sensitivity,
                auto_cuts, user_cuts, suppressed, error, updated_at
         FROM asset_scene_cuts WHERE asset_id = ?1",
    )?
    .query_row([asset_id], |r| {
        let state: String = r.get(0)?;
        Ok(SceneRow {
            state: SceneState::parse(&state).unwrap_or(SceneState::Failed),
            sensitivity: r.get(1)?,
            detector_version: r.get(2)?,
            requested_sensitivity: r.get(3)?,
            auto: decode_col(asset_id, "auto_cuts", r.get(4)?),
            user: decode_col(asset_id, "user_cuts", r.get(5)?),
            suppressed: decode_col(asset_id, "suppressed", r.get(6)?),
            error: r.get(7)?,
            updated_at: r.get(8)?,
        })
    })
    .optional()
}

fn blob(cuts: &[i64]) -> Result<Option<Vec<u8>>, SceneError> {
    encode(cuts).map_err(|_| SceneError::InvalidInput("cuts"))
}

fn sorted_unique(mut v: Vec<i64>) -> Vec<i64> {
    v.sort_unstable();
    v.dedup();
    v
}

/// 자산이 삭제 표시(`active` 아님)된 상태인가 - 폴더 제거 뒤 늦게 끝난 검출 작업이 장면 행을 되살리지 않게 한다.
/// `media_assets` 행이 없으면(단위 시험의 빈 스키마) 막지 않는다.
fn is_inactive(conn: &Connection, asset_id: i64) -> Result<bool, SceneError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM media_assets WHERE asset_id = ?1 AND status <> 'active')",
        [asset_id],
        |r| r.get(0),
    )?)
}

/// 자동 검출 결과로 `auto_cuts` 를 교체한다. 자산이 `active` 가 아니면 아무것도 쓰지 않고 0. 커밋 시점의 행을 다시 읽어 숨김·사용자 컷 ±허용 오차 안의 점을 뺀다
/// (검출 중 사용자가 편집해도 보존). `state='done'`, 지문 기록, `requested_sensitivity`·`error` 비움. 저장한 컷 수를 반환.
pub fn replace_auto_tx(
    conn: &Connection,
    asset_id: i64,
    auto: &[i64],
    sensitivity: &str,
    fps: Option<f64>,
    now: i64,
) -> Result<usize, SceneError> {
    if SceneSensitivity::parse_str(sensitivity).is_none() {
        return Err(SceneError::InvalidInput("sensitivity"));
    }
    if is_inactive(conn, asset_id)? {
        return Ok(0);
    }
    let row = get(conn, asset_id)?;
    let (user, suppressed) = row.map(|r| (r.user, r.suppressed)).unwrap_or_default();
    let tol = tolerance_ms(fps);
    let mut kept: Vec<i64> = sorted_unique(auto.to_vec())
        .into_iter()
        .filter(|&ms| !has_within(&suppressed, ms, tol) && !has_within(&user, ms, tol))
        .collect();
    kept.truncate(MAX_AUTO_CUTS);
    conn.prepare_cached(
        "INSERT INTO asset_scene_cuts
           (asset_id, state, sensitivity, detector_version, auto_cuts, updated_at)
         VALUES (?1, 'done', ?2, ?3, ?4, ?5)
         ON CONFLICT(asset_id) DO UPDATE SET
           state = 'done', sensitivity = excluded.sensitivity,
           detector_version = excluded.detector_version, requested_sensitivity = NULL,
           auto_cuts = excluded.auto_cuts, error = NULL, updated_at = excluded.updated_at",
    )?
    .execute(params![
        asset_id,
        sensitivity,
        SCENE_DETECTOR_VERSION,
        blob(&kept)?,
        now
    ])?;
    Ok(kept.len())
}

/// 고정 오류 코드로 정규화한다 (모르는 값 → `"other"`)
pub fn normalize_error(code: &str) -> &'static str {
    ERROR_CODES
        .iter()
        .copied()
        .find(|c| *c == code)
        .unwrap_or("other")
}

/// 실패·미지원으로 끝낸다 (기존 컷은 유지). `outcome` 은 `Failed`/`Unsupported` 만 허용.
pub fn set_outcome_tx(
    conn: &Connection,
    asset_id: i64,
    outcome: SceneState,
    error: Option<&str>,
    now: i64,
) -> Result<(), SceneError> {
    if !matches!(outcome, SceneState::Failed | SceneState::Unsupported) {
        return Err(SceneError::InvalidInput("state"));
    }
    if is_inactive(conn, asset_id)? {
        return Ok(()); // 삭제 표시된 자산에는 쓰지 않는다
    }
    conn.prepare_cached(
        "INSERT INTO asset_scene_cuts (asset_id, state, error, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(asset_id) DO UPDATE SET
           state = excluded.state, error = excluded.error, requested_sensitivity = NULL,
           updated_at = excluded.updated_at",
    )?
    .execute(params![
        asset_id,
        outcome.as_str(),
        error.map(normalize_error),
        now
    ])?;
    Ok(())
}

fn save_cuts(
    conn: &Connection,
    asset_id: i64,
    row: &SceneRow,
    now: i64,
) -> Result<(), SceneError> {
    conn.prepare_cached(
        "UPDATE asset_scene_cuts SET auto_cuts = ?2, user_cuts = ?3, suppressed = ?4, updated_at = ?5
         WHERE asset_id = ?1",
    )?
    .execute(params![
        asset_id,
        blob(&row.auto)?,
        blob(&row.user)?,
        blob(&row.suppressed)?,
        now
    ])?;
    Ok(())
}

/// 숨김 목록에 합친다. 상한을 넘으면 새로 숨기는 점을 우선하고 오래된(시각이 앞선) 점부터 버린다.
fn merge_suppressed(old: &[i64], added: &[i64]) -> Vec<i64> {
    let added = sorted_unique(added.to_vec());
    let room = MAX_SUPPRESSED.saturating_sub(added.len());
    let mut all: Vec<i64> = old
        .iter()
        .copied()
        .filter(|x| added.binary_search(x).is_err())
        .take(room)
        .collect();
    all.extend(added.iter().take(MAX_SUPPRESSED));
    sorted_unique(all)
}

/// 사용자 컷 추가. `0 <= ms <= duration`(길이를 알 때)·`<= u32::MAX`, 사용자 5,000개 상한.
/// 기존 컷(자동·사용자)의 ±허용 오차 안이면 no-op(`false`). 행이 없으면 `state='done'`, `sensitivity=NULL` 로 만든다.
pub fn add_user_tx(
    conn: &Connection,
    asset_id: i64,
    ms: i64,
    fps: Option<f64>,
    duration_ms: Option<i64>,
    now: i64,
) -> Result<bool, SceneError> {
    let too_late = duration_ms.filter(|d| *d > 0).is_some_and(|d| ms > d);
    if ms < 0 || ms > i64::from(u32::MAX) || too_late {
        return Err(SceneError::InvalidInput("ms"));
    }
    let tol = tolerance_ms(fps);
    let Some(mut row) = get(conn, asset_id)? else {
        conn.prepare_cached(
            "INSERT INTO asset_scene_cuts (asset_id, state, user_cuts, updated_at)
             VALUES (?1, 'done', ?2, ?3)",
        )?
        .execute(params![asset_id, blob(&[ms])?, now])?;
        return Ok(true);
    };
    if has_within(&row.user, ms, tol) || has_within(&row.auto, ms, tol) {
        return Ok(false);
    }
    if row.user.len() >= MAX_USER_CUTS {
        return Err(SceneError::InvalidInput("cuts"));
    }
    let pos = row.user.partition_point(|&x| x < ms);
    row.user.insert(pos, ms);
    save_cuts(conn, asset_id, &row, now)?;
    Ok(true)
}

/// `source` 목록에서 `ms` ±허용 오차의 가장 가까운 컷 1개를 지운다. 자동 컷은 숨김 목록에 기억한다.
/// 지웠으면 `true`, 없으면 no-op(`false`).
pub fn remove_tx(
    conn: &Connection,
    asset_id: i64,
    ms: i64,
    source: CutSource,
    fps: Option<f64>,
    now: i64,
) -> Result<bool, SceneError> {
    let Some(mut row) = get(conn, asset_id)? else {
        return Ok(false);
    };
    let tol = tolerance_ms(fps);
    match source {
        CutSource::User => {
            let Some(i) = nearest_within(&row.user, ms, tol) else {
                return Ok(false);
            };
            row.user.remove(i);
        }
        CutSource::Auto => {
            let Some(i) = nearest_within(&row.auto, ms, tol) else {
                return Ok(false);
            };
            let gone = row.auto.remove(i);
            row.suppressed = merge_suppressed(&row.suppressed, &[gone]);
        }
    }
    save_cuts(conn, asset_id, &row, now)?;
    Ok(true)
}

/// 자동 컷을 모두 숨김으로 옮기고 `auto_cuts` 를 비운다 (D10 "모두 삭제"). 지운 개수를 반환.
pub fn clear_auto_tx(conn: &Connection, asset_id: i64, now: i64) -> Result<usize, SceneError> {
    let Some(mut row) = get(conn, asset_id)? else {
        return Ok(0);
    };
    let n = row.auto.len();
    if n == 0 {
        return Ok(0);
    }
    row.suppressed = merge_suppressed(&row.suppressed, &row.auto);
    row.auto.clear();
    save_cuts(conn, asset_id, &row, now)?;
    Ok(n)
}

fn load_setting(conn: &Connection, key: &str) -> Result<Option<Value>, rusqlite::Error> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

/// 새 영상 자동 추출 설정 (키 1개만 읽는다, 없음·잘못된 값 → `false`)
pub fn load_scene_detect_auto(conn: &Connection) -> Result<bool, rusqlite::Error> {
    Ok(matches!(
        load_setting(conn, SETTING_DETECT_AUTO)?,
        Some(Value::Bool(true))
    ))
}

/// 검출 민감도 설정 (없음·잘못된 값 → `Medium`)
pub fn load_scene_sensitivity(conn: &Connection) -> Result<SceneSensitivity, rusqlite::Error> {
    Ok(load_setting(conn, SETTING_SENSITIVITY)?
        .as_ref()
        .and_then(SceneSensitivity::parse)
        .unwrap_or_default())
}
