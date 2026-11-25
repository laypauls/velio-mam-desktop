//! P2-9 관리 데이터 초기화 시험 (T-prot-10, 11).
//! 분류: Business rule / External

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::env::AtomicClock;
use common::{Snapshot, T0};
use mam_core::protect::{
    BackupOutcome, apply_pending_reset, clear_reset_marker, marker_path, plan_reset,
    read_reset_marker, write_reset_marker,
};
use mam_core::storage::DataPaths;
use tempfile::TempDir;

struct World {
    _data: TempDir,
    originals: TempDir,
    paths: DataPaths,
    backup: PathBuf,
}

fn world() -> World {
    let data = TempDir::new().unwrap();
    let paths = DataPaths::new(data.path().join("MAMData"));
    paths.ensure_layout().unwrap();
    fs::write(paths.db(), b"sqlite").unwrap();
    fs::write(paths.database_dir().join("mam.sqlite-wal"), b"wal").unwrap();
    fs::write(paths.tantivy().join("seg"), b"idx").unwrap();
    fs::write(paths.vectors_visual().join("v.bin"), b"vec").unwrap();
    fs::write(paths.thumbnails().join("t.webp"), b"thumb").unwrap();
    fs::write(paths.models().join("stt").join("m.bin"), b"model").unwrap();
    fs::write(paths.logs().join("a.log"), b"log").unwrap();
    let backup = paths.backups().join("pre_reset-1.sqlite");
    fs::write(&backup, b"backup").unwrap();
    let originals = TempDir::new().unwrap();
    fs::write(originals.path().join("photo.jpg"), b"original").unwrap();
    World {
        _data: data,
        originals,
        paths,
        backup,
    }
}

fn is_empty(dir: &Path) -> bool {
    fs::read_dir(dir).unwrap().next().is_none()
}

#[test]
fn t_prot_10_reset_wipes_db_indexes_cache_but_never_backups_or_originals() {
    // Arrange
    let w = world();
    let originals_before = Snapshot::take(w.originals.path());
    let backups_before = Snapshot::take(&w.paths.backups());
    let clock = AtomicClock::new();
    let protected = vec![w.originals.path().to_path_buf()];

    // Act
    let plan = plan_reset(
        &w.paths,
        &BackupOutcome::Created(w.backup.clone()),
        &protected,
    )
    .unwrap();
    write_reset_marker(&w.paths, &clock, &plan).unwrap();
    let marker = read_reset_marker(&w.paths).unwrap().unwrap();
    // 표지가 있는 동안에는 아직 아무것도 지워지지 않았다
    assert!(w.paths.db().exists());
    let report = apply_pending_reset(&w.paths, &protected).unwrap().unwrap();

    // Assert
    assert_eq!(marker.requested_at, T0);
    assert_eq!(
        plan.delete,
        vec![
            w.paths.database_dir(),
            w.paths.root().join("indexes"),
            w.paths.cache(),
            w.paths.plugins()
        ]
    );
    assert_eq!(report.removed, plan.delete);
    assert!(is_empty(&w.paths.database_dir()));
    assert!(is_empty(&w.paths.tantivy()));
    assert!(is_empty(&w.paths.vectors_visual()));
    assert!(is_empty(&w.paths.thumbnails()));
    assert!(!marker_path(&w.paths).exists());
    // 보존 대상
    assert_eq!(Snapshot::take(&w.paths.backups()), backups_before);
    assert_eq!(Snapshot::take(w.originals.path()), originals_before);
    assert!(w.paths.models().join("stt").join("m.bin").exists());
    assert!(w.paths.logs().join("a.log").exists());
    // 멱등: 표지가 없으면 아무 일도 하지 않는다
    assert!(apply_pending_reset(&w.paths, &protected).unwrap().is_none());
}

