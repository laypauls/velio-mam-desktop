//! 컬렉션 순서·소속 집계·재생 대기열 (T-cord-*) — 실제 SQLite(마이그레이션 적용)에 직접 자산을 넣어 쓴다.
//!
//! 분류: Happy (01 02 04 05 14 17) / Boundary (11 간격 소진·rebalance, 10 빈 입력·1만 초과, 15 중복 입력) /
//! Validation (06~10) / Business rule (03 NULL 정규화, 12 revision·outbox 불변, 18 비활성·오프라인) /
//! Concurrency·멱등 (13) / Performance 계약 (16 쿼리 계획)

use std::sync::Arc;

use mam_core::SharedClock;
use mam_core::query::{
    Availability, MediaKind, QueryService, SortDirection, SortField, SortSpec,
};
use mam_core::scan::IngestError;
use mam_core::userdata::{MEMBERSHIP_SQL, ReorderTarget, UserData};
use mam_db::{DbPool, SystemClock, migrate, open};
use rusqlite::{Connection, params};
use tempfile::TempDir;

const STEP: i64 = 1 << 20;
const POS_MAX: i64 = 1 << 62;

struct Fx {
    dir: TempDir,
    pool: Arc<DbPool>,
    ud: UserData,
}

impl Fx {
    fn new() -> Self {
        Self::new_with_readers(2)
    }

    fn new_with_readers(readers: usize) -> Self {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("mam.sqlite");
        let mut conn = open(&db).unwrap();
        migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
        conn.execute_batch(
            "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at) VALUES
               (1, 'fixed', 'v1', 'Main', 'C:\\Media', 'online', 0),
               (2, 'removable', 'v2', 'USB', 'E:\\', 'offline', 0);",
        )
        .unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db, readers).unwrap());
        let clock: SharedClock = Arc::new(SystemClock);
        let ud = UserData::new(Arc::clone(&pool), clock);
        Self { dir, pool, ud }
    }

    fn raw(&self) -> Connection {
        open(&self.dir.path().join("mam.sqlite")).unwrap()
    }

    /// 활성 자산 `1..=n` (볼륨 1, 이미지)
    fn assets(&self, n: i64) {
        for id in 1..=n {
            self.asset(id, 1, "image");
        }
    }

    fn asset(&self, id: i64, vol: i64, media: &str) {
        let c = self.raw();
        c.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, ?2, 10, 0, 0, 0)",
            params![id, media],
        )
        .unwrap();
        c.execute(
            "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
             VALUES (?1, ?2, ?3, ?3, 0)",
            params![id, vol, format!("dir/file{id}.bin")],
        )
        .unwrap();
    }

    fn collection(&self, name: &str) -> i64 {
        self.ud.create_collection(name).unwrap().collection_id
    }

    /// 표준 순서의 asset_id
    fn order(&self, cid: i64) -> Vec<i64> {
        let c = self.raw();
        let mut s = c
            .prepare(
                "SELECT asset_id FROM collection_assets WHERE collection_id = ?1
                 ORDER BY position IS NULL, position, added_at, asset_id",
            )
            .unwrap();
        s.query_map([cid], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    fn positions(&self, cid: i64) -> Vec<(i64, Option<i64>)> {
        let c = self.raw();
        let mut s = c
            .prepare("SELECT asset_id, position FROM collection_assets WHERE collection_id = ?1 ORDER BY asset_id")
            .unwrap();
        s.query_map([cid], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
    }

    fn count(&self, sql: &str) -> i64 {
        self.raw().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

fn before(id: i64) -> ReorderTarget {
    ReorderTarget::Before { asset_id: id }
}

#[test]
fn t_cord_01_add_appends_new_items_in_request_order() {
    // Arrange
    let f = Fx::new();
    f.assets(9);
    let c = f.collection("c");

    // Act: id 순서(5,3,9)가 아니라 요청 순서가 그대로 순서가 된다. 요청 안 중복은 첫 위치만 남는다
    f.ud.add_to_collection(c, &[5, 3, 9, 5]).unwrap();

    // Assert
    assert_eq!(f.order(c), [5, 3, 9]);
    let pos: Vec<i64> = f.order(c).iter().map(|a| f.positions(c).iter().find(|p| p.0 == *a).unwrap().1.unwrap()).collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]), "{pos:?}");
}

#[test]
fn t_cord_02_adding_an_existing_member_keeps_its_position_and_new_ones_go_last() {
    // Arrange
    let f = Fx::new();
    f.assets(9);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[5, 3, 9]).unwrap();
    let before_pos = f.positions(c);

    // Act: 3 은 이미 있다 -> 그대로, 1·7 만 요청 순서(7, 1)로 뒤에
    f.ud.add_to_collection(c, &[7, 3, 1]).unwrap();

    // Assert
    assert_eq!(f.order(c), [5, 3, 9, 7, 1]);
    for (id, p) in before_pos {
        assert_eq!(f.positions(c).iter().find(|x| x.0 == id).unwrap().1, p);
    }
}

