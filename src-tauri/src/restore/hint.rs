//! 백업 위치 힌트 (H2) — 사용자 지정 백업 폴더는 DB `settings` 에 있어 DB 가 손상되면 알 수 없다.
//! 백업 폴더 설정이 바뀔 때와 백업을 만들 때 MAMData 루트의 `backup-location.json` 에 유효 백업 폴더를 남기고,
//! 자동 복구는 기본 `backups/` 와 이 위치를 모두 찾는다. 힌트는 후보 위치일 뿐 — 파일은 항상 검증 후 사용한다.

use std::path::{Path, PathBuf};

use mam_core::storage::DataPaths;
use serde::{Deserialize, Serialize};

const HINT_FILE: &str = "backup-location.json";

#[derive(Serialize, Deserialize)]
struct Hint {
    dir: PathBuf,
}

fn hint_path(paths: &DataPaths) -> PathBuf {
    paths.root().join(HINT_FILE)
}

/// 힌트를 최신 백업 폴더로 갱신한다 (같으면 쓰지 않는다). 실패는 무시한다 — 백업 자체를 막지 않는다.
pub fn record(paths: &DataPaths, backup_dir: &Path) {
    if read(paths).as_deref() == Some(backup_dir) {
        return;
    }
    let Ok(json) = serde_json::to_vec(&Hint {
        dir: backup_dir.to_path_buf(),
    }) else {
        return;
    };
    let dest = hint_path(paths);
    let tmp = dest.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, &dest)) {
        tracing::warn!(%e, "cannot record the backup location hint");
    }
}

/// 힌트가 가리키는 폴더 (없거나 읽을 수 없으면 `None`)
pub fn read(paths: &DataPaths) -> Option<PathBuf> {
    let bytes = std::fs::read(hint_path(paths)).ok()?;
    serde_json::from_slice::<Hint>(&bytes).ok().map(|h| h.dir)
}
