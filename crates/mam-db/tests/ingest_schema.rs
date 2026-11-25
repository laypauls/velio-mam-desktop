//! V2 (수집) 스키마 계약 (T-db-20 ~ 22).
//!
//! 분류: Business rule (20 제약·CASCADE) / Validation (20 CHECK) / Performance 계약 (21 EXPLAIN QUERY PLAN) /
//! Business rule (22 V1 -> V2 적용: 기존 데이터 보존).
//! 쿼리 모양은 `crates/mam-core/src/{scan,jobs,reconcile}` 의 실제 SQL 과 같게 고정한다 — 바꾸면 이 테스트도 같이 본다.

mod common;

use common::{
    FixedClock, T0, applied_versions, count, insert_asset, insert_location, insert_volume,
    migrated_db, shipped_migrations, v1_db,
};
use mam_db::migrate_with;
use rusqlite::Connection;

fn plan(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn assert_indexed(conn: &Connection, sql: &str, index: &str) {
    let details = plan(conn, sql);
    let joined = details.join(" | ");
    assert!(
        details.iter().any(|d| d.contains(index)),
        "expected {index}, got: {joined}"
    );
    assert!(
        !details.iter().any(|d| d.starts_with("SCAN")
            && !d.contains("COVERING INDEX")
            && !d.contains("USING INDEX")),
        "full table scan in plan: {joined}"
    );
}

fn folder(conn: &Connection, volume: i64, key: &str) -> i64 {
    conn.execute(
        "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at)
         VALUES (?1, ?2, ?2, ?2, ?3)",
        (volume, key, T0),
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn t_db_20_folder_root_is_unique_per_volume_and_children_cascade() {
    // Arrange
    let db = migrated_db();
    let v = insert_volume(&db.conn, "vol-a");
    let f = folder(&db.conn, v, "photos");
    db.conn
        .execute(
            "INSERT INTO scan_runs (folder_id, kind, status, started_at) VALUES (?1, 'full', 'running', 1)",
            [f],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO scan_errors (folder_id, rel_path, error_code, message, last_at) VALUES (?1, 'x', 'io', 'm', 1)",
            [f],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO folder_dirs (folder_id, dir_key, parent_key, name, rel_path, seen_at) VALUES (?1, 'a', '', 'A', 'A', 1)",
            [f],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO jobs (kind, folder_id, created_at, updated_at) VALUES ('scan', ?1, 1, 1)",
            [f],
        )
        .unwrap();

    // Act / Assert: 같은 볼륨·같은 루트 중복 불가, 다른 볼륨은 가능
    let dup = db.conn.execute(
        "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at) VALUES (?1, 'Photos', 'photos', 'x', 1)",
        [v],
    );
    assert!(dup.is_err());
    let v2 = insert_volume(&db.conn, "vol-b");
    folder(&db.conn, v2, "photos");

    // 폴더 삭제 -> 실행·오류·트리·작업이 함께 정리된다
    db.conn
        .execute("DELETE FROM managed_folders WHERE folder_id = ?1", [f])
        .unwrap();
    for table in ["scan_runs", "scan_errors", "folder_dirs", "jobs"] {
        assert_eq!(
            count(&db.conn, &format!("SELECT COUNT(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    // 볼륨은 폴더가 있으면 지울 수 없다 (RESTRICT)
    assert!(
        db.conn
            .execute("DELETE FROM volumes WHERE volume_id = ?1", [v2])
            .is_err()
    );
}

#[test]
fn t_db_20b_job_rules_pending_uniqueness_and_checks() {
    let db = migrated_db();
    let ins = |kind: &str, asset: i64, status: &str| {
        db.conn.execute(
            "INSERT INTO jobs (kind, asset_id, status, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 1)",
            (kind, asset, status),
        )
    };

    assert!(ins("thumb", 1, "pending").is_ok());
    assert!(
        ins("thumb", 1, "pending").is_err(),
        "one pending job per (kind, asset)"
    );
    assert!(
        ins("thumb", 1, "running").is_ok(),
        "a running one may coexist with a pending one"
    );
    assert!(ins("meta", 1, "pending").is_ok());
    assert!(ins("bogus", 2, "pending").is_err(), "kind CHECK");
    assert!(
        ins("thumb", 3, "done").is_err(),
        "finished jobs are deleted, not stored"
    );
    assert!(
        db.conn
            .execute("INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at) VALUES (99, 'a', 'a', 'a', 1)", [])
            .is_err(),
        "FK to volumes"
    );
}

#[test]
fn t_db_20c_scan_run_and_error_checks() {
    let db = migrated_db();
    let v = insert_volume(&db.conn, "vol-a");
    let f = folder(&db.conn, v, "p");

    let bad_run = db.conn.execute(
        "INSERT INTO scan_runs (folder_id, kind, status, started_at) VALUES (?1, 'full', 'weird', 1)",
        [f],
    );
    assert!(bad_run.is_err());
    let e = |code: &str| {
        db.conn.execute(
            "INSERT INTO scan_errors (folder_id, rel_path, error_code, message, last_at) VALUES (?1, 'a', ?2, 'm', 1)",
            (f, code),
        )
    };
    assert!(e("io").is_ok());
    assert!(e("io").is_err(), "same (folder, path, code) is one row");
    assert!(e("permission_denied").is_ok());
}

#[test]
fn t_db_21_ingest_queries_use_indexes() {
    let db = migrated_db();

    // 폴더/디렉터리 자산 수 (경로 키 범위 — 실측: 30만 건 하위 계수 약 38ms, release, 웜 캐시)
    let count_sql = "SELECT COUNT(*) FROM file_locations
         WHERE volume_id = 1 AND detached_at IS NULL AND path_key >= 'a/' AND path_key < 'a0'";
    assert_indexed(&db.conn, count_sql, "uq_file_locations_active_path");
    assert!(
        plan(&db.conn, count_sql)
            .iter()
            .any(|d| d.contains("path_key>? AND path_key<?")),
        "range count must be a bounded index range: {:?}",
        plan(&db.conn, count_sql)
    );

    // 스캔 완료 시 못 본 위치 찾기 (폴더 범위)
    assert_indexed(
        &db.conn,
        "SELECT location_id, asset_id FROM file_locations
         WHERE volume_id = 1 AND detached_at IS NULL AND presence = 'present'
           AND path_key >= 'a/' AND path_key < 'a0' AND last_seen_at < 5",
        "uq_file_locations_active_path",
    );

    // ScanSession::begin 의 missing 후보 적재 (P1-4 잔여 위험)
    assert_indexed(
        &db.conn,
        "SELECT l.location_id, l.asset_id, a.file_size, a.content_hash
         FROM file_locations l JOIN media_assets a ON a.asset_id = l.asset_id
         WHERE l.detached_at IS NULL AND l.presence = 'missing'
           AND a.status = 'active' AND a.hash_state = 'done'",
        "idx_file_locations_missing",
    );

    // 작업 claim: 종류별 최우선 1건 (정렬 없이)
    let claim = "SELECT job_id, asset_id, folder_id, priority, attempts FROM jobs
         WHERE kind = 'thumb' AND status = 'pending' AND not_before <= 5
         ORDER BY priority, job_id LIMIT 1";
    assert_indexed(&db.conn, claim, "idx_jobs_ready");
    assert!(
        !plan(&db.conn, claim)
            .iter()
            .any(|d| d.contains("TEMP B-TREE"))
    );

    // 종류·상태별 건수 (요약)
    assert_indexed(
        &db.conn,
        "SELECT kind, status, COUNT(*) FROM jobs GROUP BY kind, status",
        "idx_jobs_kind_status",
    );
    // 폴더 트리 한 단계
    let tree = "SELECT dir_key, name, rel_path FROM folder_dirs INDEXED BY idx_folder_dirs_parent
         WHERE folder_id = 1 AND parent_key = 'a' ORDER BY dir_key";
    assert_indexed(&db.conn, tree, "idx_folder_dirs_parent");
    assert!(
        !plan(&db.conn, tree)
            .iter()
            .any(|d| d.contains("TEMP B-TREE"))
    );
    // 미해결 오류 키셋 페이지
    let errors = "SELECT error_id FROM scan_errors WHERE resolved_at IS NULL AND error_id < 50
         ORDER BY error_id DESC LIMIT 20";
    assert_indexed(&db.conn, errors, "idx_scan_errors_open");
    assert!(
        !plan(&db.conn, errors)
            .iter()
            .any(|d| d.contains("TEMP B-TREE"))
    );
    // 폴더의 스캔 작업 상태
    assert_indexed(
        &db.conn,
        "SELECT status FROM jobs WHERE folder_id = 1",
        "idx_jobs_folder",
    );
}

#[test]
fn t_db_22_v2_applies_on_top_of_v1_data_without_touching_it() {
    // Arrange: V1 시점 앱이 만든 데이터
    let mut db = v1_db();
    let v = insert_volume(&db.conn, "vol-a");
    let a = insert_asset(&db.conn);
    insert_location(&db.conn, a, v, "photos/a.jpg").unwrap();

    // Act: 현재 앱의 마이그레이션 전체 적용
    let report = migrate_with(
        &mut db.conn,
        &shipped_migrations(),
        &db.backup_dir,
        &FixedClock(T0 + 1),
    )
    .unwrap();

    // Assert: 추가형 — V1 데이터 그대로, 적용 전 백업 생성
    let latest = mam_db::latest_version();
    assert_eq!(report.applied, (2..=latest).collect::<Vec<_>>());
    assert!(report.backup.is_some());
    assert_eq!(applied_versions(&db.conn), (1..=latest).collect::<Vec<_>>());
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM file_locations"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM managed_folders"), 0);
}
