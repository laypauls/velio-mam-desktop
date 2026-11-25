//! P3-5 원시 임베딩 저장(T-emb-01~04)과 인덱스 세대(T-emb-05~08).
//! 분류: Happy / Business rule / External / Boundary / Concurrency. 실제 SQLite(마이그레이션 V6) + 임시 디렉터리.

mod common;

use std::path::Path;

use common::{DIM, dataset, naive_topk};
use mam_vector::{
    AnnIndex, EmbedStore, Generations, ModelKey, NewVector, OpenMode, VectorError, VectorKind,
    VectorSource, active_candidates, normalized, plan_candidates, sync_incremental,
};
use rusqlite::Connection;
use tempfile::TempDir;

const NOW: i64 = 1_700_000_000_000;

struct Env {
    _dir: TempDir,
    conn: Connection,
    store: EmbedStore,
}

fn env(assets: i64) -> Env {
    let dir = TempDir::new().unwrap();
    let mut conn = mam_db::open(&dir.path().join("t.db")).unwrap();
    mam_db::migrate(&mut conn, &dir.path().join("backup"), &mam_db::SystemClock).unwrap();
    add_assets(&conn, 1..=assets);
    let store = EmbedStore::new(dir.path().join("vectors")).unwrap();
    Env {
        _dir: dir,
        conn,
        store,
    }
}

fn add_assets(conn: &Connection, ids: impl Iterator<Item = i64>) {
    for a in ids {
        conn.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'image', 1, 1, 1, 1)",
            [a],
        )
        .unwrap();
    }
}

