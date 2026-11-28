//! 저장소 command (P2-1, D8): 설정·백업·경로. 무거운 일은 `spawn_blocking` 으로 넘기고
//! 로직은 `Storage` 메서드에 둔다(시험 가능). Rust <-> TS 계약: `src/lib/ipc.ts`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mam_core::storage::{self, DataPaths, DriveProbe, InstanceLock, SystemDriveProbe};
use mam_db::{BackupInfo, BackupKind, Clock, DbPool, Priority, Settings, SystemClock, settings};
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::State;

use crate::error::{CommandError, ErrorCode};
use crate::protection::Protection;
use crate::update_guard;

/// 앱 전역 저장소 상태. 단일 인스턴스 잠금(`_lock`)을 앱 수명 동안 쥔다.
pub struct Storage {
    pub paths: DataPaths,
    pub pool: Arc<DbPool>,
    pub bootstrap_dir: PathBuf,
    pub _lock: Option<InstanceLock>,
}

/// `app_paths` 반환 — 프론트에 절대 경로를 의도적으로 노출하는 유일한 command.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub data_dir: String,
    pub database_path: String,
    pub backups_dir: String,
    pub logs_dir: String,
}

/// `get_settings` 반환 — DB 설정 + 부트스트랩 파일의 `dataDir`(다음 실행부터 적용될 MAMData 위치, PL11).
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub data_dir: String,
    #[serde(flatten)]
    pub settings: Settings,
}

