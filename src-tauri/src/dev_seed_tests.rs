// 개발 전용 자동 등록 시험 (실제 SQLite + 실제 Ingest::add_folder 검증 경로)
// Categories: Happy / Boundary (빈 값) / Validation (상대·없는 경로) / Business rule (폴더 있으면 건너뜀, MAMData 겹침 거부)
use std::path::PathBuf;
use std::sync::Arc;

use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, SystemClock, migrate, open};

use super::*;

struct Fx {
    _dir: tempfile::TempDir,
    base: PathBuf,
    data: PathBuf,
    ingest: Arc<Ingest>,
}

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let data = base.join("MAMData");
    std::fs::create_dir_all(&data).unwrap();
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
            auto_excludes: vec![data.clone()],
            ..IngestConfig::default()
        },
    );
    Fx {
        _dir: dir,
        base,
        data,
        ingest,
    }
}

fn library(fx: &Fx, name: &str) -> PathBuf {
    let p = fx.base.join(name);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn t_devseed_01_registers_when_no_folders() {
    // Arrange: 형제 폴더 library (MAMData 와 겹치지 않음)
    let fx = fixture();
    let lib = library(&fx, "library");
    // Act
    let out = seed_folder(&fx.ingest, lib.to_str());
    // Assert
    assert!(matches!(out, SeedOutcome::Registered(_)), "{out:?}");
    assert_eq!(fx.ingest.list_folders().unwrap().len(), 1);
}

#[test]
fn t_devseed_02_skips_when_a_folder_exists() {
    // Arrange: 이미 다른 폴더가 등록돼 있다
    let fx = fixture();
    let first = library(&fx, "first");
    let second = library(&fx, "second");
    fx.ingest.add_folder(first.to_str().unwrap()).unwrap();
    // Act
    let out = seed_folder(&fx.ingest, second.to_str());
    // Assert: 사용자 구성을 건드리지 않는다
    assert_eq!(out, SeedOutcome::SkippedHasFolders);
    assert_eq!(fx.ingest.list_folders().unwrap().len(), 1);
}

#[test]
fn t_devseed_03_is_idempotent_across_restarts() {
    // Arrange: 첫 시작에 등록
    let fx = fixture();
    let lib = library(&fx, "library");
    seed_folder(&fx.ingest, lib.to_str());
    // Act: 같은 값으로 두 번째 시작
    let out = seed_folder(&fx.ingest, lib.to_str());
    // Assert
    assert_eq!(out, SeedOutcome::SkippedHasFolders);
    assert_eq!(fx.ingest.list_folders().unwrap().len(), 1);
}

#[test]
fn t_devseed_04_disabled_when_unset_or_blank() {
    let fx = fixture();
    assert_eq!(seed_folder(&fx.ingest, None), SeedOutcome::Disabled);
    assert_eq!(seed_folder(&fx.ingest, Some("  ")), SeedOutcome::Disabled);
    assert!(fx.ingest.list_folders().unwrap().is_empty());
}

#[test]
fn t_devseed_05_rejects_relative_or_missing_path() {
    // Arrange
    let fx = fixture();
    let missing = fx.base.join("nope");
    // Act + Assert
    assert_eq!(
        seed_folder(&fx.ingest, Some("temp/library")),
        SeedOutcome::InvalidPath
    );
    assert_eq!(
        seed_folder(&fx.ingest, missing.to_str()),
        SeedOutcome::InvalidPath
    );
    assert!(fx.ingest.list_folders().unwrap().is_empty());
}

#[test]
fn t_devseed_06_rejects_overlap_with_mamdata_both_directions() {
    // Arrange: MAMData 안쪽 폴더, MAMData 를 포함하는 상위 폴더
    let fx = fixture();
    let inside = fx.data.join("inner");
    std::fs::create_dir_all(&inside).unwrap();
    // Act
    let inner = seed_folder(&fx.ingest, inside.to_str());
    let outer = seed_folder(&fx.ingest, fx.base.to_str());
    // Assert: 공용 검증(add_folder)이 양방향으로 거부한다
    assert!(matches!(inner, SeedOutcome::Rejected(_)), "{inner:?}");
    assert!(matches!(outer, SeedOutcome::Rejected(_)), "{outer:?}");
    assert!(fx.ingest.list_folders().unwrap().is_empty());
}
