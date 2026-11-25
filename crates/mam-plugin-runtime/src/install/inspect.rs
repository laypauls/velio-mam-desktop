//! 설치 전 점검 — 폴더를 읽기만 하고 미리보기(권한·명령·설치 상태 비교)를 만든다.

use std::path::Path;

use mam_core::storage::paths_overlap;
use mam_db::plugin_settings::{GRANTED, INSTALLED, get};
use semver::Version;

use super::package::{Package, read_package};
use super::store::{known_perms, parse_installed};
use crate::api::{InstallChange, InstallError, InstalledSummary, PluginPreview};
use crate::runtime::PluginRuntime;
use crate::runtime::l10n;

impl PluginRuntime {
    /// 폴더를 점검한다 (쓰지 않는다)
    pub fn inspect_dir(&self, dir: &Path, locale: &str) -> Result<PluginPreview, InstallError> {
        let pkg = self.read_source(dir)?;
        self.preview_of(&pkg, locale)
    }

    /// 절대 경로·존재하는 폴더·MAMData 와 겹치지 않음을 **아무것도 읽기 전에** 확인하고 패키지를 읽는다
    pub(crate) fn read_source(&self, dir: &Path) -> Result<Package, InstallError> {
        if !dir.is_absolute() {
            return Err(InstallError::NotAbsolute);
        }
        if !std::fs::metadata(dir).is_ok_and(|m| m.is_dir()) {
            return Err(InstallError::NotADirectory);
        }
        if paths_overlap(dir, &self.deps.data_root) {
            return Err(InstallError::OverlapsDataDir);
        }
        read_package(dir, Some(&self.deps.app_version))
    }

    fn preview_of(&self, pkg: &Package, locale: &str) -> Result<PluginPreview, InstallError> {
        let m = &pkg.manifest;
        let id = m.id.as_str().to_owned();
        let (installed_raw, granted_raw) = self.db_read(|c| Ok((get(c, INSTALLED, &id)?, get(c, GRANTED, &id)?)))?;
        let previous = installed_raw.as_deref().and_then(parse_installed);
        let granted = known_perms(granted_raw.as_deref());
        let change = match &previous {
            None => InstallChange::Install,
            Some((v, _)) => match Version::parse(v) {
                Ok(old) if old == m.version => InstallChange::Reinstall,
                Ok(old) if old > m.version => InstallChange::Downgrade,
                _ => InstallChange::Upgrade,
            },
        };
        let has_kept_data = previous.is_none() && self.db_read(|c| kept_data_exists(c, &id))?;
        let added_permissions = if previous.is_some() { m.permissions.difference(&granted).copied().collect() } else { Vec::new() };
        let b = &pkg.locales;
        Ok(PluginPreview {
            id: m.id.to_string(),
            version: m.version.to_string(),
            display_name: l10n::display_name(m, b, locale),
            description: l10n::description(m, b, locale),
            permissions: m.permissions.iter().copied().collect(),
            commands: l10n::commands_info(m, b, locale),
            providers: l10n::providers_info(m, b, locale),
            unsigned: true,
            installed: previous.map(|(version, _)| InstalledSummary { version, granted: granted.into_iter().collect() }),
            change,
            added_permissions,
            has_kept_data,
        })
    }
}

/// 제거할 때 남긴 데이터(KV 행·대기/실패 작업)가 있는가 — 공개 조회 함수만 쓴다
/// (done 뿐인 작업 행은 의도적으로 "남은 데이터"로 세지 않는다 — 사용자에게 보일 가치가 없는 완료 기록)
fn kept_data_exists(c: &rusqlite::Connection, id: &str) -> Result<bool, mam_db::DbError> {
    use mam_db::{plugin_kv, plugin_tasks};
    Ok(plugin_kv::sum_cost(c, id)? > 0 || plugin_tasks::pending_count(c, id)? > 0 || plugin_tasks::failed_count(c, id)? > 0)
}
