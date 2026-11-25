//! V19 사용자 정의 메타데이터 필드 테이블 시험 (T-umeta-mig-*) — V18 DB 에 추가형으로 적용되고 제약이 걸린다.
//!
//! 분류: Happy (01 V18 에 적용, 06 CASCADE) / Boundary (01 기존 행 보존) /
//! Validation (02 타입 CHECK, 03 값 컬럼 정확히 1개, 04 STRICT 타입 불일치, 05 중복 키) /
//! Business rule (06 CASCADE, 07 인덱스 사용) / Concurrency·멱등 (08 재실행 no-op)

mod common;

use common::{FixedClock, T0, TestDb, applied_versions, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v18_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto18: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 18)
        .collect();
    migrate_with(&mut conn, &upto18, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn migrated_v19() -> TestDb {
    let mut db = v18_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    db
}

fn add_asset(conn: &Connection, id: i64) {
    conn.execute(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
         VALUES (?1, 'image', 10, 0, 0, 0)",
        [id],
    )
    .unwrap();
}

fn add_field(conn: &Connection, key: &str, vt: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO metadata_fields (name, name_key, value_type, position, created_at, updated_at)
         VALUES (?1, ?1, ?2, 0, 0, 0)",
        [key, vt],
    )
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn t_umeta_mig_01_v19_applies_to_a_v18_db_keeping_existing_rows() {
    // Arrange: 사용자 필드를 모르는 V18 앱의 자산 행
    let mut db = v18_db();
    add_asset(&db.conn, 1);
    assert_eq!(applied_versions(&db.conn).last(), Some(&18));

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![19, 20, 21, 22, 23, 24, 25, 26]);
    assert!(report.backup.is_some_and(|b| b.is_file()));
    assert_eq!(count(&db.conn, "media_assets"), 1);
    for t in ["metadata_fields", "metadata_field_options", "asset_metadata_values"] {
        assert_eq!(count(&db.conn, t), 0, "{t}");
    }
}

#[test]
fn t_umeta_mig_02_value_type_check_rejects_unknown_types() {
    let db = migrated_v19();

    for ok in ["keyword", "number", "date", "boolean", "enum", "multi_enum"] {
        add_field(&db.conn, ok, ok).unwrap();
    }
    for bad in ["text", "integer", "real", ""] {
        assert!(add_field(&db.conn, &format!("x{bad}"), bad).is_err(), "{bad}");
    }
    assert_eq!(count(&db.conn, "metadata_fields"), 6);
}

#[test]
fn t_umeta_mig_03_exactly_one_value_column_must_be_set() {
    let db = migrated_v19();
    add_asset(&db.conn, 1);
    add_field(&db.conn, "f", "keyword").unwrap();
    let insert = |text: &str, int: &str, real: &str, ord: i64| {
        db.conn.execute(
            &format!(
                "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_text, value_int, value_real, updated_at)
                 VALUES (1, 1, {ord}, {text}, {int}, {real}, 0)"
            ),
            [],
        )
    };

    assert!(insert("NULL", "NULL", "NULL", 0).is_err(), "0개");
    assert!(insert("'a'", "1", "NULL", 0).is_err(), "text+int");
    assert!(insert("NULL", "1", "1.5", 0).is_err(), "int+real");
    assert!(insert("'a'", "1", "1.5", 0).is_err(), "3개");
    assert!(insert("'a'", "NULL", "NULL", 0).is_ok());
    assert!(insert("NULL", "1", "NULL", 1).is_ok());
    assert!(insert("NULL", "NULL", "1.5", 2).is_ok());
    assert!(insert("NULL", "NULL", "9e999", 3).is_err(), "무한대");
    assert!(insert("'a'", "NULL", "NULL", -1).is_err(), "ord < 0");
    assert_eq!(count(&db.conn, "asset_metadata_values"), 3);
}

#[test]
fn t_umeta_mig_04_strict_rejects_wrong_storage_class() {
    let db = migrated_v19();
    add_asset(&db.conn, 1);
    add_field(&db.conn, "f", "keyword").unwrap();

    let r = db.conn.execute(
        "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_int, updated_at)
         VALUES (1, 1, 0, 'not a number', 0)",
        [],
    );

    assert!(r.is_err());
}

