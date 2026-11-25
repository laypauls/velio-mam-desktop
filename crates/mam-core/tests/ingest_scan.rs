//! 수집·스캔 통합 테스트 (T-scan-01 ~ 12).
//!
//! 분류: Happy (01, 09) / Business rule (02, 05, 06, 08, 10) / Concurrency-재개 (03) / External (04, 11) / Validation (07, 12).
//! 실제 SQLite + 임시 디렉터리 볼륨. 원본 보호: 스캔·제거 후 픽스처의 내용·수정 시각·목록이 변하지 않았음을 단언한다.

mod common;

use std::cell::Cell;
use std::fs;
use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use mam_core::VolumeKind;
use mam_core::scan::{IngestError, ScanOutcome, scan_folder};

fn scan(env: &Env, folder_id: i64) -> ScanOutcome {
    scan_folder(
        env.ingest.env(),
        folder_id,
        &|| false,
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn completed(o: ScanOutcome) -> mam_core::scan::ScanCounts {
    match o {
        ScanOutcome::Completed { counts } => counts,
        other => panic!("expected Completed, got {other:?}"),
    }
}

fn add(env: &Env, rel: &str) -> i64 {
    env.ingest
        .add_folder(env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id
}

fn asset_id_of(env: &Env, name: &str) -> i64 {
    env.count(&format!(
        "SELECT asset_id FROM file_locations WHERE rel_path LIKE '%{name}'"
    ))
}

fn presence_of(env: &Env, name: &str) -> Option<String> {
    env.read(|c| {
        c.query_row(
            "SELECT presence FROM file_locations WHERE rel_path LIKE ?1 AND detached_at IS NULL",
            [format!("%{name}")],
            |r| r.get(0),
        )
        .ok()
    })
}

#[test]
fn t_scan_01_scans_subfolders_and_registers_only_media_files() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/b.PNG", 2);
    env.write("lib/sub/deep/c.mp4", 3);
    env.write("lib/sub/deep/d.wav", 4);
    env.write("lib/notes.txt", 5);
    env.write("lib/sub/doc.pdf", 6);
    let id = add(&env, "lib");
    let listed = env.ingest.list_folders().unwrap();
    assert_eq!(
        listed[0].scan_state,
        mam_core::scan::ScanState::Scanning,
        "first scan is queued"
    );

    // Act
    let counts = completed(scan(&env, id));

    // Assert: 대상 형식 4건만 자산, 종류 구분, 이벤트(outbox) 기록
    assert_eq!((counts.processed, counts.created, counts.errors), (4, 4, 0));
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 4);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE media_type = 'image'"),
        2
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE media_type = 'audio'"),
        1
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'upsert'"),
        4
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'pending'"),
        4
    );

    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.asset_count, 4);
    assert!(view.last_full_scan_at.is_some());
    assert_eq!(
        view.scan_state,
        mam_core::scan::ScanState::Scanning,
        "job still queued in this test"
    );

    // 폴더 트리: 지연 로딩 한 단계씩
    let top = env.ingest.list_subfolders(id, None).unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(
        (
            top[0].name.as_str(),
            top[0].has_children,
            top[0].asset_count
        ),
        ("sub", true, 3)
    );
    let sub = env.ingest.list_subfolders(id, Some("sub")).unwrap();
    assert_eq!(sub.len(), 1);
    assert_eq!(
        (
            sub[0].rel_path.as_str(),
            sub[0].has_children,
            sub[0].asset_count
        ),
        ("sub/deep", false, 2)
    );
    assert!(matches!(
        env.ingest.list_subfolders(id, Some("nope")),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.list_subfolders(id, Some("sub/../x")),
        Err(IngestError::InvalidInput(_))
    ));
}

