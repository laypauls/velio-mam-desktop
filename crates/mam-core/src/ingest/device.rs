//! 장치 변경 알림 연결 — [`DeviceWatcher`] 의 알림을 점검 스레드 깨우기로 잇는다.
//!
//! 알림은 `supervise_once` 를 앞당길 뿐이다 (판정은 점검의 `list_volumes` 한 곳). 감지기가 시작에 실패해도
//! 30초 주기 점검이 그대로 동작한다.

use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Ingest, lock};
use crate::device_watch::DeviceWatcher;

#[derive(Default)]
pub(super) struct DeviceHook {
    watcher: Mutex<Option<(Arc<dyn DeviceWatcher>, Duration)>>,
    guard: Mutex<Option<Box<dyn Any + Send>>>,
}

impl Ingest {
    /// 장치 변경 감지기를 꽂는다. `debounce` 는 알림이 조용해진 뒤 점검을 시작하기까지의 대기(파티션 여러 개 동시 연결을
    /// 한 번으로 묶는다). [`Ingest::start`] 전에 호출한다.
    pub fn set_device_watcher(&self, watcher: Arc<dyn DeviceWatcher>, debounce: Duration) {
        *lock(&self.device.watcher) = Some((watcher, debounce));
    }

    pub(super) fn start_device_watch(&self) {
        let Some((watcher, debounce)) = lock(&self.device.watcher).clone() else {
            return;
        };
        let stop = Arc::clone(&self.stop);
        let notify = Arc::new(move || stop.request_wake(debounce));
        match watcher.start(notify) {
            Ok(guard) => *lock(&self.device.guard) = Some(guard),
            Err(e) => tracing::warn!(%e, "cannot watch device changes; periodic checks only"),
        }
    }

    /// 감지기를 멈추고 스레드를 정리한다 (핸들 drop).
    pub(super) fn stop_device_watch(&self) {
        let guard = lock(&self.device.guard).take();
        drop(guard);
    }
}
