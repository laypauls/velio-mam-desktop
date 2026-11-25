//! T-vec-09: 원시 임베딩 (a) SQLite BLOB / (b) 추가 전용 파일에서 인덱스를 재구축한다.
//! 분류: Happy / External failure / Boundary / Business rule(원본 데이터 보존).

mod common;

use common::*;
use mam_vector::raw::{self, RawFile};
use mam_vector::{Engine, Quantization, VectorError, add_parallel, create_index};
use rusqlite::{Connection, params};
use std::path::Path;

const ASSETS: usize = 100;
const FRAMES: usize = 3;

/// mam-db 마이그레이션이 적용된 임시 DB + 자산 100개 + poc_embeddings 스키마.
fn new_db(dir: &Path) -> Connection {
    let mut conn = mam_db::open(&dir.join("t.db")).unwrap();
    mam_db::migrate(&mut conn, &dir.join("backup"), &mam_db::SystemClock).unwrap();
    for a in 1..=ASSETS as i64 {
        conn.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'video', 1, 1, 1, 1)",
            [a],
        )
        .unwrap();
    }
    raw::create_schema(&conn).unwrap();
    conn
}

/// (a)·(b) 두 방식으로 모두 저장한다. embedding_id 는 AUTOINCREMENT 로 1..=300.
fn store_all(conn: &mut Connection, raw_path: &Path, d: &[Vec<f32>]) {
    let mut file = RawFile::open(raw_path, DIM).unwrap();
    let refs: Vec<&[f32]> = d.iter().map(|v| v.as_slice()).collect();
    let offsets = file.append(&refs).unwrap();
    file.sync().unwrap(); // 파일 -> SQLite 순서
    let tx = conn.transaction().unwrap();
    for (i, v) in d.iter().enumerate() {
        tx.execute(
            "INSERT INTO poc_embeddings (asset_id, segment_id, model_id, model_version, dimension, raw_offset)
             VALUES (?1, ?2, 'test-model', '1', ?3, ?4)",
            params![(i / FRAMES) as i64 + 1, (i % FRAMES) as i64, DIM as i64, offsets[i] as i64],
        )
        .unwrap();
        let id = tx.last_insert_rowid() as u64;
        raw::put_blob(&tx, id, v).unwrap();
    }
    tx.commit().unwrap();
}

fn rebuild(
    spec: &mam_vector::IndexSpec,
    src: impl FnOnce(&mut dyn FnMut(u64, &[f32]) -> mam_vector::Result<()>) -> mam_vector::Result<u64>,
) -> Box<dyn mam_vector::AnnIndex> {
    let idx = create_index(spec, ASSETS * FRAMES).unwrap();
    let (mut ids, mut rows) = (Vec::new(), Vec::new());
    let n = src(&mut |id, v| {
        ids.push(id);
        rows.extend_from_slice(v);
        Ok(())
    })
    .unwrap();
    assert_eq!(n as usize, ASSETS * FRAMES);
    add_parallel(&*idx, &ids, &rows, 4).unwrap();
    idx
}

// ---------------------------------------------------------------- T-vec-09 (Happy)
#[test]
fn t_vec_09_rebuild_from_blob_and_file_matches_original() {
    let d = dataset(ASSETS * FRAMES, DIM, 11);
    let dir = tempfile::tempdir().unwrap();
    let mut conn = new_db(dir.path());
    let raw_path = dir.path().join("raw.f16");
    store_all(&mut conn, &raw_path, &d);

    for s in matrix() {
        let l = label(&s);
        let original = build(&s, &d);
        let from_a = rebuild(&s, |f| raw::for_each_blob(&conn, DIM, f));
        let from_b = rebuild(&s, |f| raw::for_each_file(&conn, &raw_path, DIM, f));
        for probe in [0usize, 40, 150, 299] {
            let truth: Vec<u64> = original
                .search(&d[probe], 10, &params(), None)
                .unwrap()
                .iter()
                .map(|h| h.id)
                .collect();
            for (name, idx) in [("blob", &from_a), ("file", &from_b)] {
                let got: Vec<u64> = idx
                    .search(&d[probe], 10, &params(), None)
                    .unwrap()
                    .iter()
                    .map(|h| h.id)
                    .collect();
                if s.engine == Engine::Flat && s.quantization == Quantization::F16 {
                    // 같은 정밀도(f16)의 전수 비교는 완전 일치 (원시 f16 -> f32 -> 재정규화 -> f16 이 순위를 바꾸지 않는다).
                    // f32 인덱스는 원시 저장이 f16 이라 근접 동점의 순서가 바뀔 수 있어 Recall 로 본다
                    assert_eq!(got, truth, "{l} {name} probe {probe}");
                } else {
                    assert!(recall(&got, &truth) >= 0.8, "{l} {name} probe {probe}");
                }
            }
        }
        // (a) 와 (b) 는 같은 데이터이므로 서로 완전히 같은 인덱스를 만든다 (전수 비교 기준)
        if s.engine == Engine::Flat {
            let ra = from_a.search(&d[7], 10, &params(), None).unwrap();
            let rb = from_b.search(&d[7], 10, &params(), None).unwrap();
            assert_eq!(ra, rb, "{l}: (a)/(b) 불일치");
        }
    }
}

