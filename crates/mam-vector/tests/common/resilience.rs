//! P4 복원력 시험 공용 헬퍼 — 실제 SQLite(마이그레이션 + 제안 DDL) + 임시 디렉터리 raw/인덱스.
//! `#[path = "common/resilience.rs"] mod res;` 로 포함한다 (기존 `common/mod.rs` 는 건드리지 않는다).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use mam_vector::{
    AnnIndex, EmbedStore, FlatIndex, Generations, ModelKey, NewVector, OpenMode, SearchParams,
    VectorKind, VectorSource, active_candidates, plan_candidates,
};
use rusqlite::Connection;
use tempfile::TempDir;

pub const DIM: usize = 32;
pub const NOW: i64 = 1_700_000_000_000;

pub struct Env {
    pub dir: TempDir,
    pub conn: Connection,
    pub store: EmbedStore,
}

/// 자산 `assets` 건이 있는 DB (마이그레이션 적용).
pub fn env(assets: i64) -> Env {
    let dir = TempDir::new().unwrap();
    let mut conn = mam_db::open(&dir.path().join("t.db")).unwrap();
    mam_db::migrate(&mut conn, &dir.path().join("backup"), &mam_db::SystemClock).unwrap();
    for a in 1..=assets {
        conn.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'image', 1, 1, 1, 1)",
            [a],
        )
        .unwrap();
    }
    let store = EmbedStore::new(dir.path().join("vectors")).unwrap();
    Env { dir, conn, store }
}

pub fn key(version: &str) -> ModelKey {
    key_dim(version, DIM)
}

pub fn key_dim(version: &str, dimension: usize) -> ModelKey {
    ModelKey {
        kind: VectorKind::Visual,
        model_id: "clip-test".into(),
        model_version: version.into(),
        dimension,
    }
}

pub fn nv(v: &[f32]) -> NewVector<'_> {
    NewVector {
        segment_id: None,
        source: VectorSource::Image,
        frame_index: None,
        vector: v,
    }
}

/// 자산 `first_asset + i` 에 벡터 하나씩 저장한다.
pub fn put_one_each(e: &mut Env, k: &ModelKey, data: &[Vec<f32>], first_asset: i64) {
    for (i, v) in data.iter().enumerate() {
        e.store
            .put(&mut e.conn, k, first_asset + i as i64, &[nv(v)], NOW)
            .unwrap();
    }
}

/// raw 에서 읽은 활성 레코드 (embedding_id, 벡터) 와 건너뛴 id.
pub fn read_all(e: &Env, k: &ModelKey) -> (Vec<(u64, Vec<f32>)>, Vec<u64>) {
    let mut out = Vec::new();
    let rep = e
        .store
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
    (out, rep.skipped)
}

pub fn gen_dir(e: &Env, k: &ModelKey, name: &str) -> PathBuf {
    e.store.key_dir(k).join("gen").join(name)
}

pub fn active_gen(e: &Env, k: &ModelKey) -> String {
    Generations::new(&e.store, k)
        .unwrap()
        .active_name()
        .unwrap()
        .expect("active generation")
}

pub fn data_file(dir: &Path) -> PathBuf {
    dir.join("index.flat")
}

/// 전수 순위 (embedding_id, score 비트) — 재구축 전후 비교용.
pub fn full_ranking(idx: &FlatIndex, q: &[f32]) -> Vec<(u64, u32)> {
    idx.search(q, idx.len().max(1), &SearchParams::default(), None)
        .unwrap()
        .iter()
        .map(|h| (h.id, h.score.to_bits()))
        .collect()
}

/// 질의 여러 개에 대한 전수 순위.
pub fn rankings(idx: &FlatIndex, queries: &[Vec<f32>]) -> Vec<Vec<(u64, u32)>> {
    queries.iter().map(|q| full_ranking(idx, q)).collect()
}

/// 자산 단위 상위 k (SQLite 활성 집합 기준).
pub fn top_assets(e: &Env, k: &ModelKey, idx: &FlatIndex, q: &[f32], n: usize) -> Vec<u64> {
    active_candidates(&e.conn, &e.store, k, idx, q, &plan_candidates(n), None)
        .unwrap()
        .iter()
        .take(n)
        .map(|h| h.asset_id)
        .collect()
}

pub fn open_or_rebuild(e: &Env, k: &ModelKey) -> (FlatIndex, Option<mam_vector::RebuildReport>) {
    Generations::new(&e.store, k)
        .unwrap()
        .open_or_rebuild(&e.conn, &e.store, k, OpenMode::Load)
        .unwrap()
}

type Snap = Vec<(PathBuf, Vec<u8>, SystemTime)>;

fn tree_snapshot(root: &Path) -> Snap {
    fn walk(dir: &Path, out: &mut Snap) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                let m = std::fs::metadata(&p).unwrap().modified().unwrap();
                out.push((p.clone(), std::fs::read(&p).unwrap(), m));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 원본 미디어 픽스처 (임시 디렉터리) — 시나리오 끝에 내용·수정 시각 불변을 확인한다.
pub struct Originals {
    dir: TempDir,
    before: Snap,
}

impl Originals {
    pub fn new(n: usize) -> Self {
        let dir = TempDir::new().unwrap();
        for i in 0..n {
            std::fs::write(dir.path().join(format!("m{i}.jpg")), vec![i as u8 + 1; 64]).unwrap();
        }
        let before = tree_snapshot(dir.path());
        Self { dir, before }
    }

    pub fn assert_untouched(&self) {
        assert_eq!(
            self.before,
            tree_snapshot(self.dir.path()),
            "original fixtures must not change"
        );
    }
}
