//! SSH 구현 — 순수 Rust `russh`(ring 백엔드). 비동기 라이브러리라 접속마다 작은 tokio 런타임(작업 스레드 1개)을 쥐고 `block_on` 으로 동기 trait 에 맞춘다.
//!
//! 호스트 키: 서버가 내민 키의 SHA256 지문을 **인증 전에** `check_host_key` 로 판정한다 — 저장된 지문이 없거나 다르면 연결을 끊고
//! 비밀(비밀번호·키 서명)을 보내지 않는다. 명령은 `exec` 채널 하나로 읽기 명령만 돌린다 (`tail.rs` 의 `cmd`). 쓰기 채널(SFTP·셸·포워딩)은 열지 않는다.
//! 오류 문구에는 대상·종류만 담고 비밀은 없다 (russh 오류는 길이를 자르고 제어 문자를 뺀다).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, decode_secret_key};
use russh::{ChannelMsg, Disconnect};
use tokio::runtime::{Builder, Runtime};

use super::runner::{RemoteConnector, RemoteRunner, RunError, check_host_key};
use super::{Auth, RemoteSource};

/// 명령 하나의 출력 상한 (`tail` 1 MiB 상한 + 목록 여유)
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

pub struct SshConnector {
    /// 접속(핸드셰이크·인증)과 명령 하나에 거는 제한 시간
    pub timeout: Duration,
}

impl Default for SshConnector {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(20) }
    }
}

/// 서버가 내민 키를 판정한다. 지문은 `seen` 에 남겨 호출자가 사유를 만든다.
struct Verifier {
    expected: Option<String>,
    /// true = 지문만 알아 오는 용도 — 항상 끊는다 (인증하지 않는다)
    probe_only: bool,
    seen: Arc<Mutex<Option<String>>>,
}

fn fingerprint_of(k: &PublicKeyOrCertificate) -> String {
    match k {
        PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256).to_string(),
        PublicKeyOrCertificate::Certificate(c) => c.public_key().fingerprint(HashAlg::Sha256).to_string(),
    }
}

impl client::Handler for Verifier {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fp = fingerprint_of(key);
        let ok = !self.probe_only && check_host_key(self.expected.as_deref(), &fp).is_ok();
        *self.seen.lock().unwrap_or_else(|p| p.into_inner()) = Some(fp);
        Ok(ok)
    }
}

fn short(e: &impl std::fmt::Display) -> String {
    e.to_string().chars().filter(|c| !c.is_control()).take(200).collect()
}

fn runtime() -> Result<Runtime, RunError> {
    Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name("mam-ssh")
        .build()
        .map_err(|e| RunError::Connect(short(&e)))
}

/// 핸드셰이크 실패를 사유로 바꾼다 — 키 판정이 거부한 것이면 호스트 키 오류가 우선한다.
fn classify(e: russh::Error, expected: Option<&str>, seen: &Mutex<Option<String>>) -> RunError {
    let fp = seen.lock().unwrap_or_else(|p| p.into_inner()).clone();
    match fp.map(|fp| check_host_key(expected, &fp)) {
        Some(Err(host_key_error)) => host_key_error,
        _ => RunError::Connect(short(&e)),
    }
}

async fn open(src: &RemoteSource, to: Duration, probe_only: bool, seen: Arc<Mutex<Option<String>>>) -> Result<Handle<Verifier>, RunError> {
    let cfg = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        inactivity_timeout: None,
        ..client::Config::default()
    });
    let verifier = Verifier { expected: src.host_key.clone(), probe_only, seen: Arc::clone(&seen) };
    let connecting = client::connect(cfg, (src.host.as_str(), src.port), verifier);
    let mut h = tokio::time::timeout(to, connecting)
        .await
        .map_err(|_| RunError::Connect("connection timed out".into()))?
        .map_err(|e| classify(e, src.host_key.as_deref(), &seen))?;
    let authed = tokio::time::timeout(to, authenticate(&mut h, src)).await.map_err(|_| RunError::Timeout)??;
    if !authed {
        return Err(RunError::Auth);
    }
    Ok(h)
}

