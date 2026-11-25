//! 폴더 제거 뒤 늦게 끝난 작업의 결과 쓰기 회귀 (T-late-01 ~ 03).
//!
//! 분류: Concurrency (작업을 잡은 채 자산이 삭제 표시된 뒤 완료) / Business rule (삭제 표시된 자산에는 연결 행·색인 이벤트를 남기지 않는다) /
//! Boundary (활성 자산에는 평소처럼 쓴다 - 가드가 과하지 않다). 실제 SQLite(마이그레이션 적용) + 실제 작업 대기열 + 실제 `EmbedStore`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mam_db::{DbPool, Priority, SystemClock, migrate, open};
use mam_vector::{EmbedStore, ModelKey, NewVector, VectorKind, VectorSource};
use tempfile::TempDir;

use super::plan::mark_skipped;
use super::state::upsert_queued;
use super::types::StepKind;
use crate::ai_store::{
    AnalysisFields, NewAnalysis, RunRecord, TranscriptRecord, record_run, save_analysis, save_transcript,
};
use crate::hash::{Blake3Hasher, ContentHash, HashedFile};
use crate::jobs::{JobKind, JobOutcome, JobQueue, NewJob};
use crate::processing::{DerivKind, DerivState};
use crate::purge::{LINKED_TABLES, RangeSpec, purge_range_batch};
use crate::reconcile::{PendingHash, Reconciler};

struct Fx {
    _dir: TempDir,
    db: std::path::PathBuf,
    pool: Arc<DbPool>,
    queue: Arc<JobQueue>,
    store: EmbedStore,
    asset: i64,
    volume: i64,
}

fn fixture() -> Fx {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let (volume, asset) = pool
        .write(Priority::High, |c| {
            c.execute("INSERT INTO volumes (kind, volume_key, last_seen_at) VALUES ('fixed', 'v', 1)", [])?;
            let volume = c.last_insert_rowid();
            c.execute(
                "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                 VALUES ('image', 1, 1, 1, 1)",
                [],
            )?;
            let asset = c.last_insert_rowid();
            c.execute(
                "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                 VALUES (?1, ?2, 'lib/a.jpg', 'lib/a.jpg', 1)",
                [asset, volume],
            )?;
            Ok((volume, asset))
        })
        .unwrap();
    let queue = JobQueue::with_concurrency(
        Arc::clone(&pool),
        Arc::new(SystemClock),
        JobKind::ALL.map(|_| 1),
    );
    let store = EmbedStore::new(dir.path().join("vectors")).unwrap();
    Fx {
        db,
        _dir: dir,
        pool,
        queue,
        store,
        asset,
        volume,
    }
}

/// 풀 밖의 별도 연결 (쓰기 클로저의 'static 제약 없이 같은 파일에 쓴다 - 풀은 한가하다)
fn direct(fx: &Fx) -> rusqlite::Connection {
    open(&fx.db).unwrap()
}

fn key() -> ModelKey {
    ModelKey {
        kind: VectorKind::Visual,
        model_id: "m".into(),
        model_version: "1".into(),
        dimension: 4,
    }
}

fn linked_rows(fx: &Fx) -> Vec<(String, i64)> {
    let id = fx.asset;
    fx.pool
        .read(|c| {
            let mut tables: Vec<&str> = LINKED_TABLES.to_vec();
            tables.extend(["transcripts"]);
            let mut out = Vec::new();
            for t in tables {
                let n: i64 = c
                    .query_row(&format!("SELECT COUNT(*) FROM {t} WHERE asset_id = ?1"), [id], |r| r.get(0))
                    .unwrap();
                out.push((t.to_string(), n));
            }
            Ok(out)
        })
        .unwrap()
}

