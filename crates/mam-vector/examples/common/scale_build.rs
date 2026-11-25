//! P4 규모 측정 — 적재(raw f16 + SQLite 메타) -> 세대 재구축 -> 크기 보고.
//!
//! 적재는 `EmbedStore::put` 과 같은 디스크 형식(raw 파일 append + fsync -> SQLite 행)을 대량 배치로 쓴다.
//! `put` 자체는 자산 1건씩 fsync 하고 `MAX(raw_offset)` 를 훑으므로 100만 적재에 쓰지 않고, 마지막에
//! 같은 DB 에서 `put` 을 몇 건 직접 재서 그 비용을 따로 보고한다.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mam_vector::raw::RawFile;
use mam_vector::{EmbedStore, Generations, ModelKey, NewVector, VectorKind, VectorSource};
use rusqlite::{Connection, params};

use crate::common::data::dir_size;
use crate::common::sysinfo::{current_rss_bytes, mb, peak_rss_bytes};
use crate::common::{self, stats};
use crate::scale_data::{Mixture, SEM_DIM, Scale, VIS_DIM, fill_semantic, fill_visual};

/// 한 번에 적재하는 자산 수 (영상이면 벡터 5배).
const CHUNK: u64 = 20_000;
const NOW: i64 = 1_800_000_000_000;

pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn db(&self) -> PathBuf {
        self.root.join("mam.db")
    }
    pub fn vectors(&self) -> PathBuf {
        self.root.join("vectors")
    }
    pub fn backup(&self) -> PathBuf {
        self.root.join("backup")
    }
}

pub fn visual_key() -> ModelKey {
    ModelKey {
        kind: VectorKind::Visual,
        model_id: "synthetic-clip".into(),
        model_version: "1".into(),
        dimension: VIS_DIM,
    }
}

pub fn semantic_key() -> ModelKey {
    ModelKey {
        kind: VectorKind::Semantic,
        model_id: "synthetic-e5".into(),
        model_version: "1".into(),
        dimension: SEM_DIM,
    }
}

fn insert_assets(conn: &mut Connection, sc: &Scale) {
    let mut lo = 1;
    while lo <= sc.assets() {
        let hi = (lo + 50_000).min(sc.assets() + 1);
        let tx = conn.transaction().unwrap();
        {
            let mut st = tx
                .prepare(
                    "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
                )
                .unwrap();
            for a in lo..hi {
                let t = if sc.is_video(a) { "video" } else { "image" };
                st.execute(params![
                    a as i64,
                    t,
                    1_000_000 + a as i64,
                    NOW - a as i64 * 1000
                ])
                .unwrap();
            }
        }
        tx.commit().unwrap();
        lo = hi;
    }
}

