//! T-stt-04~06, 08 - 전사 저장 (revision + outbox 한 트랜잭션, 덮어쓰기, 롤백, 경계). 실제 SQLite + 마이그레이션 + 제안 DDL.
//! 분류: Happy / Concurrency(멱등) / External(실패 롤백) / Boundary / Business rule.

mod common;

use common::{TestClock, TestDb, migrated_db};
use mam_core::ai_store::{
    AiStoreError, TranscriptRecord, TranscriptSegment, load_transcript, save_transcript,
};

fn db() -> TestDb {
    migrated_db()
}

fn asset(db: &TestDb, status: &str) -> i64 {
    db.conn
        .query_row(
            "INSERT INTO media_assets (media_type, status, file_size, modified_at, created_at, updated_at)
             VALUES ('audio', ?1, 10, 1, 1, 1) RETURNING asset_id",
            [status],
            |r| r.get(0),
        )
        .unwrap()
}

fn seg(a: u64, b: u64, t: &str) -> TranscriptSegment {
    TranscriptSegment {
        start_ms: a,
        end_ms: b,
        text: t.into(),
    }
}

fn rec(lang: &str, segs: Vec<TranscriptSegment>) -> TranscriptRecord {
    TranscriptRecord {
        language: lang.into(),
        segments: segs,
        engine: "whisper.cpp".into(),
        model: "ggml-base".into(),
    }
}

fn count(db: &TestDb, sql: &str) -> i64 {
    db.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn revision(db: &TestDb, id: i64) -> i64 {
    db.conn
        .query_row(
            "SELECT revision FROM media_assets WHERE asset_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn t_stt_04_save_writes_rows_bumps_revision_and_enqueues_one_low_priority_upsert() {
    // Arrange
    let mut db = db();
    let id = asset(&db, "active");
    let clock = TestClock::new();
    let before = count(&db, "SELECT COUNT(*) FROM search_outbox");
    let r = rec(
        "ko",
        vec![seg(0, 2800, "오늘 회의"), seg(2800, 5000, "일정")],
    );

    // Act
    let out = save_transcript(&mut db.conn, &clock, id, &r).unwrap();

    // Assert
    assert_eq!(out.revision, 2);
    assert_eq!(revision(&db, id), 2);
    let v = load_transcript(&db.conn, id).unwrap().unwrap();
    assert_eq!(v.language, "ko");
    assert_eq!(v.text, "오늘 회의 일정");
    assert_eq!(
        (v.engine.as_str(), v.model.as_str()),
        ("whisper.cpp", "ggml-base")
    );
    assert_eq!(v.created_at, common::T0);
    assert_eq!(v.segments, r.segments);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM search_outbox"), before + 1);
    let (rev, ev, prio): (i64, String, i64) = db
        .conn
        .query_row(
            "SELECT revision, event_type, priority FROM search_outbox
             WHERE asset_id = ?1 ORDER BY event_id DESC LIMIT 1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((rev, ev.as_str(), prio), (2, "upsert", 1));
}

#[test]
fn t_stt_04_view_serializes_to_ui_contract_camel_case() {
    let mut db = db();
    let id = asset(&db, "active");
    save_transcript(
        &mut db.conn,
        &TestClock::new(),
        id,
        &rec("en", vec![seg(1, 2, "hi")]),
    )
    .unwrap();

    let v = serde_json::to_value(load_transcript(&db.conn, id).unwrap().unwrap()).unwrap();

    assert_eq!(v["segments"][0]["startMs"], 1);
    assert_eq!(v["segments"][0]["endMs"], 2);
    assert_eq!(v["createdAt"], common::T0);
    assert!(v.get("engine").is_some() && v.get("text").is_some());
}

#[test]
fn t_stt_05_resave_overwrites_without_duplicate_segments() {
    let mut db = db();
    let id = asset(&db, "active");
    let clock = TestClock::new();
    let first = rec("ko", vec![seg(0, 1, "a"), seg(1, 2, "b"), seg(2, 3, "c")]);
    save_transcript(&mut db.conn, &clock, id, &first).unwrap();
    clock.advance(1000);

    let out =
        save_transcript(&mut db.conn, &clock, id, &rec("en", vec![seg(0, 5, "new")])).unwrap();

    assert_eq!(out.revision, 3);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM transcripts"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM transcript_segments"), 1);
    let v = load_transcript(&db.conn, id).unwrap().unwrap();
    assert_eq!((v.language.as_str(), v.text.as_str()), ("en", "new"));
    assert_eq!(v.created_at, common::T0 + 1000);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM search_outbox WHERE revision = 3"),
        1
    );
}

