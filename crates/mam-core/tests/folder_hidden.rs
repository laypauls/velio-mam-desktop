//! 관리 폴더 숨기기 저장·조회 (T-hide-core-01 ~ 09).
//!
//! 분류: Happy (01 목록 반영·직렬화) / Business rule (02 스캔 계속, 05 제외 목록) / Validation (03 없는 폴더·등록 해제 중) /
//! Concurrency (04 멱등) / Business rule (06~09 사이드바 개수가 갤러리 기본 보기와 같은 기준). 실제 SQLite + 임시 디렉터리 볼륨.

mod common;

use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use mam_core::scan::{IngestError, ScanOutcome, scan_folder};
use mam_core::userdata::{UpdateTagsInput, UserData};
use std::sync::Arc;

fn add(env: &Env, rel: &str) -> i64 {
    env.ingest
        .add_folder(env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id
}

fn exec(env: &Env, sql: &str) {
    let sql = sql.to_string();
    env.pool
        .write(mam_db::Priority::High, move |c| Ok(c.execute_batch(&sql)))
        .unwrap()
        .unwrap();
}

fn hidden_of(env: &Env, id: i64) -> bool {
    env.ingest
        .list_folders()
        .unwrap()
        .into_iter()
        .find(|f| f.folder_id == id)
        .unwrap()
        .hidden
}

#[test]
fn t_hide_core_01_list_folders_reports_hidden_and_serializes_it_as_camel_case() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("other/b.jpg", 2);
    let lib = add(&env, "lib");
    let other = add(&env, "other");
    assert!(!hidden_of(&env, lib), "기본값은 보임");

    // Act
    env.ingest.set_folder_hidden(lib, true).unwrap();

    // Assert
    assert!(hidden_of(&env, lib));
    assert!(!hidden_of(&env, other), "다른 폴더는 영향 없음");
    let json = serde_json::to_value(&env.ingest.list_folders().unwrap()[0]).unwrap();
    assert_eq!(json["hidden"], serde_json::json!(true), "{json}");
    env.ingest.set_folder_hidden(lib, false).unwrap();
    assert!(!hidden_of(&env, lib), "해제");
}

#[test]
fn t_hide_core_02_hiding_does_not_stop_scanning_and_never_touches_originals() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/b.png", 2);
    let lib = add(&env, "lib");
    env.ingest.set_folder_hidden(lib, true).unwrap();
    let before = snapshot(&env.vol);

    // Act: 재스캔 예약이 거부되지 않고, 스캔이 숨김 폴더 자산을 그대로 수집한다
    env.ingest.rescan_folder(lib).unwrap();
    let outcome = scan_folder(env.ingest.env(), lib, &|| false, &AtomicBool::new(false)).unwrap();

    // Assert
    assert!(matches!(outcome, ScanOutcome::Completed { .. }), "{outcome:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 2);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'upsert'"),
        2,
        "색인 이벤트도 평소처럼 기록된다"
    );
    assert_eq!(snapshot(&env.vol), before, "원본 불변");
}

#[test]
fn t_hide_core_03_unknown_and_removed_folders_are_not_found() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let lib = add(&env, "lib");

    // Act + Assert: 없는 폴더
    assert!(matches!(env.ingest.set_folder_hidden(9999, true), Err(IngestError::NotFound)));
    assert!(matches!(env.ingest.set_folder_hidden(-1, false), Err(IngestError::NotFound)));

    // 등록 해제 중(enabled = 0)
    exec(&env, &format!("UPDATE managed_folders SET enabled = 0 WHERE folder_id = {lib}"));
    assert!(matches!(env.ingest.set_folder_hidden(lib, true), Err(IngestError::NotFound)));
    exec(&env, &format!("UPDATE managed_folders SET enabled = 1 WHERE folder_id = {lib}"));
    assert!(!hidden_of(&env, lib), "거부된 호출은 값을 바꾸지 않는다");

    // 제거 완료 후
    env.ingest.remove_folder(lib).unwrap();
    env.ingest.wait_removals();
    assert!(matches!(env.ingest.set_folder_hidden(lib, true), Err(IngestError::NotFound)));
}

#[test]
fn t_hide_core_04_setting_the_same_value_again_is_idempotent() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let lib = add(&env, "lib");

    // Act + Assert
    env.ingest.set_folder_hidden(lib, false).unwrap();
    env.ingest.set_folder_hidden(lib, true).unwrap();
    env.ingest.set_folder_hidden(lib, true).unwrap();
    assert!(hidden_of(&env, lib));
    env.ingest.set_folder_hidden(lib, false).unwrap();
    env.ingest.set_folder_hidden(lib, false).unwrap();
    assert!(!hidden_of(&env, lib));
}

