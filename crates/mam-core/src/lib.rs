//! MAM 코어 — 자산 식별·파일 상태 모델.
//!
//! - [`volume`]: 볼륨 식별 (드라이브 문자 변경 대응) — Windows API 는 [`VolumeProvider`] 뒤에 숨긴다
//! - [`identity`]: 크기·수정 시각·NTFS 파일 ID
//! - [`hash`]: BLAKE3 스트리밍 전체 내용 해시 (도중 변경 감지·취소)
//! - [`reconcile`]: 스캔 결과 <-> DB 판정 (신규 / 재연결 / 내용 변경 / 파일 없음), outbox 기록
//! - [`registry`]: `volumes` 등록과 온라인/오프라인 전환
//! - [`protect`]: 비밀번호 인증·원본 삭제(휴지통)/이동·관리 데이터 초기화 계획 (P2-9)
//!
//! 원본 파일은 읽기 전용으로만 연다. 규칙 정본: `.claude/rules/project.md`, `.claude/rules/db.md`.

pub mod ai_jobs;
pub mod ai_store;
pub mod assets;
pub mod device_watch;
pub mod embedding;
mod error;
pub mod hash;
pub mod identity;
pub mod ingest;
pub mod jobs;
pub mod join;
mod keyrange;
pub mod processing;
pub mod protect;
pub mod protocol;
pub mod purge;
pub mod query;
pub mod reconcile;
pub mod registry;
pub mod reuse;
pub mod scene;
pub mod secret;
pub mod scan;
pub mod storage;
pub mod subtitle;
pub mod userdata;
pub mod volume;
pub mod watch;
#[cfg(windows)]
mod win_device_watch;
#[cfg(windows)]
mod win_identity;
#[cfg(windows)]
mod win_volume;

pub use error::CoreError;
/// 스레드 간에 공유하는 시계 (시험에서는 고정·수동 전진 시계를 넣는다)
pub type SharedClock = std::sync::Arc<dyn mam_db::Clock + Send + Sync>;

pub use hash::{
    Blake3Hasher, ContentHash, ContentHasher, EdgeFile, EdgeSig, HashedFile, edge_signature,
};
pub use identity::{FileIdentityProvider, FileMeta, MetadataOnly};
pub use keyrange::{join_key, key_range};
pub use reconcile::{MediaType, MoveBy, Outcome, PendingHash, Reconciler, ScanEntry, ScanSession};
pub use registry::{
    SyncReport, register_volume, set_volume_online, sync_volumes, sync_volumes_holding,
};
pub use volume::{VolumeInfo, VolumeKind, VolumeProvider, relative_path};
#[cfg(windows)]
pub use win_device_watch::WindowsDeviceWatcher;
#[cfg(windows)]
pub use win_identity::WindowsFileIdentity;
#[cfg(windows)]
pub use win_volume::WindowsVolumes;
