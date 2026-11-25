//! 외부 자막을 플레이어 워커(mpv `sub-add`)에 넘기기 위한 임시 SRT 파일 - MAM 캐시(`cache/player-subtitles`)에만 쓴다. 원본 폴더에는 아무것도 쓰지 않는다.
//!
//! - (자산, 트랙)마다 파일 하나를 재사용한다. 이름은 `sub-{pid}-{n}.srt`
//! - 열기·닫기에서 플레이어 호출이 끝난 뒤(워커가 파일 핸들을 놓은 뒤) 지운다. 시작 때 폴더째 지우고, 종료 때도 지운다
//! - 삭제 실패는 무시한다 (다음 시작 때 정리된다)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use mam_core::subtitle::{Cue, to_srt};

use crate::error::{CommandError, ErrorCode};

pub const SUBTITLE_DIR: &str = "player-subtitles";

type Written = HashMap<(i64, String), PathBuf>;

pub struct SubtitleFiles {
    dir: PathBuf,
    seq: AtomicU64,
    written: Mutex<Written>,
}

impl SubtitleFiles {
    /// 이전 실행이 비정상 종료하며 남긴 임시 파일을 지운다 (MAM 자신의 캐시 폴더)
    pub fn new(cache_dir: &Path) -> Self {
        let dir = cache_dir.join(SUBTITLE_DIR);
        let _ = std::fs::remove_dir_all(&dir);
        Self { dir, seq: AtomicU64::new(0), written: Mutex::new(HashMap::new()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Written> {
        self.written.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `(asset_id, track_id)` 의 SRT 경로. 이미 쓴 파일이 있으면 재사용하고, 없으면 `cues` 를 SRT 로 직렬화해 쓴다
    pub fn prepare(&self, asset_id: i64, track_id: &str, cues: &[Cue]) -> Result<PathBuf, CommandError> {
        let mut written = self.lock();
        let key = (asset_id, track_id.to_owned());
        if let Some(p) = written.get(&key).filter(|p| p.is_file()) {
            return Ok(p.clone());
        }
        let io = |e: std::io::Error| CommandError::logged(ErrorCode::Io, "file operation failed", &e);
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let path = self.dir.join(format!("sub-{}-{n}.srt", std::process::id()));
        std::fs::write(&path, to_srt(cues)).map_err(|e| {
            let _ = std::fs::remove_file(&path); // 반쯤 쓴 파일을 남기지 않는다
            io(e)
        })?;
        written.insert(key, path.clone());
        Ok(path)
    }

    /// 지금까지 쓴 임시 파일을 모두 지운다 (플레이어가 열기·닫기를 마친 뒤에 부른다)
    pub fn clear(&self) {
        for (_, p) in self.lock().drain() {
            if let Err(e) = std::fs::remove_file(&p) {
                tracing::debug!(%e, "subtitle temp file was not removed");
            }
        }
    }

    /// 종료: 추적 중인 파일과 폴더를 지운다
    pub fn remove_all(&self) {
        self.clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
