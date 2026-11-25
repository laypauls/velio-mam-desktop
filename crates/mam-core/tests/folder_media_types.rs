//! 관리 폴더별 스캔 대상 유형 (T-mtype-01 ~ 18).
//!
//! 분류: Happy (01 기본값, 03 유형 지정 스캔, 11 다시 켜기, 12 정리 후 재연결) / Validation (02·13 모두 꺼짐, 14 없는 폴더) /
//! Business rule (04~07 유지 = 존재 확인만, 09 정리 = detach·태그 보존·outbox, 10 keep 무변경, 17 중첩 거부) /
//! External (08 감시 이벤트) / Boundary (15 개수 집계·인덱스 계획, 16 이전 시 유지) / Concurrency (18 스캔 실행 취소).
//! 실제 SQLite + 임시 디렉터리 볼륨. 스캔은 `scan_folder` 를 직접 호출한다 (작업 대기열 실행기를 띄우지 않는다).

mod common;

use std::fs;
use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use mam_core::scan::{
    IngestError, MediaTypeSet, PathAction, apply_path, scan_folder, set_media_types,
};

const ALL: MediaTypeSet = MediaTypeSet::ALL;
const VIDEO_ONLY: MediaTypeSet = MediaTypeSet {
    image: false,
    video: true,
    audio: false,
};
const NONE: MediaTypeSet = MediaTypeSet {
    image: false,
    video: false,
    audio: false,
};

fn scan(env: &Env, id: i64) {
    env.clock.advance(5_000);
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
}

/// lib/ 에 이미지·동영상·음성 1개씩 만들고 `types` 로 등록한다 (스캔 전)
fn setup(env: &Env, types: MediaTypeSet) -> i64 {
    env.write("lib/a.jpg", 1);
    env.write("lib/b.mp4", 2);
    env.write("lib/c.wav", 3);
    env.ingest
        .add_folder_with_types(env.path("lib").to_str().unwrap(), types)
        .unwrap()
        .folder_id
}

fn view_types(env: &Env, id: i64) -> MediaTypeSet {
    env.ingest
        .list_folders()
        .unwrap()
        .into_iter()
        .find(|f| f.folder_id == id)
        .unwrap()
        .media_types
}

/// (asset_id, presence) — 활성 위치만
fn loc(env: &Env, name: &str) -> Option<(i64, String)> {
    env.read(|c| {
        c.query_row(
            "SELECT asset_id, presence FROM file_locations WHERE rel_path LIKE ?1 AND detached_at IS NULL",
            [format!("%{name}")],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .ok()
    })
}

fn outbox_and_revisions(env: &Env) -> (i64, i64) {
    (
        env.count("SELECT COUNT(*) FROM search_outbox"),
        env.count("SELECT COALESCE(SUM(revision), 0) FROM media_assets"),
    )
}

fn tag(env: &Env, asset_id: i64, name: &str) {
    let name = name.to_string();
    env.pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO tags (name, name_key) VALUES (?1, ?1)",
                [&name],
            )?;
            let tag_id = c.last_insert_rowid();
            c.execute(
                "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, ?2, 1)",
                rusqlite::params![asset_id, tag_id],
            )?;
            Ok(())
        })
        .unwrap();
}

fn tags_of(env: &Env, asset_id: i64) -> i64 {
    env.count(&format!(
        "SELECT COUNT(*) FROM asset_tags WHERE asset_id = {asset_id}"
    ))
}

#[test]
fn t_mtype_01_default_is_all_on_and_existing_add_folder_keeps_every_type() {
    // Arrange / Act
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);

    // Assert: 뷰·DB 컬럼·직렬화 모두 전부 켜짐, 3개 다 등록
    assert_eq!(view_types(&env, id), ALL);
    assert_eq!(
        env.count("SELECT scan_image + scan_video + scan_audio FROM managed_folders"),
        3
    );
    let json = serde_json::to_value(&env.ingest.list_folders().unwrap()[0]).unwrap();
    assert_eq!(
        json["mediaTypes"],
        serde_json::json!({ "image": true, "video": true, "audio": true })
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 3);
    // add_folder(path) 도 전부 켜짐
    let env2 = Env::new();
    env2.write("lib/a.jpg", 1);
    let id2 = env2
        .ingest
        .add_folder(env2.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    assert_eq!(view_types(&env2, id2), ALL);
}