#[test]
// Arrange
fn t_scan_02_exclusion_rules_hidden_system_mamdata_backup_trash_and_user_list() {
    let env = Env::new();
    env.write("lib/ok/keep.jpg", 1);
    env.write("lib/.hidden/a.jpg", 2);
    env.write("lib/.dot.jpg", 3);
    env.write("lib/$RECYCLE.BIN/b.jpg", 4);
    env.write("lib/System Volume Information/c.jpg", 5);
    env.write("lib/skip/e.jpg", 7);
    env.write("lib/attr-hidden/f.jpg", 8);
    let status = std::process::Command::new("attrib")
        .arg("+h")
        .arg(mam_core::volume::strip_verbatim(
            &env.path("lib/attr-hidden").to_string_lossy(),
        ))
        .status()
        .unwrap();
    assert!(status.success());
    let id = add(&env, "lib");
    env.pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "UPDATE managed_folders SET excluded = '[\"skip\"]' WHERE folder_id = ?1",
                [id],
            )?;
            Ok(())
        })
        .unwrap();

    // Act
    let counts = completed(scan(&env, id));

    // Assert: 제외 규칙에 걸리지 않은 1건만
    let kept: Vec<String> = env.read(|c| {
        let mut s = c.prepare("SELECT rel_path FROM file_locations").unwrap();
        s.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    assert_eq!(counts.created, 1, "registered: {kept:?}");
    assert_eq!(presence_of(&env, "keep.jpg").as_deref(), Some("present"));
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 1);
    let tree: Vec<String> = env
        .ingest
        .list_subfolders(id, None)
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(
        tree,
        vec!["ok"],
        "excluded folders are not in the tree either"
    );
}

#[test]
fn t_scan_02b_a_folder_containing_mamdata_or_backups_is_rejected_before_any_state_changes() {
    // Arrange: 폴더 안에 MAMData 가 있다 — 이전에는 스캔에서 건너뛰기만 했지만, 초기화가 원본 트리를 건드릴 수 있어
    // 이제 등록 자체를 거부한다 (D2). auto_excludes 에 폴더 하위 백업 폴더가 있어도 같다.
    let env = Env::with_config(|c, vol| {
        c.auto_excludes = vec![
            vol.join("lib").join("MAMData"),
            vol.join("lib").join("my-backups"),
        ]
    });
    env.write("lib/MAMData/thumbnails/t.jpg", 1);
    env.write("lib/my-backups/d.jpg", 2);
    env.write("lib/real.jpg", 3);

    // Act
    let err = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap_err();
    let inside = env
        .ingest
        .add_folder(env.path("lib/MAMData/thumbnails").to_str().unwrap())
        .unwrap_err();

    // Assert
    assert!(
        matches!(err, IngestError::OverlapsManagedData { contains: true }),
        "{err:?}"
    );
    assert!(
        matches!(inside, IngestError::OverlapsManagedData { contains: false }),
        "{inside:?}"
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), 0);
}

#[test]
fn t_scan_03_resume_processes_only_after_the_cursor_without_duplicates_or_gaps() {
    // Arrange: 디렉터리 6개 x 파일 3개 + 중첩
    let env = Env::new();
    for d in ["a", "b", "b/inner", "c", "d", "e"] {
        for n in 0..3u8 {
            env.write(&format!("lib/{d}/f{n}.jpg"), n + 1);
        }
    }
    env.write("lib/root1.jpg", 9);
    let total = 6 * 3 + 1;
    let id = add(&env, "lib");

    // Act 1: 네 번째 디렉터리를 보기 직전에 중단 요청
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 3
    };
    let first = scan_folder(env.ingest.env(), id, &stop, &AtomicBool::new(false)).unwrap();
    assert_eq!(first, ScanOutcome::Paused);
    let after_first = env.count("SELECT COUNT(*) FROM media_assets");
    let (status, cursor): (String, Option<String>) = env.read(|c| {
        c.query_row(
            "SELECT status, cursor FROM scan_runs WHERE folder_id = ?1",
            [id],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .unwrap()
    });
    assert_eq!(status, "paused");
    assert!(cursor.is_some(), "resume point is stored");
    assert!(after_first > 0 && after_first < total);

    // Act 2: 시간이 흐른 뒤 재개 (원래 시작 시각을 써야 앞 구간 파일이 missing 이 되지 않는다)
    env.clock.advance(60_000);
    let counts = completed(scan(&env, id));

    // Assert: 총 생성 수 = 전체 (재개 때 앞 구간을 다시 처리하지 않았다 -> unchanged 0), 누락·중복 없음
    assert_eq!(counts.created, total as u64);
    assert_eq!(
        counts.unchanged, 0,
        "directories before the cursor were skipped"
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), total);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'present'"),
        total
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM scan_runs WHERE status = 'done'"),
        1,
        "one run, resumed"
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE revision > 1"),
        0
    );
    assert_eq!(env.ingest.list_subfolders(id, Some("b")).unwrap().len(), 1);
}

