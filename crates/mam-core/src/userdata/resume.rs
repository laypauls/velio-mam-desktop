//! 이어보기 위치 (`asset_resume_positions`, V24) — 자산당 마지막 재생 위치 1건.
//!
//! 검색 색인 대상이 아니므로 자산 `revision`·`search_outbox` 를 건드리지 않는다. 원본 미디어와 무관한 MAM 메타데이터라
//! 원본 파일은 읽지도 쓰지도 않는다(초기화에 비밀번호 불필요). 폴더 소속은 자산의 **활성 위치**(`volume_id` +
//! `path_key` 범위, `purge::range`·`purge::impact` 와 같은 `key_range`)로 판정한다.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::{UserData, json_ids};
use crate::keyrange::key_range;
use crate::scan::IngestError;

/// 시작 후 이 위치(ms) 미만은 기억하지 않는다 (D3)
pub const MIN_POSITION_MS: i64 = 5_000;
/// 끝에서 이 비율(%) 이내면 '다 봄' (D3)
const END_PERCENT: i64 = 5;
/// 끝에서 이 시간(ms) 이내면 '다 봄' — 비율과 큰 쪽을 쓴다 (D3)
const END_MARGIN_MS: i64 = 15_000;
/// 한 SQL 문에 넘기는 자산 id 수 (json_each 한 파라미터)
const ID_CHUNK: usize = 5_000;

/// 계약 `ResumePosition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumePosition {
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
}

/// 계약 `ResumeSaveOutcome` — `"saved" | "cleared" | "skipped"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeSaveOutcome {
    Saved,
    Cleared,
    Skipped,
}

/// 계약 `ResumeStats.folders[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderResumeCount {
    pub folder_id: i64,
    pub count: i64,
    /// 폴더별 이어보기 사용 여부 (`managed_folders.resume_enabled`, V25)
    pub enabled: bool,
}

/// 계약 `ResumeStats` — `total` 은 테이블 전체 행 수(어느 폴더에도 속하지 않는 기록 포함).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeStats {
    pub folders: Vec<FolderResumeCount>,
    pub total: i64,
}

/// 폴더 범위 안 활성 위치의 자산 중 기록이 있는 것 — 기록 테이블이 구동 쪽(비용 ∝ 기록 수)이고
/// 활성 자산 부분 인덱스(`uq_file_locations_active_asset`)로 점 조회한다.
pub const FOLDER_SCOPE_SQL: &str = "SELECT r.asset_id FROM asset_resume_positions r
 CROSS JOIN file_locations l ON l.asset_id = r.asset_id AND l.detached_at IS NULL
 WHERE l.volume_id = ?1 AND l.path_key >= ?2 AND l.path_key < ?3";

/// 선택한 자산 중 기록이 있는 것 (json_each 가 구동, PK 점 조회)
pub const COUNT_ASSETS_SQL: &str = "SELECT count(*) FROM asset_resume_positions
 WHERE asset_id IN (SELECT value FROM json_each(?1))";

/// '다 봄' 판정 — `position >= duration - max(duration*5%, 15s)`.
fn is_finished(position_ms: i64, duration_ms: i64) -> bool {
    let margin = (duration_ms.saturating_mul(END_PERCENT) / 100).max(END_MARGIN_MS);
    position_ms >= duration_ms.saturating_sub(margin)
}

/// 저장 판정 결과 (순수).
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// 설정 꺼짐 — 기존 기록 유지
    Disabled,
    /// 너무 짧은(또는 길이 미상) 콘텐츠 — 기존 기록 삭제
    TooShort,
    /// 시작 부분 또는 다 봄 — 기존 기록 삭제
    Clear,
    Save,
}

