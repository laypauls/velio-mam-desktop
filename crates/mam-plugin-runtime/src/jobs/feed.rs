//! 변경 피드 (B3) — `search_outbox` 의 새 이벤트를 플러그인별 `plugin_tasks` + `jobs` 로 접어 넣는다.
//!
//! 틱마다 플러그인 하나당 쓰기 트랜잭션 최대 1개(설치 확인 -> 구간 접기 -> 작업 등록 -> 커서 기록). 커서와 작업이 같은 트랜잭션이라
//! 비정상 종료가 구간을 건너뛰지 못한다. backpressure(대기 작업 상한)에 닿으면 커서를 멈춘다.

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::thread::JoinHandle;

use mam_core::jobs::JobKind;
use mam_db::plugin_tasks::{
    ProviderTypes, enqueue_jobs_tx, feed_batch, feed_cursor, init_feed_cursor, kind_busy,
    max_event_id, pending_count, set_feed_cursor,
};

use super::core::{Core, JobsTuning, put_err};
use crate::api::{AssetKind, ProviderKind, ProviderSpec};

pub(super) fn spawn(core: Arc<Core>) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("mam-plugin-feed".into())
        .spawn(move || run(&core))
}

fn run(core: &Core) {
    loop {
        if core.halted() {
            return;
        }
        let t = core.tuning();
        let more = tick(core, &t);
        if core
            .stopper
            .sleep(if more { t.feed_busy_pause } else { t.feed_idle })
        {
            return;
        }
    }
}

/// 피드가 비켜 주는 종류 — 스캔 파이프라인이 만드는 작업(스캔·메타·썸네일·해시). 사용자가 시작하는 장시간 작업(AI·장면)과
/// 인덱스 작업은 제외한다(몇 시간씩 피드를 굶기지 않도록). 일시정지된 종류는 진행하지 않으므로 기다리지 않는다.
pub(super) const SCAN_FAMILY: [JobKind; 4] =
    [JobKind::Scan, JobKind::Meta, JobKind::Thumb, JobKind::Hash];

fn scan_family_busy(core: &Core) -> bool {
    SCAN_FAMILY.iter().any(|k| {
        !core.queue.is_paused(*k)
            && core
                .read(|c| Ok(kind_busy(c, k.as_str())?))
                .unwrap_or_else(|e| warn_false(&e))
    })
}

fn warn_false(e: &dyn std::fmt::Display) -> bool {
    tracing::warn!(error = %e, "plugin feed scan-busy check failed");
    false
}

pub(super) fn media_type(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "image",
        AssetKind::Video => "video",
        AssetKind::Audio => "audio",
    }
}

/// 한 번 훑는다. 아직 처리할 구간이 남은 플러그인이 있으면 `true`
pub(super) fn tick(core: &Core, t: &JobsTuning) -> bool {
    if core.queue.is_paused(JobKind::Plugin) || scan_family_busy(core) {
        return false;
    }
    let mut by_plugin: BTreeMap<String, Vec<ProviderSpec>> = BTreeMap::new();
    for spec in core.invoker.auto_providers() {
        if spec.kind != ProviderKind::Export {
            by_plugin
                .entry(spec.plugin.as_str().to_owned())
                .or_default()
                .push(spec);
        }
    }
    let mut more = false;
    for (plugin, specs) in &by_plugin {
        if core.halted() || core.queue.is_paused(JobKind::Plugin) {
            break;
        }
        more |= feed_plugin(core, t, plugin, specs);
    }
    more
}

/// 플러그인 하나의 한 구간. 구간이 남았으면 `true`
fn feed_plugin(core: &Core, t: &JobsTuning, plugin: &str, specs: &[ProviderSpec]) -> bool {
    // 읽기 단계: 상한·커서·끝 — 쓰기 스레드를 점유하지 않는다
    let view = core.read(|c| {
        Ok((
            pending_count(c, plugin)?,
            feed_cursor(c, plugin)?,
            max_event_id(c)?,
        ))
    });
    let (pending, cursor, max) = match view {
        Ok(v) => v,
        Err(e) => return warn(plugin, "read", &e),
    };
    let Some(cursor) = cursor else {
        // 커서가 없으면(설치 훅 실패·손상) 지금 끝으로 — 이후 이벤트부터 처리한다
        let id = plugin.to_owned();
        if let Err(e) = core.write_installed(plugin, move |tx, _| {
            init_feed_cursor(tx, &id).map_err(put_err)
        }) {
            warn(plugin, "cursor init", &e);
        }
        return false;
    };
    if pending >= t.pending_cap || cursor >= max {
        return false;
    }
    let upto = max.min(cursor.saturating_add(t.feed_batch));
    let kinds: Vec<(String, Vec<&'static str>)> = specs
        .iter()
        .map(|s| {
            (
                s.provider.clone(),
                s.asset_kinds.iter().map(|k| media_type(*k)).collect(),
            )
        })
        .collect();
    let priority = JobKind::Plugin.default_priority();
    let id = plugin.to_owned();
    let done = core.write_installed(plugin, move |tx, now| {
        // 커서가 읽기 이후 바뀌었으면(설치 훅 등) 이번 구간은 건너뛴다
        if feed_cursor(tx, &id)? != Some(cursor) {
            return Ok(0);
        }
        let providers: Vec<ProviderTypes<'_>> = kinds
            .iter()
            .map(|(p, k)| (p.as_str(), k.as_slice()))
            .collect();
        let ids = feed_batch(tx, &id, &providers, cursor, upto, now)?;
        enqueue_jobs_tx(tx, &ids, priority, now)?;
        set_feed_cursor(tx, &id, upto).map_err(put_err)?;
        Ok(ids.len())
    });
    match done {
        Ok(Some(queued)) => {
            if queued > 0 {
                core.queue.notify();
            }
            upto < max
        }
        Ok(None) => false,
        Err(e) => warn(plugin, "write", &e),
    }
}

fn warn(plugin: &str, step: &str, e: &dyn std::fmt::Display) -> bool {
    tracing::warn!(plugin, step, error = %e, "plugin feed step failed");
    false
}
