//! `scan_only` — 실시간 감시 없음. 변경은 주기 재스캔(또는 수동 재스캔)으로만 반영한다. 항상 사용 가능·시작 실패 없음.

use crate::watch::detector::{
    ChangeDetector, DetectError, DetectorOptions, ProbeResult, WatchHandle, WatchTarget,
};
use crate::watch::sink::ChangeSink;

pub struct ScanOnly;

struct NoopHandle;

impl WatchHandle for NoopHandle {}

impl ChangeDetector for ScanOnly {
    fn id(&self) -> &'static str {
        "scan_only"
    }

    fn probe(&self, _target: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }

    /// 열 OS 핸들이 없어 토큰을 쥐지 않는다.
    fn start(
        &self,
        _target: &WatchTarget,
        _opts: &DetectorOptions,
        _sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        Ok(Box::new(NoopHandle))
    }
}
