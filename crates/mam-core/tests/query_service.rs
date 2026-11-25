//! P2-5 조회 서비스 — 페이지 하이드레이션·상세·최근 검색. 실제 SQLite(마이그레이션 적용).
//!
//! 분류: T-coord-09 Happy+Business rule (하이드레이션·availability·쿼리 계획) / T-coord-10 Happy (상세) /
//! T-coord-11 Business rule (최근 50개·갱신·삭제)

use std::sync::Arc;

use mam_core::SharedClock;
use mam_core::query::{
    Availability, FolderScope, MediaKind, NEWER_SQL, PAGE_SQL, PREVIOUS_SQL, QueryService,
    RELATED_SQL, SearchFilters,
};
use mam_core::scan::IngestError;
use mam_db::{Clock, DbPool, Priority};
use rusqlite::Connection;
use tempfile::TempDir;

struct StepClock(std::sync::atomic::AtomicI64);
impl Clock for StepClock {
    fn now_ms(&self) -> i64 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

struct Fx {
    _dir: TempDir,
    pool: Arc<DbPool>,
    svc: QueryService,
}

fn fixture(seed: impl FnOnce(&Connection)) -> Fx {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("t.sqlite");
    let mut conn = mam_db::open(&db).unwrap();
    mam_db::migrate(&mut conn, &dir.path().join("backups"), &mam_db::SystemClock).unwrap();
    seed(&conn);
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let clock: SharedClock = Arc::new(StepClock(1_000.into()));
    let svc = QueryService::new(Arc::clone(&pool), clock);
    Fx {
        _dir: dir,
        pool,
        svc,
    }
}

fn exec(c: &Connection, sql: &str) {
    c.execute_batch(sql).unwrap();
}

/// 볼륨 1(온라인) / 2(오프라인). 자산: 1 정상, 2 오프라인 볼륨, 3 파일 없음, 4 superseded, 5 위치 detach, 6 음성
fn seed_assets(c: &Connection) {
    exec(
        c,
        "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at) VALUES
           (1, 'fixed', 'v1', 'Main', 'C:\\Media', 'online', 0),
           (2, 'removable', 'v2', 'USB', 'E:\\', 'offline', 0);
         INSERT INTO managed_folders (folder_id, volume_id, rel_root, path_key, display_path, created_at)
           VALUES (1, 1, 'photos', 'photos', 'C:\\Media\\photos', 0);",
    );
    for (id, media, status) in [
        (1, "image", "active"),
        (2, "image", "active"),
        (3, "video", "active"),
        (4, "image", "superseded"),
        (5, "image", "active"),
        (6, "audio", "active"),
    ] {
        exec(
            c,
            &format!(
                "INSERT INTO media_assets (asset_id, media_type, status, file_size, modified_at, revision, created_at, updated_at)
                 VALUES ({id}, '{media}', '{status}', {}, {}, {id}, 0, 0)",
                id * 100,
                id * 10
            ),
        );
    }
    for (id, vol, rel, presence, detached) in [
        (1, 1, "Photos/2024/a.jpg", "present", "NULL"),
        (2, 2, "usb/b.jpg", "present", "NULL"),
        (3, 1, "Photos/c.mp4", "missing", "NULL"),
        (4, 1, "Photos/old.jpg", "present", "5"),
        (5, 1, "Photos/gone.jpg", "present", "5"),
        (6, 1, "Photos/s.mp3", "present", "NULL"),
    ] {
        exec(
            c,
            &format!(
                "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, presence, detached_at, last_seen_at)
                 VALUES ({id}, {vol}, '{rel}', lower('{rel}'), '{presence}', {detached}, 0)"
            ),
        );
    }
    exec(
        c,
        "INSERT INTO media_metadata (asset_id, width, height, duration_ms, captured_at, codec, playback, extra)
           VALUES (1, 1920, 1080, NULL, 1700000000000, 'jpeg', 'playable', '{\"container\":\"jpeg\"}'),
                  (3, 640, 360, 5000, NULL, 'h264', 'not_playable', NULL);
         INSERT INTO asset_user_data (asset_id, memo, favorite, updated_at) VALUES (1, 'hello', 1, 0);
         INSERT INTO asset_derivatives (asset_id, kind, state, updated_at) VALUES
           (1, 'thumb', 'done', 0), (3, 'thumb', 'failed', 0), (6, 'thumb', 'unsupported', 0);",
    );
}

#[test]
fn t_coord_09_hydration_maps_fields_in_request_order_and_skips_inactive() {
    // Arrange
    let fx = fixture(seed_assets);

    // Act: 요청 순서를 섞고 비활성(4,5)·없는 id(99) 를 끼운다
    let page = fx.svc.hydrate(&[6, 3, 4, 1, 99, 2, 5]).unwrap();

    // Assert
    let ids: Vec<i64> = page.iter().map(|s| s.asset_id).collect();
    assert_eq!(
        ids,
        [6, 3, 1, 2],
        "request order kept; superseded/detached/unknown dropped"
    );
    let by = |id| page.iter().find(|s| s.asset_id == id).unwrap();
    let a = by(1);
    assert_eq!(a.file_name, "a.jpg");
    assert_eq!(a.media_type, MediaKind::Image);
    assert_eq!(a.availability, Availability::Online);
    assert_eq!(
        (a.width, a.height, a.duration_ms),
        (Some(1920), Some(1080), None)
    );
    assert_eq!(
        (a.file_size, a.captured_at, a.modified_at, a.revision),
        (100, Some(1_700_000_000_000), 10, 1)
    );
    assert!(a.favorite);
    assert_eq!(a.thumb_state, "done");
    assert_eq!(by(2).availability, Availability::Offline, "volume offline");
    assert_eq!(
        by(3).availability,
        Availability::Missing,
        "file missing on a connected volume"
    );
    assert_eq!(by(3).thumb_state, "failed");
    assert_eq!(by(2).thumb_state, "pending", "no derivative row = pending");
    assert_eq!(by(6).thumb_state, "unsupported");
    assert!(fx.svc.hydrate(&[]).unwrap().is_empty());
}

#[test]
fn t_coord_09b_hydration_and_detail_queries_never_scan_the_big_tables() {
    // Arrange
    let fx = fixture(seed_assets);

    // Act
    let plans: Vec<(&str, Vec<String>)> = fx
        .pool
        .read(|c| {
            let mut out = Vec::new();
            for (name, sql, arg) in [
                ("page", PAGE_SQL, "[1,2,3]"),
                ("related", RELATED_SQL, "[1,2,3]"),
                ("previous", PREVIOUS_SQL, "1"),
                ("newer", NEWER_SQL, "1"),
            ] {
                let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
                let rows = s
                    .query_map([arg], |r| r.get::<_, String>(3))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                out.push((name, rows));
            }
            Ok(out)
        })
        .unwrap();

    // Assert: 구동 테이블은 json_each(j) / 키 조회뿐, 큰 테이블의 전체 스캔 없음
    for (name, lines) in plans {
        for line in &lines {
            let scans_big = line.starts_with("SCAN ") && !line.starts_with("SCAN j");
            assert!(!scans_big, "{name}: full scan in plan: {line}\n{lines:#?}");
        }
    }
}

#[test]
fn t_coord_10_detail_has_metadata_tags_collections_duplicates_and_versions() {
    // Arrange: 자산 1 = 중복(7, 8) + 이전 버전(9) + 태그/컬렉션, 자산 4 는 superseded 인 구자산
    let fx = fixture(|c| {
        seed_assets(c);
        for id in [7, 8, 9] {
            exec(
                c,
                &format!(
                    "INSERT INTO media_assets (asset_id, media_type, status, file_size, modified_at, created_at, updated_at)
                     VALUES ({id}, 'image', '{}', 5, 0, 0, 0);
                     INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at, detached_at)
                     VALUES ({id}, 1, 'Photos/dup{id}.jpg', 'photos/dup{id}.jpg', 0, {});",
                    if id == 9 { "superseded" } else { "active" },
                    if id == 9 { "3" } else { "NULL" },
                ),
            );
        }
        exec(
            c,
            "UPDATE media_assets SET hash_state = 'done', hash_algo = 'blake3', content_hash = zeroblob(32)
               WHERE asset_id IN (1, 7, 8);
             INSERT INTO asset_relations (from_asset_id, to_asset_id, relation, created_at) VALUES (9, 1, 'content_changed', 5);
             INSERT INTO tags (tag_id, name, name_key) VALUES (1, 'Beach', 'beach'), (2, 'Alpha', 'alpha');
             INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (1, 1, 0), (1, 2, 0);
             INSERT INTO collections (collection_id, name, name_key, created_at) VALUES (3, 'Trip', 'trip', 0);
             INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES (3, 1, 0);",
        );
    });

