//! 청크 기록 시험 (T-pending-30 ~ 33) — 큰 묶음을 `MAX_ROWS_PER_TX` 행씩 별도 트랜잭션으로 나눈다.
//! 분류: Happy (30 5000 경로 -> 3 청크·31 항목 없음) / Boundary (30 경계·31) / External (32 중간 청크 실패) / Business rule (32 마지막 청크만 커서·33 보고 합치기).
use std::path::Path;

use super::{Incoming, MAX_ROWS_PER_TX, PendingKind, UpsertReport, upsert_chunked};
use crate::scan::IngestError;

fn items(n: usize) -> Vec<Incoming> {
    (0..n)
        .map(|i| {
            Incoming::new(
                1,
                Path::new(&format!(r"C:\lib\f{i}.mov")),
                PendingKind::Path,
                "t",
            )
            .unwrap()
        })
        .collect()
}

fn ok(written: usize) -> Result<UpsertReport, IngestError> {
    Ok(UpsertReport {
        written,
        ..UpsertReport::default()
    })
}

#[test]
fn t_pending_30_5000_paths_are_written_as_three_transactions_of_at_most_2000_rows_and_only_the_last_is_flagged()
 {
    assert_eq!(MAX_ROWS_PER_TX, 2_000);
    let all = items(5_000);
    let mut calls = Vec::new();
    let r = upsert_chunked(&all, MAX_ROWS_PER_TX, |part, last| {
        calls.push((part.len(), last));
        ok(part.len())
    })
    .unwrap();
    assert_eq!(calls, vec![(2_000, false), (2_000, false), (1_000, true)]);
    assert_eq!(r.written, 5_000);
    // 경계: 정확히 2000 은 1 청크, 2001 은 2 청크
    let mut n = 0;
    upsert_chunked(&items(2_000), MAX_ROWS_PER_TX, |p: &[Incoming], last| {
        n += 1;
        assert!(last);
        ok(p.len())
    })
    .unwrap();
    assert_eq!(n, 1);
    n = 0;
    upsert_chunked(&items(2_001), MAX_ROWS_PER_TX, |p: &[Incoming], _| {
        n += 1;
        ok(p.len())
    })
    .unwrap();
    assert_eq!(n, 2);
}

#[test]
fn t_pending_31_an_empty_batch_still_runs_once_as_the_last_chunk_so_a_cursor_only_update_is_written()
 {
    let mut calls = Vec::new();
    upsert_chunked(&[], MAX_ROWS_PER_TX, |part, last| {
        calls.push((part.len(), last));
        ok(0)
    })
    .unwrap();
    assert_eq!(calls, vec![(0, true)]);
}

#[test]
fn t_pending_32_a_failure_in_a_middle_chunk_stops_before_the_last_chunk_so_the_cursor_never_advances()
 {
    let all = items(5_000);
    let mut seen_last = false;
    let mut calls = 0;
    let r = upsert_chunked(&all, MAX_ROWS_PER_TX, |part, last| {
        calls += 1;
        seen_last |= last;
        if calls == 2 {
            Err(IngestError::InvalidInput("injected"))
        } else {
            ok(part.len())
        }
    });
    assert!(matches!(r, Err(IngestError::InvalidInput("injected"))));
    assert_eq!(calls, 2, "no chunk after the failure");
    assert!(!seen_last, "the cursor-carrying chunk never ran");
}

#[test]
fn t_pending_33_reports_are_merged_with_overflowed_folders_deduplicated() {
    let all = items(4_001);
    let r = upsert_chunked(&all, 2_000, |part, _| {
        Ok(UpsertReport {
            written: part.len(),
            overflowed: vec![7, 9],
            dropped: 1,
        })
    })
    .unwrap();
    assert_eq!((r.written, r.dropped, r.overflowed), (4_001, 3, vec![7, 9]));
}
