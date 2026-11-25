//! 폴더별 변경 감지 방식·정책 API (T-fdet-01 ~ 08).
//!
//! 분류: Happy (01, 04) / Validation (02) / Boundary (03 없는 폴더, 05 오프라인) / Concurrency (06 방식 변경 중 꺼내기) /
//! Business rule (07 원본 보호, 08 이전 후 유지). 실제 SQLite + 수동 시계 + 가짜 볼륨.
#![cfg(windows)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::env::{Env, snapshot};
use mam_core::VolumeKind;
use mam_core::scan::{IngestError, ManagedFolderView};
use serde_json::{Value, json};

fn folder(env: &Env) -> i64 {
    env.write("lib/a.jpg", 1);
    env.ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id
}

fn columns(env: &Env) -> (Option<String>, Option<String>, Option<String>) {
    env.read(|c| {
        c.query_row(
            "SELECT watch_mode, watch_options, rescan_policy FROM managed_folders",
            [],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap(), r.get(2).unwrap())),
        )
        .unwrap()
    })
}

fn invalid(r: Result<ManagedFolderView, IngestError>) -> &'static str {
    match r {
        Err(IngestError::InvalidInput(f)) => f,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn t_fdet_01_scan_only_replaces_the_running_watch_and_is_reported() {
    // Arrange
    let env = Env::new();
    let id = folder(&env);
    let before = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(
        (before.watch_active.as_deref(), before.watch_mode.clone()),
        (Some("os_events"), None)
    );

    // Act
    let view = env
        .ingest
        .set_folder_watch_mode(id, Some("scan_only".into()), None)
        .unwrap();

    // Assert
    assert_eq!(view.watch_mode.as_deref(), Some("scan_only"));
    assert_eq!(view.watch_active.as_deref(), Some("scan_only"));
    assert_eq!(view.watch_fallback_key, None);
    assert!(env.ingest.is_watching(id));
    let infos = env.ingest.list_change_detectors(id).unwrap();
    assert_eq!(
        infos.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![
            "os_events",
            "polling",
            "ddp_audit",
            "fanotify_feed",
            "scan_only"
        ]
    );
    assert!(infos.iter().all(|i| i.available
        && i.reason_key.is_none()
        && i.option_fields.is_empty()
            != matches!(i.id, "polling" | "ddp_audit" | "fanotify_feed")));

    // 자동으로 되돌리면 다시 os_events
    let back = env.ingest.set_folder_watch_mode(id, None, None).unwrap();
    assert_eq!(
        (back.watch_mode.clone(), back.watch_active.as_deref()),
        (None, Some("os_events"))
    );
}

#[test]
fn t_fdet_02_invalid_mode_or_options_are_rejected_and_the_db_is_unchanged() {
    // Arrange
    let env = Env::new();
    let id = folder(&env);
    env.ingest
        .set_folder_watch_mode(id, Some("scan_only".into()), None)
        .unwrap();
    let saved = columns(&env);

    // Act / Assert
    let set = |mode: Option<&str>, options: Option<Value>| {
        env.ingest
            .set_folder_watch_mode(id, mode.map(String::from), options)
    };
    assert_eq!(invalid(set(Some("ghost"), None)), "watchMode");
    assert_eq!(invalid(set(Some(""), None)), "watchMode");
    assert_eq!(invalid(set(None, Some(json!({"a": 1})))), "watchOptions");
    assert_eq!(
        invalid(set(Some("scan_only"), Some(json!({"intervalSec": 5})))),
        "watchOptions"
    );
    assert_eq!(
        invalid(set(Some("scan_only"), Some(json!([])))),
        "watchOptions"
    );
    assert_eq!(
        invalid(set(Some("os_events"), Some(json!("x")))),
        "watchOptions"
    );
    assert_eq!(columns(&env), saved);
    assert!(
        set(Some("scan_only"), Some(Value::Null)).is_ok(),
        "null options = defaults"
    );
}

#[test]
fn t_fdet_03_unknown_folder_is_not_found() {
    let env = Env::new();
    folder(&env);

    assert!(matches!(
        env.ingest.list_change_detectors(999),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.set_folder_watch_mode(999, None, None),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest
            .set_folder_rescan_policy(999, Some(json!({"trigger": {"kind": "manual"}}))),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.set_folder_rescan_policy(999, None),
        Err(IngestError::NotFound)
    ));
}

#[test]
fn t_fdet_04_policy_is_normalized_cleared_and_invalid_input_changes_nothing() {
    // Arrange
    let env = Env::new();
    let id = folder(&env);

    // Act: 정규화해 저장
    let view = env
        .ingest
        .set_folder_rescan_policy(
            id,
            Some(json!({"trigger": {"kind": "schedule", "days": ["sun", "mon", "mon"], "at": "03:00"}, "onStart": true})),
        )
        .unwrap();

    // Assert
    let expected = json!({
        "trigger": {"kind": "schedule", "days": ["mon", "sun"], "at": "03:00"},
        "window": null,
        "onStart": true
    });
    assert_eq!(view.rescan_policy, Some(expected.clone()));
    let stored: Value = serde_json::from_str(&columns(&env).2.unwrap()).unwrap();
    assert_eq!(stored, expected);

    // 잘못된 입력은 아무것도 바꾸지 않는다
    let saved = columns(&env);
    let bad = env
        .ingest
        .set_folder_rescan_policy(id, Some(json!({"trigger": {"kind": "weekly"}})));
    assert_eq!(invalid(bad), "rescanPolicy.trigger.kind");
    let bad = env.ingest.set_folder_rescan_policy(
        id,
        Some(json!({"trigger": {"kind": "interval", "hours": 0}})),
    );
    assert_eq!(invalid(bad), "rescanPolicy.trigger.hours");
    assert_eq!(columns(&env), saved);

    // null = 전역 기본으로 되돌림
    let cleared = env.ingest.set_folder_rescan_policy(id, None).unwrap();
    assert_eq!(cleared.rescan_policy, None);
    assert_eq!(columns(&env).2, None);
}

#[test]
fn t_fdet_05_offline_folder_saves_the_mode_without_starting_a_watch() {
    // Arrange
    let env = Env::new();
    let id = folder(&env);
    env.volumes.set_connected("vol-a", false);
    env.ingest.supervise_once().unwrap();
    assert!(!env.ingest.is_watching(id));

    // Act
    let view = env
        .ingest
        .set_folder_watch_mode(id, Some("scan_only".into()), None)
        .unwrap();

    // Assert
    assert_eq!(view.watch_mode.as_deref(), Some("scan_only"));
    assert_eq!(view.watch_active, None);
    assert_eq!(view.next_rescan_at, None);
    assert!(!env.ingest.is_watching(id));
    assert_eq!(columns(&env).0.as_deref(), Some("scan_only"));
}

#[test]
fn t_fdet_06_changing_the_mode_while_ejecting_still_releases_the_volume_in_time() {
    // Arrange: 이동식 디스크의 폴더
    let env = Env::new();
    let usb = env.vol.parent().unwrap().join("usb");
    std::fs::create_dir_all(usb.join("lib")).unwrap();
    std::fs::write(usb.join("lib").join("u.jpg"), [7u8; 90]).unwrap();
    env.volumes.add("usb", VolumeKind::Removable, &usb);
    let id = env
        .ingest
        .add_folder(usb.join("lib").to_str().unwrap())
        .unwrap()
        .folder_id;

    // Act: 방식 변경을 반복하는 동안 꺼내기 준비 (반복 횟수 고정)
    let ingest = Arc::clone(&env.ingest);
    let flipper = std::thread::spawn(move || {
        for i in 0..40 {
            let mode = if i % 2 == 0 {
                Some("scan_only".to_string())
            } else {
                None
            };
            // 꺼내기 준비 뒤에는 저장만 되고 감시는 다시 걸리지 않는다
            let _ = ingest.set_folder_watch_mode(id, mode, None);
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let prepared = env.ingest.prepare_volume_eject(id);
    flipper.join().unwrap();

    // Assert
    assert!(prepared.is_ok(), "{prepared:?}");
    assert!(
        !env.ingest.is_watching(id),
        "no watch is restarted on a held volume"
    );
}

#[test]
fn t_fdet_07_changing_detection_settings_never_touches_the_originals() {
    // Arrange
    let env = Env::new();
    let id = folder(&env);
    env.write("lib/sub/b.jpg", 2);
    let before = snapshot(&env.vol.join("lib"));

    // Act
    env.ingest
        .set_folder_watch_mode(id, Some("scan_only".into()), None)
        .unwrap();
    env.ingest
        .set_folder_rescan_policy(
            id,
            Some(json!({"trigger": {"kind": "manual"}, "onStart": true})),
        )
        .unwrap();
    env.ingest.supervise_once().unwrap();
    env.ingest.set_folder_watch_mode(id, None, None).unwrap();

    // Assert
    assert_eq!(snapshot(&env.vol.join("lib")), before);
}

#[test]
fn t_fdet_08_relocating_the_folder_keeps_the_mode_and_policy() {
    // Arrange
    let env = Env::new();
    let id = folder(&env);
    env.write("moved/a.jpg", 1);
    env.ingest
        .set_folder_watch_mode(id, Some("scan_only".into()), None)
        .unwrap();
    env.ingest
        .set_folder_rescan_policy(
            id,
            Some(json!({"trigger": {"kind": "interval", "hours": 2}})),
        )
        .unwrap();

    // Act
    let out = env
        .ingest
        .relocate_folder(id, env.path("moved").to_str().unwrap())
        .unwrap();

    // Assert
    assert_eq!(out.folder.watch_mode.as_deref(), Some("scan_only"));
    assert_eq!(out.folder.watch_active.as_deref(), Some("scan_only"));
    assert_eq!(
        out.folder.rescan_policy,
        Some(
            json!({"trigger": {"kind": "interval", "hours": 2}, "window": null, "onStart": false})
        )
    );
}

#[test]
fn t_fdet_09_folder_being_removed_is_not_found_for_every_detection_api() {
    // Arrange: 등록 해제 중 (enabled = 0)
    let env = Env::new();
    let id = folder(&env);
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("UPDATE managed_folders SET enabled = 0", [])?)
        })
        .unwrap();

    // Act / Assert
    assert!(matches!(
        env.ingest.list_change_detectors(id),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.set_folder_watch_mode(id, None, None),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.set_folder_rescan_policy(id, None),
        Err(IngestError::NotFound)
    ));
}

#[test]
fn t_fdet_10_auto_mode_accepts_common_options_only() {
    // Arrange
    let env = Env::new();
    let id = folder(&env);
    let set = |mode: Option<&str>, options: Option<Value>| {
        env.ingest
            .set_folder_watch_mode(id, mode.map(String::from), options)
    };

    // Act: 자동 + 공통 옵션 끔 (false 는 기본값이 아니므로 저장된다)
    let off = set(None, Some(json!({"exclusiveOpenCheck": false}))).unwrap();

    // Assert: 모드는 자동 그대로, 옵션만 저장, 감시는 자동 선택대로 동작
    assert_eq!(
        (off.watch_mode.clone(), off.watch_active.as_deref()),
        (None, Some("os_events"))
    );
    assert_eq!(
        off.watch_options,
        Some(json!({"exclusiveOpenCheck": false}))
    );
    // 켬(기본값)은 저장하지 않는다
    assert_eq!(
        set(None, Some(json!({"exclusiveOpenCheck": true})))
            .unwrap()
            .watch_options,
        None
    );

    // Validation: 감지기 전용 옵션·잘못된 타입은 자동에서도 거부, DB 는 그대로
    set(None, Some(json!({"exclusiveOpenCheck": false}))).unwrap();
    let saved = columns(&env);
    for bad in [
        json!({"intervalSec": 60}),
        json!({"source": 1}),
        json!({"exclusiveOpenCheck": 1}),
        json!([]),
    ] {
        assert_eq!(
            invalid(set(None, Some(bad.clone()))),
            "watchOptions",
            "{bad}"
        );
    }
    assert_eq!(columns(&env), saved);
}