fn judge(
    enabled: bool,
    min_duration_sec: u32,
    position_ms: i64,
    duration_ms: Option<i64>,
) -> Verdict {
    if !enabled {
        return Verdict::Disabled;
    }
    let min_ms = i64::from(min_duration_sec) * 1000;
    if min_ms > 0 && duration_ms.is_none_or(|d| d < min_ms) {
        return Verdict::TooShort;
    }
    if position_ms < MIN_POSITION_MS {
        return Verdict::Clear;
    }
    if duration_ms.is_some_and(|d| is_finished(position_ms, d)) {
        return Verdict::Clear;
    }
    Verdict::Save
}

fn asset_exists(c: &Connection, asset_id: i64) -> Result<bool, IngestError> {
    let found: Option<i64> = c
        .query_row(
            "SELECT asset_id FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// 중복 제거한 id 를 `ID_CHUNK` 단위 JSON 배열로 나눈다 (호출당 상한 없음 — 전체 선택도 처리).
fn id_chunks(ids: &[i64]) -> Vec<String> {
    let mut v = ids.to_vec();
    v.sort_unstable();
    v.dedup();
    v.chunks(ID_CHUNK).map(json_ids).collect()
}

fn folder_range(c: &Connection, folder_id: i64) -> Result<(i64, String, String), IngestError> {
    let row: Option<(i64, String)> = c
        .query_row(
            "SELECT volume_id, path_key FROM managed_folders WHERE folder_id = ?1 AND enabled = 1",
            [folder_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (volume_id, root_key) = row.ok_or(IngestError::NotFound)?;
    let (lo, hi) = key_range(&root_key);
    Ok((volume_id, lo, hi))
}

/// 자산의 소속 폴더가 이어보기 사용 중인가. 활성 위치가 없거나 어느 폴더에도 속하지 않으면 `true`(전역 규칙만 적용).
/// 폴더끼리 겹치지 않으므로 '사용 안 함' 폴더 중 위치 키를 포함하는 것이 있으면 꺼진 것이다.
/// 활성 위치는 부분 인덱스 점 조회, 폴더는 볼륨 내 꺼진 폴더만(소수) 훑는다.
fn asset_folder_resume_enabled(c: &Connection, asset_id: i64) -> Result<bool, IngestError> {
    let loc: Option<(i64, String)> = c
        .query_row(
            "SELECT volume_id, path_key FROM file_locations WHERE asset_id = ?1 AND detached_at IS NULL",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((volume_id, path_key)) = loc else {
        return Ok(true);
    };
    let mut stmt = c.prepare_cached(
        "SELECT path_key FROM managed_folders WHERE volume_id = ?1 AND enabled = 1 AND resume_enabled = 0",
    )?;
    let roots = stmt.query_map([volume_id], |r| r.get::<_, String>(0))?;
    for root in roots {
        let (lo, hi) = key_range(&root?);
        if path_key >= lo && path_key < hi {
            return Ok(false);
        }
    }
    Ok(true)
}

impl UserData {
    /// 저장된 위치. 없는 자산·기록 없음은 `None`. 소속 폴더의 이어보기가 꺼져 있으면 기록이 있어도 `None`
    /// (기록은 유지). 전역 `resumeEnabled` 는 호출자가 설정으로 판단한다.
    pub fn resume_get(&self, asset_id: i64) -> Result<Option<ResumePosition>, IngestError> {
        self.read(|c| {
            let pos = c
                .query_row(
                    "SELECT position_ms, duration_ms FROM asset_resume_positions WHERE asset_id = ?1",
                    [asset_id],
                    |r| {
                        Ok(ResumePosition {
                            position_ms: r.get(0)?,
                            duration_ms: r.get(1)?,
                        })
                    },
                )
                .optional()?;
            match pos {
                Some(_) if !asset_folder_resume_enabled(c, asset_id)? => Ok(None),
                other => Ok(other),
            }
        })
    }

    /// 관리 폴더의 이어보기 사용 여부 설정. 기록은 건드리지 않는다. 없는·등록 해제 중인 폴더는 `NotFound`.
    pub fn resume_set_folder_enabled(&self, folder_id: i64, enabled: bool) -> Result<(), IngestError> {
        self.write(move |tx, _now| {
            let n = tx.execute(
                "UPDATE managed_folders SET resume_enabled = ?2 WHERE folder_id = ?1 AND enabled = 1",
                params![folder_id, enabled],
            )?;
            if n == 0 {
                return Err(IngestError::NotFound);
            }
            Ok(())
        })
    }

    /// 위치 저장 + 판정 (설정 `resumeEnabled`·`resumeMinDurationSec`, 규칙은 `judge`). 멱등 upsert.
    /// 음수 위치는 `invalid_input("positionMs")`, 없는 자산은 `NotFound`. `duration_ms <= 0` 은 길이 미상으로 본다.
    /// `updated_at` 은 주입 시계(`UserData` 의 clock)를 쓴다.
    pub fn resume_save(
        &self,
        asset_id: i64,
        position_ms: i64,
        duration_ms: Option<i64>,
    ) -> Result<ResumeSaveOutcome, IngestError> {
        if position_ms < 0 {
            return Err(IngestError::InvalidInput("positionMs"));
        }
        let duration_ms = duration_ms.filter(|d| *d > 0);
        self.write(move |tx, now| {
            if !asset_exists(tx, asset_id)? {
                return Err(IngestError::NotFound);
            }
            let (settings, _) = mam_db::settings::load(tx)?;
            // 전역이 꺼져 있으면 폴더 조회는 불필요. 폴더가 꺼져 있어도 같은 Disabled(기존 기록 유지)
            let enabled = settings.resume_enabled && asset_folder_resume_enabled(tx, asset_id)?;
            let verdict = judge(
                enabled,
                settings.resume_min_duration_sec,
                position_ms,
                duration_ms,
            );
            let delete = || {
                tx.execute(
                    "DELETE FROM asset_resume_positions WHERE asset_id = ?1",
                    [asset_id],
                )
            };
            match verdict {
                Verdict::Disabled => Ok(ResumeSaveOutcome::Skipped),
                Verdict::TooShort => {
                    delete()?;
                    Ok(ResumeSaveOutcome::Skipped)
                }
                Verdict::Clear => {
                    delete()?;
                    Ok(ResumeSaveOutcome::Cleared)
                }
                Verdict::Save => {
                    tx.execute(
                        "INSERT INTO asset_resume_positions (asset_id, position_ms, duration_ms, updated_at)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT (asset_id) DO UPDATE SET
                           position_ms = excluded.position_ms,
                           duration_ms = excluded.duration_ms,
                           updated_at = excluded.updated_at",
                        params![asset_id, position_ms, duration_ms, now],
                    )?;
                    Ok(ResumeSaveOutcome::Saved)
                }
            }
        })
    }

    /// 선택한 자산 중 기록이 있는 수 (없는 id 는 세지 않는다).
    pub fn resume_count_assets(&self, asset_ids: &[i64]) -> Result<i64, IngestError> {
        let chunks = id_chunks(asset_ids);
        self.read(|c| {
            let mut stmt = c.prepare_cached(COUNT_ASSETS_SQL)?;
            let mut total = 0i64;
            for json in &chunks {
                total += stmt.query_row([json], |r| r.get::<_, i64>(0))?;
            }
            Ok(total)
        })
    }

    /// 선택한 자산의 기록 삭제 (한 트랜잭션). 삭제 건수. 없는 id 는 무시한다.
    pub fn resume_clear_assets(&self, asset_ids: &[i64]) -> Result<i64, IngestError> {
        let chunks = id_chunks(asset_ids);
        self.write(move |tx, _now| {
            let mut stmt = tx.prepare_cached(
                "DELETE FROM asset_resume_positions WHERE asset_id IN (SELECT value FROM json_each(?1))",
            )?;
            let mut n = 0usize;
            for json in &chunks {
                n += stmt.execute([json])?;
            }
            Ok(n as i64)
        })
    }

    /// 관리 폴더 범위(활성 위치 기준)의 기록 삭제. 없는·등록 해제 중인 폴더는 `NotFound`.
    pub fn resume_clear_folder(&self, folder_id: i64) -> Result<i64, IngestError> {
        self.write(move |tx, _now| {
            let (volume_id, lo, hi) = folder_range(tx, folder_id)?;
            let n = tx.execute(
                &format!("DELETE FROM asset_resume_positions WHERE asset_id IN ({FOLDER_SCOPE_SQL})"),
                params![volume_id, lo, hi],
            )?;
            Ok(n as i64)
        })
    }

    /// 모든 기록 삭제 (소속 없는 기록 포함). 삭제 건수.
    pub fn resume_clear_all(&self) -> Result<i64, IngestError> {
        self.write(|tx, _now| Ok(tx.execute("DELETE FROM asset_resume_positions", [])? as i64))
    }

    /// 관리 폴더별 기록 수(0건 포함, 폴더 목록과 같은 등록 중 폴더·`folder_id` 순) + 전체 행 수.
    pub fn resume_stats(&self) -> Result<ResumeStats, IngestError> {
        self.read(|c| {
            let roots: Vec<(i64, i64, String, bool)> = {
                let mut stmt = c.prepare(
                    "SELECT folder_id, volume_id, path_key, resume_enabled FROM managed_folders WHERE enabled = 1 ORDER BY folder_id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            let mut folders = Vec::with_capacity(roots.len());
            for (folder_id, volume_id, root_key, enabled) in roots {
                let (lo, hi) = key_range(&root_key);
                let count = c.query_row(
                    &format!("SELECT count(*) FROM ({FOLDER_SCOPE_SQL})"),
                    params![volume_id, lo, hi],
                    |r| r.get(0),
                )?;
                folders.push(FolderResumeCount { folder_id, count, enabled });
            }
            let total = c.query_row("SELECT count(*) FROM asset_resume_positions", [], |r| r.get(0))?;
            Ok(ResumeStats { folders, total })
        })
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Boundary / Business rule (판정 순수 함수)
    use super::*;

    #[test]
    fn t_resume_judge_01_order_and_boundaries() {
        // 꺼짐이 모두 우선
        assert_eq!(judge(false, 180, 60_000, Some(600_000)), Verdict::Disabled);
        // 최소 길이: 179_999 < 180_000 짧음, 180_000 통과
        assert_eq!(judge(true, 180, 60_000, Some(179_999)), Verdict::TooShort);
        assert_eq!(judge(true, 180, 60_000, Some(180_000)), Verdict::Save);
        // 길이 미상: 최소 길이 > 0 이면 짧음, 0 이면 위치 규칙만
        assert_eq!(judge(true, 180, 60_000, None), Verdict::TooShort);
        assert_eq!(judge(true, 0, 60_000, None), Verdict::Save);
        assert_eq!(judge(true, 0, 4_999, None), Verdict::Clear);
        // 5초 경계
        assert_eq!(judge(true, 0, 4_999, Some(600_000)), Verdict::Clear);
        assert_eq!(judge(true, 0, 5_000, Some(600_000)), Verdict::Save);
        // 끝 경계: 600s → 5% = 30s > 15s, 임계 570_000
        assert_eq!(judge(true, 0, 569_999, Some(600_000)), Verdict::Save);
        assert_eq!(judge(true, 0, 570_000, Some(600_000)), Verdict::Clear);
        // 짧은 영상(200s): 5% = 10s < 15s → 임계 185_000
        assert_eq!(judge(true, 0, 184_999, Some(200_000)), Verdict::Save);
        assert_eq!(judge(true, 0, 185_000, Some(200_000)), Verdict::Clear);
    }
}