#[test]
fn t_scan_03b_stop_inside_the_tree_resumes_through_ancestors_of_the_cursor() {
    // Arrange: 커서가 깊은 디렉터리 안에 있을 때 조상 디렉터리의 파일은 다시 처리하지 않고 형제는 이어간다
    let env = Env::new();
    for d in ["a", "b", "b/x", "b/x/y", "b/z", "c"] {
        env.write(&format!("lib/{d}/f.jpg"), 1 + d.len() as u8);
    }
    let id = add(&env, "lib");
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 4 // root, a, b, b/x 처리 후 중단 -> 커서 = b/x
    };
    assert_eq!(
        scan_folder(env.ingest.env(), id, &stop, &AtomicBool::new(false)).unwrap(),
        ScanOutcome::Paused
    );
    let cursor: String = env.read(|c| {
        c.query_row("SELECT cursor FROM scan_runs", [], |r| r.get(0))
            .unwrap()
    });
    assert_eq!(cursor, "b/x");

    let counts = completed(scan(&env, id));

    assert_eq!(counts.created, 6);
    assert_eq!(counts.unchanged, 0);
}

#[test]
fn t_scan_04_unreadable_entries_are_recorded_and_the_scan_continues() {
    // Arrange: 읽기 거부 폴더 하나 (ACL) + 정상 파일. 먼저 한 번 스캔해 위치를 등록해 둔다
    let env = Env::new();
    env.write("lib/ok.jpg", 1);
    env.write("lib/locked/inside.jpg", 2);
    let id = add(&env, "lib");
    completed(scan(&env, id));
    env.clock.advance(1_000);
    let locked = env.path("lib/locked");
    let deny = std::process::Command::new("icacls")
        .arg(&locked)
        .args(["/deny", "*S-1-1-0:(RD)"])
        .output()
        .unwrap();
    assert!(deny.status.success(), "icacls: {deny:?}");
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::process::Command::new("icacls")
                .arg(&self.0)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
        }
    }
    let _restore = Restore(locked);

    // Act: 재스캔
    let counts = completed(scan(&env, id));

    // Assert: 오류 기록 + 나머지 계속, 읽지 못한 폴더 안의 기존 자산은 missing 이 되지 않는다
    assert_eq!(counts.errors, 1);
    let (code, rel): (String, String) = env.read(|c| {
        c.query_row(
            "SELECT error_code, rel_path FROM scan_errors WHERE resolved_at IS NULL",
            [],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .unwrap()
    });
    assert_eq!(
        (code.as_str(), rel.as_str()),
        ("permission_denied", "locked")
    );
    assert_eq!(presence_of(&env, "inside.jpg").as_deref(), Some("present"));
    let (items, total, next) = env.ingest.list_scan_errors(None, 50).unwrap();
    assert_eq!((items.len(), total, next), (1, 1, None));
    assert!(items[0].path.ends_with(r"lib\locked"), "{}", items[0].path);

    // 접근 권한이 돌아온 뒤 다시 스캔하면 오류는 자동 해결된다 (retry 의 실체)
    drop(_restore);
    env.clock.advance(1_000);
    assert_eq!(env.ingest.retry_scan_errors(None).unwrap(), 1);
    completed(scan(&env, id));
    assert_eq!(env.ingest.list_scan_errors(None, 50).unwrap().1, 0);
}

#[test]
fn t_scan_04b_error_list_pages_by_keyset_and_clamps_the_limit() {
    let env = Env::new();
    env.write("lib/ok.jpg", 1);
    let id = add(&env, "lib");
    env.pool
        .write(mam_db::Priority::High, move |c| {
            for i in 0..5 {
                c.execute(
                    "INSERT INTO scan_errors (folder_id, rel_path, error_code, message, last_at)
                     VALUES (?1, ?2, 'io', 'x', 1)",
                    rusqlite::params![id, format!("p{i}")],
                )?;
            }
            Ok(())
        })
        .unwrap();

    let (p1, total, next) = env.ingest.list_scan_errors(None, 2).unwrap();
    let (p2, _, next2) = env.ingest.list_scan_errors(next.as_deref(), 2).unwrap();
    let (p3, _, next3) = env.ingest.list_scan_errors(next2.as_deref(), 2).unwrap();

    assert_eq!((p1.len(), p2.len(), p3.len(), total), (2, 2, 1, 5));
    assert!(next.is_some() && next3.is_none());
    assert!(p1[0].error_id > p1[1].error_id && p1[1].error_id > p2[0].error_id);
    assert_eq!(
        env.ingest.list_scan_errors(None, 10_000).unwrap().0.len(),
        5
    );
    assert!(matches!(
        env.ingest.list_scan_errors(Some("abc"), 10),
        Err(IngestError::InvalidInput(_))
    ));
}