/// 작업이 실제로 하는 결과 쓰기를 모두 한 번씩 시도한다 (모든 경로가 삭제 표시된 자산을 건드리지 않아야 한다).
fn finish_like_a_late_job(fx: &Fx) {
    let (id, now) = (fx.asset, 100);
    let mut conn = direct(fx);
    let c = &mut conn;
    // 파생물 상태 (meta 실패 경로 등)
    crate::processing::set_state(c, now, id, DerivKind::Meta, DerivState::Failed, Some("x")).unwrap();
    // AI 단계 행·실행 이력
    upsert_queued(c, now, id, &[StepKind::Describe, StepKind::Embedding], true).unwrap();
    mark_skipped(c, now, id, "image").unwrap();
    record_run(c, &SystemClock, &run_record(id)).unwrap();
    // 장면전환점 결과
    mam_db::scene_cuts::replace_auto_tx(c, id, &[1000], "medium", None, now).unwrap();
    mam_db::scene_cuts::set_outcome_tx(c, id, mam_db::scene_cuts::SceneState::Failed, Some("other"), now).unwrap();
    // 해시 반영
    let rec = Reconciler {
        clock: &SystemClock,
        hasher: &Blake3Hasher,
        cancel: &AtomicBool::new(false),
    };
    let hashed = HashedFile {
        hash: ContentHash::from_bytes([7; 32]),
        size: 1,
        modified_ms: 1,
    };
    assert_eq!(rec.commit_pending_hash(c, id, &hashed).unwrap(), PendingHash::Deferred);
    // AI 결과 저장 (이미 활성 자산만 허용하던 경로 - 회귀 고정)
    let analysis = NewAnalysis {
        fields: AnalysisFields {
            description: Some("d".into()),
            ..AnalysisFields::default()
        },
        language: "ko".into(),
        provider: "local".into(),
        model: "m".into(),
    };
    let transcript = TranscriptRecord {
        language: "ko".into(),
        segments: Vec::new(),
        engine: "e".into(),
        model: "m".into(),
    };
    assert!(save_analysis(c, &SystemClock, id, &analysis).is_err());
    assert!(save_transcript(c, &SystemClock, id, &transcript).is_err());
    // 임베딩 저장: 건너뛰고 빈 목록
    let ids = put_vector(fx, c, now);
    assert!(ids.is_empty(), "no embedding id is issued for a deleted asset");
}

fn run_record(asset_id: i64) -> RunRecord {
    RunRecord {
        asset_id,
        kind: "describe".into(),
        provider: "local".into(),
        ok: true,
        error_code: None,
        message: String::new(),
        attempts: 1,
    }
}

fn put_vector(fx: &Fx, c: &mut rusqlite::Connection, now: i64) -> Vec<u64> {
    let v = [1.0f32, 0.0, 0.0, 0.0];
    let vectors = [NewVector {
        segment_id: None,
        source: VectorSource::Image,
        frame_index: None,
        vector: &v,
    }];
    fx.store.put(c, &key(), fx.asset, &vectors, now).unwrap()
}

