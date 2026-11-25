//! 해시 후 병합(D6, T-proc-09)과 폴더 재등록 재연결(D9, T-proc-14).
//!
//! 분류: Business rule (병합 조건 충족/불충족, 재연결 조건) / Concurrency (일시정지로 순서 고정) / Boundary (사본은 병합하지 않음).
//! 실제 SQLite + 임시 볼륨. 병합은 파일 ID 가 없는 볼륨(`MetadataOnly`)에서 스캔 한 번 안에 이름이 바뀐 경우를 재현한다.

mod common;

use std::fs;
use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use common::proc_env::{ProcEnv, no_tools, png_bytes};
use mam_core::MetadataOnly;
use mam_core::jobs::JobKind;
use mam_core::scan::{ScanCounts, ScanOutcome, scan_folder};
use mam_media::{ThumbSize, thumbnail_path};

fn rescan(env: &Env, folder: i64) -> ScanCounts {
    match scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false)).unwrap() {
        ScanOutcome::Completed { counts } => counts,
        other => panic!("expected Completed, got {other:?}"),
    }
}

/// 사용자 데이터 종류 (신규 자산에 붙이는 SQL).
const USER_DATA: [(&str, &str); 6] = [
    (
        "tag",
        "INSERT INTO tags (tag_id, name, name_key) VALUES (1, 'x', 'x');
         INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (:id, 1, 0)",
    ),
    (
        "memo",
        "INSERT INTO asset_user_data (asset_id, memo, favorite, updated_at) VALUES (:id, 'note', 0, 0)",
    ),
    (
        "favorite",
        "INSERT INTO asset_user_data (asset_id, memo, favorite, updated_at) VALUES (:id, '', 1, 0)",
    ),
    (
        "title",
        "INSERT INTO asset_user_data (asset_id, memo, favorite, title, updated_at) VALUES (:id, '', 0, 'my title', 0)",
    ),
    (
        "rating",
        "INSERT INTO asset_user_data (asset_id, memo, favorite, rating, updated_at) VALUES (:id, '', 0, 3, 0)",
    ),
    (
        "collection",
        "INSERT INTO collections (collection_id, name, name_key, created_at) VALUES (1, 'c', 'c', 0);
         INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES (1, :id, 0)",
    ),
];

struct Moved {
    p: ProcEnv,
    old_id: i64,
    new_id: i64,
}

/// 같은 스캔 안에서 이름이 바뀐 상황을 만든다. `user_data` 가 있으면 신규 자산(해시 전)에 붙인다.
fn rename_during_one_scan(user_data: Option<&str>) -> Moved {
    let env = Env::with_identity(|_, _| {}, std::sync::Arc::new(MetadataOnly));
    let p = ProcEnv::with_env(env, no_tools());
    let old = p.put("lib/old.png", &png_bytes(80, 60));
    // 같은 크기·같은 앞뒤 체크섬의 짝 파일 - 짝이 있어야 old 가 전체 해시를 갖는다 (크기가 유일하면 전체 해시를 생략한다)
    p.put("lib/pair.png", &png_bytes(80, 60));
    let folder = p
        .env
        .ingest
        .add_folder(p.env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle());
    let old_id = p.asset_of("old.png");
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'done'"),
        2
    );

    // 파일 감시가 이름 변경 이벤트로 옛 위치를 먼저 missing 으로 바꾸면(비동기) 병합 전제가 달라져 간헐 실패한다
    // (t_proc_09 flaky) - 이 시험은 재스캔 한 번 안의 이름 변경을 재현하므로 감시 이벤트를 끝까지 막는다.
    let _watch_off = p.env.ingest.suppress_watch(&p.env.path("lib"));
    // 해시 작업이 스캔보다 먼저 돌지 않게 멈춘 뒤 이름을 바꾸고 재스캔 (파일 ID 가 없어 신규 자산이 된다)
    p.env.ingest.jobs().pause(Some(JobKind::Hash)).unwrap();
    // 실시간 감시가 이름 변경을 먼저 반영하면 아래 재스캔이 (0, 0) 이 된다 — 부하가 크면 디바운스(2초) 안에
    // 재스캔이 끝나지 않아 간헐 실패했다. 이 폴더의 감시 이벤트를 재스캔이 끝날 때까지 억제한다.
    let _quiet = p.env.ingest.suppress_watch(&p.env.path("lib"));
    fs::rename(&old, p.env.path("lib/new.png")).unwrap();
    p.env.clock.advance(60_000); // 주입 시계: 이전 스캔보다 늦은 시각이어야 "이번에 못 본" 위치를 가린다
    let counts = rescan(&p.env, folder);
    assert_eq!((counts.created, counts.missing), (1, 1), "{counts:?}");
    let new_id = p.asset_of("new.png");
    assert_ne!(new_id, old_id);
    if let Some(sql) = user_data {
        let sql = sql.replace(":id", &new_id.to_string());
        p.env
            .pool
            .write(mam_db::Priority::High, move |c| {
                c.execute_batch(&sql)?;
                Ok(())
            })
            .unwrap();
    }

    // 해시 재개 -> 신규 자산 해시 -> 병합 판단
    p.env.ingest.jobs().resume(Some(JobKind::Hash)).unwrap();
    assert!(p.wait_idle());
    p.shutdown();
    Moved { p, old_id, new_id }
}

