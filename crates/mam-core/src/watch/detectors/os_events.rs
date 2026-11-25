//! `os_events` — `notify`(Windows ReadDirectoryChangesW) 재귀 감시. 콜백은 가볍게 메시지만 sink 로 보낸다.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::watch::detector::{
    ChangeDetector, DetectError, DetectorOptions, ProbeResult, WatchHandle, WatchTarget,
};
use crate::watch::sink::{ChangeSink, WatchMsg};

pub struct OsEvents;

/// 버퍼 넘침 / 감시 오류 사유 키 (`watch.health.*`) 와 `Degraded` 를 유지하는 시간 (재스캔을 요청했으니 짧게)
pub const HEALTH_OVERFLOW: &str = "watch.health.overflow";
pub const HEALTH_ERROR: &str = "watch.health.error";
const DEGRADED_HOLD_MS: i64 = 60_000;

struct OsEventsHandle(#[allow(dead_code)] RecommendedWatcher);

impl WatchHandle for OsEventsHandle {}

impl ChangeDetector for OsEvents {
    fn id(&self) -> &'static str {
        "os_events"
    }

    /// 모든 볼륨 종류에서 시도한다 — SMB 등에서 실패하면 시작 실패(폴백)로 드러난다.
    fn probe(&self, _target: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }

    fn start(
        &self,
        target: &WatchTarget,
        _opts: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        // 핸들러가 버려질 때(= notify 스레드가 디렉터리 핸들을 닫은 뒤) 토큰이 풀린다
        let token = sink.release_token();
        let s = sink.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let _keep = &token;
                let failed = res.is_err();
                let msgs = convert(s.folder_id(), res);
                report_health(&s, failed, &msgs);
                for m in msgs {
                    s.emit(m);
                }
            })?;
        watcher.watch(&target.root, RecursiveMode::Recursive)?;
        Ok(Box::new(OsEventsHandle(watcher)))
    }
}

/// 이벤트 넘침·감시 오류는 잠시 `Degraded`(재스캔이 보완), 변경이 있으면 마지막 신호 시각만 갱신한다.
pub(crate) fn report_health(sink: &ChangeSink, failed: bool, msgs: &[WatchMsg]) {
    if msgs.iter().any(|m| matches!(m, WatchMsg::Rescan { .. })) {
        let key = if failed {
            HEALTH_ERROR
        } else {
            HEALTH_OVERFLOW
        };
        sink.degraded_for(key, DEGRADED_HOLD_MS);
    } else if !msgs.is_empty() {
        sink.signal();
    }
}

/// notify 이벤트 -> 메시지 (접근 이벤트는 버린다).
pub fn convert(folder_id: i64, res: notify::Result<notify::Event>) -> Vec<WatchMsg> {
    match res {
        Err(_) => vec![WatchMsg::Rescan { folder_id }],
        Ok(ev) => {
            let mut out = Vec::new();
            if ev.need_rescan() {
                out.push(WatchMsg::Rescan { folder_id });
            }
            if !matches!(ev.kind, EventKind::Access(_)) {
                out.extend(
                    ev.paths
                        .into_iter()
                        .map(|path| WatchMsg::Path { folder_id, path }),
                );
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    //! 분류: External (감시 오류·넘침) / Happy
    use std::path::PathBuf;

    use notify::Event;
    use notify::event::{AccessKind, CreateKind, Flag};

    use super::*;

    #[test]
    fn t_watch_conv_01_paths_become_messages_and_access_is_dropped() {
        let ev =
            Event::new(EventKind::Create(CreateKind::File)).add_path(PathBuf::from(r"C:\a\x.jpg"));
        assert_eq!(
            convert(7, Ok(ev)),
            vec![WatchMsg::Path {
                folder_id: 7,
                path: PathBuf::from(r"C:\a\x.jpg")
            }]
        );
        let access = Event::new(EventKind::Access(AccessKind::Any)).add_path(PathBuf::from("x"));
        assert!(convert(7, Ok(access)).is_empty());
    }

    #[test]
    fn t_watch_conv_02_overflow_flag_and_errors_request_a_rescan() {
        let overflow = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert_eq!(
            convert(3, Ok(overflow)),
            vec![WatchMsg::Rescan { folder_id: 3 }]
        );
        let err = notify::Error::generic("boom");
        assert_eq!(
            convert(3, Err(err)),
            vec![WatchMsg::Rescan { folder_id: 3 }]
        );
    }
}