fn purge_all(fx: &Fx) {
    let volume = fx.volume;
    fx.pool
        .write(Priority::Low, move |c| {
            let spec = RangeSpec { volume_id: volume, root_key: "lib", media_types: None };
            let b = purge_range_batch(c, 50, &spec, 500).unwrap();
            assert_eq!(b.purged.len(), 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_late_01_job_claimed_before_the_purge_leaves_no_rows_when_it_finishes_after() {
    // Arrange: AI 작업을 잡은 채(running) 폴더 제거로 자산이 삭제 표시된다
    let fx = fixture();
    assert!(fx.queue.enqueue(NewJob {
        kind: Some(JobKind::Ai),
        asset_id: Some(fx.asset),
        folder_id: None,
        priority: None,
        not_before: 0,
    })
    .unwrap());
    let job = fx.queue.claim_next(&[JobKind::Ai]).unwrap().unwrap();
    purge_all(&fx);
    let revision: i64 = fx
        .pool
        .read(|c| Ok(c.query_row("SELECT revision FROM media_assets", [], |r| r.get(0)).unwrap()))
        .unwrap();
    let events_before: i64 = fx
        .pool
        .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM search_outbox", [], |r| r.get(0)).unwrap()))
        .unwrap();

    // Act: 늦게 끝난 작업의 결과 쓰기 + 작업 완료
    finish_like_a_late_job(&fx);
    fx.queue.finish(&job, Ok(JobOutcome::Done)).unwrap();

    // Assert: 연결 테이블 0행, revision·색인 이벤트도 늘지 않았다 (삭제 이벤트 뒤에 upsert 가 끼어들지 않는다)
    for (table, n) in linked_rows(&fx) {
        assert_eq!(n, 0, "{table} was written for a deleted asset");
    }
    let (rev_after, events_after): (i64, i64) = fx
        .pool
        .read(|c| {
            Ok((
                c.query_row("SELECT revision FROM media_assets", [], |r| r.get(0)).unwrap(),
                c.query_row("SELECT COUNT(*) FROM search_outbox", [], |r| r.get(0)).unwrap(),
            ))
        })
        .unwrap();
    assert_eq!((rev_after, events_after), (revision, events_before));
}

#[test]
fn t_late_02_the_same_writes_still_land_for_an_active_asset() {
    // Arrange: 삭제 표시 없이 같은 쓰기를 한다 (가드가 정상 경로를 막지 않는다)
    let fx = fixture();
    let (id, now) = (fx.asset, 100);

    // Act
    let mut conn = direct(&fx);
    let c = &mut conn;
    crate::processing::set_state(c, now, id, DerivKind::Meta, DerivState::Failed, Some("x")).unwrap();
    upsert_queued(c, now, id, &[StepKind::Describe], true).unwrap();
    record_run(c, &SystemClock, &run_record(id)).unwrap();
    mam_db::scene_cuts::replace_auto_tx(c, id, &[1000], "medium", None, now).unwrap();
    let ids = put_vector(&fx, c, now);

    // Assert
    assert_eq!(ids.len(), 1);
    let rows = linked_rows(&fx);
    for table in ["asset_derivatives", "ai_task_state", "analysis_runs", "asset_scene_cuts", "embeddings"] {
        assert_eq!(rows.iter().find(|(t, _)| t == table).unwrap().1, 1, "{table}");
    }
}

#[test]
fn t_late_03_purged_embeddings_are_reported_so_the_vector_cache_can_drop_them() {
    // Arrange: 활성 자산의 벡터를 저장하고 활성 맵 캐시에 올린다
    let fx = fixture();
    let now = 1;
    put_vector(&fx, &mut direct(&fx), now);
    let cache = mam_vector::ActiveMapCache::new();
    let before = fx.pool.read(|c| Ok(cache.get_or_load(c, &fx.store, &key()).unwrap().len())).unwrap();

    // Act: 제거 + 알림 (앱은 이 키로 `notify_changed` 를 부른다)
    let volume = fx.volume;
    let batch = fx
        .pool
        .write(Priority::Low, move |c| {
            let spec = RangeSpec { volume_id: volume, root_key: "lib", media_types: None };
            Ok(purge_range_batch(c, 50, &spec, 500).unwrap())
        })
        .unwrap();
    let stale = fx.pool.read(|c| Ok(cache.get_or_load(c, &fx.store, &key()).unwrap().len())).unwrap();
    assert_eq!(batch.embedding_keys.len(), 1);
    assert_eq!(
        (batch.embedding_keys[0].kind.as_str(), batch.embedding_keys[0].dimension),
        ("visual", 4)
    );
    fx.store.notify_changed(&key());
    let after = fx.pool.read(|c| Ok(cache.get_or_load(c, &fx.store, &key()).unwrap().len())).unwrap();

    // Assert: 알리기 전에는 캐시가 지운 슬롯을 들고 있고(전제), 알린 뒤에는 사라진다
    assert_eq!((before, stale, after), (1, 1, 0));
}
