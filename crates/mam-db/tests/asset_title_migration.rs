//! V9 `asset_user_data.title` 마이그레이션 시험 (T-db-title-*) — 기존 메모 행이 있는 V8 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy / Boundary(기존 행 NULL) / Business rule(추가형·구버전 INSERT 호환·재실행 no-op)

mod common;

use common::{FixedClock, T0, TestDb, applied_versions, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v8_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto8: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 8)
        .collect();
    migrate_with(&mut conn, &upto8, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn add_asset(conn: &Connection, id: i64) {
    conn.execute(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
         VALUES (?1, 'image', 10, 0, 0, 0)",
        [id],
    )
    .unwrap();
}

#[test]
fn t_db_title_01_v9_applies_to_a_v8_db_keeping_rows_with_null_title() {
    // Arrange: V8 이전 앱이 만든 메모 행 (title 컬럼을 모른다)
    let mut db = v8_db();
    add_asset(&db.conn, 1);
    db.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, memo, updated_at) VALUES (1, 'old memo', 0)",
            [],
        )
        .unwrap();
    assert_eq!(applied_versions(&db.conn).last(), Some(&8));

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27]); // V10(별점)·V11(edge_sig)·V12(컬렉션 순서)·V13(재생 구간)·V14(폴더 숨김)·V15(장면 컷)·V16(jobs scene)·V17(폴더 별칭)·V18(폴더 유형)·V19(사용자 필드)·V20(저장 검색 삭제)·V21(변경 감지)·V22(변경 대기열)·V23(원격 접속)·V24(이어보기)도 이어서 적용된다
    assert!(report.backup.is_some_and(|b| b.is_file()));
    let (memo, title): (String, Option<String>) = db
        .conn
        .query_row(
            "SELECT memo, title FROM asset_user_data WHERE asset_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((memo.as_str(), title), ("old memo", None));
}

#[test]
fn t_db_title_02_old_style_insert_without_title_still_works() {
    let mut db = v8_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    add_asset(&db.conn, 2);

    db.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, favorite, updated_at) VALUES (2, 1, 0)",
            [],
        )
        .unwrap();

    let title: Option<String> = db
        .conn
        .query_row(
            "SELECT title FROM asset_user_data WHERE asset_id = 2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title, None);
}

#[test]
fn t_db_title_03_fresh_db_has_the_column_and_rerun_is_a_noop() {
    let mut db = common::empty_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0)).unwrap();

    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    assert!(again.applied.is_empty());
    let has: bool = db
        .conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('asset_user_data') WHERE name = 'title')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(has);
}
