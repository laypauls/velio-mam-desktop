//! HTTP 전송 추상화 — 실제 구현은 ureq(rustls), 테스트는 스텁을 주입한다. 헤더·본문은 로그에 남기지 않는다.

use std::fmt;
use std::time::Duration;

use zeroize::Zeroize;

/// 응답 본문 상한 (비정상적으로 큰 응답으로 메모리를 쓰지 않도록).
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }
}

/// 인증 헤더 값이 메모리에 남지 않도록 해제 시 0 으로 덮는다.
impl Drop for HttpRequest {
    fn drop(&mut self) {
        for (_, v) in &mut self.headers {
            v.zeroize();
        }
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("host", &host_of(&self.url))
            .field("headers", &self.headers.len())
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// 전송 계층 실패 (HTTP 상태 코드는 오류가 아니라 [`HttpResponse`] 로 돌려준다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    Timeout,
    /// 연결 실패·DNS 실패 — 오프라인.
    Offline,
    Other,
}

pub trait HttpTransport: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError>;
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or("")
}

/// ureq 3 (rustls) 기반 실제 전송.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqTransport {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
    }
}

impl HttpTransport for UreqTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let mut builder = ureq::http::Request::builder()
            .method(req.method)
            .uri(req.url.as_str());
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        let request = builder
            .body(req.body.clone())
            .map_err(|_| TransportError::Other)?;
        // 요청별 제한 시간은 에이전트 설정 대신 요청 확장으로 전달한다.
        let request = self
            .agent
            .configure_request(request)
            .timeout_global(Some(req.timeout))
            .build();
        let mut response = self.agent.run(request).map_err(map_err)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(
                |(k, v): (&ureq::http::HeaderName, &ureq::http::HeaderValue)| {
                    Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))
                },
            )
            .collect();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_vec()
            .map_err(map_err)?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

fn map_err(e: ureq::Error) -> TransportError {
    use std::io::ErrorKind as K;
    use ureq::Error as E;
    match e {
        E::Timeout(_) => TransportError::Timeout,
        E::ConnectionFailed | E::HostNotFound => TransportError::Offline,
        E::Io(ref io) if io.kind() == K::TimedOut => TransportError::Timeout,
        E::Io(ref io)
            if matches!(
                io.kind(),
                K::ConnectionRefused | K::ConnectionReset | K::ConnectionAborted | K::NotConnected
            ) =>
        {
            TransportError::Offline
        }
        _ => TransportError::Other,
    }
}
