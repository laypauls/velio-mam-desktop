//! 스키마 업무 규칙 테스트 (T-db-09, T-db-13, T-db-14, T-db-15).
//!
//! 분류: Business rule (전부) / Boundary (id 재사용, detach 후 재삽입).
//! 실제 SQLite 임시 파일에 마이그레이션을 적용해 사용한다.

mod common;

use common::{T0, count, insert_asset, insert_location, insert_volume, migrated_db};

fn err_text<T: std::fmt::Debug>(res: rusqlite::Result<T>) -> String {
    res.expect_err("statement must fail").to_string()
}

fn detach(conn: &rusqlite::Connection, asset_id: i64) {
    conn.execute(
        "UPDATE file_locations SET detached_at = ?2 WHERE asset_id = ?1 AND detached_at IS NULL",
        (asset_id, T0),
    )
    .unwrap();
}

// --- T-db-09 활성 경로 유일 ---------------------------------------------

#[test]
fn t_db_09_same_active_path_twice_fails_until_detached() {
    // Arrange
    let db = migrated_db();
    let volume = insert_volume(&db.conn, "v");
    let old = insert_asset(&db.conn);
    let new = insert_asset(&db.conn);
    insert_location(&db.conn, old, volume, "photos/a.jpg").unwrap();

    // Act
    let conflict = insert_location(&db.conn, new, volume, "photos/a.jpg");
    detach(&db.conn, old);
    let after_detach = insert_location(&db.conn, new, volume, "photos/a.jpg");

    // Assert
    assert!(err_text(conflict).contains("UNIQUE constraint failed"));
    assert_eq!(after_detach.unwrap(), 1);
    assert_eq!(
        count(
            &db.conn,
            "SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL"
        ),
        1
    );
}

#[test]
fn t_db_09_same_path_on_another_volume_is_allowed() {
    // Arrange
    let db = migrated_db();
    let v1 = insert_volume(&db.conn, "v1");
    let v2 = insert_volume(&db.conn, "v2");
    let a = insert_asset(&db.conn);
    let b = insert_asset(&db.conn);

    // Act
    let first = insert_location(&db.conn, a, v1, "x.jpg");
    let second = insert_location(&db.conn, b, v2, "x.jpg");

    // Assert
    assert!(first.is_ok() && second.is_ok());
}

// --- T-db-13 자산당 활성 위치 1개 (D1) -----------------------------------

#[test]
fn t_db_13_asset_can_have_only_one_active_location() {
    // Arrange
    let db = migrated_db();
    let volume = insert_volume(&db.conn, "v");
    let asset = insert_asset(&db.conn);
    insert_location(&db.conn, asset, volume, "a.jpg").unwrap();

    // Act
    let second_active = insert_location(&db.conn, asset, volume, "b.jpg");
    detach(&db.conn, asset);
    let after_detach = insert_location(&db.conn, asset, volume, "b.jpg");

    // Assert
    assert!(err_text(second_active).contains("UNIQUE constraint failed"));
    assert_eq!(after_detach.unwrap(), 1);
    assert_eq!(
        count(
            &db.conn,
            "SELECT COUNT(*) FROM file_locations WHERE asset_id = 1"
        ),
        2,
        "detached history row is kept"
    );
}

// --- T-db-14 asset_id 재사용 금지 ----------------------------------------

#[test]
fn t_db_14_deleted_max_asset_id_is_never_reissued() {
    // Arrange
    let db = migrated_db();
    let _first = insert_asset(&db.conn);
    let max = insert_asset(&db.conn);
    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [max])
        .unwrap();

    // Act
    let next = insert_asset(&db.conn);

    // Assert
    assert!(next > max, "asset_id {max} was reissued as {next}");
}

#[test]
fn t_db_14_empty_table_after_delete_all_still_advances() {
    // Arrange: 모든 행을 지운 뒤에도 카운터가 이어진다
    let db = migrated_db();
    let only = insert_asset(&db.conn);
    db.conn.execute("DELETE FROM media_assets", []).unwrap();

    // Act
    let next = insert_asset(&db.conn);

    // Assert
    assert!(next > only);
}

// --- T-db-15 삭제 정책 ---------------------------------------------------

#[test]
fn t_db_15_asset_delete_cascades_children_and_keeps_outbox() {
    // Arrange: 메타데이터·태그·Outbox·index_progress 가 붙은 자산
    let db = migrated_db();
    let asset = insert_asset(&db.conn);
    db.conn
        .execute(
            "INSERT INTO media_metadata (asset_id, width) VALUES (?1, 10)",
            [asset],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO tags (name, name_key) VALUES ('Trip', 'trip')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, 1, ?2)",
            (asset, T0),
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO search_outbox (asset_id, revision, event_type, created_at)
             VALUES (?1, 1, 'delete', ?2)",
            (asset, T0),
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO index_progress (index_name, asset_id, applied_revision) VALUES ('tantivy', ?1, 1)",
            [asset],
        )
        .unwrap();

    // Act
    let deleted = db
        .conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [asset])
        .unwrap();

    // Assert
    assert_eq!(deleted, 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_metadata"), 0);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_tags"), 0);
    assert_eq!(
        count(&db.conn, "SELECT COUNT(*) FROM tags"),
        1,
        "tag itself stays"
    );
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM search_outbox"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM index_progress"), 1);
}

#[test]
fn t_db_15_asset_with_location_or_relation_cannot_be_deleted() {
    // Arrange
    let db = migrated_db();
    let volume = insert_volume(&db.conn, "v");
    let located = insert_asset(&db.conn);
    insert_location(&db.conn, located, volume, "a.jpg").unwrap();
    let old = insert_asset(&db.conn);
    let new = insert_asset(&db.conn);
    db.conn
        .execute(
            "INSERT INTO asset_relations (from_asset_id, to_asset_id, relation, created_at)
             VALUES (?1, ?2, 'content_changed', ?3)",
            (old, new, T0),
        )
        .unwrap();

    // Act
    let with_location = db
        .conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [located]);
    let relation_from = db
        .conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [old]);
    let relation_to = db
        .conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [new]);
    let volume_in_use = db
        .conn
        .execute("DELETE FROM volumes WHERE volume_id = ?1", [volume]);

    // Assert
    for res in [with_location, relation_from, relation_to, volume_in_use] {
        assert!(err_text(res).contains("FOREIGN KEY"));
    }
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 3);
}

#[test]
fn t_db_15_tag_delete_cascades_asset_tags() {
    // Arrange
    let db = migrated_db();
    let asset = insert_asset(&db.conn);
    db.conn
        .execute(
            "INSERT INTO tags (name, name_key) VALUES ('Trip', 'trip')",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, 1, ?2)",
            (asset, T0),
        )
        .unwrap();

    // Act
    db.conn
        .execute("DELETE FROM tags WHERE tag_id = 1", [])
        .unwrap();

    // Assert
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_tags"), 0);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 1);
}

#[test]
fn t_db_15_outbox_accepts_events_for_nonexistent_assets() {
    // Arrange / Act: 자산 행이 이미 없는 id 에 대한 delete 이벤트
    let db = migrated_db();
    let res = db.conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, created_at)
         VALUES (424242, 3, 'delete', ?1)",
        [T0],
    );

    // Assert
    assert_eq!(res.unwrap(), 1);
}
