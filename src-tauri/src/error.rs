//! command 경계 오류 모델 (P2-1, D8). 프론트에는 `{ code, message }` 만 간다 (`src/lib/ipc.ts` 의 `CommandError`).
//!
//! `code` 는 프론트 계약의 `ErrorCode` 중 하나이고 `message` 는 고정 문구(개발자용)다 —
//! 내부 절대 경로·SQL·키 값은 싣지 않는다. 상세 원인은 `tracing` 로그에만 남긴다.

use mam_core::storage::StorageError;
use mam_db::{DbError, PoolError, SettingsError};
use serde::{Serialize, Serializer};

/// 프론트 계약의 `ErrorCode` 전체. 아직 쓰는 곳이 없는 값은 이후 묶음(P2-2, P2-9)이 쓴다.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    AlreadyRegistered,
    NestedFolder,
    /// 폴더 이전 요청 시 그 폴더의 스캔이 실행 중이다 — 일시정지 후 다시 시도
    ScanRunning,
    Offline,
    /// 같은 범위의 관리 폴더가 제거 중이다 — 삭제가 끝난 뒤 다시 시도 (폴더 추가·경로 변경·재스캔)
    Busy,
    /// 파괴적 작업 직전 DB 백업에 실패했다 — 아무것도 바뀌지 않았다 (폴더 제거·유형 정리)
    BackupFailed,
    /// 디스크에 관리 폴더(제거 진행 중 포함)가 남아 있어 디스크 기록을 지울 수 없다
    VolumeHasFolders,
    AuthRequired,
    PasswordIncorrect,
    /// 비밀번호 연속 5회 실패 후 30초 잠금 (P2-9 D2)
    TooManyAttempts,
    PasswordNotSet,
    PasswordAlreadySet,
    /// 외부 AI 전송 동의 전 요청 (P3)
    ConsentRequired,
    /// 외부 AI API 4xx/5xx/timeout — 메시지는 정제됨 (P3)
    ProviderError,
    /// 진행 중인 외부 AI 요청 때문에 지금은 할 수 없다 (예: 구독 로그아웃)
    AiBusy,
    DiskFull,
    Io,
    Internal,
    /// 시작 직후 검색 색인 재구축 중 — 프론트는 오류 대신 "준비 중"으로 보이고 다시 묻는다 (P4 후속)
    IndexNotReady,
    /// 복원할 백업이 없는/손상/빈 파일이다 (DB 복원)
    BackupInvalid,
    /// 백업의 스키마 버전이 현재 앱보다 높다 (DB 복원)
    BackupTooNew,
    /// 내장 재생 엔진(워커·libmpv)을 쓸 수 없다 - 프론트는 "외부 앱으로 열기" 로 폴백한다 (콘텐츠 보기)
    PlayerUnavailable,
    /// 자산이 오프라인이거나 원본 파일이 없다 (재생 불가)
    AssetOffline,
    /// 재생 엔진이 파일을 열지 못했다 (손상·미지원 형식·출력 창 생성 실패)
    OpenFailed,
    /// 기본 오디오 출력 장치가 없거나 접근에 실패했다 (시스템 음량 조정)
    AudioDeviceUnavailable,
    /// 플레이어 워커가 재시도 후에도 응답하지 않았다 (워커는 종료·재시작된다) - 프론트가 한 번 더 시도할 수 있다
    PlayerTimeout,
    /// 플레이어 워커를 다시 시작하는 중이라 프레임 이동을 할 수 없다 - 잠시 뒤 다시 시도
    PlayerRestarting,
    /// 플레이어 워커가 연속으로 죽어 재시작을 포기했다 - 열기·재시작·닫기만 받는다
    PlayerRestartFailed,
    /// 재생 중 워커가 죽었다 (`player://state` 이벤트의 `errorCode` 로만 쓰인다)
    PlayerWorkerCrashed,
    /// 플러그인 manifest·패키지가 올바르지 않다 (설치 점검)
    PluginInvalid,
    /// 플러그인이 요구하는 앱 버전·API 버전과 맞지 않는다
    PluginIncompatible,
    /// 플러그인이 꺼져 있다 (사용자 또는 반복 결함 자동 비활성)
    PluginDisabled,
    /// 플러그인에 필요한 권한이 허용되지 않았다
    PluginPermissionDenied,
    /// 플러그인 호출이 시간 안에 끝나지 않았다
    PluginTimeout,
    /// 플러그인 실행 중 오류 — 메시지는 정적 i18n 키(`plugin.error.*`)
    PluginError,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CommandError {
    code: ErrorCode,
    message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal() -> Self {
        Self::new(ErrorCode::Internal, "internal error")
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// 원인은 로그에만 남기고 고정 문구로 응답한다.
    pub(crate) fn logged(
        code: ErrorCode,
        message: &'static str,
        cause: &dyn std::fmt::Display,
    ) -> Self {
        tracing::error!(?code, %cause, "command failed");
        Self::new(code, message)
    }
}

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("CommandError", 2)?;
        s.serialize_field("code", &self.code)?;
        s.serialize_field("message", &self.message)?;
        s.end()
    }
}

impl From<SettingsError> for CommandError {
    fn from(e: SettingsError) -> Self {
        match e {
            // 설정 이름은 프론트 계약의 일부라 노출해도 된다
            SettingsError::Unknown(name) => {
                Self::new(ErrorCode::InvalidInput, format!("unknown setting: {name}"))
            }
            SettingsError::Invalid(name) => Self::new(
                ErrorCode::InvalidInput,
                format!("invalid value for setting: {name}"),
            ),
            SettingsError::Db(e) => e.into(),
        }
    }
}

