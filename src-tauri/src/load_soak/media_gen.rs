//! 합성 미디어 파일 생성 — 작은 JPEG/PNG(템플릿 + 고유 꼬리 8바이트), 짧은 WAV, FFmpeg 가 있으면 2초 mp4.
//!
//! 파일마다 내용이 달라야 해시 중복 판정이 일어나지 않는다: JPEG/PNG 는 끝 표식 뒤의 꼬리(디코더가 무시), mp4 는 끝에
//! 붙인 `free` 박스, WAV 는 샘플 값으로 구분한다. 합성 미디어는 실제 사진·영상보다 디코딩 비용이 작다(썸네일 부하가 가볍다).

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use image::{ImageFormat, Rgb, RgbImage};

use super::stats::Lcg;
use super::synth::file_stem;

/// 템플릿 크기 (가로, 세로, 비중 %)
const SIZES: [(u32, u32, usize); 3] = [(640, 480, 60), (1280, 720, 35), (2400, 1600, 5)];
/// 스모크용 작은 템플릿 (같은 비중)
const SIZES_LIGHT: [(u32, u32, usize); 3] = [(64, 48, 60), (96, 64, 35), (128, 96, 5)];

fn gradient(w: u32, h: u32, hue: u8) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        let r = ((x * 255) / w) as u8 ^ hue;
        let g = ((y * 255) / h) as u8;
        let b = (((x + y) * 255) / (w + h)) as u8 ^ hue.rotate_left(3);
        Rgb([r, g, b])
    })
}

fn encode(img: &RgbImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, format).expect("encode template");
    out.into_inner()
}

/// 이미지·영상 템플릿 묶음.
pub struct Templates {
    jpeg: Vec<Vec<u8>>,
    png: Vec<u8>,
    mp4: Option<Vec<u8>>,
}

impl Templates {
    /// `scratch` 는 ffmpeg 출력용 임시 폴더 (MAM 데이터 밖 벤치 폴더).
    pub fn build(scratch: &Path, light: bool) -> Self {
        let mut jpeg = Vec::new();
        for (w, h, _) in if light { SIZES_LIGHT } else { SIZES } {
            for hue in 0..4u8 {
                jpeg.push(encode(&gradient(w, h, hue * 60), ImageFormat::Jpeg));
            }
        }
        let png = encode(
            &gradient(
                if light { 64 } else { 640 },
                if light { 48 } else { 480 },
                17,
            ),
            ImageFormat::Png,
        );
        Self {
            jpeg,
            png,
            mp4: if light { None } else { make_mp4(scratch) },
        }
    }

    pub fn has_video(&self) -> bool {
        self.mp4.is_some()
    }
}

fn make_mp4(scratch: &Path) -> Option<Vec<u8>> {
    fs::create_dir_all(scratch).ok()?;
    let out = scratch.join("template.mp4");
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc=duration=2:size=320x240:rate=15")
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-shortest"])
        .arg(&out)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    fs::read(&out).ok()
}

fn wav_bytes(id: u64) -> Vec<u8> {
    let samples = 8_000usize; // 1초
    let mut data = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        let s = (((i as u64 * 31 + id * 7) % 2000) as i16 - 1000) * 8;
        data.extend_from_slice(&s.to_le_bytes());
    }
    let mut v = Vec::with_capacity(44 + data.len());
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&8000u32.to_le_bytes());
    v.extend_from_slice(&16000u32.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    v
}

/// 파일 한 개의 (확장자, 내용). `id` 가 다르면 내용도 다르다.
pub fn file_for(t: &Templates, id: u64) -> (&'static str, Vec<u8>) {
    let mut rng = Lcg(id.wrapping_mul(0x9E37_79B9) ^ 0xABCD);
    let roll = rng.below(100);
    let trailer = id.to_le_bytes();
    match roll {
        0..=87 => {
            // 크기 비중(SIZES)대로 템플릿 선택: 60% / 35% / 5%
            let pick = rng.below(100);
            let size_idx = if pick < SIZES[0].2 {
                0
            } else if pick < SIZES[0].2 + SIZES[1].2 {
                1
            } else {
                2
            };
            let chosen = &t.jpeg[size_idx * 4 + rng.below(4)];
            let mut b = chosen.clone();
            b.extend_from_slice(&trailer);
            ("jpg", b)
        }
        88..=91 => {
            let mut b = t.png.clone();
            b.extend_from_slice(&trailer);
            ("png", b)
        }
        92..=95 => match &t.mp4 {
            Some(m) => {
                let mut b = m.clone();
                // 유효한 `free` 박스 (크기 16 = 헤더 8 + 8바이트 고유 값)
                b.extend_from_slice(&[0, 0, 0, 16, b'f', b'r', b'e', b'e']);
                b.extend_from_slice(&trailer);
                ("mp4", b)
            }
            None => {
                let mut b = t.png.clone();
                b.extend_from_slice(&trailer);
                ("png", b)
            }
        },
        _ => ("wav", wav_bytes(id)),
    }
}

