//! 목록·기여 항목 조회 — 읽기 락은 항목 `Arc` 를 복제하는 동안만 쥔다.

use std::sync::Arc;

use super::l10n::tr;
use super::{Entry, PluginRuntime};
use crate::api::{ExportProviderInfo, MenuItemInfo, PluginContributions, PluginInfo};

impl PluginRuntime {
    fn snapshot(&self) -> Vec<Arc<Entry>> {
        self.registry.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
    }

    /// 깨진·호환 안 되는 플러그인도 포함한다 (id 순)
    pub fn list(&self, locale: &str) -> Vec<PluginInfo> {
        self.snapshot().iter().map(|e| e.info(locale, self.worker_available)).collect()
    }

    pub(crate) fn info_of(&self, entry: &Entry, locale: &str) -> PluginInfo {
        entry.info(locale, self.worker_available)
    }

    /// 활성 + 상태 Ok 플러그인의 메뉴 항목(manifest 순서)과 내보내기 provider. `when` 은 원문 — UI 가 평가한다
    pub fn contributions(&self, locale: &str) -> PluginContributions {
        let mut out = PluginContributions::default();
        for entry in self.snapshot().iter().filter(|e| e.runnable()) {
            let Some(pkg) = entry.pkg.as_ref() else { continue };
            let (m, b) = (&pkg.manifest, &pkg.locales);
            for item in &m.contributes.menus.asset_context {
                let title = m
                    .contributes
                    .commands
                    .iter()
                    .find(|c| c.id == item.command)
                    .map_or_else(|| item.command.to_string(), |c| tr(b, &c.title, locale, c.id.as_str()));
                out.menu_items.push(MenuItemInfo {
                    plugin_id: entry.id.clone(),
                    command_id: item.command.to_string(),
                    title,
                    when: item.when.as_ref().map(|w| w.as_str().to_owned()),
                });
            }
            for p in &m.contributes.providers.export {
                out.export_providers.push(ExportProviderInfo {
                    plugin_id: entry.id.clone(),
                    provider_id: p.id.to_string(),
                    title: tr(b, &p.title, locale, p.id.as_str()),
                    file_extension: p.file_extension.clone(),
                    asset_kinds: p.asset_kinds.clone(),
                });
            }
        }
        out
    }
}
