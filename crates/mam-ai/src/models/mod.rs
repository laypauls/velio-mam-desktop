//! 모델 매니페스트·다운로드·저장소 (D5). 네트워크는 이 모듈의 다운로드에서만 쓴다.

mod download;
mod manifest;
mod space;
mod store;

pub use download::sha256_file;
pub use manifest::{Manifest, ModelFile, ModelKind, ModelSpec};
pub use space::{SpaceProbe, SystemSpace};
pub use store::{DisabledReason, FeatureState, ModelPaths, ModelStatus, ModelStore};

/// 내장 매니페스트의 모델 ID.
pub mod ids {
    pub const CLIP_VISION: &str = "clip-vit-b32-vision";
    pub const CLIP_TEXT_ML: &str = "clip-b32-multilingual-text";
    pub const E5_SMALL: &str = "multilingual-e5-small";
    pub const WHISPER_BASE: &str = "whisper-base";
    pub const WHISPER_SMALL: &str = "whisper-small";
}
