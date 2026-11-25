//! 사용자 정의 메타데이터 필드 command 의 직렬화 형태 (계약: `src/lib/ipc-contract-metadata.ts`). 모두 camelCase.
//!
//! 값은 `{ kind, value }` 로 타입을 함께 실어 보낸다 — date 와 enum 이 모두 정수라 값만으로는 구분되지 않는다.
//! date = `YYYYMMDD` 정수, number = f64, enum = optionId, multiEnum = optionId[], keyword = 문자열.

use mam_core::userdata::{AssetFieldValue, FieldOption, FieldPatch, FieldValue, MetadataField, ValueType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum FieldValueDto {
    Keyword(String),
    Number(f64),
    Date(i64),
    Boolean(bool),
    Enum(i64),
    MultiEnum(Vec<i64>),
}

impl From<FieldValue> for FieldValueDto {
    fn from(v: FieldValue) -> Self {
        match v {
            FieldValue::Keyword(s) => Self::Keyword(s),
            FieldValue::Number(n) => Self::Number(n),
            FieldValue::Date(d) => Self::Date(d),
            FieldValue::Boolean(b) => Self::Boolean(b),
            FieldValue::Enum(o) => Self::Enum(o),
            FieldValue::MultiEnum(o) => Self::MultiEnum(o),
        }
    }
}

impl From<FieldValueDto> for FieldValue {
    fn from(v: FieldValueDto) -> Self {
        match v {
            FieldValueDto::Keyword(s) => Self::Keyword(s),
            FieldValueDto::Number(n) => Self::Number(n),
            FieldValueDto::Date(d) => Self::Date(d),
            FieldValueDto::Boolean(b) => Self::Boolean(b),
            FieldValueDto::Enum(o) => Self::Enum(o),
            FieldValueDto::MultiEnum(o) => Self::MultiEnum(o),
        }
    }
}

/// 필드 하나에 대한 변경 — `{ op: "set", fieldId, value }` 또는 `{ op: "clear", fieldId }`.
/// 패치에 없는 필드는 그대로 둔다.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum FieldPatchDto {
    #[serde(rename_all = "camelCase")]
    Set { field_id: i64, value: FieldValueDto },
    #[serde(rename_all = "camelCase")]
    Clear { field_id: i64 },
}

pub fn to_patches(src: Vec<FieldPatchDto>) -> Vec<(i64, FieldPatch)> {
    src.into_iter()
        .map(|p| match p {
            FieldPatchDto::Set { field_id, value } => (field_id, FieldPatch::Set(value.into())),
            FieldPatchDto::Clear { field_id } => (field_id, FieldPatch::Clear),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldOptionView {
    pub option_id: i64,
    pub label: String,
    pub position: i64,
}

impl From<FieldOption> for FieldOptionView {
    fn from(o: FieldOption) -> Self {
        Self { option_id: o.option_id, label: o.label, position: o.position }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataFieldView {
    pub field_id: i64,
    pub name: String,
    pub value_type: ValueType,
    pub position: i64,
    /// enum/multiEnum 만 값이 있다 (표시 순서)
    pub options: Vec<FieldOptionView>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<MetadataField> for MetadataFieldView {
    fn from(f: MetadataField) -> Self {
        Self {
            field_id: f.field_id,
            name: f.name,
            value_type: f.value_type,
            position: f.position,
            options: f.options.into_iter().map(Into::into).collect(),
            created_at: f.created_at,
            updated_at: f.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFieldValueView {
    pub field_id: i64,
    pub value: FieldValueDto,
}

impl From<AssetFieldValue> for AssetFieldValueView {
    fn from(v: AssetFieldValue) -> Self {
        Self { field_id: v.field_id, value: v.value.into() }
    }
}