#[test]
fn t_cord_03_null_positions_from_an_old_app_are_normalized_on_the_next_add() {
    // Arrange: 위치 있는 행 1개 + 구 버전 앱이 넣은 NULL 행 2개 (added_at 100, 50)
    let f = Fx::new();
    f.assets(5);
    let c = f.collection("c");
    f.raw()
        .execute_batch(&format!(
            "INSERT INTO collection_assets (collection_id, asset_id, added_at, position) VALUES ({c}, 1, 999, {STEP});
             INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES ({c}, 2, 100);
             INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES ({c}, 3, 50);"
        ))
        .unwrap();
    assert_eq!(f.order(c), [1, 3, 2], "NULL 은 위치 있는 행 뒤, added_at 순");

    // Act
    f.ud.add_to_collection(c, &[4]).unwrap();

    // Assert: 순서 유지 + NULL 없음 + 새 항목은 맨 뒤
    assert_eq!(f.order(c), [1, 3, 2, 4]);
    assert!(f.positions(c).iter().all(|p| p.1.is_some()));
}

#[test]
fn t_cord_04_reorder_moves_before_a_target_to_the_start_and_to_the_end() {
    // Arrange
    let f = Fx::new();
    f.assets(5);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3, 4, 5]).unwrap();

    // Act + Assert
    f.ud.reorder_collection(c, &[4], before(2)).unwrap();
    assert_eq!(f.order(c), [1, 4, 2, 3, 5]);
    f.ud.reorder_collection(c, &[5], ReorderTarget::Start).unwrap();
    assert_eq!(f.order(c), [5, 1, 4, 2, 3]);
    f.ud.reorder_collection(c, &[1], ReorderTarget::End).unwrap();
    assert_eq!(f.order(c), [5, 4, 2, 3, 1]);
    f.ud.reorder_collection(c, &[3], before(5)).unwrap(); // 맨 앞 항목 앞 (lo 없음)
    assert_eq!(f.order(c), [3, 5, 4, 2, 1]);
}

#[test]
fn t_cord_05_multi_move_keeps_the_current_relative_order_as_one_block() {
    // Arrange
    let f = Fx::new();
    f.assets(6);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3, 4, 5, 6]).unwrap();

    // Act: 요청 순서(6,2,4)는 무시 — 현재 순서(2,4,6)가 유지된다
    f.ud.reorder_collection(c, &[6, 2, 4], before(3)).unwrap();
    assert_eq!(f.order(c), [1, 2, 4, 6, 3, 5]);
    f.ud.reorder_collection(c, &[6, 1], ReorderTarget::End).unwrap();
    assert_eq!(f.order(c), [2, 4, 3, 5, 1, 6]);
    f.ud.reorder_collection(c, &[3, 5], ReorderTarget::Start).unwrap();
    assert_eq!(f.order(c), [3, 5, 2, 4, 1, 6]);
}

#[test]
fn t_cord_06_reorder_on_a_missing_collection_is_not_found() {
    let f = Fx::new();
    f.assets(1);

    let err = f.ud.reorder_collection(999, &[1], ReorderTarget::End);

    assert!(matches!(err, Err(IngestError::NotFound)), "{err:?}");
}

#[test]
fn t_cord_07_non_member_in_the_moved_set_rejects_the_whole_call() {
    // Arrange: 4 는 존재하는 자산이지만 이 컬렉션 소속이 아니다
    let f = Fx::new();
    f.assets(4);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3]).unwrap();
    let snapshot = f.positions(c);

    // Act
    let err = f.ud.reorder_collection(c, &[3, 4], ReorderTarget::Start);

    // Assert: 전부 아니면 전무 — 3 도 옮겨지지 않았다
    assert!(matches!(err, Err(IngestError::InvalidInput("assetIds"))), "{err:?}");
    assert_eq!(f.positions(c), snapshot);
    assert_eq!(f.order(c), [1, 2, 3]);
}