/// 한 종류 전체를 적재한다. 반환: 걸린 시간.
fn ingest(
    conn: &mut Connection,
    store: &EmbedStore,
    sc: &Scale,
    key: &ModelKey,
    mix: &Mixture,
    threads: usize,
) -> Duration {
    let t0 = Instant::now();
    let visual = key.kind == VectorKind::Visual;
    let dim = key.dimension;
    std::fs::create_dir_all(store.key_dir(key)).unwrap();
    let mut raw = RawFile::open_with_floor(&store.raw_path(key), dim, 0).unwrap();
    let mut lo = 1;
    while lo <= sc.assets() {
        let hi = (lo + CHUNK).min(sc.assets() + 1);
        let rows = if visual {
            fill_visual(mix, sc, lo, hi, threads)
        } else {
            fill_semantic(mix, lo, hi, threads)
        };
        let refs: Vec<&[f32]> = rows.chunks(dim).collect();
        let offsets = raw.append(&refs).unwrap();
        raw.sync().unwrap(); // 파일 -> SQLite 순서 (put 과 같다)
        let tx = conn.transaction().unwrap();
        {
            let mut ins = tx
                .prepare(
                    "INSERT INTO embeddings (embedding_id, asset_id, segment_id, vector_kind, source, frame_index,
                        model_id, model_version, dimension, raw_offset, created_at)
                     VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .unwrap();
            let mut off = offsets.iter();
            for a in lo..hi {
                let eids = if visual {
                    sc.visual_eids(a)
                } else {
                    sc.semantic_eid(a)..sc.semantic_eid(a) + 1
                };
                for (j, eid) in eids.enumerate() {
                    let (src, fi) = match (visual, sc.is_video(a)) {
                        (false, _) => ("text", None),
                        (true, false) => ("image", None),
                        (true, true) => ("frame", Some(j as i64)),
                    };
                    ins.execute(params![
                        eid as i64,
                        a as i64,
                        key.kind.as_str(),
                        src,
                        fi,
                        key.model_id,
                        key.model_version,
                        dim as i64,
                        *off.next().unwrap() as i64,
                        NOW
                    ])
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
        lo = hi;
    }
    t0.elapsed()
}

/// 제품 `EmbedStore::put` 을 가득 찬 DB 에서 직접 재고, 쿼리 계획을 보여 준다. 잰 행·자산은 지운다.
fn put_probe(conn: &mut Connection, store: &EmbedStore, sc: &Scale, n: u64) {
    let key = visual_key();
    println!(
        "\n### EmbedStore::put 단건 비용 (시각 {}건 적재된 DB, put {n}건)",
        sc.visual()
    );
    let plan: Vec<String> = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT MAX(raw_offset) FROM embeddings
             WHERE vector_kind = 'visual' AND model_id = 'synthetic-clip' AND model_version = '1' AND dimension = 512",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    println!("`MAX(raw_offset)` 쿼리 계획: {plan:?}");
    let q = Instant::now();
    let m: Option<i64> = conn
        .query_row(
            "SELECT MAX(raw_offset) FROM embeddings
             WHERE vector_kind = 'visual' AND model_id = 'synthetic-clip' AND model_version = '1' AND dimension = 512",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "`MAX(raw_offset)` 단독 실행: {:.1} ms (결과 {m:?})",
        q.elapsed().as_secs_f64() * 1e3
    );
    let mix = Mixture::visual();
    let base = sc.assets() + sc.visual() + 10;
    let mut samples = Vec::new();
    for i in 0..n {
        let a = base + i;
        conn.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'image', 1, 1, 1, 1)",
            [a as i64],
        )
        .unwrap();
        let mut v = vec![0f32; VIS_DIM];
        mix.item(a, &mut v);
        let nv = [NewVector {
            segment_id: None,
            source: VectorSource::Image,
            frame_index: None,
            vector: &v,
        }];
        let t = Instant::now();
        store.put(conn, &key, a as i64, &nv, NOW).unwrap();
        samples.push(t.elapsed());
    }
    let s = stats::summarize(&samples);
    println!(
        "put 1건(자산 1개, 벡터 1개, fsync 포함): P50 {:.1} / max {:.1} ms (n={})",
        s.p50_ms, s.max_ms, s.n
    );
    conn.execute(
        "DELETE FROM media_assets WHERE asset_id >= ?1",
        [base as i64],
    )
    .unwrap(); // embeddings 는 ON DELETE CASCADE
}

fn rebuild(conn: &Connection, store: &EmbedStore, key: &ModelKey) {
    let gens = Generations::new(store, key).unwrap();
    let t = Instant::now();
    let rep = gens.rebuild(conn, store, key).unwrap();
    let el = t.elapsed();
    println!(
        "| 재구축 {} ({}-d) | {} 벡터 (건너뜀 {}) | {:.1} s | {:.0} vec/s | RSS 현재 {:.0} MB / 최대 {:.0} MB |",
        key.kind.as_str(),
        key.dimension,
        rep.vectors,
        rep.skipped.len(),
        el.as_secs_f64(),
        rep.vectors as f64 / el.as_secs_f64(),
        mb(current_rss_bytes()),
        mb(peak_rss_bytes())
    );
}

pub fn build(p: &Paths, sc: Scale, threads: usize) {
    std::fs::create_dir_all(&p.root).unwrap();
    let mut conn = common::open_db(&p.db());
    mam_db::migrate(&mut conn, &p.backup(), &mam_db::SystemClock).unwrap();
    let store = EmbedStore::new(p.vectors()).unwrap();

    let t = Instant::now();
    insert_assets(&mut conn, &sc);
    println!(
        "자산 {}건 적재: {:.1} s",
        sc.assets(),
        t.elapsed().as_secs_f64()
    );
    let (vk, sk) = (visual_key(), semantic_key());
    let dv = ingest(&mut conn, &store, &sc, &vk, &Mixture::visual(), threads);
    println!(
        "시각 {}건 적재 (raw+SQLite): {:.1} s, RSS 최대 {:.0} MB",
        sc.visual(),
        dv.as_secs_f64(),
        mb(peak_rss_bytes())
    );
    let ds = ingest(&mut conn, &store, &sc, &sk, &Mixture::semantic(), threads);
    println!(
        "의미 {}건 적재 (raw+SQLite): {:.1} s, RSS 최대 {:.0} MB",
        sc.assets(),
        ds.as_secs_f64(),
        mb(peak_rss_bytes())
    );

    put_probe(&mut conn, &store, &sc, 10);

    println!("\n| 재구축 | 벡터 | 시간 | 처리량 | RSS |\n|---|---|---|---|---|");
    rebuild(&conn, &store, &vk);
    rebuild(&conn, &store, &sk);

    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    report_sizes(p, &conn, &store);
    println!("\n빌드 프로세스 최대 RSS: {:.0} MB", mb(peak_rss_bytes()));
}

pub fn report_sizes(p: &Paths, conn: &Connection, store: &EmbedStore) {
    let file = |path: PathBuf| std::fs::metadata(path).map_or(0, |m| m.len());
    println!("\n### 크기");
    println!("| 항목 | MB |\n|---|---:|");
    for key in [visual_key(), semantic_key()] {
        let raw = file(store.raw_path(&key));
        let gen_dir = dir_size(&store.key_dir(&key).join("gen"));
        println!("| raw.f16 {} | {:.0} |", key.kind.as_str(), mb(raw));
        println!("| 인덱스 세대 {} | {:.0} |", key.kind.as_str(), mb(gen_dir));
    }
    println!(
        "| SQLite 파일 (mam.db, 체크포인트 후) | {:.0} |",
        mb(file(p.db()))
    );
    let stat = conn.prepare(
        "SELECT name, SUM(pgsize) FROM dbstat WHERE name IN
         ('embeddings','idx_embeddings_model','idx_embeddings_asset','media_assets') GROUP BY name",
    );
    if let Ok(mut st) = stat {
        let rows: Vec<(String, i64)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (n, b) in rows {
            println!("| SQLite 내역 {n} | {:.0} |", mb(b as u64));
        }
    } else {
        println!("| (dbstat 사용 불가 — 테이블별 내역 생략) | |");
    }
}
