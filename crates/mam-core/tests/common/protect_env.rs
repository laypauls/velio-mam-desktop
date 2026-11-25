//! 보호(P2-9) 시험 환경 — 실제 SQLite + 임시 디렉터리 볼륨 + 주입 시계 + 휴지통 대역.
#![allow(dead_code)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use mam_core::protect::{ProtectError, ProtectionGuard, Trash};
use mam_core::{MetadataOnly, VolumeKind};

use crate::common::env::AtomicClock;
use crate::common::{Vol, World, created_id};

pub const PASSWORD: &str = "correct-horse-battery";

/// Argon2 비용 때문에 시험마다 새로 해시하지 않고 공유한다
pub fn shared_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let guard = ProtectionGuard::new(Arc::new(AtomicClock::new()));
        guard.set_password(None, PASSWORD).unwrap()
    })
}

/// 호출을 기록하고, 성공하면 실제로 파일을 지워 휴지통 이동을 흉내 내는 대역 (임시 디렉터리 픽스처 전용)
#[derive(Default)]
pub struct RecordingTrash {
    pub calls: Mutex<Vec<PathBuf>>,
    pub fail_names: Mutex<HashSet<String>>,
}

impl RecordingTrash {
    pub fn fail_for(&self, file_name: &str) {
        self.fail_names
            .lock()
            .unwrap()
            .insert(file_name.to_string());
    }
    pub fn calls(&self) -> Vec<PathBuf> {
        self.calls.lock().unwrap().clone()
    }
}

impl Trash for RecordingTrash {
    fn trash(&self, path: &Path) -> Result<(), ProtectError> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if self.fail_names.lock().unwrap().contains(&name) {
            return Err(ProtectError::Io(format!("simulated failure: {name}")));
        }
        std::fs::remove_file(path).map_err(|e| ProtectError::Io(e.to_string()))
    }
}

pub struct Fx {
    pub w: World,
    pub vol: Vol,
    pub guard_clock: Arc<AtomicClock>,
    pub guard: ProtectionGuard,
    pub trash: RecordingTrash,
}

impl Fx {
    pub fn new() -> Self {
        let w = World::new();
        let vol = w.volume("vol-a", VolumeKind::Fixed);
        let guard_clock = Arc::new(AtomicClock::new());
        let guard = ProtectionGuard::new(guard_clock.clone());
        Self {
            w,
            vol,
            guard_clock,
            guard,
            trash: RecordingTrash::default(),
        }
    }

    /// 5분 면제로 인증된 상태로 만든다
    pub fn authenticate(&self) {
        self.guard
            .verify_password(Some(shared_hash()), PASSWORD, 5)
            .unwrap();
    }

    /// 파일을 쓰고 스캔해 자산을 만든다
    pub fn add(&mut self, rel: &str, bytes: &[u8]) -> i64 {
        add_to(&mut self.w, &self.vol, rel, bytes)
    }

    /// 볼륨 `rel` 폴더를 관리 폴더로 등록한다 (`""` = 볼륨 루트)
    pub fn manage(&self, vol: &Vol, rel: &str) {
        let key = rel.to_lowercase();
        self.w
            .db
            .conn
            .execute(
                "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at)
                 VALUES (?1, ?2, ?3, ?2, 0)",
                rusqlite::params![vol.id, rel, key],
            )
            .unwrap();
    }

    pub fn add_tag(&self, asset_id: i64, name: &str) {
        let c = &self.w.db.conn;
        c.execute("INSERT INTO tags (name, name_key) VALUES (?1, ?1)", [name])
            .unwrap();
        c.execute(
            "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, last_insert_rowid(), 0)",
            [asset_id],
        )
        .unwrap();
    }
}

pub fn add_to(w: &mut World, vol: &Vol, rel: &str, bytes: &[u8]) -> i64 {
    let path = vol.write(rel, bytes);
    let mut session = w.begin(vol);
    let outcome = w.scan(vol, &mut session, &MetadataOnly, &path).unwrap();
    w.finish(&session);
    created_id(outcome)
}
