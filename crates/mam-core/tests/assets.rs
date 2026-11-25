//! `mam_core::assets` 서비스 경계 (T-asset-01 ~ 07).
//!
//! 분류: Happy (exists·brief·same_content 기본) / Boundary (해시 없음, 사본 상한, 위치 없음) / Validation (없는 자산) /
//! Business rule (비활성·삭제 자산, 경로는 파일 이름만) / Performance-plan (해시 인덱스). 실제 SQLite.

mod common;

use common::env::Env;
use common::purge::{exec, insert_assets};
use mam_core::assets::{MAX_SAME_CONTENT, brief, exists, same_content};
use rusqlite::Connection;

/// 볼륨 `vol-a` 가 등록된 환경 (관리 폴더 하나를 추가하면 볼륨이 등록된다)
fn env_with_volume() -> Env {
    let env = Env::new();
    std::fs::create_dir_all(env.path("lib")).unwrap();
    env.ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap();
    env
}

fn one<T>(env: &Env, f: impl FnOnce(&Connection) -> T) -> T {
    env.read(f)
}

fn set_hash(env: &Env, ids: &[i64], hash_byte: u8) {
    let list = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let hex = format!("{hash_byte:02x}").repeat(32);
    exec(
        env,
        &format!(
            "UPDATE media_assets SET hash_state = 'done', hash_algo = 'blake3', content_hash = X'{hex}' WHERE asset_id IN ({list})"
        ),
    );
}

#[test]
fn t_asset_01_exists_is_true_for_any_status_and_false_for_unknown_ids() {
    // Arrange
    let env = env_with_volume();
    let ids = insert_assets(&env, "vol-a", "lib", 2, "image");
    exec(
        &env,
        &format!(
            "UPDATE media_assets SET status = 'deleted' WHERE asset_id = {}",
            ids[1]
        ),
    );

    // Act / Assert
    assert!(one(&env, |c| exists(c, ids[0]).unwrap()));
    assert!(
        one(&env, |c| exists(c, ids[1]).unwrap()),
        "deleted rows still exist"
    );
    assert!(!one(&env, |c| exists(c, 987_654).unwrap()));
    assert!(!one(&env, |c| exists(c, -1).unwrap()));
}

#[test]
fn t_asset_02_brief_returns_the_file_name_only_and_metadata_when_present() {
    // Arrange
    let env = env_with_volume();
    let ids = insert_assets(&env, "vol-a", "lib/sub", 1, "video");
    exec(
        &env,
        &format!(
            "INSERT INTO media_metadata (asset_id, width, height, duration_ms, captured_at) VALUES ({}, 1, 1, 5000, 1700000000000) ON CONFLICT(asset_id) DO UPDATE SET duration_ms = 5000, captured_at = 1700000000000",
            ids[0]
        ),
    );

    // Act
    let b = one(&env, |c| brief(c, ids[0]).unwrap()).unwrap();

    // Assert: 디렉터리는 나가지 않는다
    assert_eq!(
        (b.media_type.as_str(), b.status.as_str()),
        ("video", "active")
    );
    assert_eq!(
        b.file_name.as_deref(),
        Some(format!("f{}_0.jpg", ids[0]).as_str())
    );
    assert_eq!(
        (b.captured_at, b.duration_ms),
        (Some(1_700_000_000_000), Some(5000))
    );
    assert!(
        !format!("{b:?}").contains("lib"),
        "no directory part: {b:?}"
    );
}

#[test]
fn t_asset_03_brief_handles_missing_asset_missing_location_missing_metadata_and_backslashes() {
    // Arrange
    let env = env_with_volume();
    let ids = insert_assets(&env, "vol-a", "lib", 3, "image");
    exec(
        &env,
        &format!("DELETE FROM media_metadata WHERE asset_id = {}", ids[0]),
    );
    exec(
        &env,
        &format!(
            "UPDATE file_locations SET detached_at = 5 WHERE asset_id = {}",
            ids[1]
        ),
    );
    exec(
        &env,
        &format!(
            "UPDATE file_locations SET rel_path = 'a\\b\\c.jpg' WHERE asset_id = {}",
            ids[2]
        ),
    );

    // Act / Assert
    assert_eq!(one(&env, |c| brief(c, 987_654).unwrap()), None);
    let no_meta = one(&env, |c| brief(c, ids[0]).unwrap()).unwrap();
    assert_eq!((no_meta.captured_at, no_meta.duration_ms), (None, None));
    assert!(no_meta.file_name.is_some());
    let detached = one(&env, |c| brief(c, ids[1]).unwrap()).unwrap();
    assert_eq!(
        detached.file_name, None,
        "detached location is not an active location"
    );
    let backslash = one(&env, |c| brief(c, ids[2]).unwrap()).unwrap();
    assert_eq!(backslash.file_name.as_deref(), Some("c.jpg"));
}

