// Categories: Happy (검색·페이지네이션) / Validation (인자·커서) / Business rule (hybrid degraded, 오프라인 필터, 편집 반영) /
// External (색인 준비 전)
use std::time::{Duration, Instant};

use mam_core::query::{FolderScope, SearchFilters, SearchMode, SortDirection, SortField, SortSpec};
use mam_core::userdata::UserData;
use mam_db::{SystemClock, migrate, open};

use super::*;

struct Fx {
    _dir: tempfile::TempDir,
    pool: Arc<DbPool>,
    search: Arc<SearchService>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        self.search.shutdown();
    }
}

fn insert_asset(c: &rusqlite::Connection, id: i64, vol: i64, rel: &str, size: i64) {
    c.execute(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
         VALUES (?1, 'image', ?2, ?1, 0, 0)",
        rusqlite::params![id, size],
    )
    .unwrap();
    c.execute(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
         VALUES (?1, ?2, ?3, lower(?3), 0)",
        rusqlite::params![id, vol, rel],
    )
    .unwrap();
    c.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, 1, 'upsert', 0, 0)",
        [id],
    )
    .unwrap();
}

fn fixture() -> Fx {
    fixture_with(|_| {})
}

fn fixture_with(prepare: impl FnOnce(&rusqlite::Connection)) -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
    conn.execute_batch(
        "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at) VALUES
           (1, 'fixed', 'v1', 'Main', 'C:\\Media', 'online', 0),
           (2, 'removable', 'v2', 'USB', 'E:\\', 'offline', 0);
         INSERT INTO managed_folders (folder_id, volume_id, rel_root, path_key, display_path, created_at)
           VALUES (1, 1, 'photos', 'photos', 'C:\\Media\\photos', 0);",
    )
    .unwrap();
    insert_asset(&conn, 1, 1, "photos/sunset beach.jpg", 100);
    insert_asset(&conn, 2, 1, "photos/trip/beach party.jpg", 200);
    insert_asset(&conn, 3, 2, "usb/forest.jpg", 300);
    prepare(&conn);
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let clock: SharedClock = Arc::new(SystemClock);
    let query = Arc::new(QueryService::new(Arc::clone(&pool), Arc::clone(&clock)));
    let search = SearchService::start(
        Arc::clone(&pool),
        clock,
        dir.path().join("indexes").join("tantivy"),
        query,
    );
    Fx {
        _dir: dir,
        pool,
        search,
    }
}

fn filters() -> SearchFilters {
    SearchFilters {
        media_types: vec![],
        include_offline: true,
        include_missing: false,
        captured_from: None,
        captured_to: None,
        size_min: None,
        size_max: None,
        duration_min_ms: None,
        duration_max_ms: None,
        folder: None,
        favorites_only: false,
        collection_id: None,
        tag_ids: vec![],
        rating_min: None,
        rating_max: None,
        user_fields: vec![],
    }
}

fn request(query: &str) -> SearchRequest {
    SearchRequest {
        query: query.to_string(),
        filters: filters(),
        sort: SortSpec {
            field: SortField::Name,
            direction: SortDirection::Asc,
        },
        limit: 50,
        cursor: None,
        mode: None,
        direction: None,
        at: None,
    }
}

/// 색인이 열리고 Outbox 가 비워질 때까지 기다린다 (최대 30초).
fn ready(fx: &Fx, expected_total: i64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(p) = fx.search.search(&request(""))
            && p.total == expected_total
        {
            return;
        }
        assert!(Instant::now() < deadline, "index was not ready in time");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn code_of(e: SearchFailure) -> ErrorCode {
    CommandError::from(e).code()
}

fn keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[test]
fn t_ipc_30_search_page_has_the_contract_shape_and_hydrated_items() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);

    // Act
    let page = fx.search.search(&request("beach")).unwrap();
    let json = serde_json::to_value(&page).unwrap();

    // Assert
    assert_eq!(page.total, 2);
    let names: Vec<_> = page.items.iter().map(|i| i.file_name.as_str()).collect();
    assert_eq!(
        names,
        ["beach party.jpg", "sunset beach.jpg"],
        "name order, hydrated from SQLite"
    );
    assert!(!page.total_is_estimate);
    assert_eq!(
        keys(&json),
        [
            "generation",
            "items",
            "nextCursor",
            "prevCursor",
            "startOffset",
            "tookMs",
            "total",
            "totalIsEstimate"
        ],
        "degraded is omitted for keyword searches"
    );
    assert_eq!(
        keys(&json["items"][0]),
        [
            "aiState",
            "assetId",
            "availability",
            "capturedAt",
            "durationMs",
            "favorite",
            "fileName",
            "fileSize",
            "height",
            "mediaType",
            "modifiedAt",
            "rating",
            "revision",
            "thumbState",
            "title",
            "width"
        ]
    );
}

