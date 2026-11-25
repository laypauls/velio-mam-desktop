//! 사용자 정의 메타데이터 필드 필터 (프론트 `SearchFilters.userFields`) — 직렬화 모양과 현재 정의 대조.
//!
//! JSON 은 `kind` 태그 + camelCase (작업 문서 D3). 조건끼리는 AND. 검색 조정기(`mam-search`)는 구조만 검증하므로
//! 없는·삭제 중인 필드/선택지, 필드 타입과 조건 종류의 불일치는 여기서 `invalid_input` 으로 거부한다.

use serde::{Deserialize, Serialize};

use crate::scan::IngestError;
use crate::userdata::{MetadataField, ValueType};

/// 한 검색에 실을 수 있는 사용자 필드 조건 수 (필드 20개 × 여유) — 악의적 입력이 질의를 부풀리지 못하게 한다
pub const MAX_USER_FIELD_FILTERS: usize = 40;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UserFieldFilter {
    #[serde(rename_all = "camelCase")]
    KeywordAnyOf { field_id: i64, values: Vec<String> },
    #[serde(rename_all = "camelCase")]
    NumberRange {
        field_id: i64,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// `YYYYMMDD`
    #[serde(rename_all = "camelCase")]
    DateRange {
        field_id: i64,
        min: Option<i64>,
        max: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    Bool { field_id: i64, value: bool },
    #[serde(rename_all = "camelCase")]
    EnumAnyOf { field_id: i64, option_ids: Vec<i64> },
    #[serde(rename_all = "camelCase")]
    MultiEnum {
        field_id: i64,
        option_ids: Vec<i64>,
        all: bool,
    },
    #[serde(rename_all = "camelCase")]
    Has { field_id: i64 },
    #[serde(rename_all = "camelCase")]
    Missing { field_id: i64 },
}

impl UserFieldFilter {
    pub fn field_id(&self) -> i64 {
        match self {
            Self::KeywordAnyOf { field_id, .. }
            | Self::NumberRange { field_id, .. }
            | Self::DateRange { field_id, .. }
            | Self::Bool { field_id, .. }
            | Self::EnumAnyOf { field_id, .. }
            | Self::MultiEnum { field_id, .. }
            | Self::Has { field_id }
            | Self::Missing { field_id } => *field_id,
        }
    }

    /// 이 조건이 요구하는 필드 타입 (`Has`/`Missing` 은 모든 타입)
    fn required_type(&self) -> Option<ValueType> {
        match self {
            Self::KeywordAnyOf { .. } => Some(ValueType::Keyword),
            Self::NumberRange { .. } => Some(ValueType::Number),
            Self::DateRange { .. } => Some(ValueType::Date),
            Self::Bool { .. } => Some(ValueType::Boolean),
            Self::EnumAnyOf { .. } => Some(ValueType::Enum),
            Self::MultiEnum { .. } => Some(ValueType::MultiEnum),
            Self::Has { .. } | Self::Missing { .. } => None,
        }
    }

    fn option_ids(&self) -> &[i64] {
        match self {
            Self::EnumAnyOf { option_ids, .. } | Self::MultiEnum { option_ids, .. } => option_ids,
            _ => &[],
        }
    }
}

/// 현재(삭제 중이 아닌) 정의 `fields` 와 조건을 대조한다. 정의는 호출자가 검색당 한 번만 읽어 넘긴다.
pub fn validate_user_field_filters(
    fields: &[MetadataField],
    conds: &[UserFieldFilter],
) -> Result<(), IngestError> {
    if conds.len() > MAX_USER_FIELD_FILTERS {
        return Err(IngestError::InvalidInput("userFields"));
    }
    for cond in conds {
        let field = fields
            .iter()
            .find(|f| f.field_id == cond.field_id())
            .ok_or(IngestError::InvalidInput("userFields: unknown field"))?;
        if cond.required_type().is_some_and(|t| t != field.value_type) {
            return Err(IngestError::InvalidInput("userFields: type mismatch"));
        }
        if cond
            .option_ids()
            .iter()
            .any(|id| !field.options.iter().any(|o| o.option_id == *id))
        {
            return Err(IngestError::InvalidInput("userFields: unknown option"));
        }
    }
    Ok(())
}
