//! AI 시험 공용 환경 — 실제 SQLite(마이그레이션 적용) + 가짜 전송·이벤트·시계. 외부 AI 실호출은 없다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mam_ai::external::cli::fake::{FakeRunner, Step};
use mam_ai::external::cli::{CliCommand, CliEnv, UrlOpener};
use mam_ai::external::{HttpRequest, HttpResponse, HttpTransport, TransportError};
use mam_ai::models::{Manifest, ModelStore};
use mam_core::SharedClock;
use mam_core::ai_jobs::AiJobsConfig;
use mam_core::jobs::JobQueue;
use mam_db::{Clock, DbPool, SystemClock, migrate, open};

use super::embed::LocalEmbedders;
use super::events::EventSink;
use super::models::ModelService;
use super::service::{AiService, AiServiceDeps};

pub struct TestClock(pub AtomicI64);

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// 요청 수를 세고 미리 정한 응답을 돌려주는 전송. 받은 요청의 URL·본문을 기록한다.
pub struct StubTransport {
    pub calls: AtomicUsize,
    pub reply: Mutex<Box<dyn Fn() -> Result<HttpResponse, TransportError> + Send>>,
    pub seen: Mutex<Vec<(String, Vec<u8>)>>,
}

impl StubTransport {
    pub fn new(status: u16, body: &str) -> Arc<Self> {
        let body = body.as_bytes().to_vec();
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            reply: Mutex::new(Box::new(move || {
                Ok(HttpResponse {
                    status,
                    headers: vec![],
                    body: body.clone(),
                })
            })),
            seen: Mutex::new(Vec::new()),
        })
    }

    pub fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl HttpTransport for StubTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap()
            .push((req.url.clone(), req.body.clone()));
        (self.reply.lock().unwrap())()
    }
}

#[derive(Default)]
pub struct CollectSink(pub Mutex<Vec<(String, serde_json::Value)>>);

impl EventSink for CollectSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

/// 구독 CLI 대역 — 실제 `claude` 를 띄우지 않는다. 각본(`scripts`)은 시험이 실행 중에도 바꿀 수 있다.
pub struct CliScripts {
    pub version: Vec<Step>,
    pub status: Vec<Step>,
    pub request: Vec<Step>,
    pub login: Vec<Step>,
    pub logout: Vec<Step>,
}

pub fn cli_line(s: &str) -> Step {
    Step::Out(format!("{s}\n"))
}

impl Default for CliScripts {
    fn default() -> Self {
        let init = r#"{"type":"system","subtype":"init","apiKeySource":"none","model":"claude-haiku-5-5"}"#;
        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"{}","structured_output":{"description":"붉은 노을","keywords":["노을"],"ocrText":"","summary":"","language":"ko"},"usage":{"input_tokens":5,"cache_creation_input_tokens":100,"output_tokens":20}}"#;
        Self {
            version: vec![cli_line("2.1.318 (Claude Code)"), Step::Exit(Some(0))],
            status: vec![
                cli_line(
                    r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","email":"me@example.com"}"#,
                ),
                Step::Exit(Some(0)),
            ],
            request: vec![cli_line(init), cli_line(result), Step::Exit(Some(0))],
            login: vec![Step::Hang],
            logout: vec![Step::Exit(Some(0))],
        }
    }
}

pub struct TestCliEnv {
    pub installed: std::sync::atomic::AtomicBool,
    pub vars: Mutex<std::collections::HashSet<String>>,
}

pub const TEST_CLI_EXE: &str = r"C:\fake\claude.exe";

