//! 스키마 제약 테스트 (T-db-07, T-db-08, T-db-11).
//!
//! 분류: Validation (전부). 정상 값 한 건씩을 대조군으로 두어 "제약이 너무 넓게 막지 않음"도 확인한다.
//! 실제 SQLite 임시 파일에 마이그레이션을 적용해 사용한다.

mod common;

use common::{T0, count, insert_asset, insert_volume, migrated_db};
use rusqlite::Connection;

fn err_text<T: std::fmt::Debug>(res: rusqlite::Result<T>) -> String {
    res.expect_err("statement must fail").to_string()
}

fn insert_asset_with(
    conn: &Connection,
    media_type: &str,
    state: &str,
    hash: Option<&[u8]>,
    algo: Option<&str>,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO media_assets
           (media_type, hash_state, content_hash, hash_algo, file_size, modified_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5, ?5)",
        (media_type, state, hash, algo, T0),
    )
}

// --- T-db-07 FK ---------------------------------------------------------

#[test]
fn t_db_07_location_with_unknown_asset_or_volume_is_rejected() {
    // Arrange
    let db = migrated_db();
    let volume = insert_volume(&db.conn, "v");
    let asset = insert_asset(&db.conn);

    // Act
    let bad_asset = common::insert_location(&db.conn, 9_999, volume, "a");
    let bad_volume = common::insert_location(&db.conn, asset, 9_999, "a");
    let good = common::insert_location(&db.conn, asset, volume, "a");

    // Assert
    assert!(err_text(bad_asset).contains("FOREIGN KEY"));
    assert!(err_text(bad_volume).contains("FOREIGN KEY"));
    assert_eq!(good.unwrap(), 1);
}

#[test]
fn t_db_07_child_tables_reject_unknown_parents() {
    // Arrange
    let db = migrated_db();
    let asset = insert_asset(&db.conn);

    // Act
    let metadata = db
        .conn
        .execute("INSERT INTO media_metadata (asset_id) VALUES (9999)", []);
    let asset_tag = db.conn.execute(
        "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, 9999, ?2)",
        (asset, T0),
    );
    let relation = db.conn.execute(
        "INSERT INTO asset_relations (from_asset_id, to_asset_id, relation, created_at)
         VALUES (?1, 9999, 'content_changed', ?2)",
        (asset, T0),
    );

    // Assert
    assert!(err_text(metadata).contains("FOREIGN KEY"));
    assert!(err_text(asset_tag).contains("FOREIGN KEY"));
    assert!(err_text(relation).contains("FOREIGN KEY"));
}

// --- T-db-08 CHECK / STRICT --------------------------------------------

#[test]
fn t_db_08_invalid_enum_values_are_rejected() {
    // Arrange
    let db = migrated_db();
    let volume = insert_volume(&db.conn, "v");
    let asset = insert_asset(&db.conn);

    // Act
    let media_type = insert_asset_with(&db.conn, "document", "pending", None, None);
    let hash_state = insert_asset_with(&db.conn, "image", "queued", None, None);
    let status = db.conn.execute(
        "UPDATE media_assets SET status = 'trashed' WHERE asset_id = ?1",
        [asset],
    );
    let presence = db.conn.execute(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, presence, last_seen_at)
         VALUES (?1, ?2, 'a', 'a', 'offline', ?3)",
        (asset, volume, T0),
    );
    let kind = db.conn.execute(
        "INSERT INTO volumes (kind, volume_key, last_seen_at) VALUES ('cloud', 'k', ?1)",
        [T0],
    );
    let availability = db.conn.execute(
        "UPDATE volumes SET availability = 'missing' WHERE volume_id = ?1",
        [volume],
    );
    let event = db.conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, created_at) VALUES (1, 1, 'purge', ?1)",
        [T0],
    );

    // Assert
    for res in [
        media_type.map(|_| ()),
        hash_state.map(|_| ()),
        status.map(|_| ()),
        presence.map(|_| ()),
        kind.map(|_| ()),
        availability.map(|_| ()),
        event.map(|_| ()),
    ] {
        assert!(err_text(res).contains("CHECK constraint failed"));
    }
    assert!(insert_asset_with(&db.conn, "audio", "pending", None, None).is_ok());
}