#[test]
fn t_ipc_31_cursor_paging_walks_all_results_once_and_a_stale_cursor_is_invalid_input() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let mut req = request("");
    req.limit = 2;

    // Act
    let first = fx.search.search(&req).unwrap();
    req.cursor = first.next_cursor.clone();
    let second = fx.search.search(&req).unwrap();
    let mut bad = req.clone();
    bad.query = "other".into(); // 다른 질의에 쓰는 커서
    let stale = fx.search.search(&bad).unwrap_err();
    bad.cursor = Some("nonsense".into());
    let malformed = fx.search.search(&bad).unwrap_err();

    // Assert
    let all: Vec<i64> = first
        .items
        .iter()
        .chain(&second.items)
        .map(|i| i.asset_id)
        .collect();
    assert_eq!(all.len(), 3);
    assert_eq!(first.total, 3);
    assert!(first.next_cursor.is_some() && second.next_cursor.is_none());
    assert_eq!(code_of(stale), ErrorCode::InvalidInput);
    assert_eq!(code_of(malformed), ErrorCode::InvalidInput);
}

#[test]
fn t_ipc_32_hybrid_mode_returns_keyword_results_marked_degraded() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let mut hybrid = request("beach");
    hybrid.mode = Some(SearchMode::Hybrid);
    let mut browse = request("");
    browse.mode = Some(SearchMode::Hybrid);

    // Act
    let degraded = fx.search.search(&hybrid).unwrap();
    let keyword = fx.search.search(&request("beach")).unwrap();
    let no_query = fx.search.search(&browse).unwrap();

    // Assert
    assert_eq!(degraded.degraded, Some(true));
    assert_eq!(
        degraded.items, keyword.items,
        "same results as keyword search"
    );
    assert_eq!(keyword.degraded, None);
    assert_eq!(no_query.degraded, None, "browsing has nothing to degrade");
}

#[test]
fn t_ipc_33_invalid_arguments_map_to_contract_error_codes() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let mut cases: Vec<(SearchRequest, ErrorCode)> = Vec::new();
    for limit in [0, 201, -1] {
        let mut r = request("");
        r.limit = limit;
        cases.push((r, ErrorCode::InvalidInput));
    }
    let mut negative = request("");
    negative.filters.size_min = Some(-1);
    cases.push((negative, ErrorCode::InvalidInput));
    let mut unknown_folder = request("");
    unknown_folder.filters.folder = Some(FolderScope {
        folder_id: 99,
        rel_path: None,
        recursive: true,
    });
    cases.push((unknown_folder, ErrorCode::NotFound));
    cases.push((request(&"가".repeat(501)), ErrorCode::InvalidInput));

    // Act + Assert
    for (req, code) in cases {
        let err = fx.search.search(&req).unwrap_err();
        assert_eq!(code_of(err), code, "{req:?}");
    }
}

#[test]
fn t_ipc_34_offline_volumes_and_folder_scope_filters_apply() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let mut hide_offline = request("");
    hide_offline.filters.include_offline = false;
    let mut scoped = request("");
    scoped.filters.folder = Some(FolderScope {
        folder_id: 1,
        rel_path: None,
        recursive: false,
    });
    let mut scoped_deep = scoped.clone();
    scoped_deep.filters.folder = Some(FolderScope {
        folder_id: 1,
        rel_path: Some("Trip".into()),
        recursive: true,
    });

    // Act
    let ids = |r: &SearchRequest| -> Vec<i64> {
        let mut v: Vec<i64> = fx
            .search
            .search(r)
            .unwrap()
            .items
            .iter()
            .map(|i| i.asset_id)
            .collect();
        v.sort_unstable();
        v
    };

    // Assert
    assert_eq!(
        ids(&request("")),
        [1, 2, 3],
        "offline shown by default (PL5)"
    );
    assert_eq!(ids(&hide_offline), [1, 2]);
    assert_eq!(ids(&scoped), [1], "non-recursive root excludes trip/");
    assert_eq!(
        ids(&scoped_deep),
        [2],
        "relPath is matched case-insensitively"
    );
    let offline = fx.search.search(&request("forest")).unwrap();
    assert_eq!(
        serde_json::to_value(&offline.items[0]).unwrap()["availability"],
        "offline"
    );
}