#[test]
fn t_mtype_02_add_with_all_off_is_rejected_and_registers_nothing() {
    let env = Env::new();
    env.write("lib/a.jpg", 1);

    let err = env
        .ingest
        .add_folder_with_types(env.path("lib").to_str().unwrap(), NONE)
        .unwrap_err();

    assert!(matches!(err, IngestError::InvalidInput("mediaTypes")), "{err:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), 0, "no scan scheduled");
}

#[test]
fn t_mtype_03_excluded_new_files_are_not_registered_by_scan() {
    // Arrange: 동영상만 대상
    let env = Env::new();
    let id = setup(&env, VIDEO_ONLY);
    let before = snapshot(&env.vol);

    // Act
    scan(&env, id);

    // Assert
    assert_eq!(view_types(&env, id), VIDEO_ONLY);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 1);
    assert!(loc(&env, "b.mp4").is_some());
    assert!(loc(&env, "a.jpg").is_none());
    assert!(loc(&env, "c.wav").is_none());
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_errors"), 0);
    assert_eq!(snapshot(&env.vol), before, "원본 내용·수정 시각 불변");
}

#[test]
fn t_mtype_04_keep_leaves_excluded_assets_present_after_rescan() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);
    let before = snapshot(&env.vol);
    let a_id = loc(&env, "a.jpg").unwrap().0;

    // Act: 이미지·음성 제외(유지) 후 재스캔
    let detached = env.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();
    scan(&env, id);

    // Assert
    assert_eq!(detached, 0);
    assert_eq!(loc(&env, "a.jpg"), Some((a_id, "present".into())));
    assert_eq!(loc(&env, "c.wav").unwrap().1, "present");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"), 3);
    assert_eq!(snapshot(&env.vol), before);
}

#[test]
fn t_mtype_05_kept_asset_becomes_missing_when_its_file_is_really_gone() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);
    env.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();

    // Act
    fs::remove_file(env.path("lib/a.jpg")).unwrap();
    scan(&env, id);

    // Assert: 지워진 것만 missing, 남은 제외 유형 파일은 present
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "missing");
    assert_eq!(loc(&env, "c.wav").unwrap().1, "present");
    assert_eq!(loc(&env, "b.mp4").unwrap().1, "present");
}

#[test]
fn t_mtype_06_kept_missing_asset_returns_to_present_when_the_file_reappears() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);
    env.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();
    let a = env.path("lib/a.jpg");
    let bytes = fs::read(&a).unwrap();
    fs::remove_file(&a).unwrap();
    scan(&env, id);
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "missing");
    let (outbox, _) = outbox_and_revisions(&env);

    // Act
    fs::write(&a, bytes).unwrap();
    scan(&env, id);

    // Assert
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "present");
    assert!(outbox_and_revisions(&env).0 > outbox, "복귀는 색인에 알린다");
}

#[test]
fn t_mtype_07_excluded_file_content_change_and_new_files_are_not_applied() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);
    env.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();
    let (asset, _) = loc(&env, "a.jpg").unwrap();
    let size_before = env.count(&format!("SELECT file_size FROM media_assets WHERE asset_id = {asset}"));

    // Act: 제외 유형 내용 변경 + 새 파일
    fs::write(env.path("lib/a.jpg"), vec![9u8; 500]).unwrap();
    env.write("lib/new.jpg", 7);
    scan(&env, id);

    // Assert: 동결된 스냅샷 — 크기 불변, 새 자산 없음
    assert_eq!(
        env.count(&format!("SELECT file_size FROM media_assets WHERE asset_id = {asset}")),
        size_before
    );
    assert!(loc(&env, "new.jpg").is_none());
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 3);
}