#[test]
fn t_scan_05_rescan_marks_deleted_files_missing_but_offline_volume_changes_nothing() {
    // Arrange
    let env = Env::new();
    env.write("lib/keep.jpg", 1);
    let gone = env.write("lib/gone.jpg", 2);
    let id = add(&env, "lib");
    completed(scan(&env, id));
    let gone_id = asset_id_of(&env, "gone.jpg");
    let rev_before = env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {gone_id}"
    ));

    // Act 1: 파일 삭제 후 재스캔 (볼륨 online)
    fs::remove_file(&gone).unwrap();
    env.clock.advance(5_000);
    let counts = completed(scan(&env, id));

    // Assert 1
    assert_eq!(counts.missing, 1);
    assert_eq!(presence_of(&env, "gone.jpg").as_deref(), Some("missing"));
    assert_eq!(presence_of(&env, "keep.jpg").as_deref(), Some("present"));
    assert_eq!(
        env.count(&format!(
            "SELECT revision FROM media_assets WHERE asset_id = {gone_id}"
        )),
        rev_before + 1,
        "revision bump + outbox for the state change"
    );
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM search_outbox WHERE asset_id = {gone_id}"
        )),
        2
    );

    // Act 2: 볼륨이 분리됨 -> 스캔은 아무것도 바꾸지 않는다
    env.volumes.set_connected("vol-a", false);
    env.ingest.supervise_once().unwrap();
    fs::remove_file(env.path("lib/keep.jpg")).unwrap();
    env.clock.advance(5_000);
    let outcome = scan(&env, id);

    // Assert 2
    assert_eq!(outcome, ScanOutcome::Offline);
    assert_eq!(
        presence_of(&env, "keep.jpg").as_deref(),
        Some("present"),
        "offline is not deletion"
    );
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.availability, mam_core::scan::Availability::Offline);
    assert_eq!(view.asset_count, 2, "offline assets keep counting");
}

#[test]
fn t_scan_05b_unreadable_root_on_an_online_volume_is_an_error_not_a_mass_deletion() {
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/b.jpg", 2);
    let id = add(&env, "lib");
    completed(scan(&env, id));
    fs::remove_dir_all(env.path("lib")).unwrap();
    env.clock.advance(5_000);

    let outcome = scan(&env, id);
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("DELETE FROM jobs", [])?)
        })
        .unwrap(); // 이 시험은 작업 실행기를 돌리지 않으므로 대기 중인 첫 스캔 작업을 비운다

    assert_eq!(outcome, ScanOutcome::RootUnreadable);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"),
        0
    );
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.scan_state, mam_core::scan::ScanState::Failed);
    assert_eq!(
        env.ingest.list_scan_errors(None, 10).unwrap().0[0].error_code,
        "root_unreadable"
    );
}

#[test]
fn t_scan_06_removing_a_folder_deletes_its_linked_data_keeps_other_folders_and_originals() {
    // Arrange (정책 변경 2026-11-14: 예전에는 위치만 detach 하고 자산·태그를 보존했다. 상세는 folder_removal.rs T-rmv-*)
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/b.jpg", 2);
    env.write("other/c.jpg", 3);
    let lib = add(&env, "lib");
    let other = add(&env, "other");
    completed(scan(&env, lib));
    completed(scan(&env, other));
    env.pool
        .write(mam_db::Priority::High, |c| {
            c.execute(
                "INSERT INTO tags (name, name_key) VALUES ('keep', 'keep')",
                [],
            )?;
            c.execute(
                "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (1, 1, 0)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let before = snapshot(&env.vol);
    let outbox_before = env.count("SELECT COUNT(*) FROM search_outbox");

    // Act
    env.ingest.remove_folder(lib).unwrap();
    env.ingest.wait_removals();

    // Assert: 폴더·실행 기록·트리·작업·위치·태그 연결은 사라지고 자산 행은 deleted 로 남는다
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 1);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM scan_runs WHERE folder_id = 1"),
        0
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM folder_dirs WHERE folder_id = 1"),
        0
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE folder_id = 1"),
        0
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"),
        1
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'deleted'"),
        2
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_tags"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM tags WHERE name = 'keep'"), 1, "tag names stay");
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NOT NULL"),
        0,
        "no detached locations are left behind"
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL"),
        1
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'delete'"),
        2
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM search_outbox") - outbox_before,
        2,
        "each removed asset gets one delete event"
    );
    assert_eq!(snapshot(&env.vol), before, "originals untouched");

    // 없는 폴더·이미 제거된 폴더
    assert!(matches!(
        env.ingest.remove_folder(lib),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.rescan_folder(lib),
        Err(IngestError::NotFound)
    ));
    // 남은 폴더는 영향 없음
    assert_eq!(env.ingest.list_folders().unwrap()[0].asset_count, 1);
}