#[test]
fn t_cord_08_before_target_inside_the_moved_set_is_rejected() {
    let f = Fx::new();
    f.assets(3);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3]).unwrap();

    let err = f.ud.reorder_collection(c, &[1, 2], before(2));

    assert!(matches!(err, Err(IngestError::InvalidInput("beforeAssetId"))), "{err:?}");
    assert_eq!(f.order(c), [1, 2, 3]);
}

#[test]
fn t_cord_09_before_target_that_is_not_a_member_is_rejected() {
    let f = Fx::new();
    f.assets(4);
    let c = f.collection("c");
    let other = f.collection("other");
    f.ud.add_to_collection(c, &[1, 2]).unwrap();
    f.ud.add_to_collection(other, &[3]).unwrap();

    // 소속이 아닌 자산(3: 다른 컬렉션, 4: 어디에도 없음)을 기준으로 삼을 수 없다
    for target in [3, 4] {
        let err = f.ud.reorder_collection(c, &[1], before(target));
        assert!(matches!(err, Err(IngestError::InvalidInput("beforeAssetId"))), "{target}: {err:?}");
    }
    assert_eq!(f.order(c), [1, 2]);
}

#[test]
fn t_cord_10_empty_and_oversized_requests_are_invalid() {
    let f = Fx::new();
    f.assets(1);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1]).unwrap();

    let empty = f.ud.reorder_collection(c, &[], ReorderTarget::End);
    let big: Vec<i64> = (1..=10_001).collect();
    let too_many = f.ud.reorder_collection(c, &big, ReorderTarget::End);
    let add_too_many = f.ud.add_to_collection(c, &big);

    assert!(matches!(empty, Err(IngestError::InvalidInput("assetIds"))), "{empty:?}");
    assert!(matches!(too_many, Err(IngestError::InvalidInput("too many assets"))), "{too_many:?}");
    assert!(matches!(add_too_many, Err(IngestError::InvalidInput("too many assets"))));
}

#[test]
fn t_cord_11_repeated_inserts_into_one_gap_rebalance_and_keep_order_within_bounds() {
    // Arrange: 32 개. 매번 마지막 항목을 두 번째 항목 앞(좁아지는 같은 간격)에 끼운다 -> 2^20 간격이 소진돼 rebalance
    let f = Fx::new();
    f.assets(32);
    let c = f.collection("c");
    let all: Vec<i64> = (1..=32).collect();
    f.ud.add_to_collection(c, &all).unwrap();
    let mut model = all.clone();
    let mut renumbered = false;

    // Act + Assert: 매 단계 모델과 같은 순서, 위치는 엄격 증가·범위 안
    for _ in 0..30 {
        let last = *model.last().unwrap();
        let anchor = model[1];
        f.ud.reorder_collection(c, &[last], before(anchor)).unwrap();
        model.pop();
        model.insert(1, last);
        assert_eq!(f.order(c), model);
        let pos: Vec<i64> = model.iter().map(|a| f.positions(c).iter().find(|p| p.0 == *a).unwrap().1.unwrap()).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{pos:?}");
        assert!(pos.iter().all(|p| p.abs() <= POS_MAX));
        renumbered |= pos.iter().all(|p| p % STEP == 0);
    }
    assert!(renumbered, "간격이 소진되면 전체 번호를 다시 매겨야 한다");
}

#[test]
fn t_cord_12_reorder_does_not_touch_revision_or_outbox() {
    // Arrange
    let f = Fx::new();
    f.assets(3);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3]).unwrap();
    let rev = "SELECT sum(revision) FROM media_assets";
    let outbox = "SELECT count(*) FROM search_outbox";
    let (r0, o0) = (f.count(rev), f.count(outbox));
    assert!(o0 >= 3, "추가는 outbox 를 남긴다");

    // Act
    f.ud.reorder_collection(c, &[3], ReorderTarget::Start).unwrap();

    // Assert
    assert_eq!((f.count(rev), f.count(outbox)), (r0, o0));
}

#[test]
fn t_cord_13_repeating_the_same_reorder_gives_the_same_order() {
    let f = Fx::new();
    f.assets(5);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3, 4, 5]).unwrap();

    f.ud.reorder_collection(c, &[2, 5], before(4)).unwrap();
    let once = (f.order(c), f.positions(c));
    f.ud.reorder_collection(c, &[2, 5], before(4)).unwrap();

    assert_eq!(f.order(c), [1, 3, 2, 5, 4]);
    assert_eq!((f.order(c), f.positions(c)), once);
}

