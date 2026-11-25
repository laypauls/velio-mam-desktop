// Categories: Happy (계약 모양·왕복) / Validation (오류 코드 매핑) / Business rule (오프라인·멱등) / External (실제 볼륨 API)
use std::path::PathBuf;

use mam_core::ingest::{IngestConfig, IngestDeps};
use mam_core::scan::IngestError;
use mam_core::{
    Blake3Hasher, CoreError, VolumeInfo, VolumeKind, VolumeProvider, WindowsFileIdentity,
};
use mam_db::{DbPool, SystemClock, migrate, open};

use super::*;
use crate::error::ErrorCode;

/// `media` 폴더를 USB 볼륨 루트로 보는 가짜 제공자 - 실제 임시 폴더는 시스템 볼륨(C:)에 있어 실제 제공자로는 꺼낼 수 없는 디스크가 된다
pub(crate) struct MediaVolume(pub PathBuf);

impl MediaVolume {
    fn info(&self) -> VolumeInfo {
        VolumeInfo {
            key: "usb-test".into(),
            kind: VolumeKind::Removable,
            label: None,
            mount_point: self.0.to_string_lossy().into_owned(),
        }
    }
}

impl VolumeProvider for MediaVolume {
    fn volume_of(&self, path: &std::path::Path) -> Result<VolumeInfo, CoreError> {
        if path.starts_with(&self.0) {
            Ok(self.info())
        } else {
            Err(CoreError::Volume("outside".into()))
        }
    }
    fn list_volumes(&self) -> Result<Vec<VolumeInfo>, CoreError> {
        Ok(vec![self.info()])
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    media: PathBuf,
    ingest: Arc<Ingest>,
}

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let (data, media) = (base.join("MAMData"), base.join("media"));
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
            volumes: Arc::new(MediaVolume(media.clone())),
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

#[test]
fn t_ipc_eject_01_prepare_and_resume_round_trip_with_the_contract_shapes() {
    // Arrange: 실제 볼륨 API 로 폴더 등록
    let fx = fixture();
    let id = fx
        .ingest
        .add_folder(fx.media.to_str().unwrap())
        .unwrap()
        .folder_id;

    // Act
    let prepared = fx.ingest.prepare_volume_eject(id).unwrap();
    let after_prepare = serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap();
    fx.ingest.resume_volume_watch(id).unwrap();
    let after_resume = serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap();

    // Assert: EjectPrepared = { mountPoint, folderIds }, ManagedFolder.ejectPrepared + availability
    let v = serde_json::to_value(&prepared).unwrap();
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["folderIds", "mountPoint"]);
    assert_eq!(v["folderIds"], serde_json::json!([id]));
    assert!(
        v["mountPoint"].as_str().unwrap().contains(':'),
        "user-facing mount point such as C:"
    );
    assert_eq!(after_prepare[0]["ejectPrepared"], true);
    assert_eq!(after_prepare[0]["availability"], "offline");
    assert_eq!(after_resume[0]["ejectPrepared"], false);
    assert_eq!(after_resume[0]["availability"], "online");
}

#[test]
fn t_ipc_eject_02_errors_map_to_the_contract_codes_without_paths() {
    // Arrange: 네트워크 / 오프라인 / 없는 폴더 / 대기 시간 초과
    let cases = [
        (
            IngestError::InvalidInput("network volume"),
            ErrorCode::InvalidInput,
            "invalid_input",
        ),
        (IngestError::Offline, ErrorCode::Offline, "offline"),
        (IngestError::NotFound, ErrorCode::NotFound, "not_found"),
        (IngestError::Busy, ErrorCode::Busy, "busy"),
    ];

    // Act / Assert
    for (err, code, text) in cases {
        let cmd = CommandError::from(err);
        assert_eq!(cmd.code(), code);
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["code"], text);
        assert!(
            !v["message"]
                .as_str()
                .unwrap()
                .contains(std::path::MAIN_SEPARATOR)
        );
    }
}

#[test]
fn t_ipc_eject_03_unknown_folder_and_unprepared_resume_follow_the_contract() {
    // Arrange
    let fx = fixture();
    let id = fx
        .ingest
        .add_folder(fx.media.to_str().unwrap())
        .unwrap()
        .folder_id;

    // Act / Assert: 준비 안 된 상태의 resume 은 성공, 없는 폴더는 not_found
    assert!(fx.ingest.resume_volume_watch(id).is_ok());
    let missing = CommandError::from(fx.ingest.prepare_volume_eject(9999).unwrap_err());
    assert_eq!(missing.code(), ErrorCode::NotFound);
    // 두 번 준비해도 같은 결과 (멱등)
    let first = fx.ingest.prepare_volume_eject(id).unwrap();
    assert_eq!(fx.ingest.prepare_volume_eject(id).unwrap(), first);
}
