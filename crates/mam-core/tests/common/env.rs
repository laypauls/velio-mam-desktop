//! 수집(P2-2) 통합 테스트 환경 — 실제 SQLite 풀 + 수동 전진 시계 + 가짜 볼륨(임시 디렉터리 = 볼륨 루트).
#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_core::ingest::{Ingest, IngestConfig, IngestDeps, PendingConfig, SettleGate};
use mam_core::volume::strip_verbatim;
use mam_core::{
    Blake3Hasher, CoreError, SharedClock, VolumeInfo, VolumeKind, VolumeProvider,
    WindowsFileIdentity,
};
use mam_db::{Clock, DbPool, migrate, open};
use rusqlite::Connection;
use tempfile::TempDir;

pub const T0: i64 = 1_700_000_000_000;

/// 스레드 간에 공유하는 수동 시계.
pub struct AtomicClock(AtomicI64);

impl AtomicClock {
    pub fn new() -> Self {
        Self(AtomicI64::new(T0))
    }
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for AtomicClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// 가짜 볼륨 제공자: 등록한 볼륨 루트 아래 경로는 그 볼륨에 속한다. `connected` 에서 빼면 "분리"된다.
pub struct FakeVolumes {
    vols: Mutex<Vec<VolumeInfo>>,
    connected: Mutex<HashSet<String>>,
    sizes: Mutex<std::collections::HashMap<String, u64>>,
    /// `list_volumes` 호출마다 신호를 보낸다 (점검 스레드 진행을 폴링 없이 기다리는 시험용)
    observer: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// 볼륨 키 -> 장치 조회 결과 (없으면 `Some(true)`, `None` = 조회 실패)
    ejectable: Mutex<std::collections::HashMap<String, Option<bool>>>,
}

impl FakeVolumes {
    pub fn new() -> Self {
        Self {
            vols: Mutex::new(Vec::new()),
            connected: Mutex::new(HashSet::new()),
            sizes: Mutex::new(Default::default()),
            observer: Mutex::new(None),
            ejectable: Mutex::new(Default::default()),
        }
    }
    pub fn add(&self, key: &str, kind: VolumeKind, mount: &Path) {
        self.vols.lock().unwrap().push(VolumeInfo {
            key: key.to_string(),
            kind,
            label: Some(key.to_string()),
            mount_point: strip_verbatim(&mount.to_string_lossy()),
        });
        self.connected.lock().unwrap().insert(key.to_string());
    }
    /// 같은 볼륨(키)이 다른 마운트 지점(드라이브 문자)으로 다시 연결된 것처럼 바꾼다.
    pub fn remount(&self, key: &str, mount: &Path) {
        for v in self.vols.lock().unwrap().iter_mut().filter(|v| v.key == key) {
            v.mount_point = strip_verbatim(&mount.to_string_lossy());
        }
    }
    pub fn set_label(&self, key: &str, label: &str) {
        for v in self.vols.lock().unwrap().iter_mut().filter(|v| v.key == key) {
            v.label = Some(label.to_string());
        }
    }
    pub fn set_ejectable(&self, key: &str, device: Option<bool>) {
        self.ejectable.lock().unwrap().insert(key.to_string(), device);
    }
    pub fn set_total_bytes(&self, key: &str, bytes: u64) {
        self.sizes.lock().unwrap().insert(key.to_string(), bytes);
    }
    pub fn observe(&self) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.observer.lock().unwrap() = Some(tx);
        rx
    }
    pub fn set_connected(&self, key: &str, on: bool) {
        let mut c = self.connected.lock().unwrap();
        if on {
            c.insert(key.to_string());
        } else {
            c.remove(key);
        }
    }
}

fn norm(s: &str) -> String {
    strip_verbatim(s).replace('/', "\\").to_lowercase()
}

impl VolumeProvider for FakeVolumes {
    fn is_ejectable_device(&self, mount_point: &str) -> Option<bool> {
        let m = norm(mount_point);
        let key = self.vols.lock().unwrap().iter().find(|v| norm(&v.mount_point) == m)?.key.clone();
        self.ejectable.lock().unwrap().get(&key).copied().unwrap_or(Some(true))
    }

    fn total_bytes(&self, mount_point: &str) -> Option<u64> {
        let m = norm(mount_point);
        let vols = self.vols.lock().unwrap();
        let v = vols.iter().find(|v| norm(&v.mount_point) == m)?;
        self.sizes.lock().unwrap().get(&v.key).copied()
    }

    fn volume_of(&self, path: &Path) -> Result<VolumeInfo, CoreError> {
        let p = norm(&path.to_string_lossy());
        self.vols
            .lock()
            .unwrap()
            .iter()
            .find(|v| {
                let m = norm(&v.mount_point);
                let m = m.trim_end_matches('\\');
                p == m || p.starts_with(&format!("{m}\\"))
            })
            .cloned()
            .ok_or_else(|| CoreError::Volume(format!("no fake volume for {}", path.display())))
    }

