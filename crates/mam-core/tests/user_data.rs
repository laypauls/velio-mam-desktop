//! P2-7 사용자 데이터 (T-user-*) — 실제 SQLite + 스캔으로 만든 자산.
//!
//! 분류: Happy (01, 04) / Validation (02, 05, 11, 12) / Business rule (03, 06, 09) /
//! Boundary (07) / Concurrency·멱등 (08) / Performance (10: 쿼리 계획)

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::env::Env;
use mam_core::scan::{IngestError, scan_folder};
use mam_core::userdata::{UpdateTagsInput, UserData};

struct Fixture {
    env: Env,
    ud: UserData,
    ids: Vec<i64>,
}

/// 파일 `n` 개를 스캔해 자산 `n` 개를 만든다 (asset_id 오름차순).
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

/// 마지막 outbox 이벤트 (revision, priority)
fn last_event(f: &Fixture, id: i64) -> (i64, i64) {
    f.env.read(|c| {
        c.query_row(
            "SELECT revision, priority FROM search_outbox WHERE asset_id = ?1 ORDER BY event_id DESC LIMIT 1",
            [id],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .unwrap()
    })
}

fn add_tags(f: &Fixture, ids: &[i64], names: &[&str]) -> Result<(), IngestError> {
    f.ud.update_asset_tags(UpdateTagsInput {
        asset_ids: ids.to_vec(),
        add: names.iter().map(|s| s.to_string()).collect(),
        remove: vec![],
    })
}

#[test]
fn t_user_01_adds_creates_and_removes_tags_with_revision_and_outbox() {
    // Arrange
    let f = fixture(3);
    let (a, b, c) = (f.ids[0], f.ids[1], f.ids[2]);
    let (rev_a, rev_c, events) = (revision(&f, a), revision(&f, c), outbox_count(&f));

    // Act: 새 태그 두 개를 a,b 에 붙인다
    add_tags(&f, &[a, b], &["여행", "Trip"]).unwrap();

    // Assert: 태그 생성, 관계 4건, a,b 만 revision+1 / outbox 우선순위 0, c 는 그대로
    let tags = f.ud.list_tags().unwrap();
    assert_eq!(
        tags.iter()
            .map(|t| (t.name.as_str(), t.asset_count))
            .collect::<Vec<_>>(),
        vec![("Trip", 2), ("여행", 2)]
    );
    assert_eq!(f.env.count("SELECT count(*) FROM asset_tags"), 4);
    assert_eq!(revision(&f, a), rev_a + 1);
    assert_eq!(revision(&f, c), rev_c);
    assert_eq!(outbox_count(&f), events + 2);
    assert_eq!(last_event(&f, a), (rev_a + 1, 0));

    // Act: a 에서 "여행" 제거
    let travel = tags.iter().find(|t| t.name == "여행").unwrap().tag_id;
    f.ud.update_asset_tags(UpdateTagsInput {
        asset_ids: vec![a],
        add: vec![],
        remove: vec![travel],
    })
    .unwrap();

    // Assert
    assert_eq!(revision(&f, a), rev_a + 2);
    assert_eq!(last_event(&f, a), (rev_a + 2, 0));
    let travel_count =
        f.ud.list_tags()
            .unwrap()
            .into_iter()
            .find(|t| t.tag_id == travel)
            .unwrap()
            .asset_count;
    assert_eq!(travel_count, 1);
}

#[test]
fn t_user_02_tag_names_compare_case_insensitive_and_nfc_normalized() {
    // Arrange
    let f = fixture(2);

    // Act: 같은 이름의 변형 (대소문자, NFD 합성 전/후, 앞뒤 공백)
    add_tags(&f, &[f.ids[0]], &["Café"]).unwrap();
    add_tags(&f, &[f.ids[1]], &["  CAFE\u{301} "]).unwrap();
    add_tags(&f, &[f.ids[1]], &["café", "CAFÉ"]).unwrap();

    // Assert: 태그 하나, 두 자산에 붙음, 표시 이름은 처음 만든 NFC 형태
    let tags = f.ud.list_tags().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "Café");
    assert_eq!(tags[0].asset_count, 2);
}