impl CliEnv for TestCliEnv {
    fn path_var(&self) -> Option<std::ffi::OsString> {
        Some(r"C:\fake".into())
    }
    fn home_dir(&self) -> Option<PathBuf> {
        None
    }
    fn is_file(&self, p: &Path) -> bool {
        self.installed.load(Ordering::SeqCst) && p == Path::new(TEST_CLI_EXE)
    }
    fn read_to_string(&self, _p: &Path) -> Option<String> {
        None
    }
    fn has_var(&self, name: &str) -> bool {
        self.vars.lock().unwrap().contains(name)
    }
    fn is_windows(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct TestOpener(pub Mutex<Vec<String>>);

impl UrlOpener for TestOpener {
    fn open(&self, url: &str) -> std::io::Result<()> {
        self.0.lock().unwrap().push(url.to_string());
        Ok(())
    }
}

pub struct CliFx {
    pub runner: Arc<FakeRunner>,
    pub scripts: Arc<Mutex<CliScripts>>,
    pub env: Arc<TestCliEnv>,
    pub opener: Arc<TestOpener>,
}

impl CliFx {
    pub fn new() -> Self {
        let scripts: Arc<Mutex<CliScripts>> = Arc::default();
        let s = Arc::clone(&scripts);
        let runner = FakeRunner::new(move |c: &CliCommand| {
            let a: Vec<String> = c
                .args
                .iter()
                .map(|x| x.to_string_lossy().into_owned())
                .collect();
            let s = s.lock().unwrap();
            match (a.first().map(String::as_str), a.get(1).map(String::as_str)) {
                (Some("--version"), _) => Some(s.version.clone()),
                (Some("auth"), Some("status")) => Some(s.status.clone()),
                (Some("auth"), Some("login")) => Some(s.login.clone()),
                (Some("auth"), Some("logout")) => Some(s.logout.clone()),
                (Some("-p"), _) => Some(s.request.clone()),
                _ => None,
            }
        });
        Self {
            runner,
            scripts,
            env: Arc::new(TestCliEnv {
                installed: std::sync::atomic::AtomicBool::new(true),
                vars: Mutex::default(),
            }),
            opener: Arc::default(),
        }
    }
}

pub struct Fx {
    pub dir: tempfile::TempDir,
    pub cli: Arc<CliFx>,
    pub pool: Arc<DbPool>,
    pub clock: Arc<TestClock>,
    pub queue: Arc<JobQueue>,
    pub transport: Arc<StubTransport>,
    pub sink: Arc<CollectSink>,
    pub service: Arc<AiService>,
}

pub fn jpeg_bytes() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(32, 24, image::Rgb([200, 30, 30]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

pub fn openai_reply() -> String {
    let content = r#"{"description":"a red square","keywords":["red","square"],"ocrText":"","summary":"","language":"ko"}"#;
    serde_json::json!({
        "choices": [{"message": {"content": content}}],
        "usage": {"prompt_tokens": 12, "completion_tokens": 7}
    })
    .to_string()
}

impl Fx {
    pub fn new() -> Self {
        Self::with(StubTransport::new(200, &openai_reply()), 1_800_000_000_000)
    }

    pub fn with(transport: Arc<StubTransport>, now_ms: i64) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("mam.sqlite");
        let mut conn = open(&db).unwrap();
        migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
        conn.execute_batch(
            "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at)
             VALUES (1, 'fixed', 'v1', 'Main', 'C:\\Media', 'online', 0);",
        )
        .unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db, 2).unwrap());
        let clock = Arc::new(TestClock(AtomicI64::new(now_ms)));
        let shared: SharedClock = clock.clone();
        let queue = JobQueue::new(Arc::clone(&pool), Arc::clone(&shared));
        let sink = Arc::new(CollectSink::default());
        let models = ModelService::new(
            Arc::new(ModelStore::new(
                dir.path().join("models"),
                Manifest::builtin().unwrap(),
            )),
            sink.clone(),
        );
        let cli = Arc::new(CliFx::new());
        let service = build_service(
            &dir, &pool, &shared, &queue, &transport, &sink, models, &cli,
        );
        Self {
            dir,
            cli,
            pool,
            clock,
            queue,
            transport,
            sink,
            service,
        }
    }

    /// 같은 DB 위에 새 서비스를 만든다 (재시작 모사).
    pub fn restart(&self) -> Arc<AiService> {
        let shared: SharedClock = self.clock.clone();
        let models = ModelService::new(
            Arc::new(ModelStore::new(
                self.dir.path().join("models"),
                Manifest::builtin().unwrap(),
            )),
            self.sink.clone(),
        );
        build_service(
            &self.dir,
            &self.pool,
            &shared,
            &self.queue,
            &self.transport,
            &self.sink,
            models,
            &self.cli,
        )
    }

    pub fn insert_asset(&self, id: i64, media: &str, rel: &str) {
        let rel = rel.to_string();
        let media = media.to_string();
        self.pool
            .write(mam_db::Priority::High, move |c| {
                c.execute(
                    "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
                     VALUES (?1, ?2, 10, 1, 0, 0)",
                    rusqlite::params![id, media],
                )?;
                c.execute(
                    "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                     VALUES (?1, 1, ?2, lower(?2), 0)",
                    rusqlite::params![id, rel],
                )?;
                Ok(())
            })
            .unwrap();
    }

    pub fn write_thumb(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join("cache").join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, jpeg_bytes()).unwrap();
        p
    }

    pub fn consent_and_key(&self) {
        self.service.give_consent().unwrap();
        self.service
            .set_api_key("openai", "sk-test-SECRET-123456", false)
            .unwrap();
    }
}

#[allow(clippy::too_many_arguments)]
fn build_service(
    dir: &tempfile::TempDir,
    pool: &Arc<DbPool>,
    clock: &SharedClock,
    queue: &Arc<JobQueue>,
    transport: &Arc<StubTransport>,
    sink: &Arc<CollectSink>,
    models: Arc<ModelService>,
    cli: &CliFx,
) -> Arc<AiService> {
    let embed_store = Arc::new(mam_vector::EmbedStore::new(dir.path().join("vectors")).unwrap());
    AiService::new(AiServiceDeps {
        pool: Arc::clone(pool),
        clock: Arc::clone(clock),
        queue: Arc::clone(queue),
        transport: transport.clone(),
        embedders: Arc::new(LocalEmbedders::new(
            Arc::clone(models.store()),
            mam_ai::local::embed::EmbedConfig::default(),
        )),
        models,
        embed_store,
        sink: sink.clone(),
        stt_runner: Arc::new(mam_ai::local::stt::ProcessRunner),
        stt_tools: Arc::new(|| {
            Err(mam_ai::AiError::ToolNotFound {
                tool: "whisper-cli",
                searched: "test".into(),
            })
        }),
        stt_threads: 1,
        data_root: dir.path().to_path_buf(),
        jobs_cfg: AiJobsConfig::new(dir.path().join("cache")),
        utc_offset: Arc::new(|| 9 * 60),
        cli_runner: cli.runner.clone(),
        cli_env: cli.env.clone(),
        url_opener: cli.opener.clone(),
    })
    .unwrap()
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
