//! 설치·업그레이드·재설치·다운그레이드 (C1). 단계 순서는 명세와 같다:
//! 재점검(TOCTOU) -> 권한 검사 -> 스테이징 복사·rename -> 설정 트랜잭션 -> 런타임 교체 -> 옛 버전 삭제·훅·알림.

use std::path::Path;

use mam_db::Priority;
use mam_db::plugin_settings::{ENABLED, GRANTED, INSTALLED, get};
use semver::Version;

use super::fsx::remove_confined;
use super::stage::{promote, write_staging};
use super::store::{granted_raw, installed_raw, parse_installed, put_row};
use crate::api::{ChangeReason, InstallError, PluginId, PluginInfo};
use crate::runtime::{PluginRuntime, parse_granted_perms};

struct Done {
    id: String,
    pid: PluginId,
    fresh: bool,
}

impl PluginRuntime {
    /// `expected_*` 는 미리보기 때 본 값 — 그 사이 폴더가 바뀌었으면 `PreviewMismatch`. `granted` 는 정확히 이 집합으로 저장한다
    pub fn install(&self, dir: &Path, expected_id: &str, expected_version: &str, granted: Vec<String>, locale: &str) -> Result<PluginInfo, InstallError> {
        let done = {
            let _guard = self.try_lifecycle()?;
            self.install_locked(dir, expected_id, expected_version, &granted)?
        };
        // 락 밖 — 훅은 설정 커밋 뒤, 항목이 보이는 시점
        self.current_hooks().installed(&done.pid, done.fresh);
        self.emit(&done.id, if done.fresh { ChangeReason::Installed } else { ChangeReason::Upgraded });
        let current = self.entry(&done.id).ok_or(InstallError::NotInstalled)?;
        Ok(self.info_of(&current, locale))
    }

    fn install_locked(&self, dir: &Path, expected_id: &str, expected_version: &str, granted: &[String]) -> Result<Done, InstallError> {
        let pkg = self.read_source(dir)?;
        let m = &pkg.manifest;
        if m.id.as_str() != expected_id || m.version.to_string() != expected_version {
            return Err(InstallError::PreviewMismatch);
        }
        let granted = parse_granted_perms(granted, &m.permissions)?;
        let (id, version, pid) = (m.id.to_string(), m.version.to_string(), m.id.clone());
        let previous_raw = self.db_read(|c| Ok(get(c, INSTALLED, &id)?))?;
        let previous_version = previous_raw.as_deref().and_then(parse_installed).map(|(v, _)| v);

        let plugins_dir = &self.deps.plugins_dir;
        let staged = write_staging(plugins_dir, &id, &pkg)?;
        let promoted = promote(plugins_dir, &id, &version, &staged)?;

        let now = self.deps.clock.now_ms();
        let (key, installed, granted_text) = (id.clone(), installed_raw(&version, now), granted_raw(&granted));
        let written = self.db_write(Priority::High, move |c| {
            let tx = c.transaction()?;
            put_row(&tx, INSTALLED, &key, &installed)?;
            put_row(&tx, ENABLED, &key, "true")?;
            put_row(&tx, GRANTED, &key, &granted_text)?;
            tx.commit()?;
            Ok(())
        });
        if let Err(e) = written {
            promoted.rollback(plugins_dir);
            return Err(e.into());
        }

        // 런타임 교체: 새 항목 준비(권한 등록) -> 권한 갱신 -> 맵 교체 -> 옛 호스트 종료
        let entry = self.build_entry(pkg, true, granted, now);
        self.refresh_grants(&pid);
        if let Some(old) = self.put_entry(entry)
            && let Some(host) = old.host.as_ref()
        {
            host.shutdown();
        }
        promoted.commit(plugins_dir);
        if let Some(old_version) = previous_version.filter(|v| *v != version && Version::parse(v).is_ok()) {
            let old_dir = plugins_dir.join(&id).join(old_version);
            if let Err(e) = remove_confined(plugins_dir, &old_dir) {
                tracing::warn!(plugin = id.as_str(), error = %e, "old plugin version not removed; boot cleanup will retry");
            }
        }
        Ok(Done { id, pid, fresh: previous_raw.is_none() })
    }
}
