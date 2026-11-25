//! 관리 폴더 이전 (D-6) — 폴더가 다른 경로·드라이브로 옮겨졌을 때 자산·태그·메모를 유지한 채 새 경로에 다시 연결한다.
//! 파일시스템 검증(원본은 읽기 전용으로만 연다)은 트랜잭션 밖에서, DB 검증과 갱신은 한 쓰기 트랜잭션에서 한다.

use mam_db::Priority;
use serde::Serialize;

use super::Ingest;
use crate::scan::{IngestError, ManagedFolderView, RelocateTarget, relocate_folder_tx, root_keys};

/// `relocate_folder` 결과 (프론트 계약 `RelocateFolderResult`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RelocateOutcome {
    pub folder: ManagedFolderView,
    /// 새 경로로 옮겨 연결한 위치 수
    pub relocated: u64,
}

impl Ingest {
    /// 폴더의 경로를 `new_path` 로 바꾸고 새 경로 스캔을 예약한다. 이전 경로는 읽지 않으므로 옛 볼륨이 오프라인이어도 된다.
    /// 같은 경로는 `SamePath`, 다른 폴더와 겹치면 `AlreadyRegistered`/`NestedFolder`, 그 폴더 스캔이 실행 중이면 `ScanRunning`.
    pub fn relocate_folder(
        &self,
        folder_id: i64,
        new_path: &str,
    ) -> Result<RelocateOutcome, IngestError> {
        let canonical = self.resolve_new_root(new_path)?;
        let info = self.volumes.volume_of(&canonical)?;
        let (rel_root, path_key) = root_keys(&info, &canonical)?;
        let target = RelocateTarget {
            info,
            rel_root,
            path_key,
            display_path: crate::volume::strip_verbatim(&canonical.to_string_lossy()),
        };

        // 감시를 먼저 멈춘다: 옛 경로의 이벤트가 이전 중에 들어와도 apply_chunk 가드가 막지만, 불필요한 일을 줄인다
        self.watchers.stop(folder_id);
        let clock = std::sync::Arc::clone(&self.env.clock);
        let result = self.env.write(Priority::High, move |c| {
            relocate_folder_tx(c, &*clock, folder_id, &target)
        });
        // 성공이든 실패든 현재 DB 의 경로로 감시를 되살린다. 그 사이 점검 스레드가 (이전 전) 옛 경로로 감시를 시작했을 수
        // 있으므로 같은 잠금 안에서 다시 멈추고 시작한다
        {
            let _ctl = super::lock(&self.watch_ctl);
            self.watchers.stop(folder_id);
            super::lock(&self.watch_failed).remove(&folder_id);
            self.start_watch_locked(folder_id);
        }
        let relocated = result?;
        self.jobs.notify();
        Ok(RelocateOutcome {
            folder: self.view_of(folder_id)?,
            relocated,
        })
    }
}
