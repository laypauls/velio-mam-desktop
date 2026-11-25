//! 통합 테스트 공용 헬퍼 — 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 + 주입 시계.
#![allow(dead_code)] // 테스트 파일마다 쓰는 헬퍼가 다르다

use std::cell::Cell;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use mam_core::{
    Blake3Hasher, CoreError, FileIdentityProvider, MediaType, Outcome, Reconciler, ScanEntry,
    ScanSession, VolumeInfo, VolumeKind, register_volume, relative_path,
};
use mam_db::{Clock, migrate, open};
use rusqlite::Connection;
use tempfile::TempDir;

pub const T0: i64 = 1_700_000_000_000;

/// 고정·수동 전진 시계 (tests.md: 현재 시각을 직접 읽지 않는다)
pub struct TestClock(Cell<i64>);

impl TestClock {
    pub fn new() -> Self {
        Self(Cell::new(T0))
    }
    pub fn advance(&self, ms: i64) {
        self.0.set(self.0.get() + ms);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.get()
    }
}

/// 마이그레이션이 적용된 실제 SQLite 임시 파일.
pub struct TestDb {
    pub _dir: TempDir,
    pub conn: Connection,
}

pub fn migrated_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let mut conn = open(&dir.path().join("mam.sqlite")).unwrap();
    migrate(&mut conn, &dir.path().join("backups"), &TestClock::new()).unwrap();
    TestDb { _dir: dir, conn }
}

/// 가짜 볼륨: 임시 디렉터리 하나 = 볼륨 하나 (USB/NAS/드라이브 문자 변경은 이 값을 바꿔 흉내낸다).
pub struct Vol {
    pub dir: TempDir,
    pub info: VolumeInfo,
    pub id: i64,
}

impl Vol {
    pub fn new(db: &TestDb, clock: &TestClock, key: &str, kind: VolumeKind) -> Self {
        let dir = TempDir::new().unwrap();
        let info = VolumeInfo {
            key: key.to_string(),
            kind,
            label: None,
            mount_point: dir.path().to_string_lossy().into_owned(),
        };
        let id = register_volume(&db.conn, clock, &info).unwrap();
        Self { dir, info, id }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.dir.path().to_path_buf(), |p, seg| p.join(seg))
    }

    pub fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = self.path(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, bytes).unwrap();
        p
    }
}

pub fn set_mtime(path: &Path, ms_after_epoch: u64) {
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_millis(ms_after_epoch))
        .unwrap();
}

/// 스캔 1건: 메타 수집 -> 상대 경로 계산 -> reconcile
pub fn scan(
    conn: &mut Connection,
    rec: &Reconciler<'_>,
    session: &mut ScanSession,
    ident: &dyn FileIdentityProvider,
    vol: &Vol,
    path: &Path,
) -> Result<Outcome, CoreError> {
    let meta = ident.stat(path)?;
    let rel = relative_path(&vol.info.mount_point, path)?;
    rec.reconcile(
        conn,
        session,
        &ScanEntry {
            rel_path: &rel,
            abs_path: path,
            media_type: MediaType::Image,
            meta,
        },
    )
}

pub fn cancel_flag_unused() -> AtomicBool {
    AtomicBool::new(false)
}

pub fn reconciler<'a>(clock: &'a TestClock, cancel: &'a AtomicBool) -> Reconciler<'a> {
    Reconciler {
        clock,
        hasher: &Blake3Hasher,
        cancel,
    }
}

// --- DB 조회 헬퍼 -------------------------------------------------------

pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

pub struct AssetState {
    pub status: String,
    pub revision: i64,
    pub hash_state: String,
    pub size: i64,
}

pub fn asset_state(conn: &Connection, asset_id: i64) -> AssetState {
    conn.query_row(
        "SELECT status, revision, hash_state, file_size FROM media_assets WHERE asset_id = ?1",
        [asset_id],
        |r| {
            Ok(AssetState {
                status: r.get(0)?,
                revision: r.get(1)?,
                hash_state: r.get(2)?,
                size: r.get(3)?,
            })
        },
    )
    .unwrap()
}

