//! 원격 스토리지 접속 API — 목록·생성·수정·삭제·접속 시험. 검증은 여기(`watch::remote::store`)에서 하고 command 는 위임만 한다.
//!
//! 비밀(비밀번호·개인 키)은 입력으로만 받고 **어디에도 돌려주지 않는다**(`SourceView::has_secret`). `RemoteSourceInput` 의 `Debug` 는 `Secret` 이 가린다.
//! 접속 시험은 호스트 키 지문을 아직 확인하지 않았으면 **비밀을 보내지 않고** 지문만 알려 준다(사용자가 확인해 저장한 뒤에야 인증한다).

use mam_db::Priority;
use serde::{Deserialize, Serialize};

use super::Ingest;
use crate::scan::{IngestError, load_folder};
use crate::secret::{Secret, SecretError};
use crate::watch::remote::mapper::{Mapping, map_reaches};
use crate::watch::remote::runner::RunError;
use crate::watch::remote::store::{self, NewSource, SourceView, StoreError};
use crate::watch::remote::ses;
use crate::watch::remote::tail::list_sizes;
use crate::watch::remote::{Auth, RemoteSource, SourceKind};

/// 생성·수정·시험 입력 (IPC 인자). `secret` 이 없으면 수정은 저장된 비밀을 그대로 둔다.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSourceInput {
    pub kind: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub user: String,
    /// `"password"` | `"key"`
    pub auth_kind: String,
    pub secret: Option<Secret>,
    pub host_key: Option<String>,
    pub log_dir: Option<String>,
    /// `ses_queue`: 스토리지 호스트의 SES `events.db` 경로 (필수). `ddp_audit` 은 없어야 한다
    pub db_path: Option<String>,
    pub poll_sec: i64,
    pub path_map: Vec<Mapping>,
}

/// 접속 시험 결과 — 실패도 오류가 아니라 결과다 (UI 가 사유 키와 지문을 보여 준다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTestResult {
    pub ok: bool,
    /// 실패 사유 번역 키 접미사 (`remote.error.<key>`)
    pub error_key: Option<&'static str>,
    /// 서버가 내민 호스트 키 지문 (확인 전이거나 다를 때 사용자에게 보여 준다)
    pub fingerprint: Option<String>,
    pub file_count: Option<u32>,
    /// 이벤트 큐 행 수 (`ses_queue` 성공 때)
    pub row_count: Option<u64>,
    /// 이벤트 큐의 마지막 id (`ses_queue` 성공 때)
    pub max_id: Option<i64>,
}

impl RemoteTestResult {
    fn failed(e: &RunError) -> Self {
        Self { ok: false, error_key: Some(e.key()), fingerprint: e.fingerprint().map(str::to_string), file_count: None, row_count: None, max_id: None }
    }
}

impl From<StoreError> for IngestError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Invalid(f) => Self::InvalidInput(f),
            StoreError::NotFound => Self::NotFound,
            StoreError::NameTaken => Self::AlreadyRegistered,
            StoreError::InUse => Self::Busy,
            StoreError::Secret(SecretError::Unsupported) => Self::Io(std::io::Error::other("secret protection is unavailable")),
            StoreError::Secret(SecretError::Crypto(m)) => Self::Io(std::io::Error::other(m)),
            StoreError::Db(e) => Self::Sql(e),
        }
    }
}

/// 입력 -> (저장 입력, 비밀). `kind` 는 `ddp_audit`·`ses_queue`.
fn parse_input(i: &RemoteSourceInput) -> Result<(NewSource, Option<Auth>), IngestError> {
    let kind = SourceKind::parse(&i.kind).ok_or(IngestError::InvalidInput("kind"))?;
    let port = u16::try_from(i.port).ok().filter(|p| *p != 0).ok_or(IngestError::InvalidInput("port"))?;
    let poll_sec = u32::try_from(i.poll_sec).map_err(|_| IngestError::InvalidInput("pollSec"))?;
    let auth = match (i.auth_kind.as_str(), &i.secret) {
        (_, None) => None,
        ("password", Some(s)) => Some(Auth::Password(s.clone())),
        ("key", Some(s)) => Some(Auth::Key(s.clone())),
        _ => return Err(IngestError::InvalidInput("authKind")),
    };
    if !matches!(i.auth_kind.as_str(), "password" | "key") {
        return Err(IngestError::InvalidInput("authKind"));
    }
    let n = NewSource {
        kind,
        name: i.name.clone(),
        host: i.host.trim().to_string(),
        port,
        user: i.user.clone(),
        host_key: i.host_key.clone().filter(|s| !s.is_empty()),
        log_dir: i.log_dir.clone().map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
        db_path: i.db_path.clone().map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
        poll_sec,
        path_map: i.path_map.iter().map(|m| Mapping { from: m.from.trim().into(), to: m.to.trim().into() }).collect(),
    };
    store::validate(&n)?;
    Ok((n, auth))
}

impl Ingest {
    pub fn list_remote_sources(&self) -> Result<Vec<SourceView>, IngestError> {
        Ok(self.env.pool.read(|c| Ok(store::list(c)))??)
    }

