//! subtitle*.rs 통합 테스트 공용 헬퍼 (픽스처는 모두 임시 폴더에 만든다).
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use mam_core::subtitle::{ParsedTrack, load_sidecar_tracks};

pub const SRT_OK: &str = "1\n00:00:01,000 --> 00:00:02,000\n안녕하세요\n\n";

/// 폴더에 파일을 쓴다 (테스트 픽스처 생성용)
pub fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}

/// `video_name` 영상(더미 바이트)을 만들고 자막 트랙을 로드한다
pub fn load(dir: &Path, video_name: &str) -> Vec<ParsedTrack> {
    let video = put(dir, video_name, b"VIDEO");
    load_sidecar_tracks(&video)
}

pub fn file_names(tracks: &[ParsedTrack]) -> Vec<String> {
    tracks.iter().map(|t| t.track.file_name.clone()).collect()
}

pub fn ids(tracks: &[ParsedTrack]) -> Vec<String> {
    tracks.iter().map(|t| t.track.id.clone()).collect()
}

pub fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

pub fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
