//! 제거 (C2) — 순서는 계약이다: lifecycle 락 -> 항목 제거 -> 권한 해제 -> 설정 행 삭제(1 트랜잭션) -> 호스트 종료(진행 중 호출 대기)
//! -> 패키지 폴더 삭제 -> (`!keep_data`) 데이터 배치 삭제 -> 알림. P5 쓰기는 `plugin_installed` 를 확인하므로 4단계 뒤에는 쓰기가 들어가지 못한다.

use mam_db::Priority;
use mam_db::plugin_data::{BATCH, delete_plugin_batch};
use mam_db::plugin_settings::{INSTALLED, get, valid_id};

use super::fsx::remove_confined;
use super::store::delete_all_rows;
use crate::api::{ChangeReason, InstallError, PluginId};
use crate::runtime::PluginRuntime;

impl PluginRuntime {
    /// `keep_data`: KV·작업 행을 남긴다(재설치하면 다시 보인다). 아니면 배치(기본 5,000행)마다 Low 트랜잭션 1개로 지운다
    pub fn uninstall(&self, id: &str, keep_data: bool) -> Result<(), InstallError> {
        if !valid_id(id) {
            return Err(InstallError::NotInstalled);
        }
        {
            let _guard = self.try_lifecycle()?;
            self.uninstall_locked(id, keep_data)?;
        }
        self.emit(id, ChangeReason::Uninstalled);
        Ok(())
    }

    fn uninstall_locked(&self, id: &str, keep_data: bool) -> Result<(), InstallError> {
        // 2. 새 호출은 NotInstalled
        let entry = self.take_entry(id);
        if entry.is_none() {
            let key = id.to_owned();
            if self.db_read(move |c| Ok(get(c, INSTALLED, &key)?))?.is_none() {
                return Err(InstallError::NotInstalled);
            }
        }
        // 3. 권한 해제
        let pid = PluginId::try_from(id).ok();
        if let Some(p) = &pid {
            self.gateway.grants().unregister(p);
        }
        // 4. 설정 행 삭제 — 1 트랜잭션
        let key = id.to_owned();
        let deleted = self.db_write(Priority::High, move |c| {
            let tx = c.transaction()?;
            delete_all_rows(&tx, &key)?;
            tx.commit()?;
            Ok(())
        });
        if let Err(e) = deleted {
            // 아무것도 지워지지 않았다 — 메모리 상태를 되돌린다
            if let (Some(en), Some(p)) = (entry, &pid) {
                if let Some(pkg) = en.pkg.as_ref() {
                    self.gateway.grants().register(p, &pkg.manifest.permissions);
                }
                self.put_entry((*en).clone());
                self.refresh_grants(p);
            }
            return Err(e.into());
        }
        // 5. 호스트 종료 — 진행 중 호출이 끝나길 기다린다 (워커가 남지 않는다)
        if let Some(host) = entry.as_ref().and_then(|e| e.host.as_ref()) {
            host.shutdown();
        }
        // 6. 패키지 폴더 — 실패해도 설정은 이미 지워졌으니 다음 부팅 정리가 지운다
        if pid.is_some() {
            let dir = self.deps.plugins_dir.join(id);
            if let Err(e) = remove_confined(&self.deps.plugins_dir, &dir) {
                tracing::warn!(plugin = id, error = %e, "plugin folder not removed; boot cleanup will retry");
            }
        }
        // 7. 데이터
        if !keep_data {
            let batch = self.knobs.read().unwrap_or_else(|p| p.into_inner()).delete_batch.unwrap_or(BATCH);
            loop {
                let key = id.to_owned();
                let n = self.db_write(Priority::Low, move |c| {
                    let tx = c.transaction()?;
                    let n = delete_plugin_batch(&tx, &key, batch)?;
                    tx.commit()?;
                    Ok(n)
                })?;
                if n == 0 {
                    break;
                }
            }
        }
        // 계량기는 DB 합계와 어긋날 수 있다(행 삭제) — 비우면 다음 쓰기가 다시 센다
        self.gateway.forget_plugin(id);
        Ok(())
    }
}
