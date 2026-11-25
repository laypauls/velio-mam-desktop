//! 통합 시험 공용 - 로컬 HTTP 스텁 서버(실제 네트워크 호출 금지), 모델 위치 조회.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 서버가 응답하는 방식.
#[derive(Clone, Default)]
pub struct Behavior {
    /// 항상 이 상태 코드로 응답 (본문 없음).
    pub status: Option<u16>,
    /// 본문을 이 바이트만 보내고 연결을 끊는다 (Content-Length 는 전체 길이 그대로).
    pub cut_after: Option<usize>,
    /// `Range` 헤더를 무시하고 항상 200 전체를 보낸다.
    pub ignore_range: bool,
}

pub struct StubServer {
    pub addr: SocketAddr,
    pub behavior: Arc<Mutex<Behavior>>,
    /// 받은 요청의 `Range` 헤더 값 (없으면 `None`).
    pub requests: Arc<Mutex<Vec<Option<String>>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl StubServer {
    /// `body` 하나를 `/file` 경로로 제공한다.
    pub fn start(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let behavior = Arc::new(Mutex::new(Behavior::default()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (b, r, s) = (behavior.clone(), requests.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            for conn in listener.incoming() {
                if s.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(conn) = conn {
                    let (b, r, body) = (b.clone(), r.clone(), body.clone());
                    std::thread::spawn(move || serve(conn, &body, &b.lock().unwrap().clone(), &r));
                }
            }
        });
        Self {
            addr,
            behavior,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub fn set(&self, f: impl FnOnce(&mut Behavior)) {
        f(&mut self.behavior.lock().unwrap());
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr); // accept 를 깨운다
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn serve(mut conn: TcpStream, body: &[u8], b: &Behavior, requests: &Mutex<Vec<Option<String>>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match conn.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let range = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("Range: ")
                .or_else(|| l.strip_prefix("range: "))
        })
        .map(|v| v.trim().to_string());
    requests.lock().unwrap().push(range.clone());

    if let Some(code) = b.status {
        let _ = write!(
            conn,
            "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    }
    let start = match (&range, b.ignore_range) {
        (Some(r), false) => r
            .trim_start_matches("bytes=")
            .trim_end_matches('-')
            .parse::<usize>()
            .unwrap_or(0),
        _ => 0,
    };
    if start > body.len() {
        let _ = write!(
            conn,
            "HTTP/1.1 416 X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    }
    let payload = &body[start..];
    if start > 0 {
        let _ = write!(
            conn,
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len() - 1,
            body.len(),
            payload.len()
        );
    } else {
        let _ = write!(
            conn,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
    }
    let send = b.cut_after.map_or(payload.len(), |n| n.min(payload.len()));
    let _ = conn.write_all(&payload[..send]);
    let _ = conn.flush();
    // 끊기 모사: 소켓을 닫아 Content-Length 보다 짧게 끝낸다.
}

/// 실제 모델 파일 위치 - 환경 변수 `MAM_AI_TEST_MODELS` (제품 배치 `models/<id>/model.onnx` 가 있는 폴더).
/// 없으면 `None` 이고 호출 시험은 사유를 출력하고 건너뛴다.
pub fn real_models() -> Option<PathBuf> {
    std::env::var_os("MAM_AI_TEST_MODELS")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// 모델이 없으면 사유를 출력하고 `None`.
pub fn require_models(test: &str) -> Option<PathBuf> {
    let r = real_models();
    if r.is_none() {
        println!("SKIP {test}: MAM_AI_TEST_MODELS not set (real ONNX models required)");
    }
    r
}
