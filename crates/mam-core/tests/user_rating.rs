//! 별점 0~5 (T-rating-*) — 실제 SQLite + 스캔으로 만든 자산.
//!
//! 분류: Happy (01, 06) / Boundary (02, 05) / Validation (02, 05) / Business rule (03, 04, 06) /
//! Concurrency·멱등 (03) / Performance (07: 쿼리 계획) / Compat (08: 저장한 검색 하위 호환, 09: 정렬 직렬화)

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::env::Env;
use mam_core::query::{SearchFilters, SortDirection, SortField, SortSpec, hydrate_page};
use mam_core::scan::{IngestError, scan_folder};
use mam_core::userdata::{COUNT_SQLS, UserData};

struct Fixture {
    env: Env,
    ud: UserData,
    ids: Vec<i64>,
}

fn fixture(n: u8) -> Fixture {
    let env = Env::new();
    for i in 0..n {
        env.write(&format!("lib/img{i}.jpg"), i + 1);
    }
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false)).unwrap();
    let ids = env.read(|c| {
        let mut s = c
            .prepare("SELECT asset_id FROM media_assets ORDER BY asset_id")
            .unwrap();
        s.query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    Fixture { env, ud, ids }
}

fn revision(f: &Fixture, id: i64) -> i64 {
    f.env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {id}"
    ))
}

fn outbox_count(f: &Fixture) -> i64 {
    f.env.count("SELECT count(*) FROM search_outbox")
}

fn stored(f: &Fixture, id: i64) -> Option<i64> {
    f.env.read(|c| {
        c.query_row(
            "SELECT rating FROM asset_user_data WHERE asset_id = ?1",
            [id],
            |r| r.get(0),
        )
        .ok()
    })
}

