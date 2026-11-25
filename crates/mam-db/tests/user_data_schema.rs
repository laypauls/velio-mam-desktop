//! V4 (사용자 데이터) 스키마 계약 (T-db-30 ~ 31).
//!
//! 분류: Validation (30 CHECK·UNIQUE) / Business rule (30 CASCADE 방향) / Business rule (31 추가형: 기존 데이터 보존)

mod common;

use common::{applied_versions, count, insert_asset, migrated_db};

#[test]
fn t_db_30_user_data_constraints_and_cascades() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;
    let a = insert_asset(c);
    let b = insert_asset(c);
    c.execute(
        "INSERT INTO asset_user_data (asset_id, memo, favorite, updated_at) VALUES (?1, 'm', 1, 1)",
        [a],
    )
    .unwrap();
    c.execute(
        "INSERT INTO collections (collection_id, name, name_key, created_at) VALUES (1, 'Café', 'café', 1)",
        [],
    )
    .unwrap();
    c.execute_batch(&format!(
        "INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES (1, {a}, 1), (1, {b}, 1);"
    ))
    .unwrap();

    // Act + Assert (Validation): 즐겨찾기는 0/1, 이름 키·소속 쌍은 유일, 없는 자산·컬렉션은 거부
    assert!(
        c.execute(
            "INSERT INTO asset_user_data (asset_id, favorite, updated_at) VALUES (?1, 2, 1)",
            [b]
        )
        .is_err()
    );
    assert!(
        c.execute(
            "INSERT INTO collections (name, name_key, created_at) VALUES ('CAFÉ', 'café', 1)",
            []
        )
        .is_err()
    );
    assert!(
        c.execute(
            "INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES (1, ?1, 2)",
            [a]
        )
        .is_err()
    );
    assert!(
        c.execute(
            "INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES (99, ?1, 2)",
            [a]
        )
        .is_err()
    );

    // Act: 컬렉션을 지우면 소속만 사라진다 (자산·메모는 유지)
    c.execute("DELETE FROM collections WHERE collection_id = 1", [])
        .unwrap();

    // Assert
    assert_eq!(count(c, "SELECT count(*) FROM collection_assets"), 0);
    assert_eq!(count(c, "SELECT count(*) FROM media_assets"), 2);
    assert_eq!(count(c, "SELECT count(*) FROM asset_user_data"), 1);

    // Act + Assert: 자산 행을 지우면 사용자 데이터도 함께 정리된다 (FK 자식 → 자산 방향 CASCADE)
    c.execute("DELETE FROM media_assets WHERE asset_id = ?1", [a])
        .unwrap();
    assert_eq!(count(c, "SELECT count(*) FROM asset_user_data"), 0);
}

#[test]
fn t_db_31_v4_is_applied_with_the_shipped_migrations_and_is_additive() {
    // Arrange + Act
    let db = migrated_db();

    // Assert: V4 가 이력에 있고 V1 의 태그 테이블은 그대로 쓸 수 있다 (컬럼 변경 없음)
    assert!(applied_versions(&db.conn).contains(&4));
    db.conn
        .execute("INSERT INTO tags (name, name_key) VALUES ('t', 't')", [])
        .unwrap();
    let cols: Vec<String> = db
        .conn
        .prepare("SELECT name FROM pragma_table_info('tags') ORDER BY cid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(cols, ["tag_id", "name", "name_key"]);
}
