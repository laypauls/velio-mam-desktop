//! 활성·권한 관리 — 사용자 작업이라 `lifecycle` `try_lock`(실패 시 `Busy`), 설정 쓰기는 High 우선순위.
//! 훅·알림·워커 종료는 락을 놓은 뒤에 한다 (훅이 런타임을 다시 불러도 교착하지 않게).

use std::collections::BTreeSet;
use std::sync::Arc;

use mam_db::Priority;
use mam_db::plugin_settings::{ENABLED, GRANTED, valid_id};
use mam_plugin_manifest::check_engine;

use super::{Entry, PluginRuntime};
use crate::api::{ChangeReason, InstallError, Permission, PluginInfo, PluginStatus};
use crate::install::store::{granted_raw, put_row};

impl PluginRuntime {
    /// 변경할 수 있는 항목 — 없으면 `NotInstalled`, 깨졌으면 `ManifestMissing`, 앱 버전이 안 맞으면 `Engine`
    fn manageable(&self, id: &str) -> Result<Arc<Entry>, InstallError> {
        let entry = self
            .entry(id)
            .filter(|_| valid_id(id))
            .ok_or(InstallError::NotInstalled)?;
        match (&entry.pkg, entry.status) {
            (Some(_), PluginStatus::Ok) => Ok(entry),
            (Some(pkg), _) => match check_engine(&pkg.manifest, &self.deps.app_version) {
                Err(e) => Err(InstallError::Engine(e)),
                Ok(()) => Err(InstallError::ManifestMissing),
            },
            (None, _) => Err(InstallError::ManifestMissing),
        }
    }

    pub fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        locale: &str,
    ) -> Result<PluginInfo, InstallError> {
        let (new_entry, pid) = {
            let _guard = self.try_lifecycle()?;
            let entry = self.manageable(id)?;
            let pid = entry.plugin_id().ok_or(InstallError::NotInstalled)?;
            if let (true, Some(host)) = (enabled, entry.host.as_ref()) {
                host.reenable(); // 차단기 이력을 지운다
            }
            let key = id.to_owned();
            self.db_write(Priority::High, move |c| {
                put_row(c, ENABLED, &key, if enabled { "true" } else { "false" })
            })?;
            self.put_entry(Entry {
                enabled,
                ..(*entry).clone()
            });
            self.refresh_grants(&pid);
            (Arc::clone(&entry), pid)
        };
        if enabled {
            self.current_hooks().enabled(&pid);
        } else if let Some(host) = new_entry.host.as_ref() {
            host.shutdown(); // 진행 중 호출이 끝나길 기다린 뒤 워커를 종료한다
        }
        self.emit(
            id,
            if enabled {
                ChangeReason::Enabled
            } else {
                ChangeReason::Disabled
            },
        );
        let current = self.entry(id).ok_or(InstallError::NotInstalled)?;
        Ok(self.info_of(&current, locale))
    }

    /// 허용 권한을 정확히 `perms` 로 바꾼다 — 아는 권한 문자열이고 manifest 가 선언한 것의 부분집합이어야 한다
    pub fn grant_permissions(
        &self,
        id: &str,
        perms: Vec<String>,
        locale: &str,
    ) -> Result<PluginInfo, InstallError> {
        let _guard = self.try_lifecycle()?;
        let entry = self.manageable(id)?;
        let (pid, pkg) = (
            entry.plugin_id().ok_or(InstallError::NotInstalled)?,
            entry.pkg.as_ref().ok_or(InstallError::NotInstalled)?,
        );
        let granted = parse_granted_perms(&perms, &pkg.manifest.permissions)?;
        let (key, raw) = (id.to_owned(), granted_raw(&granted));
        self.db_write(Priority::High, move |c| put_row(c, GRANTED, &key, &raw))?;
        self.put_entry(Entry {
            granted,
            ..(*entry).clone()
        });
        self.refresh_grants(&pid);
        self.emit(id, ChangeReason::PermissionsChanged);
        let current = self.entry(id).ok_or(InstallError::NotInstalled)?;
        Ok(self.info_of(&current, locale))
    }
}

/// 권한 문자열 -> 집합. 모르는 문자열이거나 선언하지 않은 권한이면 `UnknownPermission`
pub(crate) fn parse_granted_perms(
    perms: &[String],
    declared: &BTreeSet<Permission>,
) -> Result<BTreeSet<Permission>, InstallError> {
    let mut out = BTreeSet::new();
    for s in perms {
        let p: Permission = s.parse().map_err(|_| InstallError::UnknownPermission)?;
        if !declared.contains(&p) {
            return Err(InstallError::UnknownPermission);
        }
        out.insert(p);
    }
    Ok(out)
}