#[test]
fn t_user_03_tag_rename_and_delete_bump_every_affected_asset() {
    // Arrange
    let f = fixture(3);
    let (a, b, c) = (f.ids[0], f.ids[1], f.ids[2]);
    add_tags(&f, &[a, b], &["old"]).unwrap();
    add_tags(&f, &[c], &["other"]).unwrap();
    let tags = f.ud.list_tags().unwrap();
    let old = tags.iter().find(|t| t.name == "old").unwrap().tag_id;
    let (ra, rb, rc) = (revision(&f, a), revision(&f, b), revision(&f, c));

    // Act: 이름 변경
    let renamed = f.ud.rename_tag(old, "새 이름").unwrap();

    // Assert: 반환 형태, 태그가 붙은 자산만 revision 증가
    assert_eq!((renamed.name.as_str(), renamed.asset_count), ("새 이름", 2));
    assert_eq!((revision(&f, a), revision(&f, b)), (ra + 1, rb + 1));
    assert_eq!(revision(&f, c), rc);
    assert_eq!(last_event(&f, a), (ra + 1, 0));

    // Act + Assert: 다른 태그와 같은 이름으로는 못 바꾼다 (롤백, revision 불변)
    assert!(matches!(
        f.ud.rename_tag(old, "OTHER"),
        Err(IngestError::AlreadyRegistered)
    ));
    assert_eq!(revision(&f, a), ra + 1);
    // 자기 이름의 대소문자만 바꾸는 것은 허용
    assert_eq!(f.ud.rename_tag(old, "새 이름").unwrap().name, "새 이름");
    assert!(matches!(
        f.ud.rename_tag(9999, "x"),
        Err(IngestError::NotFound)
    ));

    // Act: 삭제
    f.ud.delete_tag(old).unwrap();

    // Assert: 관계 사라짐·자산 보존·revision 한 번 더 증가
    assert_eq!(
        f.env.count(&format!(
            "SELECT count(*) FROM asset_tags WHERE tag_id = {old}"
        )),
        0
    );
    assert_eq!(f.env.count("SELECT count(*) FROM media_assets"), 3);
    assert_eq!((revision(&f, a), revision(&f, b)), (ra + 2, rb + 2));
    assert!(matches!(f.ud.delete_tag(old), Err(IngestError::NotFound)));
}

#[test]
fn t_user_04_memo_and_favorite_set_in_bulk_and_only_real_changes_bump() {
    // Arrange
    let f = fixture(3);
    let (a, b, c) = (f.ids[0], f.ids[1], f.ids[2]);
    let (ra, rb) = (revision(&f, a), revision(&f, b));

    // Act: 메모 설정
    f.ud.set_memo(a, "첫 번째 메모").unwrap();

    // Assert
    let memo: String = f.env.read(|c| {
        c.query_row(
            "SELECT memo FROM asset_user_data WHERE asset_id = ?1",
            [a],
            |r| r.get(0),
        )
        .unwrap()
    });
    assert_eq!(memo, "첫 번째 메모");
    assert_eq!(revision(&f, a), ra + 1);
    assert_eq!(last_event(&f, a), (ra + 1, 0));

    // 같은 값 다시 -> no-op
    let events = outbox_count(&f);
    f.ud.set_memo(a, "첫 번째 메모").unwrap();
    f.ud.set_memo(c, "").unwrap(); // 행이 없는 자산을 빈 메모로 -> no-op
    assert_eq!(outbox_count(&f), events);

    // Act: 즐겨찾기 일괄 설정 (a 는 이미 행이 있고, b 는 없다)
    f.ud.set_favorite(&[a, b, b], true).unwrap();

    // Assert: 두 자산 모두 즐겨찾기, 메모는 유지, revision +1
    assert_eq!(
        f.env
            .count("SELECT count(*) FROM asset_user_data WHERE favorite = 1"),
        2
    );
    assert_eq!((revision(&f, a), revision(&f, b)), (ra + 2, rb + 1));
    assert_eq!(
        f.env.count(&format!(
            "SELECT length(memo) FROM asset_user_data WHERE asset_id = {a}"
        )),
        "첫 번째 메모".chars().count() as i64
    );

    // 이미 즐겨찾기인 자산에 다시 설정, 즐겨찾기 아닌 자산 해제 -> no-op
    let events = outbox_count(&f);
    f.ud.set_favorite(&[a, b], true).unwrap();
    f.ud.set_favorite(&[c], false).unwrap();
    assert_eq!(outbox_count(&f), events);

    // 해제
    f.ud.set_favorite(&[a], false).unwrap();
    assert_eq!(
        f.env
            .count("SELECT count(*) FROM asset_user_data WHERE favorite = 1"),
        1
    );
    assert_eq!(revision(&f, a), ra + 3);
}

