//! 값 검증·정규화 — 순수 함수 (DB 접근 없음). 외부 입력은 모두 악의적이라고 가정한다.

use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;

use super::{FieldValue, ValueType};
use crate::scan::IngestError;

/// 필드 최대 개수 (결정 5). 스키마가 아니라 이 상수 하나로 제한하므로 올릴 때 마이그레이션이 필요 없다.
pub const MAX_METADATA_FIELDS: usize = 20;
/// keyword 값 최대 글자 수 (trim + NFC 후, 유니코드 문자 기준)
pub const MAX_KEYWORD_CHARS: usize = 200;

/// 저장될 값 한 행 (`asset_metadata_values` 의 값 컬럼). 정확히 하나만 `Some`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Row {
    pub ord: i64,
    pub text: Option<String>,
    pub int: Option<i64>,
    pub real: Option<f64>,
}

impl Row {
    fn text(s: String) -> Self {
        Self {
            ord: 0,
            text: Some(s),
            int: None,
            real: None,
        }
    }
    fn int(ord: i64, v: i64) -> Self {
        Self {
            ord,
            text: None,
            int: Some(v),
            real: None,
        }
    }
    fn real(v: f64) -> Self {
        Self {
            ord: 0,
            text: None,
            int: None,
            real: Some(v),
        }
    }
}

/// keyword 정리: trim + NFC. 비면 `None`(= 값 없음). 제어 문자·200자 초과는 `invalid_input`.
fn clean_keyword(raw: &str) -> Result<Option<String>, IngestError> {
    let t: String = raw.trim().nfc().collect();
    if t.is_empty() {
        return Ok(None);
    }
    if t.chars().count() > MAX_KEYWORD_CHARS || t.chars().any(char::is_control) {
        return Err(IngestError::InvalidInput("keyword value"));
    }
    Ok(Some(t))
}

/// `YYYYMMDD` 가 실제 달력에 있는 날짜인가 (연도 1~9999, 윤년 반영)
pub(super) fn is_valid_date(v: i64) -> bool {
    let (y, m, d) = (v / 10_000, (v / 100) % 100, v % 100);
    if v <= 0 || !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 {
        return false;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    d <= days
}

/// 타입에 맞는 저장 행으로 바꾼다. 빈 `Vec` = 값 없음(지우기와 같다).
/// `options` 는 enum/multi_enum 일 때 그 필드의 유효한 option_id 집합.
/// multi_enum 의 중복 선택지는 오류가 아니라 제거(dedup)한다 — 체크 목록 UI 의 중복 전송을 관대하게 받는다.
pub(super) fn to_rows(
    vt: ValueType,
    value: &FieldValue,
    options: &HashSet<i64>,
) -> Result<Vec<Row>, IngestError> {
    match (vt, value) {
        (ValueType::Keyword, FieldValue::Keyword(s)) => {
            Ok(clean_keyword(s)?.map(Row::text).into_iter().collect())
        }
        (ValueType::Number, FieldValue::Number(n)) => {
            if !n.is_finite() {
                return Err(IngestError::InvalidInput("number value"));
            }
            Ok(vec![Row::real(*n)])
        }
        (ValueType::Date, FieldValue::Date(d)) => {
            if !is_valid_date(*d) {
                return Err(IngestError::InvalidInput("date value"));
            }
            Ok(vec![Row::int(0, *d)])
        }
        (ValueType::Boolean, FieldValue::Boolean(b)) => Ok(vec![Row::int(0, i64::from(*b))]),
        (ValueType::Enum, FieldValue::Enum(id)) => {
            if !options.contains(id) {
                return Err(IngestError::InvalidInput("option"));
            }
            Ok(vec![Row::int(0, *id)])
        }
        (ValueType::MultiEnum, FieldValue::MultiEnum(ids)) => {
            if ids.iter().any(|id| !options.contains(id)) {
                return Err(IngestError::InvalidInput("option"));
            }
            let mut ids = ids.clone();
            ids.sort_unstable();
            ids.dedup();
            Ok(ids.into_iter().map(|id| Row::int(id, id)).collect())
        }
        _ => Err(IngestError::InvalidInput("value type mismatch")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_umeta_unit_01_date_validation_covers_calendar_edges() {
        for ok in [20260101, 20240229, 20000229, 20261231, 10101, 99991231] {
            assert!(is_valid_date(ok), "{ok}");
        }
        for bad in [
            0, -20260101, 20260229, 19000229, 20261301, 20260001, 20260100, 20260431, 20260132,
            2026101, 100000101,
        ] {
            assert!(!is_valid_date(bad), "{bad}");
        }
    }
}
