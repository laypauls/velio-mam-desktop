//! 구독 CLI 시험 공용 도구 — 가짜 환경·각본 도우미. 실제 `claude` 는 띄우지 않는다.

mod client;
mod host;
mod host_transient;
mod isolation;
mod login;
mod login_race;
mod os_runner;
mod resolve;
mod sub_limit;
mod umeta;

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::fake::{FakeRunner, Step};
use super::host::CliHost;
use super::process::CliCommand;
use super::resolve::CliEnv;
use crate::external::payload::{AnalyzeRequest, MediaType, SanitizedMeta};
use crate::external::retry::Clock;
use crate::external::types::{AnalysisKind, TransferScope};

pub(super) const EXE: &str = r"C:\fake\bin\claude.exe";

/// 파일·환경변수를 흉내 내는 환경.
#[derive(Default)]
pub(super) struct FakeEnv {
    pub path: Option<OsString>,
    pub home: Option<PathBuf>,
    pub files: Mutex<HashMap<PathBuf, String>>,
    pub vars: HashSet<String>,
    pub windows: bool,
}

impl FakeEnv {
    /// `C:\fake\bin\claude.exe` 가 PATH 에 있는 Windows 환경.
    pub fn with_claude() -> Self {
        let env = Self {
            path: Some(OsString::from(r"C:\fake\bin;C:\other")),
            windows: true,
            ..Default::default()
        };
        env.add_file(EXE, "");
        env
    }

    pub fn without_claude() -> Self {
        Self {
            path: Some(OsString::from(r"C:\fake\bin")),
            windows: true,
            ..Default::default()
        }
    }

    pub fn add_file(&self, p: &str, content: &str) {
        self.files
            .lock()
            .unwrap()
            .insert(PathBuf::from(p), content.to_string());
    }

    pub fn with_var(mut self, name: &str) -> Self {
        self.vars.insert(name.to_string());
        self
    }
}

impl CliEnv for FakeEnv {
    fn path_var(&self) -> Option<OsString> {
        self.path.clone()
    }
    fn home_dir(&self) -> Option<PathBuf> {
        self.home.clone()
    }
    fn is_file(&self, p: &Path) -> bool {
        self.files.lock().unwrap().contains_key(p)
    }
    fn read_to_string(&self, p: &Path) -> Option<String> {
        self.files.lock().unwrap().get(p).cloned()
    }
    fn has_var(&self, name: &str) -> bool {
        self.vars.contains(name)
    }
    fn is_windows(&self) -> bool {
        self.windows
    }
}

pub(super) struct FixedClock(pub i64);
impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

pub(super) fn args_of(c: &CliCommand) -> Vec<String> {
    c.args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

pub(super) fn is_cmd(c: &CliCommand, first: &str) -> bool {
    args_of(c).first().is_some_and(|a| a == first)
}

/// 실행 파일 호출 종류별 기본 각본. `request`: 요청(`-p ...`) 호출의 각본.
pub(super) struct Scripts {
    pub version: Option<Vec<Step>>,
    pub status: Vec<Step>,
    pub request: Vec<Step>,
    pub login: Vec<Step>,
    pub logout: Vec<Step>,
}

impl Default for Scripts {
    fn default() -> Self {
        Self {
            version: Some(vec![Step::Out("2.1.318 (Claude Code)\n".into()), Step::Exit(Some(0))]),
            status: vec![
                Step::Out(r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","email":"me@example.com"}"#.into()),
                Step::Exit(Some(0)),
            ],
            request: success_request(),
            login: vec![Step::Hang],
            logout: vec![Step::Exit(Some(0))],
        }
    }
}

pub(super) fn dispatch(s: Scripts) -> Arc<FakeRunner> {
    FakeRunner::new(move |c| {
        let a = args_of(c);
        match a.first().map(String::as_str) {
            Some("--version") => s.version.clone(),
            Some("auth") => match a.get(1).map(String::as_str) {
                Some("status") => Some(s.status.clone()),
                Some("login") => Some(s.login.clone()),
                Some("logout") => Some(s.logout.clone()),
                _ => None,
            },
            Some("-p") => Some(s.request.clone()),
            _ => None,
        }
    })
}

pub(super) fn host(runner: Arc<FakeRunner>, env: FakeEnv, root: &Path) -> Arc<CliHost> {
    CliHost::new(
        runner,
        Arc::new(env),
        Arc::new(FixedClock(1_000_000_000_000)),
        root.join("tmp").join("ai-cli"),
    )
}

pub(super) fn line(s: &str) -> Step {
    Step::Out(format!("{s}\n"))
}

pub(super) const INIT_SUB: &str = r#"{"type":"system","subtype":"init","apiKeySource":"none","model":"claude-haiku-5-5","tools":["StructuredOutput"]}"#;
pub(super) const INIT_KEY: &str = r#"{"type":"system","subtype":"init","apiKeySource":"ANTHROPIC_API_KEY","model":"claude-haiku-5-5","tools":[]}"#;
pub(super) const OK_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"api_error_status":null,"result":"{}","structured_output":{"description":"붉은 노을","keywords":["노을","하늘"],"ocrText":"","summary":"노을 사진","language":"ko"},"usage":{"input_tokens":2,"cache_creation_input_tokens":7187,"output_tokens":1317}}"#;
pub(super) const RETRY_NULL: &str = r#"{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":503,"error_status":null,"error":"unknown"}"#;

pub(super) fn success_request() -> Vec<Step> {
    vec![line(INIT_SUB), line(OK_RESULT), Step::Exit(Some(0))]
}

pub(super) fn error_result(status: &str, text: &str) -> String {
    format!(
        r#"{{"type":"result","subtype":"success","is_error":true,"api_error_status":{status},"result":"{text}"}}"#
    )
}

/// 작은 PNG 한 장 (`prepare_image` 가 JPEG 로 다시 인코딩한다).
pub(super) fn tiny_png() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(64, 48, image::Rgb([200, 80, 40]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// 원본 경로가 섞여 들어온 파일 이름도 정제되는지 보려고 일부러 절대 경로를 넘긴다.
pub(super) fn request() -> AnalyzeRequest {
    AnalyzeRequest {
        asset_id: Some(7),
        model: None,
        language: "ko".into(),
        kinds: vec![AnalysisKind::Describe, AnalysisKind::Keywords],
        scope: TransferScope::default(),
        meta: SanitizedMeta::new(
            r"D:\Photos\secret-trip\IMG_0001.jpg",
            Some("2026-01-02T03:04:05Z".into()),
            MediaType::Image,
            None,
        ),
        images: vec![tiny_png()],
    }
}

pub(super) fn files_in(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.count())
}
