//! 사용자 정의 메타데이터 필드 값 -> Tantivy 문서 (설계: 작업 문서 D2, PoC 금지 규칙 ①~④).
//!
//! 필드 3개로 색인한다 — 사용자 필드마다 스키마를 바꾸지 않기 위함:
//! - `umeta`(JSON, raw, INDEXED|FAST): 키 `f{field_id}` (이름이 아니라 id 라 이름 변경은 재색인이 필요 없다)
//! - `umeta_has`(STRING): 값이 있는 필드마다 `f{field_id}` term
//! - `umeta_text`(ko 분석기): `keyword` 값 (일반 키워드 검색용)
//!
//! 값 형식 고정: keyword = 색인 키 문자열([`keyword_key`]), number = 항상 f64, 나머지 = 항상 i64.
//! (정수처럼 보이는 f64 는 term 질의에서 i64 로 정규화돼 누락되므로 number 는 범위 질의로만 맞춘다.)

use std::collections::BTreeMap;

use tantivy::TantivyDocument;
use tantivy::schema::OwnedValue;
use unicode_normalization::UnicodeNormalization;

use crate::schema::Fields;

/// keyword 색인 키의 최대 글자 수. 입력 검증(200자)이 이미 막지만, 아주 긴 값(7만 자)은 Tantivy 가 조용히 버리므로
/// 색인·질의 양쪽에서 같은 상한을 둔다.
pub const MAX_KEYWORD_CHARS: usize = 200;

/// 사용자 필드 값 하나. 어느 값 컬럼이 채워졌는지로 정해진다 (`value_text` / `value_real` / `value_int`).
#[derive(Clone, Debug, PartialEq)]
pub enum UserValue {
    /// keyword — 원문(NFC). 색인 키는 [`keyword_key`] 로 만든다.
    Keyword(String),
    /// number — f64
    Number(f64),
    /// date(YYYYMMDD) · boolean(0/1) · enum / multi_enum(option_id)
    Int(i64),
}

/// 자산의 사용자 필드 값 한 건. multi_enum 은 선택한 option 마다 한 건씩 같은 `field_id` 로 들어온다.
#[derive(Clone, Debug, PartialEq)]
pub struct UserFieldValue {
    pub field_id: u64,
    pub value: UserValue,
}

/// keyword 색인·질의 공통 정규화: 앞뒤 공백 제거 + NFC + 소문자, [`MAX_KEYWORD_CHARS`] 글자까지. 빈 결과는 "값 없음".
pub fn keyword_key(raw: &str) -> String {
    let key: String = raw.trim().nfc().collect::<String>().to_lowercase();
    match key.char_indices().nth(MAX_KEYWORD_CHARS) {
        Some((end, _)) => key[..end].to_string(),
        None => key,
    }
}

/// 색인 경로 이름 `f{field_id}`.
pub(crate) fn path_of(field_id: u64) -> String {
    format!("f{field_id}")
}