#[test]
fn t_ipc_35_user_edit_reaches_search_after_wake() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let ud = UserData::new(Arc::clone(&fx.pool), Arc::new(SystemClock));
    assert_eq!(fx.search.search(&request("lighthouse")).unwrap().total, 0);

    // Act
    ud.set_memo(3, "lighthouse at night").unwrap();
    fx.search.wake();

    // Assert: 워커 주기(1초)보다 빨리 priority 0 이벤트가 반영된다
    let deadline = Instant::now() + Duration::from_secs(10);
    let found = loop {
        let p = fx.search.search(&request("lighthouse")).unwrap();
        if p.total == 1 {
            break p;
        }
        assert!(Instant::now() < deadline, "memo edit was not indexed");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(found.items[0].asset_id, 3);
}

#[test]
fn t_ipc_34b_not_ready_maps_to_the_dedicated_index_not_ready_code() {
    // Arrange: 재구축 중 검색이 돌려주는 SearchError::NotReady
    let failure = SearchFailure::Search(SearchError::NotReady);

    // Act
    let err: CommandError = failure.into();

    // Assert: 문구가 아니라 코드로 구분된다 (프론트 isSearchNotReady 는 코드만 본다)
    assert_eq!(err.code(), ErrorCode::IndexNotReady);
    assert_eq!(
        serde_json::to_value(&err).unwrap(),
        serde_json::json!({"code": "index_not_ready", "message": "search index is not ready"})
    );
}

#[test]
fn t_exit_search_01_worker_stops_on_signal_within_the_deadline_and_stopping_twice_is_safe() {
    // Arrange: 색인이 준비된 뒤 대기(IDLE_POLL) 중인 워커
    let fx = fixture();
    ready(&fx, 3);

    // Act
    let started = Instant::now();
    fx.search.request_stop();
    let abandoned = fx.search.shutdown_within(Duration::from_secs(5));
    let again = fx.search.shutdown_within(Duration::ZERO);

    // Assert: 대기 시간을 채우지 않고 깨어나며, 두 번째 호출은 정리할 것이 없다
    assert_eq!(abandoned, 0);
    assert_eq!(again, 0);
    assert!(started.elapsed() < Duration::from_secs(3));
}

fn set_title_setting(fx: &Fx, on: bool) {
    fx.pool
        .write(Priority::High, move |c| {
            Ok(mam_db::settings::update(
                c,
                serde_json::json!({"titleEditEnabled": on})
                    .as_object()
                    .unwrap(),
            )
            .is_ok())
        })
        .unwrap()
        .then_some(())
        .unwrap();
}

fn ids_of(p: &SearchPage) -> Vec<i64> {
    p.items.iter().map(|i| i.asset_id).collect()
}

