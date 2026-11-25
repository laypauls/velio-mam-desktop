//! 기동 알림 — 복원·자동 복구가 일어났음을 첫 화면에 한 번 알린다 (`take_startup_notice`).
//!
//! 기동은 DB 를 열기 전에 일어나므로 DB 가 아니라 MAMData 의 `startup-notice.json` 에 둔다. 읽으면 지워진다(1회).
//! 여러 번 쌓이면 마지막 것만 남는다.

use mam_core::storage::DataPaths;
use serde::{Deserialize, Serialize};

const NOTICE_FILE: &str = "startup-notice.json";

/// 프론트 계약 `StartupNotice` (`kind` 로 구분하는 유니온, 필드는 camelCase).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StartupNotice {
    #[serde(rename = "restored", rename_all = "camelCase")]
    Restored {
        backup_file: String,
        created_at: i64,
    },
    #[serde(rename = "recovered", rename_all = "camelCase")]
    Recovered {
        backup_file: Option<String>,
        created_at: Option<i64>,
        /// 보관한 손상 DB 파일의 절대 경로
        corrupt_file: String,
        consent_reset: bool,
    },
}

fn notice_path(paths: &DataPaths) -> std::path::PathBuf {
    paths.root().join(NOTICE_FILE)
}

pub fn write(paths: &DataPaths, notice: &StartupNotice) -> std::io::Result<()> {
    let json = serde_json::to_vec(notice).map_err(std::io::Error::other)?;
    let dest = notice_path(paths);
    let tmp = dest.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &dest)
}

/// 알림을 읽고 지운다. 없거나 읽을 수 없으면(손상 포함) `None` — 손상된 파일은 지워 매번 실패하지 않게 한다.
pub fn take(paths: &DataPaths) -> Option<StartupNotice> {
    let file = notice_path(paths);
    let bytes = std::fs::read(&file).ok()?;
    if let Err(e) = std::fs::remove_file(&file) {
        tracing::warn!(%e, "cannot remove the startup notice file");
    }
    serde_json::from_slice(&bytes).ok()
}