/// `-0.0` 과 `0.0` 은 정렬 가능 변환에서 다른 값이 된다 — 색인·질의 모두 `0.0` 으로 통일한다.
pub(crate) fn canon_f64(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// 사용자 필드 값을 문서에 싣는다. 값이 없는 필드는 아무것도 더하지 않는다.
pub(crate) fn add_user_fields(d: &mut TantivyDocument, f: &Fields, values: &[UserFieldValue]) {
    if values.is_empty() {
        return;
    }
    let mut by_field: BTreeMap<u64, Vec<OwnedValue>> = BTreeMap::new();
    for v in values {
        let json = match &v.value {
            UserValue::Keyword(s) => {
                let key = keyword_key(s);
                if key.is_empty() {
                    continue;
                }
                let text = s.trim().nfc().collect::<String>();
                d.add_text(f.umeta_text, text);
                OwnedValue::Str(key)
            }
            UserValue::Number(x) if x.is_finite() => OwnedValue::F64(canon_f64(*x)),
            UserValue::Number(_) => continue,
            UserValue::Int(x) => OwnedValue::I64(*x),
        };
        by_field.entry(v.field_id).or_default().push(json);
    }
    if by_field.is_empty() {
        return;
    }
    let mut object: BTreeMap<String, OwnedValue> = BTreeMap::new();
    for (field_id, mut vals) in by_field {
        let key = path_of(field_id);
        d.add_text(f.umeta_has, &key);
        let json = if vals.len() == 1 {
            vals.remove(0)
        } else {
            OwnedValue::Array(vals)
        };
        object.insert(key, json);
    }
    d.add_object(f.umeta, object);
}

#[cfg(test)]
mod tests {
    // Categories: Happy (타입별 형식) / Boundary (빈 값·긴 값·-0.0·NaN) / Business rule (정규화 일치)
    use tantivy::schema::Value;

    use super::*;
    use crate::schema::build_schema;

    fn texts(d: &TantivyDocument, f: tantivy::schema::Field) -> Vec<String> {
        d.get_all(f)
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    fn kw(field_id: u64, s: &str) -> UserFieldValue {
        UserFieldValue {
            field_id,
            value: UserValue::Keyword(s.into()),
        }
    }

    #[test]
    fn t_umeta_doc_01_keyword_key_is_nfc_lowercase_trimmed_and_capped() {
        // NFD "é" 와 NFC "é", 대소문자, 앞뒤 공백이 같은 키가 된다
        assert_eq!(keyword_key("  Cafe\u{301}  "), keyword_key("CAF\u{e9}"));
        assert_eq!(keyword_key("Hello"), "hello");
        // 한글 NFD(자모 분리) 와 NFC 가 같은 키
        assert_eq!(keyword_key("\u{1112}\u{1161}\u{11ab}"), "\u{d55c}");
        assert_eq!(keyword_key("   "), "");
        // 상한: 200 글자까지 (바이트가 아니라 글자 단위)
        let long = "가".repeat(MAX_KEYWORD_CHARS + 50);
        assert_eq!(keyword_key(&long).chars().count(), MAX_KEYWORD_CHARS);
    }

    #[test]
    fn t_umeta_doc_02_values_are_written_to_the_three_fields() {
        // Arrange
        let (_, f) = build_schema();
        let values = [
            kw(1, "Alpha"),
            UserFieldValue {
                field_id: 2,
                value: UserValue::Number(3.0),
            },
            UserFieldValue {
                field_id: 3,
                value: UserValue::Int(20261120),
            },
            UserFieldValue {
                field_id: 4,
                value: UserValue::Int(7),
            },
            UserFieldValue {
                field_id: 4,
                value: UserValue::Int(9),
            },
        ];
        let mut d = TantivyDocument::default();

        // Act
        add_user_fields(&mut d, &f, &values);

        // Assert
        assert_eq!(texts(&d, f.umeta_has), ["f1", "f2", "f3", "f4"]);
        assert_eq!(texts(&d, f.umeta_text), ["Alpha"]);
        assert_eq!(d.get_all(f.umeta).count(), 1, "one JSON object");
    }

    #[test]
    fn t_umeta_doc_03_empty_and_invalid_values_add_nothing() {
        // Boundary: 값 없음 / 빈 keyword / 비유한 number
        let (_, f) = build_schema();
        let mut d = TantivyDocument::default();
        add_user_fields(&mut d, &f, &[]);
        add_user_fields(
            &mut d,
            &f,
            &[
                kw(1, "   "),
                UserFieldValue {
                    field_id: 2,
                    value: UserValue::Number(f64::NAN),
                },
                UserFieldValue {
                    field_id: 3,
                    value: UserValue::Number(f64::INFINITY),
                },
            ],
        );
        assert_eq!(d.get_all(f.umeta).count(), 0);
        assert_eq!(d.get_all(f.umeta_has).count(), 0);
        assert_eq!(d.get_all(f.umeta_text).count(), 0);
    }
}