    // Act
    let d = fx.svc.detail(1).unwrap();
    let old = fx.svc.detail(9).unwrap();

    // Assert
    assert_eq!(d.status, "active");
    assert_eq!(d.path, "C:\\Media\\Photos\\2024\\a.jpg");
    assert_eq!(
        (d.folder_id, d.volume_label.as_deref()),
        (Some(1), Some("Main"))
    );
    assert_eq!(d.codec.as_deref(), Some("jpeg"));
    assert!(d.playable);
    assert_eq!(d.extra.get("container").map(String::as_str), Some("jpeg"));
    assert_eq!(d.hash_state, "done");
    assert_eq!(d.memo, "hello");
    assert_eq!(
        d.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["Alpha", "Beach"]
    );
    assert_eq!(d.collections[0].name, "Trip");
    assert_eq!(
        d.duplicates.iter().map(|s| s.asset_id).collect::<Vec<_>>(),
        [7, 8]
    );
    assert_eq!(
        d.previous_versions
            .iter()
            .map(|s| s.asset_id)
            .collect::<Vec<_>>(),
        [9]
    );
    assert_eq!(
        d.previous_versions[0].file_name, "dup9.jpg",
        "last known location of a superseded asset"
    );
    assert!(d.newer_version.is_none());
    assert_eq!(old.status, "superseded");
    assert_eq!(old.newer_version.map(|s| s.asset_id), Some(1));
    assert!(matches!(fx.svc.detail(12345), Err(IngestError::NotFound)));
}