#[test]
fn t_title_ipc_01_title_search_and_title_sort_follow_the_setting_without_reindex() {
    // Arrange: 설정 켜고 제목 지정 -> 색인 반영 대기
    let fx = fixture();
    ready(&fx, 3);
    set_title_setting(&fx, true);
    let ud = UserData::new(Arc::clone(&fx.pool), Arc::new(SystemClock));
    ud.set_title(1, Some("aaa first")).unwrap();
    fx.search.wake();
    let deadline = Instant::now() + Duration::from_secs(10);
    while fx.search.search(&request("aaa")).unwrap().total != 1 {
        assert!(Instant::now() < deadline, "title was not indexed");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Act + Assert (켜짐): 제목으로 검색되고 파일명으로도 검색되며, 이름순은 제목 기준
    assert_eq!(ids_of(&fx.search.search(&request("aaa")).unwrap()), [1]);
    assert_eq!(fx.search.search(&request("beach")).unwrap().total, 2);
    assert_eq!(ids_of(&fx.search.search(&request("")).unwrap()), [1, 2, 3]);

    // Act + Assert (꺼짐): 같은 인덱스에서 저장된 제목은 검색되지 않고, 파일명은 여전히 검색되며, 이름순은 파일명 기준
    set_title_setting(&fx, false);
    assert_eq!(fx.search.search(&request("aaa")).unwrap().total, 0);
    assert_eq!(fx.search.search(&request("beach")).unwrap().total, 2);
    assert_eq!(ids_of(&fx.search.search(&request("")).unwrap()), [2, 3, 1]);
}

#[test]
fn t_ipc_40_backward_and_at_pages_report_positions_and_a_js_safe_generation() {
    // Arrange: 이름순 3건 (beach party, forest, sunset beach)
    let fx = fixture();
    ready(&fx, 3);
    let mut req = request("");
    req.limit = 2;
    let first = fx.search.search(&req).unwrap();

    // Act
    req.cursor = first.next_cursor.clone();
    let second = fx.search.search(&req).unwrap();
    let mut back = request("");
    back.limit = 2;
    back.cursor = second.prev_cursor.clone();
    back.direction = Some(PageDirection::Backward);
    let before = fx.search.search(&back).unwrap();
    let mut seek = request("");
    seek.limit = 2;
    seek.at = Some(1);
    let at1 = fx.search.search(&seek).unwrap();
    seek.at = Some(3);
    let end = fx.search.search(&seek).unwrap();

    // Assert
    let ids = |p: &SearchPage| p.items.iter().map(|i| i.asset_id).collect::<Vec<_>>();
    assert_eq!((first.start_offset, second.start_offset), (0, 2));
    assert!(first.prev_cursor.is_none() && second.prev_cursor.is_some());
    assert_eq!(ids(&before), ids(&first));
    assert_eq!(at1.start_offset, 1);
    assert_eq!(ids(&at1), [ids(&first)[1], ids(&second)[0]]);
    assert!(end.items.is_empty());
    assert_eq!(
        (end.total, end.start_offset),
        (3, 3),
        "at == total is an empty page"
    );
    for p in [&first, &second, &before, &at1, &end] {
        assert_eq!(p.generation, first.generation);
        assert!(
            (0..(1i64 << 53)).contains(&p.generation),
            "fits a JS safe integer"
        );
    }
}

#[test]
fn t_ipc_41_conflicting_paging_arguments_map_to_invalid_input() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let cursor = {
        let mut r = request("");
        r.limit = 1;
        fx.search.search(&r).unwrap().next_cursor.unwrap()
    };
    let mut both = request("");
    both.limit = 1;
    both.cursor = Some(cursor);
    both.at = Some(0);
    let mut orphan = request("");
    orphan.direction = Some(PageDirection::Backward);
    let mut negative = request("");
    negative.at = Some(-1);

    // Act + Assert
    for (name, r) in [
        ("cursor+at", both),
        ("backward without cursor", orphan),
        ("negative at", negative),
    ] {
        assert_eq!(
            code_of(fx.search.search(&r).unwrap_err()),
            ErrorCode::InvalidInput,
            "{name}"
        );
    }
}

fn index_request(sort: SortField, direction: SortDirection) -> SearchIndexRequest {
    SearchIndexRequest {
        query: String::new(),
        filters: filters(),
        sort: SortSpec {
            field: sort,
            direction,
        },
        mode: None,
    }
}

#[test]
fn t_ipc_42_search_index_has_the_contract_shape_and_matches_the_search_total() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);

    // Act
    let by_name = fx
        .search
        .search_index(&index_request(SortField::Name, SortDirection::Asc))
        .unwrap();
    let by_name_desc = fx
        .search
        .search_index(&index_request(SortField::Name, SortDirection::Desc))
        .unwrap();
    let by_size = fx
        .search
        .search_index(&index_request(SortField::Size, SortDirection::Asc))
        .unwrap();
    let by_modified = fx
        .search
        .search_index(&index_request(SortField::ModifiedAt, SortDirection::Desc))
        .unwrap();
    let json = serde_json::to_value(&by_name).unwrap();
    let page = fx.search.search(&request("")).unwrap();

    // Assert: 파일명 beach party / forest / sunset beach -> b, f, s
    let pairs = |i: &SearchIndex| {
        i.sections
            .iter()
            .map(|s| (s.key.clone(), s.count))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        pairs(&by_name),
        [
            ("b".to_string(), 1),
            ("f".to_string(), 1),
            ("s".to_string(), 1)
        ]
    );
    assert_eq!(
        pairs(&by_name_desc),
        [
            ("s".to_string(), 1),
            ("f".to_string(), 1),
            ("b".to_string(), 1)
        ]
    );
    assert_eq!(by_name.kind, IndexKind::Initial);
    assert_eq!(
        (by_size.kind, pairs(&by_size)),
        (IndexKind::None, vec![("all".to_string(), 3)])
    );
    assert_eq!(by_modified.kind, IndexKind::Month);
    assert_eq!(
        by_modified.sections.iter().map(|s| s.count).sum::<i64>(),
        by_modified.total
    );
    assert_eq!(by_name.total, page.total);
    assert_eq!(
        by_name.generation, page.generation,
        "index and page agree on the generation"
    );
    assert_eq!(keys(&json), ["generation", "kind", "sections", "total"]);
    assert_eq!(keys(&json["sections"][0]), ["count", "key", "label"]);
    assert!(json["sections"][0]["label"].is_null());
    assert_eq!(json["kind"], "initial");
}