#[test]
fn t_proc_09_merges_into_the_missing_asset_when_the_new_asset_has_no_user_data() {
    // Arrange / Act
    let m = rename_during_one_scan(None);
    let (p, old_id, new_id) = (&m.p, m.old_id, m.new_id);

    // Assert: 기존 자산이 새 경로의 위치를 이어받고, 신규 자산은 소프트 삭제
    let loc: (String, String) = p.env.read(|c| {
        c.query_row(
            "SELECT rel_path, presence FROM file_locations WHERE asset_id = ?1 AND detached_at IS NULL",
            [old_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    });
    assert!(loc.0.ends_with("new.png"), "{loc:?}");
    assert_eq!(loc.1, "present");
    assert_eq!(
        p.env.count(&format!(
            "SELECT COUNT(*) FROM media_assets WHERE asset_id = {old_id} AND status = 'active'"
        )),
        1
    );
    assert_eq!(
        p.env.count(&format!(
            "SELECT COUNT(*) FROM media_assets WHERE asset_id = {new_id} AND status = 'deleted'"
        )),
        1
    );
    assert_eq!(
        p.env.count(&format!(
            "SELECT COUNT(*) FROM file_locations WHERE asset_id = {new_id} AND detached_at IS NULL"
        )),
        0
    );
    // 신규 자산의 처리 상태·작업·산출물은 정리됨, 기존 자산의 파생물은 그대로
    assert!(p.deriv(new_id, "thumb").is_none());
    assert!(!thumbnail_path(&p.cache, new_id, ThumbSize::Small).exists());
    assert_eq!(p.deriv(old_id, "thumb").unwrap().0, "done");
    // 두 자산 모두 색인에 알려진다 (최신 revision 의 upsert 이벤트 - 색인 쪽이 비활성 자산을 지운다)
    for id in [old_id, new_id] {
        let rev = p.revision(id);
        assert_eq!(
            p.env.count(&format!(
                "SELECT COUNT(*) FROM search_outbox WHERE asset_id = {id} AND revision = {rev}"
            )),
            1,
            "asset {id}"
        );
    }
    // 활성 위치는 파일마다 하나 (old 가 new.png 를 이어받고 pair.png) - 사라진 위치는 이력으로만 남는다
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL"),
        2
    );
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL AND presence = 'missing'"
        ),
        0
    );
}

