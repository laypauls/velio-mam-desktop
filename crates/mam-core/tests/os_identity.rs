//! T-id-01~04: 실제 Windows 볼륨·파일 ID API (임시 디렉터리).
//!
//! 분류: Happy / Boundary / External
//! 실제 USB 탈착·NAS·드라이브 문자 변경은 장비가 없어 시험하지 않는다 (reconcile 시험의 가짜 provider 로 대체).
#![cfg(windows)]

mod common;

use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use mam_core::{
    Blake3Hasher, ContentHasher, CoreError, FileIdentityProvider, VolumeKind, VolumeProvider,
    WindowsFileIdentity, WindowsVolumes, relative_path,
};
use tempfile::TempDir;

#[test]
fn t_id_01_system_volume_key_and_kind() {
    // Arrange: 같은 볼륨 안의 두 경로
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.bin");
    let sub = dir.path().join("sub");
    fs::create_dir_all(&sub).unwrap();
    // Act
    let va = WindowsVolumes.volume_of(&a).unwrap();
    let vb = WindowsVolumes.volume_of(&sub.join("b.bin")).unwrap();
    // Assert: GUID 경로 키, 같은 볼륨은 같은 키, 마운트 지점은 드라이브 문자와 분리된 값
    assert!(va.key.starts_with(r"\\?\Volume{"), "key = {}", va.key);
    assert_eq!(va, vb);
    assert_eq!(va.kind, VolumeKind::Fixed);
    assert!(va.mount_point.ends_with('\\'));
    // 마운트 지점 기준 상대 경로가 계산된다
    let rel = relative_path(&va.mount_point, &sub.join("b.bin")).unwrap();
    assert!(rel.ends_with(r"sub\b.bin"), "rel = {rel}");
    // 열거 결과에도 같은 키가 있다 (온라인 전환 판정의 근거)
    let listed = WindowsVolumes.list_volumes().unwrap();
    assert!(listed.iter().any(|v| v.key == va.key));
}

#[test]
fn t_id_01_unc_path_is_network_volume_without_io() {
    let v = WindowsVolumes
        .volume_of(Path::new(r"\\NAS\Share\dir\f.mp4"))
        .unwrap();
    assert_eq!(v.kind, VolumeKind::Network);
    assert_eq!(v.key, "//nas/share");
    assert_eq!(v.mount_point, r"\\NAS\Share\");
    assert_eq!(
        relative_path(&v.mount_point, Path::new(r"\\nas\share\dir\f.mp4")).unwrap(),
        r"dir\f.mp4"
    );
}

#[test]
fn t_id_02_file_id_survives_rename_and_move_but_not_copy() {
    // Arrange
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.bin");
    fs::write(&a, b"content").unwrap();
    let id_a = WindowsFileIdentity.stat(&a).unwrap().file_identity;
    assert!(id_a.is_some(), "temp volume is expected to be NTFS/ReFS");
    // Act: 이름 변경, 다른 폴더로 이동, 복사
    let b = dir.path().join("b.bin");
    fs::rename(&a, &b).unwrap();
    fs::create_dir_all(dir.path().join("moved")).unwrap();
    let c = dir.path().join("moved").join("c.bin");
    fs::rename(&b, &c).unwrap();
    let copy = dir.path().join("copy.bin");
    fs::copy(&c, &copy).unwrap();
    // Assert
    assert_eq!(WindowsFileIdentity.stat(&c).unwrap().file_identity, id_a);
    assert_ne!(WindowsFileIdentity.stat(&copy).unwrap().file_identity, id_a);
}

#[test]
fn t_id_03_boundary_long_path_korean_nfd_and_readonly() {
    let dir = TempDir::new().unwrap();
    let never = AtomicBool::new(false);

    // 260자 초과 경로
    let mut long = dir.path().to_path_buf();
    while long.to_string_lossy().len() < 300 {
        long.push("d".repeat(40));
    }
    fs::create_dir_all(&long).unwrap();
    let long_file = long.join("long.bin");
    fs::write(&long_file, b"long").unwrap();
    let meta = WindowsFileIdentity.stat(&long_file).unwrap();
    assert_eq!(meta.size, 4);
    assert!(meta.file_identity.is_some());
    assert!(Blake3Hasher.hash_file(&long_file, &never).is_ok());
    assert!(WindowsVolumes.volume_of(&long_file).is_ok());

    // 한글 NFD(자모 분해) 파일명
    let nfd = dir
        .path()
        .join("\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}.jpg");
    fs::write(&nfd, b"kr").unwrap();
    assert_eq!(WindowsFileIdentity.stat(&nfd).unwrap().size, 2);

    // 읽기 전용 파일: 통계·해시 모두 가능해야 한다
    let ro = dir.path().join("ro.bin");
    fs::write(&ro, b"ro").unwrap();
    let mut perms = fs::metadata(&ro).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&ro, perms).unwrap();
    assert!(
        WindowsFileIdentity
            .stat(&ro)
            .unwrap()
            .file_identity
            .is_some()
    );
    assert!(Blake3Hasher.hash_file(&ro, &never).is_ok());
    // 정리 가능하도록 해제
    let mut perms = fs::metadata(&ro).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&ro, perms).unwrap();
}

#[test]
fn t_id_04_external_errors_are_returned_not_panicked() {
    let dir = TempDir::new().unwrap();
    let never = AtomicBool::new(false);

    // 없는 파일
    let missing = dir.path().join("nope.bin");
    assert!(matches!(
        WindowsFileIdentity.stat(&missing),
        Err(CoreError::Io { .. })
    ));
    assert!(Blake3Hasher.hash_file(&missing, &never).is_err());

    // 디렉터리를 파일처럼 요청
    assert!(matches!(
        WindowsFileIdentity.stat(dir.path()),
        Err(CoreError::NotAFile(_))
    ));

    // 잘못된 문자가 든 경로
    assert!(
        WindowsFileIdentity
            .stat(&dir.path().join("a<b>.bin"))
            .is_err()
    );

    // 배타적으로 잠긴 파일(다른 프로세스가 독점 사용 중) -> 해시는 오류, 패닉 없음
    let locked = dir.path().join("locked.bin");
    fs::write(&locked, b"x").unwrap();
    let _hold = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .unwrap();
    assert!(matches!(
        Blake3Hasher.hash_file(&locked, &never),
        Err(CoreError::Io { .. })
    ));

    // 존재하지 않는 드라이브
    let free = ('D'..='Z')
        .map(|c| format!("{c}:\\"))
        .find(|root| !Path::new(root).exists())
        .expect("a free drive letter");
    assert!(
        WindowsVolumes
            .volume_of(Path::new(&format!("{free}none\\x.bin")))
            .is_err()
    );
}

#[test]
fn t_id_05_real_device_query_answers_for_the_system_drive_and_fails_softly_for_garbage() {
    // Arrange: 개발 PC 의 시스템 드라이브(내장일 수도 있다 - 값은 환경에 따라 다르므로 단정하지 않는다)
    let root = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()) + "\\";

    // Act
    let system = WindowsVolumes.is_ejectable_device(&root);
    let garbage = WindowsVolumes.is_ejectable_device(r"Q:\no\such\mount");

    // Assert: 패닉 없이 Some/None 으로 답하고, 존재하지 않는 마운트는 조회 실패(None)다
    println!("device query for {root}: {system:?}");
    assert!(system.is_some(), "the system drive can be queried");
    assert_eq!(garbage, None);
}
