//! 폴더 이전(D-6) 통합 시험 — 재연결·원본 보호·mtime 규칙 (T-relocate-01, 02, 03, 07, 09, 12).
//!
//! 분류: Happy (01) / Business rule (02, 03, 07, 09, 12) / External (12 — 옛 볼륨 오프라인).
//! 실제 SQLite + 임시 디렉터리 볼륨 둘. 원본 보호: 시험 전후로 옛·새 폴더의 내용·수정 시각이 바뀌지 않았음을 단언한다.

mod common;

use std::cell::Cell;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};

use common::env::snapshot;
use common::reloc::{Reloc, hours_later};
use mam_core::scan::{ScanOutcome, scan_folder};

fn presence(r: &Reloc, name: &str) -> String {
    r.env.read(|c| {
        c.query_row(
            "SELECT presence FROM file_locations WHERE rel_path LIKE ?1 AND detached_at IS NULL",
            [format!("%{name}")],
            |row| row.get(0),
        )
        .unwrap()
    })
}

fn status(r: &Reloc, asset: i64) -> String {
    r.env.read(|c| {
        c.query_row("SELECT status FROM media_assets WHERE asset_id = ?1", [asset], |row| row.get(0)).unwrap()
    })
}

#[test]
fn t_relocate_01_copy_to_another_volume_relinks_the_same_assets_and_keeps_tags() {
    // Arrange: vol-a 의 lib 를 vol-b 로 복사해 두었다
    let r = Reloc::new();
    let id = r.seeded();
    let (a, b, c) = (r.asset_id("a.jpg"), r.asset_id("b.jpg"), r.asset_id("c.png"));
    r.tag(a, "keep");
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    let (old_before, new_before) = (snapshot(&r.env.path("lib")), snapshot(&r.path_b("lib")));

    // Act
    let out = r.relocate(id, &r.path_b("lib")).unwrap();
    let cleared = r.env.count("SELECT COUNT(*) FROM file_locations WHERE file_identity IS NULL");
    let counts = r.completed(id);

    // Assert: 같은 자산이 새 볼륨·새 경로에 연결되고 태그가 유지된다
    assert_eq!(out.relocated, 3);
    assert!(out.folder.path.ends_with(r"vol_b\lib"), "{}", out.folder.path);
    assert_eq!(cleared, 3, "identity is refilled by the scan");
    assert_eq!((counts.created, counts.moved, counts.missing), (0, 0, 0));
    assert_eq!([r.asset_id("a.jpg"), r.asset_id("b.jpg"), r.asset_id("c.png")], [a, b, c]);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM media_assets"), 3);
    let vol_b = r.volume_id("vol-b");
    assert_eq!(
        r.env.count(&format!(
            "SELECT COUNT(*) FROM file_locations WHERE volume_id = {vol_b} AND presence = 'present'
             AND detached_at IS NULL AND file_identity IS NOT NULL AND path_key LIKE 'lib/%'"
        )),
        3
    );
    assert_eq!(r.env.count(&format!("SELECT COUNT(*) FROM asset_tags WHERE asset_id = {a}")), 1);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'"), 1);
    assert_eq!((old_before, new_before), (snapshot(&r.env.path("lib")), snapshot(&r.path_b("lib"))));
}

#[test]
fn t_relocate_02_changed_file_identity_does_not_trigger_hashing_or_new_assets() {
    // Arrange: 해시 없는 자산 + 복사본은 파일 ID 가 다르다
    let r = Reloc::new();
    let id = r.seeded();
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    let reads_before = r.probe.full.load(Ordering::SeqCst);

    // Act
    r.relocate(id, &r.path_b("lib")).unwrap();
    let counts = r.completed(id);

    // Assert: 새 자산도 ContentChanged 도 없고, 전체 해시를 읽지 않았다
    assert_eq!(counts.created, 0);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM media_assets"), 3);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM asset_relations"), 0);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM media_assets WHERE status <> 'active'"), 0);
    assert_eq!(r.probe.full.load(Ordering::SeqCst), reads_before, "no full-hash reads");
}

