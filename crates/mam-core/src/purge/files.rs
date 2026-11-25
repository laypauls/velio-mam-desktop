//! 썸네일·미리보기 캐시 파일 삭제. 경로는 `<cache_root>/thumbnails|previews/...` 로만 만들어지며 원본 폴더는 건드리지 않는다.

use std::path::Path;

use mam_media::{ThumbSize, strip_path, thumbnail_path};

/// 자산들의 썸네일(2단계)·프레임 스트립 파일을 지운다. 없는 파일·삭제 실패는 무시한다 (캐시는 다시 만들 수 있고,
/// 이 자산들은 이미 삭제 표시라 다시 만들어지지도 않는다). 지운 파일 수를 돌려준다.
pub fn remove_cache_files(cache_root: &Path, asset_ids: &[i64]) -> usize {
    let mut removed = 0;
    for &id in asset_ids {
        let paths = [
            thumbnail_path(cache_root, id, ThumbSize::Small),
            thumbnail_path(cache_root, id, ThumbSize::Large),
            strip_path(cache_root, id),
        ];
        removed += paths
            .iter()
            .filter(|p| std::fs::remove_file(p).is_ok())
            .count();
    }
    removed
}
