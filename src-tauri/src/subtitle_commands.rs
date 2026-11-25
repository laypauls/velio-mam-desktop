//! 동영상 외부 자막 command - 목록 / 큐 / 플레이어 자막 선택. 탐색·파싱은 `mam_core::subtitle`, 임시 SRT 는 `player_subtitle`.
//! 계약 정본: 설계 `docs/requests/2026-11/20261113-163831-architect-subtitles-design.md` §3, TS `src/lib/ipc-contract-subtitle.ts`.
//!
//! - 원본·자막 파일은 읽기만 한다. 파일 경로는 오류 메시지에 싣지 않는다 (탐색 실패 로그는 core 가 경로와 함께 남긴다)
//! - `subtitle_tracks` 는 항상 다시 탐색하고(열 때마다 1회) 캐시를 교체한다. `subtitle_cues`·`player_set_subtitle` 은 캐시를 쓰고 없으면 같은 탐색을 한다
//! - 탐색은 차단형 파일시스템 호출이라 `spawn_blocking` 에서 실행한다 (열기와 직렬화하지 않는다)

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mam_core::reuse::Reuse;
use mam_core::scan::IngestError;
use mam_core::subtitle::{Cue, ParsedTrack, SubtitleTrack, load_sidecar_tracks};
use tauri::State;

use crate::error::{CommandError, ErrorCode};
use crate::player_commands::PlayerService;

/// 최근 자산 캐시 개수
const CACHE_ASSETS: usize = 4;
/// 트랙 id 최대 글자 수 (계약: 512자)
const MAX_TRACK_ID_CHARS: usize = 512;

type Cache = VecDeque<(i64, Arc<Vec<ParsedTrack>>)>;

#[derive(Default)]
pub struct SubtitleService {
    /// 앞쪽이 가장 최근
    cache: Mutex<Cache>,
}

fn check_track_id(id: &str) -> Result<(), CommandError> {
    if id.is_empty() || id.chars().count() > MAX_TRACK_ID_CHARS {
        return Err(CommandError::new(ErrorCode::InvalidInput, "invalid trackId"));
    }
    Ok(())
}

fn original_path(reuse: &Reuse, asset_id: i64) -> Result<PathBuf, CommandError> {
    match reuse.original_path(asset_id) {
        Ok(p) => Ok(p),
        Err(IngestError::Offline | IngestError::NotFound) => {
            Err(CommandError::new(ErrorCode::AssetOffline, "the original file is offline or missing"))
        }
        Err(e) => Err(e.into()),
    }
}

impl SubtitleService {
    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 다시 탐색하고 캐시를 교체한다 (가장 최근으로)
    fn rescan(&self, asset_id: i64, video: &Path) -> Arc<Vec<ParsedTrack>> {
        let parsed = Arc::new(load_sidecar_tracks(video));
        let mut cache = self.lock();
        cache.retain(|(id, _)| *id != asset_id);
        cache.push_front((asset_id, Arc::clone(&parsed)));
        cache.truncate(CACHE_ASSETS);
        parsed
    }

    /// 캐시에 있으면 가장 최근으로 올려 돌려주고, 없으면 탐색한다
    fn cached_or_scan(
        &self,
        asset_id: i64,
        video: impl FnOnce() -> Result<PathBuf, CommandError>,
    ) -> Result<Arc<Vec<ParsedTrack>>, CommandError> {
        {
            let mut cache = self.lock();
            if let Some(pos) = cache.iter().position(|(id, _)| *id == asset_id)
                && let Some(entry) = cache.remove(pos)
            {
                let parsed = Arc::clone(&entry.1);
                cache.push_front(entry);
                return Ok(parsed);
            }
        }
        Ok(self.rescan(asset_id, &video()?))
    }

    #[cfg(test)]
    fn cached_assets(&self) -> Vec<i64> {
        self.lock().iter().map(|(id, _)| *id).collect()
    }

    /// 정렬된 트랙 (없으면 `[]`). 항상 다시 탐색한다. 오프라인·파일 없음·자산 없음 → `asset_offline`
    pub fn tracks(&self, reuse: &Reuse, asset_id: i64) -> Result<Vec<SubtitleTrack>, CommandError> {
        let video = original_path(reuse, asset_id)?;
        Ok(self.rescan(asset_id, &video).iter().map(|p| p.track.clone()).collect())
    }

    /// 트랙의 큐. 빈/긴 `track_id` → `invalid_input`, 없는 트랙 → `not_found`, 오프라인 → `asset_offline`
    pub fn cues(&self, reuse: &Reuse, asset_id: i64, track_id: &str) -> Result<Vec<Cue>, CommandError> {
        check_track_id(track_id)?;
        let parsed = self.cached_or_scan(asset_id, || original_path(reuse, asset_id))?;
        parsed
            .iter()
            .find(|p| p.track.id == track_id)
            .map(|p| p.cues.clone())
            .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "subtitle track not found"))
    }
}

type Subs<'a> = State<'a, Arc<SubtitleService>>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, CommandError> + Send + 'static) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        tracing::error!(%e, "blocking task failed");
        CommandError::internal()
    })?
}

#[tauri::command]
pub async fn subtitle_tracks(subs: Subs<'_>, reuse: State<'_, Arc<Reuse>>, asset_id: i64) -> Result<Vec<SubtitleTrack>, CommandError> {
    let (subs, reuse) = (Arc::clone(&subs), Arc::clone(&reuse));
    blocking(move || subs.tracks(&reuse, asset_id)).await
}

#[tauri::command]
pub async fn subtitle_cues(subs: Subs<'_>, reuse: State<'_, Arc<Reuse>>, asset_id: i64, track_id: String) -> Result<Vec<Cue>, CommandError> {
    let (subs, reuse) = (Arc::clone(&subs), Arc::clone(&reuse));
    blocking(move || subs.cues(&reuse, asset_id, &track_id)).await
}

/// `track_id` 가 `null` 이면 자막을 끈다. 열린 미디어가 없거나 다른 자산이면 Ok(무시)
#[tauri::command]
pub async fn player_set_subtitle(
    player: State<'_, Arc<PlayerService>>,
    subs: Subs<'_>,
    reuse: State<'_, Arc<Reuse>>,
    asset_id: i64,
    track_id: Option<String>,
) -> Result<(), CommandError> {
    let (player, subs, reuse) = (Arc::clone(&player), Arc::clone(&subs), Arc::clone(&reuse));
    blocking(move || set_subtitle_for(&player, &subs, &reuse, asset_id, track_id)).await
}

/// 인자 검증 -> (다른 자산이 열려 있으면 탐색·임시 SRT 없이 Ok) -> 큐 조회 -> 플레이어에 전달
fn set_subtitle_for(
    player: &PlayerService,
    subs: &SubtitleService,
    reuse: &Reuse,
    asset_id: i64,
    track_id: Option<String>,
) -> Result<(), CommandError> {
    if let Some(id) = &track_id {
        check_track_id(id)?;
    }
    if player.is_other_asset(asset_id) {
        return Ok(()); // 늦게 도착한 이전 자산의 요청 - 엔진도 무시한다
    }
    let sub = match track_id {
        Some(id) => {
            let cues = subs.cues(reuse, asset_id, &id)?;
            Some((id, cues))
        }
        None => None,
    };
    player.set_subtitle(asset_id, sub)
}

#[cfg(test)]
#[path = "subtitle_commands_tests.rs"]
mod tests;