#[test]
fn t_scan_06b_a_scan_in_flight_cannot_resurrect_locations_of_a_removed_folder() {
    // Arrange: 스캔 중에 폴더가 제거되는 경합 — 중단 콜백 안에서 제거한다
    let env = Env::new();
    for d in ["a", "b", "c"] {
        env.write(&format!("lib/{d}/f.jpg"), 1);
    }
    let id = add(&env, "lib");
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            env.ingest.remove_folder(id).unwrap();
            env.ingest.wait_removals();
        }
        false
    };

    // Act
    let outcome = scan_folder(env.ingest.env(), id, &stop, &AtomicBool::new(false)).unwrap();

    // Assert: 제거 이후의 배치는 쓰이지 않는다 -> 활성 위치 0
    assert_eq!(outcome, ScanOutcome::Gone);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL"),
        0
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 0);
}

#[test]
fn t_scan_07_duplicate_and_nested_registrations_are_rejected_with_the_reason() {
    // Arrange
    let env = Env::new();
    fs::create_dir_all(env.path("lib/sub")).unwrap();
    fs::create_dir_all(env.path("sibling")).unwrap();
    let first = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap();

    // Act / Assert
    let same = env.ingest.add_folder(env.path("lib").to_str().unwrap());
    assert!(matches!(same, Err(IngestError::AlreadyRegistered)));
    let upper = env.path("LIB");
    assert!(
        matches!(
            env.ingest.add_folder(upper.to_str().unwrap()),
            Err(IngestError::AlreadyRegistered)
        ),
        "case-insensitive: same folder"
    );
    assert!(matches!(
        env.ingest.add_folder(env.path("lib/sub").to_str().unwrap()),
        Err(IngestError::NestedFolder {
            inside_existing: true
        })
    ));
    assert!(
        matches!(
            env.ingest.add_folder(env.vol.to_str().unwrap()),
            Err(IngestError::NestedFolder {
                inside_existing: false
            })
        ),
        "a parent of a registered folder"
    );
    let sibling = env
        .ingest
        .add_folder(env.path("sibling").to_str().unwrap())
        .unwrap();
    assert_ne!(sibling.folder_id, first.folder_id);
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 2);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan'"),
        2,
        "no job for rejected adds"
    );
}

#[test]
fn t_scan_07b_invalid_paths_are_rejected_before_any_state_changes() {
    let env = Env::new();
    let file = env.write("lib/a.jpg", 1);
    let inside_data = env.data.join("sub");
    fs::create_dir_all(&inside_data).unwrap();

    for (path, why) in [
        ("", "empty"),
        ("relative\\dir", "relative"),
        (file.to_str().unwrap(), "a file"),
        (env.path("missing").to_str().unwrap(), "does not exist"),
        (env.data.to_str().unwrap(), "MAMData itself"),
        (inside_data.to_str().unwrap(), "inside MAMData"),
    ] {
        let err = env.ingest.add_folder(path).unwrap_err();
        let ok = if why.contains("MAMData") {
            matches!(err, IngestError::OverlapsManagedData { .. })
        } else {
            matches!(err, IngestError::InvalidInput(_))
        };
        assert!(ok, "{why}: {err:?}");
    }
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), 0);
}

#[test]
fn t_scan_08_original_protection_scan_rescan_and_remove_never_modify_the_fixtures() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/b.mp4", 2);
    env.write("lib/sub/c.txt", 3);
    env.write("lib/empty/.keep", 4);
    let before = snapshot(&env.vol);

    // Act
    let id = add(&env, "lib");
    completed(scan(&env, id));
    env.clock.advance(1_000);
    completed(scan(&env, id));
    env.ingest.list_subfolders(id, None).unwrap();
    env.ingest.remove_folder(id).unwrap();
    env.ingest.wait_removals();

    // Assert: 내용·수정 시각·목록 불변
    assert_eq!(snapshot(&env.vol), before);
}

