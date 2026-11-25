//! 레지스트리 항목 — 불변 값. 상태가 바뀌면(활성·권한) 새 `Entry` 로 교체한다(`Arc<PluginHost>` 는 공유).

use std::collections::BTreeSet;
use std::sync::Arc;

use mam_plugin_host::PluginHost;
use mam_plugin_manifest::{LocaleBundle, Manifest};

use super::l10n;
use crate::api::{
    AssetKind, Permission, PluginId, PluginInfo, PluginStatus, ProviderKind, ProviderSpec,
};

/// 읽기 전용 패키지 내용 (Broken 항목에는 없다)
pub(crate) struct Pkg {
    pub manifest: Manifest,
    pub locales: LocaleBundle,
}

#[derive(Clone)]
pub(crate) struct Entry {
    pub id: String,
    pub version: String,
    pub pkg: Option<Arc<Pkg>>,
    pub status: PluginStatus,
    pub enabled: bool,
    pub granted: BTreeSet<Permission>,
    pub installed_at: i64,
    /// 상태 Ok 일 때만 있다 — 플러그인당 1개
    pub host: Option<Arc<PluginHost>>,
}

impl Entry {
    /// 패키지를 읽지 못한 항목 — 목록에는 남고 호출은 되지 않는다
    pub(crate) fn broken(id: &str, version: &str, installed_at: i64) -> Self {
        Self {
            id: id.to_owned(),
            version: version.to_owned(),
            pkg: None,
            status: PluginStatus::Broken,
            enabled: false,
            granted: BTreeSet::new(),
            installed_at,
            host: None,
        }
    }

    /// 호출·자동 실행 대상인가
    pub(crate) fn runnable(&self) -> bool {
        self.status == PluginStatus::Ok && self.enabled && self.host.is_some()
    }

    pub(crate) fn plugin_id(&self) -> Option<PluginId> {
        self.pkg.as_ref().map(|p| p.manifest.id.clone())
    }

    pub(crate) fn provider_spec(&self, provider: &str) -> Option<ProviderSpec> {
        let pkg = self.pkg.as_ref()?;
        let p = &pkg.manifest.contributes.providers;
        let plain = |list: &[mam_plugin_manifest::Provider], kind: ProviderKind| {
            list.iter()
                .find(|x| x.id.as_str() == provider)
                .map(|x| (kind, x.asset_kinds.clone(), None))
        };
        let (kind, asset_kinds, file_extension): (ProviderKind, Vec<AssetKind>, Option<String>) =
            plain(&p.metadata, ProviderKind::Metadata)
                .or_else(|| plain(&p.analysis, ProviderKind::Analysis))
                .or_else(|| {
                    p.export
                        .iter()
                        .find(|x| x.id.as_str() == provider)
                        .map(|x| {
                            (
                                ProviderKind::Export,
                                x.asset_kinds.clone(),
                                Some(x.file_extension.clone()),
                            )
                        })
                })?;
        Some(ProviderSpec {
            plugin: pkg.manifest.id.clone(),
            provider: provider.to_owned(),
            kind,
            asset_kinds,
            file_extension,
        })
    }

    /// 자동 실행 대상(metadata·analysis) provider 목록
    pub(crate) fn auto_specs(&self) -> Vec<ProviderSpec> {
        let Some(pkg) = self.pkg.as_ref() else {
            return Vec::new();
        };
        let p = &pkg.manifest.contributes.providers;
        p.metadata
            .iter()
            .chain(&p.analysis)
            .filter_map(|x| self.provider_spec(x.id.as_str()))
            .collect()
    }

    pub(crate) fn info(&self, locale: &str, worker_available: bool) -> PluginInfo {
        let Some(pkg) = self.pkg.as_ref() else {
            return PluginInfo {
                id: self.id.clone(),
                version: self.version.clone(),
                display_name: self.id.clone(),
                description: None,
                status: self.status,
                enabled: false,
                unsigned: true,
                permissions: Vec::new(),
                granted: Vec::new(),
                commands: Vec::new(),
                providers: Vec::new(),
                installed_at: self.installed_at,
                worker_available: false,
            };
        };
        let (m, b) = (&pkg.manifest, &pkg.locales);
        PluginInfo {
            id: self.id.clone(),
            version: self.version.clone(),
            display_name: l10n::display_name(m, b, locale),
            description: l10n::description(m, b, locale),
            status: self.status,
            enabled: self.enabled,
            unsigned: true,
            permissions: m.permissions.iter().copied().collect(),
            granted: self.granted.intersection(&m.permissions).copied().collect(),
            commands: l10n::commands_info(m, b, locale),
            providers: l10n::providers_info(m, b, locale),
            installed_at: self.installed_at,
            worker_available: self.host.is_some() && worker_available,
        }
    }
}
