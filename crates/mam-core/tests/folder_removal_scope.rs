//! 폴더 제거·유형 정리의 범위와 계약 (T-rmv-20 ~ 28).
//!
//! 분류: Happy (20 영향 개수, 22 유형 정리) / Validation (21 없는 폴더·제거 중, 24 직렬화) / Business rule (23 keep 모드,
//! 25 즉시 숨김, 26 스키마 가드) / External (27 유형 정리 백업 실패) / Performance-plan (28 EXPLAIN QUERY PLAN).
//! 실제 SQLite + 임시 디렉터리 볼륨.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::sync::{Arc, Mutex};

use common::env::Env;
use common::purge::{exec, ids_sql, insert_assets, seed_linked, status_counts, total_linked, volume_id};
use mam_core::purge::LINKED_TABLES;
use mam_core::scan::{IngestError, MediaTypeSet};
use mam_vector::VectorKind;

fn add(env: &Env, rel: &str) -> i64 {
    fs::create_dir_all(env.path(rel)).unwrap();
    env.ingest
        .add_folder(env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id
}

#[test]
fn t_rmv_20_impact_counts_only_what_would_be_lost_and_skips_relocated_assets() {
    // Arrange: 자산 4개 (3개는 연결 데이터 있음), 경로 변경으로 다른 폴더에 활성 위치가 있는 자산 1개
    let env = Env::new();
    let (a, _b) = (add(&env, "a"), add(&env, "b"));
    let ids = insert_assets(&env, "vol-a", "a", 4, "image");
    seed_linked(&env, &ids[..3]);
    let moved = insert_assets(&env, "vol-a", "b", 1, "image");
    seed_linked(&env, &moved);
    let vid = volume_id(&env, "vol-a");
    exec(&env, &format!(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, detached_at, last_seen_at)
         VALUES ({}, {vid}, 'a/old.jpg', 'a/old.jpg', 5, 1)", moved[0]));

    // Act
    let impact = env.ingest.folder_removal_impact(a).unwrap();

    // Assert
    assert_eq!(
        (impact.media_count, impact.tag_links, impact.notes, impact.collection_items, impact.ai_results),
        (4, 3, 3, 3, 3)
    );
}

#[test]
fn t_rmv_21_impact_of_unknown_or_removing_folder_is_not_found() {
    let env = Env::new();
    let a = add(&env, "a");
    assert!(matches!(env.ingest.folder_removal_impact(999), Err(IngestError::NotFound)));
    env.ingest.begin_folder_removal(a).unwrap();
    assert!(matches!(env.ingest.folder_removal_impact(a), Err(IngestError::NotFound)));
}

#[test]
fn t_rmv_22_media_type_purge_deletes_linked_data_of_the_excluded_type_only() {
    // Arrange
    let env = Env::new();
    let lib = add(&env, "lib");
    let img = insert_assets(&env, "vol-a", "lib", 3, "image");
    let vid = insert_assets(&env, "vol-a", "lib", 2, "video");
    seed_linked(&env, &img);
    seed_linked(&env, &vid);
    let img_before = total_linked(&env, &img);
    let cache = env.data.join("cache");
    env.ingest.set_cache_root(cache.clone());
    let thumb = mam_media::thumbnail_path(&cache, vid[0], mam_media::ThumbSize::Small);
    fs::create_dir_all(thumb.parent().unwrap()).unwrap();
    fs::write(&thumb, b"x").unwrap();
    let backups = Arc::new(Mutex::new(0));
    let c = Arc::clone(&backups);
    env.ingest.set_backup_hook(Arc::new(move || {
        *c.lock().unwrap() += 1;
        Ok(())
    }));

    // Act: 영상 제외 + 정리
    let removed = env
        .ingest
        .set_folder_media_types(lib, MediaTypeSet { image: true, video: false, audio: true }, true)
        .unwrap();

    // Assert
    assert_eq!(removed, 2);
    assert_eq!(total_linked(&env, &vid), 0);
    assert_eq!(status_counts(&env, &vid, "deleted"), 2);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM file_locations WHERE asset_id IN ({})", ids_sql(&vid))), 0);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'delete' AND asset_id IN ({})", ids_sql(&vid))), 2);
    assert_eq!(total_linked(&env, &img), img_before);
    assert_eq!(status_counts(&env, &img, "active"), 3);
    assert!(!thumb.exists());
    assert_eq!(*backups.lock().unwrap(), 1);
    assert!(env.ingest.list_folders().unwrap()[0].media_types.image);
}

