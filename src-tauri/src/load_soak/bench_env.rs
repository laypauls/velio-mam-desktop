//! 측정 환경 — `lib.rs` 의 기동 순서(풀 → 수집 → 처리 → AI 실행기 → 검색)를 헤드리스로 재현한다.
//!
//! 제품과 다른 점: 외부 AI·STT·임베딩 인코더가 가짜이고(`fakes`), Tauri 이벤트·보호·백업 스케줄러는 없다. 나머지(DbPool, Ingest,
//! 스캔·감시, 메타·썸네일·해시 실행기, AI 스케줄러·실행기, SearchService 색인 워커, 하이브리드 검색)는 제품 코드다.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use mam_core::ai_jobs::{AiDeps, AiExecutor, AiJobsConfig, AiRuntimeSettings, AiScheduler};
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::jobs::{JobCounts, JobKind};
use mam_core::processing::ProcessingEnv;
use mam_core::query::QueryService;
use mam_core::scan::{add_folder_row, root_keys};
use mam_core::storage::DataPaths;
use mam_core::userdata::UserData;
use mam_core::volume::VolumeProvider;
use mam_core::{Blake3Hasher, SharedClock, WindowsFileIdentity, WindowsVolumes, register_volume};
use mam_db::{DEFAULT_READERS, DbPool, SystemClock, migrate, open};
use mam_media::{MediaTools, ProcessRunner, ToolConfig};
use mam_vector::EmbedStore;

use super::fakes::{EmbedStats, FakeExternal, FakeStt, FixedSettings, SynthEmbedding, SynthQuery};
use crate::search_service::SearchService;

#[derive(Clone)]
pub struct Options {
    /// 감시 디바운스 (제품 기본 2000)
    pub debounce_ms: i64,
    /// 주기 점검 간격 (제품 기본 30초)
    pub supervise_every: Duration,
    /// 가짜 외부 AI 응답 지연
    pub external_latency: Duration,
    /// 가짜 STT 지연
    pub stt_latency: Duration,
    /// 메타·썸네일 작업 동시 실행 수 (None = 제품 기본값)
    pub meta: Option<usize>,
    pub thumb: Option<usize>,
    /// ffmpeg `-threads` (None = 제품 기본값)
    pub ffmpeg_threads: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        let d = IngestConfig::default();
        Self {
            debounce_ms: d.debounce_ms,
            supervise_every: d.supervise_every,
            external_latency: Duration::from_millis(50),
            stt_latency: Duration::from_millis(20),
            meta: None,
            thumb: None,
            ffmpeg_threads: None,
        }
    }
}

pub struct Bench {
    pub paths: DataPaths,
    pub pool: Arc<DbPool>,
    pub ingest: Arc<Ingest>,
    pub search: Arc<SearchService>,
    pub user: UserData,
    pub scheduler: AiScheduler,
    pub store: Arc<EmbedStore>,
    pub rt: AiRuntimeSettings,
    pub ext: Arc<FakeExternal>,
    pub stt: Arc<FakeStt>,
    pub embed_stats: Arc<EmbedStats>,
}

/// `root/MAMData` 레이아웃·DB 를 만들고 마이그레이션한다. 반환: (데이터 경로, 열린 연결 — 시드용).
pub fn prepare_db(root: &Path) -> (DataPaths, rusqlite::Connection) {
    let paths = DataPaths::new(root.join("MAMData"));
    for d in [
        paths.database_dir(),
        paths.cache(),
        paths.backups(),
        paths.tmp(),
        paths.root().join("indexes").join("vectors"),
    ] {
        std::fs::create_dir_all(d).expect("create MAMData layout");
    }
    let mut conn = open(&paths.db()).expect("open db");
    migrate(&mut conn, &paths.backups(), &SystemClock).expect("migrate");
    (paths, conn)
}

