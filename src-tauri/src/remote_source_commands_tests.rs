// Categories: Validation (T-ipc-remote-01 계약 모양·02 TS 입력·03 오류 코드) / Business rule (04 접속 시험·05 수정 시 비밀 유지) / 비노출 (01·02·03)
// 가짜 접속기만 쓴다 — 네트워크 없음. TS 쪽 짝: `src/lib/ipc-remote.test.ts` (T-ui-ipc-remote-*).
use std::path::PathBuf;

use mam_core::ingest::{IngestConfig, IngestDeps};
use mam_core::watch::remote::RemoteHub;
use mam_core::watch::remote::fake::{FakeConnector, FakeDdp, FakeSecretBox};
use mam_core::watch::remote::fake_ses::{FakeSes, SES_SCHEMA, insert_event};
use mam_core::watch::remote::hub::HubRefs;
use mam_core::watch::{DdpAudit, DetectorRegistry, FanotifyFeed};
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, SystemClock, migrate, open};
use serde_json::{Value, json};

use super::*;
use crate::error::ErrorCode;

const FP: &str = "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECRET: &str = "pw-never-leaks-ipc";
const SES_DB: &str = "/srv/ses/events.db";

struct Fx {
    _dir: tempfile::TempDir,
    _ses_dir: Option<tempfile::TempDir>,
    media: PathBuf,
    ingest: Arc<Ingest>,
    conn: Arc<FakeConnector>,
}

fn fixture() -> Fx {
    let ddp = FakeDdp::new("/audit");
    ddp.write("audit-log.0", "");
    build(FakeConnector::new(ddp, FP))
}

/// `ses_queue` 시험용 — 진짜 SQLite(SES 스키마) 위의 가짜 호스트에 붙는다. 행 3건(id 1..=3).
fn fixture_ses() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("events.db");
    let c = rusqlite::Connection::open(&file).unwrap();
    c.execute_batch(SES_SCHEMA).unwrap();
    for n in ["a", "b", "c"] {
        insert_event(&c, "create", &format!("/data/incoming/{n}.mov"), false).unwrap();
    }
    let mut fx = build(FakeConnector::ses(FakeSes::new(&file, SES_DB), FP));
    fx._ses_dir = Some(dir);
    fx
}

fn build(fake: FakeConnector) -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let (data, media) = (base.join("MAMData"), base.join("media"));
    std::fs::create_dir_all(&media).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let db = data.join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &data.join("backups"), &SystemClock).unwrap();
    drop(conn);
    let fake = Arc::new(fake);
    let hub = RemoteHub::new(fake.clone());
    let mut config = IngestConfig {
        auto_excludes: vec![data],
        ..IngestConfig::default()
    };
    config.remote = Arc::clone(&hub);
    config.secrets = Arc::new(FakeSecretBox);
    config.detectors = Arc::new(
        DetectorRegistry::platform_default_with(vec![
            Arc::new(DdpAudit::new(Arc::clone(&hub))),
            Arc::new(FanotifyFeed::new(Arc::clone(&hub))),
        ])
        .with_ref_resolver(Arc::new(HubRefs(hub))),
    );
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::new(DbPool::open(&db, 2).unwrap()),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        config,
    );
    Fx {
        _dir: dir,
        _ses_dir: None,
        media,
        ingest,
        conn: fake,
    }
}

/// 프론트(`RemoteSourceInput`)가 보내는 JSON 모양
fn ts_input(fx: &Fx, name: &str, secret: Value, host_key: Value) -> Value {
    json!({
        "kind": "ddp_audit", "name": name, "host": "10.0.0.5", "port": 22, "user": "audit", "authKind": "password",
        "secret": secret, "hostKey": host_key, "logDir": "/audit", "dbPath": null, "pollSec": 3,
        "pathMap": [{"from": "/", "to": fx.media.to_string_lossy()}],
    })
}

fn parse(v: Value) -> RemoteSourceInput {
    serde_json::from_value(v).expect("TS-shaped input")
}

fn sorted_keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