#[test]
fn t_mtype_08_watch_events_follow_the_same_rules() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);
    env.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();
    env.clock.advance(5_000);

    // Act / Assert 1: 제외 유형 새 파일 -> 등록하지 않는다
    env.write("lib/new.jpg", 7);
    let act = apply_path(env.ingest.env(), id, &env.path("lib/new.jpg")).unwrap();
    assert_eq!(act, PathAction::Ignored);
    assert!(loc(&env, "new.jpg").is_none());
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 3);

    // 2: 유지한 파일의 이벤트 -> 그대로 present
    env.clock.advance(5_000);
    apply_path(env.ingest.env(), id, &env.path("lib/a.jpg")).unwrap();
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "present");

    // 3: 유지한 파일이 지워진 이벤트 -> missing
    env.clock.advance(5_000);
    fs::remove_file(env.path("lib/a.jpg")).unwrap();
    let act = apply_path(env.ingest.env(), id, &env.path("lib/a.jpg")).unwrap();
    assert_eq!(act, PathAction::Missing(1));
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "missing");

    // 4: 다시 나타나면 present
    env.clock.advance(5_000);
    env.write("lib/a.jpg", 1);
    apply_path(env.ingest.env(), id, &env.path("lib/a.jpg")).unwrap();
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "present");

    // 5: 대상 유형 파일은 평소대로 등록
    env.write("lib/d.mp4", 4);
    let act = apply_path(env.ingest.env(), id, &env.path("lib/d.mp4")).unwrap();
    assert_eq!(act, PathAction::Reconciled);
    assert!(loc(&env, "d.mp4").is_some());
}

#[test]
fn t_mtype_09_purge_deletes_linked_data_of_only_excluded_types_and_writes_delete_events() {
    // Arrange: 정책 변경 — 정리(purge)는 제외한 유형의 연결 데이터를 영구 삭제한다 (예전에는 위치만 detach 하고 보존)
    let env = Env::new();
    let id = setup(&env, ALL);
    env.write("lib/sub/e.png", 5);
    scan(&env, id);
    let before = snapshot(&env.vol);
    let (a_id, _) = loc(&env, "a.jpg").unwrap();
    let (b_id, _) = loc(&env, "b.mp4").unwrap();
    tag(&env, a_id, "여행");
    tag(&env, b_id, "영상");
    let (outbox0, rev0) = outbox_and_revisions(&env);

    // Act: 이미지 제외 + 정리
    let types = MediaTypeSet {
        image: false,
        video: true,
        audio: true,
    };
    let removed = env.ingest.set_folder_media_types(id, types, true).unwrap();

    // Assert: 이미지 2건(하위 폴더 포함)의 위치·태그 연결이 사라지고 자산 행은 deleted 로 남는다. 다른 유형은 그대로. 원본 불변
    assert_eq!(removed, 2);
    assert!(loc(&env, "a.jpg").is_none());
    assert!(loc(&env, "e.png").is_none());
    assert_eq!(loc(&env, "b.mp4").unwrap(), (b_id, "present".into()));
    assert!(loc(&env, "c.wav").is_some());
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 4, "자산 행은 번호 재사용 방지로 남는다");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'deleted' AND media_type = 'image'"), 2);
    assert_eq!(tags_of(&env, a_id), 0, "태그 연결 삭제");
    assert_eq!(tags_of(&env, b_id), 1, "제외하지 않은 유형은 보존");
    assert_eq!(env.count("SELECT COUNT(*) FROM tags WHERE name = '여행'"), 1, "태그 이름은 남는다");
    let (outbox1, rev1) = outbox_and_revisions(&env);
    assert_eq!(outbox1 - outbox0, 2);
    assert_eq!(rev1 - rev0, 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'delete'"), 2);
    assert_eq!(snapshot(&env.vol), before);
    assert_eq!(view_types(&env, id), types);
}

