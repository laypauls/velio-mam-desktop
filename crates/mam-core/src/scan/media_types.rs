//! 관리 폴더별 스캔 대상 유형 (이미지·동영상·음성) — 값 타입, 변경, 개수 집계.
//!
//! 제외된 유형의 파일은 신규 자산을 만들지 않는다. 이미 활성 위치가 있는 경로만 "봤다" 로 표시해 `missing` 오판을 막는다
//! (`scan::work`). 정리(purge)는 `Reconciler::detach_under_media` 가 담당한다.

use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::IngestError;
use crate::keyrange::key_range;
use crate::reconcile::MediaType;

/// 스캔 대상 유형 집합 (프론트 계약 `MediaTypeSet`). 최소 1개는 켜져 있어야 저장할 수 있다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTypeSet {
    pub image: bool,
    pub video: bool,
    pub audio: bool,
}

impl MediaTypeSet {
    pub const ALL: Self = Self {
        image: true,
        video: true,
        audio: true,
    };

    pub fn contains(self, t: MediaType) -> bool {
        match t {
            MediaType::Image => self.image,
            MediaType::Video => self.video,
            MediaType::Audio => self.audio,
        }
    }

    pub fn any(self) -> bool {
        self.image || self.video || self.audio
    }

    /// 꺼져 있는 유형의 `media_assets.media_type` 문자열들
    pub fn excluded(self) -> Vec<&'static str> {
        [
            (self.image, MediaType::Image),
            (self.video, MediaType::Video),
            (self.audio, MediaType::Audio),
        ]
        .into_iter()
        .filter(|(on, _)| !*on)
        .map(|(_, t)| t.as_str())
        .collect()
    }
}

impl Default for MediaTypeSet {
    fn default() -> Self {
        Self::ALL
    }
}

/// 폴더 하위 활성 위치의 유형별 개수 (프론트 계약 `MediaTypeCounts`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTypeCounts {
    pub image: i64,
    pub video: i64,
    pub audio: i64,
}

/// 대상 유형을 바꾼다. 모두 꺼짐은 `InvalidInput`, 없는 폴더·등록 해제 중은 `NotFound`.
///
/// 같은 트랜잭션에서 이 폴더의 진행·일시정지 중 스캔 실행(`scan_runs`)을 취소한다 — 이전 설정으로 만든 재개 지점(cursor)을
/// 이어받으면 새로 켠 유형이 이미 지나간 디렉터리에서 수집되지 않기 때문이다. 이전 설정으로 돌던 스캔은
/// `apply_chunk` 의 설정 대조에서 스스로 멈춘다.
pub fn set_media_types(
    conn: &mut Connection,
    now: i64,
    folder_id: i64,
    types: MediaTypeSet,
) -> Result<(), IngestError> {
    if !types.any() {
        return Err(IngestError::InvalidInput("mediaTypes"));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous = load_types(&tx, folder_id)?;
    tx.execute(
        "UPDATE managed_folders SET scan_image = ?2, scan_video = ?3, scan_audio = ?4 WHERE folder_id = ?1",
        params![folder_id, types.image, types.video, types.audio],
    )?;
    if previous != types {
        tx.execute(
            "UPDATE scan_runs SET status = 'cancelled', finished_at = ?2
             WHERE folder_id = ?1 AND status IN ('running', 'paused')",
            params![folder_id, now],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn load_types(conn: &Connection, folder_id: i64) -> Result<MediaTypeSet, IngestError> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT scan_image, scan_video, scan_audio FROM managed_folders WHERE folder_id = ?1 AND enabled = 1",
        [folder_id],
        |r| {
            Ok(MediaTypeSet {
                image: r.get::<_, i64>(0)? != 0,
                video: r.get::<_, i64>(1)? != 0,
                audio: r.get::<_, i64>(2)? != 0,
            })
        },
    )
    .optional()?
    .ok_or(IngestError::NotFound)
}

/// 폴더(`root_key`) 하위 활성 위치의 유형별 개수 — `uq_file_locations_active_path` 범위 + 자산 PK 조인.
pub fn count_by_type(
    conn: &Connection,
    volume_id: i64,
    root_key: &str,
) -> Result<MediaTypeCounts, IngestError> {
    let (lo, hi) = key_range(root_key);
    let mut stmt = conn.prepare_cached(COUNT_BY_TYPE_SQL)?;
    let rows = stmt.query_map(params![volume_id, lo, hi], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut out = MediaTypeCounts::default();
    for row in rows {
        let (t, n) = row?;
        match t.as_str() {
            "image" => out.image = n,
            "video" => out.video = n,
            "audio" => out.audio = n,
            _ => {}
        }
    }
    Ok(out)
}

pub const COUNT_BY_TYPE_SQL: &str = "SELECT a.media_type, COUNT(*)
     FROM file_locations l JOIN media_assets a ON a.asset_id = l.asset_id
     WHERE l.volume_id = ?1 AND l.detached_at IS NULL AND l.path_key >= ?2 AND l.path_key < ?3
     GROUP BY a.media_type";