#[test]
fn t_ipc_remote_01_views_and_test_results_have_the_contract_keys_and_never_a_secret() {
    let fx = fixture();
    let view = fx
        .ingest
        .create_remote_source(parse(ts_input(&fx, "ddp-1", json!(SECRET), json!(FP))))
        .unwrap();
    let v = serde_json::to_value(&view).unwrap();
    assert_eq!(
        sorted_keys(&v),
        [
            "authKind",
            "createdAt",
            "dbPath",
            "hasSecret",
            "host",
            "hostKey",
            "kind",
            "logDir",
            "name",
            "pathMap",
            "pollSec",
            "port",
            "sourceId",
            "updatedAt",
            "user"
        ]
    );
    assert_eq!(
        (
            v["kind"].as_str(),
            v["authKind"].as_str(),
            v["hasSecret"].as_bool()
        ),
        (Some("ddp_audit"), Some("password"), Some(true))
    );
    assert_eq!(v["pathMap"][0]["from"], "/");
    assert!(!v.to_string().contains(SECRET));

    let r = fx
        .ingest
        .test_remote_source(
            None,
            parse(ts_input(&fx, "ddp-1", json!(SECRET), json!(FP))),
        )
        .unwrap();
    let t = serde_json::to_value(&r).unwrap();
    assert_eq!(
        sorted_keys(&t),
        [
            "errorKey",
            "fileCount",
            "fingerprint",
            "maxId",
            "ok",
            "rowCount"
        ]
    );
    assert_eq!(
        (
            t["ok"].as_bool(),
            t["fileCount"].as_i64(),
            t["errorKey"].is_null()
        ),
        (Some(true), Some(1), true)
    );
    assert!(!t.to_string().contains(SECRET));
}

#[test]
fn t_ipc_remote_02_ts_shaped_input_deserializes_with_null_or_absent_secret_and_its_debug_hides_the_secret()
 {
    let fx = fixture();
    let with = parse(ts_input(&fx, "a", json!(SECRET), Value::Null));
    assert_eq!(
        (with.port, with.poll_sec, with.host_key.as_deref()),
        (22, 3, None)
    );
    assert!(!format!("{with:?}").contains(SECRET));
    let null_secret = parse(ts_input(&fx, "a", Value::Null, json!(FP)));
    assert!(null_secret.secret.is_none());
    let mut absent = ts_input(&fx, "a", Value::Null, json!(FP));
    absent.as_object_mut().unwrap().remove("secret");
    assert!(parse(absent).secret.is_none());
    // 잘못된 모양은 역직렬화 단계에서 거부 (port 가 문자열)
    let mut bad = ts_input(&fx, "a", json!(SECRET), json!(FP));
    bad["port"] = json!("22");
    assert!(serde_json::from_value::<RemoteSourceInput>(bad).is_err());
}

#[test]
fn t_ipc_remote_03_errors_map_to_contract_codes_with_field_names_only() {
    let fx = fixture();
    let first = fx
        .ingest
        .create_remote_source(parse(ts_input(&fx, "ddp-1", json!(SECRET), json!(FP))))
        .unwrap();
    let code = |e: mam_core::scan::IngestError| CommandError::from(e);
    let dup = code(
        fx.ingest
            .create_remote_source(parse(ts_input(
                &fx,
                "DDP-1".to_lowercase().as_str(),
                json!(SECRET),
                json!(FP),
            )))
            .unwrap_err(),
    );
    let mut bad_port = ts_input(&fx, "x", json!(SECRET), json!(FP));
    bad_port["port"] = json!(70000);
    let bad = code(fx.ingest.create_remote_source(parse(bad_port)).unwrap_err());
    let no_secret = code(
        fx.ingest
            .create_remote_source(parse(ts_input(&fx, "y", Value::Null, json!(FP))))
            .unwrap_err(),
    );
    let missing = code(
        fx.ingest
            .update_remote_source(999, parse(ts_input(&fx, "z", Value::Null, json!(FP))))
            .unwrap_err(),
    );
    let missing_del = code(fx.ingest.delete_remote_source(999).unwrap_err());
    // 쓰는 폴더가 있는 접속은 삭제 거부 (busy)
    let folder = fx
        .ingest
        .add_folder(fx.media.to_str().unwrap())
        .unwrap()
        .folder_id;
    fx.ingest
        .set_folder_watch_mode(
            folder,
            Some("ddp_audit".into()),
            Some(json!({"source": first.source_id})),
        )
        .unwrap();
    let busy = code(fx.ingest.delete_remote_source(first.source_id).unwrap_err());

    assert_eq!(dup.code(), ErrorCode::AlreadyRegistered);
    assert_eq!(
        (bad.code(), no_secret.code()),
        (ErrorCode::InvalidInput, ErrorCode::InvalidInput)
    );
    assert_eq!(
        (missing.code(), missing_del.code()),
        (ErrorCode::NotFound, ErrorCode::NotFound)
    );
    assert_eq!(busy.code(), ErrorCode::Busy);
    let msg = serde_json::to_string(&bad).unwrap();
    assert!(msg.contains("port"), "{msg}");
    for e in [dup, bad, no_secret, missing, missing_del, busy] {
        let s = format!("{e:?}{}", serde_json::to_string(&e).unwrap());
        assert!(!s.contains(SECRET) && !s.contains("10.0.0.5"), "{s}");
    }
}

