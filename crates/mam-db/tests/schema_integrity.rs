//! 스키마 전반 무결성 테스트 (T-db-08 STRICT 가드, T-db-11 해시 길이).
//!
//! 분류: Validation / Boundary (해시 0, 1, 31, 32, 33 바이트).

mod common;

use common::{T0, count, migrated_db};
use rusqlite::Connection;

fn insert_with_hash(conn: &Connection, len: usize, algo: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO media_assets
           (media_type, hash_state, content_hash, hash_algo, file_size, modified_at, created_at, updated_at)
         VALUES ('image', 'done', zeroblob(?1), ?2, 1, ?3, ?3, ?3)",
        (len as i64, algo, T0),
    )
}

#[test]
fn t_db_11_hash_length_must_be_32_bytes() {
    // Arrange
    let db = migrated_db();

    // Act / Assert: 경계값 0, 1, 31, 33 은 실패, 32 는 성공
    for bad in [0usize, 1, 31, 33] {
        let err = insert_with_hash(&db.conn, bad, "blake3")
            .expect_err("wrong length must fail")
            .to_string();
        assert!(err.contains("CHECK constraint failed"), "len {bad}: {err}");
    }
    assert_eq!(insert_with_hash(&db.conn, 32, "blake3").unwrap(), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 1);
}

#[test]
fn t_db_11_32_byte_hash_with_other_algo_is_rejected() {
    let db = migrated_db();

    let res = insert_with_hash(&db.conn, 32, "sha256");

    assert!(res.is_err());
}

#[test]
fn t_db_08_every_v1_table_is_strict() {
    // Arrange
    let db = migrated_db();
    let mut stmt = db
        .conn
        .prepare(
            "SELECT name, strict FROM pragma_table_list
             WHERE schema = 'main' AND type = 'table'
               AND name NOT LIKE 'sqlite_%' AND name <> 'refinery_schema_history'",
        )
        .unwrap();

    // Act
    let tables: Vec<(String, bool)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // Assert: refinery 이력 테이블을 뺀 모든 테이블이 STRICT (새 마이그레이션이 STRICT 를 빼먹으면 실패)
    assert_eq!(tables.len(), 41, "{tables:?}"); // V1 10개 + V2 5개 + V3 1개 + V4 3개 + V5 2개(V20 에서 saved_searches 삭제 -1) + V6 8개 + V13 1개 + V15 1개 + V19 3개 + V22 1개 + V23 2개 + V24 1개 + V26 4개
    let non_strict: Vec<_> = tables.iter().filter(|(_, strict)| !strict).collect();
    assert!(non_strict.is_empty(), "non-STRICT tables: {non_strict:?}");
}