#[test]
fn t_scan_09_rename_between_scans_is_a_move_not_a_new_asset() {
    // Arrange
    let env = Env::new();
    let old = env.write("lib/old-name.jpg", 1);
    let id = add(&env, "lib");
    completed(scan(&env, id));
    fs::rename(&old, env.path("lib/new-name.jpg")).unwrap();
    env.clock.advance(5_000);

    // Act
    let counts = completed(scan(&env, id));

    // Assert: 같은 자산이 새 경로로 재연결, 새 자산 없음, missing 없음
    assert_eq!(counts.moved, 1);
    assert_eq!(counts.created, 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(
        presence_of(&env, "new-name.jpg").as_deref(),
        Some("present")
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL"),
        1
    );
}

#[test]
fn t_scan_10_changed_content_is_hashed_outside_the_write_thread_and_supersedes_the_asset() {
    // Arrange: 해시가 끝난 자산이 있어야 변경 판정에 해시가 필요하다
    let env = Env::new();
    let p = env.write("lib/photo.jpg", 1);
    let id = add(&env, "lib");
    completed(scan(&env, id));
    let digest = *blake3::hash(&fs::read(&p).unwrap()).as_bytes();
    env.pool
        .write(mam_db::Priority::Low, move |c| {
            c.execute(
                "UPDATE media_assets SET content_hash = ?1, hash_algo = 'blake3', hash_state = 'done'",
                [digest.as_slice()],
            )?;
            Ok(())
        })
        .unwrap();
    fs::write(&p, b"completely different bytes, longer").unwrap();
    env.clock.advance(5_000);

    // Act
    let counts = completed(scan(&env, id));

    // Assert: 새 자산 + 구자산 superseded (PL6), 위치는 새 자산이 가진다
    assert_eq!(counts.updated, 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 2);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'superseded'"),
        1
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_relations"), 1);
    assert_eq!(
        env.count("SELECT asset_id FROM file_locations WHERE detached_at IS NULL"),
        2
    );
}

#[test]
fn t_scan_11_volume_root_can_be_a_managed_folder() {
    // Arrange: USB 드라이브 루트 등록
    let env = Env::new();
    env.write("a.jpg", 1);
    env.write("deep/b.jpg", 2);
    let view = env.ingest.add_folder(env.vol.to_str().unwrap()).unwrap();

    // Act
    let counts = completed(scan(&env, view.folder_id));

    // Assert
    assert_eq!(counts.created, 2);
    assert_eq!(env.ingest.list_folders().unwrap()[0].asset_count, 2);
    assert_eq!(
        env.ingest.list_subfolders(view.folder_id, None).unwrap()[0].name,
        "deep"
    );
    // 볼륨 루트가 등록돼 있으면 그 안의 폴더는 하위 중복
    assert!(matches!(
        env.ingest.add_folder(env.path("deep").to_str().unwrap()),
        Err(IngestError::NestedFolder {
            inside_existing: true
        })
    ));
}

#[test]
fn t_scan_12_two_folders_on_one_volume_do_not_mark_each_others_files_missing() {
    // Arrange
    let env = Env::new();
    env.write("one/a.jpg", 1);
    env.write("two/b.jpg", 2);
    let one = add(&env, "one");
    let two = add(&env, "two");
    completed(scan(&env, one));
    completed(scan(&env, two));
    env.clock.advance(5_000);

    // Act: 한 폴더만 재스캔
    let counts = completed(scan(&env, one));

    // Assert: 다른 폴더의 파일은 건드리지 않는다
    assert_eq!(counts.missing, 0);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"),
        0
    );
}

#[test]
fn t_scan_13_network_volume_root_unreachable_marks_the_volume_offline() {
    // Arrange: 네트워크 볼륨(가짜) 위의 폴더 — 공유가 응답하지 않으면 삭제가 아니라 오프라인
    let env = Env::new();
    let nas = env.dir.path().join("nas");
    fs::create_dir_all(nas.join("media")).unwrap();
    let nas = fs::canonicalize(&nas).unwrap();
    env.volumes.add("nas-1", VolumeKind::Network, &nas);
    fs::write(nas.join("media").join("x.jpg"), b"x").unwrap();
    let id = env
        .ingest
        .add_folder(nas.join("media").to_str().unwrap())
        .unwrap()
        .folder_id;
    completed(scan(&env, id));
    fs::remove_dir_all(nas.join("media")).unwrap();
    env.clock.advance(5_000);

    // Act
    let outcome = scan(&env, id);

    // Assert
    assert_eq!(outcome, ScanOutcome::RootUnreadable);
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.availability, mam_core::scan::Availability::Offline);
    assert_eq!(view.volume_kind, VolumeKind::Network);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"),
        0
    );
}