#[test]
fn t_coord_10b_duplicates_are_capped_at_50() {
    // Arrange: 같은 해시 60건
    let fx = fixture(|c| {
        seed_assets(c);
        for id in 100..160 {
            exec(
                c,
                &format!(
                    "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at, hash_state, hash_algo, content_hash)
                     VALUES ({id}, 'image', 1, 0, 0, 0, 'done', 'blake3', zeroblob(32));
                     INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                     VALUES ({id}, 1, 'Photos/d{id}.jpg', 'photos/d{id}.jpg', 0);"
                ),
            );
        }
    });

    // Act
    let d = fx.svc.detail(100).unwrap();

    // Assert
    assert_eq!(d.duplicates.len(), 50);
    assert!(d.duplicates.iter().all(|s| s.asset_id != 100));
}

fn filters(query_tag: i64) -> SearchFilters {
    SearchFilters {
        media_types: vec![MediaKind::Video],
        include_offline: true,
        include_missing: false,
        captured_from: None,
        captured_to: None,
        size_min: None,
        size_max: None,
        duration_min_ms: None,
        duration_max_ms: None,
        folder: Some(FolderScope {
            folder_id: 1,
            rel_path: Some("2024".into()),
            recursive: true,
        }),
        favorites_only: false,
        collection_id: None,
        tag_ids: vec![query_tag],
        rating_min: None,
        rating_max: None,
        user_fields: vec![],
    }
}

fn default_filters() -> SearchFilters {
    SearchFilters {
        media_types: vec![],
        tag_ids: vec![],
        rating_min: None,
        rating_max: None,
        user_fields: vec![],
        folder: None,
        ..filters(0)
    }
}

#[test]
fn t_coord_11_recent_searches_keep_50_update_same_conditions_and_clear() {
    // Arrange
    let fx = fixture(|_| {});

    // Act: 같은 조건 두 번 + 서로 다른 조건 59번
    fx.svc.record_recent("beach", &filters(1)).unwrap();
    for i in 0..59 {
        fx.svc.record_recent(&format!("q{i}"), &filters(1)).unwrap();
    }
    fx.svc.record_recent("beach", &filters(1)).unwrap(); // 맨 앞으로 + 갱신
    fx.svc.record_recent("beach", &filters(2)).unwrap(); // 필터가 다르면 다른 항목
    fx.svc.record_recent("  ", &default_filters()).unwrap(); // 질의·필터 없음은 기록하지 않는다
    let list = fx.svc.list_recent().unwrap();

    // Assert
    assert_eq!(list.len(), 50, "kept at 50");
    assert_eq!(
        (list[0].query.as_str(), list[0].filters.tag_ids.clone()),
        ("beach", vec![2])
    );
    assert_eq!(
        (list[1].query.as_str(), list[1].filters.tag_ids.clone()),
        ("beach", vec![1])
    );
    assert_eq!(
        list.iter()
            .filter(|r| r.query == "beach" && r.filters.tag_ids == [1])
            .count(),
        1
    );
    assert!(
        list.windows(2).all(|w| w[0].used_at >= w[1].used_at),
        "newest first"
    );
    assert!(
        !list.iter().any(|r| r.query == "q0"),
        "oldest entries are pruned"
    );

    // Act + Assert: 삭제, 질의 길이 검증
    fx.svc.clear_recent().unwrap();
    assert!(fx.svc.list_recent().unwrap().is_empty());
    let long = "가".repeat(501);
    assert!(matches!(
        fx.svc.record_recent(&long, &filters(1)),
        Err(IngestError::InvalidInput(_))
    ));
}

#[test]
fn t_coord_12b_folder_and_volume_helpers_validate_input() {
    // Arrange
    let fx = fixture(seed_assets);
    let scope = |folder_id, rel: Option<&str>| FolderScope {
        folder_id,
        rel_path: rel.map(str::to_string),
        recursive: true,
    };

    // Act + Assert
    assert_eq!(fx.svc.online_volume_ids().unwrap(), [1]);
    assert_eq!(
        fx.svc.resolve_folder(&scope(1, None)).unwrap(),
        (1, String::new())
    );
    assert_eq!(
        fx.svc
            .resolve_folder(&scope(1, Some("2024\\Trip")))
            .unwrap(),
        (1, "2024/trip".to_string()),
        "backslashes and case are normalized like path_key"
    );
    assert!(matches!(
        fx.svc.resolve_folder(&scope(77, None)),
        Err(IngestError::NotFound)
    ));
    // 쓰기 우선순위 확인용 — 풀이 살아 있다
    fx.pool.write(Priority::High, |_| Ok(())).unwrap();
}
