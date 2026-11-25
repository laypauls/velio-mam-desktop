//! 감지기가 변경을 보고하는 통로 — 어댑터는 "무엇이 바뀌었나"만 보내고, 디바운스·반영은 공통 계층이 한다.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use super::health::HealthBoard;
use super::release::{ReleaseToken, ReleaseTracker, SinkId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchMsg {
    Path {
        folder_id: i64,
        path: PathBuf,
    },
    /// 하위 폴더 단위 보고 (새 폴더·이름 바뀐 폴더·바뀐 디렉터리) — 반영은 `apply_path` 의 폴더 순회와 같다.
    /// `dir` 아래 파일은 바로 반영하지 않고 변경 대기열의 안정화 게이트를 거친다(복사 중인 파일 보호).
    Subtree {
        folder_id: i64,
        dir: PathBuf,
    },
    /// 이벤트 버퍼 넘침·감시 오류 — 놓친 변경이 있을 수 있으니 폴더를 다시 스캔해야 한다
    Rescan {
        folder_id: i64,
    },
}

impl WatchMsg {
    pub fn folder_id(&self) -> i64 {
        match self {
            Self::Path { folder_id, .. }
            | Self::Subtree { folder_id, .. }
            | Self::Rescan { folder_id } => *folder_id,
        }
    }
}

#[derive(Clone)]
pub struct ChangeSink {
    folder_id: i64,
    tx: Sender<WatchMsg>,
    sink: SinkId,
    tracker: Arc<ReleaseTracker>,
    health: Arc<HealthBoard>,
}

impl ChangeSink {
    pub(crate) fn new(
        folder_id: i64,
        tx: Sender<WatchMsg>,
        sink: SinkId,
        tracker: Arc<ReleaseTracker>,
        health: Arc<HealthBoard>,
    ) -> Self {
        Self {
            folder_id,
            tx,
            sink,
            tracker,
            health,
        }
    }

    pub fn folder_id(&self) -> i64 {
        self.folder_id
    }

    /// 메시지를 보낸다. 종료 중이라 수신자가 없으면 무시한다.
    pub fn emit(&self, msg: WatchMsg) {
        debug_assert_eq!(msg.folder_id(), self.folder_id, "message for a different folder");
        let _ = self.tx.send(msg);
    }

    pub fn path(&self, path: PathBuf) {
        self.emit(WatchMsg::Path {
            folder_id: self.folder_id,
            path,
        });
    }

    /// 하위 폴더 하나를 통째로 다시 보라고 보고한다 (폴더 생성·이름 변경 등).
    pub fn subtree(&self, dir: PathBuf) {
        self.emit(WatchMsg::Subtree {
            folder_id: self.folder_id,
            dir,
        });
    }

    /// 놓쳤을 수 있다 — 폴더 재스캔을 요청한다 (감지기 불변 조건 1).
    pub fn rescan(&self) {
        self.emit(WatchMsg::Rescan {
            folder_id: self.folder_id,
        });
    }

    /// 변경 신호를 받았다 — 마지막 신호 시각을 갱신한다 (상태가 `Degraded` 여도 그 기한은 건드리지 않는다).
    pub fn signal(&self) {
        self.health.signal(self.folder_id, self.sink);
    }

    /// 신호원에 문제가 있다 (재연결 중·응답 느림) — `recovered` 가 불릴 때까지 `Degraded`.
    pub fn degraded(&self, reason_key: &'static str) {
        self.health.degrade(self.folder_id, self.sink, reason_key, None);
    }

    /// 짧은 문제 (이벤트 넘침 등) — `hold_ms` 뒤 저절로 정상으로 돌아온다.
    pub fn degraded_for(&self, reason_key: &'static str, hold_ms: i64) {
        self.health
            .degrade(self.folder_id, self.sink, reason_key, Some(hold_ms));
    }

    /// `degraded` 로 알린 문제가 풀렸다.
    pub fn recovered(&self) {
        self.health.recover(self.folder_id, self.sink);
    }

    /// OS 핸들이 열려 있을 수 있는 동안 쥔다. 버리면 핸들이 닫힌 것이다.
    pub fn release_token(&self) -> ReleaseToken {
        self.tracker.token(self.sink)
    }
}
