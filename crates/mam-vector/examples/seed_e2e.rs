//! P4 종단 성능 측정용 — 합성 라이브러리(`mam-search` `gen_dataset` DB)의 모든 활성 자산에 의미(e5) 벡터를 붙이고
//! 활성 인덱스 세대를 만든다. 앱이 `MAMData/indexes/vectors` 를 그대로 읽는다.
//!
//! 실행: `cargo run --release -p mam-vector --example seed_e2e -- --db <mam.sqlite> --root <MAMData>/indexes/vectors`
//! 옵션: `--model-id multilingual-e5-small --model-version <첫 파일 sha256 앞 12자> --dim 384 --seed 7`
//!
//! 벡터는 클러스터 중심 + 잡음의 합성 값이라 **순위 품질은 평가할 수 없다** (속도·적재 경로만 시험).

use std::time::Instant;

use mam_db::open;
use mam_vector::raw::RawFile;
use mam_vector::{EmbedStore, Generations, ModelKey, VectorKind};
use rusqlite::params;

fn arg(name: &str, default: Option<&str>) -> String {
    let argv: Vec<String> = std::env::args().collect();
    argv.iter()
        .position(|a| a == &format!("--{name}"))
        .and_then(|i| argv.get(i + 1).cloned())
        .or_else(|| default.map(str::to_string))
        .unwrap_or_else(|| panic!("missing --{name}"))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// [-1, 1)
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

fn main() {
    let db = arg("db", None);
    let root = arg("root", None);
    let dim: usize = arg("dim", Some("384")).parse().unwrap();
    let key = ModelKey {
        kind: VectorKind::Semantic,
        model_id: arg("model-id", Some("multilingual-e5-small")),
        model_version: arg("model-version", Some("ca456c06b3a9")),
        dimension: dim,
    };
    let mut rng = Rng(arg("seed", Some("7")).parse::<u64>().unwrap() | 1);
    let started = Instant::now();

    let mut conn = open(std::path::Path::new(&db)).expect("open db");
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT asset_id FROM media_assets WHERE status = 'active' ORDER BY asset_id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let centers: Vec<Vec<f32>> = (0..500)
        .map(|_| (0..dim).map(|_| rng.unit()).collect())
        .collect();

    let store = EmbedStore::new(&root).expect("store");
    std::fs::create_dir_all(store.key_dir(&key)).unwrap();
    let mut raw = RawFile::open(&store.raw_path(&key), dim).expect("raw");
    let now = 1_700_000_000_000i64;
    for chunk in ids.chunks(10_000) {
        let vectors: Vec<Vec<f32>> = chunk
            .iter()
            .map(|_| {
                let c = &centers[(rng.next() % centers.len() as u64) as usize];
                let mut v: Vec<f32> = c.iter().map(|x| x + 0.6 * rng.unit()).collect();
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
                v.iter_mut().for_each(|x| *x /= n);
                v
            })
            .collect();
        let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        let offsets = raw.append(&refs).expect("append");
        let tx = conn.transaction().unwrap();
        {
            let mut ins = tx
                .prepare(
                    "INSERT INTO embeddings (asset_id, segment_id, vector_kind, source, frame_index,
                        model_id, model_version, dimension, raw_offset, created_at)
                     VALUES (?1, NULL, 'semantic', 'text', NULL, ?2, ?3, ?4, ?5, ?6)",
                )
                .unwrap();
            for (id, off) in chunk.iter().zip(&offsets) {
                ins.execute(params![
                    id,
                    key.model_id,
                    key.model_version,
                    dim as i64,
                    *off as i64,
                    now
                ])
                .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    raw.sync().unwrap();
    let report = Generations::new(&store, &key)
        .unwrap()
        .rebuild(&conn, &store, &key)
        .expect("rebuild generation");
    println!(
        "seeded {} vectors (dim {dim}) into {root}; generation {} ({} vectors) in {:.1}s",
        ids.len(),
        report.generation,
        report.vectors,
        started.elapsed().as_secs_f64()
    );
}
