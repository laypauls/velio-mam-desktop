//! `JobHooks` 구현 — 런타임(P6)이 설치·활성화 뒤에 부른다.

use mam_core::jobs::JobKind;
use mam_db::plugin_tasks::{enqueue_jobs_tx, init_feed_cursor, pending_page};

use super::PluginJobs;
use super::core::{Core, REQUEUE_BATCH, put_err};
use crate::api::{JobHooks, PluginId};

impl JobHooks for PluginJobs {
    /// 새로 설치(`fresh`)하면 피드 커서를 현재 끝으로 — 설치 이전의 이벤트는 작업으로 만들지 않는다 (Q3). 업그레이드는 커서를 건드리지 않는다
    fn installed(&self, plugin: &PluginId, fresh: bool) {
        if !fresh {
            return;
        }
        let id = plugin.as_str().to_owned();
        let res = self.core.write_installed(plugin.as_str(), move |tx, _| init_feed_cursor(tx, &id).map_err(put_err));
        if let Err(e) = res {
            tracing::warn!(plugin = plugin.as_str(), error = %e, "cannot initialize the plugin feed cursor (lazy init at the next tick)");
        }
    }

    /// 활성화되면 대기 중이던 작업(꺼진 동안 쌓였거나 `Disabled` 로 남은 것)을 `jobs` 에 다시 올린다
    fn enabled(&self, plugin: &PluginId) {
        requeue_pending(&self.core, plugin.as_str());
    }
}

/// 플러그인의 `pending` 작업이 있는 자산마다 `jobs` 행을 올린다 (`task_id` 키셋 배치, 배치마다 한 쓰기)
fn requeue_pending(core: &Core, plugin: &str) {
    let priority = JobKind::Plugin.default_priority();
    let mut after = 0i64;
    loop {
        if core.halted() {
            return;
        }
        let page = match core.read(|c| Ok(pending_page(c, plugin, after, REQUEUE_BATCH)?)) {
            Ok(p) => p,
            Err(e) => return tracing::warn!(plugin, error = %e, "cannot read pending plugin tasks"),
        };
        let Some(&(last, _)) = page.last() else { break };
        let full = page.len() as i64 >= REQUEUE_BATCH;
        let mut assets: Vec<i64> = page.iter().map(|&(_, a)| a).collect();
        assets.sort_unstable();
        assets.dedup();
        match core.write_installed(plugin, move |tx, now| Ok(enqueue_jobs_tx(tx, &assets, priority, now)?)) {
            Ok(Some(_)) => {}
            Ok(None) => return, // 그 사이 제거됨
            Err(e) => return tracing::warn!(plugin, error = %e, "cannot re-queue pending plugin tasks"),
        }
        after = last;
        if !full {
            break;
        }
    }
    core.queue.notify();
}