#[derive(Debug, Default, Clone)]
pub struct GenReport {
    pub files: usize,
    pub bytes: u64,
    pub videos: usize,
    pub audios: usize,
    pub secs: f64,
}

fn dir_of(root: &Path, id: u64) -> PathBuf {
    root.join(format!("d{:04}", id / 1000))
}

/// `ids` 범위의 파일을 `root` 아래에 만든다 (8 스레드). 파일명은 영어·한국어 어휘 + 번호.
pub fn generate(root: &Path, t: &Templates, ids: std::ops::Range<u64>, seed: u64) -> GenReport {
    let started = Instant::now();
    fs::create_dir_all(root).expect("create library root");
    let mut dirs: Vec<PathBuf> = ids.clone().step_by(1000).map(|i| dir_of(root, i)).collect();
    dirs.push(dir_of(root, ids.end.saturating_sub(1)));
    dirs.sort();
    dirs.dedup();
    for d in &dirs {
        fs::create_dir_all(d).expect("create dir");
    }
    let threads = 8u64;
    let reports: Vec<GenReport> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|k| {
                let ids = ids.clone();
                s.spawn(move || {
                    let mut r = GenReport::default();
                    let mut id = ids.start + k;
                    while id < ids.end {
                        let (ext, bytes) = file_for(t, id);
                        let mut rng = Lcg(seed ^ id);
                        let stem = file_stem(&mut rng, id as i64);
                        let path = dir_of(root, id).join(format!("{stem}.{ext}"));
                        fs::write(&path, &bytes).expect("write media file");
                        r.files += 1;
                        r.bytes += bytes.len() as u64;
                        r.videos += usize::from(ext == "mp4");
                        r.audios += usize::from(ext == "wav");
                        id += threads;
                    }
                    r
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("gen thread"))
            .collect()
    });
    let mut total = GenReport::default();
    for r in reports {
        total.files += r.files;
        total.bytes += r.bytes;
        total.videos += r.videos;
        total.audios += r.audios;
    }
    total.secs = started.elapsed().as_secs_f64();
    total
}

/// 폴더 지문 (파일 수, 총 바이트, 수정 시각 합/최대 ns) — 원본 보호 회귀 가드.
pub fn fingerprint(root: &Path) -> (u64, u64, u128, u128) {
    let (mut n, mut bytes, mut sum, mut max) = (0u64, 0u64, 0u128, 0u128);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(e.path());
            } else {
                let m = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                n += 1;
                bytes += meta.len();
                sum += m;
                max = max.max(m);
            }
        }
    }
    (n, bytes, sum, max)
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (생성·고유성) / Boundary (빈 범위) / Business rule (원본 지문)
    use super::*;

    #[test]
    fn t_p4_gen_01_files_are_unique_and_fingerprint_is_stable() {
        // Arrange
        let dir = tempfile::TempDir::new().unwrap();
        let t = Templates::build(&dir.path().join("scratch"), true);
        let root = dir.path().join("lib");
        // Act
        let r = generate(&root, &t, 0..40, 1);
        let fp1 = fingerprint(&root);
        let fp2 = fingerprint(&root);
        // Assert
        assert_eq!(r.files, 40);
        assert_eq!(fp1.0, 40);
        assert_eq!(fp1, fp2, "읽기만 하면 지문이 바뀌지 않는다");
        let a = file_for(&t, 3);
        let b = file_for(&t, 4);
        assert_ne!(a.1, b.1);
    }

    #[test]
    fn t_p4_gen_02_empty_range_creates_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let t = Templates::build(&dir.path().join("scratch"), true);
        let r = generate(&dir.path().join("lib"), &t, 5..5, 1);
        assert_eq!(r.files, 0);
    }
}
