//! V3 (자산 처리) 스키마 계약 (T-db-30 ~ 33).
//!
//! 분류: Validation (30 CHECK·PK) / Business rule (31 CASCADE, 32 V3 가 V1~V2 데이터를 건드리지 않음) /
//! Performance 계약 (33 EXPLAIN QUERY PLAN).
//! 쿼리 모양은 `crates/mam-core/src/{processing,reconcile}` 의 실제 SQL 과 같게 고정한다 - 바꾸면 이 테스트도 같이 본다.

mod common;

use common::{
    FixedClock, T0, applied_versions, count, insert_asset, insert_location, insert_volume,
    migrated_db, shipped_migrations, v1_db,
};
use mam_db::migrate_with;
use rusqlite::Connection;

fn plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn deriv(conn: &Connection, asset: i64, kind: &str, state: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO asset_derivatives (asset_id, kind, state, updated_at) VALUES (?1, ?2, ?3, ?4)",
        (asset, kind, state, T0),
    )
}

#[test]
fn t_db_30_derivative_rows_are_unique_per_kind_and_validated() {
    // Arrange
    let db = migrated_db();
    let a = insert_asset(&db.conn);
    deriv(&db.conn, a, "thumb", "done").unwrap();

    // Act / Assert: (asset, kind) 중복 불가, 같은 자산의 다른 종류는 가능
    assert!(deriv(&db.conn, a, "thumb", "failed").is_err());
    deriv(&db.conn, a, "meta", "pending").unwrap();
    // 알 수 없는 종류·상태·없는 자산은 거부
    assert!(deriv(&db.conn, a, "proxy", "done").is_err());
    assert!(deriv(&db.conn, a, "strip", "running").is_err());
    assert!(deriv(&db.conn, 9999, "strip", "done").is_err());
    // 멱등 UPSERT (실행기가 쓰는 형태)
    for _ in 0..2 {
        db.conn
            .execute(
                "INSERT INTO asset_derivatives (asset_id, kind, state, error, updated_at)
                 VALUES (?1, 'thumb', 'failed', 'boom', ?2)
                 ON CONFLICT (asset_id, kind) DO UPDATE SET
                   state = excluded.state, error = excluded.error, updated_at = excluded.updated_at",
                (a, T0 + 1),
            )
            .unwrap();
    }
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_derivatives"), 2);
    assert_eq!(
        count(
            &db.conn,
            "SELECT COUNT(*) FROM asset_derivatives WHERE state = 'failed'"
        ),
        1
    );
}

#[test]
fn t_db_30b_extended_metadata_columns_accept_values_and_check_playback() {
    // Arrange
    let db = migrated_db();
    let a = insert_asset(&db.conn);

    // Act: V1 컬럼 + V3 컬럼 (모두 NULL 허용)
    db.conn
        .execute("INSERT INTO media_metadata (asset_id) VALUES (?1)", [a])
        .unwrap();
    db.conn
        .execute(
            "UPDATE media_metadata SET width = 1920, height = 1080, duration_ms = 5000, codec = 'h264',
               audio_codec = 'aac', fps = 29.97, bitrate = 800000, camera = 'Canon X', orientation = 1,
               playback = 'conditional', extra = '{\"container\":\"mp4\"}'
             WHERE asset_id = ?1",
            [a],
        )
        .unwrap();

    // Assert
    let fps: f64 = db
        .conn
        .query_row(
            "SELECT fps FROM media_metadata WHERE asset_id = ?1",
            [a],
            |r| r.get(0),
        )
        .unwrap();
    assert!((fps - 29.97).abs() < 1e-9);
    assert!(
        db.conn
            .execute(
                "UPDATE media_metadata SET playback = 'maybe' WHERE asset_id = ?1",
                [a]
            )
            .is_err()
    );
}

#[test]
fn t_db_31_derivatives_are_removed_with_their_asset() {
    // Arrange
    let db = migrated_db();
    let a = insert_asset(&db.conn);
    deriv(&db.conn, a, "thumb", "done").unwrap();
    deriv(&db.conn, a, "strip", "failed").unwrap();

    // Act: 자산 행 삭제 (위치가 없을 때만 가능)
    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [a])
        .unwrap();

    // Assert
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_derivatives"), 0);
}

