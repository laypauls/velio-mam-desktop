//! 백업 폴더 관리 — 생성(SQLite 백업 API), 목록, 보관 개수 정리 (P2-1, D5).
//!
//! 파일 이름 규칙: 수동 `mam-<ms>.sqlite`, 자동 `mam-auto-<ms>.sqlite`, 마이그레이션 직전
//! `mam-pre-migrate-v<N>-<ms>.sqlite`, 업데이트 직전 `mam-pre-update-v<버전>-<ms>.sqlite`(P1-7, 종류는
//! `pre_migrate` 로 취급), 초기화 직전 `mam-pre-reset-<ms>.sqlite`(P2-9), 복원 직전 `mam-pre-restore-<ms>.sqlite`. 생성 중에는 `.partial` 로 쓰고
//! 끝나면 이름을 바꾸므로 목록에 부분 파일이 보이지 않는다. 쓰기 실패(디스크 부족 등)면 부분 파일을 지우고
//! 오류를 돌려준다.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::backup::snapshot;
use crate::error::DbError;
use crate::migrate::Clock;

/// 안전 백업(마이그레이션·업데이트·초기화 직전)은 보관 설정과 별개로 최근 이 개수만 남긴다.
pub const SAFETY_BACKUPS_KEPT: usize = 3;

/// 프론트 계약의 `BackupKind` (`auto` | `manual` | `pre_migrate` | `pre_reset` | `pre_restore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    Auto,
    Manual,
    PreMigrate,
    PreReset,
    PreRestore,
    /// 관리 폴더 제거·스캔 유형 정리·기존 보관 데이터 정리 직전 (연결 데이터 영구 삭제)
    PreRemove,
}

impl BackupKind {
    fn is_safety(self) -> bool {
        matches!(
            self,
            Self::PreMigrate | Self::PreReset | Self::PreRestore | Self::PreRemove
        )
    }
}

/// 프론트에는 파일 이름만 보인다(절대 경로 비노출). `created_at` 은 UTC Unix ms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub file_name: String,
    pub created_at: i64,
    pub size_bytes: u64,
    pub kind: BackupKind,
}

fn classify(name: &str) -> Option<(BackupKind, i64)> {
    let stem = name.strip_prefix("mam-")?.strip_suffix(".sqlite")?;
    let digits = stem.rsplit('-').next()?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let kind = if stem.starts_with("pre-migrate-") || stem.starts_with("pre-update-") {
        BackupKind::PreMigrate
    } else if stem.starts_with("pre-reset-") {
        BackupKind::PreReset
    } else if stem.starts_with("pre-restore-") {
        BackupKind::PreRestore
    } else if stem.starts_with("pre-remove-") {
        BackupKind::PreRemove
    } else if stem.starts_with("auto-") {
        BackupKind::Auto
    } else if digits == stem {
        BackupKind::Manual
    } else {
        return None; // `mam-notes-123.sqlite` 같은 사용자 파일은 건드리지 않는다
    };
    Some((kind, digits.parse().ok()?))
}

fn file_name_for(kind: BackupKind, now_ms: i64) -> Result<String, DbError> {
    match kind {
        BackupKind::Manual => Ok(format!("mam-{now_ms}.sqlite")),
        BackupKind::Auto => Ok(format!("mam-auto-{now_ms}.sqlite")),
        BackupKind::PreReset => Ok(format!("mam-pre-reset-{now_ms}.sqlite")),
        BackupKind::PreRestore => Ok(format!("mam-pre-restore-{now_ms}.sqlite")),
        BackupKind::PreRemove => Ok(format!("mam-pre-remove-{now_ms}.sqlite")),
        BackupKind::PreMigrate => Err(DbError::Pragma(
            "pre_migrate backups are created by the migration runner".into(),
        )),
    }
}

/// `db_path` 의 일관된 스냅샷을 `dir` 에 만들고 수동·자동 백업이 `keep` 개를 넘으면 오래된 것을 지운다.
/// `protect` 의 파일(진행 중 업데이트의 복구용 백업 등)은 정리에서 제외한다.
pub fn create_backup(
    db_path: &Path,
    dir: &Path,
    kind: BackupKind,
    keep: usize,
    protect: &[PathBuf],
    clock: &dyn Clock,
) -> Result<BackupInfo, DbError> {
    create_backup_with(db_path, dir, kind, keep, protect, clock, &snapshot)
}

/// [`create_backup`] 의 복사 단계를 주입할 수 있는 형태 (시험에서 쓰기 실패를 만든다).
pub fn create_backup_with(
    db_path: &Path,
    dir: &Path,
    kind: BackupKind,
    keep: usize,
    protect: &[PathBuf],
    clock: &dyn Clock,
    copy: &dyn Fn(&Path, &Path) -> Result<(), DbError>,
) -> Result<BackupInfo, DbError> {
    std::fs::create_dir_all(dir)?;
    remove_partials(dir);
    let created_at = clock.now_ms();
    let file_name = file_name_for(kind, created_at)?;
    let final_path = dir.join(&file_name);
    if final_path.exists() {
        return Err(DbError::BackupExists(final_path));
    }
    let partial = dir.join(format!("{file_name}.partial"));
    if let Err(e) = copy(db_path, &partial) {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&partial, &final_path) {
        let _ = std::fs::remove_file(&partial);
        return Err(e.into());
    }
    let size_bytes = std::fs::metadata(&final_path)?.len();
    // 정리 실패는 방금 만든 백업을 무효로 하지 않는다
    let _ = prune_backups(dir, keep, protect);
    Ok(BackupInfo {
        file_name,
        created_at,
        size_bytes,
        kind,
    })
}

fn remove_partials(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("mam-") && name.ends_with(".sqlite.partial") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// 백업 목록(최신 순). 폴더가 없으면 빈 목록. 규칙에 맞지 않는 파일은 무시한다.
pub fn list_backups(dir: &Path) -> Result<Vec<BackupInfo>, DbError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some((kind, created_at)) = classify(&file_name) else {
            continue;
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        out.push(BackupInfo {
            file_name,
            created_at,
            size_bytes: entry.metadata()?.len(),
            kind,
        });
    }
    out.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.file_name.cmp(&a.file_name))
    });
    Ok(out)
}

/// 수동·자동 백업은 최근 `keep` 개, 안전 백업(마이그레이션·업데이트·초기화 직전)은 최근
/// [`SAFETY_BACKUPS_KEPT`] 개만 남긴다. 지운 파일 이름을 돌려준다. `protect` 는 지우지 않는다.
pub fn prune_backups(dir: &Path, keep: usize, protect: &[PathBuf]) -> Result<Vec<String>, DbError> {
    let mut removed = Vec::new();
    let (mut regular, mut safety) = (0usize, 0usize);
    for b in list_backups(dir)? {
        let (count, limit) = if b.kind.is_safety() {
            (&mut safety, SAFETY_BACKUPS_KEPT)
        } else {
            (&mut regular, keep)
        };
        *count += 1;
        let protected = protect
            .iter()
            .any(|p| p.file_name().is_some_and(|n| n == b.file_name.as_str()));
        if *count <= limit || protected {
            continue;
        }
        std::fs::remove_file(dir.join(&b.file_name))?;
        removed.push(b.file_name);
    }
    Ok(removed)
}
