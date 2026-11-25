//! `mam_core::assets::detail` (T-asset-08 ~ 12) — 플러그인 호스트용 자산 상세.
//!
//! 분류: Happy / Boundary(위치·메타데이터 없음) / Business rule(경로 미노출, 오프라인·삭제는 데이터) / Validation(파일명 정제) / Performance-plan

mod common;

use common::env::Env;
use common::purge::{exec, insert_assets};
use mam_core::assets::{MAX_FILE_NAME_CHARS, detail, sanitize_file_name};

fn setup(media: &str) -> (Env, i64) {
    let env = Env::new();
    std::fs::create_dir_all(env.path("lib")).unwrap();
    env.ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap();
    let id = insert_assets(&env, "vol-a", "lib/secret_dir", 1, media)[0];
    (env, id)
}

#[test]
fn t_asset_08_detail_returns_all_fields_for_a_present_online_asset() {
    // Arrange
    let (env, id) = setup("video");
    exec(
        &env,
        &format!(
            "INSERT INTO media_metadata (asset_id, width, height, duration_ms, captured_at) VALUES ({id}, 1920, 1080, 5000, 1700000000000) ON CONFLICT(asset_id) DO UPDATE SET width = 1920, height = 1080, duration_ms = 5000, captured_at = 1700000000000"
        ),
    );
    // Act
    let d = env.read(|c| detail(c, id).unwrap()).unwrap();
    // Assert
    assert_eq!(
        (
            d.media_type.as_str(),
            d.status.as_str(),
            d.availability.as_str()
        ),
        ("video", "active", "online")
    );
    assert_eq!(d.presence.as_deref(), Some("present"));
    assert_eq!(
        d.file_name.as_deref(),
        Some(format!("f{id}_0.jpg").as_str())
    );
    assert_eq!(
        (d.captured_at, d.duration_ms, d.width, d.height),
        (Some(1_700_000_000_000), Some(5000), Some(1920), Some(1080))
    );
    assert!(
        !format!("{d:?}").contains("secret_dir"),
        "경로는 나가지 않는다: {d:?}"
    );
}

#[test]
fn t_asset_09_offline_volume_missing_file_and_inactive_status_are_reported_as_data() {
    let (env, id) = setup("image");
    exec(&env, "UPDATE volumes SET availability = 'offline'");
    exec(
        &env,
        &format!("UPDATE file_locations SET presence = 'missing' WHERE asset_id = {id}"),
    );
    exec(
        &env,
        &format!("UPDATE media_assets SET status = 'superseded' WHERE asset_id = {id}"),
    );
    let d = env.read(|c| detail(c, id).unwrap()).unwrap();
    assert_eq!(
        (
            d.availability.as_str(),
            d.presence.as_deref(),
            d.status.as_str()
        ),
        ("offline", Some("missing"), "superseded")
    );
}

#[test]
fn t_asset_10_no_active_location_or_metadata_or_asset_gives_nulls_or_none() {
    let (env, id) = setup("audio");
    exec(
        &env,
        &format!("DELETE FROM media_metadata WHERE asset_id = {id}"),
    );
    let with_loc = env.read(|c| detail(c, id).unwrap()).unwrap();
    assert_eq!(
        (
            with_loc.captured_at,
            with_loc.duration_ms,
            with_loc.width,
            with_loc.height
        ),
        (None, None, None, None)
    );
    exec(
        &env,
        &format!("UPDATE file_locations SET detached_at = 5 WHERE asset_id = {id}"),
    );
    let no_loc = env.read(|c| detail(c, id).unwrap()).unwrap();
    assert_eq!(
        (
            no_loc.file_name,
            no_loc.presence,
            no_loc.availability.as_str()
        ),
        (None, None, "offline")
    );
    assert_eq!(env.read(|c| detail(c, 987_654).unwrap()), None);
}

#[test]
fn t_asset_11_file_name_is_last_component_without_control_chars_capped_at_200() {
    assert_eq!(sanitize_file_name("a/b\\c.jpg").as_deref(), Some("c.jpg"));
    assert_eq!(
        sanitize_file_name("x/\u{7}na\nme\r.jpg").as_deref(),
        Some("name.jpg")
    );
    assert_eq!(
        sanitize_file_name("dir/\u{1}\u{2}"),
        None,
        "제어 문자만 남으면 None"
    );
    assert_eq!(sanitize_file_name("dir/"), None);
    let long = format!("d/{}", "가".repeat(300));
    assert_eq!(
        sanitize_file_name(&long).unwrap().chars().count(),
        MAX_FILE_NAME_CHARS
    );
}

#[test]
fn t_asset_12_detail_query_uses_primary_keys_and_the_asset_location_index() {
    let (env, _) = setup("image");
    let plan: Vec<String> = env.read(|c| {
        let mut stmt = c
            .prepare(
                "EXPLAIN QUERY PLAN SELECT a.media_type, l.rel_path, v.availability FROM media_assets a
                 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
                 LEFT JOIN file_locations l ON l.location_id = (SELECT location_id FROM file_locations WHERE asset_id = a.asset_id AND detached_at IS NULL ORDER BY location_id LIMIT 1)
                 LEFT JOIN volumes v ON v.volume_id = l.volume_id WHERE a.asset_id = ?1",
            )
            .unwrap();
        stmt.query_map([1i64], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect()
    });
    assert!(
        !plan.iter().any(|d| d.starts_with("SCAN")),
        "full scan: {plan:?}"
    );
}
