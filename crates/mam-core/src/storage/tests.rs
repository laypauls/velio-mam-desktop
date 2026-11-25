// Categories: Happy (01 레이아웃, 위치 우선순위) / Validation (02 NAS·상대경로·쓰기 불가) /
// Concurrency (03 두 번째 인스턴스) / Boundary (04 손상된 부트스트랩, 미러)
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

struct FakeProbe(&'static [&'static str]);

impl DriveProbe for FakeProbe {
    fn is_remote_drive(&self, drive_root: &str) -> bool {
        self.0.contains(&drive_root)
    }
}

const LOCAL_ONLY: FakeProbe = FakeProbe(&[]);

fn write_bootstrap(dir: &Path, content: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("location.json"), content).unwrap();
}

#[test]
fn t_store_01_ensure_layout_creates_structure_and_is_idempotent() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let paths = DataPaths::new(tmp.path().join("MAMData"));

    // Act
    paths.ensure_layout().unwrap();
    std::fs::write(paths.db(), b"keep me").unwrap();
    paths.ensure_layout().unwrap();

    // Assert
    for dir in [
        paths.database_dir(),
        paths.tantivy(),
        paths.vectors_visual(),
        paths.vectors_semantic(),
        paths.thumbnails(),
        paths.previews(),
        paths.models().join("embedding"),
        paths.models().join("stt"),
        paths.plugins(),
        paths.backups(),
        paths.logs(),
        paths.updates(),
        paths.tmp(),
    ] {
        assert!(dir.is_dir(), "{dir:?} missing");
    }
    assert_eq!(std::fs::read(paths.db()).unwrap(), b"keep me");
    assert!(!paths.tmp().join(".write-probe").exists());
}

#[test]
fn t_store_01b_default_location_when_nothing_configured() {
    let tmp = TempDir::new().unwrap();
    let r = resolve_location(None, tmp.path(), &LOCAL_ONLY);
    assert_eq!(r.data_dir, tmp.path().join("MAMData"));
    assert_eq!(r.source, LocationSource::Default);
    assert!(r.warnings.is_empty());
}

#[test]
fn t_store_01c_precedence_env_over_file_over_default() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let chosen = tmp.path().join("chosen");
    write_location(tmp.path(), &chosen, &LOCAL_ONLY).unwrap();
    let env = tmp.path().join("from-env");

    // Act
    let from_file = resolve_location(None, tmp.path(), &LOCAL_ONLY);
    let from_env = resolve_location(Some(env.clone().into_os_string()), tmp.path(), &LOCAL_ONLY);
    let pending = pending_location(tmp.path());
    clear_location(tmp.path()).unwrap();
    let after_clear = resolve_location(None, tmp.path(), &LOCAL_ONLY);

    // Assert
    assert_eq!(
        (from_file.data_dir, from_file.source),
        (chosen.clone(), LocationSource::File)
    );
    assert_eq!(
        (from_env.data_dir, from_env.source),
        (env, LocationSource::Env)
    );
    assert_eq!(pending, Some(chosen));
    assert_eq!(after_clear.source, LocationSource::Default);
}

#[test]
fn t_store_02_rejects_unc_network_drive_and_relative_paths() {
    // Arrange
    let probe = FakeProbe(&["Z:\\"]);
    let tmp = TempDir::new().unwrap();

    // Act + Assert
    for unc in [
        r"\\nas\share\MAMData",
        "//nas/share/MAMData",
        r"\\?\UNC\nas\share",
    ] {
        assert!(
            matches!(
                validate_location(Path::new(unc), &probe),
                Err(StorageError::NetworkLocation(_))
            ),
            "{unc}"
        );
    }
    for remote in [r"Z:\MAMData", r"z:\MAMData"] {
        assert!(matches!(
            validate_location(Path::new(remote), &probe),
            Err(StorageError::NetworkLocation(_))
        ));
    }
    assert!(matches!(
        validate_location(Path::new(r"relative\MAMData"), &probe),
        Err(StorageError::NotAbsolute(_))
    ));
    assert!(validate_location(Path::new(r"C:\MAMData"), &probe).is_ok());
    assert!(validate_location(Path::new(r"\\?\C:\MAMData"), &probe).is_ok());
    // 거부된 위치는 부트스트랩 파일에 기록되지 않는다
    assert!(write_location(tmp.path(), Path::new(r"\\nas\share"), &probe).is_err());
    assert!(!tmp.path().join("location.json").exists());
}

#[test]
fn t_store_02b_unwritable_location_is_an_error() {
    // Arrange: MAMData 자리에 일반 파일이 있어 디렉터리를 만들 수 없다
    let tmp = TempDir::new().unwrap();
    let blocker = tmp.path().join("MAMData");
    std::fs::write(&blocker, b"i am a file").unwrap();

    // Act
    let layout = DataPaths::new(&blocker).ensure_layout();
    let write = write_location(tmp.path(), &blocker, &LOCAL_ONLY);

    // Assert
    assert!(matches!(layout, Err(StorageError::Io { .. })));
    assert!(write.is_err());
    assert!(!tmp.path().join("location.json").exists());
}

