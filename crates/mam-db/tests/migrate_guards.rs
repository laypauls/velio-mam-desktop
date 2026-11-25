//! 마이그레이션 가드 통합 테스트 (T-db-04, T-db-06).
//!
//! 분류: Business rule (04 새 스키마·번호 누락·순서 역전) / Validation (06 체크섬).
//! 가드는 백업·적용보다 먼저 동작해야 하므로 "DB 무변경 + 백업 없음"을 함께 단언한다.

mod common;

use common::{
    FixedClock, T0, V1_SQL, applied_versions, dir_entries, empty_db, future_migration, schema_dump,
    shipped_migrations, user_tables, v1, v1_db, v2,
};
use mam_db::{DbError, Migration, latest_version, migrate, migrate_with};
use refinery::error::Kind;

#[test]
fn t_db_04_db_newer_than_app_is_rejected_and_untouched() {
    // Arrange: 앱이 아는 최신보다 높은 버전까지 적용된 DB 를 실제 embedded 목록의 앱으로 연다 (D2: 최신 기준)
    let mut db = empty_db();
    let mut all = shipped_migrations();
    all.push(future_migration());
    migrate_with(&mut db.conn, &all, &db.backup_dir, &FixedClock(T0)).unwrap();
    let backups_before = dir_entries(&db.backup_dir);
    let schema_before = schema_dump(&db.conn);

    // Act
    let err = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 5));

    // Assert
    match err {
        Err(DbError::SchemaTooNew { db: d, app }) => {
            assert_eq!(d, latest_version() + 1);
            assert_eq!(app, latest_version());
        }
        other => panic!("expected SchemaTooNew, got {other:?}"),
    }
    assert_eq!(schema_dump(&db.conn), schema_before);
    assert_eq!(
        applied_versions(&db.conn),
        (1..=latest_version() + 1).collect::<Vec<_>>()
    );
    assert_eq!(dir_entries(&db.backup_dir), backups_before, "no new backup");
}

#[test]
fn t_db_04_refinery_alone_also_aborts_on_missing_version() {
    // Arrange: 도구 자체의 abort_missing 동작 증거 (래퍼 사전 검증과 별개)
    let mut db = v1_db();
    migrate_with(&mut db.conn, &[v1(), v2()], &db.backup_dir, &FixedClock(T0)).unwrap();

    // Act
    let res = refinery::Runner::new(&[v1()])
        .set_grouped(true)
        .set_abort_missing(true)
        .run(&mut db.conn);

    // Assert
    let err = res.expect_err("refinery must abort");
    assert!(matches!(err.kind(), Kind::MissingVersion(_)), "{err:?}");
}

#[test]
fn t_db_04_hole_in_applied_history_is_rejected() {
    // Arrange: 적용 이력 [1,2,3] 인데 앱은 [1,3] 만 안다 (번호 누락)
    let mut db = empty_db();
    let v3 = Migration::unapplied("V3__third", "CREATE TABLE t3 (a INTEGER) STRICT;").unwrap();
    migrate_with(
        &mut db.conn,
        &[v1(), v2(), v3.clone()],
        &db.backup_dir,
        &FixedClock(T0),
    )
    .unwrap();

    let schema_before = schema_dump(&db.conn);

    // Act
    let err = migrate_with(
        &mut db.conn,
        &[v1(), v3],
        &db.backup_dir,
        &FixedClock(T0 + 1),
    );

    // Assert: 오류, DB 무변경, 백업 없음
    assert!(
        matches!(err, Err(DbError::UnknownMigration { version: 2 })),
        "{err:?}"
    );
    assert_eq!(schema_dump(&db.conn), schema_before);
    assert_eq!(applied_versions(&db.conn), vec![1, 2, 3]);
    assert!(dir_entries(&db.backup_dir).is_empty());
}