#[test]
fn t_hide_core_05_hidden_folder_ids_keeps_a_folder_that_is_being_removed() {
    // Arrange
    let env = Env::new();
    env.write("a/x.jpg", 1);
    env.write("b/y.jpg", 2);
    env.write("c/z.jpg", 3);
    let (a, b, c) = (add(&env, "a"), add(&env, "b"), add(&env, "c"));
    let ids = || {
        env.read(|conn| mam_core::scan::hidden_folder_ids(conn).unwrap())
    };
    assert!(ids().is_empty());

    // Act
    env.ingest.set_folder_hidden(a, true).unwrap();
    env.ingest.set_folder_hidden(c, true).unwrap();
    exec(&env, &format!("UPDATE managed_folders SET enabled = 0 WHERE folder_id = {c}"));

    // Assert: 등록 해제 중인 c 도 제외 목록에 남는다 (detach 도중 색인 문서가 다시 보이면 안 된다). 숨기지 않은 b 는 없다
    assert_eq!(ids(), [a as u64, c as u64]);
    let _ = b;
}

/// 폴더 `lib`(a, b) 와 `other`(c) 를 스캔해 두고 자산 id 를 돌려준다.
fn scanned_two_folders(env: &Env) -> (i64, i64, [i64; 3]) {
    env.write("lib/a.jpg", 1);
    env.write("lib/b.jpg", 2);
    env.write("other/c.jpg", 3);
    let (lib, other) = (add(env, "lib"), add(env, "other"));
    for f in [lib, other] {
        scan_folder(env.ingest.env(), f, &|| false, &AtomicBool::new(false)).unwrap();
    }
    let id = |n: &str| env.count(&format!("SELECT asset_id FROM file_locations WHERE rel_path LIKE '%{n}'"));
    (lib, other, [id("a.jpg"), id("b.jpg"), id("c.jpg")])
}

#[test]
fn t_hide_core_06_summary_total_assets_drops_when_hidden_and_returns_when_unhidden() {
    // Arrange
    let env = Env::new();
    let (lib, _other, _) = scanned_two_folders(&env);
    assert_eq!(env.ingest.summary().unwrap().total_assets, 3);

    // Act + Assert: 요약 캐시가 있어도 토글 직후 반영된다
    env.ingest.set_folder_hidden(lib, true).unwrap();
    assert_eq!(env.ingest.summary().unwrap().total_assets, 1);
    env.ingest.set_folder_hidden(lib, false).unwrap();
    assert_eq!(env.ingest.summary().unwrap().total_assets, 3);
}

#[test]
fn t_hide_core_07_rating_counts_exclude_hidden_folder_assets_including_unrated() {
    // Arrange: a=5점, b=미평가, c=5점
    let env = Env::new();
    let (lib, _other, [a, _b, c]) = scanned_two_folders(&env);
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    ud.set_rating(&[a, c], 5).unwrap();
    assert_eq!(ud.rating_counts().unwrap(), [1, 0, 0, 0, 0, 2]);

    // Act + Assert
    env.ingest.set_folder_hidden(lib, true).unwrap();
    assert_eq!(ud.rating_counts().unwrap(), [0, 0, 0, 0, 0, 1], "a(5점)·b(미평가) 가 빠진다");
    env.ingest.set_folder_hidden(lib, false).unwrap();
    assert_eq!(ud.rating_counts().unwrap(), [1, 0, 0, 0, 0, 2]);
}

#[test]
fn t_hide_core_08_tag_counts_exclude_hidden_folder_assets() {
    // Arrange
    let env = Env::new();
    let (lib, _other, [a, _b, c]) = scanned_two_folders(&env);
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    ud.update_asset_tags(UpdateTagsInput { asset_ids: vec![a, c], add: vec!["trip".into()], remove: vec![] })
        .unwrap();
    let count = || ud.list_tags().unwrap()[0].asset_count;
    assert_eq!(count(), 2);

    // Act + Assert
    env.ingest.set_folder_hidden(lib, true).unwrap();
    assert_eq!(count(), 1);
    env.ingest.set_folder_hidden(lib, false).unwrap();
    assert_eq!(count(), 2);
}

#[test]
fn t_hide_core_09_counts_still_exclude_a_hidden_folder_that_is_being_removed() {
    // Arrange: 등록 해제 중(enabled = 0) + 숨김 - 위치가 아직 detach 되기 전
    let env = Env::new();
    let (lib, _other, [a, _b, c]) = scanned_two_folders(&env);
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    ud.set_rating(&[a, c], 5).unwrap();
    env.ingest.set_folder_hidden(lib, true).unwrap();

    // Act
    exec(&env, &format!("UPDATE managed_folders SET enabled = 0 WHERE folder_id = {lib}"));

    // Assert
    assert_eq!(ud.rating_counts().unwrap(), [0, 0, 0, 0, 0, 1]);
    assert_eq!(env.ingest.summary().unwrap().total_assets, 1);
}
