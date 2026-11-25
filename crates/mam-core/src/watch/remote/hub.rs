//! 원격 접속 허브 — 접속(`remote_sources` 행) 하나당 수집기 스레드 하나, 폴더 여럿이 공유한다.
//!
//! 감지기(`ddp_audit`)가 폴더 감시를 시작할 때 `attach`, 감시가 멈추면(핸들 drop) `detach` 한다. 마지막 폴더가 떨어지면 스레드를 멈춘다.
//! DB·시계·기록기는 `Ingest::new` 가 `bind` 로 꽂는다 (감지기는 `IngestConfig` 단계에서 만들어져 그때는 DB 가 없다).
//! 스레드는 `StopSignal`(조건 변수)로만 깨운다 — 폴링 루프·PC 전역 조건 없음.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use mam_db::DbPool;

use super::collector::{Attached, BatchCommitter, Collector, Deps, Feed, Folders, SourceLoader, Timings};
use super::ses_collector::SesCollector;
use super::mapper::map_reaches;
use super::runner::RemoteConnector;
use super::stop::{StopSignal, Woke};
use super::store;
use super::SourceKind;
use crate::SharedClock;
use crate::secret::SecretBox;
use crate::volume::strip_verbatim;
use crate::watch::{ChangeSink, DetectError};

/// `bind` 로 꽂히는 런타임 의존성
pub struct HubDeps {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub committer: Arc<dyn BatchCommitter>,
    pub secrets: Arc<dyn SecretBox>,
}

struct Running {
    folders: Folders,
    stop: Arc<StopSignal>,
    /// 수집기가 한 주기 이상 돌았다 — 이후 붙는 폴더는 그 전의 줄을 못 봤으니 재스캔을 1회 요청한다
    started: Arc<AtomicBool>,
}

pub struct RemoteHub {
    connector: Arc<dyn RemoteConnector>,
    timings: Timings,
    deps: OnceLock<HubDeps>,
    running: Mutex<HashMap<i64, Running>>,
}

impl std::fmt::Debug for RemoteHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteHub").field("bound", &self.deps.get().is_some()).finish_non_exhaustive()
    }
}

/// 폴더가 접속에 붙어 있는 동안 쥔다 — 버리면 떨어진다.
pub struct AttachGuard {
    hub: Arc<RemoteHub>,
    source_id: i64,
    folder_id: i64,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        self.hub.detach(self.source_id, self.folder_id);
    }
}

impl RemoteHub {
    pub fn new(connector: Arc<dyn RemoteConnector>) -> Arc<Self> {
        Self::with_timings(connector, Timings::default())
    }

    pub fn with_timings(connector: Arc<dyn RemoteConnector>, timings: Timings) -> Arc<Self> {
        Arc::new(Self { connector, timings, deps: OnceLock::new(), running: Mutex::new(HashMap::new()) })
    }

    /// DB·시계·기록기를 꽂는다 (처음 한 번만 — 이미 꽂혀 있으면 무시).
    pub fn bind(&self, deps: HubDeps) {
        let _ = self.deps.set(deps);
    }

    pub fn is_bound(&self) -> bool {
        self.deps.get().is_some()
    }

    pub fn connector(&self) -> &Arc<dyn RemoteConnector> {
        &self.connector
    }

    pub fn secrets(&self) -> Option<&Arc<dyn SecretBox>> {
        self.deps.get().map(|d| &d.secrets)
    }

    fn running(&self) -> MutexGuard<'_, HashMap<i64, Running>> {
        self.running.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 접속의 종류·저장된 설정 요약 (복호하지 않는다). 허브가 아직 꽂히지 않았거나 없으면 `None`.
    pub fn view(&self, source_id: i64) -> Option<store::SourceView> {
        let d = self.deps.get()?;
        d.pool.read(|c| Ok(store::view(c, source_id))).ok()?.ok()
    }

    pub fn kind_of(&self, source_id: i64) -> Option<SourceKind> {
        let d = self.deps.get()?;
        d.pool.read(|c| Ok(store::kind_of(c, source_id))).ok()?
    }

    /// 접속 설정이 바뀌었다 — 도는 수집기가 접속을 닫고 새 설정을 읽게 한다.
    pub fn reload(&self, source_id: i64) {
        if let Some(r) = self.running().get(&source_id) {
            r.stop.reload();
        }
    }