#[test]
fn t_umeta_mig_05_duplicate_keys_are_rejected() {
    let db = migrated_v19();
    add_asset(&db.conn, 1);
    add_field(&db.conn, "f", "multi_enum").unwrap();
    assert!(add_field(&db.conn, "f", "keyword").is_err(), "name_key UNIQUE");
    let opt = |label: &str| {
        db.conn.execute(
            "INSERT INTO metadata_field_options (field_id, label, label_key, position) VALUES (1, ?1, ?1, 0)",
            [label],
        )
    };
    opt("a").unwrap();
    assert!(opt("a").is_err(), "(field_id, label_key) UNIQUE");
    let val = || {
        db.conn.execute(
            "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_int, updated_at)
             VALUES (1, 1, 1, 1, 0)",
            [],
        )
    };
    val().unwrap();
    assert!(val().is_err(), "PK (asset_id, field_id, ord)");
    // FK: 없는 자산·필드
    assert!(
        db.conn
            .execute(
                "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_int, updated_at)
                 VALUES (99, 1, 0, 1, 0)",
                []
            )
            .is_err()
    );
    assert!(
        db.conn
            .execute(
                "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_int, updated_at)
                 VALUES (1, 99, 0, 1, 0)",
                []
            )
            .is_err()
    );
}

#[test]
fn t_umeta_mig_06_cascades_from_field_and_asset() {
    // Arrange
    let db = migrated_v19();
    add_asset(&db.conn, 1);
    add_asset(&db.conn, 2);
    add_field(&db.conn, "a", "enum").unwrap();
    add_field(&db.conn, "b", "keyword").unwrap();
    db.conn
        .execute(
            "INSERT INTO metadata_field_options (field_id, label, label_key, position) VALUES (1, 'x', 'x', 0)",
            [],
        )
        .unwrap();
    for (asset, field) in [(1, 1), (2, 1), (1, 2)] {
        db.conn
            .execute(
                "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_int, updated_at)
                 VALUES (?1, ?2, 0, 1, 0)",
                [asset, field],
            )
            .unwrap();
    }

    // Act / Assert: 자산 삭제 -> 그 자산의 값만
    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id = 2", [])
        .unwrap();
    assert_eq!(count(&db.conn, "asset_metadata_values"), 2);

    // 필드 삭제 -> 선택지·값 CASCADE, 다른 필드는 보존
    db.conn
        .execute("DELETE FROM metadata_fields WHERE field_id = 1", [])
        .unwrap();
    assert_eq!(count(&db.conn, "metadata_field_options"), 0);
    assert_eq!(count(&db.conn, "asset_metadata_values"), 1);
    // AUTOINCREMENT: 삭제한 field_id 는 재발급되지 않는다
    add_field(&db.conn, "c", "keyword").unwrap();
    let id: i64 = db
        .conn
        .query_row("SELECT field_id FROM metadata_fields WHERE name = 'c'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(id, 3);
}

#[test]
fn t_umeta_mig_07_field_asset_lookup_uses_the_field_index() {
    let db = migrated_v19();

    let details: Vec<String> = db
        .conn
        .prepare("EXPLAIN QUERY PLAN SELECT DISTINCT asset_id FROM asset_metadata_values WHERE field_id = 1")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    let joined = details.join(" | ");
    assert!(
        details
            .iter()
            .any(|d| d.starts_with("SEARCH") && d.contains("idx_asset_metadata_values_field")),
        "{joined}"
    );
    assert!(!details.iter().any(|d| d.starts_with("SCAN")), "{joined}");
}

#[test]
fn t_umeta_mig_09_deleting_at_is_nullable_on_fields_and_options_and_frees_keys() {
    // Arrange
    let db = migrated_v19();
    add_field(&db.conn, "a", "enum").unwrap();
    db.conn
        .execute(
            "INSERT INTO metadata_field_options (field_id, label, label_key, position) VALUES (1, 'x', 'x', 0)",
            [],
        )
        .unwrap();

    // Assert: 두 테이블 모두 NULL 허용 INTEGER 컬럼, 기본값 NULL
    for table in ["metadata_fields", "metadata_field_options"] {
        let (ty, notnull): (String, i64) = db
            .conn
            .query_row(
                &format!("SELECT type, \"notnull\" FROM pragma_table_info('{table}') WHERE name = 'deleting_at'"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((ty.as_str(), notnull), ("INTEGER", 0), "{table}");
        let set: i64 = db
            .conn
            .query_row(&format!("SELECT count(*) FROM {table} WHERE deleting_at IS NOT NULL"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(set, 0, "{table}");
    }

    // Act: 삭제 표시 + key sentinel 로 같은 이름·선택지를 다시 쓸 수 있다
    db.conn
        .execute("UPDATE metadata_fields SET deleting_at = 5, name_key = char(0) || 'del:1' WHERE field_id = 1", [])
        .unwrap();
    db.conn
        .execute(
            "UPDATE metadata_field_options SET deleting_at = 5, label_key = char(0) || 'del:1' WHERE option_id = 1",
            [],
        )
        .unwrap();

    // Assert
    add_field(&db.conn, "a", "enum").unwrap();
    db.conn
        .execute(
            "INSERT INTO metadata_field_options (field_id, label, label_key, position) VALUES (1, 'x', 'x', 0)",
            [],
        )
        .unwrap();
}

#[test]
fn t_umeta_mig_08_rerun_is_a_noop() {
    let mut db = migrated_v19();

    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();

    assert!(again.applied.is_empty());
}
