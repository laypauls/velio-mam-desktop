// Categories: Validation (계약 모양·오류 코드) / Business rule (정규화·왕복)
use std::path::PathBuf;

use mam_core::ingest::{IngestConfig, IngestDeps};
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, SystemClock, migrate, open};
use serde_json::json;

use super::*;
use crate::error::ErrorCode;

struct Fx {
    _dir: tempfile::TempDir,
    media: PathBuf,
    ingest: Arc<Ingest>,
}

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let data = base.join("MAMData");
    let media = base.join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(media.join("a.jpg"), b"a").unwrap();
    let db = data.join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &data.join("backups"), &SystemClock).unwrap();
    drop(conn);
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::new(DbPool::open(&db, 2).unwrap()),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![data],
            ..IngestConfig::default()
        },
    );
    Fx {
        _dir: dir,
        media,
        ingest,
    }
}

fn sorted_keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

fn add(fx: &Fx) -> i64 {
    fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap().folder_id
}

#[test]
fn t_ipc_detect_01_view_carries_the_six_detection_fields_in_camel_case() {
    // Arrange
    let fx = fixture();
    let id = add(&fx);

    // Act
    let view = fx.ingest.set_folder_watch_mode(id, Some("scan_only".into()), None).unwrap();
    let v = serde_json::to_value(&view).unwrap();

    // Assert
    assert_eq!(v["watchMode"], "scan_only");
    assert_eq!(v["watchActive"], "scan_only");
    for null in ["watchOptions", "watchFallbackKey", "rescanPolicy"] {
        assert!(v[null].is_null(), "{null}");
    }
    assert!(v["nextRescanAt"].is_i64(), "never-scanned folder is due now");
    for k in ["watchMode", "watchOptions", "watchActive", "watchFallbackKey", "rescanPolicy", "nextRescanAt"] {
        assert!(sorted_keys(&v).contains(&k), "{k}");
    }
}

#[test]
fn t_ipc_detect_02_detector_info_has_the_contract_keys_in_registry_order() {
    let fx = fixture();
    let id = add(&fx);

    let infos = serde_json::to_value(fx.ingest.list_change_detectors(id).unwrap()).unwrap();

    let items = infos.as_array().unwrap();
    assert_eq!(items[0]["id"], "os_events");
    assert_eq!(items.last().unwrap()["id"], "scan_only");
    for item in items {
        assert_eq!(sorted_keys(item), ["available", "experimental", "id", "optionFields", "reasonKey"]);
        assert_eq!(item["available"], true);
        assert!(item["reasonKey"].is_null());
        // polling(확인 간격)·ddp_audit·fanotify_feed(원격 접속 참조) 만 실험 표시 + 옵션을 선언한다
        let declares = item["id"] == "polling" || item["id"] == "ddp_audit" || item["id"] == "fanotify_feed";
        assert_eq!(item["experimental"], declares);
        assert_eq!(item["optionFields"].as_array().unwrap().is_empty(), !declares);
        if item["id"] == "ddp_audit" || item["id"] == "fanotify_feed" {
            assert_eq!(item["optionFields"][0]["kind"], "ref");
        }
    }
}

#[test]
fn t_ipc_detect_03_invalid_input_and_missing_folder_map_to_contract_codes() {
    let fx = fixture();
    let id = add(&fx);

    let bad_mode = CommandError::from(fx.ingest.set_folder_watch_mode(id, Some("ghost".into()), None).unwrap_err());
    let bad_opts = CommandError::from(fx.ingest.set_folder_watch_mode(id, None, Some(json!({"a": 1}))).unwrap_err());
    let missing = CommandError::from(fx.ingest.set_folder_watch_mode(999, None, None).unwrap_err());
    let missing_list = CommandError::from(fx.ingest.list_change_detectors(999).unwrap_err());

    assert_eq!(bad_mode.code(), ErrorCode::InvalidInput);
    assert_eq!(bad_opts.code(), ErrorCode::InvalidInput);
    assert_eq!(missing.code(), ErrorCode::NotFound);
    assert_eq!(missing_list.code(), ErrorCode::NotFound);
}

#[test]
fn t_ipc_detect_04_ts_shaped_policy_is_accepted_and_unknown_kind_names_the_field() {
    // Arrange: 프론트 `RescanPolicy` 모양
    let fx = fixture();
    let id = add(&fx);
    let ts = json!({
        "trigger": {"kind": "schedule", "days": ["mon", "fri"], "at": "03:00"},
        "window": {"start": "22:00", "end": "06:00"},
        "onStart": false
    });

    // Act
    let ok = fx.ingest.set_folder_rescan_policy(id, Some(ts.clone())).unwrap();
    let err = CommandError::from(
        fx.ingest
            .set_folder_rescan_policy(id, Some(json!({"trigger": {"kind": "weekly"}})))
            .unwrap_err(),
    );

    // Assert
    assert_eq!(ok.rescan_policy, Some(ts));
    assert_eq!(err.code(), ErrorCode::InvalidInput);
    assert_eq!(serde_json::to_value(&err).unwrap()["message"], "invalid input: rescanPolicy.trigger.kind");
}

#[test]
fn t_ipc_detect_05_list_folders_round_trips_the_saved_settings() {
    let fx = fixture();
    let id = add(&fx);
    fx.ingest.set_folder_rescan_policy(id, Some(json!({"trigger": {"kind": "manual"}, "onStart": true}))).unwrap();
    fx.ingest.set_folder_watch_mode(id, Some("scan_only".into()), None).unwrap();

    let listed = serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap();

    assert_eq!(listed[0]["rescanPolicy"], json!({"trigger": {"kind": "manual"}, "window": null, "onStart": true}));
    assert_eq!(listed[0]["watchMode"], "scan_only");
    assert!(listed[0]["nextRescanAt"].is_i64(), "never scanned -> due now even under manual");
}

#[test]
fn t_ipc_detect_06_auto_mode_takes_the_common_bool_option_and_round_trips_it_in_camel_case() {
    // Arrange: 프론트가 자동 + 복사 완료 확인 끔을 보낸다
    let fx = fixture();
    let id = add(&fx);

    // Act
    let saved = serde_json::to_value(fx.ingest.set_folder_watch_mode(id, None, Some(json!({"exclusiveOpenCheck": false}))).unwrap()).unwrap();
    let bad = CommandError::from(fx.ingest.set_folder_watch_mode(id, None, Some(json!({"intervalSec": 60}))).unwrap_err());

    // Assert: 모드 null, 옵션은 불리언 그대로 (목록에도 같은 값)
    assert!(saved["watchMode"].is_null());
    assert_eq!(saved["watchOptions"], json!({"exclusiveOpenCheck": false}));
    assert_eq!(serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap()[0]["watchOptions"], json!({"exclusiveOpenCheck": false}));
    assert_eq!(bad.code(), ErrorCode::InvalidInput, "detector-specific options stay rejected for auto");
}