async fn authenticate(h: &mut Handle<Verifier>, src: &RemoteSource) -> Result<bool, RunError> {
    let res = match &src.auth {
        Auth::Password(p) => h.authenticate_password(src.user.clone(), p.expose().to_string()).await,
        Auth::Key(pem) => {
            // 암호가 걸린 키는 지원하지 않는다 — 오류에 키 내용은 없다
            let key = decode_secret_key(pem.expose(), None).map_err(|_| RunError::BadKey)?;
            let hash = h.best_supported_rsa_hash().await.map_err(|e| RunError::Connect(short(&e)))?.flatten();
            h.authenticate_publickey(src.user.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await
        }
    };
    res.map(|r| r.success()).map_err(|e| RunError::Connect(short(&e)))
}

impl RemoteConnector for SshConnector {
    fn connect(&self, src: &RemoteSource) -> Result<Box<dyn RemoteRunner>, RunError> {
        let rt = runtime()?;
        let seen = Arc::new(Mutex::new(None));
        let handle = rt.block_on(open(src, self.timeout, false, seen))?;
        Ok(Box::new(SshRunner { rt: Some(rt), handle, timeout: self.timeout }))
    }

    fn fingerprint(&self, host: &str, port: u16) -> Result<String, RunError> {
        // 인증 정보 없이 호스트 키만 받는다 (비밀을 보내지 않는다)
        let probe = RemoteSource {
            source_id: 0,
            kind: super::SourceKind::DdpAudit,
            name: String::new(),
            host: host.to_string(),
            port,
            user: String::new(),
            auth: Auth::Password(crate::secret::Secret::new(String::new())),
            host_key: None,
            log_dir: None,
            db_path: None,
            poll_sec: 1,
            path_map: Vec::new(),
        };
        let rt = runtime()?;
        let seen = Arc::new(Mutex::new(None));
        let res = rt.block_on(open(&probe, self.timeout, true, Arc::clone(&seen)));
        rt.shutdown_background();
        let fp = seen.lock().unwrap_or_else(|p| p.into_inner()).clone();
        match (fp, res) {
            (Some(fp), _) => Ok(fp),
            (None, Err(e)) => Err(e),
            (None, Ok(_)) => Err(RunError::Connect("no host key offered".into())),
        }
    }
}

struct SshRunner {
    rt: Option<Runtime>,
    handle: Handle<Verifier>,
    timeout: Duration,
}

async fn exec(h: &Handle<Verifier>, cmd: &str) -> Result<Vec<u8>, RunError> {
    let mut ch = h.channel_open_session().await.map_err(|_| RunError::Closed)?;
    ch.exec(true, cmd).await.map_err(|_| RunError::Closed)?;
    let (mut out, mut err, mut code) = (Vec::new(), Vec::new(), None);
    while let Some(msg) = ch.wait().await {
        match msg {
            ChannelMsg::Data { data } => {
                if out.len() + data.len() > MAX_OUTPUT {
                    return Err(RunError::Exec("output too large".into()));
                }
                out.extend_from_slice(&data);
            }
            ChannelMsg::ExtendedData { data, ext: 1 } if err.len() < 256 => err.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
            _ => {}
        }
    }
    match code {
        Some(0) => Ok(out),
        // 종료 코드 9 = 로그 폴더 없음 (`cmd::list_sizes` 의 `cd … || exit 9`)
        Some(9) => Err(RunError::NoLogDir),
        // 종료 코드 10·11·12 = sqlite3 없음 / DB 파일을 읽을 수 없음 / sqlite3 실행 실패 (`ses::cmd` 의 래퍼)
        Some(10) => Err(RunError::NoSqlite),
        Some(11) => Err(RunError::DbUnreadable(String::new())),
        Some(12) => Err(RunError::DbUnreadable(short(&String::from_utf8_lossy(&err)))),
        Some(c) => Err(RunError::Exec(format!("exit status {c}: {}", short(&String::from_utf8_lossy(&err))))),
        None => Err(RunError::Closed),
    }
}

impl RemoteRunner for SshRunner {
    fn run(&mut self, cmd: &str) -> Result<Vec<u8>, RunError> {
        let Some(rt) = self.rt.as_ref() else { return Err(RunError::Closed) };
        rt.block_on(async { tokio::time::timeout(self.timeout, exec(&self.handle, cmd)).await.map_err(|_| RunError::Timeout)? })
    }
}

impl Drop for SshRunner {
    fn drop(&mut self) {
        if let Some(rt) = self.rt.take() {
            // 정중히 끊되 오래 기다리지 않는다
            let _ = rt.block_on(async {
                tokio::time::timeout(Duration::from_secs(2), self.handle.disconnect(Disconnect::ByApplication, "", "en")).await
            });
            rt.shutdown_background();
        }
    }
}

#[cfg(test)]
#[path = "ssh_tests.rs"]
mod tests;
