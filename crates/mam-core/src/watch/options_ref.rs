//! 참조 옵션(`OptionKind::Ref`) — 감지기 옵션이 다른 레코드(원격 접속 등)의 ID 를 가리킬 때의 검증.
//!
//! 존재 확인은 주입된 [`RefResolver`] 가 한다 (원격 접속 테이블은 후속 단계라, 해석기가 없으면 모든 참조가 "알 수 없음").

use serde_json::Value;

use super::detector::DetectError;

/// 참조 대상이 있는지 묻는다. DB 조회를 하는 구현은 가볍게 유지한다 (옵션 저장 경로에서만 불린다).
pub trait RefResolver: Send + Sync {
    /// `field` = 옵션 키 (예: `source`), `id` = 가리키는 ID
    fn exists(&self, field: &str, id: i64) -> bool;
}

/// 해석기가 없는 기본값 — 모든 참조를 거부한다.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRefs;

impl RefResolver for NoRefs {
    fn exists(&self, _field: &str, _id: i64) -> bool {
        false
    }
}

/// 값이 양의 정수 ID 이고 해석기가 아는 ID 인지 확인해 ID 를 돌려준다.
/// 형식이 틀리면(문자열·실수·불리언·null·객체·0 이하) `InvalidOptions`, 모르는 ID 면 `UnknownReference`.
pub(super) fn check_ref(key: &str, value: &Value, refs: &dyn RefResolver) -> Result<i64, DetectError> {
    const FIELD: &str = "watchOptions";
    let id = value
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or(DetectError::InvalidOptions { field: FIELD })?;
    if refs.exists(key, id) {
        Ok(id)
    } else {
        Err(DetectError::UnknownReference { field: FIELD })
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (01) / Validation (02 모르는 ID·03 잘못된 형) / Business rule (04 해석기 없음 = 거부, 05 기본값 없이 저장)
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;
    use crate::watch::detector::{OptionField, OptionKind, parse_integer_options, parse_options_checked};

    const FIELDS: &[OptionField] = &[
        OptionField { key: "source", kind: OptionKind::Ref, min: 1, max: i64::MAX, default: 0 },
        OptionField { key: "intervalSec", kind: OptionKind::Integer, min: 10, max: 3600, default: 300 },
    ];

    struct Known(HashSet<(&'static str, i64)>);

    impl RefResolver for Known {
        fn exists(&self, field: &str, id: i64) -> bool {
            self.0.iter().any(|(f, i)| *f == field && *i == id)
        }
    }

    fn known() -> Known {
        Known(HashSet::from([("source", 7)]))
    }

    #[test]
    fn t_optref_01_a_known_id_is_accepted_and_stored_next_to_other_options() {
        // Arrange / Act
        let o = parse_options_checked(FIELDS, Some(&json!({"source": 7, "intervalSec": 60})), &known()).unwrap();

        // Assert
        assert_eq!(o.get_i64("source"), Some(7));
        assert_eq!(o.get_i64("intervalSec"), Some(60));
    }

    #[test]
    fn t_optref_02_an_unknown_id_is_rejected_as_unknown_reference_not_as_a_format_error() {
        for id in [8, 1, i64::MAX] {
            let err = parse_options_checked(FIELDS, Some(&json!({"source": id})), &known());
            assert!(matches!(err, Err(DetectError::UnknownReference { .. })), "{id}");
        }
        // 필드 키까지 일치해야 한다 (다른 키의 7 번은 이 옵션의 참조가 아니다)
        let other = Known(HashSet::from([("target", 7)]));
        assert!(matches!(
            parse_options_checked(FIELDS, Some(&json!({"source": 7})), &other),
            Err(DetectError::UnknownReference { .. })
        ));
    }

    #[test]
    fn t_optref_03_wrong_types_and_non_positive_ids_are_format_errors_and_never_reach_the_resolver() {
        struct Panics;
        impl RefResolver for Panics {
            fn exists(&self, _: &str, _: i64) -> bool {
                panic!("resolver must not be asked about malformed values");
            }
        }
        for bad in [json!("7"), json!(7.5), json!(true), json!(null), json!({"id": 7}), json!([7]), json!(0), json!(-3)] {
            let err = parse_options_checked(FIELDS, Some(&json!({"source": bad.clone()})), &Panics);
            assert!(matches!(err, Err(DetectError::InvalidOptions { field: "watchOptions" })), "{bad}");
        }
    }

    #[test]
    fn t_optref_04_without_a_resolver_every_reference_is_unknown_and_options_without_one_still_parse() {
        // Act / Assert: 기본 경로(`parse_integer_options`)에는 해석기가 없다
        let err = parse_integer_options(FIELDS, Some(&json!({"source": 7})));
        assert!(matches!(err, Err(DetectError::UnknownReference { .. })));
        // 참조 옵션을 안 쓰면 영향 없음 (기본값·None)
        assert!(parse_integer_options(FIELDS, None).is_ok());
        assert!(parse_integer_options(FIELDS, Some(&json!({"intervalSec": 60}))).is_ok());
    }

    #[test]
    fn t_optref_05_a_reference_has_no_default_so_it_is_always_stored() {
        let o = parse_options_checked(FIELDS, Some(&json!({"source": 7})), &known()).unwrap();
        assert_eq!(o.to_stored().as_deref(), Some(r#"{"source":7}"#));
    }
}