#[test]
fn t_cord_14_membership_counts_all_partial_and_none() {
    // Arrange: 컬렉션 A = {1,2,3}, B = {1}, C = 비어 있음
    let f = Fx::new();
    f.assets(4);
    let (a, b, c) = (f.collection("a"), f.collection("b"), f.collection("c"));
    f.ud.add_to_collection(a, &[1, 2, 3]).unwrap();
    f.ud.add_to_collection(b, &[1]).unwrap();

    // Act
    let m = f.ud.collection_membership(&[1, 2]).unwrap();
    let none = f.ud.collection_membership(&[4]).unwrap();

    // Assert: A=2(전부), B=1(일부), C 는 0 이라 없다
    let got: Vec<(i64, i64)> = m.iter().map(|x| (x.collection_id, x.count)).collect();
    assert_eq!(got, [(a, 2), (b, 1)]);
    assert!(!got.iter().any(|x| x.0 == c));
    assert!(none.is_empty());
    assert!(f.ud.collection_membership(&[]).unwrap().is_empty());
    let json = serde_json::to_value(&m[0]).unwrap();
    assert_eq!(json, serde_json::json!({"collectionId": a, "count": 2}));
}

#[test]
fn t_cord_15_membership_counts_duplicate_input_ids_once_and_caps_the_input() {
    let f = Fx::new();
    f.assets(2);
    let a = f.collection("a");
    f.ud.add_to_collection(a, &[1, 2]).unwrap();

    let m = f.ud.collection_membership(&[1, 1, 1, 2]).unwrap();
    let big: Vec<i64> = (1..=10_001).collect();

    assert_eq!(m.iter().map(|x| x.count).collect::<Vec<_>>(), [2]);
    assert!(matches!(f.ud.collection_membership(&big), Err(IngestError::InvalidInput("too many assets"))));
}

#[test]
fn t_cord_16_membership_query_plan_uses_the_asset_index() {
    let f = Fx::new();
    let c = f.raw();
    let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {MEMBERSHIP_SQL}")).unwrap();
    let plan: Vec<String> = s
        .query_map(["[1,2]"], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let plan = plan.join(" | ");
    println!("PLAN membership: {plan}");

    assert!(plan.contains("idx_collection_assets_asset"), "{plan}");
    assert!(!plan.contains("SCAN x"), "{plan}");
}

#[test]
fn t_cord_17_play_queue_follows_collection_order_with_null_positions_last() {
    // Arrange: 1,2 위치 있음 (2 가 먼저), 3·4 는 구 버전 앱이 넣은 NULL (added_at 순)
    let f = Fx::new();
    f.assets(4);
    f.raw().execute("INSERT INTO media_metadata (asset_id, duration_ms) VALUES (2, 4500)", []).unwrap();
    f.raw().execute("INSERT INTO asset_user_data (asset_id, title, updated_at) VALUES (2, '둘째', 0)", []).unwrap();
    let c = f.collection("c");
    f.raw()
        .execute_batch(&format!(
            "INSERT INTO collection_assets (collection_id, asset_id, added_at, position) VALUES ({c}, 1, 1, {}), ({c}, 2, 2, {});
             INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES ({c}, 4, 10), ({c}, 3, 20);",
            2 * STEP,
            STEP
        ))
        .unwrap();

    // Act
    let q = f.ud.collection_play_queue(c).unwrap();

    // Assert: 순서 + 가벼운 필드
    assert_eq!(q.iter().map(|i| i.asset_id).collect::<Vec<_>>(), [2, 1, 4, 3]);
    assert_eq!(q[0].file_name, "file2.bin");
    assert_eq!(q[0].title.as_deref(), Some("둘째"));
    assert_eq!(q[0].duration_ms, Some(4500));
    assert_eq!((q[1].title.clone(), q[1].duration_ms), (None, None));
    assert_eq!((q[0].media_type, q[0].availability), (MediaKind::Image, Availability::Online));
    let json = serde_json::to_value(&q[0]).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"assetId": 2, "mediaType": "image", "availability": "online",
            "fileName": "file2.bin", "title": "둘째", "durationMs": 4500})
    );
}

