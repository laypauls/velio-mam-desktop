//! 마이그레이션 중 외래 키 처리와 목록 검증 테스트 (T-db-03, T-db-04, T-db-07 확장).
//!
//! 분류: Business rule (테이블 재생성이 자식 행을 지우지 않음, 중복 버전 거부) /
//! External failure (FK 위반·적용 실패 후에도 foreign_keys 복원).
//!
//! 러너는 적용 구간에서만 foreign_keys = OFF 로 두고 끝나면 항상 ON 으로 복원한다.
//! 마이그레이션 SQL 에는 PRAGMA 를 쓰지 않는다.

mod common;

use common::{
    FixedClock, T0, applied_versions, count, dir_entries, empty_db, insert_asset, insert_location,
    insert_volume, user_tables, v1, v1_db,
};
use mam_db::{DbError, Migration, migrate_with};
use rusqlite::Connection;

/// 12단계 방식 테이블 재생성: 새 테이블 생성 -> 복사 -> DROP -> RENAME (컬럼 `note` 추가).
const REBUILD_MEDIA_ASSETS: &str = "
CREATE TABLE media_assets_new (
    asset_id     INTEGER PRIMARY KEY AUTOINCREMENT,
    media_type   TEXT NOT NULL,
    status       TEXT NOT NULL,
    content_hash BLOB,
    hash_algo    TEXT,
    hash_state   TEXT NOT NULL,
    file_size    INTEGER NOT NULL,
    modified_at  INTEGER NOT NULL,
    revision     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    note         TEXT
) STRICT;
INSERT INTO media_assets_new
    (asset_id, media_type, status, content_hash, hash_algo, hash_state, file_size,
     modified_at, revision, created_at, updated_at)
SELECT asset_id, media_type, status, content_hash, hash_algo, hash_state, file_size,
     modified_at, revision, created_at, updated_at
FROM media_assets;
DROP TABLE media_assets;
ALTER TABLE media_assets_new RENAME TO media_assets;
";

fn fk_pragma(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn t_db_07_table_rebuild_migration_keeps_cascade_children() {
    // Arrange: 메타데이터·태그·활성 위치가 붙은 자산이 있는 DB
    let mut db = v1_db();
    let volume = insert_volume(&db.conn, "v");
    let asset = insert_asset(&db.conn);
    insert_location(&db.conn, asset, volume, "a.jpg").unwrap();
    db.conn
        .execute(
            "INSERT INTO media_metadata (asset_id, width) VALUES (?1, 10)",
            [asset],
        )
        .unwrap();
    db.conn
        .execute("INSERT INTO tags (name, name_key) VALUES ('t', 't')", [])
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, 1, ?2)",
            (asset, T0),
        )
        .unwrap();
    let v2 = Migration::unapplied("V2__rebuild_assets", REBUILD_MEDIA_ASSETS).unwrap();

    // Act
    let report = migrate_with(&mut db.conn, &[v1(), v2], &db.backup_dir, &FixedClock(T0)).unwrap();

    // Assert: 자식 행이 CASCADE 로 지워지지 않았고 FK 가 다시 켜졌다
    assert_eq!(report.applied, vec![2]);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_metadata"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_tags"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM file_locations"), 1);
    assert_eq!(
        count(&db.conn, "SELECT COUNT(*) FROM pragma_foreign_key_check"),
        0
    );
    assert_eq!(fk_pragma(&db.conn), 1);
}

#[test]
fn t_db_07_fk_violation_after_migration_is_an_error_with_backup_path() {
    // Arrange: V2 가 존재하지 않는 부모를 가리키는 행을 만든다 (FK OFF 구간이라 INSERT 는 통과)
    let mut db = v1_db();
    let bad = Migration::unapplied(
        "V2__orphan",
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
         VALUES (9999, 9999, 'x', 'x', 0);",
    )
    .unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[v1(), bad], &db.backup_dir, &FixedClock(T0));

    // Assert
    let Err(DbError::ForeignKeyViolation {
        violations,
        backup: Some(backup),
    }) = err
    else {
        panic!("expected ForeignKeyViolation with backup, got {err:?}");
    };
    assert!(violations >= 1);
    assert!(backup.is_file());
    assert_eq!(fk_pragma(&db.conn), 1);
    // 알려진 약점: 검사는 커밋 뒤라 V2 는 이미 적용돼 있다 -> 복구는 백업으로 한다
    assert_eq!(applied_versions(&db.conn), vec![1, 2]);
}

#[test]
fn t_db_03_foreign_keys_are_restored_after_failed_apply() {
    // Arrange
    let mut db = v1_db();
    let bad = Migration::unapplied("V2__bad", "INSERT INTO no_such_table VALUES (1);").unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[v1(), bad], &db.backup_dir, &FixedClock(T0));

    // Assert
    assert!(matches!(err, Err(DbError::ApplyFailed { .. })), "{err:?}");
    assert_eq!(fk_pragma(&db.conn), 1);
}

#[test]
fn t_db_04_duplicate_version_in_app_list_is_rejected_on_existing_db() {
    // Arrange
    let mut db = v1_db();
    let a = Migration::unapplied("V2__a", "CREATE TABLE a (x INTEGER) STRICT;").unwrap();
    let b = Migration::unapplied("V2__b", "CREATE TABLE b (x INTEGER) STRICT;").unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[v1(), a, b], &db.backup_dir, &FixedClock(T0));

    // Assert: 백업·적용 모두 없음
    assert!(
        matches!(err, Err(DbError::DuplicateMigration { version: 2 })),
        "{err:?}"
    );
    assert!(dir_entries(&db.backup_dir).is_empty());
    assert_eq!(applied_versions(&db.conn), vec![1]);
}

#[test]
fn t_db_04_duplicate_version_in_app_list_is_rejected_on_fresh_db() {
    // Arrange
    let mut db = empty_db();
    let a = Migration::unapplied("V1__a", "CREATE TABLE a (x INTEGER) STRICT;").unwrap();
    let b = Migration::unapplied("V1__b", "CREATE TABLE b (x INTEGER) STRICT;").unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[a, b], &db.backup_dir, &FixedClock(T0));

    // Assert
    assert!(
        matches!(err, Err(DbError::DuplicateMigration { version: 1 })),
        "{err:?}"
    );
    assert!(dir_entries(&db.backup_dir).is_empty());
    assert!(user_tables(&db.conn).is_empty());
}