#[test]
fn t_db_04_out_of_order_pending_migration_is_rejected_without_backup() {
    // Arrange: [1,3] 적용 후 뒤늦게 V2 가 생긴 경우 (브랜치 번호 충돌 시나리오)
    let mut db = empty_db();
    let v3 = Migration::unapplied("V3__third", "CREATE TABLE t3 (a INTEGER) STRICT;").unwrap();
    migrate_with(
        &mut db.conn,
        &[v1(), v3.clone()],
        &db.backup_dir,
        &FixedClock(T0),
    )
    .unwrap();

    // Act
    let err = migrate_with(
        &mut db.conn,
        &[v1(), v2(), v3],
        &db.backup_dir,
        &FixedClock(T0 + 1),
    );

    // Assert
    assert!(
        matches!(
            err,
            Err(DbError::OutOfOrder {
                version: 2,
                applied_max: 3
            })
        ),
        "{err:?}"
    );
    assert!(dir_entries(&db.backup_dir).is_empty());
    assert_eq!(applied_versions(&db.conn), vec![1, 3]);
}

#[test]
fn t_db_06_modified_applied_migration_is_a_checksum_error() {
    // Arrange: 적용 후 V1 파일 내용이 바뀐 상황
    let mut db = v1_db();
    let tampered =
        Migration::unapplied("V1__core_schema", &format!("{V1_SQL}\n-- edited")).unwrap();

    // Act
    let err = migrate_with(
        &mut db.conn,
        &[tampered, v2()],
        &db.backup_dir,
        &FixedClock(T0),
    );

    // Assert: 오류, 스키마 무변경, 백업 없음 (검증이 백업보다 먼저)
    assert!(
        matches!(err, Err(DbError::ChecksumMismatch { version: 1, .. })),
        "{err:?}"
    );
    assert!(!user_tables(&db.conn).contains(&"future_table".to_string()));
    assert!(dir_entries(&db.backup_dir).is_empty());
}

#[test]
fn t_db_06_refinery_alone_detects_divergent_checksum() {
    // Arrange: 도구 자체의 abort_divergent 동작 증거 (래퍼 사전 검증과 별개)
    let mut db = v1_db();
    let tampered =
        Migration::unapplied("V1__core_schema", &format!("{V1_SQL}\n-- edited")).unwrap();

    // Act
    let res = refinery::Runner::new(&[tampered, v2()])
        .set_grouped(true)
        .set_abort_divergent(true)
        .run(&mut db.conn);

    // Assert
    let err = res.expect_err("refinery must abort");
    assert!(matches!(err.kind(), Kind::DivergentVersion(..)), "{err:?}");
    assert_eq!(applied_versions(&db.conn), vec![1]);
}

#[test]
fn t_db_06_crlf_checkout_is_not_a_checksum_mismatch() {
    // Arrange: LF 로 적용된 뒤 같은 내용이 CRLF 로 체크아웃된 경우
    let mut db = v1_db();
    let crlf = Migration::unapplied(
        "V1__core_schema",
        &V1_SQL.replace("\r\n", "\n").replace('\n', "\r\n"),
    )
    .unwrap();

    // Act
    let report = migrate_with(&mut db.conn, &[crlf], &db.backup_dir, &FixedClock(T0)).unwrap();

    // Assert
    assert!(report.applied.is_empty());
}

#[test]
fn t_db_04_schema_too_new_is_reported_before_checksum_mismatch() {
    // Arrange: DB 는 V2 까지 적용, 롤백된 구버전 앱의 V1 은 (변조돼) 체크섬도 다르다
    let mut db = v1_db();
    migrate_with(&mut db.conn, &[v1(), v2()], &db.backup_dir, &FixedClock(T0)).unwrap();
    let tampered =
        Migration::unapplied("V1__core_schema", &format!("{V1_SQL}\n-- edited")).unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[tampered], &db.backup_dir, &FixedClock(T0));

    // Assert
    assert!(
        matches!(err, Err(DbError::SchemaTooNew { db: 2, app: 1 })),
        "{err:?}"
    );
}
