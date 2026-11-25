//! 명시적 재구축 요청 (백업 복원·DB 자동 복구, 요청 20261107-123710 H3). T-rebuildreq-01.
//! 분류: Business rule(문서 수가 같아도 내용이 달라지면 요청으로 재구축) / Boundary(요청 없으면 허용 오차 안에서 재구축 안 함) /
//! Concurrency(재구축 성공 뒤에만 표지 삭제 — 멱등). 실제 SQLite + 임시 디렉터리 Tantivy.

mod common;
#[path = "common/resilience.rs"]
mod res;

use mam_search::sync::{
    RebuildReason, SyncConfig, SyncWorker, rebuild_request_path, request_rebuild,
};
use res::*;

#[test]
fn t_rebuildreq_01_requested_rebuild_replaces_stale_content_even_when_doc_count_matches() {
    // Arrange: 3건 색인 -> 종료 -> 인덱스 모르게 SQLite 의 파일명만 바뀐다 (복원된 DB 모사: 문서 수 동일)
    let env = Env::new();
    for name in ["old_a.jpg", "old_b.jpg", "old_c.jpg"] {
        add_asset(&env.conn, 1, name, 0);
    }
    let w = env.worker();
    w.drain(&env.conn, &LATE).unwrap();
    drop(w);
    env.conn
        .execute(
            "UPDATE file_locations SET rel_path = 'new_a.jpg', path_key = 'new_a.jpg' WHERE rel_path = 'old_a.jpg'",
            [],
        )
        .unwrap();
    env.conn
        .execute(
            "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
             SELECT asset_id, revision + 1, 'upsert', 0, 0 FROM media_assets WHERE asset_id = 1",
            [],
        )
        .unwrap();
    env.conn
        .execute("UPDATE search_outbox SET status = 'done'", [])
        .unwrap();

    // Act 1: 요청 없음 -> 문서 수가 같아 허용 오차 안이라 재구축하지 않는다 (옛 내용이 남는다)
    let (w, outcome) =
        SyncWorker::open(&env.conn, &env.index_dir(), opts(), SyncConfig::default()).unwrap();
    assert!(outcome.rebuilt.is_none());
    assert_eq!(find(w.index().unwrap(), "old_a"), vec![1]);
    drop(w);

    // Act 2: 요청 -> 재구축, 표지는 재구축 뒤에 지워진다
    request_rebuild(&env.index_dir()).unwrap();
    assert!(rebuild_request_path(&env.index_dir()).is_file());
    let (w, outcome) =
        SyncWorker::open(&env.conn, &env.index_dir(), opts(), SyncConfig::default()).unwrap();

    // Assert
    let report = outcome.rebuilt.expect("rebuild requested");
    assert_eq!(report.reason, RebuildReason::Requested);
    assert!(!rebuild_request_path(&env.index_dir()).exists());
    assert!(find(w.index().unwrap(), "old_a").is_empty());
    assert_eq!(find(w.index().unwrap(), "new_a"), vec![1]);
    drop(w);
    let (_w, again) =
        SyncWorker::open(&env.conn, &env.index_dir(), opts(), SyncConfig::default()).unwrap();
    assert!(again.rebuilt.is_none(), "request is consumed once");
}