#[test]
fn t_proc_09b_does_not_merge_when_the_new_asset_has_any_user_data() {
    for (label, sql) in USER_DATA {
        // Arrange / Act
        let m = rename_during_one_scan(Some(sql));
        let (p, old_id, new_id) = (&m.p, m.old_id, m.new_id);

        // Assert: old·new·pair 모두 활성, 신규 자산은 새 위치를 유지하고 옛 자산은 missing 으로 남는다 (사용자가 판단)
        assert_eq!(
            p.env
                .count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"),
            3,
            "{label}"
        );
        assert_eq!(
            p.env.count(&format!("SELECT COUNT(*) FROM file_locations WHERE asset_id = {new_id} AND detached_at IS NULL AND presence = 'present'")),
            1,
            "{label}"
        );
        assert_eq!(
            p.env.count(&format!("SELECT COUNT(*) FROM file_locations WHERE asset_id = {old_id} AND detached_at IS NULL AND presence = 'missing'")),
            1,
            "{label}"
        );
        // 해시는 기록됐다 (중복 판정에는 쓸 수 있다)
        assert_eq!(
            p.env
                .count("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'done'"),
            3,
            "{label}"
        );
    }
}

#[test]
fn t_proc_09c_identical_copies_that_both_exist_are_never_merged() {
    // Arrange: 같은 내용의 파일 두 개가 함께 존재 (완전 중복 - 자산은 파일마다 따로)
    let p = ProcEnv::new(no_tools());
    let bytes = png_bytes(64, 64);
    p.put("lib/a.png", &bytes);
    p.put("lib/b.png", &bytes);
    p.env
        .ingest
        .add_folder(p.env.path("lib").to_str().unwrap())
        .unwrap();

    // Act
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle());
    p.shutdown();

    // Assert: 둘 다 활성·같은 해시, 위치도 각자 보존
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"),
        2
    );
    assert_eq!(
        p.env
            .count("SELECT COUNT(DISTINCT content_hash) FROM media_assets"),
        1
    );
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL AND presence = 'present'"
        ),
        2
    );
}

fn tag(env: &Env, asset_id: i64) {
    env.pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT OR IGNORE INTO tags (tag_id, name, name_key) VALUES (1, 'x', 'x')",
                [],
            )?;
            c.execute(
                "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, 1, 0)",
                [asset_id],
            )?;
            Ok(())
        })
        .unwrap();
}

/// 예전 `remove_folder`(위치만 detach, 자산·태그 보존)가 남기던 상태를 만든다. 지금의 제거는 연결 데이터까지 지우지만
/// (T-rmv-*), 그렇게 남은 옛 데이터와 "관리 폴더 밖으로 이동한 자산" 의 재연결 경로(D9, `detached_asset_at`)는 그대로 남아 있다.
fn legacy_detach_folder(env: &Env, folder: i64) {
    env.pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "UPDATE file_locations SET detached_at = 1 WHERE detached_at IS NULL",
                [],
            )?;
            c.execute("DELETE FROM managed_folders WHERE folder_id = ?1", [folder])?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_proc_14_re_adding_a_folder_with_legacy_detached_locations_reattaches_assets_with_their_user_data()
 {
    // Arrange: 스캔 -> 태그 -> 폴더 제거(위치 detach, 자산 보존)
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/b.png", 2);
    let before = snapshot(&env.vol);
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    assert_eq!(rescan(&env, folder).created, 2);
    let (a, b) = (
        env.count("SELECT asset_id FROM file_locations WHERE rel_path LIKE '%a.jpg'"),
        env.count("SELECT asset_id FROM file_locations WHERE rel_path LIKE '%b.png'"),
    );
    tag(&env, a);
    legacy_detach_folder(&env, folder);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL"),
        0
    );
    let rev_a = env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {a}"
    ));

    // Act: 같은 폴더를 다시 등록해 스캔
    let again = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    let counts = rescan(&env, again);

    // Assert: 새 자산 없이 기존 자산에 다시 연결 - 태그·자산 id 유지, 색인에 알림
    assert_eq!((counts.created, counts.updated), (0, 2), "{counts:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 2);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL"),
        2
    );
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM file_locations WHERE asset_id = {a} AND detached_at IS NULL"
        )),
        1
    );
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM file_locations WHERE asset_id = {b} AND detached_at IS NULL"
        )),
        1
    );
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM asset_tags WHERE asset_id = {a}"
        )),
        1
    );
    let rev_after = env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {a}"
    ));
    assert!(rev_after > rev_a);
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM search_outbox WHERE asset_id = {a} AND revision = {rev_after}"
        )),
        1
    );
    // 원본은 그대로
    assert_eq!(snapshot(&env.vol), before);
    // 한 번 더 스캔해도 변화 없음 (멱등)
    let third = rescan(&env, again);
    assert_eq!(
        (third.created, third.updated, third.unchanged),
        (0, 0, 2),
        "{third:?}"
    );
}

