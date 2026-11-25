//! V10 `asset_user_data.rating` 마이그레이션 시험 (T-rating-db-*) — 기존 행이 있는 V9 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy / Boundary(기존 행 0) / Validation(CHECK 0..=5) / Business rule(구버전 INSERT 호환·부분 인덱스·재실행 no-op)

mod common;

use common::{FixedClock, T0, TestDb, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v9_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto9: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 9)
        .collect();
    migrate_with(&mut conn, &upto9, &backup_dir, &FixedClock(T0)).unwrap();
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

fn rating_of(conn: &Connection, id: i64) -> i64 {
    conn.query_row(
        "SELECT rating FROM asset_user_data WHERE asset_id = ?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn t_rating_db_01_v10_applies_to_a_v9_db_keeping_existing_rows_unrated() {
    // Arrange: 별점을 모르는 앱이 만든 즐겨찾기 행
    let mut db = v9_db();
    add_asset(&db.conn, 1);
    db.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, favorite, updated_at) VALUES (1, 1, 0)",
            [],
        )
        .unwrap();

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27]); // V11(edge_sig)·V12(컬렉션 순서)·V13(재생 구간)·V14(폴더 숨김)·V15(장면 컷)·V16(jobs scene)·V17(폴더 별칭)·V18(폴더 유형)·V19(사용자 필드)·V20(저장 검색 삭제)·V21(변경 감지)·V22(변경 대기열)·V23(원격 접속)·V24(이어보기)도 이어서 적용된다
    assert!(report.backup.is_some_and(|b| b.is_file()));
    assert_eq!(rating_of(&db.conn, 1), 0);
}

#[test]
fn t_rating_db_02_check_constraint_rejects_values_outside_0_to_5() {
    let mut db = v9_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    add_asset(&db.conn, 2);

    for ok in [0, 5] {
        db.conn
            .execute(
                "INSERT OR REPLACE INTO asset_user_data (asset_id, rating, updated_at) VALUES (2, ?1, 0)",
                [ok],
            )
            .unwrap();
        assert_eq!(rating_of(&db.conn, 2), ok);
    }
    for bad in [-1, 6, 255] {
        let err = db.conn.execute(
            "UPDATE asset_user_data SET rating = ?1 WHERE asset_id = 2",
            [bad],
        );
        assert!(err.is_err(), "rating {bad} must violate the CHECK");
    }
    assert_eq!(rating_of(&db.conn, 2), 5);
}

#[test]
fn t_rating_db_03_old_style_insert_works_partial_index_exists_and_rerun_is_a_noop() {
    let mut db = v9_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    add_asset(&db.conn, 3);

    // 구 버전 앱의 INSERT (rating 컬럼을 모른다)
    db.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, memo, updated_at) VALUES (3, 'm', 0)",
            [],
        )
        .unwrap();
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();

    assert_eq!(rating_of(&db.conn, 3), 0);
    assert!(again.applied.is_empty());
    let partial: bool = db
        .conn
        .query_row(
            "SELECT sql LIKE '%WHERE rating > 0%' FROM sqlite_master WHERE name = 'idx_asset_user_data_rating'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(partial);
}
