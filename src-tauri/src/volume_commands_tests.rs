// Categories: Happy (이벤트·목록 왕복) / Validation (계약 모양) / Boundary (모르는 키)
use std::path::Path;
use std::sync::Mutex;

use mam_core::ingest::{IngestConfig, IngestDeps};
use mam_core::{
    Blake3Hasher, CoreError, VolumeInfo, VolumeKind, VolumeProvider, WindowsFileIdentity,
};
use mam_db::{DbPool, SystemClock, migrate, open};

use super::*;

/// 연결 목록을 테스트가 바꾸는 가짜 제공자
#[derive(Default)]
struct Fake(Mutex<Vec<VolumeInfo>>);

impl VolumeProvider for Fake {
    fn volume_of(&self, _: &Path) -> Result<VolumeInfo, CoreError> {
        Err(CoreError::Volume("unused".into()))
    }
    fn list_volumes(&self) -> Result<Vec<VolumeInfo>, CoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

#[derive(Default)]
struct Capture(Mutex<Vec<(String, serde_json::Value)>>);

impl EventSink for Capture {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().unwrap().push((event.into(), payload));
    }
}

fn fixture() -> (tempfile::TempDir, Arc<Ingest>, Arc<Fake>, Arc<Capture>) {
    let dir = tempfile::TempDir::new().unwrap();
    let data = dir.path().join("MAMData");
    std::fs::create_dir_all(&data).unwrap();
    let db = data.join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &data.join("backups"), &SystemClock).unwrap();
    drop(conn);
    let fake = Arc::new(Fake::default());
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::new(DbPool::open(&db, 2).unwrap()),
            clock: Arc::new(SystemClock),
            volumes: fake.clone(),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig::default(),
    );
    let cap = Arc::new(Capture::default());
    wire_pending_events(&ingest, cap.clone());
    (dir, ingest, fake, cap)
}

fn usb() -> VolumeInfo {
    VolumeInfo {
        key: "usb".into(),
        kind: VolumeKind::Removable,
        label: Some("SanDisk".into()),
        mount_point: r"E:\".into(),
    }
}

#[test]
fn t_ipc_vol_01_pending_event_and_list_use_the_contract_shape() {
    // Arrange
    let (_d, ingest, fake, cap) = fixture();
    ingest.supervise_once().unwrap();

    // Act
    fake.0.lock().unwrap().push(usb());
    ingest.supervise_once().unwrap();

    // Assert: 이벤트 `{ volumes: [...] }` 와 조회 결과가 같은 camelCase 모양
    let ev = cap.0.lock().unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].0, "volumes://pending-changed");
    let expected = serde_json::json!({"volumes": [{
        "volumeKey": "usb", "label": "SanDisk", "kind": "removable",
        "mountPoint": "E:\\", "totalBytes": null
    }]});
    assert_eq!(ev[0].1, expected);
    assert_eq!(
        serde_json::to_value(ingest.pending_volumes()).unwrap(),
        expected["volumes"]
    );
}

#[test]
fn t_ipc_vol_02_dismiss_remember_then_list_and_unignore_round_trip() {
    // Arrange
    let (_d, ingest, fake, cap) = fixture();
    ingest.supervise_once().unwrap();
    fake.0.lock().unwrap().push(usb());
    ingest.supervise_once().unwrap();

    // Act
    ingest.dismiss_new_volume("usb", true).unwrap();
    let ignored = serde_json::to_value(ingest.list_ignored_volumes().unwrap()).unwrap();

    // Assert: IgnoredVolume 계약 키, 이벤트로 빈 목록 통지
    let obj = ignored[0].as_object().unwrap();
    let mut keys: Vec<_> = obj.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["ignoredAt", "label", "lastMountPoint", "volumeKey"]);
    assert!(obj["ignoredAt"].is_i64());
    assert_eq!(
        cap.0.lock().unwrap().last().unwrap().1,
        serde_json::json!({"volumes": []})
    );

    // 다시 묻기: 연결돼 있으므로 바로 대기 목록으로
    ingest.unignore_volume("usb").unwrap();
    assert_eq!(ingest.pending_volumes().len(), 1);
    assert!(ingest.list_ignored_volumes().unwrap().is_empty());
}

#[test]
fn t_ipc_vol_03_unknown_keys_are_ok_no_ops() {
    let (_d, ingest, _fake, cap) = fixture();

    ingest.dismiss_new_volume("nope", true).unwrap();
    ingest.unignore_volume("nope").unwrap();

    assert!(cap.0.lock().unwrap().is_empty());
    assert!(ingest.list_ignored_volumes().unwrap().is_empty());
}