#[test]
fn t_mtype_10_keep_changes_nothing_but_the_setting_and_schedules_a_rescan() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);
    let before = outbox_and_revisions(&env);
    env.ingest.jobs().pause(Some(mam_core::jobs::JobKind::Scan)).unwrap();

    // Act
    let detached = env.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();

    // Assert
    assert_eq!(detached, 0);
    assert_eq!(outbox_and_revisions(&env), before);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NOT NULL"), 0);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'"),
        1
    );
    // purge 지정이어도 제외되는 유형이 없으면(새로 켜기만) 아무것도 정리하지 않는다
    let detached = env.ingest.set_folder_media_types(id, ALL, true).unwrap();
    assert_eq!(detached, 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NOT NULL"), 0);
}

#[test]
fn t_mtype_11_reenabling_a_type_collects_it_on_the_next_scan() {
    // Arrange: 동영상만으로 시작
    let env = Env::new();
    let id = setup(&env, VIDEO_ONLY);
    scan(&env, id);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 1);

    // Act
    env.ingest.set_folder_media_types(id, ALL, false).unwrap();
    scan(&env, id);

    // Assert
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 3);
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "present");
}

#[test]
fn t_mtype_12_purge_then_reenable_registers_a_new_asset_and_the_tags_do_not_return() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    scan(&env, id);
    let (a_id, _) = loc(&env, "a.jpg").unwrap();
    tag(&env, a_id, "보관");
    env.ingest.set_folder_media_types(id, VIDEO_ONLY, true).unwrap();
    scan(&env, id);
    assert!(loc(&env, "a.jpg").is_none(), "purge 뒤 스캔도 다시 등록하지 않는다");

    // Act
    env.ingest.set_folder_media_types(id, ALL, false).unwrap();
    scan(&env, id);

    // Assert: 정책 변경(D9 폐기) — 새 자산으로 처음부터 다시 스캔하고 태그는 돌아오지 않는다
    let (new_id, presence) = loc(&env, "a.jpg").unwrap();
    assert_eq!(presence, "present");
    assert_ne!(new_id, a_id);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 5, "3 old (2 purged, 1 video kept) + 2 new (image, audio)");
    assert_eq!(tags_of(&env, a_id), 0);
    assert_eq!(tags_of(&env, new_id), 0);
}

#[test]
fn t_mtype_13_changing_to_all_off_is_rejected_and_keeps_the_previous_setting() {
    let env = Env::new();
    let id = setup(&env, VIDEO_ONLY);
    scan(&env, id);

    let err = env.ingest.set_folder_media_types(id, NONE, true).unwrap_err();

    assert!(matches!(err, IngestError::InvalidInput("mediaTypes")), "{err:?}");
    assert_eq!(view_types(&env, id), VIDEO_ONLY);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NOT NULL"), 0);
}

#[test]
fn t_mtype_14_unknown_or_removing_folder_is_not_found() {
    let env = Env::new();
    let id = setup(&env, ALL);

    assert!(matches!(
        env.ingest.set_folder_media_types(9999, ALL, false),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.count_folder_assets_by_type(9999),
        Err(IngestError::NotFound)
    ));
    // 등록 해제된 폴더
    env.ingest.remove_folder(id).unwrap();
    env.ingest.wait_removals();
    assert!(matches!(
        env.ingest.set_folder_media_types(id, ALL, true),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        env.ingest.count_folder_assets_by_type(id),
        Err(IngestError::NotFound)
    ));
}

#[test]
fn t_mtype_15_count_by_type_counts_active_locations_and_uses_the_path_index() {
    // Arrange
    let env = Env::new();
    let id = setup(&env, ALL);
    env.write("lib/sub/d.jpg", 4);
    env.write("other/x.jpg", 8); // 폴더 밖
    scan(&env, id);

    // Act
    let counts = env.ingest.count_folder_assets_by_type(id).unwrap();

    // Assert
    assert_eq!((counts.image, counts.video, counts.audio), (2, 1, 1));
    assert_eq!(
        serde_json::to_value(counts).unwrap(),
        serde_json::json!({ "image": 2, "video": 1, "audio": 1 })
    );
    // 정리 후에는 활성 위치만 센다
    env.ingest
        .set_folder_media_types(id, VIDEO_ONLY, true)
        .unwrap();
    let counts = env.ingest.count_folder_assets_by_type(id).unwrap();
    assert_eq!((counts.image, counts.video, counts.audio), (0, 1, 0));
    // 인덱스 계획: file_locations 는 인덱스 범위 검색이어야 한다 (전체 스캔 금지)
    let plan: Vec<String> = env.read(|c| {
        let mut stmt = c
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {}",
                mam_core::scan::COUNT_BY_TYPE_SQL
            ))
            .unwrap();
        stmt.query_map(rusqlite::params![1, "a", "b"], |r| r.get::<_, String>(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    });
    let plan = plan.join(" | ");
    assert!(
        plan.contains("SEARCH l USING") && !plan.contains("SCAN l"),
        "plan: {plan}"
    );
}

