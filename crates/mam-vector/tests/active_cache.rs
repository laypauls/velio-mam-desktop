//! T-cache-01~09: 활성 맵 캐시(`ActiveMapCache`)와 `active_candidates_cached`.
//! 분류: Happy / Business rule / Boundary / Concurrency / External(리셋 통지). 실제 SQLite(마이그레이션 V6) + 임시 디렉터리.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use common::{DIM, Rng, dataset};
use mam_vector::{
    ActiveMap, ActiveMapCache, AnnIndex, EmbedStore, Generations, ModelKey, NewVector, OpenMode,
    VectorKind, VectorSource, active_candidates, active_candidates_cached, plan_candidates,
};
use rusqlite::Connection;
use tempfile::TempDir;

const ASSETS: i64 = 12;

struct Env {
    dir: TempDir,
    conn: Connection,
    store: EmbedStore,
    cache: ActiveMapCache,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let mut conn = mam_db::open(&dir.path().join("t.db")).unwrap();
    mam_db::migrate(&mut conn, &dir.path().join("backup"), &mam_db::SystemClock).unwrap();
    for a in 1..=ASSETS {
        conn.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'image', 1, 1, 1, 1)",
            [a],
        )
        .unwrap();
    }
    let store = EmbedStore::new(dir.path().join("vectors")).unwrap();
    Env {
        dir,
        conn,
        store,
        cache: ActiveMapCache::new(),
    }
}

fn key(kind: VectorKind, version: &str) -> ModelKey {
    ModelKey {
        kind,
        model_id: "m".into(),
        model_version: version.into(),
        dimension: DIM,
    }
}

fn nv(v: &[f32]) -> NewVector<'_> {
    NewVector {
        segment_id: None,
        source: VectorSource::Image,
        frame_index: None,
        vector: v,
    }
}

impl Env {
    fn put(&mut self, k: &ModelKey, asset: i64, vecs: &[Vec<f32>], now: i64) {
        let nvs: Vec<NewVector> = vecs.iter().map(|v| nv(v)).collect();
        self.store.put(&mut self.conn, k, asset, &nvs, now).unwrap();
    }

    fn map(&self, k: &ModelKey) -> Arc<ActiveMap> {
        self.cache.get_or_load(&self.conn, &self.store, k).unwrap()
    }

    fn truth(&self, k: &ModelKey) -> HashMap<u64, u64> {
        self.store.asset_map(&self.conn, k).unwrap()
    }
}

fn assert_same(map: &ActiveMap, truth: &HashMap<u64, u64>, ctx: &str) {
    assert_eq!(map.len(), truth.len(), "{ctx}: size");
    assert!(
        truth.iter().all(|(e, a)| map.asset_of(*e) == Some(*a)),
        "{ctx}: content"
    );
    assert!(map.ids().windows(2).all(|w| w[0] < w[1]), "{ctx}: sorted");
    assert!(truth.keys().all(|e| map.contains(*e)), "{ctx}: bitmap");
}

// ---------------------------------------------------------------- T-cache-01 (Happy)
#[test]
fn t_cache_01_first_load_equals_sqlite_active_rows() {
    // Arrange
    let mut e = env();
    let k = key(VectorKind::Visual, "1");
    let d = dataset(8, DIM, 1);
    for (i, v) in d.iter().enumerate() {
        e.put(&k, 1 + i as i64 % 4, std::slice::from_ref(v), 10 + i as i64);
    }
    // Act
    let map = e.map(&k);
    // Assert: 재임베딩으로 폐기된 행은 빠지고 활성 4개 자산만 남는다
    assert_same(&map, &e.truth(&k), "first load");
    assert_eq!(map.len(), 4);
    assert_eq!(map.embeddings_of_assets(&[1, 3]).len(), 2);
}

// ---------------------------------------------------------------- T-cache-02 (Concurrency: 스냅샷)
#[test]
fn t_cache_02_hit_returns_same_snapshot_and_old_snapshot_is_not_mutated() {
    let mut e = env();
    let k = key(VectorKind::Visual, "1");
    let d = dataset(3, DIM, 2);
    e.put(&k, 1, &d[..1], 1);
    let a = e.map(&k);
    let b = e.map(&k);
    assert!(Arc::ptr_eq(&a, &b), "no change -> same Arc, no reload");
    // 변경 후: 새 스냅샷은 반영, 쥐고 있던 옛 스냅샷은 그대로
    e.put(&k, 2, &d[1..2], 2);
    let c = e.map(&k);
    assert_eq!(a.len(), 1, "old snapshot untouched");
    assert_eq!(c.len(), 2);
    assert_same(&c, &e.truth(&k), "after put");
}