#[test]
fn t_cord_18_play_queue_excludes_inactive_assets_and_reports_offline_and_missing() {
    // Arrange: 1 정상, 2 오프라인 볼륨, 3 파일 없음, 4 대체됨(비활성), 5 위치 분리
    let f = Fx::new();
    f.asset(1, 1, "video");
    f.asset(2, 2, "audio");
    f.asset(3, 1, "image");
    f.asset(4, 1, "image");
    f.asset(5, 1, "image");
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3, 4, 5]).unwrap();
    let raw = f.raw();
    raw.execute("UPDATE file_locations SET presence = 'missing' WHERE asset_id = 3", []).unwrap();
    raw.execute("UPDATE media_assets SET status = 'superseded' WHERE asset_id = 4", []).unwrap();
    raw.execute("UPDATE file_locations SET detached_at = 1 WHERE asset_id = 5", []).unwrap();

    // Act
    let q = f.ud.collection_play_queue(c).unwrap();

    // Assert
    let got: Vec<_> = q.iter().map(|i| (i.asset_id, i.media_type, i.availability)).collect();
    assert_eq!(
        got,
        [
            (1, MediaKind::Video, Availability::Online),
            (2, MediaKind::Audio, Availability::Offline),
            (3, MediaKind::Image, Availability::Missing)
        ]
    );
}

#[test]
fn t_cord_19_play_queue_of_an_unknown_collection_is_not_found_and_an_empty_one_is_empty() {
    let f = Fx::new();
    let c = f.collection("empty");

    assert!(matches!(f.ud.collection_play_queue(999), Err(IngestError::NotFound)));
    assert!(f.ud.collection_play_queue(c).unwrap().is_empty());
}

#[test]
fn t_cord_20_collection_order_sort_serializes_as_camel_case_contract() {
    // Arrange
    let sort = SortSpec { field: SortField::CollectionOrder, direction: SortDirection::Asc };

    // Act
    let json = serde_json::to_value(sort).unwrap();

    // Assert: TS 계약 `collectionOrder` 와 일치
    assert_eq!(json, serde_json::json!({"field": "collectionOrder", "direction": "asc"}));
}

#[test]
fn t_cord_21_order_rows_return_position_and_added_at_and_unknown_collection_is_empty() {
    let f = Fx::new();
    f.assets(2);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[2, 1]).unwrap();
    f.raw()
        .execute(&format!("UPDATE collection_assets SET position = NULL WHERE collection_id = {c} AND asset_id = 1"), [])
        .unwrap();
    let q = QueryService::new(Arc::clone(&f.pool), Arc::new(SystemClock));

    let mut rows = q.collection_order_rows(c).unwrap();
    rows.sort_unstable_by_key(|r| r.0);

    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].0, rows[0].1), (1, None));
    assert_eq!((rows[1].0, rows[1].1), (2, Some(STEP)));
    assert!(q.collection_order_rows(999).unwrap().is_empty());
}

#[test]
fn t_cord_22_order_rows_query_uses_an_index_on_the_collection_prefix() {
    let f = Fx::new();
    let c = f.raw();
    let mut s = c
        .prepare("EXPLAIN QUERY PLAN SELECT asset_id, position, added_at FROM collection_assets WHERE collection_id = ?1")
        .unwrap();
    let plan = s.query_map([1], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect::<Vec<_>>().join(" | ");

    println!("PLAN order_rows: {plan}");
    assert!(plan.contains("SEARCH") && plan.contains("collection_id=?"), "{plan}");
}

#[test]
fn t_cord_23_exhausted_gaps_fall_back_to_a_full_renumber_that_keeps_block_order() {
    // Arrange: 위치를 연속 정수(1..=8)로 만들어 어떤 이웃 사이에도 간격이 없다
    let f = Fx::new();
    f.assets(8);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &(1..=8).collect::<Vec<_>>()).unwrap();
    let dense = |f: &Fx| {
        let order = f.order(c);
        for (i, a) in order.iter().enumerate() {
            f.raw()
                .execute(
                    "UPDATE collection_assets SET position = ?3 WHERE collection_id = ?1 AND asset_id = ?2",
                    params![c, a, i as i64 + 1],
                )
                .unwrap();
        }
    };
    dense(&f);

    // Act + Assert: Before — 이웃 사이 간격이 없어 전체 번호 재부여, 이동 블록은 현재 순서대로 한 덩어리
    f.ud.reorder_collection(c, &[7, 2], before(5)).unwrap();
    assert_eq!(f.order(c), [1, 3, 4, 2, 7, 5, 6, 8]);
    let multiples = |f: &Fx| f.positions(c).into_iter().all(|p| p.1.unwrap() % STEP == 0);
    assert!(multiples(&f), "폴백은 rank × STEP 으로 간격을 되찾는다");

    // Start — 맨 앞 위치가 하한에 붙어 있어 앞쪽에 둘 자리가 없다
    f.raw()
        .execute(
            "UPDATE collection_assets SET position = ?2 WHERE collection_id = ?1 AND asset_id = 1",
            params![c, -POS_MAX],
        )
        .unwrap();
    f.ud.reorder_collection(c, &[8, 3], ReorderTarget::Start).unwrap();
    assert_eq!(f.order(c), [3, 8, 1, 4, 2, 7, 5, 6]);
    assert!(multiples(&f));

    // End — 정상 간격이면 폴백 없이 뒤에 붙는다
    dense(&f);
    f.ud.reorder_collection(c, &[3, 5], ReorderTarget::End).unwrap();
    assert_eq!(f.order(c), [8, 1, 4, 2, 7, 6, 3, 5]);
}