#[test]
fn t_stt_06_missing_or_inactive_asset_changes_nothing() {
    let mut db = db();
    let deleted = asset(&db, "deleted");
    let outbox = count(&db, "SELECT COUNT(*) FROM search_outbox");
    let revs = count(&db, "SELECT SUM(revision) FROM media_assets");

    for id in [9999, deleted] {
        let r = rec("ko", vec![seg(0, 1, "x")]);
        let e = save_transcript(&mut db.conn, &TestClock::new(), id, &r).unwrap_err();
        assert!(matches!(e, AiStoreError::NotFound(i) if i == id), "{e}");
    }

    assert_eq!(count(&db, "SELECT COUNT(*) FROM transcripts"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM search_outbox"), outbox);
    assert_eq!(count(&db, "SELECT SUM(revision) FROM media_assets"), revs);
}

#[test]
fn t_stt_06_failure_midway_rolls_back_everything_and_keeps_previous_transcript() {
    // 두 번째 저장의 구간이 CHECK(end_ms >= start_ms)를 어겨 도중 실패 -> 이전 전사·revision·outbox 유지
    let mut db = db();
    let id = asset(&db, "active");
    let clock = TestClock::new();
    save_transcript(&mut db.conn, &clock, id, &rec("ko", vec![seg(0, 1, "old")])).unwrap();
    let outbox = count(&db, "SELECT COUNT(*) FROM search_outbox");

    let bad = rec("en", vec![seg(0, 9, "ok"), seg(9, 3, "bad")]);
    let e = save_transcript(&mut db.conn, &clock, id, &bad).unwrap_err();

    assert!(matches!(e, AiStoreError::Db(_)), "{e}");
    assert_eq!(revision(&db, id), 2);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM search_outbox"), outbox);
    let v = load_transcript(&db.conn, id).unwrap().unwrap();
    assert_eq!((v.language.as_str(), v.text.as_str()), ("ko", "old"));
}

#[test]
fn t_stt_08_empty_transcript_and_mixed_language_text_are_stored_verbatim() {
    let mut db = db();
    let (a, b) = (asset(&db, "active"), asset(&db, "active"));
    let clock = TestClock::new();
    let mixed = rec(
        "ko",
        vec![
            seg(0, 1, "오늘 meeting 은 Q3 리뷰"),
            seg(1, 2, "OK 좋습니다"),
        ],
    );

    save_transcript(&mut db.conn, &clock, a, &rec("", vec![])).unwrap();
    save_transcript(&mut db.conn, &clock, b, &mixed).unwrap();

    let va = load_transcript(&db.conn, a).unwrap().unwrap();
    assert!(va.segments.is_empty() && va.text.is_empty() && va.language.is_empty());
    let vb = load_transcript(&db.conn, b).unwrap().unwrap();
    assert_eq!(vb.text, "오늘 meeting 은 Q3 리뷰 OK 좋습니다");
    assert_eq!(load_transcript(&db.conn, 12345).unwrap(), None);
}

#[test]
fn t_stt_06_deleting_asset_cascades_to_transcript_and_segments() {
    let mut db = db();
    let id = asset(&db, "active");
    let r = rec("ko", vec![seg(0, 1, "a")]);
    save_transcript(&mut db.conn, &TestClock::new(), id, &r).unwrap();

    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [id])
        .unwrap();

    assert_eq!(count(&db, "SELECT COUNT(*) FROM transcripts"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM transcript_segments"), 0);
}
