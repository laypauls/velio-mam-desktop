//! 관리 데이터 초기화 (D5) — 계획 계산, `reset-pending` 표지, 재시작 시 적용.
//!
//! 흐름: 호출자가 `mam_db` 로 `pre_reset` 백업을 만들고 -> [`plan_reset`] (백업 결과가 없으면 계획 자체가 실패) ->
//! [`write_reset_marker`] -> 앱 종료/재시작 -> 시작 초반(DB 를 열기 전) [`apply_pending_reset`] -> DB 마이그레이션.
//! 원본 폴더·백업 폴더·업데이트 안전장치·잠금 파일·모델·로그·MAMData 위치 설정(`location.json`)은 대상이 아니다.

use std::path::{Path, PathBuf};

use mam_db::Clock;
use serde::{Deserialize, Serialize};

use super::ProtectError;
use super::paths::{is_within, same_path};
use crate::storage::DataPaths;

const MARKER_FILE: &str = "reset-pending.json";

/// 호출자가 수행한 자동 백업의 결과 — 실패하면 초기화는 진행되지 않는다
#[derive(Debug, Clone)]
pub enum BackupOutcome {
    Created(PathBuf),
    Failed(String),
}

/// 지울 MAMData 하위 경로 (DB 폴더 · 인덱스 · 캐시) 와 그 근거가 된 백업 파일
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPlan {
    pub delete: Vec<PathBuf>,
    pub backup: PathBuf,
}

/// 초기화 계획. `protected` 는 절대 지우면 안 되는 경로(관리 폴더 루트, 사용자 지정 백업 폴더 등) —
/// **양방향**으로 검사한다: 삭제 대상이 보호 경로를 포함하거나, 보호 경로 안에 있거나(같은 경로 포함),
/// MAMData 루트 자체가 보호 경로 안에 있으면 거부한다 (경로는 정규화해 비교). 관리 폴더가 MAMData 의 부모이거나
/// 같은 경우가 여기에 해당한다. 백업 파일이 없거나 삭제 대상 안에 있어도 거부한다.
/// `apply_pending_reset` 도 삭제 직전에 이 검사를 다시 한다 (표지가 낡았을 수 있다).
pub fn plan_reset(
    paths: &DataPaths,
    backup: &BackupOutcome,
    protected: &[PathBuf],
) -> Result<ResetPlan, ProtectError> {
    let backup = match backup {
        BackupOutcome::Created(p) => p.clone(),
        BackupOutcome::Failed(why) => return Err(ProtectError::BackupFailed(why.clone())),
    };
    if !backup.is_file() {
        return Err(ProtectError::BackupFailed(
            "backup file does not exist".into(),
        ));
    }
    let indexes = paths
        .tantivy()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| ProtectError::Internal("unexpected index layout".into()))?;
    let delete = vec![
        paths.database_dir(),
        indexes,
        paths.cache(),
        paths.plugins(),
    ];
    if protected.iter().any(|p| is_within(paths.root(), p)) {
        return Err(ProtectError::InvalidInput(
            "MAMData is inside a protected path".into(),
        ));
    }
    for target in &delete {
        if !is_within(target, paths.root()) || same_path(target, paths.root()) {
            return Err(ProtectError::Internal(
                "reset target outside MAMData".into(),
            ));
        }
        if is_within(&backup, target) {
            return Err(ProtectError::BackupFailed(
                "backup is inside a folder that would be wiped".into(),
            ));
        }
        if protected
            .iter()
            .any(|p| is_within(p, target) || is_within(target, p))
        {
            return Err(ProtectError::InvalidInput(
                "a protected path overlaps a reset target".into(),
            ));
        }
    }
    Ok(ResetPlan { delete, backup })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetMarker {
    pub requested_at: i64,
    pub backup: PathBuf,
}

pub fn marker_path(paths: &DataPaths) -> PathBuf {
    paths.root().join(MARKER_FILE)
}

/// 표지 기록 (임시 파일 -> rename 으로 원자적)
pub fn write_reset_marker(
    paths: &DataPaths,
    clock: &dyn Clock,
    plan: &ResetPlan,
) -> Result<(), ProtectError> {
    let marker = ResetMarker {
        requested_at: clock.now_ms(),
        backup: plan.backup.clone(),
    };
    let json =
        serde_json::to_vec_pretty(&marker).map_err(|e| ProtectError::Internal(e.to_string()))?;
    let dest = marker_path(paths);
    let tmp = dest.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| ProtectError::io(&tmp, &e))?;
    std::fs::rename(&tmp, &dest).map_err(|e| ProtectError::io(&dest, &e))
}

/// 표지가 없으면 `None`, 손상됐으면 `Internal`
pub fn read_reset_marker(paths: &DataPaths) -> Result<Option<ResetMarker>, ProtectError> {
    let file = marker_path(paths);
    match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| ProtectError::Internal("reset marker is corrupt".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ProtectError::io(&file, &e)),
    }
}

/// 표지를 지운다 (초기화 취소, 손상된 표지 정리)
pub fn clear_reset_marker(paths: &DataPaths) -> Result<(), ProtectError> {
    let file = marker_path(paths);
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ProtectError::io(&file, &e)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetReport {
    pub removed: Vec<PathBuf>,
    pub backup: PathBuf,
}

/// 시작 시 호출: 표지가 있으면 계획을 다시 검증해 지우고 하위 구조를 새로 만든 뒤 표지를 제거한다.
/// 도중에 실패하면 표지가 남아 다음 시작에 다시 시도한다 (삭제는 멱등). DB 마이그레이션은 호출자가 이어서 한다.
pub fn apply_pending_reset(
    paths: &DataPaths,
    protected: &[PathBuf],
) -> Result<Option<ResetReport>, ProtectError> {
    let Some(marker) = read_reset_marker(paths)? else {
        return Ok(None);
    };
    let plan = plan_reset(paths, &BackupOutcome::Created(marker.backup), protected)?;
    for dir in &plan.delete {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ProtectError::io(dir, &e)),
        }
    }
    paths
        .ensure_layout()
        .map_err(|e| ProtectError::Io(e.to_string()))?;
    clear_reset_marker(paths)?;
    Ok(Some(ResetReport {
        removed: plan.delete,
        backup: plan.backup,
    }))
}