fn exec(f: &Fixture, sql: &str) {
    let sql = sql.to_string();
    f.env
        .pool
        .write(mam_db::Priority::Low, move |c| {
            c.execute_batch(&sql)?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_rating_01_set_rating_creates_or_updates_rows_and_is_visible_in_summaries() {
    // Arrange: a 는 메모 행이 있고 b 는 행이 없다
    let f = fixture(3);
    let (a, b, c) = (f.ids[0], f.ids[1], f.ids[2]);
    f.ud.set_memo(a, "keep me").unwrap();

    // Act
    f.ud.set_rating(&[a, b, b], 4).unwrap();

    // Assert: 저장값, 메모 유지, 행이 없던 c 는 그대로 (행 없음 = 0)
    assert_eq!(
        (stored(&f, a), stored(&f, b), stored(&f, c)),
        (Some(4), Some(4), None)
    );
    assert_eq!(
        f.env.count(&format!(
            "SELECT length(memo) FROM asset_user_data WHERE asset_id = {a}"
        )),
        7
    );
    let page = f.env.read(|conn| hydrate_page(conn, &[a, b, c]).unwrap());
    assert_eq!(page.iter().map(|s| s.rating).collect::<Vec<_>>(), [4, 4, 0]);
    let json = serde_json::to_value(&page[0]).unwrap();
    assert_eq!(json["rating"], serde_json::json!(4));
}

#[test]
fn t_rating_02_boundaries_0_and_5_are_accepted_and_6_or_more_is_rejected_without_changes() {
    // Arrange
    let f = fixture(2);
    let a = f.ids[0];

    // Act + Assert: 경계 5, 0
    f.ud.set_rating(&[a], 5).unwrap();
    assert_eq!(stored(&f, a), Some(5));
    f.ud.set_rating(&[a], 0).unwrap();
    assert_eq!(stored(&f, a), Some(0));

    // 6, 255 거부 - revision·outbox·값 불변
    let (rev, events) = (revision(&f, a), outbox_count(&f));
    for bad in [6u8, 7, 255] {
        assert!(
            matches!(
                f.ud.set_rating(&[a], bad),
                Err(IngestError::InvalidInput(_))
            ),
            "{bad}"
        );
    }
    assert_eq!(
        (stored(&f, a), revision(&f, a), outbox_count(&f)),
        (Some(0), rev, events)
    );
    // 범위 밖 값은 자산 목록이 비어 있어도 거부한다 (입력 검증이 먼저)
    assert!(matches!(
        f.ud.set_rating(&[], 6),
        Err(IngestError::InvalidInput(_))
    ));
}

#[test]
fn t_rating_03_same_value_is_a_noop_and_zero_keeps_the_row() {
    // Arrange
    let f = fixture(2);
    let (a, b) = (f.ids[0], f.ids[1]);
    f.ud.set_memo(a, "memo").unwrap();
    f.ud.set_rating(&[a], 3).unwrap();
    let (rev_a, rev_b, events) = (revision(&f, a), revision(&f, b), outbox_count(&f));

    // Act: 같은 값 재설정, 행 없는 자산에 0 설정 -> 전부 no-op
    f.ud.set_rating(&[a], 3).unwrap();
    f.ud.set_rating(&[b], 0).unwrap();

    // Assert
    assert_eq!(
        (revision(&f, a), revision(&f, b), outbox_count(&f)),
        (rev_a, rev_b, events)
    );
    assert_eq!(
        stored(&f, b),
        None,
        "setting 0 on a rowless asset must not create a row"
    );

    // Act: a 는 이미 3, b 만 바뀐다 -> 바뀐 자산만 +1
    f.ud.set_rating(&[a, b], 3).unwrap();
    assert_eq!((revision(&f, a), revision(&f, b)), (rev_a, rev_b + 1));
    f.ud.set_rating(&[a], 0).unwrap();

    // Assert: 0 으로 내려도 행(메모)은 유지된다
    assert_eq!(stored(&f, a), Some(0));
    assert_eq!(
        f.env.count(&format!(
            "SELECT length(memo) FROM asset_user_data WHERE asset_id = {a}"
        )),
        4
    );
    assert_eq!(revision(&f, a), rev_a + 1);
}

#[test]
fn t_rating_04_revision_and_high_priority_outbox_are_written_in_the_same_transaction() {
    // Arrange
    let f = fixture(3);
    let (a, b, c) = (f.ids[0], f.ids[1], f.ids[2]);
    let (rev_a, events) = (revision(&f, a), outbox_count(&f));

    // Act
    f.ud.set_rating(&[a, b], 2).unwrap();

    // Assert: 바뀐 자산마다 이벤트 1건, revision 일치, priority 0
    assert_eq!(outbox_count(&f), events + 2);
    let ev: (i64, i64, String) = f.env.read(|conn| {
        conn.query_row(
            "SELECT revision, priority, event_type FROM search_outbox WHERE asset_id = ?1 ORDER BY event_id DESC LIMIT 1",
            [a],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    });
    assert_eq!(ev, (rev_a + 1, 0, "upsert".to_string()));

    // 존재하지 않는 자산이 섞이면 전체 롤백 (값·revision·outbox 모두 불변)
    let (rev_c, events) = (revision(&f, c), outbox_count(&f));
    assert!(matches!(
        f.ud.set_rating(&[c, 424_242], 5),
        Err(IngestError::NotFound)
    ));
    assert_eq!(
        (stored(&f, c), revision(&f, c), outbox_count(&f)),
        (None, rev_c, events)
    );
}

#[test]
fn t_rating_05_empty_list_is_a_noop_and_more_than_10000_ids_are_rejected() {
    // Arrange
    let f = fixture(1);
    let events = outbox_count(&f);

    // Act + Assert
    f.ud.set_rating(&[], 3).unwrap();
    assert_eq!(outbox_count(&f), events);
    let many: Vec<i64> = (1..=10_001).collect();
    assert!(matches!(
        f.ud.set_rating(&many, 3),
        Err(IngestError::InvalidInput(_))
    ));
}

#[test]
fn t_rating_06_counts_have_6_slots_count_unrated_without_rows_and_follow_the_gallery_default() {
    // Arrange: 6 자산, 행 없음
    let f = fixture(6);
    let id = f.ids.clone();
    assert_eq!(f.ud.rating_counts().unwrap(), [6, 0, 0, 0, 0, 0]);

    // Act: ★5 1개, ★3 2개, 한 자산은 ★1 후 0 (0 행)
    f.ud.set_rating(&[id[0]], 5).unwrap();
    f.ud.set_rating(&[id[1], id[2]], 3).unwrap();
    f.ud.set_rating(&[id[3]], 1).unwrap();
    f.ud.set_rating(&[id[3]], 0).unwrap();

    // Assert: 미평가 = 행 없음 2 + 0 행 1
    assert_eq!(f.ud.rating_counts().unwrap(), [3, 0, 0, 2, 0, 1]);

    // 파일 없음 -> 제외
    exec(
        &f,
        &format!(
            "UPDATE file_locations SET presence = 'missing' WHERE asset_id = {}",
            id[2]
        ),
    );
    assert_eq!(f.ud.rating_counts().unwrap(), [3, 0, 0, 1, 0, 1]);

    // 볼륨 오프라인 -> 포함 (파일 없음이 아닌 위치는 그대로 센다)
    exec(&f, "UPDATE volumes SET availability = 'offline'");
    assert_eq!(f.ud.rating_counts().unwrap(), [3, 0, 0, 1, 0, 1]);

    // 삭제·대체된 자산은 제외
    exec(
        &f,
        &format!(
            "UPDATE media_assets SET status = 'deleted' WHERE asset_id = {};
             UPDATE media_assets SET status = 'superseded' WHERE asset_id = {}",
            id[0], id[5]
        ),
    );
    assert_eq!(f.ud.rating_counts().unwrap(), [2, 0, 0, 1, 0, 0]);
}

#[test]
fn t_rating_07_count_queries_use_the_partial_index_and_never_scan_user_data() {
    // Arrange
    let f = fixture(2);

    // Act
    let plans: Vec<String> = f.env.read(|c| {
        COUNT_SQLS
            .iter()
            .flat_map(|sql| {
                let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
                s.query_map([], |r| r.get::<_, String>(3))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect::<Vec<_>>()
            })
            .collect()
    });

    // Assert
    let all = plans.join("\n");
    assert!(all.contains("idx_asset_user_data_rating"), "{all}");
    assert!(!all.contains("SCAN u"), "{all}");
    assert!(!all.contains("SCAN a"), "{all}");
}

#[test]
fn t_rating_08_saved_filters_without_rating_fields_still_deserialize_and_new_fields_round_trip() {
    // Arrange: 별점 도입 전 앱이 저장한 필터 JSON (ratingMin/Max 없음)
    let old = r#"{"mediaTypes":[],"includeOffline":true,"includeMissing":false,
        "capturedFrom":null,"capturedTo":null,"sizeMin":null,"sizeMax":null,
        "durationMinMs":null,"durationMaxMs":null,"folder":null,"favoritesOnly":false,
        "collectionId":null,"tagIds":[]}"#;

    // Act
    let mut f: SearchFilters = serde_json::from_str(old).unwrap();

    // Assert: 필드가 없으면 None, 기본 필터로 인식
    assert_eq!((f.rating_min, f.rating_max), (None, None));
    assert!(f.is_default());
    f.rating_min = Some(0);
    assert!(
        !f.is_default(),
        "a rating bound (even 0 = unrated) is a condition"
    );
    f.rating_min = None;
    f.rating_max = Some(5);
    assert!(!f.is_default());

    // 새 필드는 camelCase 로 왕복한다
    let json = serde_json::to_value(&f).unwrap();
    assert_eq!(json["ratingMax"], serde_json::json!(5));
    assert_eq!(json["ratingMin"], serde_json::Value::Null);
    assert_eq!(serde_json::from_value::<SearchFilters>(json).unwrap(), f);
}

#[test]
fn t_rating_09_sort_field_rating_serializes_as_rating() {
    // Arrange
    let spec = SortSpec {
        field: SortField::Rating,
        direction: SortDirection::Desc,
    };

    // Act
    let json = serde_json::to_value(spec).unwrap();

    // Assert
    assert_eq!(
        json,
        serde_json::json!({"field": "rating", "direction": "desc"})
    );
    assert_eq!(serde_json::from_value::<SortSpec>(json).unwrap(), spec);
}