#[test]
fn t_user_05_collection_names_are_unique_case_and_nfc_insensitive() {
    // Arrange
    let f = fixture(1);
    let first = f.ud.create_collection("Café 모음").unwrap();
    assert_eq!((first.asset_count, first.name.as_str()), (0, "Café 모음"));

    // Act + Assert: 대소문자·NFD 변형은 중복
    for dup in ["CAFÉ 모음", "cafe\u{301} 모음", "  café 모음  "] {
        assert!(
            matches!(
                f.ud.create_collection(dup),
                Err(IngestError::AlreadyRegistered)
            ),
            "{dup}"
        );
    }
    let second = f.ud.create_collection("다른").unwrap();
    assert!(matches!(
        f.ud.rename_collection(second.collection_id, "CAFÉ 모음"),
        Err(IngestError::AlreadyRegistered)
    ));
    // 자기 자신의 대소문자 변경과 일반 이름 변경은 허용
    assert_eq!(
        f.ud.rename_collection(first.collection_id, "CAFÉ 모음")
            .unwrap()
            .name,
        "CAFÉ 모음"
    );
    assert_eq!(
        f.ud.rename_collection(second.collection_id, "새 이름")
            .unwrap()
            .name,
        "새 이름"
    );
    assert!(matches!(
        f.ud.rename_collection(9999, "x"),
        Err(IngestError::NotFound)
    ));
    assert_eq!(f.ud.list_collections().unwrap().len(), 2);
}

#[test]
fn t_user_06_deleting_a_collection_keeps_assets_and_reindexes_members() {
    // Arrange
    let f = fixture(3);
    let (a, b, c) = (f.ids[0], f.ids[1], f.ids[2]);
    let col = f.ud.create_collection("묶음").unwrap().collection_id;
    f.ud.add_to_collection(col, &[a, b]).unwrap();
    let list = f.ud.list_collections().unwrap();
    assert_eq!(list[0].asset_count, 2);
    let (ra, rc) = (revision(&f, a), revision(&f, c));

    // Act
    f.ud.delete_collection(col).unwrap();

    // Assert: 컬렉션·소속만 사라지고 자산 3건은 그대로, 소속이던 자산만 revision 증가
    assert_eq!(f.env.count("SELECT count(*) FROM collections"), 0);
    assert_eq!(f.env.count("SELECT count(*) FROM collection_assets"), 0);
    assert_eq!(f.env.count("SELECT count(*) FROM media_assets"), 3);
    assert_eq!(revision(&f, a), ra + 1);
    assert_eq!(revision(&f, c), rc);
    assert_eq!(last_event(&f, a), (ra + 1, 0));
    assert!(matches!(
        f.ud.delete_collection(col),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        f.ud.add_to_collection(col, &[a]),
        Err(IngestError::NotFound)
    ));
}

#[test]
fn t_user_06b_collection_membership_add_remove_is_idempotent() {
    // Arrange
    let f = fixture(2);
    let (a, b) = (f.ids[0], f.ids[1]);
    let col = f.ud.create_collection("c").unwrap().collection_id;

    // Act
    f.ud.add_to_collection(col, &[a, b]).unwrap();
    let events = outbox_count(&f);
    f.ud.add_to_collection(col, &[a, b]).unwrap(); // 중복 추가
    let after_dup = outbox_count(&f);
    f.ud.remove_from_collection(col, &[a]).unwrap();
    f.ud.remove_from_collection(col, &[a]).unwrap(); // 이미 없음

    // Assert
    assert_eq!(
        after_dup, events,
        "중복 추가는 revision/outbox 를 만들지 않는다"
    );
    assert_eq!(outbox_count(&f), events + 1);
    assert_eq!(f.ud.list_collections().unwrap()[0].asset_count, 1);
}

