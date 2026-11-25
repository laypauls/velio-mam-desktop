//! 원시 임베딩 저장 방식 (D2) 측정: (a) SQLite BLOB vs (b) 추가 전용 파일.
//!
//! 실행: `bench_raw --dir <dataset> [--engine flat --quant f16] [--skip-backup]`
//! 항목: DB 크기(BLOB 테이블 분리), SQLite 백업 API 시간 ((a) 상태 DB vs BLOB 을 뺀 (b) 상태 DB + raw 파일 복사),
//!       원시 -> 메모리 읽기 처리량, 원시 -> 인덱스 재구축 시간, f16 저장의 Recall 손실 (f32 정답 대비).
//! 데이터셋은 `gen_vectors --store both` 로 만든 것이어야 한다. 임시 파일은 `<dir>/tmp-raw` 에 만들고 지운다.

#[path = "common/mod.rs"]
mod common;

use std::time::Instant;

use common::data::{self, Info, Layout};
use common::{args, sysinfo};
use mam_vector::raw;
use mam_vector::{
    AnnIndex, FlatIndex, IndexSpec, Quantization, SearchParams, add_parallel, create_index,
};
use rusqlite::Connection;
use rusqlite::backup::{Backup, StepResult};

const BATCH: usize = 32_768;

fn main() {
    let a = args::Args::parse();
    let l = Layout::new(
        a.path("dir")
            .unwrap_or_else(|| args::fail("--dir is required")),
    );
    let info = Info::read(&l);
    let spec = common::spec_from_args(&a, info.dim);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    println!("\n== bench_raw ==");
    let cond = common::print_conditions();
    println!(
        "Dataset: vectors={} dim={} (f16 record = {} bytes)",
        info.vectors,
        info.dim,
        info.dim * 2
    );
    let conn = common::open_db(&l.db());
    let mut json = vec![
        "\"tool\":\"bench_raw\"".to_string(),
        format!("\"vectors\":{}", info.vectors),
        format!("\"dim\":{}", info.dim),
    ];

    // 1) 크기
    let db_bytes = std::fs::metadata(l.db()).unwrap().len();
    let pages = |name: &str| -> u64 {
        conn.query_row(
            "SELECT coalesce(sum(pgsize),0) FROM dbstat WHERE name = ?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as u64
    };
    let blob_bytes = pages("poc_embedding_vectors");
    let meta_bytes = pages("poc_embeddings") + pages("idx_poc_embeddings_asset");
    let raw_bytes = std::fs::metadata(l.raw()).map(|m| m.len()).unwrap_or(0);
    let payload = info.vectors as u64 * info.dim as u64 * 2;
    println!("\n| size | MB |\n|---|---:|");
    println!(
        "| f16 payload (vectors x dim x 2) | {:.1} |",
        sysinfo::mb(payload)
    );
    println!(
        "| (a) BLOB table pages in DB | {:.1} (overhead {:.1}%) |",
        sysinfo::mb(blob_bytes),
        100.0 * (blob_bytes as f64 / payload as f64 - 1.0)
    );
    println!(
        "| poc_embeddings meta + asset index | {:.1} |",
        sysinfo::mb(meta_bytes)
    );
    println!("| (b) raw.f16 file | {:.1} |", sysinfo::mb(raw_bytes));
    println!(
        "| whole dataset.db (with (a) BLOBs) | {:.1} |",
        sysinfo::mb(db_bytes)
    );
    json.push(format!("\"sizes\":{{\"payload_mb\":{:.1},\"blob_mb\":{:.1},\"meta_mb\":{:.1},\"raw_mb\":{:.1},\"db_mb\":{:.1}}}", sysinfo::mb(payload), sysinfo::mb(blob_bytes), sysinfo::mb(meta_bytes), sysinfo::mb(raw_bytes), sysinfo::mb(db_bytes)));

    // 2) SQLite 백업 API 시간
    let tmp = l.dir.join("tmp-raw");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    if !a.flag("skip-backup") {
        let secs_a = backup_secs(&conn, &tmp.join("backup-a.db"));
        // (b) 상태 DB: 복사본에서 BLOB 테이블을 지우고 VACUUM 한 뒤 같은 방식으로 백업
        let b_db = tmp.join("b-state.db");
        std::fs::copy(l.db(), &b_db).unwrap();
        let bconn = common::open_db(&b_db);
        bconn
            .execute_batch("DROP TABLE poc_embedding_vectors;")
            .unwrap();
        let t = Instant::now();
        bconn.execute_batch("VACUUM;").unwrap();
        let vacuum_secs = t.elapsed().as_secs_f64();
        let b_size = std::fs::metadata(&b_db).unwrap().len();
        let secs_b = backup_secs(&bconn, &tmp.join("backup-b.db"));
        drop(bconn);
        let t = Instant::now();
        std::fs::copy(l.raw(), tmp.join("raw-copy.f16")).unwrap();
        let raw_copy_secs = t.elapsed().as_secs_f64();
        println!(
            "\n| backup (SQLite backup API, 100 pages/step) | seconds | file MB |\n|---|---:|---:|"
        );
        println!(
            "| (a) DB with BLOBs | {secs_a:.2} | {:.1} |",
            sysinfo::mb(db_bytes)
        );
        println!(
            "| (b) DB without BLOBs (after VACUUM {vacuum_secs:.1}s) | {secs_b:.2} | {:.1} |",
            sysinfo::mb(b_size)
        );
        println!(
            "| (b) + raw.f16 plain copy | {raw_copy_secs:.2} | {:.1} |",
            sysinfo::mb(raw_bytes)
        );
        json.push(format!("\"backup\":{{\"a_secs\":{secs_a:.2},\"b_db_secs\":{secs_b:.2},\"b_db_mb\":{:.1},\"b_vacuum_secs\":{vacuum_secs:.1},\"raw_copy_secs\":{raw_copy_secs:.2}}}", sysinfo::mb(b_size)));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // 3) 읽기 처리량 (인덱스에 넣지 않고 순회만)
    let dim = info.dim;
    let t = Instant::now();
    let n_a = raw::for_each_blob(&conn, dim, |_, _| Ok(())).unwrap();
    let read_a = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let n_b = raw::for_each_file(&conn, &l.raw(), dim, |_, _| Ok(())).unwrap();
    let read_b = t.elapsed().as_secs_f64();
    println!("\n| read raw -> f32 (no index) | seconds | vectors/s |\n|---|---:|---:|");
    println!(
        "| (a) SQLite BLOB scan | {read_a:.2} | {:.0} |",
        n_a as f64 / read_a
    );
    println!(
        "| (b) raw.f16 via SQLite offsets (mmap) | {read_b:.2} | {:.0} |",
        n_b as f64 / read_b
    );
    json.push(format!(
        "\"read\":{{\"a_secs\":{read_a:.2},\"b_secs\":{read_b:.2}}}"
    ));

    // 4) 원시 -> 인덱스 재구축 (선택한 엔진)
    let mut rebuild_rows = Vec::new();
    for (label, from_blob) in [("(a) BLOB", true), ("(b) file", false)] {
        let (secs, idx) = rebuild(&conn, &l, &spec, &info, from_blob, threads);
        println!(
            "\n{label}: rebuild {} into {} ({}) took {secs:.1}s ({:.0} vectors/s)",
            idx.len(),
            common::index_name(&spec),
            spec.quantization.as_str(),
            idx.len() as f64 / secs
        );
        rebuild_rows.push(format!("{{\"source\":\"{label}\",\"secs\":{secs:.1}}}"));
        drop(idx);
    }
    json.push(format!(
        "\"rebuild\":{{\"index\":{},\"rows\":[{}]}}",
        common::json_str(&common::index_name(&spec)),
        rebuild_rows.join(",")
    ));

    // 5) f16 저장 손실: 원시(f16) -> flat f16 / flat f32(디코드) 의 Recall (f32 원본 정답 대비)
    let queries = data::read_queries(&l, &info);
    let truth = data::read_truth(&l, &info);
    let p = SearchParams::default();
    let mut rec_rows = Vec::new();
    println!(
        "\n| flat index built from raw f16 | Recall@10 | Recall@100 | loss@10 |\n|---|---:|---:|---:|"
    );
    for q in [Quantization::F16, Quantization::F32] {
        let flat = FlatIndex::with_capacity(dim, q, info.vectors);
        let mut ids = Vec::new();
        let mut rows = Vec::new();
        raw::for_each_file(&conn, &l.raw(), dim, |id, v| {
            ids.push(id);
            rows.extend_from_slice(v);
            if ids.len() == BATCH {
                for (i, &id) in ids.iter().enumerate() {
                    flat.add(id, &rows[i * dim..(i + 1) * dim])?;
                }
                ids.clear();
                rows.clear();
            }
            Ok(())
        })
        .unwrap();
        for (i, &id) in ids.iter().enumerate() {
            flat.add(id, &rows[i * dim..(i + 1) * dim]).unwrap();
        }
        let (mut r10, mut r100) = (0.0, 0.0);
        for (qv, t) in queries.iter().zip(&truth) {
            let h10 = flat.search(qv, 10, &p, None).unwrap();
            r10 += data::recall_at(&h10.iter().map(|h| h.id).collect::<Vec<_>>(), t, 10);
            let h100 = flat.search(qv, 100, &p, None).unwrap();
            r100 += data::recall_at(&h100.iter().map(|h| h.id).collect::<Vec<_>>(), t, 100);
        }
        let n = queries.len() as f64;
        println!(
            "| index {} | {:.4} | {:.4} | {:.4} |",
            q.as_str(),
            r10 / n,
            r100 / n,
            1.0 - r10 / n
        );
        rec_rows.push(format!(
            "{{\"index_quant\":\"{}\",\"recall10\":{:.4},\"recall100\":{:.4}}}",
            q.as_str(),
            r10 / n,
            r100 / n
        ));
    }
    json.push(format!("\"f16_loss\":[{}]", rec_rows.join(",")));
    println!(
        "\npeak working set: {:.1} MB",
        sysinfo::mb(sysinfo::peak_rss_bytes())
    );
    json.push(format!("\"conditions\":{}", common::conditions_json(&cond)));
    println!("JSON: {{{}}}", json.join(","));
}

/// SQLite 백업 API 로 `dst` 에 일관된 스냅샷을 만든다 (100 페이지씩). 걸린 시간(초).
fn backup_secs(src: &Connection, dst: &std::path::Path) -> f64 {
    let mut dconn = Connection::open(dst).unwrap();
    let t = Instant::now();
    {
        let b = Backup::new(src, &mut dconn).unwrap();
        loop {
            match b.step(100).unwrap() {
                StepResult::Done => break,
                _ => continue,
            }
        }
    }
    t.elapsed().as_secs_f64()
}

fn rebuild(
    conn: &Connection,
    l: &Layout,
    spec: &IndexSpec,
    info: &Info,
    from_blob: bool,
    threads: usize,
) -> (f64, Box<dyn AnnIndex>) {
    let dim = info.dim;
    let idx = create_index(spec, info.vectors).unwrap();
    let (mut ids, mut rows) = (Vec::<u64>::new(), Vec::<f32>::new());
    let t = Instant::now();
    let mut sink = |id: u64, v: &[f32]| -> mam_vector::Result<()> {
        ids.push(id);
        rows.extend_from_slice(v);
        if ids.len() == BATCH {
            add_parallel(&*idx, &ids, &rows, threads)?;
            ids.clear();
            rows.clear();
        }
        Ok(())
    };
    if from_blob {
        raw::for_each_blob(conn, dim, &mut sink).unwrap();
    } else {
        raw::for_each_file(conn, &l.raw(), dim, &mut sink).unwrap();
    }
    if !ids.is_empty() {
        add_parallel(&*idx, &ids, &rows, threads).unwrap();
    }
    let secs = t.elapsed().as_secs_f64();
    (secs, idx)
}