#[test]
fn t_mtype_16_relocate_keeps_the_media_types() {
    let env = Env::new();
    let id = setup(&env, VIDEO_ONLY);
    scan(&env, id);
    env.write("lib2/keep.txt", 1);
    env.ingest.jobs().pause(Some(mam_core::jobs::JobKind::Scan)).unwrap();

    let out = env
        .ingest
        .relocate_folder(id, env.path("lib2").to_str().unwrap())
        .unwrap();

    assert_eq!(out.folder.media_types, VIDEO_ONLY);
}

#[test]
fn t_mtype_17_nested_managed_folders_are_rejected_so_types_never_conflict() {
    let env = Env::new();
    env.write("lib/sub/a.jpg", 1);
    env.ingest
        .add_folder_with_types(env.path("lib/sub").to_str().unwrap(), VIDEO_ONLY)
        .unwrap();

    let parent = env
        .ingest
        .add_folder_with_types(env.path("lib").to_str().unwrap(), ALL)
        .unwrap_err();

    assert!(matches!(parent, IngestError::NestedFolder { .. }), "{parent:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 1);
}

#[test]
fn t_mtype_18_changing_types_cancels_unfinished_scan_runs_so_the_next_scan_starts_fresh() {
    // Arrange: 일시정지된 스캔 실행(cursor 있음)
    let env = Env::new();
    let id = setup(&env, ALL);
    env.pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO scan_runs (folder_id, kind, status, started_at, cursor) VALUES (?1, 'full', 'paused', 1, 'zzz')",
                [id],
            )?;
            Ok(())
        })
        .unwrap();

    // Act: 같은 값 재설정은 취소하지 않는다
    env.ingest.set_folder_media_types(id, ALL, false).unwrap();
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_runs WHERE status = 'paused'"), 1);
    // 값이 바뀌면 취소한다
    env.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();

    // Assert
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_runs WHERE status = 'cancelled'"), 1);
    scan(&env, id);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM scan_runs WHERE status = 'done' AND cursor IS NOT NULL"),
        1,
        "새 실행이 처음부터 돌아 cursor 를 새로 만든다"
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 1, "동영상 1건");
}

#[test]
fn t_mtype_19_scan_started_with_old_types_stops_when_types_change_mid_walk() {
    // Arrange: 하위 폴더가 있어 순회 중 stop 콜백이 2번 불린다. 두 번째 호출에서 설정을 바꾼다 (진행 중 스캔 vs 설정 변경 경쟁)
    let env = Env::new();
    let id = setup(&env, ALL);
    env.write("lib/sub/d.mp4", 4);
    let calls = std::cell::Cell::new(0);
    let change = || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            env.pool
                .write(mam_db::Priority::High, move |c| Ok(set_media_types(c, 1, id, VIDEO_ONLY)))
                .unwrap()
                .unwrap();
        }
        false
    };
    env.clock.advance(5_000);

    // Act
    let out = scan_folder(env.ingest.env(), id, &change, &AtomicBool::new(false)).unwrap();

    // Assert: 옛 설정(전부)으로 모은 항목은 쓰이지 않는다 — 제외된 이미지·음성이 등록되지 않는다
    assert_eq!(out, mam_core::scan::ScanOutcome::Gone);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_errors"), 0);
}