#[test]
fn t_rmv_23_keep_mode_and_nothing_to_purge_delete_nothing_and_skip_the_backup() {
    // Arrange
    let env = Env::new();
    let lib = add(&env, "lib");
    let vid = insert_assets(&env, "vol-a", "lib", 2, "video");
    seed_linked(&env, &vid);
    let before = total_linked(&env, &vid);
    let backups = Arc::new(Mutex::new(0));
    let c = Arc::clone(&backups);
    env.ingest.set_backup_hook(Arc::new(move || {
        *c.lock().unwrap() += 1;
        Ok(())
    }));
    let no_video = MediaTypeSet { image: true, video: false, audio: true };

    // Act: purge=false -> 유지, 지울 유형(오디오)에 자산이 없으면 purge=true 라도 백업·삭제 없음
    let kept = env.ingest.set_folder_media_types(lib, no_video, false).unwrap();
    let nothing = env
        .ingest
        .set_folder_media_types(lib, MediaTypeSet { image: true, video: true, audio: false }, true)
        .unwrap();

    // Assert
    assert_eq!((kept, nothing), (0, 0));
    assert_eq!(total_linked(&env, &vid), before);
    assert_eq!(*backups.lock().unwrap(), 0);
}

#[test]
fn t_rmv_24_contract_types_serialize_with_the_camel_case_names() {
    use mam_core::ingest::FolderRemovalProgress;
    let env = Env::new();
    let a = add(&env, "a");
    insert_assets(&env, "vol-a", "a", 1, "image");

    let impact = serde_json::to_value(env.ingest.folder_removal_impact(a).unwrap()).unwrap();
    let progress = serde_json::to_value(FolderRemovalProgress {
        folder_id: 7,
        done: 1,
        total: 2,
        finished: false,
        error: None,
    })
    .unwrap();

    assert_eq!(impact, serde_json::json!({"mediaCount": 1, "tagLinks": 0, "notes": 0, "collectionItems": 0, "aiResults": 0}));
    assert_eq!(progress, serde_json::json!({"folderId": 7, "done": 1, "total": 2, "finished": false, "error": null}));
}

#[test]
fn t_rmv_25_removing_folder_leaves_the_sidebar_total_and_hidden_filter_immediately() {
    // Arrange
    let env = Env::new();
    let (a, _b) = (add(&env, "a"), add(&env, "b"));
    insert_assets(&env, "vol-a", "a", 3, "image");
    insert_assets(&env, "vol-a", "b", 2, "image");
    assert_eq!(env.ingest.summary().unwrap().total_assets, 5);

    // Act: 삭제 루프를 돌리기 전, 제거 표시만 한 상태
    env.ingest.begin_folder_removal(a).unwrap();

    // Assert
    assert_eq!(env.ingest.summary().unwrap().total_assets, 2);
    assert_eq!(env.read(|c| mam_core::scan::hidden_folder_ids(c).unwrap()), vec![a as u64]);
}