#[test]
fn t_cord_24_positions_near_the_upper_bound_are_renumbered_instead_of_overflowing() {
    // Arrange: 마지막 항목 위치가 상한 바로 아래
    let f = Fx::new();
    f.assets(4);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3]).unwrap();
    f.raw()
        .execute(
            "UPDATE collection_assets SET position = ?2 WHERE collection_id = ?1 AND asset_id = 3",
            params![c, POS_MAX - 5],
        )
        .unwrap();

    // Act: 맨 뒤로 이동(이웃 + STEP 이 상한을 넘는다)과 추가 모두 상한 안에서 끝난다
    f.ud.reorder_collection(c, &[1], ReorderTarget::End).unwrap();
    assert_eq!(f.order(c), [2, 3, 1]);
    f.ud.add_to_collection(c, &[4]).unwrap();

    // Assert
    assert_eq!(f.order(c), [2, 3, 1, 4]);
    let pos: Vec<i64> = f.order(c).iter().map(|a| f.positions(c).iter().find(|p| p.0 == *a).unwrap().1.unwrap()).collect();
    assert!(pos.windows(2).all(|w| w[0] < w[1]) && pos.iter().all(|p| p.abs() <= POS_MAX), "{pos:?}");
}

/// 풀 읽기 연결 1개짜리라 대기열 조회와 이 조회가 같은 연결을 쓴다 -> (mmap_size, cache_size)
fn read_settings(f: &Fx) -> (i64, i64) {
    f.pool
        .read(|c| {
            let mmap = c.query_row("PRAGMA mmap_size", [], |r| r.get::<_, i64>(0))?;
            let cache = c.query_row("PRAGMA cache_size", [], |r| r.get::<_, i64>(0))?;
            Ok((mmap, cache))
        })
        .unwrap()
}

#[test]
fn t_cord_25_play_queue_restores_the_connection_read_settings_on_success_and_not_found() {
    // Arrange: 읽기 연결 1개 풀 -> 모든 읽기가 같은 연결
    let f = Fx::new_with_readers(1);
    f.assets(3);
    let c = f.collection("c");
    f.ud.add_to_collection(c, &[1, 2, 3]).unwrap();
    let default = read_settings(&f);
    assert!(default.0 < (2 << 30) && default.1 > -262_144, "기본값이 이미 올라가 있다: {default:?}");

    // Act + Assert: 성공 후 원래 값
    assert_eq!(f.ud.collection_play_queue(c).unwrap().len(), 3);
    assert_eq!(read_settings(&f), default);
    // 오류(not_found) 경로 후에도 원래 값
    assert!(matches!(f.ud.collection_play_queue(999), Err(IngestError::NotFound)));
    assert_eq!(read_settings(&f), default);
    // 겹치는 호출: 여러 스레드가 동시에 불러도 (연결 1개라 직렬) 끝난 뒤 원래 값
    let ud = Arc::new(UserData::new(Arc::clone(&f.pool), Arc::new(SystemClock)));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let ud = Arc::clone(&ud);
            std::thread::spawn(move || ud.collection_play_queue(c).unwrap().len())
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), 3);
    }
    assert_eq!(read_settings(&f), default);
}
