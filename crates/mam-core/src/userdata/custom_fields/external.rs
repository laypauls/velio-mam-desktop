//! 외부 AI 전송용 사용자 필드 `(필드 이름, 표시 값)` 목록 (D5).
//! 값이 있는 필드만, 필드 표시 순서대로. 삭제 중인 필드·선택지는 `load_asset_values`/`load_fields` 가 이미 제외한다.
//! 길이 제한·제어 문자 제거는 전송 계층(`mam_ai::external::SanitizedMeta`)이 한다.

use std::collections::HashMap;

use rusqlite::Connection;

use super::FieldValue;
use super::defs::load_fields;
use super::values::load_asset_values;
use crate::scan::IngestError;

/// 자산 하나의 사용자 필드를 외부 전송용 문자열로 만든다. 쿼리는 값 1회 + 필드·선택지 2회(고정).
/// enum/multi_enum 은 `option_id` 가 아니라 선택지 이름(선택지 표시 순서)으로 내보낸다.
pub fn load_external_user_fields(
    c: &Connection,
    asset_id: i64,
) -> Result<Vec<(String, String)>, IngestError> {
    let values = load_asset_values(c, asset_id)?;
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let fields = load_fields(c)?;
    let by_id: HashMap<i64, &super::MetadataField> =
        fields.iter().map(|f| (f.field_id, f)).collect();
    let mut out = Vec::with_capacity(values.len());
    for v in &values {
        let Some(field) = by_id.get(&v.field_id) else {
            continue;
        };
        let label = |id: i64| {
            field
                .options
                .iter()
                .find(|o| o.option_id == id)
                .map(|o| o.label.clone())
        };
        let text = match &v.value {
            FieldValue::Keyword(s) => s.clone(),
            FieldValue::Number(n) => n.to_string(),
            FieldValue::Date(ymd) => format!(
                "{:04}-{:02}-{:02}",
                ymd / 10_000,
                ymd / 100 % 100,
                ymd % 100
            ),
            FieldValue::Boolean(b) => b.to_string(),
            FieldValue::Enum(id) => match label(*id) {
                Some(l) => l,
                None => continue,
            },
            FieldValue::MultiEnum(ids) => {
                // 선택지 표시 순서(position)로 정렬 — 값 행 순서(ord)와 무관하게 결정적
                let labels: Vec<&str> = field
                    .options
                    .iter()
                    .filter(|o| ids.contains(&o.option_id))
                    .map(|o| o.label.as_str())
                    .collect();
                if labels.is_empty() {
                    continue;
                }
                labels.join(", ")
            }
        };
        out.push((field.name.clone(), text));
    }
    Ok(out)
}