#[test]
fn t_db_32_v3_applies_on_top_of_v1_data_without_touching_it() {
    // Arrange: V1 시점 앱이 만든 데이터 (메타 행 포함)
    let mut db = v1_db();
    let v = insert_volume(&db.conn, "vol-a");
    let a = insert_asset(&db.conn);
    insert_location(&db.conn, a, v, "photos/a.jpg").unwrap();
    db.conn
        .execute(
            "INSERT INTO media_metadata (asset_id, width, height, codec) VALUES (?1, 640, 480, 'jpeg')",
            [a],
        )
        .unwrap();

    // Act
    let report = migrate_with(
        &mut db.conn,
        &shipped_migrations(),
        &db.backup_dir,
        &FixedClock(T0 + 1),
    )
    .unwrap();

    // Assert: 추가형 - 기존 행·값 그대로, 새 컬럼은 NULL
    assert!(report.applied.contains(&3));
    assert_eq!(
        applied_versions(&db.conn),
        (1..=mam_db::latest_version()).collect::<Vec<_>>()
    );
    let row: (i64, i64, String, Option<String>, Option<String>) = db
        .conn
        .query_row(
            "SELECT width, height, codec, audio_codec, playback FROM media_metadata WHERE asset_id = ?1",
            [a],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(row, (640, 480, "jpeg".to_string(), None, None));
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM file_locations"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_derivatives"), 0);
}

#[test]
fn t_db_33_processing_queries_use_indexes() {
    // Arrange
    let db = migrated_db();

    // 재등록 재연결: detach 위치 부분 인덱스 (reconcile::store::DETACHED_AT_SQL 과 같은 모양)
    let detached = "SELECT l.asset_id FROM file_locations l INDEXED BY idx_file_locations_detached_path
        JOIN media_assets a ON a.asset_id = l.asset_id
        WHERE l.volume_id = 1 AND l.path_key = 'a/b.jpg' AND l.detached_at IS NOT NULL
          AND a.status = 'active' AND a.file_size = 10 AND a.modified_at = 5
          AND NOT EXISTS (SELECT 1 FROM file_locations x WHERE x.asset_id = a.asset_id AND x.detached_at IS NULL)
        ORDER BY l.location_id DESC LIMIT 1";
    let p = plan(&db.conn, detached);
    assert!(p.contains("idx_file_locations_detached_path"), "{p}");
    assert!(
        p.contains("uq_file_locations_active_asset"),
        "NOT EXISTS must use the active-asset index: {p}"
    );

    // 파생물 상태 조회 (PK)
    let p = plan(
        &db.conn,
        "SELECT state FROM asset_derivatives WHERE asset_id = 1 AND kind = 'thumb'",
    );
    assert!(p.contains("PRIMARY KEY") || p.contains("autoindex"), "{p}");

    // 실패 목록
    let p = plan(
        &db.conn,
        "SELECT asset_id FROM asset_derivatives WHERE kind = 'thumb' AND state = 'failed'",
    );
    assert!(p.contains("idx_asset_derivatives_failed"), "{p}");

    // 보충 등록: 해시 대기 자산은 부분 인덱스, 파생물 누락 검사는 PK 탐색 (자산 전체 스캔은 키 순서 배치 안에서만)
    let p = plan(
        &db.conn,
        "SELECT asset_id FROM media_assets WHERE hash_state = 'pending' AND status = 'active'",
    );
    assert!(p.contains("idx_media_assets_hash_pending"), "{p}");
    let p = plan(
        &db.conn,
        "SELECT a.asset_id, a.media_type FROM media_assets a
         WHERE a.asset_id > 10 AND a.status = 'active'
           AND NOT EXISTS (SELECT 1 FROM asset_derivatives d WHERE d.asset_id = a.asset_id AND d.kind = 'meta')
         ORDER BY a.asset_id LIMIT 1000",
    );
    assert!(
        p.contains("SEARCH a USING INTEGER PRIMARY KEY"),
        "batch must walk the PK range: {p}"
    );
    assert!(
        p.contains("SEARCH d USING"),
        "existence check must be an index lookup: {p}"
    );
}
