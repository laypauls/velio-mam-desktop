//! 시험용 가짜 원격 — SSH·네트워크 없이 로그 폴더 하나를 흉내 낸다 (실호출 금지 정책). 통합 시험(`tests/`)과 `src-tauri` 시험도 쓴다.
//!
//! 수집기가 보내는 명령 두 가지(`audit-log.*` 크기 목록, `tail -c +N`)에만 답한다. 호스트 키 규칙은 운영 구현과 같은 `check_host_key` 를 쓴다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::RemoteSource;
use super::fake_ses::FakeSes;
use super::runner::{RemoteConnector, RemoteRunner, RunError, check_host_key};
use crate::secret::{SecretBox, SecretError};
use zeroize::Zeroizing;

#[derive(Default)]
struct State {
    dir: String,
    files: BTreeMap<String, String>,
    fail: Option<RunError>,
    dir_missing: bool,
    extra_listing: String,
    calls: u32,
}

/// 로그 폴더 하나. 복제해도 같은 상태를 가리킨다.
#[derive(Clone, Default)]
pub struct FakeDdp(Arc<Mutex<State>>);

impl FakeDdp {
    pub fn new(dir: &str) -> Self {
        Self(Arc::new(Mutex::new(State {
            dir: dir.trim_end_matches('/').into(),
            ..State::default()
        })))
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 파일 끝에 덧붙인다 (없으면 만든다).
    pub fn write(&self, name: &str, s: &str) {
        self.st().files.entry(name.into()).or_default().push_str(s);
    }

    /// 파일 내용을 통째로 바꾼다 (갈아치움·줄어듦).
    pub fn set(&self, name: &str, s: &str) {
        self.st().files.insert(name.into(), s.into());
    }

    pub fn remove(&self, name: &str) {
        self.st().files.remove(name);
    }

    pub fn size(&self, name: &str) -> usize {
        self.st().files.get(name).map_or(0, String::len)
    }

    /// 설정하면 모든 명령이 이 오류로 실패한다.
    pub fn set_fail(&self, e: Option<RunError>) {
        self.st().fail = e;
    }

    pub fn set_dir_missing(&self, missing: bool) {
        self.st().dir_missing = missing;
    }

    /// 목록 출력 끝에 날것의 줄을 덧붙인다 (악의적·깨진 파일 이름 시험).
    pub fn raw_listing_extra(&self, s: &str) {
        self.st().extra_listing = s.into();
    }

    /// 지금까지 받은 명령 수
    pub fn calls(&self) -> u32 {
        self.st().calls
    }

    pub fn runner(&self) -> FakeRunner {
        FakeRunner(self.clone())
    }
}

pub struct FakeRunner(FakeDdp);

impl RemoteRunner for FakeRunner {
    fn run(&mut self, cmd: &str) -> Result<Vec<u8>, RunError> {
        let mut st = self.0.st();
        st.calls += 1;
        if let Some(e) = st.fail.clone() {
            return Err(e);
        }
        if cmd.contains("audit-log.*") {
            if st.dir_missing {
                return Err(RunError::NoLogDir);
            }
            let mut s: String = st
                .files
                .iter()
                .map(|(n, c)| format!("{} {n}\n", c.len()))
                .collect();
            s.push_str(&st.extra_listing);
            return Ok(s.into_bytes());
        }
        if let Some(rest) = cmd.strip_prefix("tail -c +") {
            let (n, path) = rest
                .split_once(' ')
                .ok_or_else(|| RunError::Exec("bad tail".into()))?;
            let n: usize = n.parse().map_err(|_| RunError::Exec("bad offset".into()))?;
            let path = path.trim_matches('\'');
            let name = path.strip_prefix(&format!("{}/", st.dir)).unwrap_or(path);
            let body = st.files.get(name).map_or(&[][..], |s| s.as_bytes());
            return Ok(body.get(n.saturating_sub(1)..).unwrap_or(&[]).to_vec());
        }
        Err(RunError::Exec(format!("unknown command: {cmd:?}")))
    }
}

/// 접속 시도를 세고 실패를 주입할 수 있는 가짜 연결기.
pub struct FakeConnector {
    pub ddp: FakeDdp,
    /// 있으면 이 가짜 SES 호스트에 붙는다 (`ses_queue` 시험)
    ses: Option<FakeSes>,
    fingerprint: String,
    connects: AtomicU32,
    /// 앞으로 N 번의 접속 시도를 이 오류로 실패시킨다
    fail_next: Mutex<(u32, Option<RunError>)>,
    /// 접속에 쓰인 (호스트, 사용자) — 비밀은 기록하지 않는다
    seen: Mutex<Vec<(String, String)>>,
}

impl FakeConnector {
    pub fn new(ddp: FakeDdp, fingerprint: &str) -> Self {
        Self {
            ddp,
            ses: None,
            fingerprint: fingerprint.into(),
            connects: AtomicU32::new(0),
            fail_next: Mutex::new((0, None)),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// 가짜 SES 호스트에 붙는 연결기
    pub fn ses(ses: FakeSes, fingerprint: &str) -> Self {
        Self {
            ses: Some(ses),
            ..Self::new(FakeDdp::default(), fingerprint)
        }
    }

    pub fn fail_connects(&self, times: u32, e: RunError) {
        *self.fail_next.lock().unwrap_or_else(|p| p.into_inner()) = (times, Some(e));
    }

    pub fn connects(&self) -> u32 {
        self.connects.load(Ordering::SeqCst)
    }

    pub fn seen(&self) -> Vec<(String, String)> {
        self.seen.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl RemoteConnector for FakeConnector {
    fn connect(&self, src: &RemoteSource) -> Result<Box<dyn RemoteRunner>, RunError> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((src.host.clone(), src.user.clone()));
        {
            let mut f = self.fail_next.lock().unwrap_or_else(|p| p.into_inner());
            if f.0 > 0 {
                f.0 -= 1;
                if let Some(e) = f.1.clone() {
                    return Err(e);
                }
            }
        }
        check_host_key(src.host_key.as_deref(), &self.fingerprint)?;
        Ok(match &self.ses {
            Some(ses) => Box::new(ses.runner()),
            None => Box::new(self.ddp.runner()),
        })
    }

    fn fingerprint(&self, _host: &str, _port: u16) -> Result<String, RunError> {
        Ok(self.fingerprint.clone())
    }
}

/// 감사 로그 한 줄 (끝 개행 포함)
pub fn line(kind: &str, path: &str) -> String {
    format!("2026-10-23 09:30:00.123456789  {kind}  iqn.kbs-01  |{path}\n")
}

/// 시험용 비밀 상자 — DPAPI 대신 바이트를 가린다 (평문이 blob 에 그대로 있지 않다는 것만 보장). **운영에 쓰지 않는다.**
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeSecretBox;

impl SecretBox for FakeSecretBox {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SecretError> {
        Ok(std::iter::once(0xA5)
            .chain(plain.iter().map(|b| b ^ 0x5A))
            .collect())
    }

    fn open(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        match blob.split_first() {
            Some((0xA5, rest)) => Ok(Zeroizing::new(rest.iter().map(|b| b ^ 0x5A).collect())),
            _ => Err(SecretError::Crypto("fake blob corrupted")),
        }
    }
}
