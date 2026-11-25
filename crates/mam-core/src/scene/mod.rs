//! 동영상 장면전환점 검출 작업 (`jobs.kind = 'scene'`) — 설계 `architect-scene-cuts-design` §4.3.
//!
//! - 결과(컷 위치)만 `asset_scene_cuts` 에 쓴다. 원본은 읽기 전용으로만 열고 MAM 산출물도 파일로 만들지 않는다
//! - 검출은 영상 전체 디코딩이라 일시정지·종료 시 FFmpeg 를 죽이고(`Yielded`, 시도 횟수 불변) 재개하면 처음부터 다시 한다
//! - 오프라인·파일 없음 자산은 건드리지 않고 작업만 끝낸다 — 행은 `pending` 으로 남고 볼륨이 돌아오면
//!   `Ingest::supervise_once` 가 재등록한다
//! - 검색 색인 대상이 아니므로 `revision`·`search_outbox` 를 쓰지 않는다

mod executor;
mod store;

use mam_db::SceneSensitivity;
use mam_media::{THRESHOLD_HIGH, THRESHOLD_LOW, THRESHOLD_MEDIUM};

pub use executor::{SceneExecutor, SceneUpdated};
pub use store::{requeue_pending, requeue_pending_in_chunks};

/// 민감도 → `scdet` 임계값. 바뀌면 `mam_db::scene_cuts::SCENE_DETECTOR_VERSION` 을 올린다.
pub fn threshold_for(s: SceneSensitivity) -> f32 {
    match s {
        SceneSensitivity::Low => THRESHOLD_LOW,
        SceneSensitivity::Medium => THRESHOLD_MEDIUM,
        SceneSensitivity::High => THRESHOLD_HIGH,
    }
}

/// 이벤트 `scene://updated` 의 페이로드 — 프론트 계약 `{ assetId }`. 실행기 알림(`SceneUpdated`)이 커밋 뒤에 보낸다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneUpdatedPayload {
    pub asset_id: i64,
}

/// 이벤트 이름
pub const SCENE_UPDATED_EVENT: &str = "scene://updated";
