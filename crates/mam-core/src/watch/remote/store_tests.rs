//! 접속 저장소 시험 (T-remote-store-01 ~ 10). 실제 SQLite(임시 파일 + 마이그레이션 V1~V23), 비밀 상자는 가짜.
//! 분류: Happy (01) / Validation (02·03) / Business rule (04 삭제 거부·05 비밀 유지·06 비노출·07 커서) / Boundary (08 이름 중복) / External (09 깨진 blob).

use rusqlite::Connection;
use tempfile::TempDir;

use crate::watch::remote::fake::FakeSecretBox;
use crate::watch::remote::store::{self, NewSource, StoreError};
use crate::watch::remote::{Auth, DEFAULT_LOG_DIR, Mapping, SourceKind};
use crate::secret::Secret;
use crate::secret::SecretBox;

const FP: &str = "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn db() -> (TempDir, Connection) {
    let dir = TempDir::new().unwrap();
    let mut conn = mam_db::open(&dir.path().join("mam.sqlite")).unwrap();
    mam_db::migrate(&mut conn, &dir.path().join("backups"), &mam_db::SystemClock).unwrap();
    (dir, conn)
}

fn input(name: &str) -> NewSource {
    NewSource {
        kind: SourceKind::DdpAudit,
        name: name.into(),
        host: "10.0.0.5".into(),
        port: 22,
        user: "audit".into(),
        host_key: Some(FP.into()),
        log_dir: None,
        db_path: None,
        poll_sec: 3,
        path_map: vec![Mapping { from: "/".into(), to: r"F:\UHD".into() }],
    }
}

fn pw(s: &str) -> Auth {
    Auth::Password(Secret::new(s.into()))
}

fn make(conn: &Connection, name: &str) -> i64 {
    store::create(conn, &input(name), &pw("hunter2"), &FakeSecretBox, 1_000).unwrap()
}

#[test]
fn t_remote_store_01_create_list_view_load_roundtrip_and_the_secret_is_decrypted_only_by_load() {
    let (_d, conn) = db();
    let id = make(&conn, "ddp-1");
    let v = store::view(&conn, id).unwrap();
    assert_eq!((v.name.as_str(), v.kind, v.auth_kind, v.has_secret, v.port, v.poll_sec), ("ddp-1", "ddp_audit", "password", true, 22, 3));
    assert_eq!(v.host_key.as_deref(), Some(FP));
    assert_eq!(store::list(&conn).unwrap(), vec![v]);
    let src = store::load(&conn, id, &FakeSecretBox).unwrap();
    assert_eq!(src.auth.secret().expose(), "hunter2");
    assert_eq!(src.path_map[0].to, r"F:\UHD");
    assert_eq!(src.log_dir(), DEFAULT_LOG_DIR);
    assert_eq!(store::kind_of(&conn, id), Some(SourceKind::DdpAudit));
}

#[test]
fn t_remote_store_02_inputs_that_break_the_rules_are_rejected_by_field_name_only() {
    let (_d, conn) = db();
    let bad = |f: &dyn Fn(&mut NewSource), field: &str| {
        let mut i = input("x");
        f(&mut i);
        match store::create(&conn, &i, &pw("p"), &FakeSecretBox, 1) {
            Err(StoreError::Invalid(got)) => assert_eq!(got, field),
            other => panic!("{field}: {other:?}"),
        }
    };
    bad(&|i| i.name = "  ".into(), "name");
    bad(&|i| i.name = "a".repeat(101), "name");
    bad(&|i| i.host = "bad host".into(), "host");
    bad(&|i| i.host = String::new(), "host");
    bad(&|i| i.port = 0, "port");
    bad(&|i| i.user = String::new(), "user");
    bad(&|i| i.poll_sec = 0, "pollSec");
    bad(&|i| i.poll_sec = 3601, "pollSec");
    bad(&|i| i.host_key = Some("MD5:abc".into()), "hostKey");
    bad(&|i| i.host_key = Some("SHA256:short".into()), "hostKey");
    bad(&|i| i.log_dir = Some("relative/dir".into()), "logDir");
    bad(&|i| i.log_dir = Some("/tmp/a\nb".into()), "logDir");
    bad(&|i| i.path_map.clear(), "pathMap");
    bad(&|i| i.path_map[0].to = "relative".into(), "pathMap");
    bad(&|i| i.path_map[0].to = "F:UHD".into(), "pathMap");
    // 종류별 필드: ses_queue 는 db_path 필수·log_dir 금지, ddp_audit 은 db_path 금지
    bad(&|i| i.kind = SourceKind::SesQueue, "dbPath");
    bad(&|i| i.db_path = Some("/srv/ses/events.db".into()), "dbPath");
    // 비밀
    for secret in [pw(""), pw(&"x".repeat(store::MAX_PASSWORD_LEN + 1)), Auth::Key(Secret::new("k".repeat(store::MAX_KEY_LEN + 1)))] {
        assert!(matches!(store::create(&conn, &input("y"), &secret, &FakeSecretBox, 1), Err(StoreError::Invalid("auth"))));
    }
    assert!(store::list(&conn).unwrap().is_empty(), "nothing was stored");
}

