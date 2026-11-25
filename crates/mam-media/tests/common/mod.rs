//! 테스트·샘플 생성기 공용 도우미 (합성 이미지, EXIF JPEG, DNG 구조 파일, 스텁 실행기).
#![allow(dead_code)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{ImageFormat, Rgb, RgbImage};
use mam_media::error::{MediaError, Result};
use mam_media::tool::{CommandOutput, CommandRunner, MediaTools, RunOptions, Tool};

/// 왼쪽 절반 빨강, 오른쪽 절반 파랑 — 방향 적용 여부를 픽셀로 확인한다.
pub fn red_blue(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, _| {
        if x < w / 2 {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    })
}

pub fn encode(img: &RgbImage, fmt: ImageFormat) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, fmt).unwrap();
    out.into_inner()
}

/// 리틀 엔디언 TIFF 로 EXIF(방향 + 촬영일)를 만든다.
fn exif_tiff(orientation: u16, datetime: &str) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(b"II*\0");
    t.extend_from_slice(&8u32.to_le_bytes());
    // IFD0: Orientation, ExifIFD 포인터
    t.extend_from_slice(&2u16.to_le_bytes());
    t.extend_from_slice(&entry(0x0112, 3, 1, orientation as u32));
    t.extend_from_slice(&entry(0x8769, 4, 1, 38));
    t.extend_from_slice(&0u32.to_le_bytes());
    // Exif IFD @38: DateTimeOriginal (값은 IFD 뒤 @56)
    t.extend_from_slice(&1u16.to_le_bytes());
    t.extend_from_slice(&entry(0x9003, 2, 20, 56));
    t.extend_from_slice(&0u32.to_le_bytes());
    t.extend_from_slice(datetime.as_bytes());
    t.push(0);
    t
}

fn entry(tag: u16, typ: u16, count: u32, value: u32) -> [u8; 12] {
    let mut e = [0u8; 12];
    e[0..2].copy_from_slice(&tag.to_le_bytes());
    e[2..4].copy_from_slice(&typ.to_le_bytes());
    e[4..8].copy_from_slice(&count.to_le_bytes());
    e[8..12].copy_from_slice(&value.to_le_bytes());
    e
}

/// JPEG 에 EXIF APP1(방향·촬영일)을 끼운다. `datetime` 은 `YYYY:MM:DD HH:MM:SS`.
pub fn jpeg_with_exif(img: &RgbImage, orientation: u16, datetime: &str) -> Vec<u8> {
    let jpeg = encode(img, ImageFormat::Jpeg);
    let tiff = exif_tiff(orientation, datetime);
    let mut seg = Vec::new();
    seg.extend_from_slice(&[0xFF, 0xE1]);
    seg.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
    seg.extend_from_slice(b"Exif\0\0");
    seg.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&seg);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// DNG(TIFF 구조) 합성 파일: IFD0(방향) + SubIFD 2개 — (1) RAW 데이터 흉내(CFA, 무손실 JPEG 헤더)
/// (2) 내장 JPEG 프리뷰. 실제 카메라 RAW 가 아니다.
pub fn synthetic_dng(preview_jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let decoy: Vec<u8> = [&[0xFF, 0xD8, 0xFF, 0xC3, 0x00, 0x0B][..], &[0u8; 64]].concat();
    let decoy_off = 154u32;
    let preview_off = decoy_off + decoy.len() as u32;
    let mut f = Vec::new();
    f.extend_from_slice(b"II*\0");
    f.extend_from_slice(&8u32.to_le_bytes());
    // IFD0 @8 (30바이트)
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&entry(0x0112, 3, 1, orientation as u32));
    f.extend_from_slice(&entry(0x014A, 4, 2, 38));
    f.extend_from_slice(&0u32.to_le_bytes());
    // SubIFD 오프셋 배열 @38
    f.extend_from_slice(&46u32.to_le_bytes());
    f.extend_from_slice(&100u32.to_le_bytes());
    // SubIFD 1 @46: RAW 데이터 흉내 (Compression 7, Photometric CFA)
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&entry(0x0103, 3, 1, 7));
    f.extend_from_slice(&entry(0x0106, 3, 1, 32803));
    f.extend_from_slice(&entry(0x0111, 4, 1, decoy_off));
    f.extend_from_slice(&entry(0x0117, 4, 1, decoy.len() as u32));
    f.extend_from_slice(&0u32.to_le_bytes());
    // SubIFD 2 @100: 프리뷰 (Compression 7, YCbCr)
    f.extend_from_slice(&4u16.to_le_bytes());
    f.extend_from_slice(&entry(0x0103, 3, 1, 7));
    f.extend_from_slice(&entry(0x0106, 3, 1, 6));
    f.extend_from_slice(&entry(0x0111, 4, 1, preview_off));
    f.extend_from_slice(&entry(0x0117, 4, 1, preview_jpeg.len() as u32));
    f.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(f.len(), decoy_off as usize);
    f.extend_from_slice(&decoy);
    f.extend_from_slice(preview_jpeg);
    f
}