/// 실제 볼륨 API 로 `dir` 를 관리 폴더 행으로 등록한다 (스캔 작업은 만들지 않는다) — 합성 자산을 이 폴더 아래에 적재할 때 쓴다.
/// 반환: (volume_id, folder_id, 볼륨 기준 rel_root)
pub fn register_folder_row(conn: &mut rusqlite::Connection, dir: &Path) -> (i64, i64, String) {
    std::fs::create_dir_all(dir).expect("create folder");
    let canonical = std::fs::canonicalize(dir).expect("canonicalize");
    let info = WindowsVolumes.volume_of(&canonical).expect("volume_of");
    let (rel_root, key) = root_keys(&info, &canonical).expect("root_keys");
    let volume_id = register_volume(conn, &SystemClock, &info).expect("register volume");
    let display = mam_core::volume::strip_verbatim(&canonical.to_string_lossy());
    let now = mam_db::Clock::now_ms(&SystemClock);
    let folder_id = add_folder_row(
        conn,
        now,
        volume_id,
        &rel_root,
        &key,
        &display,
        Default::default(),
    )
    .expect("folder row");
    // 합성 라이브러리는 "방금 전체 스캔을 마친" 상태 — 주기 재스캔 대상이 아니다
    conn.execute(
        "UPDATE managed_folders SET last_full_scan_at = ?1 WHERE folder_id = ?2",
        rusqlite::params![now, folder_id],
    )
    .expect("mark scanned");
    (volume_id, folder_id, rel_root)
}

impl Bench {
    /// 제품 `lib.rs` 7~8b 단계와 같은 순서로 서비스를 올린다. `prepare_db` 이후에 호출한다.
    pub fn start(root: &Path, opts: &Options) -> Arc<Self> {
        let paths = DataPaths::new(root.join("MAMData"));
        let pool = Arc::new(DbPool::open(&paths.db(), DEFAULT_READERS).expect("pool"));
        let jobs_cfg = AiJobsConfig::new(paths.cache());
        let mut caps = jobs_cfg.queue_caps();
        for (cap, kind) in caps.iter_mut().zip(JobKind::ALL) {
            match kind {
                JobKind::Meta => *cap = opts.meta.unwrap_or(*cap),
                JobKind::Thumb => *cap = opts.thumb.unwrap_or(*cap),
                _ => {}
            }
        }
        let clock: SharedClock = Arc::new(SystemClock);
        let ingest = Ingest::new(
            IngestDeps {
                pool: Arc::clone(&pool),
                clock: Arc::clone(&clock),
                volumes: Arc::new(WindowsVolumes),
                identity: Arc::new(WindowsFileIdentity),
                hasher: Arc::new(Blake3Hasher),
            },
            IngestConfig {
                auto_excludes: vec![paths.root().to_path_buf(), paths.backups()],
                debounce_ms: opts.debounce_ms,
                supervise_every: opts.supervise_every,
                concurrency: caps,
                ..IngestConfig::default()
            },
        );
        let mut tools = MediaTools::discover(&ToolConfig::default(), Arc::new(ProcessRunner));
        if let Some(t) = opts.ffmpeg_threads {
            tools.threads = t;
        }
        let tools = Arc::new(tools);
        ingest.enable_processing(ProcessingEnv::new(
            Arc::clone(&pool),
            Arc::clone(&clock),
            tools,
            Arc::new(Blake3Hasher),
            paths.cache(),
        ));
        let store = Arc::new(
            EmbedStore::new(paths.root().join("indexes").join("vectors")).expect("embed store"),
        );
        let ext = Arc::new(FakeExternal {
            latency: opts.external_latency,
            calls: AtomicU64::new(0),
        });
        let stt = Arc::new(FakeStt {
            latency: opts.stt_latency,
            calls: AtomicU64::new(0),
        });
        let embed_stats = Arc::new(EmbedStats::default());
        let settings = FixedSettings::on();
        let rt = settings.0.clone();
        let executor = AiExecutor::new(
            AiDeps {
                pool: Arc::clone(&pool),
                clock: Arc::clone(&clock),
                settings: Arc::new(settings),
                external: Arc::clone(&ext) as _,
                stt: Arc::clone(&stt) as _,
                embedding: Arc::new(SynthEmbedding {
                    pool: Arc::clone(&pool),
                    clock: Arc::clone(&clock),
                    store: Arc::clone(&store),
                    data_root: paths.root().to_path_buf(),
                    stats: Arc::clone(&embed_stats),
                }),
            },
            jobs_cfg.clone(),
        );
        ingest.register_executor(JobKind::Ai, Arc::new(executor));
        ingest.start().expect("ingest start");
        let user = UserData::new(Arc::clone(&pool), Arc::clone(&clock));
        let query = Arc::new(QueryService::new(Arc::clone(&pool), Arc::clone(&clock)));
        let search = SearchService::start(
            Arc::clone(&pool),
            Arc::clone(&clock),
            paths.tantivy(),
            Arc::clone(&query),
        );
        search.set_query_embedder(Arc::new(SynthQuery));
        search.set_embed_store(Arc::clone(&store));
        let scheduler = AiScheduler::new(
            Arc::clone(&pool),
            Arc::clone(ingest.jobs()),
            clock,
            jobs_cfg.sweep_batch,
        );
        Arc::new(Self {
            paths,
            pool,
            ingest,
            search,
            user,
            scheduler,
            store,
            rt,
            ext,
            stt,
            embed_stats,
        })
    }