#[test]
fn t_user_07_zero_assets_is_a_noop_and_over_limit_or_unknown_is_rejected_atomically() {
    // Arrange
    let f = fixture(2);
    let events = outbox_count(&f);

    // Act + Assert: 0개 -> no-op (태그도 만들지 않는다)
    add_tags(&f, &[], &["x"]).unwrap();
    f.ud.set_favorite(&[], true).unwrap();
    assert_eq!(f.env.count("SELECT count(*) FROM tags"), 0);
    assert_eq!(outbox_count(&f), events);

    // 10,001개 -> invalid_input
    let many: Vec<i64> = (1..=10_001).collect();
    assert!(matches!(
        add_tags(&f, &many, &["x"]),
        Err(IngestError::InvalidInput(_))
    ));
    assert!(matches!(
        f.ud.set_favorite(&many, true),
        Err(IngestError::InvalidInput(_))
    ));
    // 정확히 10,000개는 한도 안 (없는 자산이라 NotFound 로 거부되지만 한도 오류는 아니다)
    assert!(matches!(
        add_tags(&f, &many[..10_000], &["x"]),
        Err(IngestError::NotFound)
    ));

    // 존재하지 않는 자산이 섞이면 전체 롤백 (태그 생성·관계·revision 없음)
    let r = revision(&f, f.ids[0]);
    assert!(matches!(
        add_tags(&f, &[f.ids[0], 424_242], &["x"]),
        Err(IngestError::NotFound)
    ));
    assert_eq!(f.env.count("SELECT count(*) FROM tags"), 0);
    assert_eq!(revision(&f, f.ids[0]), r);
    assert_eq!(outbox_count(&f), events);
}

#[test]
fn t_user_08_adding_the_same_tag_twice_creates_no_duplicates_or_extra_revision() {
    // Arrange
    let f = fixture(1);
    let a = f.ids[0];
    let before = revision(&f, a);

    // Act
    add_tags(&f, &[a], &["dup"]).unwrap();
    add_tags(&f, &[a], &["dup", "DUP"]).unwrap();

    // Assert
    assert_eq!(f.env.count("SELECT count(*) FROM tags"), 1);
    assert_eq!(f.env.count("SELECT count(*) FROM asset_tags"), 1);
    assert_eq!(revision(&f, a), before + 1);
}

