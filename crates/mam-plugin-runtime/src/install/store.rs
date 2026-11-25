//! 설정 행(`plugin_*:<id>`) 입출력 도우미 — 오류를 `DbError` 로 맞춰 `DbPool::write*` 클로저에서 `?` 로 쓴다.

use std::collections::BTreeSet;

use mam_db::DbError;
use mam_db::plugin_settings::{self as ps, PutError};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::api::Permission;

pub(crate) fn put_row(c: &Connection, prefix: &str, id: &str, raw: &str) -> Result<(), DbError> {
    ps::put(c, prefix, id, raw).map_err(put_err)
}

pub(crate) fn delete_all_rows(c: &Connection, id: &str) -> Result<(), DbError> {
    ps::delete_plugin(c, id).map_err(put_err)
}

fn put_err(e: PutError) -> DbError {
    match e {
        PutError::Db(e) => DbError::Sqlite(e),
        PutError::Id(_) => DbError::Sqlite(rusqlite::Error::InvalidParameterName("plugin id".to_owned())),
    }
}

/// `plugin_installed` 원문 — `{"version":"1.2.0","installedAt":<ms>}`
pub(crate) fn installed_raw(version: &str, installed_at: i64) -> String {
    json!({ "version": version, "installedAt": installed_at }).to_string()
}

/// (version, installedAt). 손상된 문서는 `None`
pub(crate) fn parse_installed(raw: &str) -> Option<(String, i64)> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let version = v.get("version")?.as_str()?.to_owned();
    Some((version, v.get("installedAt").and_then(Value::as_i64).unwrap_or(0)))
}

/// 저장된 권한 문자열 -> 아는 권한만 (모르는 값은 버린다 = 거부 쪽)
pub(crate) fn known_perms(raw: Option<&str>) -> BTreeSet<Permission> {
    raw.map(ps::parse_granted).unwrap_or_default().iter().filter_map(|s| s.parse().ok()).collect()
}

pub(crate) fn granted_raw(perms: &BTreeSet<Permission>) -> String {
    ps::granted_to_raw(&perms.iter().map(|p| p.as_str().to_owned()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Validation(손상 입력은 거부 쪽)

    #[test]
    fn t_pinst_store_01_installed_roundtrip_and_corrupt() {
        let raw = installed_raw("1.2.0", 42);
        assert_eq!(parse_installed(&raw), Some(("1.2.0".to_owned(), 42)));
        assert_eq!(parse_installed("not json"), None);
        assert_eq!(parse_installed(r#"{"installedAt":1}"#), None);
        assert_eq!(parse_installed(r#"{"version":"1.0.0"}"#), Some(("1.0.0".to_owned(), 0)));
    }

    #[test]
    fn t_pinst_store_02_known_perms_drops_unknown_and_corrupt() {
        let perms = known_perms(Some(r#"["kv.read","bogus",7,"tag.write"]"#));
        assert_eq!(perms, BTreeSet::from([Permission::KvRead, Permission::TagWrite]));
        assert!(known_perms(Some("{oops")).is_empty());
        assert!(known_perms(None).is_empty());
        assert_eq!(granted_raw(&perms), r#"["kv.read","tag.write"]"#);
    }
}