#[test]
fn t_relocate_03_file_absent_at_the_new_location_turns_missing_only_after_the_scan_completes() {
    // Arrange: 새 위치에는 c.png 가 없다
    let r = Reloc::new();
    let id = r.seeded();
    let c = r.asset_id("c.png");
    r.tag(c, "keep");
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    fs::remove_file(r.path_b("lib/sub/deep/c.png")).unwrap();

    // Act 1: 이전 직후 (Q1=P) — 상태는 그대로 present
    r.relocate(id, &r.path_b("lib")).unwrap();
    let during = presence(&r, "c.png");
    // Act 2: 스캔 완료
    let counts = r.completed(id);

    // Assert
    assert_eq!(during, "present");
    assert_eq!(counts.missing, 1);
    assert_eq!(presence(&r, "c.png"), "missing");
    assert_eq!(presence(&r, "a.jpg"), "present");
    assert_eq!(r.asset_id("c.png"), c, "asset is kept");
    let (vol, key): (i64, String) = r.env.read(|conn| {
        conn.query_row(
            "SELECT volume_id, path_key FROM file_locations WHERE asset_id = ?1 AND detached_at IS NULL",
            [c],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    });
    assert_eq!((vol, key.as_str()), (r.volume_id("vol-b"), "lib/sub/deep/c.png"));
    assert_eq!(r.env.count(&format!("SELECT COUNT(*) FROM asset_tags WHERE asset_id = {c}")), 1);
}

#[test]
fn t_relocate_07_mtime_changed_copy_follows_the_normal_content_change_rules() {
    // Arrange: 해시 없는 a.jpg / 전체 해시가 있는 b.jpg. 복사본의 수정 시각이 둘 다 달라졌다 (R-1 a: 특별 처리 없음)
    let r = Reloc::new();
    let id = r.seeded();
    let (a, b) = (r.asset_id("a.jpg"), r.asset_id("b.jpg"));
    r.tag(a, "keep");
    let hash = blake3::hash(&fs::read(r.env.path("lib/sub/b.jpg")).unwrap());
    r.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "UPDATE media_assets SET content_hash = ?2, hash_algo = 'blake3', hash_state = 'done' WHERE asset_id = ?1",
                rusqlite::params![b, hash.as_bytes().to_vec()],
            )?;
            Ok(())
        })
        .unwrap();
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    hours_later(&r.path_b("lib/a.jpg"), 2);
    hours_later(&r.path_b("lib/sub/b.jpg"), 2);

    // Act
    r.relocate(id, &r.path_b("lib")).unwrap();
    let counts = r.completed(id);

    // Assert: 해시 없는 쪽은 새 자산 + 옛 자산 superseded(+관계, 태그는 옛 자산에 남음)
    assert_eq!(status(&r, a), "superseded");
    assert_ne!(r.asset_id("a.jpg"), a);
    assert_eq!(r.env.count(&format!("SELECT COUNT(*) FROM asset_relations WHERE from_asset_id = {a}")), 1);
    assert_eq!(r.env.count(&format!("SELECT COUNT(*) FROM asset_tags WHERE asset_id = {a}")), 1);
    // 전체 해시가 있는 쪽은 해시가 같아 같은 자산의 Touched
    assert_eq!(r.asset_id("b.jpg"), b);
    assert_eq!(status(&r, b), "active");
    assert_eq!(r.env.count("SELECT COUNT(*) FROM media_assets"), 4, "one new asset (ContentChanged), counted as updated: {counts:?}");
    assert_eq!(r.env.count(&format!("SELECT COUNT(*) FROM asset_relations WHERE from_asset_id = {b}")), 0);
    assert_eq!(r.probe.full.load(Ordering::SeqCst), 1, "only the hashed asset was re-hashed");
}

#[test]
fn t_relocate_09_paused_scan_cursor_is_cancelled_so_the_next_scan_covers_everything() {
    // Arrange: 스캔이 두 번째 디렉터리쯤에서 멈춘 상태 (커서 있음)
    let r = Reloc::new();
    for (i, d) in ["a", "b", "c", "d"].into_iter().enumerate() {
        r.env.write(&format!("lib/{d}/f.jpg"), 10 + u8::try_from(i).unwrap());
    }
    let id = r.add("lib");
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 2
    };
    let first = scan_folder(r.env.ingest.env(), id, &stop, &AtomicBool::new(false)).unwrap();
    assert_eq!(first, ScanOutcome::Paused);
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));

    // Act
    r.relocate(id, &r.path_b("lib")).unwrap();
    let cancelled = r.env.count("SELECT COUNT(*) FROM scan_runs WHERE status = 'cancelled'");
    let counts = r.completed(id);

    // Assert: 커서를 이어받지 않고 루트부터 전부 훑었다
    assert_eq!(cancelled, 1);
    assert_eq!(counts.processed, 4);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM media_assets"), 4);
    assert_eq!(
        r.env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'present' AND detached_at IS NULL"),
        4
    );
}

#[test]
fn t_relocate_12_old_volume_offline_still_relocates_and_the_scan_restores_presence() {
    // Arrange: 옛 볼륨이 연결 해제됐고 옛 폴더는 읽을 수 없다
    let r = Reloc::new();
    let id = r.seeded();
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    r.env.volumes.set_connected("vol-a", false);
    r.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE volumes SET availability = 'offline' WHERE volume_key = 'vol-a'", [])?;
            Ok(())
        })
        .unwrap();
    fs::remove_dir_all(r.env.path("lib")).unwrap();

    // Act
    let out = r.relocate(id, &r.path_b("lib")).unwrap();
    let before_scan = r.env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'present'");
    let counts = r.completed(id);

    // Assert: 이전 중에는 present 그대로, 스캔 후 새 파일 기준으로 present
    assert_eq!(out.relocated, 3);
    assert_eq!(before_scan, 3);
    assert_eq!(counts.missing, 0);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'present'"), 3);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM media_assets"), 3);
}