#[test]
fn t_prot_10b_plan_refuses_when_a_protected_path_or_the_backup_is_inside_a_target() {
    let w = world();

    let inside_cache = w.paths.cache().join("my-originals");
    let e = plan_reset(
        &w.paths,
        &BackupOutcome::Created(w.backup.clone()),
        &[inside_cache],
    )
    .unwrap_err();
    assert_eq!(e.code(), "invalid_input");

    let bad_backup = w.paths.database_dir().join("pre_reset.sqlite");
    fs::write(&bad_backup, b"b").unwrap();
    let e = plan_reset(&w.paths, &BackupOutcome::Created(bad_backup), &[]).unwrap_err();
    assert_eq!(e.code(), "io");
    assert!(w.paths.db().exists());
}

#[test]
fn t_prot_11_backup_failure_aborts_reset_and_leaves_data_untouched() {
    // Arrange
    let w = world();
    let before = Snapshot::take(w.paths.root());

    // Act
    let failed = plan_reset(&w.paths, &BackupOutcome::Failed("disk full".into()), &[]);
    let missing = plan_reset(
        &w.paths,
        &BackupOutcome::Created(w.paths.backups().join("nope.sqlite")),
        &[],
    );

    // Assert: 계획 단계에서 거부되므로 표지도 만들 수 없다
    assert_eq!(failed.unwrap_err().code(), "io");
    assert_eq!(missing.unwrap_err().code(), "io");
    assert!(!marker_path(&w.paths).exists());
    assert_eq!(Snapshot::take(w.paths.root()), before);
}

#[test]
fn t_prot_11b_pending_marker_with_vanished_backup_does_not_wipe_and_keeps_marker() {
    let w = world();
    let clock = AtomicClock::new();
    let plan = plan_reset(&w.paths, &BackupOutcome::Created(w.backup.clone()), &[]).unwrap();
    write_reset_marker(&w.paths, &clock, &plan).unwrap();
    fs::remove_file(&w.backup).unwrap(); // 재시작 사이에 백업이 사라졌다

    let e = apply_pending_reset(&w.paths, &[]).unwrap_err();

    assert_eq!(e.code(), "io");
    assert!(w.paths.db().exists());
    assert!(marker_path(&w.paths).exists());
    // 호출자가 취소를 선택하면 표지를 지울 수 있다
    clear_reset_marker(&w.paths).unwrap();
    clear_reset_marker(&w.paths).unwrap();
    assert!(read_reset_marker(&w.paths).unwrap().is_none());
}

#[test]
fn t_prot_11c_corrupt_marker_is_reported_not_applied() {
    let w = world();
    fs::write(marker_path(&w.paths), b"{not json").unwrap();

    let e = apply_pending_reset(&w.paths, &[]).unwrap_err();

    assert_eq!(e.code(), "internal");
    assert!(w.paths.db().exists());
}

// --- T-prot-10c 양방향 보호 경로 검사 ----------------------------------------------

#[test]
fn t_prot_10c_managed_folder_equal_to_or_containing_mamdata_refuses_reset() {
    // Arrange: 관리 폴더 = MAMData 루트 / 그 부모 / 삭제 대상과 같은 경로 / 대소문자만 다른 경로
    let w = world();
    let before = Snapshot::take(w.paths.root());
    let backup = BackupOutcome::Created(w.backup.clone());
    let root = w.paths.root().to_path_buf();
    let parent = root.parent().unwrap().to_path_buf();
    let upper = PathBuf::from(root.to_string_lossy().to_uppercase());
    let same_as_target = vec![w.paths.cache()];

    // Act + Assert
    for protected in [vec![root], vec![parent], vec![upper], same_as_target] {
        let e = plan_reset(&w.paths, &backup, &protected).unwrap_err();
        assert_eq!(e.code(), "invalid_input", "{protected:?}");
    }
    assert_eq!(Snapshot::take(w.paths.root()), before);
}