#[test]
fn t_ipc_43_search_index_validates_filters_like_search() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let mut negative = index_request(SortField::Name, SortDirection::Asc);
    negative.filters.size_min = Some(-1);
    let mut long = index_request(SortField::Name, SortDirection::Asc);
    long.query = "가".repeat(501);
    let mut unknown = index_request(SortField::Name, SortDirection::Asc);
    unknown.filters.folder = Some(FolderScope {
        folder_id: 99,
        rel_path: None,
        recursive: true,
    });

    // Act + Assert
    assert_eq!(
        code_of(fx.search.search_index(&negative).unwrap_err()),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code_of(fx.search.search_index(&long).unwrap_err()),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code_of(fx.search.search_index(&unknown).unwrap_err()),
        ErrorCode::NotFound
    );
    // 필터가 구간 합에 반영된다 (USB 볼륨 오프라인 숨김 -> 2건)
    let mut hide = index_request(SortField::Name, SortDirection::Asc);
    hide.filters.include_offline = false;
    assert_eq!(fx.search.search_index(&hide).unwrap().total, 2);
}

// ---------------------------------------------------------------- 촬영일 대체값 (T-capfb-ipc-*)
// Categories: Happy (none/modified 정렬·목차·필터) / Boundary (ms -> 초 내림 경계) /
// Business rule (기본값 = 수정일, 토글에 재색인 없음) / Concurrency (토글 전 커서는 stale)

const T_2024_03_10_NOON_MS: i64 = 1_710_072_000_000;
const T_2024_05_15_NOON_MS: i64 = 1_715_774_400_000;
const T_2022_01_15_NOON_MS: i64 = 1_642_248_000_000;

