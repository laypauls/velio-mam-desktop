//! `open_backup_dir` command: 현재 백업 폴더를 탐색기로 연다. 경로는 프론트에서 받지 않고 현재 설정으로 계산한다.
//! 폴더는 만들지 않으며(없으면 `not_found`), 원본·백업 파일에 쓰지 않는 읽기 전용 동작이다.
//! Rust <-> TS 계약: `src/lib/ipc.ts`.

use std::sync::Arc;

use mam_core::reuse::{Launcher, ShellLauncher};
use mam_core::scan::IngestError;
use tauri::State;

use crate::error::{CommandError, ErrorCode};
use crate::storage_commands::Storage;

impl Storage {
    /// 현재 백업 폴더(기본 위치 포함)가 존재하는 디렉터리일 때만 `launcher.open` 으로 연다.
    /// 실행 실패는 `IngestError::from(io)` -> `CommandError` 로 `reveal_in_explorer` 와 같게 매핑한다.
    pub fn open_backup_dir(&self, launcher: &dyn Launcher) -> Result<(), CommandError> {
        let dir = self.backup_dir(&self.db_settings()?);
        if !dir.is_dir() {
            return Err(CommandError::new(ErrorCode::NotFound, "not found"));
        }
        launcher
            .open(&dir)
            .map_err(|e| CommandError::from(IngestError::from(e)))
    }
}

#[tauri::command]
pub async fn open_backup_dir(storage: State<'_, Arc<Storage>>) -> Result<(), CommandError> {
    let storage = Arc::clone(&storage);
    tauri::async_runtime::spawn_blocking(move || storage.open_backup_dir(&ShellLauncher))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
}

#[cfg(test)]
mod tests {
    // Categories: Happy (01 기본 위치, 02 사용자 지정) / Validation (03 폴더 없음, 04 파일) / External (05 실행 실패)
    use std::sync::Mutex;
    use std::io;
    use std::path::{Path, PathBuf};

    use mam_core::storage::DataPaths;
    use mam_db::{Clock, DbPool, settings};

    use super::*;

    struct FixedClock;

    impl Clock for FixedClock {
        fn now_ms(&self) -> i64 {
            1
        }
    }

    #[derive(Default)]
    struct StubLauncher {
        opened: Mutex<Vec<PathBuf>>,
        fail_with: Mutex<Option<io::ErrorKind>>,
    }

    impl Launcher for StubLauncher {
        fn open(&self, path: &Path) -> io::Result<()> {
            self.opened.lock().unwrap().push(path.to_path_buf());
            match *self.fail_with.lock().unwrap() {
                Some(kind) => Err(io::Error::from(kind)),
                None => Ok(()),
            }
        }

        fn reveal(&self, _: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    fn setup() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = DataPaths::new(dir.path().join("MAMData"));
        paths.ensure_layout().unwrap();
        let mut conn = mam_db::open(&paths.db()).unwrap();
        mam_db::migrate(&mut conn, &paths.backups(), &FixedClock).unwrap();
        drop(conn);
        let storage = Storage {
            pool: Arc::new(DbPool::open(&paths.db(), 2).unwrap()),
            paths,
            bootstrap_dir: dir.path().join("boot"),
            _lock: None,
        };
        (dir, storage)
    }

    fn set_backup_dir(s: &Storage, dir: &Path) {
        let patch = serde_json::json!({"backup": {"dir": dir.to_str().unwrap()}});
        let patch = patch.as_object().unwrap().clone();
        s.pool
            .write(mam_db::Priority::High, move |c| {
                Ok(settings::update(c, &patch))
            })
            .unwrap()
            .unwrap();
    }

    #[test]
    fn t_ipc_backupdir_01_default_location_is_opened_once() {
        let (_dir, s) = setup();
        let launcher = StubLauncher::default();
        let expected = s.paths.backups();
        assert!(expected.is_dir());

        s.open_backup_dir(&launcher).unwrap();

        assert_eq!(*launcher.opened.lock().unwrap(), vec![expected]);
    }

    #[test]
    fn t_ipc_backupdir_02_user_set_dir_is_opened() {
        let (dir, s) = setup();
        let custom = dir.path().join("MyBackups");
        std::fs::create_dir(&custom).unwrap();
        set_backup_dir(&s, &custom);
        let launcher = StubLauncher::default();

        s.open_backup_dir(&launcher).unwrap();

        assert_eq!(*launcher.opened.lock().unwrap(), vec![custom]);
    }

    #[test]
    fn t_ipc_backupdir_03_missing_dir_is_not_found_and_not_created() {
        let (dir, s) = setup();
        let missing = dir.path().join("gone");
        set_backup_dir(&s, &missing);
        let launcher = StubLauncher::default();

        let err = s.open_backup_dir(&launcher).unwrap_err();

        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(launcher.opened.lock().unwrap().is_empty());
        assert!(!missing.exists(), "must not create the folder");
    }

    #[test]
    fn t_ipc_backupdir_04_file_instead_of_dir_is_not_found() {
        let (dir, s) = setup();
        let file = dir.path().join("backups.txt");
        std::fs::write(&file, b"x").unwrap();
        set_backup_dir(&s, &file);
        let launcher = StubLauncher::default();

        let err = s.open_backup_dir(&launcher).unwrap_err();

        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(launcher.opened.lock().unwrap().is_empty());
    }

    #[test]
    fn t_ipc_backupdir_05_launcher_failure_maps_to_io() {
        let (_dir, s) = setup();
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            let launcher = StubLauncher::default();
            *launcher.fail_with.lock().unwrap() = Some(kind);

            let err = s.open_backup_dir(&launcher).unwrap_err();

            assert_eq!(err.code(), ErrorCode::Io, "{kind:?}");
            assert_eq!(launcher.opened.lock().unwrap().len(), 1);
        }
    }
}