#[test]
fn t_prot_10d_stale_marker_is_rechecked_at_apply_time_and_wipes_nothing() {
    // Arrange: 표지는 보호 경로가 없던 때 만들어졌고, 재시작 전에 관리 폴더가 MAMData 부모로 등록됐다
    let w = world();
    let clock = AtomicClock::new();
    let plan = plan_reset(&w.paths, &BackupOutcome::Created(w.backup.clone()), &[]).unwrap();
    write_reset_marker(&w.paths, &clock, &plan).unwrap();
    let wiped = [w.paths.database_dir(), w.paths.cache(), w.paths.tantivy()];
    let before: Vec<Snapshot> = wiped.iter().map(|d| Snapshot::take(d)).collect();
    let managed = vec![w.paths.root().parent().unwrap().to_path_buf()];

    // Act
    let e = apply_pending_reset(&w.paths, &managed).unwrap_err();

    // Assert: 거부되고 표지는 남으며 (cache 하위 포함) 파일은 그대로다
    assert_eq!(e.code(), "invalid_input");
    assert!(marker_path(&w.paths).exists());
    assert!(w.paths.thumbnails().join("t.webp").exists());
    let after: Vec<Snapshot> = wiped.iter().map(|d| Snapshot::take(d)).collect();
    assert_eq!(after, before);
}

#[test]
fn t_prot_10e_unrelated_sibling_protected_path_does_not_block_reset() {
    // Arrange: 접두어만 같은 형제 폴더는 포함 관계가 아니다
    let w = world();
    let sibling = PathBuf::from(format!("{}-media", w.paths.root().display()));
    fs::create_dir_all(&sibling).unwrap();

    // Act
    let plan = plan_reset(
        &w.paths,
        &BackupOutcome::Created(w.backup.clone()),
        &[sibling.clone(), w.originals.path().to_path_buf()],
    );

    // Assert
    assert!(plan.is_ok());
    fs::remove_dir_all(sibling).unwrap();
}

#[test]
fn t_pinst_22_reset_plan_includes_the_plugins_folder_and_keeps_every_protection() {
    // 분류: Business rule (플러그인 패키지 사본은 MAM 산출물이라 초기화 대상) / Validation (겹침 보호는 그대로)
    // Arrange: 설치된 플러그인 사본 + 보존 대상
    let w = world();
    let pkg = w.paths.plugins().join("com.example.demo").join("1.0.0");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("plugin.json"), b"{}").unwrap();
    let originals_before = Snapshot::take(w.originals.path());
    let backups_before = Snapshot::take(&w.paths.backups());
    let clock = AtomicClock::new();
    let protected = vec![w.originals.path().to_path_buf()];

    // Act
    let plan = plan_reset(
        &w.paths,
        &BackupOutcome::Created(w.backup.clone()),
        &protected,
    )
    .unwrap();
    write_reset_marker(&w.paths, &clock, &plan).unwrap();
    let report = apply_pending_reset(&w.paths, &protected).unwrap().unwrap();

    // Assert: 플러그인 사본은 지워지고 폴더는 다시 만들어진다. 원본·백업은 그대로
    assert!(plan.delete.contains(&w.paths.plugins()));
    assert!(report.removed.contains(&w.paths.plugins()));
    assert!(w.paths.plugins().is_dir());
    assert!(is_empty(&w.paths.plugins()));
    assert_eq!(Snapshot::take(w.originals.path()), originals_before);
    assert_eq!(Snapshot::take(&w.paths.backups()), backups_before);

    // 보호 경로가 plugins/ 안(또는 위)에 있으면 거부, 백업이 plugins/ 안이면 거부
    let w = world();
    let inside = w.paths.plugins().join("my-originals");
    let e = plan_reset(
        &w.paths,
        &BackupOutcome::Created(w.backup.clone()),
        &[inside],
    )
    .unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    let bad_backup = w.paths.plugins().join("pre_reset.sqlite");
    fs::write(&bad_backup, b"b").unwrap();
    let e = plan_reset(&w.paths, &BackupOutcome::Created(bad_backup), &[]).unwrap_err();
    assert_eq!(e.code(), "io");
}
