//! 사이드바 "전체 미디어" 개수 (`summary().total_assets`) — 별점 개수와 같은 기준 (T-total-01 ~ 04).
//!
//! 분류: Business rule (01 purge 로 고아가 된 자산 제외, 02 폴더 제거, 04 숨김·고아 혼합) /
//! Boundary (03 파일 없음 위치 제외, 04 합계 = 별점 합).
//! 실제 SQLite + 임시 디렉터리 볼륨. 캐시는 작업 generation 이 아니라 직접 SQL 변경 시에만 문제가 되므로,
//! 값을 읽기 전 `jobs` 상태가 바뀌지 않는 경로는 `set_folder_hidden` 등 기존 API 로 generation 을 올린다.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::env::Env;
use mam_core::scan::{MediaTypeSet, scan_folder};
use mam_core::userdata::UserData;

fn scan(env: &Env, id: i64) {
    env.clock.advance(5_000);
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
}

fn add(env: &Env, dir: &str) -> i64 {
    env.ingest
        .add_folder_with_types(env.path(dir).to_str().unwrap(), MediaTypeSet::ALL)
        .unwrap()
        .folder_id
}

fn total(env: &Env) -> i64 {
    env.ingest.summary().unwrap().total_assets
}

fn rating_sum(env: &Env) -> i64 {
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    ud.rating_counts().unwrap().iter().sum::<u64>() as i64
}

/// 요약 캐시(2초 TTL + generation)를 우회하려면 generation 이 바뀌어야 한다 - 숨김 토글로 올린다 (t_hide_core_06 방식)
fn bump_generation(env: &Env, folder: i64) {
    env.ingest.set_folder_hidden(folder, true).unwrap();
    env.ingest.set_folder_hidden(folder, false).unwrap();
}

#[test]
fn t_total_01_purged_assets_are_not_counted_and_total_equals_rating_sum() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/b.mp4", 2);
    env.write("lib/c.wav", 3);
    let id = add(&env, "lib");
    scan(&env, id);
    assert_eq!(total(&env), 3);

    // Act: 이미지 제외 + 정리 -> 제외한 유형의 자산은 연결 데이터와 위치가 지워지고 deleted 로 남는다
    let types = MediaTypeSet {
        image: false,
        video: true,
        audio: true,
    };
    env.ingest.set_folder_media_types(id, types, true).unwrap();
    bump_generation(&env, id);

    // Assert
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"),
        2
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'deleted'"),
        1,
        "자산 행은 deleted 로 보존"
    );
    assert_eq!(total(&env), 2);
    assert_eq!(total(&env), rating_sum(&env));
}

#[test]
fn t_total_02_removing_a_folder_drops_its_assets_from_the_total() {
    // Arrange
    let env = Env::new();
    env.write("a/x.jpg", 1);
    env.write("a/y.jpg", 2);
    env.write("b/z.jpg", 3);
    let a = add(&env, "a");
    let b = add(&env, "b");
    scan(&env, a);
    scan(&env, b);
    assert_eq!(total(&env), 3);

    // Act
    env.ingest.remove_folder(a).unwrap();
    env.ingest.wait_removals();
    bump_generation(&env, b);

    // Assert
    assert_eq!(total(&env), 1);
    assert_eq!(total(&env), rating_sum(&env));
}

#[test]
fn t_total_03_missing_presence_location_is_excluded() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/b.jpg", 2);
    let id = add(&env, "lib");
    scan(&env, id);
    assert_eq!(total(&env), 2);

    // Act
    env.pool
        .write(mam_db::Priority::High, |c| {
            c.execute(
                "UPDATE file_locations SET presence = 'missing' WHERE rel_path LIKE '%a.jpg'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    bump_generation(&env, id);

    // Assert
    assert_eq!(total(&env), 1);
    assert_eq!(total(&env), rating_sum(&env));
}

#[test]
fn t_total_04_hidden_folder_and_orphans_are_both_excluded() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/b.mp4", 2);
    env.write("other/c.jpg", 3);
    let lib = add(&env, "lib");
    let other = add(&env, "other");
    scan(&env, lib);
    scan(&env, other);

    // Act: other 의 이미지는 purge(고아), lib 는 숨김
    let types = MediaTypeSet {
        image: false,
        video: true,
        audio: true,
    };
    env.ingest
        .set_folder_media_types(other, types, true)
        .unwrap();
    env.ingest.set_folder_hidden(lib, true).unwrap();

    // Assert
    assert_eq!(total(&env), 0);
    assert_eq!(total(&env), rating_sum(&env));
    env.ingest.set_folder_hidden(lib, false).unwrap();
    assert_eq!(total(&env), 2);
    assert_eq!(total(&env), rating_sum(&env));
}
