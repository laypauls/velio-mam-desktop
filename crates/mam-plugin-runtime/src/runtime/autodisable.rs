//! 자동 비활성화 — 반복 결함으로 호스트 차단기가 열리면 `plugin_enabled=false` 를 저장하고 권한을 닫고 알린다.
//! 호스트 락이 풀린 뒤(호출이 끝난 뒤)에만 실행한다. 쓰기는 `lifecycle` 아래에서 하므로 설치·제거와 섞이지 않는다.

use std::sync::Arc;

use mam_db::Priority;
use mam_db::plugin_settings::{ENABLED, INSTALLED, get};
use mam_plugin_host::{HostState, PluginHost};

use super::{Entry, PluginRuntime};
use crate::api::{ChangeReason, InvokeError, RunError};
use crate::install::store::put_row;

impl PluginRuntime {
    /// 호출 오류 -> `RunError`. 차단기가 열렸으면 자동 비활성화한다 (`Disabled` 오류는 `RunError::Disabled`)
    pub(crate) fn after_invoke_error(&self, entry: &Entry, host: &Arc<PluginHost>, e: InvokeError) -> RunError {
        let already = e == InvokeError::Disabled;
        if already || (e.is_fault() && host.state() == HostState::Disabled) {
            self.auto_disable(&entry.id, host);
        }
        if already { RunError::Disabled } else { RunError::Invoke(e) }
    }

    fn auto_disable(&self, id: &str, host: &Arc<PluginHost>) {
        let _guard = self.lifecycle.lock().unwrap_or_else(|p| p.into_inner());
        // 그 사이 제거·교체됐거나 이미 꺼졌으면 할 일이 없다
        let Some(cur) = self.entry(id).filter(|c| c.enabled && c.host.as_ref().is_some_and(|h| Arc::ptr_eq(h, host))) else { return };
        let key = id.to_owned();
        let persisted = self.db_write(Priority::Low, move |c| {
            if get(c, INSTALLED, &key)?.is_none() {
                return Ok(false); // 제거된 플러그인의 설정 행을 되살리지 않는다
            }
            put_row(c, ENABLED, &key, "false")?;
            Ok(true)
        });
        let Some(pid) = cur.plugin_id() else { return };
        match persisted {
            Ok(false) => return,
            Ok(true) => {
                self.put_entry(Entry { enabled: false, ..(*cur).clone() });
                self.refresh_grants(&pid);
            }
            Err(e) => {
                // 저장하지 못해도 이번 실행에서는 닫는다 (실패 시 닫힘) — 다음 부팅은 DB 값을 따른다
                tracing::error!(plugin = id, error = %e, "failed to persist auto-disable; closing in memory");
                self.put_entry(Entry { enabled: false, ..(*cur).clone() });
                if let Some(pkg) = cur.pkg.as_ref() {
                    self.gateway.grants().register(&pid, &pkg.manifest.permissions);
                }
            }
        }
        tracing::warn!(plugin = id, "plugin auto-disabled after repeated faults");
        self.emit(id, ChangeReason::AutoDisabled);
    }
}
