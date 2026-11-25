//! T-stt-07~08 - 구간 찾기·색인 텍스트·쿼리 플랜. 분류: Happy / Boundary / Business rule.

mod common;

use common::{TestClock, TestDb, migrated_db};
use mam_core::ai_store::{
    INDEX_TEXT_MAX_CHARS, TranscriptRecord, TranscriptSegment, find_segments, save_transcript,
    transcript_index_text,
};

fn seg(a: u64, b: u64, t: &str) -> TranscriptSegment {
    TranscriptSegment {
        start_ms: a,
        end_ms: b,
        text: t.into(),
    }
}

fn saved(segs: Vec<TranscriptSegment>) -> (TestDb, i64) {
    let mut db = migrated_db();
    let id: i64 = db
        .conn
        .query_row(
            "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
             VALUES ('audio', 1, 1, 1, 1) RETURNING asset_id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let rec = TranscriptRecord {
        language: "ko".into(),
        segments: segs,
        engine: "whisper.cpp".into(),
        model: "m".into(),
    };
    save_transcript(&mut db.conn, &TestClock::new(), id, &rec).unwrap();
    (db, id)
}

#[test]
fn t_stt_07_find_segments_returns_times_of_matching_segments_case_insensitive() {
    let (db, id) = saved(vec![
        seg(0, 2800, "오늘 Meeting 시작"),
        seg(2800, 5000, "예산 논의"),
        seg(5000, 7000, "다음 MEETING 은 금요일"),
    ]);

    let hits = find_segments(&db.conn, id, &["meeting"], 10).unwrap();
    assert_eq!(
        hits.iter()
            .map(|h| (h.start_ms, h.end_ms))
            .collect::<Vec<_>>(),
        vec![(0, 2800), (5000, 7000)]
    );
    // 부분 일치(한글)와 다중 질의어 OR
    assert_eq!(
        find_segments(&db.conn, id, &["예산", "금요"], 10)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn t_stt_07_find_segments_boundaries_limit_empty_terms_unknown_asset() {
    let (db, id) = saved(vec![seg(0, 1, "a b"), seg(1, 2, "a c"), seg(2, 3, "a d")]);

    assert_eq!(find_segments(&db.conn, id, &["a"], 2).unwrap().len(), 2);
    assert!(find_segments(&db.conn, id, &["a"], 0).unwrap().is_empty());
    assert!(find_segments(&db.conn, id, &[], 5).unwrap().is_empty());
    assert!(find_segments(&db.conn, id, &[""], 5).unwrap().is_empty());
    assert!(find_segments(&db.conn, id, &["zzz"], 5).unwrap().is_empty());
    assert!(find_segments(&db.conn, 777, &["a"], 5).unwrap().is_empty());
}

#[test]
fn t_stt_07_unicode_lowercase_matches_non_ascii_letters() {
    let (db, id) = saved(vec![seg(0, 1, "ÉCOLE de Paris"), seg(1, 2, "Straße")]);

    assert_eq!(find_segments(&db.conn, id, &["école"], 5).unwrap().len(), 1);
    assert_eq!(
        find_segments(&db.conn, id, &["straße"], 5).unwrap().len(),
        1
    );
}

#[test]
fn t_stt_08_index_text_joins_skips_blank_and_caps_on_char_boundary() {
    assert_eq!(transcript_index_text(&[]), "");
    assert_eq!(
        transcript_index_text(&[seg(0, 1, " 가 "), seg(1, 2, "  "), seg(2, 3, "b")]),
        "가 b"
    );

    // 멀티바이트 문자로 상한을 넘기면 정확히 상한 문자 수에서 절단 (바이트 경계 패닉 없음)
    let long = "가".repeat(INDEX_TEXT_MAX_CHARS - 10);
    let t = transcript_index_text(&[seg(0, 1, &long), seg(1, 2, &"나".repeat(50))]);
    assert_eq!(t.chars().count(), INDEX_TEXT_MAX_CHARS);
    assert!(t.ends_with("나나나나나나나나") && t.contains("가 나"));

    // 정확히 상한이면 절단 없음, 다음 구간은 버려진다
    let exact = "x".repeat(INDEX_TEXT_MAX_CHARS);
    let t = transcript_index_text(&[seg(0, 1, &exact), seg(1, 2, "tail")]);
    assert_eq!(t.chars().count(), INDEX_TEXT_MAX_CHARS);
    assert!(!t.contains("tail"));
}

#[test]
fn t_stt_07_queries_use_indexes_per_explain_query_plan() {
    let (db, _) = saved(vec![seg(0, 1, "a")]);
    let plan = |sql: &str| -> String {
        let mut st = db
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        st.query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>()
            .join(" | ")
    };

    let segs = plan(
        "SELECT s.start_ms, s.end_ms, s.text FROM transcripts t
         JOIN transcript_segments s ON s.transcript_id = t.transcript_id
         WHERE t.asset_id = 1 ORDER BY s.seq",
    );
    assert!(segs.contains("idx_transcript_segments_order"), "{segs}");
    assert!(!segs.contains("SCAN s"), "{segs}");
    assert!(!segs.contains("TEMP B-TREE"), "no sort needed: {segs}");

    let head = plan("SELECT language FROM transcripts WHERE asset_id = 1");
    assert!(head.contains("SEARCH") && head.contains("INDEX"), "{head}");
}