    /// 폴더를 접속에 붙인다. 접속이 아직 없으면 수집기 스레드를 시작한다.
    pub fn attach(
        self: &Arc<Self>,
        source_id: i64,
        kind: SourceKind,
        folder_id: i64,
        root: &Path,
        sink: ChangeSink,
    ) -> Result<AttachGuard, DetectError> {
        let d = self.deps.get().ok_or_else(|| DetectError::Start("remote hub is not ready".into()))?;
        let loader = loader_for(d);
        let src = loader(source_id).ok_or_else(|| DetectError::Start("remote source not found".into()))?;
        if src.kind != kind {
            return Err(DetectError::Start("remote source kind mismatch".into()));
        }
        let root = strip_verbatim(&root.to_string_lossy());
        if !map_reaches(&src.path_map, &root) {
            return Err(DetectError::Start("path map does not reach the folder".into()));
        }
        let mut map = self.running();
        if let Some(r) = map.get(&source_id) {
            let started = r.started.load(Ordering::SeqCst);
            lock(&r.folders).push(Attached { folder_id, root, sink, needs_rescan: started });
        } else {
            let folders: Folders = Arc::new(Mutex::new(vec![Attached { folder_id, root, sink, needs_rescan: false }]));
            let stop = Arc::new(StopSignal::default());
            let started = Arc::new(AtomicBool::new(false));
            let deps = Deps {
                connector: Arc::clone(&self.connector),
                committer: Arc::clone(&d.committer),
                clock: Arc::clone(&d.clock),
                timings: self.timings,
                folders: Arc::clone(&folders),
                loader: Arc::clone(&loader),
            };
            let collector: Box<dyn Feed> = match kind {
                SourceKind::DdpAudit => Box::new(Collector::new(src, deps)),
                SourceKind::SesQueue => Box::new(SesCollector::new(src, deps)),
            };
            let (s, st) = (Arc::clone(&stop), Arc::clone(&started));
            std::thread::Builder::new()
                .name(format!("mam-remote-{source_id}"))
                .spawn(move || collector_loop(collector, &s, &st, &loader, source_id))
                .map_err(|e| DetectError::Start(format!("cannot start the remote collector: {e}")))?;
            map.insert(source_id, Running { folders, stop, started });
        }
        Ok(AttachGuard { hub: Arc::clone(self), source_id, folder_id })
    }

    fn detach(&self, source_id: i64, folder_id: i64) {
        let mut map = self.running();
        let Some(r) = map.get(&source_id) else { return };
        let empty = {
            let mut f = lock(&r.folders);
            f.retain(|a| a.folder_id != folder_id);
            f.is_empty()
        };
        if empty && let Some(r) = map.remove(&source_id) {
            // 기다리지 않는다 — 스레드는 신호를 보고 스스로 끝난다 (진행 중인 원격 명령은 제한 시간 안에 끝난다)
            r.stop.stop();
        }
    }

    /// 붙어 있는 접속 수 (진단·시험)
    pub fn running_sources(&self) -> usize {
        self.running().len()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn loader_for(d: &HubDeps) -> SourceLoader {
    let (pool, secrets) = (Arc::clone(&d.pool), Arc::clone(&d.secrets));
    Arc::new(move |id| match pool.read(|c| Ok(store::load(c, id, secrets.as_ref()))) {
        Ok(Ok(s)) => Some(s),
        Ok(Err(e)) => {
            // 비밀·입력 값은 오류에 없다 (필드 이름·종류만)
            tracing::warn!(source_id = id, error = %e, "cannot load the remote source");
            None
        }
        Err(e) => {
            tracing::warn!(source_id = id, error = %e, "cannot read the remote source");
            None
        }
    })
}

fn collector_loop(mut c: Box<dyn Feed>, stop: &StopSignal, started: &AtomicBool, loader: &SourceLoader, id: i64) {
    loop {
        if stop.is_stopped() {
            break;
        }
        if stop.take_reload() {
            match loader(id) {
                Some(s) => c.reload(s),
                None => break, // 삭제됨 (사용 중이면 삭제가 거부되므로 정상 경로는 아니다)
            }
        }
        let wait = c.step();
        started.store(true, Ordering::SeqCst);
        if stop.wait(wait) == Woke::Stop {
            break;
        }
    }
}

/// 레지스트리에 꽂는 참조 해석기 — `source` 옵션이 `remote_sources` 에 있는 접속을 가리키는지 확인한다 (종류는 감지기가 다시 확인한다).
pub struct HubRefs(pub Arc<RemoteHub>);

impl crate::watch::RefResolver for HubRefs {
    fn exists(&self, field: &str, id: i64) -> bool {
        field == "source" && self.0.kind_of(id).is_some()
    }
}
