//! 저장된 메타데이터 읽기 모델 — 검색·상세 화면(P2-5)이 최종 후보에 대해서만 부른다.
//! 프론트 계약 `AssetDetail` 의 codec/audioCodec/fps/bitrate/camera/extra/playable 에 대응한다 (serde camelCase).

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub codec: Option<String>,
    pub audio_codec: Option<String>,
    pub fps: Option<f64>,
    pub bitrate: Option<i64>,
    pub camera: Option<String>,
    /// UTC Unix ms
    pub captured_at: Option<i64>,
    /// WebView2 에서 직접 재생·표시할 수 있는가 (`conditional` 도 시도해 볼 수 있으므로 true). 아직 모르면 false
    pub playable: bool,
    /// 표시용 추가 메타데이터 (현재: `container`)
    pub extra: BTreeMap<String, String>,
}

/// 메타데이터 추출이 끝난 자산의 정보. 아직 처리 전이면 `None`.
pub fn media_info(conn: &Connection, asset_id: i64) -> Result<Option<MediaInfo>, CoreError> {
    let row = conn
        .query_row(
            "SELECT width, height, duration_ms, codec, audio_codec, fps, bitrate, camera,
                    captured_at, playback, extra
             FROM media_metadata WHERE asset_id = ?1",
            [asset_id],
            |r| {
                let playback: Option<String> = r.get(9)?;
                let extra: Option<String> = r.get(10)?;
                Ok(MediaInfo {
                    width: r.get(0)?,
                    height: r.get(1)?,
                    duration_ms: r.get(2)?,
                    codec: r.get(3)?,
                    audio_codec: r.get(4)?,
                    fps: r.get(5)?,
                    bitrate: r.get(6)?,
                    camera: r.get(7)?,
                    captured_at: r.get(8)?,
                    playable: matches!(playback.as_deref(), Some("playable" | "conditional")),
                    extra: extra
                        .and_then(|j| serde_json::from_str::<BTreeMap<String, String>>(&j).ok())
                        .unwrap_or_default(),
                })
            },
        )
        .optional()?;
    Ok(row)
}
