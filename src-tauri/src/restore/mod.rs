//! DB 백업 복원 · 원천 DB 손상 자동 복구 (요청 20261107-123710-backup-restore-db-recovery).
//!
//! 사용자 복원은 초기화(`reset.rs`, P2-9 D5)와 같은 방식이다: 요청 시(`prepare`) 인증 -> 백업 검증 -> `pre_restore` 백업 ->
//! `restore-pending.json` 표지 -> (command 가) 서비스 정지·재시작. 시작 시([`apply_pending`]) **DB 를 열기 전에** 표지를 적용한다.
//!
//! 정책: PR1 복원해도 현재 비밀번호 해시·면제 시간 유지 / 외부 AI 동의는 현재 값과 백업 값 중 철회 쪽 /
//! H1 `sqlite_sequence` 를 복원 전 값 이상으로 올려 asset_id 재사용(옛 썸네일이 새 자산에 붙음)을 막는다 /
//! H3·H4 Tantivy·벡터 인덱스는 전체 재구축을 명시적으로 요청한다([`post::request_derived_rebuild`]).
//!
//! 적용은 멱등이다: 입출력 오류면 표지를 남기고 다음 기동에서 같은 순서로 다시 한다(최대 [`MAX_ATTEMPTS`] 번 — 영구 오류로
//! 앱이 영영 못 뜨는 일을 막는다). 백업이 쓸 수 없는 것으로 판명되면 DB 는 그대로 두고 표지만 버린다.

pub(crate) mod commands;
pub mod hint;
pub mod notice;
#[cfg(test)]
mod plugin_tests;
pub mod plugins;
pub mod post;
mod prepare;
pub mod recover;
#[cfg(test)]
mod recover_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use mam_core::storage::DataPaths;
use mam_db::DbError;
use serde::{Deserialize, Serialize};

use notice::StartupNotice;

pub const MARKER_FILE: &str = "restore-pending.json";
/// 입출력 오류로 적용에 실패해도 표지를 남겨 다시 시도하는 최대 횟수 (이후에는 표지를 버리고 기동을 계속한다)
pub const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentState {
    pub given: bool,
    pub at: Option<i64>,
}

/// 복원 요청 시점의 "현재 값" — 복원된 DB 위에 다시 덮어쓰거나 하한으로 쓴다.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptState {
    /// `settings.password_hash` 행의 원문(JSON 문자열). 없으면 비밀번호 미설정 -> 복원 DB 의 행도 지운다
    pub password_hash: Option<String>,
    /// `settings.auth_exempt_minutes` 행의 원문. 없으면 복원 DB 의 행을 지워 기본값으로 돌린다
    pub auth_exempt_minutes: Option<String>,
    pub consent: ConsentState,
    /// 복원 전 `sqlite_sequence` (테이블 -> 최대 발급 번호) — H1
    pub sequences: BTreeMap<String, i64>,
    /// `settings` 의 `ai_key_<provider>` 행 원문(DPAPI 암호문 그대로 — 평문 키 아님). 복원 후 백업의 키 행을 모두 지우고 이 값으로 덮어쓴다
    #[serde(default)]
    pub ai_keys: BTreeMap<String, String>,
    /// 현재 설치된 플러그인의 설정 (구 복원 표지에는 없다 -> 설치 없음으로 읽는다)
    #[serde(default)]
    pub plugins: KeptPlugins,
}

/// 복원 요청 시점의 플러그인 설정 — 규칙은 [`plugins`] 모듈 문서. 키는 모두 설치된 플러그인 ID.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptPlugins {
    /// `plugin_installed:<id>` 행 원문
    #[serde(default)]
    pub installed: BTreeMap<String, String>,
    /// 현재 활성인 플러그인
    #[serde(default)]
    pub enabled: BTreeSet<String>,
    /// 현재 허용된 권한 문자열
    #[serde(default)]
    pub granted: BTreeMap<String, BTreeSet<String>>,
}

/// 해시·키 암호문이 로그에 새지 않도록 값은 가리고 개수만 보인다.
impl std::fmt::Debug for KeptState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeptState")
            .field("password_set", &self.password_hash.is_some())
            .field("consent", &self.consent)
            .field("sequences", &self.sequences.len())
            .field("ai_keys", &self.ai_keys.len())
            .field("plugins", &self.plugins.installed.len())
            .finish()
    }
}