/// 패치의 `backup.dir`: 없으면 `None`, `null` 이면 `Some(None)`(기본 위치로), 문자열이면 `Some(Some(경로))`.
fn backup_dir_in_patch(patch: &Map<String, Value>) -> Option<Option<PathBuf>> {
    match patch.get("backup")?.as_object()?.get("dir")? {
        Value::String(s) if !s.is_empty() => Some(Some(PathBuf::from(s))),
        _ => Some(None),
    }
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl Storage {
    pub(crate) fn db_settings(&self) -> Result<Settings, CommandError> {
        let (loaded, warnings) = self.pool.read(settings::load)?;
        for w in warnings {
            tracing::warn!("{w}");
        }
        Ok(loaded)
    }

    fn view(&self, settings: Settings) -> SettingsView {
        let data_dir = storage::pending_location(&self.bootstrap_dir)
            .unwrap_or_else(|| self.paths.root().to_path_buf());
        SettingsView {
            data_dir: text(&data_dir),
            settings,
        }
    }

    pub fn get_settings(&self) -> Result<SettingsView, CommandError> {
        Ok(self.view(self.db_settings()?))
    }

    /// `patch` = 계약의 `SettingsPatch`. `dataDir` 는 검증 후 부트스트랩 파일에, 나머지는 DB 에 한 트랜잭션으로 저장한다.
    /// `dataDir` 로 고른 폴더 아래에는 항상 `MAMData` 를 붙이고(D1), MAMData·`backup.dir` 가 관리 폴더(`managed`)와 겹치면
    /// `nested_folder` 로 거부한다(D2) — 원본 트리 안에 `cache` 등이 생겨 초기화가 원본을 지우는 사고를 막는다.
    /// 어느 하나라도 검증에 실패하면 아무것도 바뀌지 않는다(위치 기록의 입출력 실패만 DB 갱신 뒤에 일어날 수 있다).
    pub fn update_settings(
        &self,
        mut patch: Map<String, Value>,
        probe: &dyn DriveProbe,
        managed: &[PathBuf],
    ) -> Result<SettingsView, CommandError> {
        let backup_patch = backup_dir_in_patch(&patch);
        let new_dir = match patch.remove("dataDir") {
            None => None,
            Some(Value::String(s)) if !s.is_empty() => {
                let chosen = PathBuf::from(s);
                storage::validate_location(&chosen, probe)?;
                Some(storage::resolve_data_root(&chosen))
            }
            Some(_) => {
                return Err(CommandError::new(
                    ErrorCode::InvalidInput,
                    "invalid value for setting: dataDir",
                ));
            }
        };
        self.check_overlaps(new_dir.as_deref(), backup_patch.as_ref(), managed)?;
        let outcome = self
            .pool
            .write(Priority::High, move |c| Ok(settings::update(c, &patch)))?;
        let updated = outcome?;
        if backup_patch.is_some() {
            // H2: DB 가 손상되면 사용자 지정 백업 폴더를 알 수 없으므로 MAMData 에 위치 힌트를 남긴다
            crate::restore::hint::record(&self.paths, &self.backup_dir(&updated));
        }
        if let Some(dir) = new_dir {
            storage::write_location(&self.bootstrap_dir, &dir, probe)?;
        }
        Ok(self.view(updated))
    }

    /// D2: 새 MAMData 루트·사용자 지정 백업 폴더가 관리 폴더나 MAMData 내부(`database`·`cache`·`indexes`)와 겹치는지 본다.
    fn check_overlaps(
        &self,
        new_root: Option<&Path>,
        backup_patch: Option<&Option<PathBuf>>,
        managed: &[PathBuf],
    ) -> Result<(), CommandError> {
        let nested = |m: &'static str| CommandError::new(ErrorCode::NestedFolder, m);
        if let Some(root) = new_root
            && managed.iter().any(|f| storage::paths_overlap(root, f))
        {
            return Err(nested("data directory overlaps a managed folder"));
        }
        // 이번에 바뀌는 백업 폴더, 아니면 (데이터 위치만 바뀌는 경우) 현재 설정된 백업 폴더
        let backup = match backup_patch {
            Some(b) => b.clone(),
            None if new_root.is_some() => self.db_settings()?.backup.dir.map(PathBuf::from),
            None => None,
        };
        let Some(backup) = backup else {
            return Ok(());
        };
        if managed.iter().any(|f| storage::paths_overlap(&backup, f)) {
            return Err(nested("backup directory overlaps a managed folder"));
        }
        let pending = storage::pending_location(&self.bootstrap_dir);
        let roots = [Some(self.paths.root()), pending.as_deref(), new_root];
        for root in roots.into_iter().flatten() {
            let internals = ["database", "cache", "indexes"].map(|n| root.join(n));
            if internals.iter().any(|d| storage::paths_overlap(&backup, d)) {
                return Err(nested("backup directory overlaps the MAM data directory"));
            }
        }
        Ok(())
    }

    pub(crate) fn backup_dir(&self, s: &Settings) -> PathBuf {
        s.backup
            .dir
            .as_deref()
            .map_or_else(|| self.paths.backups(), PathBuf::from)
    }

    pub fn backup_now(&self, clock: &dyn Clock) -> Result<BackupInfo, CommandError> {
        let s = self.db_settings()?;
        let protect = update_guard::protected_backups(&self.paths, clock);
        let info = mam_db::create_backup(
            &self.paths.db(),
            &self.backup_dir(&s),
            BackupKind::Manual,
            s.backup.keep as usize,
            &protect,
            clock,
        )?;
        crate::restore::hint::record(&self.paths, &self.backup_dir(&s));
        tracing::info!(backup = %info.file_name, bytes = info.size_bytes, "backup created");
        Ok(info)
    }

    pub fn list_backups(&self) -> Result<Vec<BackupInfo>, CommandError> {
        let s = self.db_settings()?;
        Ok(mam_db::list_backups(&self.backup_dir(&s))?)
    }

    pub fn app_paths(&self) -> Result<AppPaths, CommandError> {
        let s = self.db_settings()?;
        Ok(AppPaths {
            data_dir: text(self.paths.root()),
            database_path: text(&self.paths.db()),
            backups_dir: text(&self.backup_dir(&s)),
            logs_dir: text(&self.paths.logs()),
        })
    }
}

/// 설정 변경의 앱 계층 검사: 관리 폴더 목록을 겹침 검사에 넘기고(D2), 비밀번호가 있을 때 인증 면제 시간을 늘리는
/// 변경은 인증 면제 안에서만 허용한다(D7, `auth_required`). 면제 0분이면 `verify_password` 의 1회 허용을 소모한다.
impl Protection {
    pub fn update_settings(
        &self,
        patch: Map<String, Value>,
        probe: &dyn DriveProbe,
    ) -> Result<SettingsView, CommandError> {
        if let Some(next) = patch.get("authExemptMinutes").and_then(Value::as_u64) {
            let current = u64::from(self.storage.db_settings()?.auth_exempt_minutes);
            if next > current && self.stored_hash()?.is_some() {
                self.guard.require_auth(true)?;
            }
        }
        let managed: Vec<PathBuf> = self
            .ingest
            .list_folders()?
            .into_iter()
            .map(|f| PathBuf::from(f.path))
            .collect();
        let touches_scan_cap = patch.contains_key("maxConcurrentScans");
        let view = self.storage.update_settings(patch, probe, &managed)?;
        if touches_scan_cap {
            // 점검 주기(30초)를 기다리지 않고 바로 반영한다 (이미 도는 스캔은 끊지 않는다)
            self.ingest
                .set_scan_concurrency(view.settings.max_concurrent_scans as usize);
        }
        Ok(view)
    }
}

type Shared<'a> = State<'a, Arc<Storage>>;

async fn blocking<T, F>(storage: &Shared<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Storage) -> Result<T, CommandError> + Send + 'static,
{
    let storage = Arc::clone(storage);
    tauri::async_runtime::spawn_blocking(move || f(&storage))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
}

#[tauri::command]
pub async fn get_settings(storage: Shared<'_>) -> Result<SettingsView, CommandError> {
    blocking(&storage, |s| s.get_settings()).await
}

#[tauri::command]
pub async fn update_settings(
    protection: State<'_, Arc<Protection>>,
    patch: Map<String, Value>,
) -> Result<SettingsView, CommandError> {
    let protection = Arc::clone(&protection);
    tauri::async_runtime::spawn_blocking(move || {
        protection.update_settings(patch, &SystemDriveProbe)
    })
    .await
    .map_err(|e| {
        tracing::error!(%e, "blocking task failed");
        CommandError::internal()
    })?
}

#[tauri::command]
pub async fn backup_now(storage: Shared<'_>) -> Result<BackupInfo, CommandError> {
    blocking(&storage, |s| s.backup_now(&SystemClock)).await
}

#[tauri::command]
pub async fn list_backups(storage: Shared<'_>) -> Result<Vec<BackupInfo>, CommandError> {
    blocking(&storage, |s| s.list_backups()).await
}

#[tauri::command]
pub async fn app_paths(storage: Shared<'_>) -> Result<AppPaths, CommandError> {
    blocking(&storage, |s| s.app_paths()).await
}

#[cfg(test)]
mod tests {
    // Categories: Happy (06 설정·백업 왕복, 09 경로) / Validation (07 잘못된 패치) / Business rule (08 위치 변경 NAS 거부,
    // 보관 개수) / External (10 풀 종료 후 오류)
    use std::cell::Cell;

    use super::*;

    struct TestClock(Cell<i64>);

    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            let v = self.0.get();
            self.0.set(v + 1);
            v
        }
    }

    struct Remote;

    impl DriveProbe for Remote {
        fn is_remote_drive(&self, _: &str) -> bool {
            true
        }
    }

    fn setup() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = DataPaths::new(dir.path().join("MAMData"));
        paths.ensure_layout().unwrap();
        let mut conn = mam_db::open(&paths.db()).unwrap();
        mam_db::migrate(&mut conn, &paths.backups(), &TestClock(Cell::new(1))).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&paths.db(), 2).unwrap());
        let storage = Storage {
            paths,
            pool,
            bootstrap_dir: dir.path().join("boot"),
            _lock: None,
        };
        (dir, storage)
    }

    fn patch(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn update(s: &Storage, v: Value) -> Result<SettingsView, CommandError> {
        s.update_settings(patch(v), &SystemDriveProbe, &[])
    }

    #[test]
    fn t_ipc_06_settings_roundtrip_and_backup_respects_keep() {
        // Arrange
        let (_dir, s) = setup();
        let clock = TestClock(Cell::new(1_700_000_000_000));

        // Act
        let updated = update(&s, serde_json::json!({"backup": {"keep": 2}})).unwrap();
        for _ in 0..4 {
            s.backup_now(&clock).unwrap();
        }
        let listed = s.list_backups().unwrap();

        // Assert
        assert_eq!(updated.settings.backup.keep, 2);
        assert_eq!(s.get_settings().unwrap().settings.backup.keep, 2);
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|b| b.kind == BackupKind::Manual));
        assert!(listed[0].created_at > listed[1].created_at, "newest first");
        let json = serde_json::to_string(&listed).unwrap();
        assert!(
            !json.contains("MAMData") && !json.contains('\\'),
            "names only: {json}"
        );
    }

    #[test]
    fn t_ipc_06b_settings_view_matches_the_frontend_contract_shape() {
        let (_dir, s) = setup();
        let value = serde_json::to_value(s.get_settings().unwrap()).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "authExemptMinutes",
                "backup",
                "capturedAtFallback",
                "closeToTray",
                "closeViewerShortcut",
                "collectionPlayRangesOnly",
                "dataDir",
                "doubleClickAction",
                "firstRunCompleted",
                "galleryCardDisplay",
                "galleryPageSize",
                "galleryPaging",
                "generatedSubtitleVisible",
                "hideShortcut",
                "indexRail",
                "language",
                "layout",
                "maxConcurrentScans",
                "mediaOpenAutoplay",
                "originalActionsEnabled",
                "playRangeKeys",
                "playRangesOnly",
                "playerPlayingStepSec",
                "playerVolume",
                "rescanIntervalHours",
                "resumeEnabled",
                "resumeMinDurationSec",
                "sceneCutsVisible",
                "sceneDetectAuto",
                "sceneDetectSensitivity",
                "sidebarRatingMenu",
                "similarMaxResults",
                "similarMinSimilarity",
                "slideshowImageSeconds",
                "sortFieldsEnabled",
                "subtitleDefaultLanguage",
                "subtitleVisible",
                "theme",
                "titleEditEnabled",
                "videoPlayerBuiltIn",
                "viewerPlacement"
            ]
        );
        assert_eq!(value["dataDir"], text(s.paths.root()));
    }

    #[test]
    fn t_ipc_07_invalid_patch_is_rejected_with_invalid_input_and_changes_nothing() {
        let (_dir, s) = setup();
        for body in [
            serde_json::json!({"backup": {"keep": 0}}),
            serde_json::json!({"zzz": 1}),
            serde_json::json!({"authExemptMinutes": 61}),
            serde_json::json!({"dataDir": 5}),
            serde_json::json!({"dataDir": ""}),
            serde_json::json!({"dataDir": "relative"}),
        ] {
            let err = update(&s, body.clone()).unwrap_err();
            assert_eq!(err.code(), ErrorCode::InvalidInput, "{body}");
        }
        assert_eq!(s.get_settings().unwrap().settings, Settings::default());
    }

    #[test]
    fn t_ipc_08_data_dir_rejects_network_and_a_rejected_patch_changes_nothing() {
        // Arrange
        let (dir, s) = setup();
        let target = dir.path().join("elsewhere");
        let target_text = target.to_str().unwrap();

        // Act
        let nas = update(
            &s,
            serde_json::json!({"dataDir": r"\\nas\share\MAMData", "backup": {"keep": 3}}),
        );
        let mapped = s.update_settings(
            patch(serde_json::json!({"dataDir": target_text})),
            &Remote,
            &[],
        );
        let ok = update(&s, serde_json::json!({"dataDir": target_text})).unwrap();

        // Assert
        assert_eq!(nas.unwrap_err().code(), ErrorCode::InvalidInput);
        assert_eq!(mapped.unwrap_err().code(), ErrorCode::InvalidInput);
        assert_eq!(
            ok.data_dir,
            text(&target.join("MAMData")),
            "pending location is the final path (fixed MAMData subfolder)"
        );
        assert_eq!(s.get_settings().unwrap().settings.backup.keep, 7);
        assert_ne!(
            text(s.paths.root()),
            target_text,
            "current location is unchanged until restart"
        );
        assert_eq!(s.app_paths().unwrap().data_dir, text(s.paths.root()));
    }

    #[test]
    fn t_ipc_09_app_paths_reports_effective_locations() {
        let (_dir, s) = setup();
        let p = s.app_paths().unwrap();
        assert!(
            p.database_path.ends_with(r"database\mam.sqlite"),
            "{}",
            p.database_path
        );
        assert!(p.backups_dir.ends_with("backups"));
        assert!(p.logs_dir.ends_with("logs"));
        update(&s, serde_json::json!({"backup": {"dir": r"D:\MyBackups"}})).unwrap();
        assert_eq!(s.app_paths().unwrap().backups_dir, r"D:\MyBackups");
    }

    #[test]
    fn t_ipc_10_commands_fail_cleanly_after_writer_shutdown() {
        let (_dir, s) = setup();
        s.pool.shutdown();
        let err = update(&s, serde_json::json!({"backup": {"keep": 3}})).unwrap_err();
        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(s.get_settings().is_ok(), "reads still work");
    }

    #[test]
    fn t_ipc_11_backup_write_failure_reports_disk_full() {
        // Arrange: 백업 폴더 지정이 파일이라 쓰기 불가 -> io, 디스크 부족은 DbError 매핑으로 확인(T-ipc-05)
        let (dir, s) = setup();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        update(
            &s,
            serde_json::json!({"backup": {"dir": blocker.to_str().unwrap()}}),
        )
        .unwrap();

        // Act
        let err = s.backup_now(&TestClock(Cell::new(1))).unwrap_err();

        // Assert
        assert_eq!(err.code(), ErrorCode::Io);
    }
}