    /// 제품 종료 순서: 색인 워커 → 수집 → DB 풀.
    pub fn shutdown(&self) {
        self.search.shutdown();
        self.ingest.shutdown();
        self.pool.shutdown();
    }

    /// 대기 중 색인 이벤트 수 (pending).
    pub fn outbox_pending(&self) -> i64 {
        self.count("SELECT count(*) FROM search_outbox WHERE status = 'pending'")
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.pool
            .read(|c| Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))?))
            .unwrap_or(-1)
    }

    /// 종류별 작업 수 (pending, running, failed).
    pub fn job_counts(&self) -> Vec<(JobKind, JobCounts)> {
        self.ingest.jobs().counts().unwrap_or_default()
    }

    /// (pending 합, running 합) — 스캔·메타·썸네일·해시·AI 모두.
    pub fn jobs_busy(&self) -> (u64, u64) {
        let c = self.job_counts();
        (
            c.iter().map(|(_, n)| n.pending).sum(),
            c.iter().map(|(_, n)| n.running).sum(),
        )
    }

    /// 인덱스 문서 수 기준 비교용: 활성 자산 + 활성 위치(파일 없음 포함) 수.
    pub fn sqlite_active_docs(&self) -> i64 {
        self.count(
            "SELECT count(*) FROM media_assets a
             JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
             WHERE a.status = 'active'",
        )
    }
}

impl Bench {
    /// 임의 SELECT 의 각 행을 " | " 로 이은 문자열로 (진단 출력용).
    pub fn rows(&self, sql: &str) -> Vec<String> {
        use rusqlite::types::Value;
        self.pool
            .read(|c| {
                let mut st = c.prepare(sql)?;
                let n = st.column_count();
                let rows = st.query_map([], |r| {
                    let mut parts = Vec::with_capacity(n);
                    for i in 0..n {
                        parts.push(match r.get::<_, Value>(i)? {
                            Value::Null => "NULL".to_string(),
                            Value::Integer(v) => v.to_string(),
                            Value::Real(v) => v.to_string(),
                            Value::Text(s) => s,
                            Value::Blob(_) => "<blob>".to_string(),
                        });
                    }
                    Ok(parts.join(" | "))
                })?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap_or_default()
    }

    /// 실패 사유 요약 출력 (파생물·작업·AI 단계·색인 이벤트).
    pub fn print_failure_reasons(&self) {
        println!("\n### 실패 사유 (있을 때만)");
        for (label, sql) in [
            (
                "파생물",
                "SELECT kind, substr(error, 1, 140), count(*) FROM asset_derivatives WHERE state = 'failed' GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 6",
            ),
            (
                "작업",
                "SELECT kind, attempts, substr(last_error, 1, 140), count(*) FROM jobs WHERE status = 'failed' GROUP BY 1, 2, 3 ORDER BY 4 DESC LIMIT 6",
            ),
            (
                "AI 단계",
                "SELECT kind, error_code, count(*) FROM ai_task_state WHERE state = 'failed' GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 6",
            ),
            (
                "색인 이벤트",
                "SELECT attempts, substr(last_error, 1, 140), count(*) FROM search_outbox WHERE status = 'failed' GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 6",
            ),
            (
                "스캔 오류",
                "SELECT error_code, substr(message, 1, 120), count(*) FROM scan_errors WHERE resolved_at IS NULL GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 6",
            ),
        ] {
            for row in self.rows(sql) {
                println!("- {label}: {row}");
            }
        }
    }
}
