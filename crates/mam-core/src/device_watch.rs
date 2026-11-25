//! 장치(볼륨) 연결·분리 알림 — 플랫폼별 감지를 [`DeviceWatcher`] 뒤에 숨긴다.
//!
//! 알림은 볼륨 정보를 나르지 않는다: "뭔가 바뀌었다" 만 알리고, 수집 서비스가 점검(`supervise_once`)을 즉시 깨운다.
//! 판정은 항상 [`crate::VolumeProvider::list_volumes`] 한 경로에서만 한다. 알림이 누락돼도 30초 주기 점검이 보완한다.
//!
//! # macOS 설계 메모 (구현 없음 — macOS 지원이 제품 범위로 확정될 때 별도 작업)
//! - 감지: DiskArbitration `DARegisterDiskAppearedCallback` / `DARegisterDiskDisappearedCallback`
//!   (또는 `NSWorkspace.didMountNotification` / `didUnmountNotification`)
//! - 식별 키: `kDADiskDescriptionVolumeUUIDKey` (볼륨 UUID), 마운트 지점은 `/Volumes/<이름>`
//! - `VolumeProvider`·`DeviceWatcher` 의 macOS 구현만 추가하면 새 볼륨 판정·대기 목록·이벤트 코어는 그대로 쓴다
//! - 시스템 폴더 제외: `.Spotlight-V100`·`.Trashes`·`.fseventsd`·`.TemporaryItems`·`.DocumentRevisions-V100` 은
//!   `.` 접두 규칙(`scan::walk::is_hidden_name`)으로 이미 제외된다

use std::any::Any;
use std::sync::Arc;

use crate::error::CoreError;

/// 장치 변경 알림이 올 때마다 (감시 스레드에서) 호출되는 가벼운 콜백.
pub type DeviceNotify = Arc<dyn Fn() + Send + Sync>;

/// 장치 변경 감지기.
pub trait DeviceWatcher: Send + Sync {
    /// 감지를 시작한다. 반환된 핸들을 drop 하면 감지가 멈추고 스레드가 정리된다.
    /// 시작에 실패하면 오류 — 호출자는 주기 점검만으로 계속 동작한다.
    fn start(&self, notify: DeviceNotify) -> Result<Box<dyn Any + Send>, CoreError>;
}
