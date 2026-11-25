//! 사용자 데이터 (P2-7) — 태그·메모·즐겨찾기·가상 컬렉션.
//!
//! 모든 변경은 한 SQLite 트랜잭션 안에서 (1) 데이터 변경, (2) 영향받은 자산의 `revision` 증가,
//! (3) `search_outbox` 기록(사용자 편집 = priority 0)을 함께 수행한다. 실제로 값이 바뀐 자산만 revision 을 올린다
//! (같은 태그를 두 번 추가해도 두 번째는 no-op). 원본 파일은 건드리지 않는다.

mod collection_order;
mod collection_queries;
mod collections;
mod custom_fields;
mod notes;
mod play_ranges;
mod plugin_tags;
mod rating;
mod resume;
mod scene_cuts;
mod tags;

pub use collection_order::ReorderTarget;
pub use collection_queries::{CollectionMembership, MEMBERSHIP_SQL, PLAY_QUEUE_SQL, PlayQueueItem};
pub(crate) use custom_fields::load_fields as load_metadata_fields;
pub use custom_fields::{
    AssetFieldValue, FieldOption, FieldPatch, FieldValue, MAX_KEYWORD_CHARS, MAX_METADATA_FIELDS,
    METADATA_PURGE_CHUNK, MetadataField, ValueType, load_asset_values, load_external_user_fields,
};
pub use play_ranges::{
    GET_SQL as PLAY_RANGES_GET_SQL, MAX_PLAY_RANGES, PlayRange, PlayRangeSummary,
    SUMMARY_SQL as PLAY_RANGE_SUMMARY_SQL, min_range_len_ms, normalize_ranges,
};
pub use plugin_tags::{PluginTagError, PluginTagLimits, PluginTagOutcome, add_tags_tx};
pub use rating::{COUNT_SQLS, MAX_RATING};
pub use resume::{
    COUNT_ASSETS_SQL as RESUME_COUNT_ASSETS_SQL, FOLDER_SCOPE_SQL as RESUME_FOLDER_SCOPE_SQL,
    FolderResumeCount, MIN_POSITION_MS as RESUME_MIN_POSITION_MS, ResumePosition,
    ResumeSaveOutcome, ResumeStats,
};
pub use scene_cuts::{CutSource, SceneCut, SceneCuts, SceneFailure, SceneStateView};

use std::sync::Arc;

use mam_db::{DbPool, Priority};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::SharedClock;
use crate::scan::IngestError;

/// 한 호출이 다루는 자산 수 상한 (D2)
pub const MAX_ASSETS_PER_CALL: usize = 10_000;
/// 이름(태그·컬렉션) 최대 글자 수
pub const MAX_NAME_CHARS: usize = 100;
/// 한 호출에서 추가·제거할 수 있는 태그 수
pub const MAX_TAGS_PER_CALL: usize = 100;
/// 콘텐츠 제목 최대 글자 수 (trim 후, 유니코드 문자 기준)
pub const MAX_TITLE_CHARS: usize = 200;
/// 메모 최대 글자 수
pub const MAX_MEMO_CHARS: usize = 10_000;
/// 영향받는 자산이 이 수를 넘는 태그·컬렉션 삭제/이름 변경은 outbox priority 1 (D3)
pub const BULK_OUTBOX_THRESHOLD: usize = 1_000;

const USER_EDIT_PRIORITY: i64 = 0;
const BULK_PRIORITY: i64 = 1;

/// `list_tags` 항목 (계약 `Tag`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagView {
    pub tag_id: i64,
    pub name: String,
    pub asset_count: i64,
}

/// `list_collections` 항목 (계약 `Collection`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionView {
    pub collection_id: i64,
    pub name: String,
    pub asset_count: i64,
    pub created_at: i64,
}

/// `update_asset_tags` 인자 (계약 `UpdateTagsInput`)
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTagsInput {
    pub asset_ids: Vec<i64>,
    /// 태그 이름 — 없으면 만든다
    #[serde(default)]
    pub add: Vec<String>,
    /// 제거할 태그 id
    #[serde(default)]
    pub remove: Vec<i64>,
}

pub struct UserData {
    pool: Arc<DbPool>,
    clock: SharedClock,
}

impl UserData {
    pub fn new(pool: Arc<DbPool>, clock: SharedClock) -> Self {
        Self { pool, clock }
    }

    /// 사용자 편집 = 고우선순위 쓰기. 클로저 안에서 트랜잭션을 열고 커밋까지 끝낸다.
    fn write<T, F>(&self, f: F) -> Result<T, IngestError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Transaction<'_>, i64) -> Result<T, IngestError> + Send + 'static,
    {
        let now = self.clock.now_ms();
        self.pool.write(Priority::High, move |c| {
            Ok((|| {
                let mut tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let out = f(&mut tx, now)?;
                tx.commit()?;
                Ok(out)
            })())
        })?
    }

    fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, IngestError>,
    ) -> Result<T, IngestError> {
        self.pool.read(|c| Ok(f(c)))?
    }
}

/// 표시용 이름: 앞뒤 공백 제거 + NFC. 비어 있거나 너무 길거나 제어 문자가 있으면 `invalid_input`.
pub(crate) fn clean_name(raw: &str) -> Result<String, IngestError> {
    let name: String = raw.trim().nfc().collect();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(IngestError::InvalidInput("name"));
    }
    Ok(name)
}

/// 비교 키: NFC + 소문자 (대소문자·정규화 형태가 달라도 같은 이름)
pub(crate) fn name_key(clean: &str) -> String {
    clean.nfc().collect::<String>().to_lowercase()
}

/// 중복 제거 + 상한 검사. 빈 목록은 그대로 돌려준다 (호출자가 no-op 처리).
pub(crate) fn clean_ids(ids: &[i64]) -> Result<Vec<i64>, IngestError> {
    if ids.len() > MAX_ASSETS_PER_CALL {
        return Err(IngestError::InvalidInput("too many assets"));
    }
    let mut v = ids.to_vec();
    v.sort_unstable();
    v.dedup();
    Ok(v)
}

pub(crate) fn json_ids(ids: &[i64]) -> String {
    let mut s = String::with_capacity(ids.len() * 8 + 2);
    s.push('[');
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&id.to_string());
    }
    s.push(']');
    s
}

/// 모든 자산이 존재하는지 확인한다 (하나라도 없으면 `NotFound` — 호출 전체가 롤백된다).
pub(crate) fn require_assets(tx: &Connection, ids: &[i64]) -> Result<(), IngestError> {
    if ids.is_empty() {
        return Ok(());
    }
    let found: i64 = tx.query_row(
        "SELECT count(*) FROM media_assets WHERE asset_id IN (SELECT value FROM json_each(?1))",
        [json_ids(ids)],
        |r| r.get(0),
    )?;
    if usize::try_from(found).ok() != Some(ids.len()) {
        return Err(IngestError::NotFound);
    }
    Ok(())
}

/// 대량 영향이면 저우선 (D3). 사용자가 직접 고른 자산 편집은 항상 0.
pub(crate) fn bulk_priority(affected: usize) -> i64 {
    if affected > BULK_OUTBOX_THRESHOLD {
        BULK_PRIORITY
    } else {
        USER_EDIT_PRIORITY
    }
}

/// 영향받은 자산의 revision 을 올리고 같은 트랜잭션에 outbox 를 남긴다.
pub(crate) fn bump_assets(
    tx: &Connection,
    now: i64,
    ids: &[i64],
    priority: i64,
) -> Result<(), IngestError> {
    if ids.is_empty() {
        return Ok(());
    }
    let json = json_ids(ids);
    tx.execute(
        "UPDATE media_assets SET revision = revision + 1, updated_at = ?2
         WHERE asset_id IN (SELECT value FROM json_each(?1))",
        params![json, now],
    )?;
    tx.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         SELECT asset_id, revision, 'upsert', ?2, ?3 FROM media_assets
         WHERE asset_id IN (SELECT value FROM json_each(?1))",
        params![json, priority, now],
    )?;
    Ok(())
}

/// `... RETURNING asset_id` (또는 id 만 고르는 SELECT) 문장을 실행해 자산 id 를 모은다.
pub(crate) fn returning_ids(
    tx: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<i64>, IngestError> {
    let mut stmt = tx.prepare_cached(sql)?;
    let rows = stmt.query_map(p, |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 입력 순서를 유지한 중복 제거 + 상한 검사 (컬렉션 추가처럼 요청 순서가 의미 있을 때).
pub(crate) fn dedup_keep_order(ids: &[i64]) -> Result<Vec<i64>, IngestError> {
    if ids.len() > MAX_ASSETS_PER_CALL {
        return Err(IngestError::InvalidInput("too many assets"));
    }
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    Ok(ids.iter().copied().filter(|id| seen.insert(*id)).collect())
}

/// 태그·컬렉션 이름 정제(trim + NFC, ≤100자, 제어 문자 없음) — 호스트 API 게이트웨이가 호출 전에 같은 규칙으로 검사한다.
pub fn clean_tag_name(raw: &str) -> Result<String, IngestError> {
    clean_name(raw)
}

/// 이름 비교 키(NFC + 소문자)
pub fn tag_name_key(clean: &str) -> String {
    name_key(clean)
}
