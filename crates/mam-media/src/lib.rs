//! MAM 미디어 처리 — 형식 판별, FFprobe/FFmpeg 사이드카, 메타데이터, 썸네일, WebView2 재생 판정.
//!
//! - 원본 파일은 **읽기 전용**으로만 연다. 산출물(썸네일)은 호출자가 준 캐시 루트에만 쓴다.
//! - FFmpeg/FFprobe 는 별도 프로세스(사이드카)로 실행한다 — 타임아웃·출력 상한·강제 종료 적용.
//!   실행 파일이 없으면 영상·음성 처리만 오류(`ToolNotFound`)가 되고 이미지 처리는 계속 동작한다.
//! - 모든 실패는 자산 단위 오류([`MediaError`])로 돌려준다 — 패닉으로 번지지 않는다.

pub mod error;
pub mod format;
pub mod frames;
pub mod gate;
pub mod image_meta;
pub mod metadata;
pub mod playback;
pub mod probe;
mod proc_tree;
pub mod raw_preview;
pub mod scene;
pub mod strip;
pub mod thumbnail;
pub mod tool;
pub mod wic;

pub use error::{MediaError, Result};
pub use format::{Container, Detected, MediaKind, detect};
pub use gate::SidecarGate;
pub use image_meta::{ImageMeta, read_image_meta};
pub use metadata::{MediaMetadata, extract_metadata, parse_iso_ms};
pub use playback::{Playback, PlaybackVerdict, image_in_webview, playback_of, webview_playback};
pub use probe::{AudioInfo, MediaProbe, VideoInfo};
pub use scene::{
    MAX_SCENE_OUTPUT, RawCut, SceneDetectOptions, THRESHOLD_HIGH, THRESHOLD_LOW, THRESHOLD_MEDIUM,
    detect_scene_cuts, parse_scdet_output, scene_timeout,
};
pub use strip::{STRIP_FRAMES, StripOutput, create_frame_strip, strip_path};
pub use thumbnail::{
    DecodeMethod, ThumbSize, ThumbnailOptions, ThumbnailOutput, create_thumbnails, thumbnail_path,
};
pub use tool::{
    CancellableProcessRunner, CommandRunner, MediaTools, ProcessRunner, Tool, ToolConfig,
};
pub use wic::decode_with_wic;