#[test]
fn t_asset_04_brief_reports_inactive_status_instead_of_hiding_the_asset() {
    // Arrange
    let env = env_with_volume();
    let ids = insert_assets(&env, "vol-a", "lib", 1, "audio");
    exec(
        &env,
        &format!(
            "UPDATE media_assets SET status = 'deleted' WHERE asset_id = {}",
            ids[0]
        ),
    );

    // Act
    let b = one(&env, |c| brief(c, ids[0]).unwrap()).unwrap();

    // Assert
    assert_eq!(
        (b.status.as_str(), b.media_type.as_str()),
        ("deleted", "audio")
    );
}

#[test]
fn t_asset_05_same_content_is_none_for_inactive_or_unknown_and_self_only_without_a_hash() {
    // Arrange
    let env = env_with_volume();
    let ids = insert_assets(&env, "vol-a", "lib", 2, "image");
    exec(
        &env,
        &format!(
            "UPDATE media_assets SET status = 'deleted' WHERE asset_id = {}",
            ids[1]
        ),
    );

    // Act / Assert
    assert_eq!(one(&env, |c| same_content(c, 987_654).unwrap()), None);
    assert_eq!(
        one(&env, |c| same_content(c, ids[1]).unwrap()),
        None,
        "inactive"
    );
    assert_eq!(
        one(&env, |c| same_content(c, ids[0]).unwrap()),
        Some(vec![ids[0]]),
        "no hash yet"
    );
}

#[test]
fn t_asset_06_same_content_lists_self_first_then_copies_capped_at_the_limit() {
    // Arrange: 1,003 개 중 같은 해시 — 상한은 사본 수에 적용된다 (자기 자신 제외)
    let env = env_with_volume();
    let n = usize::try_from(MAX_SAME_CONTENT).unwrap() + 3;
    let ids = insert_assets(&env, "vol-a", "lib", n, "image");
    set_hash(&env, &ids, 0xab);
    let other = insert_assets(&env, "vol-a", "lib2", 1, "image");
    set_hash(&env, &other, 0xcd);

    // Act
    let all = one(&env, |c| same_content(c, ids[0]).unwrap()).unwrap();
    let lone = one(&env, |c| same_content(c, other[0]).unwrap()).unwrap();

    // Assert
    assert_eq!(all[0], ids[0]);
    assert_eq!(all.len(), usize::try_from(MAX_SAME_CONTENT).unwrap() + 1);
    assert!(all.iter().all(|id| ids.contains(id)) && !all.contains(&other[0]));
    assert_eq!(lone, vec![other[0]]);
}

#[test]
fn t_asset_07_same_content_copy_lookup_uses_the_hash_index() {
    // Arrange
    let env = env_with_volume();

    // Act
    let plan: Vec<String> = one(&env, |c| {
        let mut stmt = c
            .prepare(
                "EXPLAIN QUERY PLAN SELECT asset_id FROM media_assets
                 WHERE hash_algo = ?1 AND content_hash = ?2 AND asset_id <> ?3 LIMIT ?4",
            )
            .unwrap();
        stmt.query_map(rusqlite::params!["blake3", vec![1u8], 1i64, 10i64], |r| {
            r.get::<_, String>(3)
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    });

    // Assert
    let joined = plan.join(" | ");
    assert!(joined.contains("idx_media_assets_hash"), "{joined}");
    assert!(!plan.iter().any(|d| d.starts_with("SCAN")), "{joined}");
}