    pub fn create_remote_source(&self, input: RemoteSourceInput) -> Result<SourceView, IngestError> {
        let (n, auth) = parse_input(&input)?;
        let auth = auth.ok_or(IngestError::InvalidInput("secret"))?;
        let secrets = self.config.secrets.clone();
        let now = self.env.clock.now_ms();
        let id = self.env.write(Priority::High, move |c| Ok(store::create(c, &n, &auth, secrets.as_ref(), now)?))?;
        Ok(self.env.pool.read(|c| Ok(store::view(c, id)))??)
    }

    /// 수정. 이 접속을 쓰는 폴더가 있으면 새 경로 변환도 그 폴더 루트에 닿아야 한다(`InvalidInput("pathMap")`). 도는 수집기는 재접속한다.
    pub fn update_remote_source(&self, source_id: i64, input: RemoteSourceInput) -> Result<SourceView, IngestError> {
        let (n, auth) = parse_input(&input)?;
        let (secrets, now) = (self.config.secrets.clone(), self.env.clock.now_ms());
        let rules = n.path_map.clone();
        self.env.write(Priority::High, move |c| {
            // 쓰는 폴더가 새 변환에서 빠지지 않게 같은 트랜잭션 안에서 확인한다 (확인과 저장 사이에 폴더가 바뀌지 않는다)
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            for fid in store::referencing_folders(&tx, source_id)? {
                let root = load_folder(&tx, fid)?.root_path();
                if !map_reaches(&rules, &crate::volume::strip_verbatim(&root.to_string_lossy())) {
                    return Err(IngestError::InvalidInput("pathMap"));
                }
            }
            store::update(&tx, source_id, &n, auth.as_ref(), secrets.as_ref(), now)?;
            tx.commit()?;
            Ok(())
        })?;
        self.config.remote.reload(source_id);
        Ok(self.env.pool.read(|c| Ok(store::view(c, source_id)))??)
    }

    /// 삭제 — 쓰는 폴더가 있으면 `Busy` (먼저 폴더의 감지 방식을 바꿔야 한다).
    pub fn delete_remote_source(&self, source_id: i64) -> Result<(), IngestError> {
        self.env.write(Priority::High, move |c| Ok(store::delete(c, source_id)?))
    }

    /// 접속 시험 — 네트워크를 쓴다(사용자가 누른 경우에만). `source_id` 가 있고 입력에 비밀이 없으면 저장된 비밀을 쓴다.
    /// 호스트 키 지문이 없으면 **인증하지 않고** 지문만 알려 준다.
    pub fn test_remote_source(&self, source_id: Option<i64>, input: RemoteSourceInput) -> Result<RemoteTestResult, IngestError> {
        let (n, auth) = parse_input(&input)?;
        let connector = self.config.remote.connector().clone();
        let Some(host_key) = n.host_key.clone() else {
            return Ok(match connector.fingerprint(&n.host, n.port) {
                Ok(fp) => RemoteTestResult { ok: false, error_key: Some(RunError::HostKeyUnconfirmed { actual: fp.clone() }.key()), fingerprint: Some(fp), file_count: None, row_count: None, max_id: None },
                Err(e) => RemoteTestResult::failed(&e),
            });
        };
        let auth = match (auth, source_id) {
            (Some(a), _) => a,
            (None, Some(id)) => {
                let secrets = self.config.secrets.clone();
                self.env.pool.read(|c| Ok(store::load(c, id, secrets.as_ref())))??.auth
            }
            (None, None) => return Err(IngestError::InvalidInput("secret")),
        };
        let src = RemoteSource {
            source_id: source_id.unwrap_or(0),
            kind: n.kind,
            name: n.name.clone(),
            host: n.host.clone(),
            port: n.port,
            user: n.user.clone(),
            auth,
            host_key: Some(host_key.clone()),
            log_dir: n.log_dir.clone(),
            db_path: n.db_path.clone(),
            poll_sec: n.poll_sec,
            path_map: n.path_map.clone(),
        };
        let ok = |file_count, row_count, max_id| RemoteTestResult { ok: true, error_key: None, fingerprint: Some(host_key.clone()), file_count, row_count, max_id };
        Ok(match (src.kind, connector.connect(&src)) {
            (_, Err(e)) => RemoteTestResult::failed(&e),
            // sqlite3 존재·DB 읽기 가능 여부와 큐의 행 수·최대 id (읽기 전용)
            (SourceKind::SesQueue, Ok(mut run)) => match ses::probe(run.as_mut(), src.db_path.as_deref().unwrap_or_default()) {
                Ok(p) => ok(None, Some(p.rows), Some(p.max_id)),
                Err(e) => RemoteTestResult::failed(&e),
            },
            (SourceKind::DdpAudit, Ok(mut run)) => match list_sizes(run.as_mut(), src.log_dir()) {
                Ok(sizes) => ok(Some(sizes.len() as u32), None, None),
                Err(e) => RemoteTestResult::failed(&e),
            },
        })
    }
}
