//! 사용자 정의 메타데이터 필드 (D1) — 필드 정의·선택지·자산별 값의 저장소 계층.
//!
//! - 값 변경은 한 트랜잭션에서 (1) 값 변경, (2) 실제로 바뀐 자산의 `revision` 증가, (3) `search_outbox` 기록을 함께 한다.
//!   outbox priority 는 기존 체계를 따른다: 0 = 사용자 편집(고우선), 1 = 대량·재색인(저우선).
//!   대량 지정은 바뀐 자산 중 앞 [`BULK_OUTBOX_THRESHOLD`](super::BULK_OUTBOX_THRESHOLD)(1,000)건만 0, 나머지는 1 (결정 9).
//!   필드·선택지 삭제로 인한 재색인은 건수와 무관하게 항상 1 (결정 13·17).
//! - 필드 타입 ↔ 값 컬럼 일치, 선택지 소속, 필드 수 상한은 테이블 간 규칙이라 여기서 검증한다. 타입 변경 API 는 없다 (결정 4).
//! - 원본 미디어는 건드리지 않는다.

mod defs;
mod external;
mod options;
mod purge;
mod validate;
mod values;

pub(crate) use defs::load_fields;
pub use external::load_external_user_fields;
pub use purge::METADATA_PURGE_CHUNK;
pub use validate::{MAX_KEYWORD_CHARS, MAX_METADATA_FIELDS};
pub use values::load_asset_values;

use rusqlite::Connection;

use super::{BULK_PRIORITY, bump_assets};
use crate::scan::IngestError;

/// 필드 값 타입 (생성 후 변경 불가)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueType {
    Keyword,
    Number,
    Date,
    Boolean,
    Enum,
    MultiEnum,
}

impl ValueType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Number => "number",
            Self::Date => "date",
            Self::Boolean => "boolean",
            Self::Enum => "enum",
            Self::MultiEnum => "multi_enum",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "keyword" => Self::Keyword,
            "number" => Self::Number,
            "date" => Self::Date,
            "boolean" => Self::Boolean,
            "enum" => Self::Enum,
            "multi_enum" => Self::MultiEnum,
            _ => return None,
        })
    }

    /// 선택지를 갖는 타입인가
    pub fn has_options(self) -> bool {
        matches!(self, Self::Enum | Self::MultiEnum)
    }
}

/// 필드 값. `Enum`/`MultiEnum` 은 `option_id` 를, `Date` 는 `YYYYMMDD` 정수를 담는다.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Keyword(String),
    Number(f64),
    Date(i64),
    Boolean(bool),
    Enum(i64),
    MultiEnum(Vec<i64>),
}

/// 필드 하나에 대한 변경: 값 덮어쓰기 또는 지우기. 패치에 없는 필드는 건드리지 않는다.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldPatch {
    Set(FieldValue),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldOption {
    pub option_id: i64,
    pub label: String,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataField {
    pub field_id: i64,
    pub name: String,
    pub value_type: ValueType,
    pub position: i64,
    pub options: Vec<FieldOption>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 자산 하나의 필드 값 (검색 색인 문서 구성에도 쓴다 — 타입을 함께 돌려준다)
#[derive(Debug, Clone, PartialEq)]
pub struct AssetFieldValue {
    pub field_id: i64,
    pub value_type: ValueType,
    pub value: FieldValue,
}

/// 삭제 요청된 필드·선택지의 `name_key`/`label_key` sentinel — 제어 문자를 포함해 사용자 이름과 충돌하지 않는다
/// (`clean_name` 이 제어 문자를 거부). 삭제 요청 즉시 같은 이름을 다시 쓸 수 있게 한다.
pub(super) fn deleting_key(id: i64) -> String {
    format!("\u{0}del:{id}")
}

/// 필드 존재 확인 + 타입 조회 (없거나 삭제 중이면 `NotFound`)
pub(crate) fn field_type(c: &Connection, field_id: i64) -> Result<ValueType, IngestError> {
    let raw: Option<String> = rusqlite::OptionalExtension::optional(c.query_row(
        "SELECT value_type FROM metadata_fields WHERE field_id = ?1 AND deleting_at IS NULL",
        [field_id],
        |r| r.get(0),
    ))?;
    raw.and_then(|s| ValueType::parse(&s))
        .ok_or(IngestError::NotFound)
}

/// 큰 자산 목록(필드 삭제 시 최대 100만 건)을 한 문장의 JSON 인자로 보내지 않도록 나눠 revision·outbox 를 기록한다.
const BUMP_CHUNK: usize = 20_000;

pub(super) fn bump_chunked(
    tx: &Connection,
    now: i64,
    ids: &[i64],
    priority: i64,
) -> Result<(), IngestError> {
    for chunk in ids.chunks(BUMP_CHUNK) {
        bump_assets(tx, now, chunk, priority)?;
    }
    Ok(())
}

/// 필드·선택지 삭제에 따른 재색인은 건수와 무관하게 저우선순위
pub(super) fn bump_low_priority(tx: &Connection, now: i64, ids: &[i64]) -> Result<(), IngestError> {
    bump_chunked(tx, now, ids, BULK_PRIORITY)
}
