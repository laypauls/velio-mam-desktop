//! params 검증 — 모든 params 는 JSON 객체이고 `deny_unknown_fields`(`pluginId` 같은 추가 필드는 `bad_request`).
//! 메시지는 고정 문자열이며 입력을 되돌려 보내지 않는다.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::errors::GwError;

/// JS 가 정확히 표현하는 최대 정수 (2^53 - 1)
const MAX_SAFE_INT: i64 = (1 << 53) - 1;
pub(crate) const MAX_KEY_CHARS: usize = 128;
pub(crate) const MAX_LIST_LIMIT: i64 = 200;
pub(crate) const DEFAULT_LIST_LIMIT: usize = 50;
pub(crate) const MAX_TAGS_PER_CALL: usize = 20;

/// params 가 객체이고 필드가 정확히 맞아야 한다
pub(crate) fn parse<T: DeserializeOwned>(params: Value) -> Result<T, GwError> {
    if !params.is_object() {
        return Err(GwError::BadRequest("invalid params"));
    }
    serde_json::from_value(params).map_err(|_| GwError::BadRequest("invalid params"))
}

/// 정수 `1..=2^53-1` 만 (실수·문자열·null·0·음수는 거부)
pub(crate) fn asset_id(v: &Value) -> Result<i64, GwError> {
    match v.as_i64() {
        Some(id) if (1..=MAX_SAFE_INT).contains(&id) => Ok(id),
        _ => Err(GwError::BadRequest("invalid asset id")),
    }
}

/// 생략·null = 플러그인 전역 키
pub(crate) fn opt_asset_id(v: &Option<Value>) -> Result<Option<i64>, GwError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => asset_id(v).map(Some),
    }
}

fn text_ok(s: &str) -> bool {
    s.chars().count() <= MAX_KEY_CHARS && !s.chars().any(char::is_control)
}

/// 1~128자, 제어 문자 없음. 바이트 그대로 비교한다(정규화 없음)
pub(crate) fn key(k: &str) -> Result<(), GwError> {
    if k.is_empty() || !text_ok(k) {
        Err(GwError::BadRequest("invalid key"))
    } else {
        Ok(())
    }
}

pub(crate) fn prefix(p: &str) -> Result<(), GwError> {
    if text_ok(p) {
        Ok(())
    } else {
        Err(GwError::BadRequest("invalid prefix"))
    }
}

pub(crate) fn cursor(c: &str) -> Result<(), GwError> {
    if c.is_empty() || !text_ok(c) {
        Err(GwError::BadRequest("invalid cursor"))
    } else {
        Ok(())
    }
}

pub(crate) fn limit(v: &Option<Value>) -> Result<usize, GwError> {
    match v {
        None | Some(Value::Null) => Ok(DEFAULT_LIST_LIMIT),
        Some(v) => match v.as_i64() {
            Some(n) if (1..=MAX_LIST_LIMIT).contains(&n) => {
                Ok(usize::try_from(n).unwrap_or(DEFAULT_LIST_LIMIT))
            }
            _ => Err(GwError::BadRequest("invalid limit")),
        },
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AssetParams {
    pub asset_id: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DerivedParams {
    pub asset_id: Value,
    pub kind: String,
    #[serde(default)]
    pub size: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct KvKeyParams {
    pub key: String,
    #[serde(default)]
    pub asset_id: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct KvSetParams {
    pub key: String,
    pub value: Value,
    #[serde(default)]
    pub asset_id: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct KvListParams {
    #[serde(default)]
    pub asset_id: Option<Value>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct TagAddParams {
    pub asset_id: Value,
    pub tags: Value,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // 분류: Validation / Boundary

    #[test]
    fn t_pgw_params_01_asset_id_accepts_only_safe_positive_integers() {
        assert_eq!(asset_id(&json!(1)).unwrap(), 1);
        assert_eq!(asset_id(&json!((1i64 << 53) - 1)).unwrap(), (1 << 53) - 1);
        for bad in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!(1.0),
            json!("1"),
            json!(null),
            json!(1i64 << 53),
            json!(u64::MAX),
            json!([1]),
            json!(true),
        ] {
            assert!(asset_id(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn t_pgw_params_02_key_prefix_cursor_limit_bounds() {
        assert!(key("a").is_ok() && key(&"가".repeat(128)).is_ok());
        assert!(
            key("").is_err()
                && key(&"a".repeat(129)).is_err()
                && key("a\u{0}b").is_err()
                && key("a\nb").is_err()
        );
        assert!(prefix("").is_ok() && prefix(&"a".repeat(129)).is_err());
        assert!(cursor("").is_err() && cursor("k").is_ok());
        assert_eq!(limit(&None).unwrap(), 50);
        assert_eq!(limit(&Some(json!(200))).unwrap(), 200);
        for bad in [json!(0), json!(201), json!(-1), json!(1.5), json!("5")] {
            assert!(limit(&Some(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn t_pgw_params_03_objects_only_and_no_unknown_fields() {
        assert!(parse::<AssetParams>(json!({"assetId": 1})).is_ok());
        assert!(parse::<AssetParams>(json!({"assetId": 1, "pluginId": "x"})).is_err());
        assert!(parse::<AssetParams>(json!([1])).is_err());
        assert!(parse::<AssetParams>(json!(null)).is_err());
        assert!(parse::<AssetParams>(json!({})).is_err());
        assert!(parse::<KvKeyParams>(json!({"key": "a", "assetId": null})).is_ok());
    }
}
