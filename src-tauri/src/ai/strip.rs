//! 영상 프레임 스트립(P2-3, 셀을 가로로 이어 붙인 WebP)에서 셀 하나를 잘라 임시 파일로 만든다.
//! 어댑터가 외부 전송/임베딩 전에 쓰고, 끝나면 [`TempCell`] 드롭으로 지운다. 출력은 MAMData 안의 임시 폴더뿐이다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::{ImageFormat, ImageReader};

use mam_core::ai_jobs::{ImageInput, StepFailure};

#[derive(Debug, thiserror::Error)]
pub enum StripError {
    #[error("invalid cell index")]
    Index,
    #[error("image error")]
    Image,
    #[error("io error")]
    Io,
}

/// 요청 이미지를 실제 파일 경로로 푼다 - 프레임이면 스트립에서 칸을 잘라 `tmp_dir` 에 쓴다.
/// 돌려받은 `TempCell` 들은 사용이 끝날 때까지 들고 있어야 한다 (드롭 시 삭제).
pub fn resolve_images(
    images: &[ImageInput],
    tmp_dir: &Path,
) -> Result<(Vec<PathBuf>, Vec<TempCell>), StepFailure> {
    let mut paths = Vec::with_capacity(images.len());
    let mut cells = Vec::new();
    for img in images {
        match img.frame {
            None => paths.push(img.path.clone()),
            Some(f) => {
                let cell =
                    crop_strip_cell(&img.path, f.index, f.cells, tmp_dir).map_err(|e| match e {
                        StripError::Index => {
                            StepFailure::new("invalid_input", "프레임 번호가 올바르지 않습니다")
                        }
                        StripError::Io => {
                            StepFailure::new("thumbnail_missing", "프레임 스트립을 읽지 못했습니다")
                        }
                        StripError::Image => {
                            StepFailure::new("decode_failed", "프레임 스트립을 해석하지 못했습니다")
                        }
                    })?;
                paths.push(cell.path().to_path_buf());
                cells.push(cell);
            }
        }
    }
    Ok((paths, cells))
}

/// 잘라낸 셀 임시 파일 — 드롭 시 삭제.
#[derive(Debug)]
pub struct TempCell(PathBuf);

impl TempCell {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempCell {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `path` 스트립의 `index` 번째(0부터) 셀을 PNG 로 `tmp_dir` 에 쓴다.
pub fn crop_strip_cell(
    path: &Path,
    index: u32,
    cells: u32,
    tmp_dir: &Path,
) -> Result<TempCell, StripError> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if cells == 0 || index >= cells {
        return Err(StripError::Index);
    }
    let img = ImageReader::open(path)
        .map_err(|_| StripError::Io)?
        .with_guessed_format()
        .map_err(|_| StripError::Io)?
        .decode()
        .map_err(|_| StripError::Image)?;
    let cell_w = img.width() / cells;
    if cell_w == 0 {
        return Err(StripError::Image);
    }
    let cell = img.crop_imm(index * cell_w, 0, cell_w, img.height());
    std::fs::create_dir_all(tmp_dir).map_err(|_| StripError::Io)?;
    let out = tmp_dir.join(format!(
        "cell-{}-{}.png",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    cell.save_with_format(&out, ImageFormat::Png)
        .map_err(|_| StripError::Io)?;
    Ok(TempCell(out))
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Boundary / Validation
    use image::{Rgb, RgbImage};

    use super::*;

    fn strip(dir: &Path, cells: u32) -> PathBuf {
        let mut img = RgbImage::new(cells * 8, 6);
        for (x, _, px) in img.enumerate_pixels_mut() {
            *px = Rgb([(x / 8 * 40) as u8, 0, 0]);
        }
        let p = dir.join("strip.png");
        img.save(&p).unwrap();
        p
    }

    #[test]
    fn t_p3w_strip_01_crops_requested_cell_and_cleans_up() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let src = strip(dir.path(), 5);
        let tmp = dir.path().join("tmp");
        // Act
        let cell = crop_strip_cell(&src, 3, 5, &tmp).unwrap();
        let out = image::open(cell.path()).unwrap().to_rgb8();
        // Assert
        assert_eq!((out.width(), out.height()), (8, 6));
        assert_eq!(out.get_pixel(0, 0).0[0], 120);
        let p = cell.path().to_path_buf();
        drop(cell);
        assert!(!p.exists());
        assert!(src.exists(), "strip itself is never touched");
    }

    #[test]
    fn t_p3w_strip_02_rejects_bad_index_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = strip(dir.path(), 5);
        let tmp = dir.path().join("tmp");
        assert!(matches!(
            crop_strip_cell(&src, 5, 5, &tmp),
            Err(StripError::Index)
        ));
        assert!(matches!(
            crop_strip_cell(&src, 0, 0, &tmp),
            Err(StripError::Index)
        ));
        assert!(matches!(
            crop_strip_cell(&dir.path().join("none.webp"), 0, 5, &tmp),
            Err(StripError::Io)
        ));
    }
}