#[test]
fn t_ipc_remote_04_test_without_a_pinned_host_key_returns_the_fingerprint_and_makes_no_authenticated_connection()
 {
    let fx = fixture();
    let r = fx
        .ingest
        .test_remote_source(
            None,
            parse(ts_input(&fx, "ddp-1", json!(SECRET), Value::Null)),
        )
        .unwrap();
    let t = serde_json::to_value(&r).unwrap();
    assert_eq!(
        (
            t["ok"].as_bool(),
            t["errorKey"].as_str(),
            t["fingerprint"].as_str()
        ),
        (Some(false), Some("host_key_unconfirmed"), Some(FP))
    );
    assert_eq!(fx.conn.connects(), 0);
    // 지문이 다르면 거부 + 서버 지문
    let other = "SHA256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let r = serde_json::to_value(
        fx.ingest
            .test_remote_source(
                None,
                parse(ts_input(&fx, "ddp-1", json!(SECRET), json!(other))),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (r["errorKey"].as_str(), r["fingerprint"].as_str()),
        (Some("host_key_mismatch"), Some(FP))
    );
}

#[test]
fn t_ipc_remote_05_update_without_a_secret_keeps_the_stored_one_so_a_following_test_still_authenticates()
 {
    let fx = fixture();
    let view = fx
        .ingest
        .create_remote_source(parse(ts_input(&fx, "ddp-1", json!(SECRET), json!(FP))))
        .unwrap();
    let mut change = ts_input(&fx, "ddp-1", Value::Null, json!(FP));
    change["port"] = json!(2222);
    let updated = fx
        .ingest
        .update_remote_source(view.source_id, parse(change))
        .unwrap();
    assert_eq!(updated.port, 2222);
    let r = fx
        .ingest
        .test_remote_source(
            Some(view.source_id),
            parse(ts_input(&fx, "ddp-1", Value::Null, json!(FP))),
        )
        .unwrap();
    assert!(r.ok, "the stored secret was reused");
}

/// `ses_queue` 입력 — 로그 폴더 대신 DB 경로
fn ts_ses_input(fx: &Fx, name: &str, db_path: Value) -> Value {
    let mut v = ts_input(fx, name, json!(SECRET), json!(FP));
    v["kind"] = json!("ses_queue");
    v["logDir"] = Value::Null;
    v["dbPath"] = db_path;
    v["pathMap"] = json!([{"from": "/data/incoming", "to": fx.media.to_string_lossy()}]);
    v
}

#[test]
fn t_ipc_remote_06_ses_queue_views_and_test_results_carry_db_path_row_count_and_max_id_and_never_a_secret()
 {
    let fx = fixture_ses();
    let view = fx
        .ingest
        .create_remote_source(parse(ts_ses_input(&fx, "ses-1", json!(SES_DB))))
        .unwrap();
    let v = serde_json::to_value(&view).unwrap();
    assert_eq!(
        (
            v["kind"].as_str(),
            v["dbPath"].as_str(),
            v["logDir"].is_null()
        ),
        (Some("ses_queue"), Some(SES_DB), true)
    );
    assert!(!v.to_string().contains(SECRET));
    let r = fx
        .ingest
        .test_remote_source(None, parse(ts_ses_input(&fx, "ses-1", json!(SES_DB))))
        .unwrap();
    let t = serde_json::to_value(&r).unwrap();
    assert_eq!(
        (
            t["ok"].as_bool(),
            t["rowCount"].as_i64(),
            t["maxId"].as_i64(),
            t["fileCount"].is_null()
        ),
        (Some(true), Some(3), Some(3), true)
    );
    assert!(!t.to_string().contains(SECRET));
    // 없는 DB 경로 = 결과(오류 아님)로 사유 키
    let r = fx
        .ingest
        .test_remote_source(
            None,
            parse(ts_ses_input(&fx, "ses-1", json!("/srv/nowhere.db"))),
        )
        .unwrap();
    assert_eq!((r.ok, r.error_key), (false, Some("db_unreadable")));
}

#[test]
fn t_ipc_remote_07_unsafe_or_missing_db_paths_are_rejected_with_the_field_name_only() {
    let fx = fixture_ses();
    for bad in [
        Value::Null,
        json!("relative.db"),
        json!("/a;rm -rf /"),
        json!("/a/'b"),
        json!("/a/$(id)"),
        json!(
            "/a/b
c"
        ),
        json!("/a/../b"),
    ] {
        let e = CommandError::from(
            fx.ingest
                .create_remote_source(parse(ts_ses_input(&fx, "x", bad.clone())))
                .unwrap_err(),
        );
        assert_eq!(e.code(), ErrorCode::InvalidInput, "{bad}");
        let s = serde_json::to_string(&e).unwrap();
        assert!(
            s.contains("dbPath") && !s.contains(SECRET) && !s.contains("rm -rf"),
            "{s}"
        );
    }
    assert!(
        fx.ingest.list_remote_sources().unwrap().is_empty(),
        "nothing stored"
    );
}