// ---------------------------------------------------------------- T-cache-03 (Business rule + 회귀)
#[test]
fn t_cache_03_random_puts_discards_and_backwards_clock_keep_cache_equal_to_sqlite() {
    let mut e = env();
    let k = key(VectorKind::Visual, "1");
    let d = dataset(40, DIM, 3);
    e.put(&k, 1, &d[..2], 100);
    let _ = e.map(&k);
    let mut rng = Rng(7);
    for step in 0..60 {
        let asset = 1 + (rng.next_u64() % ASSETS as u64) as i64;
        let n = (rng.next_u64() % 4) as usize;
        let now = 1_000 - (rng.next_u64() % 500) as i64; // 거꾸로 가기도 하고 같은 값이 반복되기도 한다
        let at = (rng.next_u64() % 36) as usize;
        // Act
        e.put(&k, asset, &d[at..at + n], now);
        // Assert: 매 단계 (변경마다 질의가 있는 최악 빈도)
        assert_same(&e.map(&k), &e.truth(&k), &format!("step {step}"));
    }
}

// ---------------------------------------------------------------- T-cache-04 (External: put 을 거치지 않은 변경)
#[test]
fn t_cache_04_direct_sql_changes_need_notify_changed_and_then_reload() {
    let mut e = env();
    let k = key(VectorKind::Visual, "1");
    let d = dataset(3, DIM, 4);
    for (i, v) in d.iter().enumerate() {
        e.put(&k, 1 + i as i64, std::slice::from_ref(v), 1);
    }
    assert_eq!(e.map(&k).len(), 3);
    // 자산 영구 삭제 (FK CASCADE) — put 을 거치지 않았다
    e.conn
        .execute("DELETE FROM media_assets WHERE asset_id = 2", [])
        .unwrap();
    // 통지 전에는 캐시가 모른다 (계약), 통지하면 다음 질의가 전체 재로드한다
    assert_eq!(e.map(&k).len(), 3, "cache cannot see non-put changes");
    e.store.notify_changed(&k);
    let map = e.map(&k);
    assert_same(&map, &e.truth(&k), "after notify");
    assert_eq!(map.len(), 2);
    // invalidate 도 같은 효과
    e.conn
        .execute("DELETE FROM media_assets WHERE asset_id = 3", [])
        .unwrap();
    e.cache.invalidate(&e.store, &k);
    assert_eq!(e.map(&k).len(), 1);
}

// ---------------------------------------------------------------- T-cache-05 (Business rule: 세대 교체)
#[test]
fn t_cache_05_generation_change_forces_full_reload() {
    let mut e = env();
    let k = key(VectorKind::Visual, "1");
    let d = dataset(3, DIM, 5);
    for (i, v) in d.iter().enumerate() {
        e.put(&k, 1 + i as i64, std::slice::from_ref(v), 1);
    }
    let g = Generations::new(&e.store, &k).unwrap();
    g.rebuild(&e.conn, &e.store, &k).unwrap();
    let before = e.map(&k);
    // 통지 없이 SQL 로 변경 -> 세대가 같으면 모른다
    e.conn
        .execute("DELETE FROM media_assets WHERE asset_id = 1", [])
        .unwrap();
    assert!(Arc::ptr_eq(&before, &e.map(&k)));
    // 재구축으로 세대 이름이 바뀌면 전체 재로드 (안전망)
    g.rebuild(&e.conn, &e.store, &k).unwrap();
    let after = e.map(&k);
    assert!(!Arc::ptr_eq(&before, &after));
    assert_same(&after, &e.truth(&k), "after rebuild");
    assert_eq!(after.len(), 2);
}

