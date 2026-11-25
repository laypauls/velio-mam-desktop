//! 폴더 이전(D-6) 통합 시험 공용 환경 — 볼륨 둘(vol-a 기본, vol-b 추가) + 전체 해시 호출을 세는 계산기.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use mam_core::scan::{ScanCounts, ScanOutcome, scan_folder};
use mam_core::{
    Blake3Hasher, ContentHasher, CoreError, EdgeFile, HashedFile, VolumeKind, WindowsFileIdentity,
};
use rusqlite::Connection;

use super::env::Env;

/// 전체 해시·앞뒤 체크섬 호출 횟수를 세는 계산기 (원본 읽기 횟수 단언용).
#[derive(Default)]
pub struct Probe {
    pub full: AtomicUsize,
    pub edge: AtomicUsize,
}

impl ContentHasher for Probe {
    fn hash_file(&self, path: &Path, cancel: &AtomicBool) -> Result<HashedFile, CoreError> {
        self.full.fetch_add(1, Ordering::SeqCst);
        Blake3Hasher.hash_file(path, cancel)
    }
    fn edge_signature(&self, path: &Path, cancel: &AtomicBool) -> Result<EdgeFile, CoreError> {
        self.edge.fetch_add(1, Ordering::SeqCst);
        mam_core::edge_signature(path, cancel)
    }
}

pub struct Reloc {
    pub env: Env,
    pub probe: Arc<Probe>,
    /// 두 번째 볼륨(vol-b) 루트
    pub vol_b: PathBuf,
}

impl Reloc {
    pub fn new() -> Self {
        let probe = Arc::new(Probe::default());
        let env = Env::with_deps(|_, _| {}, Arc::new(WindowsFileIdentity), probe.clone());
        let vol_b = env.vol.parent().unwrap().join("vol_b");
        fs::create_dir_all(&vol_b).unwrap();
        env.volumes.add("vol-b", VolumeKind::Fixed, &vol_b);
        Self { env, probe, vol_b }
    }

    /// vol-a 에 `lib/a.jpg`, `lib/sub/b.jpg`, `lib/sub/deep/c.png` 를 만들고 `lib` 를 등록·스캔한다. 폴더 id 를 돌려준다.
    pub fn seeded(&self) -> i64 {
        self.env.write("lib/a.jpg", 1);
        self.env.write("lib/sub/b.jpg", 2);
        self.env.write("lib/sub/deep/c.png", 3);
        let id = self.add("lib");
        self.completed(id);
        id
    }

    pub fn add(&self, rel: &str) -> i64 {
        self.env
            .ingest
            .add_folder(self.env.path(rel).to_str().unwrap())
            .unwrap()
            .folder_id
    }

    pub fn path_b(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.vol_b.clone(), |p, s| p.join(s))
    }

    pub fn scan(&self, folder_id: i64) -> ScanOutcome {
        self.env.clock.advance(60_000);
        scan_folder(
            self.env.ingest.env(),
            folder_id,
            &|| false,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    pub fn completed(&self, folder_id: i64) -> ScanCounts {
        match self.scan(folder_id) {
            ScanOutcome::Completed { counts } => counts,
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    /// 폴더 트리를 복사한다 (파일 수정 시각 보존 — 같은 파일의 복사본으로 보이게).
    pub fn copy_tree(&self, from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for e in fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dst = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                self.copy_tree(&e.path(), &dst);
            } else {
                fs::copy(e.path(), &dst).unwrap();
                set_mtime(&dst, fs::metadata(e.path()).unwrap().modified().unwrap());
            }
        }
    }

    pub fn relocate(
        &self,
        folder_id: i64,
        to: &Path,
    ) -> Result<mam_core::ingest::RelocateOutcome, mam_core::scan::IngestError> {
        self.env
            .ingest
            .relocate_folder(folder_id, to.to_str().unwrap())
    }

    pub fn asset_id(&self, name: &str) -> i64 {
        self.env.count(&format!(
            "SELECT asset_id FROM file_locations WHERE rel_path LIKE '%{name}' AND detached_at IS NULL"
        ))
    }

    pub fn volume_id(&self, key: &str) -> i64 {
        self.env.count(&format!(
            "SELECT volume_id FROM volumes WHERE volume_key = '{key}'"
        ))
    }

    pub fn tag(&self, asset_id: i64, name: &str) {
        let name = name.to_string();
        self.env
            .pool
            .write(mam_db::Priority::High, move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO tags (name, name_key) VALUES (?1, ?1)",
                    [&name],
                )?;
                c.execute(
                    "INSERT INTO asset_tags (asset_id, tag_id, created_at)
                     SELECT ?1, tag_id, 0 FROM tags WHERE name_key = ?2",
                    rusqlite::params![asset_id, &name],
                )?;
                Ok(())
            })
            .unwrap();
    }

    /// 상태 비교용 DB 덤프 (관리 폴더·위치·볼륨·작업·스캔 실행·색인 이벤트·자산 revision).
    pub fn db_state(&self) -> Vec<String> {
        self.env.read(|c| {
            let mut out = Vec::new();
            for sql in [
                "SELECT * FROM managed_folders ORDER BY folder_id",
                "SELECT * FROM file_locations ORDER BY location_id",
                "SELECT * FROM volumes ORDER BY volume_id",
                "SELECT * FROM jobs ORDER BY job_id",
                "SELECT * FROM scan_runs ORDER BY run_id",
                "SELECT * FROM search_outbox ORDER BY event_id",
                "SELECT asset_id, revision, status FROM media_assets ORDER BY asset_id",
            ] {
                out.extend(dump(c, sql));
            }
            out
        })
    }
}

fn dump(c: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = c.prepare(sql).unwrap();
    let n = stmt.column_count();
    stmt.query_map([], |r| {
        Ok((0..n)
            .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
            .collect::<Vec<_>>()
            .join("|"))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

pub fn set_mtime(path: &Path, t: SystemTime) {
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(t).unwrap();
}

pub fn hours_later(path: &Path, h: u64) {
    let t = fs::metadata(path).unwrap().modified().unwrap() + Duration::from_secs(h * 3600);
    set_mtime(path, t);
}