#[test]
fn t_remote_store_03_fingerprint_format_is_exactly_sha256_plus_43_base64_chars() {
    assert!(store::valid_fingerprint(FP));
    assert!(store::valid_fingerprint("SHA256:AbCdEfGhIjKlMnOpQrStUvWxYz0123456789+/abcde"));
    for bad in ["", "SHA256:", "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa=", "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "] {
        assert!(!store::valid_fingerprint(bad), "{bad:?}");
    }
}

#[test]
fn t_remote_store_04_a_source_used_by_a_folder_cannot_be_deleted_and_an_unused_one_takes_its_cursor_along() {
    let (_d, conn) = db();
    let id = make(&conn, "ddp-1");
    conn.execute("INSERT INTO volumes (kind, volume_key, last_seen_at) VALUES ('fixed', 'v', 1)", []).unwrap();
    conn.execute(
        "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, watch_mode, watch_options, created_at)
         VALUES (1, 'lib', 'lib', 'F:\\lib', 'ddp_audit', ?1, 1)",
        [format!(r#"{{"source":{id}}}"#)],
    )
    .unwrap_or_else(|e| panic!("fixture: {e}"));
    store::save_cursor(&conn, id, r#"{"offsets":{}}"#, 5).unwrap();
    assert!(matches!(store::delete(&conn, id), Err(StoreError::InUse)));
    assert_eq!(store::referencing_folders(&conn, id).unwrap().len(), 1);
    // 깨진 JSON 행이 다른 접속의 조회를 망치지 않는다
    conn.execute("UPDATE managed_folders SET watch_options = '{not json'", []).unwrap();
    assert!(store::referencing_folders(&conn, id).unwrap().is_empty());
    store::delete(&conn, id).unwrap();
    assert_eq!(store::load_cursor(&conn, id), None, "ON DELETE CASCADE");
    assert!(matches!(store::delete(&conn, id), Err(StoreError::NotFound)));
}

#[test]
fn t_remote_store_05_update_without_a_secret_keeps_the_stored_one_and_with_one_replaces_it() {
    let (_d, conn) = db();
    let id = make(&conn, "ddp-1");
    let mut n = input("renamed");
    n.port = 2222;
    store::update(&conn, id, &n, None, &FakeSecretBox, 2_000).unwrap();
    let s = store::load(&conn, id, &FakeSecretBox).unwrap();
    assert_eq!((s.name.as_str(), s.port, s.auth.secret().expose()), ("renamed", 2222, "hunter2"));
    store::update(&conn, id, &n, Some(&Auth::Key(Secret::new("PEM".into()))), &FakeSecretBox, 3_000).unwrap();
    let s = store::load(&conn, id, &FakeSecretBox).unwrap();
    assert!(matches!(s.auth, Auth::Key(_)) && s.auth.secret().expose() == "PEM");
    assert_eq!(store::view(&conn, id).unwrap().updated_at, 3_000);
    assert!(matches!(store::update(&conn, 999, &n, None, &FakeSecretBox, 1), Err(StoreError::NotFound)));
}

#[test]
fn t_remote_store_06_secrets_are_absent_from_the_database_file_views_serde_debug_and_errors() {
    let (dir, conn) = db();
    let id = store::create(&conn, &input("ddp-1"), &pw("S3cr3t-p@ss"), &FakeSecretBox, 1).unwrap();
    // 행: 평문이 어느 컬럼에도 없다
    let dump: String = conn
        .query_row("SELECT kind||name||host||user||auth_kind||hex(auth_blob)||coalesce(host_key,'')||path_map_json FROM remote_sources", [], |r| r.get(0))
        .unwrap();
    assert!(!dump.contains("S3cr3t"));
    conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", []).ok();
    let raw = std::fs::read(dir.path().join("mam.sqlite")).unwrap();
    assert!(!raw.windows(11).any(|w| w == b"S3cr3t-p@ss"), "not in the database file");
    // 읽기 모델 / serde / Debug
    let v = store::view(&conn, id).unwrap();
    let json = serde_json::to_string(&v).unwrap();
    assert!(!json.contains("S3cr3t") && json.contains("\"hasSecret\":true") && !json.contains("authBlob"));
    let src = store::load(&conn, id, &FakeSecretBox).unwrap();
    assert!(!format!("{src:?}").contains("S3cr3t"));
    // 오류 문구 — 이름 중복·잘못된 입력에도 비밀이 없다
    let e1 = store::create(&conn, &input("ddp-1"), &pw("S3cr3t-p@ss"), &FakeSecretBox, 1).unwrap_err();
    let mut bad = input("z");
    bad.port = 0;
    let e2 = store::create(&conn, &bad, &pw("S3cr3t-p@ss"), &FakeSecretBox, 1).unwrap_err();
    for e in [e1, e2] {
        assert!(!format!("{e}{e:?}").contains("S3cr3t"));
    }
}

#[test]
fn t_remote_store_07_cursor_is_upserted_per_source_and_a_missing_source_violates_the_foreign_key() {
    let (_d, conn) = db();
    let id = make(&conn, "ddp-1");
    assert_eq!(store::load_cursor(&conn, id), None);
    store::save_cursor(&conn, id, "one", 1).unwrap();
    store::save_cursor(&conn, id, "two", 2).unwrap();
    assert_eq!(store::load_cursor(&conn, id).as_deref(), Some("two"));
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM watch_cursors", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
    assert!(store::save_cursor(&conn, 4242, "x", 1).is_err(), "FK: no such source");
}

#[test]
fn t_remote_store_08_names_are_unique_case_sensitive_by_sqlite_and_trimmed() {
    let (_d, conn) = db();
    make(&conn, "ddp-1");
    assert!(matches!(store::create(&conn, &input(" ddp-1 "), &pw("p"), &FakeSecretBox, 1), Err(StoreError::NameTaken)));
    let id2 = make(&conn, "ddp-2");
    assert!(matches!(store::update(&conn, id2, &input("ddp-1"), None, &FakeSecretBox, 1), Err(StoreError::NameTaken)));
}

#[test]
fn t_remote_store_09_a_corrupted_secret_blob_is_an_error_not_a_panic_and_nothing_leaks() {
    let (_d, conn) = db();
    let id = make(&conn, "ddp-1");
    conn.execute("UPDATE remote_sources SET auth_blob = x'00ff'", []).unwrap();
    let e = store::load(&conn, id, &FakeSecretBox).unwrap_err();
    assert!(matches!(e, StoreError::Secret(_)));
    // 목록·요약은 복호하지 않으므로 계속 동작한다
    assert_eq!(store::list(&conn).unwrap().len(), 1);
    let _ = FakeSecretBox.open(b"");
}

#[test]
fn t_remote_store_10_the_schema_rejects_values_the_app_should_never_write() {
    let (_d, conn) = db();
    let bad = |sql: &str| conn.execute(sql, []).is_err();
    let base = "INSERT INTO remote_sources (kind, name, host, port, user, auth_kind, auth_blob, poll_sec, path_map_json, created_at, updated_at)";
    assert!(bad(&format!("{base} VALUES ('ftp', 'a', 'h', 22, 'u', 'password', x'01', 3, '[]', 1, 1)")));
    assert!(bad(&format!("{base} VALUES ('ddp_audit', 'a', 'h', 0, 'u', 'password', x'01', 3, '[]', 1, 1)")));
    assert!(bad(&format!("{base} VALUES ('ddp_audit', 'a', 'h', 22, 'u', 'token', x'01', 3, '[]', 1, 1)")));
    assert!(bad(&format!("{base} VALUES ('ddp_audit', 'a', 'h', 22, 'u', 'password', x'01', 0, '[]', 1, 1)")));
    assert!(!bad(&format!("{base} VALUES ('ses_queue', 'ok', 'h', 22, 'u', 'key', x'01', 3, '[]', 1, 1)")));
}