/// 스텁 실행기 — 호출을 기록하고 정해진 결과를 돌려준다.
pub struct StubRunner<F>(pub F);

impl<F> CommandRunner for StubRunner<F>
where
    F: Fn(Tool, &[OsString]) -> Result<CommandOutput> + Send + Sync,
{
    fn run(
        &self,
        tool: Tool,
        _program: &Path,
        args: &[OsString],
        _opts: &RunOptions,
    ) -> Result<CommandOutput> {
        (self.0)(tool, args)
    }
}

pub fn stub_tools<F>(f: F) -> MediaTools
where
    F: Fn(Tool, &[OsString]) -> Result<CommandOutput> + Send + Sync + 'static,
{
    MediaTools::with_paths(
        Some(PathBuf::from("ffprobe-stub")),
        Some(PathBuf::from("ffmpeg-stub")),
        Arc::new(StubRunner(f)),
    )
}

pub fn ok_output(stdout: &[u8]) -> CommandOutput {
    CommandOutput {
        code: Some(0),
        stdout: stdout.to_vec(),
        stderr: String::new(),
    }
}

pub fn fail_output(code: i32, stderr: &str) -> CommandOutput {
    CommandOutput {
        code: Some(code),
        stdout: Vec::new(),
        stderr: stderr.to_string(),
    }
}

/// 도구가 없는 상태 (degraded).
pub fn no_tools() -> MediaTools {
    MediaTools::with_paths(None, None, Arc::new(mam_media::tool::ProcessRunner))
}

/// 도구는 있지만 호출되면 실패하는 스텁 (이 경로에서 사이드카가 실행되면 안 됨을 단언).
pub fn must_not_run_tools() -> MediaTools {
    stub_tools(|_, _| Err(MediaError::InvalidOutput("must not run".into())))
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// 픽스처를 임시 디렉터리로 복사한다 (원본 보호: 실제 픽스처·사용자 폴더 접근 금지).
pub fn copy_fixture(name: &str, dir: &Path) -> PathBuf {
    let dest = dir.join(name);
    fs::copy(fixtures_dir().join(name), &dest).unwrap();
    dest
}

/// 파일의 (내용, 수정 시각) 스냅샷 — 원본 불변 단언용.
pub fn snapshot(path: &Path) -> (Vec<u8>, std::time::SystemTime) {
    (
        fs::read(path).unwrap(),
        fs::metadata(path).unwrap().modified().unwrap(),
    )
}

/// 실제 ffmpeg/ffprobe (PATH) 를 쓰는 도구. 없으면 `None` — 호출 테스트는 사유를 출력하고 건너뛴다.
pub fn real_tools() -> Option<MediaTools> {
    use mam_media::tool::{ProcessRunner, ToolConfig};
    let t = MediaTools::discover(&ToolConfig::default(), Arc::new(ProcessRunner));
    (t.has_ffmpeg() && t.has_ffprobe()).then_some(t)
}

/// 스킵 사유를 출력한다 (`cargo test -- --nocapture` 에서 보인다).
pub fn skip(test: &str, reason: &str) {
    eprintln!("SKIP {test}: {reason}");
}