/// 1: 촬영일 2024-03-10(수정일 2020) / 2: 촬영일 없음, 수정일 2024-05-15 / 3: 촬영일 없음, 수정일 2022-01-15 12:00:00.999
fn capfb_fixture() -> Fx {
    let fx = fixture_with(|c| {
        for (id, modified) in [
            (1, 1_577_880_000_000),
            (2, T_2024_05_15_NOON_MS),
            (3, T_2022_01_15_NOON_MS + 999),
        ] {
            c.execute(
                "UPDATE media_assets SET modified_at = ?2 WHERE asset_id = ?1",
                rusqlite::params![id, modified],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO media_metadata (asset_id, duration_ms, captured_at) VALUES (1, 1000, ?1)",
            [T_2024_03_10_NOON_MS],
        )
        .unwrap();
    });
    ready(&fx, 3);
    fx
}

fn set_fallback(fx: &Fx, value: &str) {
    let patch = serde_json::json!({ "capturedAtFallback": value });
    fx.pool
        .write(Priority::High, move |c| {
            Ok(mam_db::settings::update(c, patch.as_object().unwrap()).is_ok())
        })
        .unwrap()
        .then_some(())
        .unwrap();
}

fn captured_request(dir: SortDirection) -> SearchRequest {
    let mut r = request("");
    r.sort = SortSpec {
        field: SortField::CapturedAt,
        direction: dir,
    };
    r
}

fn month_sections(fx: &Fx, dir: SortDirection) -> Vec<(String, i64)> {
    let mut r = index_request(SortField::CapturedAt, dir);
    r.filters = filters();
    let idx = fx.search.search_index(&r).unwrap();
    idx.sections.into_iter().map(|s| (s.key, s.count)).collect()
}

#[test]
fn t_capfb_ipc_01_default_uses_modified_for_assets_without_a_capture_date_and_none_leaves_them_last()
 {
    // Arrange: 설정을 저장하지 않은 상태 = 기본값 "modified"
    let fx = capfb_fixture();

    // Act + Assert (기본 = 수정일 대체): 2024-05(수정일) > 2024-03(촬영일) > 2022-01(수정일)
    assert_eq!(
        ids_of(
            &fx.search
                .search(&captured_request(SortDirection::Desc))
                .unwrap()
        ),
        [2, 1, 3]
    );
    assert_eq!(
        ids_of(
            &fx.search
                .search(&captured_request(SortDirection::Asc))
                .unwrap()
        ),
        [3, 1, 2]
    );

    // Act + Assert ("none"): 촬영일 있는 자산이 먼저, 없는 자산은 방향과 무관하게 뒤(asset_id 순) — 같은 인덱스, 재색인 없음
    set_fallback(&fx, "none");
    assert_eq!(
        ids_of(
            &fx.search
                .search(&captured_request(SortDirection::Desc))
                .unwrap()
        ),
        [1, 2, 3]
    );
    assert_eq!(
        ids_of(
            &fx.search
                .search(&captured_request(SortDirection::Asc))
                .unwrap()
        ),
        [1, 2, 3]
    );

    // 다시 "modified"
    set_fallback(&fx, "modified");
    assert_eq!(
        ids_of(
            &fx.search
                .search(&captured_request(SortDirection::Desc))
                .unwrap()
        ),
        [2, 1, 3]
    );
}

#[test]
fn t_capfb_ipc_02_month_sections_follow_the_setting() {
    // Arrange
    let fx = capfb_fixture();

    // Act + Assert (modified): 수정일로 대체된 달이 구간이 되고 "none" 이 없다
    assert_eq!(
        month_sections(&fx, SortDirection::Desc),
        [
            ("2024-05".to_string(), 1),
            ("2024-03".to_string(), 1),
            ("2022-01".to_string(), 1)
        ]
    );

    // Act + Assert (none): 촬영일 있는 달 + "none" 2건
    set_fallback(&fx, "none");
    assert_eq!(
        month_sections(&fx, SortDirection::Desc),
        [("2024-03".to_string(), 1), ("none".to_string(), 2)]
    );
}

#[test]
fn t_capfb_ipc_03_captured_range_filter_follows_the_setting() {
    // Arrange: 2024-04-01 이후 (UTC 2024-04-01 00:00:00)
    let fx = capfb_fixture();
    let ranged = |from: Option<i64>, to: Option<i64>| {
        let mut r = captured_request(SortDirection::Desc);
        r.filters.captured_from = from;
        r.filters.captured_to = to;
        r
    };
    let after_april = ranged(Some(1_711_929_600_000), None);
    let march = ranged(Some(1_709_251_200_000), Some(1_711_929_599_000));

    // Act + Assert (modified): 수정일이 대체된 asset 2 가 걸린다
    assert_eq!(ids_of(&fx.search.search(&after_april).unwrap()), [2]);
    assert_eq!(ids_of(&fx.search.search(&march).unwrap()), [1]);

    // Act + Assert (none): 촬영일 없는 자산은 범위 필터에서 제외
    set_fallback(&fx, "none");
    assert!(fx.search.search(&after_april).unwrap().items.is_empty());
    assert_eq!(ids_of(&fx.search.search(&march).unwrap()), [1]);
}

#[test]
fn t_capfb_ipc_04_modified_ms_is_floored_to_seconds_at_the_range_boundary() {
    // Arrange: asset 3 수정일 = 2022-01-15 12:00:00.999 -> 색인 값 ...000 초
    let fx = capfb_fixture();
    let to = |ms: i64| {
        let mut r = captured_request(SortDirection::Desc);
        r.filters.captured_to = Some(ms);
        r
    };

    // Act + Assert: 상한이 정확히 12:00:00.000 이면 포함 (내림 값과 같다), 11:59:59.999 이면 제외
    assert_eq!(
        ids_of(&fx.search.search(&to(T_2022_01_15_NOON_MS)).unwrap()),
        [3]
    );
    assert!(
        fx.search
            .search(&to(T_2022_01_15_NOON_MS - 1))
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn t_capfb_ipc_05_a_cursor_issued_before_the_toggle_is_stale_after_it() {
    // Arrange: modified 정렬 1페이지 (limit 1) 의 다음 커서
    let fx = capfb_fixture();
    let mut first = captured_request(SortDirection::Desc);
    first.limit = 1;
    let cursor = fx.search.search(&first).unwrap().next_cursor.unwrap();
    let mut next = first.clone();
    next.cursor = Some(cursor);
    assert!(
        fx.search.search(&next).is_ok(),
        "same setting: cursor is accepted"
    );

    // Act: 설정 전환
    set_fallback(&fx, "none");

    // Assert: 다른 필드로 만든 커서를 새 정렬에 재사용하지 못한다 (첫 페이지부터 다시)
    assert_eq!(
        code_of(fx.search.search(&next).unwrap_err()),
        ErrorCode::InvalidInput
    );
    assert_eq!(ids_of(&fx.search.search(&first).unwrap()), [1]);
}

#[test]
fn t_capfb_ipc_06_unreadable_setting_value_falls_back_to_modified() {
    // Arrange: 손상된 저장값
    let fx = capfb_fixture();
    fx.pool
        .write(Priority::High, |c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES ('captured_at_fallback', '\"bogus\"')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

    // Act + Assert: 기본값(수정일)으로 동작
    assert_eq!(
        ids_of(
            &fx.search
                .search(&captured_request(SortDirection::Desc))
                .unwrap()
        ),
        [2, 1, 3]
    );
}

// ---------------------------------------------------------------- 별점 (T-search-rating-ipc-*)
// Categories: Happy (필터·정렬·요약 노출) / Validation (범위 밖 경계) / Business rule (편집 반영)

fn wait_total(fx: &Fx, req: &SearchRequest, expected: i64) -> SearchPage {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let p = fx.search.search(req).unwrap();
        if p.total == expected {
            return p;
        }
        assert!(Instant::now() < deadline, "rating edit was not indexed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn t_search_rating_ipc_01_rating_filter_sort_and_summary_follow_set_rating() {
    // Arrange: 1=★5, 2=★2, 3=미평가
    let fx = fixture();
    ready(&fx, 3);
    let ud = UserData::new(Arc::clone(&fx.pool), Arc::new(SystemClock));
    ud.set_rating(&[1], 5).unwrap();
    ud.set_rating(&[2], 2).unwrap();
    fx.search.wake();
    let mut by_min = request("");
    by_min.filters.rating_min = Some(2);

    // Act
    let both = wait_total(&fx, &by_min, 2);
    let mut exact = request("");
    exact.filters.rating_min = Some(5);
    exact.filters.rating_max = Some(5);
    let mut unrated = request("");
    unrated.filters.rating_min = Some(0);
    unrated.filters.rating_max = Some(0);
    let mut sorted = request("");
    sorted.sort = SortSpec {
        field: SortField::Rating,
        direction: SortDirection::Desc,
    };

    // Assert
    assert_eq!(ids_of(&both), [2, 1], "name order");
    assert_eq!(
        both.items.iter().map(|i| i.rating).collect::<Vec<_>>(),
        [2, 5]
    );
    assert_eq!(ids_of(&fx.search.search(&exact).unwrap()), [1]);
    assert_eq!(ids_of(&fx.search.search(&unrated).unwrap()), [3]);
    assert_eq!(ids_of(&fx.search.search(&sorted).unwrap()), [1, 2, 3]);
    sorted.sort.direction = SortDirection::Asc;
    assert_eq!(ids_of(&fx.search.search(&sorted).unwrap()), [3, 2, 1]);
    let idx = fx
        .search
        .search_index(&SearchIndexRequest {
            query: String::new(),
            filters: by_min.filters.clone(),
            sort: sorted.sort,
            mode: None,
        })
        .unwrap();
    assert_eq!((idx.total, idx.kind), (2, IndexKind::None));
}

#[test]
fn t_search_rating_ipc_02_rating_bounds_above_5_are_invalid_input_in_search_and_index() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);

    for (min, max) in [(Some(6), None), (None, Some(6)), (Some(0), Some(255))] {
        let mut r = request("");
        r.filters.rating_min = min;
        r.filters.rating_max = max;

        // Act
        let err = fx.search.search(&r).unwrap_err();
        let idx_err = fx
            .search
            .search_index(&SearchIndexRequest {
                query: String::new(),
                filters: r.filters.clone(),
                sort: r.sort,
                mode: None,
            })
            .unwrap_err();

        // Assert
        assert_eq!(code_of(err), ErrorCode::InvalidInput, "{min:?}..{max:?}");
        assert_eq!(
            code_of(idx_err),
            ErrorCode::InvalidInput,
            "{min:?}..{max:?}"
        );
    }
    // 경계값 5 는 허용
    let mut ok = request("");
    ok.filters.rating_min = Some(5);
    ok.filters.rating_max = Some(5);
    assert_eq!(fx.search.search(&ok).unwrap().total, 0);
}

// ---------------------------------------------------------------- 컬렉션 순서 (t_ipc_44, 45)
// Categories: Validation (44: 컬렉션 없는 collectionOrder) / Business rule (45: 재배치 즉시 반영, outbox 없음)

#[test]
fn t_ipc_44_collection_order_without_a_collection_is_invalid_input_for_search_and_index() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let mut r = request("");
    r.sort = SortSpec {
        field: SortField::CollectionOrder,
        direction: SortDirection::Asc,
    };

    // Act + Assert: search_assets / search_index 모두 컬렉션 없이는 거부
    assert_eq!(
        code_of(fx.search.search(&r).unwrap_err()),
        ErrorCode::InvalidInput
    );
    let idx = index_request(SortField::CollectionOrder, SortDirection::Asc);
    assert_eq!(
        code_of(fx.search.search_index(&idx).unwrap_err()),
        ErrorCode::InvalidInput
    );
    // 컬렉션이 있으면 search_index 는 구간 없는 목차를 돌려준다
    let mut with = index_request(SortField::CollectionOrder, SortDirection::Asc);
    with.filters.collection_id = Some(1);
    assert_eq!(fx.search.search_index(&with).unwrap().kind, IndexKind::None);
}

#[test]
fn t_ipc_45_collection_order_page_reflects_a_reorder_immediately_without_outbox() {
    // Arrange: 컬렉션에 3,1,2 순서로 추가 -> 색인에 컬렉션 id 가 반영될 때까지 기다린다
    let fx = fixture();
    ready(&fx, 3);
    let ud = UserData::new(Arc::clone(&fx.pool), Arc::new(SystemClock));
    let cid = ud.create_collection("순서").unwrap().collection_id;
    ud.add_to_collection(cid, &[3, 1, 2]).unwrap();
    fx.search.wake();
    let mut r = request("");
    r.filters.collection_id = Some(cid);
    r.sort = SortSpec {
        field: SortField::CollectionOrder,
        direction: SortDirection::Asc,
    };
    let first = wait_total(&fx, &r, 3);
    assert_eq!(ids_of(&first), [3, 1, 2]);
    let outbox_before: i64 = fx
        .pool
        .read(|c| Ok(c.query_row("SELECT count(*) FROM search_outbox", [], |r| r.get(0))?))
        .unwrap();

    // Act: 재배치 직후 — 색인 대기 없이 바로 조회
    ud.reorder_collection(cid, &[3], mam_core::userdata::ReorderTarget::End)
        .unwrap();
    let after = fx.search.search(&r).unwrap();
    let mut desc = r.clone();
    desc.sort.direction = SortDirection::Desc;

    // Assert
    assert_eq!(ids_of(&after), [1, 2, 3]);
    assert_eq!(ids_of(&fx.search.search(&desc).unwrap()), [3, 2, 1]);
    let outbox_after: i64 = fx
        .pool
        .read(|c| Ok(c.query_row("SELECT count(*) FROM search_outbox", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(
        outbox_after, outbox_before,
        "재배치는 outbox 를 남기지 않는다"
    );
    // 하이브리드 요청이 와도 순서 정렬은 키워드 경로로 처리된다 (degraded 표시)
    let mut hybrid = r.clone();
    hybrid.query = "beach".into();
    hybrid.mode = Some(SearchMode::Hybrid);
    let h = fx.search.search(&hybrid).unwrap();
    assert_eq!(h.degraded, Some(true));
}

#[path = "search_hide_svc_tests.rs"]
mod hide;
