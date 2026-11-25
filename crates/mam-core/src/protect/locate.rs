//! 자산의 현재 원본 위치 조회와 사전 점검 (삭제·이동 공통).

use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};

use super::ProtectError;
use crate::identity::modified_ms;

/// 활성 위치 + 소속 볼륨 + 자산 기록(크기·수정 시각)
#[derive(Debug, Clone)]
pub(super) struct Located {
    pub asset_id: i64,
    pub location_id: i64,
    pub volume_key: String,
    pub rel_path: String,
    pub mount: Option<String>,
    pub online: bool,
    pub size: i64,
    pub modified_at: i64,
}

pub(super) fn locate(conn: &Connection, asset_id: i64) -> Result<Located, ProtectError> {
    conn.query_row(
        "SELECT fl.location_id, v.volume_key, fl.rel_path, v.last_mount_point,
                v.availability = 'online', a.file_size, a.modified_at
         FROM file_locations fl
         JOIN volumes v ON v.volume_id = fl.volume_id
         JOIN media_assets a ON a.asset_id = fl.asset_id
         WHERE fl.asset_id = ?1 AND fl.detached_at IS NULL AND a.status = 'active'",
        [asset_id],
        |r| {
            Ok(Located {
                asset_id,
                location_id: r.get(0)?,
                volume_key: r.get(1)?,
                rel_path: r.get(2)?,
                mount: r.get(3)?,
                online: r.get(4)?,
                size: r.get(5)?,
                modified_at: r.get(6)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| ProtectError::NotFound(format!("asset {asset_id} has no active location")))
}

impl Located {
    /// 오프라인·마운트 미상·파일 없음·마지막 스캔 이후 변경은 `Offline`/`Io` 로 거부한다.
    /// 반환값은 지금 기준 절대 경로.
    pub fn checked_path(&self) -> Result<PathBuf, ProtectError> {
        if !self.online {
            return Err(ProtectError::Offline("offline".into()));
        }
        let mount = self
            .mount
            .as_deref()
            .ok_or_else(|| ProtectError::Offline("volume mount point unknown".into()))?;
        let rel: PathBuf = self.rel_path.split(['\\', '/']).collect();
        // 위치 행이 손상돼도 볼륨 밖을 건드리지 않는다
        if rel.as_os_str().is_empty()
            || rel.components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(ProtectError::Internal(format!(
                "invalid stored path for asset {}",
                self.asset_id
            )));
        }
        let path = Path::new(mount).join(rel);
        let meta = std::fs::metadata(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ProtectError::Offline("file_missing".into()),
            _ => ProtectError::io(&path, &e),
        })?;
        if !meta.is_file() {
            return Err(ProtectError::Offline("file_missing".into()));
        }
        let modified = meta.modified().map_err(|e| ProtectError::io(&path, &e))?;
        if i64::try_from(meta.len()).ok() != Some(self.size)
            || modified_ms(modified) != self.modified_at
        {
            return Err(ProtectError::Io("changed_since_scan".into()));
        }
        Ok(path)
    }
}
