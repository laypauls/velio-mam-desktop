//! 합성 벡터 생성기 — P1-2 `gen_dataset` 이 만든 SQLite 의 자산 목록에 벡터를 붙인다.
//!
//! 실행: `cargo run --release -p mam-vector --example gen_vectors -- --assets-db <dataset.db> --out <dir> --dim 768`
//! 옵션: `--seed 42` `--clusters N`(기본 자산수/250) `--store a|b|both` `--kind clustered|uniform`
//!       `--queries 1000` `--truth-k 500` `--threads N`
//!
//! 산출물(`--out`): dataset.db(복사본 + poc_embeddings), vectors.f32(원본 f32), raw.f16((b)), queries.f32,
//! truth.bin(정답 상위 truth_k), info.txt. 정답은 **f32 원본** 에 대한 전수 비교다.
//! D3: `poc_*` 테이블은 이 측정 DB 에만 만든다 (마이그레이션 아님).

#[path = "common/mod.rs"]
mod common;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use common::data::{Info, Layout};
use common::model::{Kind, Model, frames_for};
use common::rng::Rng;
use common::{MODEL_ID, MODEL_VERSION, args, json_str};
use mam_vector::exact_topk_f32;
use mam_vector::raw::{self, RawFile};
use rusqlite::params;

const ASSET_BATCH: usize = 20_000;

struct AssetRow {
    asset_id: i64,
    frames: usize,
}

