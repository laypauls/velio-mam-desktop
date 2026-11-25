//! 전체 실행 (B6) — provider 하나를 모든 활성 자산에 등록한다. 플러그인+provider 당 하나만(single-flight) 도는 백그라운드 스레드가
//! `asset_id` 키셋 배치(기본 5,000)마다 한 Low 트랜잭션으로 작업·`jobs` 를 올린다. 이미 현재 revision 이 끝난 행은 건드리지 않는다.
//! `PENDING_CAP` 은 적용하지 않는다(피드가 백로그 동안 기다린다 — 명세 E5). 제거·종료 시 멈춘다.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use mam_core::jobs::JobKind;
use mam_db::plugin_tasks::{enqueue_jobs_tx, run_all_batch};

use super::core::Core;
use super::feed::media_type;
use crate::api::{ProviderKind, RunAllResult, RunError};

#[derive(Debug, thiserror::Error)]
pub enum RunAllError {
    #[error(transparent)]
    Run(#[from] RunError),
    /// 내보내기 provider 는 자산 전체에 자동 실행하지 않는다
    #[error("export providers cannot be run on all assets")]
    ExportProvider,
    #[error("cannot start the run-all thread: {0}")]
    Spawn(std::io::Error),
}

#[derive(Default)]
struct State {
    running: HashSet<(String, String)>,
    handles: Vec<JoinHandle<()>>,
}

#[derive(Default)]
pub(super) struct Registry {
    state: Mutex<State>,
}

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 이미 돌고 있으면 `false`
    fn begin(&self, key: &(String, String)) -> bool {
        self.lock().running.insert(key.clone())
    }

    fn finish(&self, key: &(String, String)) {
        self.lock().running.remove(key);
    }

    fn keep(&self, h: JoinHandle<()>) {
        let mut s = self.lock();
        s.handles.retain(|h| !h.is_finished());
        s.handles.push(h);
    }

    pub(super) fn join_all(&self) {
        let handles = std::mem::take(&mut self.lock().handles);
        for h in handles {
            let _ = h.join();
        }
    }
}

/// 스레드가 어떻게 끝나든(패닉 포함) single-flight 표시를 지운다
struct Running<'a> {
    reg: &'a Registry,
    key: (String, String),
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.reg.finish(&self.key);
    }
}

pub(super) fn start(
    core: &Arc<Core>,
    reg: &Arc<Registry>,
    plugin: &str,
    provider: &str,
) -> Result<RunAllResult, RunAllError> {
    let spec = core.invoker.provider(plugin, provider)?;
    if spec.kind == ProviderKind::Export {
        return Err(RunAllError::ExportProvider);
    }
    let key = (plugin.to_owned(), provider.to_owned());
    if core.halted() || !reg.begin(&key) {
        return Ok(RunAllResult { started: false });
    }
    let kinds: Vec<&'static str> = spec.asset_kinds.iter().map(|k| media_type(*k)).collect();
    let (c, r, k) = (Arc::clone(core), Arc::clone(reg), key.clone());
    match std::thread::Builder::new()
        .name("mam-plugin-run-all".into())
        .spawn(move || {
            let _running = Running {
                reg: &r,
                key: k.clone(),
            };
            worker(&c, &k.0, &k.1, &kinds);
        }) {
        Ok(h) => {
            reg.keep(h);
            Ok(RunAllResult { started: true })
        }
        Err(e) => {
            reg.finish(&key);
            Err(RunAllError::Spawn(e))
        }
    }
}

fn worker(core: &Core, plugin: &str, provider: &str, kinds: &[&'static str]) {
    let priority = JobKind::Plugin.default_priority();
    let mut after = 0i64;
    loop {
        if core.halted() {
            return;
        }
        let t = core.tuning();
        let (id, prov, ks, limit) = (
            plugin.to_owned(),
            provider.to_owned(),
            kinds.to_vec(),
            t.run_all_batch,
        );
        let done = core.write_installed(plugin, move |tx, now| {
            let page = run_all_batch(
                tx,
                &id,
                &[(prov.as_str(), ks.as_slice())],
                after,
                limit,
                now,
            )?;
            enqueue_jobs_tx(tx, &page.pending, priority, now)?;
            Ok((page.last, page.scanned, page.pending.len()))
        });
        match done {
            // 제거됨 — 멈춘다
            Ok(None) => return,
            Ok(Some((last, scanned, queued))) => {
                if queued > 0 {
                    core.queue.notify();
                }
                match last {
                    Some(l) if i64::try_from(scanned).is_ok_and(|n| n >= limit) => after = l,
                    _ => return,
                }
            }
            Err(e) => {
                return tracing::warn!(plugin, provider, error = %e, "run-all batch failed; stopped");
            }
        }
        // 쓰기 스레드를 다른 쓰기에 양보하고 정지 신호를 듣는다
        if core.stopper.sleep(t.feed_busy_pause) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Concurrency(single-flight 표시)

    #[test]
    fn t_pjob_runall_01_registry_is_single_flight_per_key() {
        let reg = Registry::default();
        let a = ("p".to_owned(), "x".to_owned());
        let b = ("p".to_owned(), "y".to_owned());
        assert!(reg.begin(&a));
        assert!(
            !reg.begin(&a),
            "same plugin+provider is rejected while running"
        );
        assert!(reg.begin(&b), "another provider is independent");
        {
            let _g = Running {
                reg: &reg,
                key: a.clone(),
            };
        }
        assert!(reg.begin(&a), "guard drop frees the slot");
    }
}