impl From<DbError> for CommandError {
    fn from(e: DbError) -> Self {
        if e.is_disk_full() {
            Self::logged(ErrorCode::DiskFull, "not enough disk space", &e)
        } else if matches!(e, DbError::Io(_)) {
            Self::logged(ErrorCode::Io, "file operation failed", &e)
        } else {
            Self::logged(ErrorCode::Internal, "database operation failed", &e)
        }
    }
}

impl From<PoolError> for CommandError {
    fn from(e: PoolError) -> Self {
        match e {
            PoolError::Db(e) => e.into(),
            other => Self::logged(ErrorCode::Internal, "database is not available", &other),
        }
    }
}

impl From<StorageError> for CommandError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::NetworkLocation(_) => Self::logged(
                ErrorCode::InvalidInput,
                "network locations are not allowed",
                &e,
            ),
            StorageError::NotAbsolute(_) => Self::logged(
                ErrorCode::InvalidInput,
                "location must be an absolute path",
                &e,
            ),
            StorageError::NotWritable(_) => {
                Self::logged(ErrorCode::InvalidInput, "location is not writable", &e)
            }
            StorageError::Io { .. } => Self::logged(ErrorCode::Io, "file operation failed", &e),
            _ => Self::logged(ErrorCode::Internal, "storage operation failed", &e),
        }
    }
}

#[cfg(test)]
mod tests {
    // Categories: Validation (직렬화 계약) / Business rule (내부 경로 비노출)
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn t_ipc_03_serializes_exactly_code_and_message_with_snake_case_codes() {
        // Arrange
        let err = CommandError::new(
            ErrorCode::InvalidInput,
            "invalid value for setting: backup.keep",
        );

        // Act
        let value = serde_json::to_value(&err).unwrap();

        // Assert
        assert_eq!(
            value,
            serde_json::json!({"code": "invalid_input", "message": "invalid value for setting: backup.keep"})
        );
        let all = [
            (ErrorCode::NotFound, "not_found"),
            (ErrorCode::AlreadyRegistered, "already_registered"),
            (ErrorCode::NestedFolder, "nested_folder"),
            (ErrorCode::ScanRunning, "scan_running"),
            (ErrorCode::AuthRequired, "auth_required"),
            (ErrorCode::PasswordIncorrect, "password_incorrect"),
            (ErrorCode::TooManyAttempts, "too_many_attempts"),
            (ErrorCode::PasswordNotSet, "password_not_set"),
            (ErrorCode::PasswordAlreadySet, "password_already_set"),
            (ErrorCode::ConsentRequired, "consent_required"),
            (ErrorCode::ProviderError, "provider_error"),
            (ErrorCode::AiBusy, "ai_busy"),
            (ErrorCode::DiskFull, "disk_full"),
            (ErrorCode::Io, "io"),
            (ErrorCode::Internal, "internal"),
            (ErrorCode::Offline, "offline"),
            (ErrorCode::IndexNotReady, "index_not_ready"),
            (ErrorCode::BackupInvalid, "backup_invalid"),
            (ErrorCode::BackupTooNew, "backup_too_new"),
            (ErrorCode::PlayerUnavailable, "player_unavailable"),
            (ErrorCode::AssetOffline, "asset_offline"),
            (ErrorCode::OpenFailed, "open_failed"),
            (
                ErrorCode::AudioDeviceUnavailable,
                "audio_device_unavailable",
            ),
            (ErrorCode::PlayerTimeout, "player_timeout"),
            (ErrorCode::PlayerRestarting, "player_restarting"),
            (ErrorCode::PlayerRestartFailed, "player_restart_failed"),
            (ErrorCode::PlayerWorkerCrashed, "player_worker_crashed"),
            (ErrorCode::PluginInvalid, "plugin_invalid"),
            (ErrorCode::PluginIncompatible, "plugin_incompatible"),
            (ErrorCode::PluginDisabled, "plugin_disabled"),
            (
                ErrorCode::PluginPermissionDenied,
                "plugin_permission_denied",
            ),
            (ErrorCode::PluginTimeout, "plugin_timeout"),
            (ErrorCode::PluginError, "plugin_error"),
        ];
        for (code, text) in all {
            assert_eq!(serde_json::to_value(code).unwrap(), text);
        }
    }

    #[test]
    fn t_ipc_04_internal_paths_never_reach_the_message() {
        // Arrange
        let secret_path = PathBuf::from(r"C:\Users\someone\AppData\Local\MAMDesktop\MAMData");
        let errors: Vec<CommandError> = vec![
            StorageError::NetworkLocation(secret_path.clone()).into(),
            StorageError::NotAbsolute(secret_path.clone()).into(),
            StorageError::NotWritable(secret_path.clone()).into(),
            StorageError::Io {
                path: secret_path.clone(),
                source: std::io::Error::other("x"),
            }
            .into(),
            DbError::BackupExists(secret_path.clone()).into(),
            DbError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)).into(),
            PoolError::WriterGone.into(),
        ];

        // Act + Assert
        for e in errors {
            assert!(!e.message.contains("Users"), "{:?}: {}", e.code, e.message);
        }
    }

    #[test]
    fn t_ipc_05_maps_errors_to_contract_codes() {
        let disk: CommandError =
            DbError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)).into();
        let io: CommandError = DbError::Io(std::io::Error::other("x")).into();
        let bad: CommandError = SettingsError::Invalid("backup.keep".into()).into();
        let unknown: CommandError = SettingsError::Unknown("zzz".into()).into();
        let down: CommandError = PoolError::WriterGone.into();
        assert_eq!(
            [
                disk.code(),
                io.code(),
                bad.code(),
                unknown.code(),
                down.code()
            ],
            [
                ErrorCode::DiskFull,
                ErrorCode::Io,
                ErrorCode::InvalidInput,
                ErrorCode::InvalidInput,
                ErrorCode::Internal
            ]
        );
    }
}
