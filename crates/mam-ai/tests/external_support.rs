//! 외부 AI 테스트 공용 도구 — 스텁 전송(실호출 금지), 고정 시계/슬립, 이미지·요청 픽스처 헬퍼.
//! 다른 `external_*.rs` 가 `#[path]` 로 가져다 쓴다.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mam_ai::external::{
    AnalysisKind, AnalyzeRequest, Clock, ExternalClient, HttpRequest, HttpResponse, HttpTransport,
    MediaType, Provider, RetryPolicy, SanitizedMeta, Sleeper, TransferScope, TransportError,
    UsageRecord, UsageSink,
};
use mam_ai::keys::KeyVault;
use serde_json::{Value, json};

pub const SECRET: &str = "sk-test-SECRET-0123456789abcdef";

#[derive(Default)]
pub struct StubTransport {
    queue: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
    pub requests: Mutex<Vec<HttpRequest>>,
}

impl StubTransport {
    pub fn with(responses: Vec<Result<HttpResponse, TransportError>>) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(responses.into()),
            requests: Mutex::default(),
        })
    }
    pub fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl HttpTransport for StubTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.requests.lock().unwrap().push(req.clone());
        self.queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(TransportError::Other))
    }
}

pub fn resp(
    status: u16,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<HttpResponse, TransportError> {
    Ok(HttpResponse {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.as_bytes().to_vec(),
    })
}

pub struct FixedClock(pub i64);
impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

#[derive(Default)]
pub struct RecordingSleeper(pub Mutex<Vec<Duration>>);
impl Sleeper for RecordingSleeper {
    fn sleep(&self, d: Duration) {
        self.0.lock().unwrap().push(d);
    }
}

#[derive(Default)]
pub struct VecSink(pub Mutex<Vec<UsageRecord>>);
impl UsageSink for VecSink {
    fn record(&self, u: &UsageRecord) {
        self.0.lock().unwrap().push(u.clone());
    }
}

pub struct Rig {
    pub client: ExternalClient,
    pub transport: Arc<StubTransport>,
    pub sleeper: Arc<RecordingSleeper>,
    pub sink: Arc<VecSink>,
    pub vault: Arc<KeyVault>,
}

pub fn rig(
    provider: Provider,
    responses: Vec<Result<HttpResponse, TransportError>>,
    with_key: bool,
) -> Rig {
    let transport = StubTransport::with(responses);
    let sleeper = Arc::new(RecordingSleeper::default());
    let sink = Arc::new(VecSink::default());
    let vault = Arc::new(KeyVault::new());
    if with_key {
        vault.set(provider, SECRET, false).unwrap();
    }
    let retry = RetryPolicy::with(Arc::new(FixedClock(1_700_000_000_000)), sleeper.clone());
    let client = ExternalClient::new(
        provider,
        transport.clone(),
        vault.clone(),
        retry,
        Some(sink.clone()),
    );
    Rig {
        client,
        transport,
        sleeper,
        sink,
        vault,
    }
}

/// 결정적인 PNG (그라데이션).
pub fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
    });
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

pub fn request(images: Vec<Vec<u8>>) -> AnalyzeRequest {
    AnalyzeRequest {
        asset_id: Some(42),
        model: None,
        language: "ko".into(),
        kinds: vec![
            AnalysisKind::Describe,
            AnalysisKind::Keywords,
            AnalysisKind::Ocr,
        ],
        scope: TransferScope::default(),
        meta: SanitizedMeta::new(
            "C:\\Users\\alice\\Videos\\trip\\beach.jpg",
            Some("2024-05-01".into()),
            MediaType::Image,
            None,
        ),
        images,
    }
}

pub const MODEL_JSON: &str = r#"{"description":"해변 풍경","keywords":["해변","바다"],"ocrText":"","summary":"여름 해변","language":"ko"}"#;

pub fn openai_ok(text: &str) -> String {
    json!({"choices": [{"message": {"content": text}}], "usage": {"prompt_tokens": 120, "completion_tokens": 30}}).to_string()
}
pub fn gemini_ok(text: &str) -> String {
    json!({"candidates": [{"content": {"parts": [{"text": text}]}}], "usageMetadata": {"promptTokenCount": 200, "candidatesTokenCount": 40}}).to_string()
}
pub fn claude_ok(text: &str) -> String {
    json!({"content": [{"type": "text", "text": text}], "usage": {"input_tokens": 300, "output_tokens": 50}}).to_string()
}

/// 기록된 요청(url·헤더·본문)을 안정 비교용 JSON 으로 만든다: base64 이미지는 자리표시자, 키는 `<KEY>`.
pub fn normalize(req: &HttpRequest, image_b64: &str) -> Value {
    let body = String::from_utf8(req.body.clone())
        .unwrap()
        .replace(image_b64, "<IMAGE_B64>");
    let mut headers = serde_json::Map::new();
    for (k, v) in &req.headers {
        headers.insert(k.to_lowercase(), json!(v.replace(SECRET, "<KEY>")));
    }
    json!({"url": req.url, "headers": headers, "body": serde_json::from_str::<Value>(&body).unwrap()})
}

/// 픽스처와 비교. `UPDATE_FIXTURES=1` 이면 현재 값으로 다시 기록한다.
pub fn assert_fixture(name: &str, actual: &Value) {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "external",
        name,
    ]
    .iter()
    .collect();
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(actual).unwrap() + "\n").unwrap();
    }
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n");
    let expected: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(&expected, actual, "fixture {name} differs");
}