#[test]
fn t_rmv_26_every_table_with_an_asset_id_column_has_a_delete_or_keep_decision() {
    // 새 마이그레이션이 asset_id 를 가진 테이블을 추가하면 이 시험이 실패해, 삭제/보존을 정하도록 강제한다.
    // 보존: file_locations(범위 삭제가 따로 처리) · ai_usage(사용량 이력) · search_outbox · index_progress(색인 워커) · media_assets
    let env = Env::new();
    let found: BTreeSet<String> = env.read(|c| {
        let mut stmt = c
            .prepare(
                "SELECT m.name FROM sqlite_master m, pragma_table_info(m.name) p
                 WHERE m.type = 'table' AND p.name = 'asset_id'",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    });
    let decided: BTreeSet<String> = LINKED_TABLES
        .iter()
        .copied()
        .chain(["transcripts", "file_locations", "ai_usage", "search_outbox", "index_progress", "media_assets"])
        .map(str::to_owned)
        .collect();

    let undecided: Vec<_> = found.difference(&decided).collect();
    assert!(undecided.is_empty(), "decide delete/keep for: {undecided:?}");
}

#[test]
fn t_rmv_27_media_type_purge_with_failed_backup_changes_neither_types_nor_data() {
    // Arrange
    let env = Env::new();
    let lib = add(&env, "lib");
    let vid = insert_assets(&env, "vol-a", "lib", 2, "video");
    seed_linked(&env, &vid);
    let before = total_linked(&env, &vid);
    env.ingest.set_backup_hook(Arc::new(|| Err("no space".into())));

    // Act
    let err = env
        .ingest
        .set_folder_media_types(lib, MediaTypeSet { image: true, video: false, audio: true }, true)
        .unwrap_err();

    // Assert
    assert!(matches!(err, IngestError::BackupFailed), "{err:?}");
    assert!(env.ingest.list_folders().unwrap()[0].media_types.video, "types are unchanged");
    assert_eq!(total_linked(&env, &vid), before);
    assert_eq!(status_counts(&env, &vid, "active"), 2);
}

#[test]
fn t_rmv_28_batch_selects_and_impact_use_the_location_indexes() {
    // 문장은 purge/range.rs·impact.rs 의 질의와 같아야 한다 (EXPLAIN QUERY PLAN: 인덱스 범위 조회, file_locations 전체 스캔 없음)
    let env = Env::new();
    let plan = |sql: &str| -> String {
        env.read(|c| {
            let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let rows: Vec<String> = stmt
                .query_map(rusqlite::params![1, "a/", "a0", 500], |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.join(" | ")
        })
    };
    let active = plan("SELECT l.location_id, l.asset_id FROM file_locations l
        WHERE l.volume_id = ?1 AND l.detached_at IS NULL AND l.path_key >= ?2 AND l.path_key < ?3 LIMIT ?4");
    let detached = plan("SELECT l.location_id, l.asset_id FROM file_locations l
        WHERE l.volume_id = ?1 AND l.detached_at IS NOT NULL AND l.path_key >= ?2 AND l.path_key < ?3 LIMIT ?4");
    let elsewhere = env.read(|c| {
        let mut stmt = c
            .prepare("EXPLAIN QUERY PLAN SELECT EXISTS (SELECT 1 FROM file_locations
               WHERE asset_id = ?1 AND detached_at IS NULL AND NOT (volume_id = ?2 AND path_key >= ?3 AND path_key < ?4))")
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map(rusqlite::params![1, 1, "a/", "a0"], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.join(" | ")
    });

    assert!(active.contains("uq_file_locations_active_path"), "{active}");
    assert!(detached.contains("idx_file_locations_detached_path"), "{detached}");
    assert!(elsewhere.contains("uq_file_locations_active_asset"), "{elsewhere}");
    for p in [&active, &detached, &elsewhere] {
        assert!(!p.contains("SCAN file_locations"), "{p}");
    }
}

#[test]
fn t_rmv_29_impact_and_legacy_candidate_queries_use_indexes() {
    // purge/impact.rs 의 SCOPE·집계, purge/legacy.rs 의 후보 질의와 같은 문장 (EXPLAIN QUERY PLAN)
    let env = Env::new();
    let plan = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> String {
        env.read(|c| {
            let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let rows: Vec<String> = stmt
                .query_map(params, |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.join(" | ")
        })
    };
    let scope = "WITH s AS (
        SELECT l.asset_id AS asset_id FROM file_locations l
        JOIN media_assets a ON a.asset_id = l.asset_id AND a.status = 'active'
        WHERE l.volume_id = ?1 AND l.detached_at IS NULL AND l.path_key >= ?2 AND l.path_key < ?3)";
    let impact_tags = plan(
        &format!("{scope} SELECT COUNT(*) FROM s JOIN asset_tags t ON t.asset_id = s.asset_id"),
        &[&1, &"a/", &"a0"],
    );
    let impact_ai = plan(
        &format!("{scope} SELECT COUNT(*) FROM s WHERE EXISTS (SELECT 1 FROM ai_analyses x WHERE x.asset_id = s.asset_id)
            OR EXISTS (SELECT 1 FROM transcripts x WHERE x.asset_id = s.asset_id)"),
        &[&1, &"a/", &"a0"],
    );
    let legacy = plan(
        "SELECT a.asset_id FROM media_assets a
         WHERE a.asset_id > ?1 AND a.status = 'active'
           AND NOT EXISTS (SELECT 1 FROM file_locations l WHERE l.asset_id = a.asset_id AND l.detached_at IS NULL)
         ORDER BY a.asset_id LIMIT ?2",
        &[&0, &500],
    );

    assert!(impact_tags.contains("uq_file_locations_active_path") && impact_tags.contains("asset_tags"), "{impact_tags}");
    assert!(impact_ai.contains("uq_file_locations_active_path"), "{impact_ai}");
    assert!(legacy.contains("uq_file_locations_active_asset"), "{legacy}");
    assert!(legacy.contains("SEARCH a USING INTEGER PRIMARY KEY"), "{legacy}");
    for p in [&impact_tags, &impact_ai] {
        assert!(!p.contains("SCAN file_locations"), "{p}");
    }
}

#[test]
fn t_rmv_30_embedding_hook_receives_the_model_keys_of_deleted_embeddings_only() {
    // Arrange
    let env = Env::new();
    let (a, b) = (add(&env, "a"), add(&env, "b"));
    let ia = insert_assets(&env, "vol-a", "a", 2, "image");
    let ib = insert_assets(&env, "vol-a", "b", 1, "image");
    seed_linked(&env, &ia); // embeddings: visual / m / 1 / 4
    type Key = (VectorKind, String, usize);
    let calls: Arc<Mutex<Vec<Vec<Key>>>> = Arc::default();
    let sink = Arc::clone(&calls);
    env.ingest.set_embeddings_changed_hook(Arc::new(move |keys| {
        sink.lock()
            .unwrap()
            .push(keys.iter().map(|k| (k.kind, k.model_id.clone(), k.dimension)).collect());
    }));

    // Act: 임베딩이 없는 폴더 -> 알리지 않는다, 있는 폴더 -> 키 1개 (자산 2개여도 중복 없음)
    env.ingest.remove_folder(b).unwrap();
    env.ingest.wait_removals();
    let none = calls.lock().unwrap().len();
    env.ingest.remove_folder(a).unwrap();
    env.ingest.wait_removals();

    // Assert
    assert_eq!(none, 0);
    assert_eq!(*calls.lock().unwrap(), vec![vec![(VectorKind::Visual, "m".to_string(), 4)]]);
    assert_eq!(status_counts(&env, &ib, "deleted"), 1);
}
