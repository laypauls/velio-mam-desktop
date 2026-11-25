//! 데이터셋 폴더 배치와 읽기 도구.
//!
//! `<dir>/dataset.db` (mam-search gen_dataset DB + poc_embeddings), `vectors.f32` (행 i = embedding_id i+1),
//! `raw.f16` ((b) 원시 파일), `queries.f32`, `truth.bin` (질의별 정확한 상위 truth_k 의 embedding_id, u32), `info.txt`.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::args;

pub struct Layout {
    pub dir: PathBuf,
}

impl Layout {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
    pub fn db(&self) -> PathBuf {
        self.dir.join("dataset.db")
    }
    pub fn vectors(&self) -> PathBuf {
        self.dir.join("vectors.f32")
    }
    pub fn raw(&self) -> PathBuf {
        self.dir.join("raw.f16")
    }
    pub fn queries(&self) -> PathBuf {
        self.dir.join("queries.f32")
    }
    pub fn truth(&self) -> PathBuf {
        self.dir.join("truth.bin")
    }
    pub fn info(&self) -> PathBuf {
        self.dir.join("info.txt")
    }
    /// 인덱스 폴더: `index-<name>`
    pub fn index(&self, name: &str) -> PathBuf {
        self.dir.join(format!("index-{name}"))
    }
}

#[derive(Clone, Debug)]
pub struct Info {
    pub assets: u64,
    pub vectors: usize,
    pub dim: usize,
    pub queries: usize,
    pub truth_k: usize,
    pub seed: u64,
    pub kind: String,
    pub clusters: usize,
}

impl Info {
    pub fn render(&self) -> String {
        format!(
            "assets={}\nvectors={}\ndim={}\nqueries={}\ntruth_k={}\nseed={}\nkind={}\nclusters={}\n",
            self.assets,
            self.vectors,
            self.dim,
            self.queries,
            self.truth_k,
            self.seed,
            self.kind,
            self.clusters
        )
    }

    pub fn read(l: &Layout) -> Self {
        let text = std::fs::read_to_string(l.info())
            .unwrap_or_else(|e| args::fail(&format!("read {}: {e}", l.info().display())));
        let get = |k: &str| -> String {
            text.lines()
                .find_map(|ln| ln.strip_prefix(&format!("{k}=")))
                .unwrap_or_else(|| args::fail(&format!("info.txt: missing {k}")))
                .to_string()
        };
        let num = |k: &str| -> u64 { get(k).parse().unwrap() };
        Self {
            assets: num("assets"),
            vectors: num("vectors") as usize,
            dim: num("dim") as usize,
            queries: num("queries") as usize,
            truth_k: num("truth_k") as usize,
            seed: num("seed"),
            kind: get("kind"),
            clusters: num("clusters") as usize,
        }
    }
}

pub fn read_f32_file(path: &Path, count: usize) -> Vec<f32> {
    let mut v = vec![0f32; count];
    let mut f =
        File::open(path).unwrap_or_else(|e| args::fail(&format!("{}: {e}", path.display())));
    f.read_exact(bytemuck::cast_slice_mut(&mut v))
        .unwrap_or_else(|e| args::fail(&format!("{}: {e}", path.display())));
    v
}

pub fn read_queries(l: &Layout, info: &Info) -> Vec<Vec<f32>> {
    let all = read_f32_file(&l.queries(), info.queries * info.dim);
    all.chunks(info.dim).map(<[f32]>::to_vec).collect()
}

/// 질의별 정확한 상위 `truth_k` embedding_id.
pub fn read_truth(l: &Layout, info: &Info) -> Vec<Vec<u32>> {
    let mut raw = vec![0u32; info.queries * info.truth_k];
    let mut f = File::open(l.truth()).unwrap_or_else(|e| args::fail(&format!("truth: {e}")));
    f.read_exact(bytemuck::cast_slice_mut(&mut raw))
        .unwrap_or_else(|e| args::fail(&format!("truth: {e}")));
    raw.chunks(info.truth_k).map(<[u32]>::to_vec).collect()
}

/// `vectors.f32` 를 `rows_per_chunk` 행씩 읽어 콜백에 넘긴다 (mmap 을 쓰지 않아 작업 집합이 부풀지 않는다).
/// 콜백 인자: (첫 embedding_id, ids, 행 데이터).
pub fn for_each_chunk(
    l: &Layout,
    info: &Info,
    rows_per_chunk: usize,
    mut f: impl FnMut(&[u64], &[f32]),
) {
    let mut file =
        File::open(l.vectors()).unwrap_or_else(|e| args::fail(&format!("vectors.f32: {e}")));
    let mut buf = vec![0f32; rows_per_chunk * info.dim];
    let mut first = 0usize;
    while first < info.vectors {
        let rows = rows_per_chunk.min(info.vectors - first);
        let slice = &mut buf[..rows * info.dim];
        file.read_exact(bytemuck::cast_slice_mut(slice))
            .unwrap_or_else(|e| args::fail(&format!("vectors.f32: {e}")));
        let ids: Vec<u64> = (first as u64 + 1..first as u64 + 1 + rows as u64).collect();
        f(&ids, slice);
        first += rows;
    }
}

/// 질의별 Recall@k: 반환 id 중 정답 상위 k 에 있는 비율.
pub fn recall_at(found: &[u64], truth: &[u32], k: usize) -> f64 {
    let t = &truth[..k.min(truth.len())];
    let hit = found
        .iter()
        .take(k)
        .filter(|id| t.contains(&(**id as u32)))
        .count();
    hit as f64 / t.len().max(1) as f64
}

/// 속도: 데이터셋 폴더의 `index-*` 디렉터리 총 크기.
pub fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
