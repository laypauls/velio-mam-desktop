//! 업데이트 확인·설치 command (P1-7, D8). 설치는 `install_update` 를 **명시적으로 호출할 때만** 한다 —
//! 사용자 승인 UI 는 P2-9/P4 이며, 지금은 command 와 `update-state` 이벤트까지다.
//!
//! Rust <-> TS 계약: `src/lib/ipc.ts` 의 `UpdateCheck`, `UpdateStatePayload`, `UpdateError`.

use std::sync::Mutex;

use mam_db::{Clock, SystemClock};
use serde::{Serialize, Serializer};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::update_guard::{self, Paths};

pub const STATE_EVENT: &str = "update-state";

/// `check_update` 가 찾은 업데이트 — `install_update` 가 꺼내 쓴다.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub available: bool,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatePayload {
    /// `downloading` | `installing` | `failed`
    pub phase: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("update check failed: {0}")]
    Check(String),
    #[error("no update has been checked; call check_update first")]
    NotChecked,
    /// 이전 버전 설치기가 없으면 실패 시 복구할 수 없으므로 업데이트를 시작하지 않는다.
    #[error("previous installer not found at {0}; update refused because it cannot be rolled back")]
    PreviousInstallerMissing(String),
    #[error("cannot prepare rollback data: {0}")]
    Prepare(String),
    #[error("update install failed: {0}")]
    Install(String),
}

impl UpdateError {
    fn code(&self) -> &'static str {
        match self {
            Self::Check(_) => "check_failed",
            Self::NotChecked => "not_checked",
            Self::PreviousInstallerMissing(_) => "previous_installer_missing",
            Self::Prepare(_) => "prepare_failed",
            Self::Install(_) => "install_failed",
        }
    }
}

