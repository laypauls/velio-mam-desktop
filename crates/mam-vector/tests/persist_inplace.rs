//! 활성 세대 증분 저장은 열려 있을 수 있는 파일을 바꾸지 않는다 (P4 부하 측정 결함: 5초 저장이 메타를 지우고
//! 데이터를 덮어써 동시 검색의 9.7%(10만)~22%(100만)가 키워드로 강등).
//! 분류: T-persist-01 Concurrency / 02 External(크래시) / 03 Boundary(정리 상한) / 04 Business rule / 05 Validation.
//! 실제 SQLite + 임시 디렉터리 인덱스.

mod common;
#[path = "common/resilience.rs"]
mod res;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{affinity, dataset};
use mam_vector::{
    AnnIndex, EmbedStore, Generations, ModelKey, OpenMode, SearchParams, VectorError,
    data_file_versions, sync_incremental,
};
use res::*;

const TOTAL: usize = 160;
const FIRST: usize = 20;
const STEP: usize = 8;

/// 자산 `from..to` 에 벡터를 저장한다 (embedding_id == 자산 id).
fn put_range(e: &mut Env, k: &ModelKey, d: &[Vec<f32>], from: usize, to: usize) {
    for (i, v) in d.iter().enumerate().take(to).skip(from) {
        e.store
            .put(&mut e.conn, k, i as i64 + 1, &[nv(v)], NOW)
            .unwrap();
    }
}

/// 초기 FIRST 건으로 첫 세대 g1 을 만든 환경.
fn start() -> (Env, ModelKey, Vec<Vec<f32>>) {
    affinity::exclude_inconsistent_cores();
    let mut e = env(TOTAL as i64);
    let k = key("1");
    let d = dataset(TOTAL, DIM, 7);
    put_range(&mut e, &k, &d, 0, FIRST);
    let (idx, rep) = open_or_rebuild(&e, &k);
    assert!(rep.is_some());
    drop(idx);
    (e, k, d)
}

/// 추가분을 더해 저장한다 (앱 AI 틱과 같은 순서).
fn append_and_persist(e: &Env, k: &ModelKey) -> u64 {
    let g = Generations::new(&e.store, k).unwrap();
    let (idx, _) = g
        .open_or_rebuild(&e.conn, &e.store, k, OpenMode::Load)
        .unwrap();
    let r = sync_incremental(&e.conn, &e.store, k, &idx).unwrap();
    g.persist(k, &idx).unwrap();
    r.read
}