#[test]
fn t_store_03_second_instance_gets_already_running() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let lock_file = tmp.path().join(".lock");
    let first = InstanceLock::acquire(&lock_file).unwrap();

    // Act
    let second = InstanceLock::acquire(&lock_file);
    drop(first);
    let third = InstanceLock::acquire(&lock_file);

    // Assert
    assert!(matches!(second, Err(StorageError::AlreadyRunning)));
    assert!(third.is_ok(), "lock must be released on drop");
}

#[test]
fn t_store_04_corrupt_or_invalid_bootstrap_falls_back_to_default_with_warning() {
    for (name, content) in [
        ("garbage", "{ not json"),
        ("wrong shape", r#"{"dir": "C:\\x"}"#),
        ("empty", ""),
        ("network", r#"{"dataDir": "\\\\nas\\share\\MAMData"}"#),
        ("relative", r#"{"dataDir": "rel\\MAMData"}"#),
    ] {
        // Arrange
        let tmp = TempDir::new().unwrap();
        write_bootstrap(tmp.path(), content);

        // Act
        let r = resolve_location(None, tmp.path(), &LOCAL_ONLY);

        // Assert
        assert_eq!(r.data_dir, tmp.path().join("MAMData"), "{name}");
        assert_eq!(r.source, LocationSource::Default, "{name}");
        assert_eq!(r.warnings.len(), 1, "{name}");
        // 손상된 파일은 지우지 않는다 (사용자가 고칠 수 있도록)
        assert!(tmp.path().join("location.json").exists(), "{name}");
    }
}

#[test]
fn t_store_04b_relative_env_is_ignored_with_warning() {
    let tmp = TempDir::new().unwrap();
    let r = resolve_location(Some("relative".into()), tmp.path(), &LOCAL_ONLY);
    assert_eq!(r.source, LocationSource::Default);
    assert_eq!(r.warnings.len(), 1);
}

#[test]
fn t_store_04c_mirror_roundtrips_non_ascii_and_skips_identical_rewrite() {
    // Arrange
    let tmp = TempDir::new().unwrap();
    let dir = PathBuf::from(r"D:\미디어 데이터\MAMData");

    // Act
    write_mirror(tmp.path(), &dir).unwrap();
    let first = std::fs::metadata(mirror_file(tmp.path()))
        .unwrap()
        .modified()
        .unwrap();
    write_mirror(tmp.path(), &dir).unwrap();
    let second = std::fs::metadata(mirror_file(tmp.path()))
        .unwrap()
        .modified()
        .unwrap();

    // Assert
    assert_eq!(read_mirror(tmp.path()), Some(dir));
    assert_eq!(first, second);
    let bytes = std::fs::read(mirror_file(tmp.path())).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xFE], "UTF-16LE BOM for NSIS");
}

mod overlap_tests {
    // Categories: Happy / Boundary (대소문자·구분자·`..`) / Business rule (D1 고정 하위 폴더)
    use std::path::Path;

    use super::super::{is_same_or_inside, paths_overlap, resolve_data_root};

    #[test]
    fn t_store_overlap_01_equal_parent_and_child_overlap_siblings_do_not() {
        assert!(paths_overlap(Path::new(r"D:\Work"), Path::new(r"d:/work/")));
        assert!(paths_overlap(
            Path::new(r"D:\Work"),
            Path::new(r"D:\Work\cache")
        ));
        assert!(paths_overlap(
            Path::new(r"D:\Work\cache"),
            Path::new(r"D:\Work")
        ));
        assert!(!paths_overlap(
            Path::new(r"D:\Work"),
            Path::new(r"D:\Workshop")
        ));
        assert!(!paths_overlap(Path::new(r"D:\Work"), Path::new(r"E:\Work")));
        assert!(paths_overlap(
            Path::new(r"D:\Work\a\..\b"),
            Path::new(r"D:\Work\b\x")
        ));
    }

    #[test]
    fn t_store_overlap_02_is_same_or_inside_is_directional() {
        assert!(is_same_or_inside(
            Path::new(r"D:\Work\x"),
            Path::new(r"D:\Work")
        ));
        assert!(is_same_or_inside(
            Path::new(r"D:\Work"),
            Path::new(r"D:\Work")
        ));
        assert!(!is_same_or_inside(
            Path::new(r"D:\Work"),
            Path::new(r"D:\Work\x")
        ));
    }

    #[test]
    fn t_store_overlap_03_existing_real_dirs_resolve_through_canonicalize() {
        let dir = tempfile::TempDir::new().unwrap();
        let work = dir.path().join("Work");
        std::fs::create_dir_all(work.join("cache")).unwrap();
        let upper = dir.path().join("WORK").join("CACHE");
        assert!(paths_overlap(&work, &upper));
    }

    #[test]
    fn t_store_overlap_04_data_root_gets_a_fixed_subfolder_unless_already_a_root() {
        // Arrange
        let dir = tempfile::TempDir::new().unwrap();
        let existing_root = dir.path().join("root");
        std::fs::create_dir_all(existing_root.join("database")).unwrap();
        std::fs::write(existing_root.join("database").join("mam.sqlite"), b"").unwrap();

        // Act / Assert
        assert_eq!(
            resolve_data_root(dir.path()),
            dir.path().join("MAMData"),
            "plain folder"
        );
        assert_eq!(resolve_data_root(&existing_root), existing_root);
    }
}