#[test]
fn t_user_09_large_tag_rename_uses_low_priority_outbox() {
    // Arrange: 자산 1,001 건에 같은 태그 (직접 삽입 — 스캔보다 빠르다)
    let f = fixture(1);
    f.env
        .pool
        .write(mam_db::Priority::High, |c| {
            let tx = c.transaction()?;
            for _ in 0..1_000 {
                tx.execute(
                    "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                     VALUES ('image', 1, 1, 1, 1)",
                    [],
                )?;
            }
            tx.execute("INSERT INTO tags (name, name_key) VALUES ('big', 'big')", [])?;
            tx.execute(
                "INSERT INTO asset_tags (asset_id, tag_id, created_at)
                 SELECT asset_id, 1, 1 FROM media_assets",
                [],
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    assert_eq!(f.env.count("SELECT count(*) FROM asset_tags"), 1_001);

    // Act
    f.ud.rename_tag(1, "bigger").unwrap();

    // Assert: 1,001 건 모두 새 이벤트, 우선순위 1 (사용자 직접 편집 0 보다 낮다)
    assert_eq!(
        f.env.count(
            "SELECT count(*) FROM search_outbox WHERE priority = 1 AND revision >= 2 AND event_type = 'upsert'
             AND event_id > (SELECT max(event_id) - 1001 FROM search_outbox)"
        ),
        1_001
    );
    // 소량이면 0 (t_user_03 에서 확인) — 직접 선택 편집은 항상 0
    f.ud.set_favorite(&[f.ids[0]], true).unwrap();
    assert_eq!(last_event(&f, f.ids[0]).1, 0);
}

#[test]
fn t_user_10_queries_use_indexes() {
    // Arrange
    let f = fixture(1);
    let plan = |sql: &str| -> String {
        f.env.read(|c| {
            let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            s.query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
                .join(" | ")
        })
    };

    // Act + Assert: 태그 자산 수 / 컬렉션 자산 수 / 갱신 대상 선택이 전체 스캔이 아니다
    let count = plan(
        "SELECT count(*) FROM asset_tags x JOIN media_assets a ON a.asset_id = x.asset_id
         AND a.status = 'active' WHERE x.tag_id = 1",
    );
    assert!(count.contains("idx_asset_tags_tag"), "{count}");
    let bump = plan(
        "UPDATE media_assets SET revision = revision + 1
         WHERE asset_id IN (SELECT value FROM json_each('[1,2]'))",
    );
    assert!(!bump.contains("SCAN media_assets"), "{bump}");
    let members = plan("SELECT asset_id FROM collection_assets WHERE collection_id = 1");
    assert!(members.contains("SEARCH collection_assets"), "{members}");
    let fav = plan("SELECT asset_id FROM asset_user_data WHERE favorite = 1");
    assert!(fav.contains("idx_asset_user_data_favorite"), "{fav}");
}

#[test]
fn t_user_11_invalid_names_and_memo_are_rejected() {
    // Arrange
    let f = fixture(1);
    let a = f.ids[0];
    let long = "가".repeat(101);

    // Act + Assert
    for bad in ["", "   ", "a\nb", "a\u{1f}b", long.as_str()] {
        assert!(
            matches!(
                add_tags(&f, &[a], &[bad]),
                Err(IngestError::InvalidInput(_))
            ),
            "{bad:?}"
        );
        assert!(
            matches!(
                f.ud.create_collection(bad),
                Err(IngestError::InvalidInput(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        f.ud.rename_tag(1, " "),
        Err(IngestError::InvalidInput(_))
    ));
    assert!(matches!(
        f.ud.set_memo(a, &"x".repeat(10_001)),
        Err(IngestError::InvalidInput(_))
    ));
    assert!(matches!(
        f.ud.set_memo(a, "a\0b"),
        Err(IngestError::InvalidInput(_))
    ));
    assert!(matches!(
        f.ud.set_memo(999_999, "x"),
        Err(IngestError::NotFound)
    ));
    // 10,000 자는 허용
    f.ud.set_memo(a, &"x".repeat(10_000)).unwrap();
    assert_eq!(f.env.count("SELECT count(*) FROM tags"), 0);
}

#[test]
fn t_user_12_contract_shapes_deserialize_and_serialize_in_camel_case() {
    // Arrange: 프론트가 보내는 JSON
    let input: UpdateTagsInput = serde_json::from_value(serde_json::json!({
        "assetIds": [1, 2], "add": ["a"], "remove": [3]
    }))
    .unwrap();
    let f = fixture(1);
    let tag = {
        add_tags(&f, &[f.ids[0]], &["t"]).unwrap();
        f.ud.list_tags().unwrap().remove(0)
    };
    let col = f.ud.create_collection("c").unwrap();

    // Act
    let tag_json = serde_json::to_value(&tag).unwrap();
    let col_json = serde_json::to_value(&col).unwrap();

    // Assert
    assert_eq!(
        (input.asset_ids, input.add, input.remove),
        (vec![1, 2], vec!["a".to_string()], vec![3])
    );
    assert_eq!(
        tag_json,
        serde_json::json!({"tagId": tag.tag_id, "name": "t", "assetCount": 1})
    );
    assert_eq!(
        col_json,
        serde_json::json!({
            "collectionId": col.collection_id, "name": "c", "assetCount": 0, "createdAt": col.created_at
        })
    );
}