/// (asset_id, revision, event_type) — event_id 순
pub fn outbox(conn: &Connection) -> Vec<(i64, i64, String)> {
    let mut stmt = conn
        .prepare("SELECT asset_id, revision, event_type FROM search_outbox ORDER BY event_id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

pub struct ActiveLoc {
    pub asset_id: i64,
    pub volume_id: i64,
    pub rel_path: String,
    pub presence: String,
}

/// 활성(detach 되지 않은) 위치 전부
pub fn active_locations(conn: &Connection) -> Vec<ActiveLoc> {
    let mut stmt = conn
        .prepare(
            "SELECT asset_id, volume_id, rel_path, presence FROM file_locations
             WHERE detached_at IS NULL ORDER BY location_id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok(ActiveLoc {
            asset_id: r.get(0)?,
            volume_id: r.get(1)?,
            rel_path: r.get(2)?,
            presence: r.get(3)?,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

pub fn created_id(o: Outcome) -> i64 {
    match o {
        Outcome::Created { asset_id } => asset_id,
        other => panic!("expected Created, got {other:?}"),
    }
}

// --- 원본 보호 가드 -----------------------------------------------------

/// 픽스처 디렉터리의 파일 목록·내용·수정 시각 스냅샷 (tests.md: 원본 보호 회귀 가드)
#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot(Vec<(PathBuf, Vec<u8>, SystemTime)>);

impl Snapshot {
    pub fn take(root: &Path) -> Self {
        let mut out = Vec::new();
        collect(root, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Self(out)
    }
}

fn collect(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>, SystemTime)>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            collect(&p, out);
        } else {
            let mtime = File::open(&p)
                .unwrap()
                .metadata()
                .unwrap()
                .modified()
                .unwrap();
            out.push((p.clone(), fs::read(&p).unwrap(), mtime));
        }
    }
}

// --- 시험 세계 ----------------------------------------------------------

/// DB + 시계 + 취소 플래그. reconcile 호출을 필드 분리 차용으로 감싼다.
pub struct World {
    pub db: TestDb,
    pub clock: TestClock,
    pub cancel: AtomicBool,
}

impl World {
    pub fn new() -> Self {
        Self {
            db: migrated_db(),
            clock: TestClock::new(),
            cancel: AtomicBool::new(false),
        }
    }

    pub fn volume(&self, key: &str, kind: VolumeKind) -> Vol {
        Vol::new(&self.db, &self.clock, key, kind)
    }

    /// 새 스캔 세션 — 시계를 1초 전진시켜 `scan_at` 이 직전 스캔보다 커지게 한다
    pub fn begin(&self, vol: &Vol) -> ScanSession {
        self.clock.advance(1_000);
        ScanSession::begin(&self.db.conn, &self.clock, vol.id).unwrap()
    }

    pub fn scan(
        &mut self,
        vol: &Vol,
        session: &mut ScanSession,
        ident: &dyn FileIdentityProvider,
        path: &Path,
    ) -> Result<Outcome, CoreError> {
        let rec = Reconciler {
            clock: &self.clock,
            hasher: &Blake3Hasher,
            cancel: &self.cancel,
        };
        scan(&mut self.db.conn, &rec, session, ident, vol, path)
    }

    pub fn finish(&mut self, session: &ScanSession) -> usize {
        self.clock.advance(1);
        let rec = Reconciler {
            clock: &self.clock,
            hasher: &Blake3Hasher,
            cancel: &self.cancel,
        };
        rec.finish_scan(&mut self.db.conn, session).unwrap()
    }
}

// --- 가짜 제공자 --------------------------------------------------------

/// 열거 결과를 시험이 직접 정하는 볼륨 제공자 (USB 탈착·드라이브 문자 변경 시뮬레이션)
pub struct FakeVolumes(pub Vec<VolumeInfo>);

impl mam_core::VolumeProvider for FakeVolumes {
    fn volume_of(&self, path: &Path) -> Result<VolumeInfo, CoreError> {
        self.0
            .iter()
            .find(|v| relative_path(&v.mount_point, path).is_ok())
            .cloned()
            .ok_or_else(|| CoreError::Volume(format!("no fake volume for {}", path.display())))
    }
    fn list_volumes(&self) -> Result<Vec<VolumeInfo>, CoreError> {
        Ok(self.0.clone())
    }
}

/// 항상 "해시 도중 변경" 을 보고하는 해시 계산기
pub struct AlwaysChanged;

impl mam_core::ContentHasher for AlwaysChanged {
    fn hash_file(&self, path: &Path, _: &AtomicBool) -> Result<mam_core::HashedFile, CoreError> {
        Err(CoreError::ChangedDuringHash(path.to_path_buf()))
    }
}

pub mod env;
pub mod proc_env;
pub mod purge;
pub mod reloc;
pub mod umeta;
