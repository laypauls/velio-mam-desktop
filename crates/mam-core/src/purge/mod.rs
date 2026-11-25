//! 자산 연결 데이터 영구 삭제 — 관리 폴더 제거·스캔 유형 정리·기존 보관 데이터 정리가 함께 쓴다.
//!
//! 지우는 것은 MAM 이 만든 정보(DB 행·썸네일·미리보기)뿐이다. **원본 미디어는 읽지도 쓰지도 않는다.**
//! 자산 행은 `status = 'deleted'` 로 남기고(번호 재사용 방지) 검색 인덱스에는 `delete` 이벤트를 보낸다.
//!
//! - [`linked`]: 자산 id 목록의 연결 데이터 행 삭제 + 소프트 삭제 + outbox (한 트랜잭션 안에서 호출)
//! - [`range`]: 폴더 범위 한 배치 (다른 곳에 활성 위치가 있는 자산은 지우지 않고 이 범위의 위치 행만 지운다)
//! - [`legacy`]: 폴더 제거 때 남겨 둔 옛 보관 데이터(활성인데 활성 위치가 없는 자산) 정리
//! - [`files`]: 썸네일·미리보기 캐시 파일 삭제 (MAM 데이터 폴더 안만)

mod files;
mod impact;
mod legacy;
mod linked;
mod range;

pub use files::remove_cache_files;
pub use impact::{FolderRemovalImpact, folder_removal_impact};
pub use legacy::{legacy_candidates, purge_legacy_batch};
pub use linked::{EmbeddingKey, LINKED_TABLES, asset_is_active, purge_assets};
pub use range::{RangeBatch, RangeSpec, count_in_range, purge_range_batch};