fn gen_dirs(e: &Env, k: &ModelKey) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(e.store.key_dir(k).join("gen"))
        .unwrap()
        .flatten()
        .filter(|x| x.path().is_dir())
        .map(|x| x.file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

#[test]
fn t_persist_01_reader_never_sees_missing_meta_or_torn_data_while_writer_persists() {
    // Arrange
    let (mut e, k, d) = start();
    let committed: Arc<Mutex<BTreeSet<usize>>> = Arc::new(Mutex::new(BTreeSet::from([FIRST])));
    let stop = Arc::new(AtomicBool::new(false));
    let root = e.dir.path().join("vectors");
    let (kr, dr, cr, sr) = (k.clone(), d.clone(), committed.clone(), stop.clone());
    let reader = std::thread::spawn(move || {
        let store = EmbedStore::new(root).unwrap();
        let g = Generations::new(&store, &kr).unwrap();
        let (mut opens, mut errors, mut bad) = (0u32, Vec::new(), Vec::new());
        let mut held = None; // 이전 매핑을 한 번 더 붙잡아, 쓰는 쪽이 옛 버전을 지우려는 상황을 만든다
        while !sr.load(Ordering::Relaxed) {
            match g.open_active(&kr, OpenMode::View) {
                Ok(Some(idx)) => {
                    opens += 1;
                    let n = idx.len();
                    // 인덱스는 id 1..=n 을 담아야 한다 (커밋된 상태 중 하나)
                    let top = idx
                        .search(&dr[n - 1], 1, &SearchParams::default(), None)
                        .unwrap();
                    if top.first().map(|h| h.id) != Some(n as u64) {
                        bad.push(format!("n={n} top={top:?}"));
                    }
                    if !cr.lock().unwrap().contains(&n) {
                        bad.push(format!("n={n} not a committed state"));
                    }
                    held = Some(idx);
                }
                Ok(None) => errors.push("no active".to_string()),
                Err(err) => errors.push(err.to_string()),
            }
        }
        drop(held);
        (opens, errors, bad)
    });

    // Act: 쓰는 쪽이 N 번 추가+저장
    let mut n = FIRST;
    for _ in 0..12 {
        let next = (n + STEP).min(TOTAL);
        put_range(&mut e, &k, &d, n, next);
        committed.lock().unwrap().insert(next); // 저장 공개 전에 커밋 집합에 넣는다
        assert_eq!(append_and_persist(&e, &k), (next - n) as u64);
        n = next;
    }
    stop.store(true, Ordering::Relaxed);
    let (opens, errors, bad) = reader.join().unwrap();

    // Assert
    assert!(opens > 20, "reader made progress: {opens}");
    assert!(errors.is_empty(), "open errors: {errors:?}");
    assert!(bad.is_empty(), "inconsistent results: {bad:?}");
    assert_eq!(
        gen_dirs(&e, &k),
        vec!["g1"],
        "one generation, name unchanged"
    );
    let v = data_file_versions(&gen_dir(&e, &k, "g1"));
    assert!(v.len() <= 2, "{v:?}");
}

#[test]
fn t_persist_02_crash_before_pointer_switch_keeps_previous_version_active() {
    // Arrange: 한 번 저장해 버전 1 을 만든 뒤, 데이터는 썼지만 포인터는 못 바꾼 흔적을 남긴다
    let (mut e, k, d) = start();
    put_range(&mut e, &k, &d, FIRST, FIRST + STEP);
    append_and_persist(&e, &k);
    let dir = gen_dir(&e, &k, "g1");
    let before = data_file_versions(&dir);
    let good = std::fs::read(dir.join("index.1.flat")).unwrap();
    std::fs::write(dir.join("index.2.flat"), &good[..good.len() / 2]).unwrap(); // 잘린 고아
    std::fs::write(dir.join("index.3.tmp"), b"partial").unwrap();

    // Act
    let g = Generations::new(&e.store, &k).unwrap();
    let idx = g.open_active(&k, OpenMode::View).unwrap().unwrap();

    // Assert: 이전 버전이 그대로 쓸 수 있다
    assert_eq!(idx.len(), FIRST + STEP);
    assert_eq!(before, vec![0, 1]);
    drop(idx);

    // 다음 저장이 고아를 지우고 번호를 건너뛰어 새로 쓴다
    put_range(&mut e, &k, &d, FIRST + STEP, FIRST + 2 * STEP);
    append_and_persist(&e, &k);
    let idx = g.open_active(&k, OpenMode::View).unwrap().unwrap();
    assert_eq!(idx.len(), FIRST + 2 * STEP);
    let after = data_file_versions(&dir);
    assert!(after.len() <= 2 && !after.contains(&2), "{after:?}");
    assert!(!dir.join("index.3.tmp").exists());
}

#[test]
fn t_persist_02b_corrupt_pointer_is_rebuilt_not_fatal() {
    // Arrange
    let (e, k, _d) = start();
    let dir = gen_dir(&e, &k, "g1");
    std::fs::write(dir.join("DATA"), b"not-a-number").unwrap();
    let g = Generations::new(&e.store, &k).unwrap();

    // Act / Assert
    assert!(matches!(
        g.open_active(&k, OpenMode::View),
        Err(VectorError::Corrupt(_))
    ));
    let (idx, rep) = g
        .open_or_rebuild(&e.conn, &e.store, &k, OpenMode::Load)
        .unwrap();
    assert!(rep.is_some());
    assert_eq!(idx.len(), FIRST);
}

#[test]
fn t_persist_03_old_files_and_generations_stay_bounded() {
    // Arrange: g1 을 View 로 붙잡은 채 재구축(g2) -> Windows 는 g1 삭제 실패, 다른 OS 는 삭제
    let (mut e, k, d) = start();
    let g = Generations::new(&e.store, &k).unwrap();
    let held = g.open_active(&k, OpenMode::View).unwrap().unwrap();
    g.rebuild(&e.conn, &e.store, &k).unwrap();
    assert_eq!(g.active_name().unwrap().as_deref(), Some("g2"));

    // Act: 보유 해제 뒤 여러 번 저장
    drop(held);
    for r in 0..6 {
        put_range(&mut e, &k, &d, FIRST + r * 2, FIRST + r * 2 + 2);
        append_and_persist(&e, &k);
    }

    // Assert: 남은 세대·파일이 상한 안
    assert_eq!(gen_dirs(&e, &k), vec!["g2"]);
    let v = data_file_versions(&gen_dir(&e, &k, "g2"));
    assert_eq!(v.len(), 2, "active + previous only: {v:?}");
    assert_eq!(*v.last().unwrap(), 6);
}

#[test]
fn t_persist_04_open_view_survives_persist_and_name_is_stable_for_cache() {
    // Arrange
    let (mut e, k, d) = start();
    let g = Generations::new(&e.store, &k).unwrap();
    let old_view = g.open_active(&k, OpenMode::View).unwrap().unwrap();
    let name = g.active_name().unwrap();

    // Act: 두 번 저장 (옛 버전이 지워질 차례여도 매핑 중이면 남는다)
    for r in 0..2 {
        put_range(&mut e, &k, &d, FIRST + r * STEP, FIRST + (r + 1) * STEP);
        append_and_persist(&e, &k);
    }

    // Assert
    assert_eq!(old_view.len(), FIRST);
    let top = old_view
        .search(&d[FIRST - 1], 1, &SearchParams::default(), None)
        .unwrap();
    assert_eq!(top[0].id, FIRST as u64);
    assert_eq!(
        g.active_name().unwrap(),
        name,
        "ACTIVE pointer unchanged -> no map reload"
    );
    let fresh = g.open_active(&k, OpenMode::View).unwrap().unwrap();
    assert_eq!(fresh.len(), FIRST + 2 * STEP);
}

#[test]
fn t_persist_05_validation_missing_meta_and_throttle() {
    // Arrange
    let (e, k, _d) = start();
    let g = Generations::new(&e.store, &k).unwrap();
    let (idx, _) = g
        .open_or_rebuild(&e.conn, &e.store, &k, OpenMode::Load)
        .unwrap();
    let dir = gen_dir(&e, &k, "g1");

    // Act / Assert: 방금 만든 파일은 긴 주기에 걸려 저장하지 않고, 0 이면 저장한다
    assert!(
        !g.persist_if_older(&k, &idx, Duration::from_secs(3600))
            .unwrap()
    );
    assert_eq!(data_file_versions(&dir), vec![0]);
    assert!(g.persist_if_older(&k, &idx, Duration::ZERO).unwrap());
    assert_eq!(data_file_versions(&dir), vec![0, 1]);

    // 메타가 없는 세대에는 버전을 쓰지 않는다
    std::fs::remove_file(dir.join("mam-vector.meta")).unwrap();
    assert!(matches!(
        g.persist(&k, &idx),
        Err(VectorError::IndexMismatch(_))
    ));
    assert_eq!(data_file_versions(&dir), vec![0, 1]);
}