    fn list_volumes(&self) -> Result<Vec<VolumeInfo>, CoreError> {
        if let Some(tx) = self.observer.lock().unwrap().as_ref() {
            let _ = tx.send(());
        }
        let c = self.connected.lock().unwrap();
        Ok(self
            .vols
            .lock()
            .unwrap()
            .iter()
            .filter(|v| c.contains(&v.key))
            .cloned()
            .collect())
    }
}

pub struct Env {
    pub dir: TempDir,
    pub pool: Arc<DbPool>,
    pub clock: Arc<AtomicClock>,
    pub volumes: Arc<FakeVolumes>,
    pub ingest: Arc<Ingest>,
    /// 첫 번째(기본) 볼륨 루트
    pub vol: PathBuf,
    pub data: PathBuf,
}

impl Env {
    pub fn new() -> Self {
        Self::with_config(|_, _| {})
    }

    pub fn with_config(tweak: impl FnOnce(&mut IngestConfig, &Path)) -> Self {
        Self::with_identity(tweak, Arc::new(WindowsFileIdentity))
    }

    /// 파일 ID 제공자를 바꾼다 (`MetadataOnly` = 파일 ID 없는 FAT/NAS 와 같은 동작).
    pub fn with_identity(
        tweak: impl FnOnce(&mut IngestConfig, &Path),
        identity: Arc<dyn mam_core::FileIdentityProvider + Send + Sync>,
    ) -> Self {
        Self::with_deps(tweak, identity, Arc::new(Blake3Hasher))
    }

    /// 스캔이 쓰는 해시 계산기까지 바꾼다 (호출 횟수 세기용).
    pub fn with_deps(
        tweak: impl FnOnce(&mut IngestConfig, &Path),
        identity: Arc<dyn mam_core::FileIdentityProvider + Send + Sync>,
        hasher: Arc<dyn mam_core::ContentHasher + Send + Sync>,
    ) -> Self {
        let dir = TempDir::new().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let data = base.join("MAMData");
        let vol = base.join("vol_a");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&vol).unwrap();
        let db_path = data.join("mam.sqlite");
        let clock = Arc::new(AtomicClock::new());
        let mut conn = open(&db_path).unwrap();
        migrate(&mut conn, &data.join("backups"), &*clock).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db_path, 2).unwrap());
        let volumes = Arc::new(FakeVolumes::new());
        volumes.add("vol-a", VolumeKind::Fixed, &vol);
        let mut config = IngestConfig {
            auto_excludes: vec![data.clone()],
            debounce_ms: 150,
            // 안정화 게이트도 시험용으로 압축 (제품 기본은 표본 3회·간격 2초) — 주입 시계로 판단한다
            pending: PendingConfig {
                gate: SettleGate {
                    samples: 3,
                    min_gap_ms: 100,
                    give_up_ms: 600_000,
                },
                ..PendingConfig::default()
            },
            supervise_every: Duration::from_millis(200),
            ..IngestConfig::default()
        };
        tweak(&mut config, &vol);
        let shared: SharedClock = clock.clone();
        let ingest = Ingest::new(
            IngestDeps {
                pool: Arc::clone(&pool),
                clock: shared,
                volumes: volumes.clone(),
                identity,
                hasher,
            },
            config,
        );
        Self {
            dir,
            pool,
            clock,
            volumes,
            ingest,
            vol,
            data,
        }
    }

    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        self.pool
            .read(|c| Ok(f(c)))
            .unwrap_or_else(|e| panic!("read failed: {e}"))
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.read(|c| c.query_row(sql, [], |r| r.get(0)).unwrap())
    }

    /// 볼륨 루트 아래에 파일을 만든다 (부모 디렉터리 포함). 내용은 `seed` 로 달라진다.
    pub fn write(&self, rel: &str, seed: u8) -> PathBuf {
        let p = rel.split('/').fold(self.vol.clone(), |p, seg| p.join(seg));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, vec![seed; 64 + usize::from(seed)]).unwrap();
        p
    }

    pub fn clock_now(&self) -> i64 {
        self.clock.now_ms()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.vol.clone(), |p, seg| p.join(seg))
    }
}

/// 조건이 참이 될 때까지 폴링한다 (스레드·파일시스템 이벤트 대기 전용; 실패 시 호출자가 단언).
pub fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    cond()
}

/// 디렉터리 전체의 (상대 경로, 크기, 수정 시각 ms, blake3) 스냅샷 — 원본 보호 단언용.
pub fn snapshot(root: &Path) -> Vec<(String, u64, u128, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, u64, u128, String)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            let meta = fs::metadata(&p).unwrap();
            if meta.is_dir() {
                out.push((format!("{rel}/"), 0, 0, String::new()));
                walk(&p, root, out);
            } else {
                let mtime = meta
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis();
                let hash = blake3::hash(&fs::read(&p).unwrap()).to_hex().to_string();
                out.push((rel, meta.len(), mtime, hash));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}