fn key(version: &str) -> ModelKey {
    ModelKey {
        kind: VectorKind::Visual,
        model_id: "clip-test".into(),
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

/// 자산 i+1 에 벡터 하나씩 (embedding_id = i+1).
fn put_one_each(e: &mut Env, k: &ModelKey, data: &[Vec<f32>], first_asset: i64) {
    for (i, v) in data.iter().enumerate() {
        e.store
            .put(&mut e.conn, k, first_asset + i as i64, &[nv(v)], NOW)
            .unwrap();
    }
}

fn read_all(e: &Env, k: &ModelKey) -> Vec<(u64, Vec<f32>)> {
    let mut out = Vec::new();
    e.store
        .for_each_active(
            &e.conn,
            k,
            |_| true,
            |id, v| {
                out.push((id, v.to_vec()));
                Ok(())
            },
        )
        .unwrap();
    out
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

// ------------------------------------------------------------ T-emb-01 (Happy)
#[test]
fn t_emb_01_put_stores_rows_offsets_and_f16_vectors() {
    // Arrange
    let mut e = env(1);
    let k = key("1");
    let d = dataset(3, DIM, 1);
    let vs = [
        NewVector {
            segment_id: Some(7),
            source: VectorSource::Frame,
            frame_index: Some(0),
            vector: &d[0],
        },
        NewVector {
            segment_id: Some(7),
            source: VectorSource::Frame,
            frame_index: Some(1),
            vector: &d[1],
        },
        NewVector {
            segment_id: Some(7),
            source: VectorSource::Frame,
            frame_index: Some(2),
            vector: &d[2],
        },
    ];
    // Act
    let ids = e.store.put(&mut e.conn, &k, 1, &vs, NOW).unwrap();
    // Assert
    assert_eq!(ids, vec![1, 2, 3]);
    let offs: Vec<i64> = e
        .conn
        .prepare("SELECT raw_offset FROM embeddings ORDER BY embedding_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let rl = DIM as i64 * 2;
    assert_eq!(offs, vec![0, rl, 2 * rl]);
    let (kind, src, model, dim): (String, String, String, i64) = e
        .conn
        .query_row(
            "SELECT vector_kind, source, model_id, dimension FROM embeddings WHERE embedding_id = 2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (kind.as_str(), src.as_str(), model.as_str(), dim),
        ("visual", "frame", "clip-test", DIM as i64)
    );
    let got = read_all(&e, &k);
    assert_eq!(got.len(), 3);
    for ((id, v), orig) in got.iter().zip(&d) {
        let want = normalized(orig);
        let err = v
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(err < 1e-3, "id {id} f16 error {err}");
    }
}

// ------------------------------------------------------------ T-emb-02 (Business rule)
#[test]
fn t_emb_02_reembed_supersedes_old_rows_only_for_same_asset_kind_model() {
    let mut e = env(2);
    let k = key("1");
    let d = dataset(6, DIM, 2);
    e.store
        .put(&mut e.conn, &k, 1, &[nv(&d[0]), nv(&d[1])], NOW)
        .unwrap(); // 1,2
    e.store.put(&mut e.conn, &k, 2, &[nv(&d[2])], NOW).unwrap(); // 3
    let k2 = key("2"); // 다른 모델 버전 -> 영향 없어야 한다
    e.store.put(&mut e.conn, &k2, 1, &[nv(&d[3])], NOW).unwrap(); // 4

    // Act: 자산 1 재임베딩
    let later = NOW + 1000;
    let ids = e
        .store
        .put(&mut e.conn, &k, 1, &[nv(&d[4])], later)
        .unwrap();

    // Assert
    assert_eq!(ids, vec![5]);
    assert_eq!(e.store.active_ids(&e.conn, &k).unwrap(), vec![3, 5]);
    assert_eq!(e.store.active_ids(&e.conn, &k2).unwrap(), vec![4]);
    let sup: Vec<Option<i64>> = (1..=5)
        .map(|id| {
            e.conn
                .query_row(
                    "SELECT superseded_at FROM embeddings WHERE embedding_id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
        })
        .collect();
    assert_eq!(sup, vec![Some(later), Some(later), None, None, None]);
    // 빈 목록 = 벡터만 폐기
    e.store.put(&mut e.conn, &k, 2, &[], later).unwrap();
    assert_eq!(e.store.active_ids(&e.conn, &k).unwrap(), vec![5]);
}

// ------------------------------------------------------------ T-emb-03 (External)
#[test]
fn t_emb_03a_raw_write_failure_leaves_sqlite_untouched() {
    let mut e = env(1);
    let k = key("1");
    let d = dataset(1, DIM, 3);
    // raw.f16 자리에 디렉터리를 만들어 열기를 실패시킨다
    std::fs::create_dir_all(e.store.raw_path(&k)).unwrap();

    let r = e.store.put(&mut e.conn, &k, 1, &[nv(&d[0])], NOW);

    assert!(matches!(r, Err(VectorError::Io(_))), "{r:?}");
    assert_eq!(count(&e.conn, "SELECT COUNT(*) FROM embeddings"), 0);
}

#[test]
fn t_emb_03b_sqlite_failure_leaves_only_orphan_raw_tail() {
    let mut e = env(1);
    let k = key("1");
    let d = dataset(3, DIM, 4);
    e.store.put(&mut e.conn, &k, 1, &[nv(&d[0])], NOW).unwrap();

    // 존재하지 않는 자산(FK 위반) -> SQLite 실패, raw 에는 고아 꼬리가 남는다
    let r = e.store.put(&mut e.conn, &k, 999, &[nv(&d[1])], NOW);
    assert!(r.is_err());
    let raw_len = std::fs::metadata(e.store.raw_path(&k)).unwrap().len();
    assert_eq!(raw_len, DIM as u64 * 4);
    assert_eq!(e.store.active_ids(&e.conn, &k).unwrap(), vec![1]);
    assert_eq!(read_all(&e, &k).len(), 1);

    // 이후 정상 쓰기는 고아 꼬리 뒤에 이어 붙고 읽기에 영향이 없다
    let ids = e.store.put(&mut e.conn, &k, 1, &[nv(&d[2])], NOW).unwrap();
    let got = read_all(&e, &k);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, ids[0]);
    let want = normalized(&d[2]);
    assert!(
        got[0]
            .1
            .iter()
            .zip(&want)
            .all(|(a, b)| (a - b).abs() < 1e-3)
    );
}

// ------------------------------------------------------------ T-emb-04 (Boundary)
#[test]
fn t_emb_04_ids_never_reused_and_bad_vectors_rejected() {
    let mut e = env(2);
    let k = key("1");
    let d = dataset(3, DIM, 5);
    e.store.put(&mut e.conn, &k, 2, &[nv(&d[0])], NOW).unwrap(); // id 1
    // 가장 큰 id 의 행을 지운 뒤에도 id 는 재사용되지 않는다 (AUTOINCREMENT)
    e.conn
        .execute("DELETE FROM media_assets WHERE asset_id = 2", [])
        .unwrap();
    assert_eq!(
        count(&e.conn, "SELECT COUNT(*) FROM embeddings"),
        0,
        "ON DELETE CASCADE"
    );
    let ids = e.store.put(&mut e.conn, &k, 1, &[nv(&d[1])], NOW).unwrap();
    assert_eq!(ids, vec![2]);

    // 차원 불일치 / NaN / 영벡터 -> 파일·DB 무변경
    let before = std::fs::metadata(e.store.raw_path(&k)).unwrap().len();
    let short = vec![0.5f32; DIM - 1];
    let mut nan = d[2].clone();
    nan[0] = f32::NAN;
    let zero = vec![0f32; DIM];
    for bad in [&short, &nan, &zero] {
        let r = e.store.put(&mut e.conn, &k, 1, &[nv(bad)], NOW);
        assert!(
            matches!(
                r,
                Err(VectorError::DimensionMismatch { .. } | VectorError::InvalidInput(_))
            ),
            "{r:?}"
        );
    }
    assert_eq!(
        std::fs::metadata(e.store.raw_path(&k)).unwrap().len(),
        before
    );
    assert_eq!(e.store.active_ids(&e.conn, &k).unwrap(), vec![2]);
}

// ------------------------------------------------------------ T-emb-05 (Happy)
#[test]
fn t_emb_05_generation_search_matches_exact_scan() {
    let mut e = env(200);
    let k = key("1");
    let d = dataset(200, DIM, 6);
    put_one_each(&mut e, &k, &d, 1);
    let gens = Generations::new(&e.store, &k).unwrap();

    let (idx, rebuilt) = gens
        .open_or_rebuild(&e.conn, &e.store, &k, OpenMode::Load)
        .unwrap();

    let rep = rebuilt.expect("첫 열기는 구축");
    assert_eq!((rep.generation.as_str(), rep.vectors), ("g1", 200));
    assert_eq!(gens.active_name().unwrap().as_deref(), Some("g1"));
    // 두 번째 열기는 재구축 없이 저장된 세대를 연다
    let (_, again) = gens
        .open_or_rebuild(&e.conn, &e.store, &k, OpenMode::View)
        .unwrap();
    assert!(again.is_none());
    for qi in [3usize, 77, 150] {
        let hits = active_candidates(
            &e.conn,
            &e.store,
            &k,
            &idx,
            &d[qi],
            &plan_candidates(10),
            None,
        )
        .unwrap();
        let got: Vec<u64> = hits.iter().take(10).map(|h| h.asset_id).collect();
        let truth = naive_topk(&d, &d[qi], 10);
        assert_eq!(got[0], truth[0], "top-1 q{qi}");
        assert!(
            common::recall(&got, &truth) >= 0.8,
            "q{qi} {got:?} vs {truth:?}"
        );
    }
}

// ------------------------------------------------------------ T-emb-06 (Business rule)
#[test]
fn t_emb_06_model_swap_builds_separate_generation_without_mixing() {
    let mut e = env(40);
    let (k1, k2) = (key("1"), key("2"));
    let d1 = dataset(40, DIM, 7);
    let d2 = dataset(10, DIM, 8);
    put_one_each(&mut e, &k1, &d1, 1);
    let g1 = Generations::new(&e.store, &k1).unwrap();
    g1.open_or_rebuild(&e.conn, &e.store, &k1, OpenMode::Load)
        .unwrap();

    // 새 모델(버전 2)로 일부 자산만 재임베딩 -> 별도 디렉터리·세대
    put_one_each(&mut e, &k2, &d2, 1);
    let g2 = Generations::new(&e.store, &k2).unwrap();
    let (i2, rep) = g2
        .open_or_rebuild(&e.conn, &e.store, &k2, OpenMode::Load)
        .unwrap();

    assert_ne!(e.store.key_dir(&k1), e.store.key_dir(&k2));
    assert_eq!(rep.unwrap().vectors, 10);
    assert_eq!(i2.len(), 10, "옛 모델 벡터가 섞이지 않는다");
    assert_eq!(e.store.active_ids(&e.conn, &k1).unwrap().len(), 40);
    assert_eq!(e.store.active_ids(&e.conn, &k2).unwrap().len(), 10);
    // 옛 모델 인덱스는 교체 전까지 그대로 쓸 수 있다
    let (i1, none) = g1
        .open_or_rebuild(&e.conn, &e.store, &k1, OpenMode::Load)
        .unwrap();
    assert!(none.is_none());
    assert_eq!(i1.len(), 40);
}

// ------------------------------------------------------------ T-emb-07 (External)
fn rebuild_expect(e: &Env, gens: &Generations, k: &ModelKey, n: usize, gen_name: &str) {
    let (idx, rep) = gens
        .open_or_rebuild(&e.conn, &e.store, k, OpenMode::Load)
        .unwrap();
    let rep = rep.expect("재구축되어야 한다");
    assert_eq!((rep.generation.as_str(), idx.len()), (gen_name, n));
    assert!(rep.skipped.is_empty());
}

fn gen_dir(e: &Env, k: &ModelKey, name: &str) -> std::path::PathBuf {
    e.store.key_dir(k).join("gen").join(name)
}

fn data_file(dir: &Path) -> std::path::PathBuf {
    dir.join("index.flat")
}

#[test]
fn t_emb_07a_corrupt_or_mismatched_index_is_rebuilt_from_sqlite_and_raw() {
    let mut e = env(50);
    let k = key("1");
    put_one_each(&mut e, &k, &dataset(50, DIM, 9), 1);
    let gens = Generations::new(&e.store, &k).unwrap();
    rebuild_expect(&e, &gens, &k, 50, "g1");

    // 데이터 파일 잘림 -> Corrupt -> 재구축
    let f = data_file(&gen_dir(&e, &k, "g1"));
    let bytes = std::fs::read(&f).unwrap();
    std::fs::write(&f, &bytes[..bytes.len() / 2]).unwrap();
    rebuild_expect(&e, &gens, &k, 50, "g2");
    assert!(!gen_dir(&e, &k, "g1").exists(), "옛 세대 정리");

    // 메타 변조(모델 버전) -> IndexMismatch -> 재구축
    let meta = gen_dir(&e, &k, "g2").join("mam-vector.meta");
    let text = std::fs::read_to_string(&meta)
        .unwrap()
        .replace("model_version=1", "model_version=9");
    std::fs::write(&meta, text).unwrap();
    assert!(matches!(
        gens.open_active(&k, OpenMode::Load),
        Err(VectorError::IndexMismatch(_))
    ));
    rebuild_expect(&e, &gens, &k, 50, "g3");

    // 세대 디렉터리 통째 삭제
    std::fs::remove_dir_all(gen_dir(&e, &k, "g3")).unwrap();
    rebuild_expect(&e, &gens, &k, 50, "g1"); // 디렉터리가 모두 사라졌으므로 번호는 다시 시작
}

#[test]
fn t_emb_07b_damaged_raw_record_is_skipped_and_reported() {
    let mut e = env(10);
    let k = key("1");
    put_one_each(&mut e, &k, &dataset(10, DIM, 10), 1);
    // raw 파일 꼬리 2개 레코드 손실
    let raw = e.store.raw_path(&k);
    let len = std::fs::metadata(&raw).unwrap().len();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&raw)
        .unwrap()
        .set_len(len - 2 * DIM as u64 * 2)
        .unwrap();
    let gens = Generations::new(&e.store, &k).unwrap();

    let rep = gens.rebuild(&e.conn, &e.store, &k).unwrap();

    assert_eq!(rep.vectors, 8);
    assert_eq!(rep.skipped, vec![9, 10]);
}

// ------------------------------------------------------------ T-emb-08 (Concurrency)
#[test]
fn t_emb_08_incremental_add_while_searching_and_resume_after_restart() {
    let mut e = env(250);
    let k = key("1");
    let d = dataset(250, DIM, 11);
    put_one_each(&mut e, &k, &d[..100], 1);
    let gens = Generations::new(&e.store, &k).unwrap();
    let (idx, _) = gens
        .open_or_rebuild(&e.conn, &e.store, &k, OpenMode::Load)
        .unwrap();
    gens.persist(&k, &idx).unwrap();
    put_one_each(&mut e, &k, &d[100..200], 101);

    // 증분 추가 중 검색 (다른 스레드는 별도 연결)
    let db_path = e.conn.path().map(std::path::PathBuf::from).unwrap();
    let added = std::thread::scope(|s| {
        let h = s.spawn(|| {
            let conn = mam_db::open(&db_path).unwrap();
            sync_incremental(&conn, &e.store, &k, &idx).unwrap()
        });
        for _ in 0..50 {
            let hits = idx.search(&d[5], 5, &Default::default(), None).unwrap();
            assert!(!hits.is_empty());
        }
        h.join().unwrap()
    });
    assert_eq!(added.read, 100);
    assert_eq!(idx.len(), 200);
    // 같은 동기화를 다시 해도 중복이 없다 (멱등)
    assert_eq!(
        sync_incremental(&e.conn, &e.store, &k, &idx).unwrap().read,
        0
    );

    // 재시작: 저장된 세대(100개)를 열고 SQLite 의 나머지를 이어서 따라잡는다
    drop(idx);
    put_one_each(&mut e, &k, &d[200..], 201);
    let reopened = gens.open_active(&k, OpenMode::Load).unwrap().unwrap();
    assert_eq!(reopened.len(), 100);
    let caught = sync_incremental(&e.conn, &e.store, &k, &reopened).unwrap();
    assert_eq!((caught.read, reopened.len()), (150, 250));
}

// ------------------------------------------------------------ T-emb-09 (Happy / Business rule / Boundary)
#[test]
fn t_emb_09_asset_vectors_returns_only_the_active_vectors_of_that_asset_in_frame_order() {
    // Arrange: 자산 1 은 프레임 3개 -> 재임베딩(1개), 자산 2 는 1개, 자산 3 은 없음
    let mut e = env(3);
    let k = key("1");
    let d = dataset(5, DIM, 3);
    let frames: Vec<NewVector> = (0..3)
        .map(|i| NewVector {
            segment_id: None,
            source: VectorSource::Frame,
            frame_index: Some(i),
            vector: &d[i as usize],
        })
        .collect();
    e.store.put(&mut e.conn, &k, 1, &frames, NOW).unwrap();
    e.store.put(&mut e.conn, &k, 2, &[nv(&d[3])], NOW).unwrap();

    // Act
    let two = e.store.asset_vectors(&e.conn, &k, 1).unwrap();
    e.store
        .put(&mut e.conn, &k, 1, &[nv(&d[4])], NOW + 1)
        .unwrap();
    let after_reembed = e.store.asset_vectors(&e.conn, &k, 1).unwrap();
    let none = e.store.asset_vectors(&e.conn, &k, 3).unwrap();
    let other_model = e.store.asset_vectors(&e.conn, &key("2"), 1).unwrap();

    // Assert
    assert_eq!(two.len(), 3);
    assert_eq!(after_reembed.len(), 1, "superseded frames are not returned");
    let expected = normalized(&d[4]);
    let dot: f32 = after_reembed[0]
        .iter()
        .zip(&expected)
        .map(|(a, b)| a * b)
        .sum();
    assert!(dot > 0.99, "f16 round-trip keeps the direction: {dot}");
    assert!(none.is_empty());
    assert!(
        other_model.is_empty(),
        "other model versions are never mixed"
    );
}