/// 프론트에는 `{ code, message }` 로 전달한다.
impl Serialize for UpdateError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("UpdateError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

fn check_result(current_version: &str, found: Option<&Update>) -> UpdateCheck {
    UpdateCheck {
        available: found.is_some(),
        current_version: current_version.to_owned(),
        version: found.map(|u| u.version.clone()),
        notes: found.and_then(|u| u.body.clone()),
        date: found.and_then(|u| u.date.map(|d| d.to_string())),
    }
}

/// 설치 전 복구 재료를 갖춘다: 이전 설치기 존재 확인 -> DB 스냅샷 -> `pending` journal.
/// 하나라도 실패하면 아무것도 남기지 않고 오류를 돌려준다(설치를 시작하지 않는다).
pub fn prepare_for_install(
    paths: &Paths,
    from_version: &str,
    to_version: &str,
    clock: &dyn Clock,
) -> Result<(), UpdateError> {
    let installer = paths.current_installer();
    if !installer.is_file() {
        return Err(UpdateError::PreviousInstallerMissing(
            installer.display().to_string(),
        ));
    }
    let prepare = |e: &dyn std::fmt::Display| UpdateError::Prepare(e.to_string());

    let db_backup = if paths.db().is_file() {
        let path = paths.backups().join(format!(
            "mam-pre-update-v{from_version}-{}.sqlite",
            clock.now_ms()
        ));
        mam_db::snapshot(&paths.db(), &path).map_err(|e| prepare(&e))?;
        Some(path)
    } else {
        None
    };
    if let Err(e) = update_guard::prepare_update(paths, from_version, to_version, db_backup, clock)
    {
        let _ = update_guard::abort_update(paths);
        return Err(prepare(&e));
    }
    Ok(())
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<UpdateCheck, UpdateError> {
    let updater = app
        .updater()
        .map_err(|e| UpdateError::Check(e.to_string()))?;
    let found = updater
        .check()
        .await
        .map_err(|e| UpdateError::Check(e.to_string()))?;
    let result = check_result(&app.package_info().version.to_string(), found.as_ref());
    *pending.0.lock().unwrap_or_else(|p| p.into_inner()) = found;
    Ok(result)
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
    paths: State<'_, Paths>,
) -> Result<(), UpdateError> {
    let update = pending
        .0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .ok_or(UpdateError::NotChecked)?;
    prepare_for_install(
        &paths,
        &app.package_info().version.to_string(),
        &update.version,
        &SystemClock,
    )?;

    let emit = |phase, downloaded, total| {
        let _ = app.emit(
            STATE_EVENT,
            UpdateStatePayload {
                phase,
                downloaded,
                total,
            },
        );
    };
    let mut downloaded = 0u64;
    // Windows 에서는 설치기 실행에 성공하면 이 함수 안에서 프로세스가 종료된다 — 돌아왔다면 실패다.
    let result = update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                emit("downloading", downloaded, total);
            },
            || emit("installing", 0, None),
        )
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = update_guard::abort_update(&paths);
            emit("failed", 0, None);
            Err(UpdateError::Install(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    // Categories: Business rule (14 이전 설치기 없으면 거부) / Happy (15 복구 재료 준비) /
    // External (16 DB 스냅샷 실패) / Validation (17 직렬화 계약)
    use std::cell::Cell;

    use super::*;

    struct TestClock(Cell<i64>);

    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.get()
        }
    }

    const T0: i64 = 1_700_000_000_000;

    fn setup() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = Paths::new(dir.path().join("MAMData"));
        paths.ensure_layout().unwrap();
        (dir, paths)
    }

    #[test]
    fn t_upd_14_install_is_refused_without_previous_installer_and_leaves_no_trace() {
        // Arrange
        let (_dir, paths) = setup();

        // Act
        let err =
            prepare_for_install(&paths, "0.1.0", "0.1.1", &TestClock(Cell::new(T0))).unwrap_err();

        // Assert
        assert!(
            matches!(err, UpdateError::PreviousInstallerMissing(_)),
            "{err:?}"
        );
        assert!(!paths.journal().exists());
        assert!(!paths.in_progress_marker().exists());
    }

    #[test]
    fn t_upd_15_prepare_snapshots_db_and_writes_pending_journal() {
        // Arrange: 마이그레이션된 DB 와 이전 설치기
        let (_dir, paths) = setup();
        std::fs::write(paths.current_installer(), b"old").unwrap();
        let clock = TestClock(Cell::new(T0));
        let mut conn = mam_db::open(&paths.db()).unwrap();
        mam_db::migrate(&mut conn, &paths.backups(), &clock).unwrap();
        drop(conn);

        // Act
        prepare_for_install(&paths, "0.1.0", "0.1.1", &clock).unwrap();

        // Assert
        let expected = paths
            .backups()
            .join(format!("mam-pre-update-v0.1.0-{T0}.sqlite"));
        assert!(expected.is_file());
        let journal: update_guard::Journal =
            serde_json::from_slice(&std::fs::read(paths.journal()).unwrap()).unwrap();
        assert_eq!(journal.db_backup, Some(expected));
        assert_eq!(journal.state, update_guard::State::Pending);
        assert!(paths.in_progress_marker().exists());
    }

    #[test]
    fn t_upd_15_prepare_without_a_db_yet_records_no_backup() {
        // Arrange
        let (_dir, paths) = setup();
        std::fs::write(paths.current_installer(), b"old").unwrap();

        // Act
        prepare_for_install(&paths, "0.1.0", "0.1.1", &TestClock(Cell::new(T0))).unwrap();

        // Assert
        let journal: update_guard::Journal =
            serde_json::from_slice(&std::fs::read(paths.journal()).unwrap()).unwrap();
        assert_eq!(journal.db_backup, None);
    }

    #[test]
    fn t_upd_16_corrupt_db_aborts_prepare_without_writing_journal() {
        // Arrange: SQLite 가 아닌 DB 파일 -> 스냅샷 실패
        let (_dir, paths) = setup();
        std::fs::write(paths.current_installer(), b"old").unwrap();
        std::fs::write(paths.db(), vec![0xAB; 4096]).unwrap();

        // Act
        let err =
            prepare_for_install(&paths, "0.1.0", "0.1.1", &TestClock(Cell::new(T0))).unwrap_err();

        // Assert
        assert!(matches!(err, UpdateError::Prepare(_)), "{err:?}");
        assert!(!paths.journal().exists());
    }

    #[test]
    fn t_upd_17_contract_serialization_keys() {
        // Arrange + Act
        let check = serde_json::to_value(check_result("0.1.0", None)).unwrap();
        let err = serde_json::to_value(UpdateError::NotChecked).unwrap();
        let state = serde_json::to_value(UpdateStatePayload {
            phase: "downloading",
            downloaded: 5,
            total: Some(10),
        })
        .unwrap();

        // Assert
        assert_eq!(
            check,
            serde_json::json!({
                "available": false, "currentVersion": "0.1.0",
                "version": null, "notes": null, "date": null
            })
        );
        assert_eq!(err["code"], "not_checked");
        assert!(err["message"].is_string());
        assert_eq!(
            state,
            serde_json::json!({"phase": "downloading", "downloaded": 5, "total": 10})
        );
    }
}