#[test]
fn t_proc_14b_changed_file_after_legacy_detach_is_a_new_asset_and_the_old_one_stays_detached() {
    // Arrange
    let env = Env::new();
    let path = env.write("lib/a.jpg", 1);
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    rescan(&env, folder);
    let old = env.count("SELECT asset_id FROM media_assets");
    tag(&env, old);
    legacy_detach_folder(&env, folder);
    // 제거된 사이에 파일이 바뀜 (크기 변경)
    fs::write(&path, vec![9u8; 500]).unwrap();

    // Act
    let again = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    let counts = rescan(&env, again);

    // Assert: 기존 규칙대로 신규 자산 - 옛 자산(태그 포함)은 위치 없이 보존
    assert_eq!((counts.created, counts.updated), (1, 0), "{counts:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 2);
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM file_locations WHERE asset_id = {old} AND detached_at IS NULL"
        )),
        0
    );
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM asset_tags WHERE asset_id = {old}"
        )),
        1
    );
}

#[test]
fn t_proc_14c_superseded_or_already_attached_assets_are_not_reattached() {
    // Arrange: 위치가 detach 된 자산이 이미 superseded
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    rescan(&env, folder);
    legacy_detach_folder(&env, folder);
    env.pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE media_assets SET status = 'superseded'", [])?;
            Ok(())
        })
        .unwrap();

    // Act
    let again = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    let counts = rescan(&env, again);

    // Assert: 비활성 자산에는 연결하지 않는다 -> 신규 자산
    assert_eq!((counts.created, counts.updated), (1, 0), "{counts:?}");
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"),
        1
    );
}

#[test]
fn t_rating_merge_01_rating_only_asset_is_kept_and_an_unrated_row_still_merges() {
    // Arrange / Act: 별점만 있는 신규 자산은 사용자 데이터가 있는 것으로 본다 (병합하면 별점이 사라진다)
    let rated = rename_during_one_scan(Some(
        "INSERT INTO asset_user_data (asset_id, rating, updated_at) VALUES (:id, 5, 0)",
    ));
    // 별점 0(미평가) 행만 있는 신규 자산은 사용자 데이터가 아니다
    let unrated = rename_during_one_scan(Some(
        "INSERT INTO asset_user_data (asset_id, rating, updated_at) VALUES (:id, 0, 0)",
    ));

    // Assert
    let status = |m: &Moved| {
        m.p.env.count(&format!(
            "SELECT COUNT(*) FROM media_assets WHERE asset_id = {} AND status = 'active'",
            m.new_id
        ))
    };
    assert_eq!(
        status(&rated),
        1,
        "rating-only asset must survive the merge"
    );
    assert_eq!(
        rated.p.env.count(&format!(
            "SELECT rating FROM asset_user_data WHERE asset_id = {}",
            rated.new_id
        )),
        5
    );
    assert_eq!(
        status(&unrated),
        0,
        "rating 0 is not user data -> merged away"
    );
}
