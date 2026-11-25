//! 분석·시각 임베딩에 쓸 이미지 선택 - 이미지는 썸네일, 영상은 P2-3 프레임 스트립의 칸 (MAMData 안의 파일만).

use std::path::PathBuf;

use mam_media::{STRIP_FRAMES, strip_path};
use rusqlite::OptionalExtension;

use super::AiJobError;
use super::executor::AiExecutor;
use super::types::{FrameCell, ImageInput};

/// `cells` 칸 중 `want` 칸을 위치 순으로 고르게 고른다 (양 끝 포함). `want` 는 1..=cells 로 보정, 하나면 가운데.
pub(super) fn pick_cells(cells: u32, want: u32) -> Vec<u32> {
    if cells == 0 {
        return Vec::new();
    }
    let n = want.clamp(1, cells);
    if n == cells {
        return (0..cells).collect();
    }
    if n == 1 {
        return vec![cells / 2];
    }
    let (span, steps) = (u64::from(cells - 1), u64::from(n - 1));
    (0..u64::from(n))
        .map(|i| u32::try_from((2 * i * span + steps) / (2 * steps)).unwrap_or(0))
        .collect()
}

/// 시각 입력 준비 상태.
pub(super) enum Visual {
    Ready(Vec<ImageInput>),
    /// 필요한 파생물(썸네일/스트립)이 아직 만들어지는 중 - 시도를 쓰지 않고 기다린다
    Waiting,
    /// 파생물 작업이 끝났는데 쓸 파일이 없다 - `thumbnail_missing` (재시도 대상)
    Missing,
}

impl AiExecutor {
    /// 영상 프레임 스트립 상태 (`asset_derivatives.kind = 'strip'`) - 행이 없으면 `None`(아직 안 만들어짐).
    fn strip_state(&self, asset_id: i64) -> Result<Option<String>, AiJobError> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT state FROM asset_derivatives WHERE asset_id = ?1 AND kind = 'strip'",
                [asset_id],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    /// 썸네일 1장 (큰 것 우선) - 이미지, 그리고 프레임을 쓸 수 없는 영상의 대체.
    fn thumb_inputs(&self, asset_id: i64) -> Vec<ImageInput> {
        self.thumbs(asset_id)
            .into_iter()
            .map(|path| ImageInput { path, frame: None })
            .collect()
    }

    /// 썸네일 파생물 상태 (`asset_derivatives.kind = 'thumb'`) - 행이 없으면 `None`(아직 안 만들어짐).
    fn thumb_state(&self, asset_id: i64) -> Result<Option<String>, AiJobError> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT state FROM asset_derivatives WHERE asset_id = ?1 AND kind = 'thumb'",
                [asset_id],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    /// 외부 분석·시각 임베딩 입력 준비 상태.
    ///
    /// - 이미지: 썸네일 파일이 있으면 `Ready`. 없고 썸네일 작업이 끝난 상태(done/failed/unsupported)면 `Missing`,
    ///   아직 안 끝났으면(행 없음/pending) `Waiting`
    /// - 영상 + 스트립 `done` + 파일 있음 -> 스트립 칸 `min(video_frames, 칸 수)` 개 (고르게), 파일 없음 -> `Missing`
    /// - 영상 + 스트립 `unsupported`/`failed` (썸네일 작업이 재시도하지 않는 상태) -> 썸네일 1장으로 대체 (없으면 `Missing`)
    /// - 영상 + 스트립 미생성(행 없음/`pending`) -> `Waiting`. 단 썸네일 자체가 끝났는데 스트립이 없으면
    ///   (failed/unsupported) 더 기다려도 만들어지지 않으므로 `Missing`
    pub(super) fn visual_inputs(
        &self,
        asset_id: i64,
        media_type: &str,
        video_frames: u8,
    ) -> Result<Visual, AiJobError> {
        let thumbs = |this: &Self| {
            let t = this.thumb_inputs(asset_id);
            if t.is_empty() {
                Visual::Missing
            } else {
                Visual::Ready(t)
            }
        };
        if media_type != "video" {
            let ready = thumbs(self);
            if matches!(ready, Visual::Ready(_)) {
                return Ok(ready);
            }
            return self.wait_or_missing(asset_id);
        }
        match self.strip_state(asset_id)?.as_deref() {
            Some("done") => {
                let path: PathBuf = strip_path(&self.cfg.cache_root, asset_id);
                if !path.is_file() {
                    return Ok(Visual::Missing);
                }
                let cells = u32::try_from(STRIP_FRAMES).unwrap_or(5);
                Ok(Visual::Ready(
                    pick_cells(cells, u32::from(video_frames))
                        .into_iter()
                        .map(|index| ImageInput {
                            path: path.clone(),
                            frame: Some(FrameCell { index, cells }),
                        })
                        .collect(),
                ))
            }
            Some("unsupported" | "failed") => Ok(thumbs(self)),
            _ => self.wait_or_missing(asset_id),
        }
    }

    /// 입력이 아직 없을 때: 썸네일 작업이 끝났다면(성공이든 실패든) 더 기다려도 생기지 않는다.
    fn wait_or_missing(&self, asset_id: i64) -> Result<Visual, AiJobError> {
        Ok(match self.thumb_state(asset_id)?.as_deref() {
            Some("done" | "failed" | "unsupported") => Visual::Missing,
            _ => Visual::Waiting,
        })
    }
}

#[cfg(test)]
mod tests {
    //! T-aij-35a - 프레임 고르게 선택 (Boundary). 분류: Boundary
    use super::pick_cells;

    #[test]
    fn t_aij_35a_pick_cells_is_even_ordered_and_clamped() {
        assert_eq!(pick_cells(7, 5), vec![0, 2, 3, 5, 6]);
        assert_eq!(pick_cells(5, 5), vec![0, 1, 2, 3, 4]);
        assert_eq!(pick_cells(5, 3), vec![0, 2, 4]);
        assert_eq!(pick_cells(5, 1), vec![2]);
        assert_eq!(pick_cells(5, 0), vec![2], "0 은 1 로 보정");
        assert_eq!(pick_cells(5, 9), vec![0, 1, 2, 3, 4], "칸 수를 넘지 않는다");
        assert!(pick_cells(0, 5).is_empty());
    }
}