#[test]
fn t_db_08_strict_rejects_wrong_storage_class() {
    // Arrange
    let db = migrated_db();

    // Act: INTEGER 컬럼에 변환 불가능한 TEXT
    let res = db.conn.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES ('image', 'big', 1, 1, 1)",
        [],
    );

    // Assert
    assert!(err_text(res).contains("cannot store TEXT value in INTEGER column"));
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 0);
}

#[test]
fn t_db_08_boundary_values_are_rejected() {
    // Arrange
    let db = migrated_db();
    let volume = insert_volume(&db.conn, "v");
    let asset = insert_asset(&db.conn);
    let other = insert_asset(&db.conn);

    // Act
    let negative_size = db.conn.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES ('image', -1, 1, 1, 1)",
        [],
    );
    let zero_revision = db.conn.execute(
        "UPDATE media_assets SET revision = 0 WHERE asset_id = ?1",
        [asset],
    );
    let empty_key = common::insert_location(&db.conn, asset, volume, "");
    let self_relation = db.conn.execute(
        "INSERT INTO asset_relations (from_asset_id, to_asset_id, relation, created_at)
         VALUES (?1, ?1, 'content_changed', ?2)",
        (asset, T0),
    );
    let bad_relation = db.conn.execute(
        "INSERT INTO asset_relations (from_asset_id, to_asset_id, relation, created_at)
         VALUES (?1, ?2, 'copied', ?3)",
        (asset, other, T0),
    );
    let good_relation = db.conn.execute(
        "INSERT INTO asset_relations (from_asset_id, to_asset_id, relation, created_at)
         VALUES (?1, ?2, 'content_changed', ?3)",
        (asset, other, T0),
    );

    // Assert
    for res in [
        negative_size,
        zero_revision,
        empty_key,
        self_relation,
        bad_relation,
    ] {
        assert!(err_text(res).contains("CHECK constraint failed"));
    }
    assert_eq!(good_relation.unwrap(), 1);
}

// --- T-db-11 해시 상태 일관성 -------------------------------------------

#[test]
fn t_db_11_hash_state_and_hash_must_agree() {
    // Arrange
    let db = migrated_db();
    let hash = [7u8; 32];

    // Act
    let done_without_hash = insert_asset_with(&db.conn, "image", "done", None, None);
    let pending_with_hash =
        insert_asset_with(&db.conn, "image", "pending", Some(&hash), Some("blake3"));
    let failed_with_hash =
        insert_asset_with(&db.conn, "image", "failed", Some(&hash), Some("blake3"));
    let hash_without_algo = insert_asset_with(&db.conn, "image", "done", Some(&hash), None);
    let algo_without_hash = insert_asset_with(&db.conn, "image", "pending", None, Some("blake3"));
    let unsupported_algo =
        insert_asset_with(&db.conn, "image", "done", Some(&hash), Some("sha256"));

    // Assert
    for res in [
        done_without_hash,
        pending_with_hash,
        failed_with_hash,
        hash_without_algo,
        algo_without_hash,
        unsupported_algo,
    ] {
        assert!(err_text(res).contains("CHECK constraint failed"));
    }
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 0);
}

#[test]
fn t_db_11_valid_hash_combinations_are_accepted() {
    // Arrange
    let db = migrated_db();
    let hash = [7u8; 32];

    // Act
    let done = insert_asset_with(&db.conn, "video", "done", Some(&hash), Some("blake3"));
    let pending = insert_asset_with(&db.conn, "video", "pending", None, None);
    let failed = insert_asset_with(&db.conn, "video", "failed", None, None);

    // Assert
    assert!(done.is_ok() && pending.is_ok() && failed.is_ok());
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 3);
}

#[test]
fn t_db_11_hash_state_can_transition_pending_to_done_in_one_update() {
    // Arrange
    let db = migrated_db();
    let asset = insert_asset(&db.conn);

    // Act: 상태·해시·알고리즘은 한 UPDATE 로 같이 바꿔야 한다
    let partial = db.conn.execute(
        "UPDATE media_assets SET hash_state = 'done' WHERE asset_id = ?1",
        [asset],
    );
    let full = db.conn.execute(
        "UPDATE media_assets SET hash_state = 'done', content_hash = zeroblob(32), hash_algo = 'blake3'
         WHERE asset_id = ?1",
        [asset],
    );

    // Assert
    assert!(err_text(partial).contains("CHECK constraint failed"));
    assert_eq!(full.unwrap(), 1);
}