// ---------------------------------------------------------------- T-cache-06 (Boundary: 키 분리·빈 상태)
#[test]
fn t_cache_06_keys_are_isolated_and_empty_store_gives_empty_map() {
    let mut e = env();
    let (v1, v2, sem) = (
        key(VectorKind::Visual, "1"),
        key(VectorKind::Visual, "2"),
        key(VectorKind::Semantic, "1"),
    );
    assert!(e.map(&v1).is_empty());
    let d = dataset(3, DIM, 6);
    e.put(&v1, 1, &d[..1], 1);
    e.put(&v2, 2, &d[1..2], 1);
    e.put(&sem, 3, &d[2..3], 1);
    for k in [&v1, &v2, &sem] {
        let m = e.map(k);
        assert_eq!(m.len(), 1);
        assert_same(&m, &e.truth(k), "isolated");
    }
    // v2 만 바꿔도 v1 스냅샷은 그대로 (같은 Arc)
    let a = e.map(&v1);
    e.put(&v2, 2, &d[..1], 2);
    assert!(Arc::ptr_eq(&a, &e.map(&v1)));
}

// ---------------------------------------------------------------- T-cache-07 (Happy + Business rule: 후보)
#[test]
fn t_cache_07_cached_candidates_equal_uncached_for_none_and_allowed_and_skip_superseded() {
    // Arrange: 자산 1..8 벡터 적재 + 색인, 그 뒤 자산 2 를 재임베딩(옛 벡터는 인덱스에 남아 있지만 폐기)
    let mut e = env();
    let k = key(VectorKind::Visual, "1");
    let d = dataset(12, DIM, 7);
    for (i, v) in d.iter().take(8).enumerate() {
        e.put(&k, 1 + i as i64, std::slice::from_ref(v), 1);
    }
    let g = Generations::new(&e.store, &k).unwrap();
    g.rebuild(&e.conn, &e.store, &k).unwrap();
    e.put(&k, 2, &d[8..9], 2);
    let index = g.open_active(&k, OpenMode::View).unwrap().unwrap();
    let plan = plan_candidates(5);
    let allowed_assets = [1u64, 2, 5];
    let allowed = e.map(&k).embeddings_of_assets(&allowed_assets);
    for q in [&d[0], &d[8], &d[10]] {
        for allow in [None, Some(allowed.as_slice())] {
            // Act
            let old = active_candidates(&e.conn, &e.store, &k, &index, q, &plan, allow).unwrap();
            let new =
                active_candidates_cached(&e.conn, &e.store, &e.cache, &k, &index, q, &plan, allow)
                    .unwrap();
            // Assert: 같은 자산·순서 (점수 차는 ulp 수준)
            let ids = |h: &[mam_vector::AssetHit]| h.iter().map(|x| x.asset_id).collect::<Vec<_>>();
            assert_eq!(ids(&old), ids(&new), "allow={}", allow.is_some());
            if allow.is_some() {
                assert!(new.iter().all(|h| allowed_assets.contains(&h.asset_id)));
            }
        }
    }
    // 폐기된 옛 벡터(자산 2 의 첫 임베딩)는 인덱스에 남아 있어도 후보가 아니다 (새 벡터는 아직 색인 전이라 자산 2 는 없다)
    let all = active_candidates_cached(&e.conn, &e.store, &e.cache, &k, &index, &d[1], &plan, None)
        .unwrap();
    assert_eq!(all.iter().filter(|h| h.asset_id == 2).count(), 0);
    assert_eq!(
        index.len(),
        8,
        "stale index still has the superseded vector"
    );
}

// ---------------------------------------------------------------- T-cache-08 (Concurrency)
#[test]
fn t_cache_08_readers_during_writes_never_fail_and_converge_to_sqlite() {
    // Arrange
    let mut e = env();
    let k = key(VectorKind::Visual, "1");
    let d = dataset(30, DIM, 8);
    e.put(&k, 1, &d[..1], 1);
    let db = e.dir.path().join("t.db");
    let (store, cache) = (&e.store, &e.cache);
    let done = std::sync::atomic::AtomicBool::new(false);
    // Act: 읽기 스레드 3개(자기 연결)가 계속 get_or_load 하는 동안 쓰기 스레드가 put
    std::thread::scope(|s| {
        for _ in 0..3 {
            s.spawn(|| {
                let conn = mam_db::open(&db).unwrap();
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    let m = cache.get_or_load(&conn, store, &k).unwrap();
                    assert!(m.ids().windows(2).all(|w| w[0] < w[1]));
                }
            });
        }
        for (i, v) in d.iter().take(25).enumerate() {
            let nvs = [nv(v)];
            store
                .put(&mut e.conn, &k, 1 + (i as i64 % ASSETS), &nvs, i as i64)
                .unwrap();
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    // Assert
    assert_same(&e.map(&k), &e.truth(&k), "converged");
}