impl std::fmt::Debug for KeptPlugins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeptPlugins")
            .field("installed", &self.installed.len())
            .field("enabled", &self.enabled.len())
            .field("granted", &self.granted.len())
            .finish()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreMarker {
    pub requested_at: i64,
    /// 복원할 백업 파일의 절대 경로
    pub backup: PathBuf,
    pub backup_file: String,
    pub backup_created_at: i64,
    pub kept: KeptState,
    #[serde(default)]
    pub attempts: u32,
}

pub fn marker_path(paths: &DataPaths) -> PathBuf {
    paths.root().join(MARKER_FILE)
}

/// 복원 표지가 남아 있는가 (재시작 중이거나 이전 적용이 중단됐다)
pub fn is_pending(paths: &DataPaths) -> bool {
    marker_path(paths).is_file()
}

/// 표지를 원자적으로 쓴다 (임시 파일 -> rename)
pub fn write_marker(paths: &DataPaths, marker: &RestoreMarker) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(marker).map_err(std::io::Error::other)?;
    let dest = marker_path(paths);
    let tmp = dest.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &dest)
}

fn remove_marker(paths: &DataPaths) -> std::io::Result<()> {
    match std::fs::remove_file(marker_path(paths)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StartupRestore {
    /// 표지 없음
    None,
    /// 적용했다 (백업 파일 이름)
    Applied(String),
    /// 백업이 쓸 수 없거나 표지가 손상돼 DB 를 건드리지 않고 표지를 버렸다 (또는 재시도 한도 초과)
    Discarded(String),
}

#[derive(Debug, thiserror::Error)]
#[error("restore step failed (will retry on next start): {0}")]
pub struct RetryLater(String);

enum StepError {
    /// 영구 실패 — DB 는 그대로, 표지를 버린다
    Discard(String),
    /// 입출력 등 일시 실패 — 표지를 남겨 재시도
    Retry(String),
}

impl StepError {
    fn from_db(e: DbError) -> Self {
        match e {
            DbError::BackupInvalid { .. } => Self::Discard(e.to_string()),
            other => Self::Retry(other.to_string()),
        }
    }
}

/// 시작 시 호출: journal·초기화 표지 처리 뒤, **DB 를 열기 전에**. `Err` 이면 표지를 남겼으니 호출자는 기동을 멈춘다.
pub fn apply_pending(paths: &DataPaths) -> Result<StartupRestore, RetryLater> {
    let file = marker_path(paths);
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(StartupRestore::None),
        Err(e) => return Err(RetryLater(format!("{}: {e}", file.display()))),
    };
    let Ok(marker) = serde_json::from_slice::<RestoreMarker>(&bytes) else {
        remove_marker(paths).map_err(|e| RetryLater(e.to_string()))?;
        return Ok(StartupRestore::Discarded(
            "restore marker is corrupt".into(),
        ));
    };
    match run(paths, &marker) {
        Ok(()) => {
            remove_marker(paths).map_err(|e| RetryLater(e.to_string()))?;
            Ok(StartupRestore::Applied(marker.backup_file))
        }
        Err(StepError::Discard(why)) => {
            remove_marker(paths).map_err(|e| RetryLater(e.to_string()))?;
            Ok(StartupRestore::Discarded(why))
        }
        Err(StepError::Retry(why)) => {
            let attempts = marker.attempts + 1;
            if attempts >= MAX_ATTEMPTS {
                remove_marker(paths).map_err(|e| RetryLater(e.to_string()))?;
                return Ok(StartupRestore::Discarded(format!(
                    "gave up after {attempts} attempts: {why}"
                )));
            }
            let _ = write_marker(paths, &RestoreMarker { attempts, ..marker });
            Err(RetryLater(why))
        }
    }
}

fn run(paths: &DataPaths, marker: &RestoreMarker) -> Result<(), StepError> {
    // 요청 이후 백업이 바뀌었거나 사라졌을 수 있다 — 교체 전에 다시 검사한다 (거부되면 DB 는 그대로)
    let meta = mam_db::inspect_backup(&marker.backup).map_err(StepError::from_db)?;
    if meta.schema_version > mam_db::latest_version() {
        return Err(StepError::Discard(
            "backup schema is newer than this app".into(),
        ));
    }
    mam_db::restore_backup(&marker.backup, &paths.db()).map_err(StepError::from_db)?;
    post::finish_user_restore(paths, &marker.kept).map_err(StepError::Retry)?;
    let notice = StartupNotice::Restored {
        backup_file: marker.backup_file.clone(),
        created_at: marker.backup_created_at,
    };
    if let Err(e) = notice::write(paths, &notice) {
        tracing::warn!(%e, "cannot record the restore notice");
    }
    Ok(())
}