#[test]
fn t_vec_09b_f16_storage_loss_is_small() {
    // D2: f16 저장 시 Recall 차이 — 작은 데이터에서도 상위 10 이 유지되는지 (본 측정은 examples 에서)
    let d = dataset(ASSETS * FRAMES, DIM, 12);
    let f32_idx = build(&spec(Engine::Flat, Quantization::F32), &d);
    let f16_idx = build(&spec(Engine::Flat, Quantization::F16), &d);
    let mut total = 0.0;
    for probe_v in d.iter().take(30) {
        let t: Vec<u64> = f32_idx
            .search(probe_v, 10, &params(), None)
            .unwrap()
            .iter()
            .map(|h| h.id)
            .collect();
        let g: Vec<u64> = f16_idx
            .search(probe_v, 10, &params(), None)
            .unwrap()
            .iter()
            .map(|h| h.id)
            .collect();
        total += recall(&g, &t);
    }
    assert!(total / 30.0 >= 0.99, "f16 recall {}", total / 30.0);
}

// ---------------------------------------------------------------- External failure / Boundary
#[test]
fn t_vec_09c_raw_file_failures() {
    let d = dataset(ASSETS * FRAMES, DIM, 13);
    let dir = tempfile::tempdir().unwrap();
    let mut conn = new_db(dir.path());
    let raw_path = dir.path().join("raw.f16");
    store_all(&mut conn, &raw_path, &d);
    let full = std::fs::read(&raw_path).unwrap();

    // 파일이 잘렸는데 SQLite 가 뒤쪽 오프셋을 가리킨다 -> Corrupt (패닉 없음)
    std::fs::write(&raw_path, &full[..full.len() / 2]).unwrap();
    let r = raw::for_each_file(&conn, &raw_path, DIM, |_, _| Ok(()));
    assert!(matches!(r, Err(VectorError::Corrupt(_))), "{:?}", r.err());

    // 비정상 종료로 꼬리가 반쪽인 파일을 다시 열면 온전한 레코드까지 잘라 낸다
    let rec = DIM * 2;
    std::fs::write(&raw_path, &full[..rec * 5 + rec / 2]).unwrap();
    let f = RawFile::open(&raw_path, DIM).unwrap();
    assert_eq!(f.len_bytes() as usize, rec * 5);
    assert_eq!(
        std::fs::metadata(&raw_path).unwrap().len() as usize,
        rec * 5
    );

    // 다른 차원으로 append 하면 거부
    let mut f = RawFile::open(&raw_path, DIM).unwrap();
    assert!(matches!(
        f.append(&[&d[0][..DIM - 1]]),
        Err(VectorError::DimensionMismatch { .. })
    ));

    // 빈 파일 -> 0건
    let empty = dir.path().join("empty.f16");
    std::fs::write(&empty, []).unwrap();
    assert_eq!(
        raw::for_each_file(&conn, &empty, DIM, |_, _| Ok(())).unwrap(),
        0
    );
}

#[test]
fn t_vec_09d_blob_dimension_mismatch_is_reported() {
    let d = dataset(10, DIM, 14);
    let dir = tempfile::tempdir().unwrap();
    let conn = new_db(dir.path());
    conn.execute(
        "INSERT INTO poc_embeddings (asset_id, model_id, model_version, dimension) VALUES (1, 'm', '1', ?1)",
        [DIM as i64],
    )
    .unwrap();
    raw::put_blob(&conn, 1, &d[0]).unwrap();
    let r = raw::for_each_blob(&conn, DIM + 8, |_, _| Ok(()));
    assert!(
        matches!(r, Err(VectorError::DimensionMismatch { .. })),
        "{:?}",
        r.err()
    );
    // 콜백 오류는 그대로 전파된다
    let r = raw::for_each_blob(&conn, DIM, |_, _| Err(VectorError::ReadOnly));
    assert!(matches!(r, Err(VectorError::ReadOnly)));
}

#[test]
fn t_vec_09e_embedding_id_is_never_reused() {
    // AUTOINCREMENT: 삭제 후 새 행이 이전 id 를 재사용하지 않는다 (ANN 키 충돌 방지)
    let dir = tempfile::tempdir().unwrap();
    let conn = new_db(dir.path());
    let ins = |c: &Connection| {
        c.execute(
            "INSERT INTO poc_embeddings (asset_id, model_id, model_version, dimension) VALUES (1, 'm', '1', 8)",
            [],
        )
        .unwrap();
        c.last_insert_rowid()
    };
    let first = ins(&conn);
    conn.execute(
        "DELETE FROM poc_embeddings WHERE embedding_id = ?1",
        [first],
    )
    .unwrap();
    assert!(ins(&conn) > first);
}