fn main() {
    let a = args::Args::parse();
    let src: std::path::PathBuf = a
        .path("assets-db")
        .unwrap_or_else(|| args::fail("--assets-db is required"));
    let dim: usize = a.num("dim", 768);
    let seed: u64 = a.num("seed", 42);
    let nq: usize = a.num("queries", 1000);
    let truth_k: usize = a.num("truth-k", 500);
    let store = a.get("store").unwrap_or("both").to_string();
    let kind = match a.get("kind").unwrap_or("clustered") {
        "clustered" => Kind::Clustered,
        "uniform" => Kind::Uniform,
        o => args::fail(&format!("--kind must be clustered|uniform, got {o}")),
    };
    let threads: usize = a.num(
        "threads",
        std::thread::available_parallelism().map_or(4, |n| n.get()),
    );
    let (do_a, do_b) = match store.as_str() {
        "a" => (true, false),
        "b" => (false, true),
        "both" => (true, true),
        o => args::fail(&format!("--store must be a|b|both, got {o}")),
    };

    // 1) 자산 목록 (읽기 전용으로 원본 DB 에서)
    let src_conn = common::open_db(&src);
    let assets: Vec<AssetRow> = {
        let mut st = src_conn
            .prepare(
                "SELECT a.asset_id, a.media_type, m.duration_ms FROM media_assets a
                 LEFT JOIN media_metadata m USING (asset_id) ORDER BY a.asset_id",
            )
            .unwrap();
        st.query_map([], |r| {
            Ok(AssetRow {
                asset_id: r.get(0)?,
                frames: frames_for(&r.get::<_, String>(1)?, r.get(2)?),
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    drop(src_conn);
    let total: usize = assets.iter().map(|x| x.frames).sum();
    let clusters: usize = a.num("clusters", (assets.len() / 250).max(64));
    let out_dir = a.path("out").unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/bench-p1-3")
            .join(format!("{}-d{dim}", assets.len()))
    });
    let l = Layout::new(out_dir);
    std::fs::create_dir_all(&l.dir).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", l.db().display()));
    }
    let _ = std::fs::remove_file(l.raw());
    std::fs::copy(&src, l.db()).unwrap_or_else(|e| args::fail(&format!("copy db: {e}")));
    eprintln!(
        "assets={} vectors={total} dim={dim} clusters={clusters} threads={threads}",
        assets.len()
    );

    // 2) 스키마
    let mut conn = common::open_db(&l.db());
    conn.pragma_update(None, "synchronous", "OFF").unwrap();
    raw::create_schema(&conn).unwrap();
    let model = Model::new(dim, clusters, seed, kind);

    // 3) 배치 단위로 생성 -> vectors.f32 / SQLite / raw.f16
    let mut vec_out = BufWriter::with_capacity(8 << 20, File::create(l.vectors()).unwrap());
    let mut raw_file = do_b.then(|| RawFile::open(&l.raw(), dim).unwrap());
    let (mut t_gen, mut t_meta, mut t_blob, mut t_file) = (0f64, 0f64, 0f64, 0f64);
    let started = Instant::now();
    let mut next_id = 1u64;
    for batch in assets.chunks(ASSET_BATCH) {
        let rows: usize = batch.iter().map(|x| x.frames).sum();
        let t = Instant::now();
        let mut data = vec![0f32; rows * dim];
        // 자산 -> 데이터 구간 분할 후 스레드 병렬 생성 (자산별 독립 시드)
        let mut offsets = Vec::with_capacity(batch.len());
        let mut acc = 0usize;
        for x in batch {
            offsets.push(acc);
            acc += x.frames;
        }
        let per = batch.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let mut rest: &mut [f32] = &mut data;
            for (ci, chunk) in batch.chunks(per).enumerate() {
                let n_rows: usize = chunk.iter().map(|x| x.frames).sum();
                let (mine, tail) = rest.split_at_mut(n_rows * dim);
                rest = tail;
                let model = &model;
                let _ = ci;
                s.spawn(move || {
                    let mut pos = 0usize;
                    let mut asset_v = vec![0f32; dim];
                    for x in chunk {
                        if x.frames == 0 {
                            continue;
                        }
                        let mut rng = Rng::for_item(seed, x.asset_id as u64);
                        model.asset_vec(&mut rng, &mut asset_v);
                        for f in 0..x.frames {
                            let out = &mut mine[(pos + f) * dim..(pos + f + 1) * dim];
                            // 이미지(프레임 1개)는 자산 벡터 그대로, 영상은 프레임마다 약간씩 다르게
                            if x.frames == 1 {
                                out.copy_from_slice(&asset_v);
                            } else {
                                model.frame_vec(&mut rng, &asset_v, out);
                            }
                        }
                        pos += x.frames;
                    }
                });
            }
        });
        t_gen += t.elapsed().as_secs_f64();
        vec_out.write_all(bytemuck::cast_slice(&data)).unwrap();

        // (b) 파일 먼저 -> fsync -> SQLite
        let mut file_offsets: Vec<u64> = Vec::new();
        if let Some(rf) = raw_file.as_mut() {
            let t = Instant::now();
            let refs: Vec<&[f32]> = data.chunks(dim).collect();
            file_offsets = rf.append(&refs).unwrap();
            rf.sync().unwrap();
            t_file += t.elapsed().as_secs_f64();
        }
        let tx = conn.transaction().unwrap();
        {
            let t = Instant::now();
            let mut ins = tx
                .prepare_cached(
                    "INSERT INTO poc_embeddings (embedding_id, asset_id, segment_id, vector_kind, model_id, model_version, dimension, raw_offset)
                     VALUES (?1, ?2, ?3, 'visual', ?4, ?5, ?6, ?7)",
                )
                .unwrap();
            let mut row = 0usize;
            for x in batch {
                for f in 0..x.frames {
                    ins.execute(params![
                        (next_id + row as u64) as i64,
                        x.asset_id,
                        f as i64,
                        MODEL_ID,
                        MODEL_VERSION,
                        dim as i64,
                        file_offsets.get(row).map(|o| *o as i64)
                    ])
                    .unwrap();
                    row += 1;
                }
            }
            t_meta += t.elapsed().as_secs_f64();
        }
        if do_a {
            let t = Instant::now();
            for (r, v) in data.chunks(dim).enumerate() {
                raw::put_blob(&tx, next_id + r as u64, v).unwrap();
            }
            t_blob += t.elapsed().as_secs_f64();
        }
        tx.commit().unwrap();
        next_id += rows as u64;
        eprintln!(
            "  vectors {}/{total} ({:.0}s)",
            next_id - 1,
            started.elapsed().as_secs_f64()
        );
    }
    vec_out.flush().unwrap();
    drop(vec_out);
    let ingest_secs = started.elapsed().as_secs_f64();

    // 4) 무결성 + ANALYZE
    let fk: usize = conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .count();
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    if fk != 0 || integrity != "ok" {
        eprintln!("FATAL: foreign_key_check violations={fk}, integrity_check={integrity}");
        std::process::exit(1);
    }
    conn.execute_batch("ANALYZE").unwrap();
    conn.pragma_update(None, "synchronous", "FULL").unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let n_rows: i64 = conn
        .query_row("SELECT count(*) FROM poc_embeddings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n_rows as usize, total);

    // 5) 질의 (색인에 넣지 않은 새 자산 벡터) + 정답
    let mut qdata = vec![0f32; nq * dim];
    for (i, q) in qdata.chunks_mut(dim).enumerate() {
        let mut rng = Rng::for_item(seed ^ 0x51ED_u64, i as u64);
        model.asset_vec(&mut rng, q);
    }
    File::create(l.queries())
        .unwrap()
        .write_all(bytemuck::cast_slice(&qdata))
        .unwrap();

    let t = Instant::now();
    let mut truth = vec![0u32; nq * truth_k];
    {
        let f = File::open(l.vectors()).unwrap();
        // SAFETY: 읽기 전용 매핑, 이 프로세스가 방금 쓰고 닫은 파일이다.
        let map = unsafe { memmap2::Mmap::map(&f).unwrap() };
        let rows: &[f32] = bytemuck::cast_slice(&map[..]);
        let ids: Vec<u64> = (1..=total as u64).collect();
        for (i, q) in qdata.chunks(dim).enumerate() {
            let hits = exact_topk_f32(&ids, rows, dim, q, truth_k, threads);
            for (j, h) in hits.iter().enumerate() {
                truth[i * truth_k + j] = h.id as u32;
            }
            if (i + 1) % 100 == 0 {
                eprintln!("  truth {}/{nq} ({:.0}s)", i + 1, t.elapsed().as_secs_f64());
            }
        }
    }
    let truth_secs = t.elapsed().as_secs_f64();
    File::create(l.truth())
        .unwrap()
        .write_all(bytemuck::cast_slice(&truth))
        .unwrap();

    let info = Info {
        assets: assets.len() as u64,
        vectors: total,
        dim,
        queries: nq,
        truth_k,
        seed,
        kind: a.get("kind").unwrap_or("clustered").to_string(),
        clusters,
    };
    std::fs::write(l.info(), info.render()).unwrap();

    // 6) 보고
    let blob_bytes: i64 = conn
        .query_row(
            "SELECT coalesce(sum(pgsize),0) FROM dbstat WHERE name='poc_embedding_vectors'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let db_bytes = std::fs::metadata(l.db()).map(|m| m.len()).unwrap_or(0);
    let raw_bytes = std::fs::metadata(l.raw()).map(|m| m.len()).unwrap_or(0);
    let cond = common::sysinfo::conditions();
    println!("\n== gen_vectors ==");
    println!(
        "CPU: {}\nRAM: {} GB\nOS: {}",
        cond.cpu, cond.ram_gb, cond.os
    );
    println!(
        "Dataset: assets={} vectors={total} (avg {:.2}/asset) dim={dim} kind={:?} clusters={clusters} seed={seed} store={store}",
        assets.len(),
        total as f64 / assets.len() as f64,
        kind
    );
    println!("| step | seconds |\n|---|---:|");
    println!("| generate vectors (threads={threads}) | {t_gen:.1} |");
    println!("| SQLite poc_embeddings rows | {t_meta:.1} |");
    if do_a {
        println!("| (a) BLOB insert | {t_blob:.1} |");
    }
    if do_b {
        println!("| (b) file append + fsync | {t_file:.1} |");
    }
    println!("| ingest total | {ingest_secs:.1} |");
    println!("| exact truth top-{truth_k} x {nq} queries | {truth_secs:.1} |");
    println!(
        "DB: {:.1} MB (blob table pages {:.1} MB), raw.f16 {:.1} MB, vectors.f32 {:.1} MB",
        common::sysinfo::mb(db_bytes),
        common::sysinfo::mb(blob_bytes as u64),
        common::sysinfo::mb(raw_bytes),
        common::sysinfo::mb(total as u64 * dim as u64 * 4)
    );
    println!(
        "JSON: {{\"tool\":\"gen_vectors\",\"assets\":{},\"vectors\":{total},\"dim\":{dim},\"store\":{},\"gen_secs\":{t_gen:.1},\"meta_secs\":{t_meta:.1},\"blob_secs\":{t_blob:.1},\"file_secs\":{t_file:.1},\"ingest_secs\":{ingest_secs:.1},\"truth_secs\":{truth_secs:.1},\"db_mb\":{:.1},\"blob_mb\":{:.1},\"raw_mb\":{:.1},\"conditions\":{}}}",
        assets.len(),
        json_str(&store),
        common::sysinfo::mb(db_bytes),
        common::sysinfo::mb(blob_bytes as u64),
        common::sysinfo::mb(raw_bytes),
        common::conditions_json(&cond)
    );
}
