//! 패키지 -> 레지스트리 항목. 호스트는 여기서만 만든다 (플러그인당 정확히 1개, 상태 Ok 일 때만).

use std::collections::BTreeSet;
use std::sync::Arc;

use mam_plugin_host::{HostApi, HostConfig, PluginHost};
use mam_plugin_manifest::check_engine;

use super::{Entry, PluginRuntime, Pkg};
use crate::api::{Permission, PluginStatus};
use crate::install::package::Package;

impl PluginRuntime {
    /// 워커를 못 찾았으면(`worker_exe: None`) 빈 경로를 준다 — 호스트는 빈 경로를 절대 띄우지 않고 호출은 `InvokeError::Start` 가 된다.
    /// MAMData 안의 대체 경로로 떨어지지 않는다(거기에 심은 실행 파일이 실행되는 것을 막는다)
    fn make_host(&self, pkg: &Package, source: String) -> Arc<PluginHost> {
        let exe = self.deps.worker_exe.clone().unwrap_or_default();
        let mut cfg = HostConfig::new(exe, source);
        let tweak = self.knobs.read().unwrap_or_else(|p| p.into_inner()).host_tweak.clone();
        if let Some(tweak) = tweak {
            tweak(&mut cfg);
        }
        let api: Arc<dyn HostApi> = Arc::clone(&self.gateway) as Arc<dyn HostApi>;
        Arc::new(PluginHost::new(pkg.manifest.id.clone(), cfg, api, Arc::clone(&self.deps.host_clock)))
    }

    /// 읽은 패키지 + 저장된 상태 -> 항목. 앱 버전이 안 맞으면 Incompatible(호스트·권한 등록 없음 = 닫힘).
    /// Ok 면 권한 레지스트리에 선언 권한을 등록한다(허용 상태는 `refresh` 전까지 닫힘)
    pub(crate) fn build_entry(&self, pkg: Package, enabled: bool, granted: BTreeSet<Permission>, installed_at: i64) -> Entry {
        let compatible = check_engine(&pkg.manifest, &self.deps.app_version).is_ok();
        let host = compatible.then(|| {
            let host = self.make_host(&pkg, pkg.main_source.clone());
            self.gateway.grants().register(&pkg.manifest.id, &pkg.manifest.permissions);
            host
        });
        Entry {
            id: pkg.manifest.id.to_string(),
            version: pkg.manifest.version.to_string(),
            status: if compatible { PluginStatus::Ok } else { PluginStatus::Incompatible },
            enabled,
            granted,
            installed_at,
            host,
            pkg: Some(Arc::new(Pkg { manifest: pkg.manifest, locales: pkg.locales })),
        }
    }
}
