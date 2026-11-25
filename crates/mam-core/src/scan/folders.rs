//! 관리 폴더 — 등록·조회·제거, 상태 계산.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;

use super::IngestError;
use super::media_types::MediaTypeSet;
use crate::keyrange::key_range;
use crate::volume::{VolumeInfo, VolumeKind};

/// `managed_folders` 한 행 + 소속 볼륨 정보.
#[derive(Debug, Clone)]
pub struct FolderRow {
    pub folder_id: i64,
    pub volume_id: i64,
    pub rel_root: String,
    pub path_key: String,
    pub display_path: String,
    pub enabled: bool,
    /// 검색·전체 보기에서 제외 (수집·분석은 계속)
    pub hidden: bool,
    /// 사용자 지정 표시 이름 (표시 전용, 없으면 None)
    pub alias: Option<String>,
    /// 스캔 대상 유형 (제외된 유형은 신규 등록하지 않는다)
    pub media_types: MediaTypeSet,
    /// 폴더 루트 기준 상대 경로('/' 구분)
    pub excluded: Vec<String>,
    pub last_full_scan_at: Option<i64>,
    pub volume_kind: VolumeKind,
    pub volume_label: Option<String>,
    pub mount_point: Option<String>,
    pub online: bool,
    /// 변경 감지 방식 ID (NULL = 자동). 값 검증은 감지기 레지스트리가 한다
    pub watch_mode: Option<String>,
    /// 감지기별 옵션 JSON 텍스트 (NULL = 기본값)
    pub watch_options: Option<String>,
    /// 재스캔 일정 정책 JSON 텍스트 (NULL = 전역 기본)
    pub rescan_policy: Option<String>,
}

impl FolderRow {
    /// 지금 기준 절대 경로 (볼륨의 현재 마운트 지점 + 상대 경로 — 드라이브 문자가 바뀌어도 따라간다).
    pub fn root_path(&self) -> PathBuf {
        match &self.mount_point {
            Some(m) if self.rel_root.is_empty() => PathBuf::from(m),
            Some(m) => PathBuf::from(m).join(&self.rel_root),
            None => PathBuf::from(&self.display_path),
        }
    }
}

const SELECT: &str = "SELECT f.folder_id, f.volume_id, f.rel_root, f.path_key, f.display_path,
        f.enabled, f.excluded, f.last_full_scan_at, v.kind, v.label, v.last_mount_point, v.availability, f.hidden, f.alias,
        f.scan_image, f.scan_video, f.scan_audio, f.watch_mode, f.watch_options, f.rescan_policy
     FROM managed_folders f JOIN volumes v ON v.volume_id = f.volume_id";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FolderRow> {
    let excluded: String = r.get(6)?;
    let kind: String = r.get(8)?;
    let availability: String = r.get(11)?;
    Ok(FolderRow {
        folder_id: r.get(0)?,
        volume_id: r.get(1)?,
        rel_root: r.get(2)?,
        path_key: r.get(3)?,
        display_path: r.get(4)?,
        enabled: r.get::<_, i64>(5)? != 0,
        hidden: r.get::<_, i64>(12)? != 0,
        alias: r.get(13)?,
        media_types: MediaTypeSet {
            image: r.get::<_, i64>(14)? != 0,
            video: r.get::<_, i64>(15)? != 0,
            audio: r.get::<_, i64>(16)? != 0,
        },
        excluded: serde_json::from_str(&excluded).unwrap_or_default(),
        last_full_scan_at: r.get(7)?,
        volume_kind: VolumeKind::parse(&kind).unwrap_or(VolumeKind::Fixed),
        volume_label: r.get(9)?,
        mount_point: r.get(10)?,
        online: availability == "online",
        watch_mode: r.get(17)?,
        watch_options: r.get(18)?,
        rescan_policy: r.get(19)?,
    })
}

pub fn load_folder(conn: &Connection, folder_id: i64) -> Result<FolderRow, IngestError> {
    conn.query_row(
        &format!("{SELECT} WHERE f.folder_id = ?1"),
        [folder_id],
        from_row,
    )
    .optional()?
    .ok_or(IngestError::NotFound)
}

pub fn load_all(conn: &Connection) -> Result<Vec<FolderRow>, IngestError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY f.folder_id"))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 폴더를 등록한다 (`volume_id` 는 이미 등록된 볼륨). 같은 폴더는 `AlreadyRegistered`,
/// 상위·하위 폴더와 겹치면 `NestedFolder` — 하위는 상위에 이미 포함되므로 거부한다. 대상 유형이 모두 꺼져 있으면 `InvalidInput`.
pub fn add_folder_row(
    conn: &mut Connection,
    now: i64,
    volume_id: i64,
    rel_root: &str,
    path_key: &str,
    display_path: &str,
    types: MediaTypeSet,
) -> Result<i64, IngestError> {
    if !types.any() {
        return Err(IngestError::InvalidInput("mediaTypes"));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_overlap(&tx, volume_id, path_key, None)?;
    tx.execute(
        "INSERT INTO managed_folders
           (volume_id, rel_root, path_key, display_path, created_at, scan_image, scan_video, scan_audio)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            volume_id,
            rel_root,
            path_key,
            display_path,
            now,
            types.image,
            types.video,
            types.audio
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(id)
}

/// 같은 볼륨의 다른 관리 폴더(`except` 제외)와 같거나 포함 관계인지 검사한다.
pub(super) fn check_overlap(
    conn: &Connection,
    volume_id: i64,
    path_key: &str,
    except: Option<i64>,
) -> Result<(), IngestError> {
    let mut stmt = conn.prepare(
        "SELECT path_key, enabled FROM managed_folders WHERE volume_id = ?1 AND folder_id IS NOT ?2",
    )?;
    let existing: Vec<(String, bool)> = stmt
        .query_map(params![volume_id, except], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (other, enabled) in &existing {
        let overlaps =
            other == path_key || is_inside(path_key, other) || is_inside(other, path_key);
        // 제거 중(`enabled = 0`)인 폴더와 겹치면 삭제가 끝날 때까지 기다려야 한다
        if overlaps && !enabled {
            return Err(IngestError::Busy);
        }
        if other == path_key {
            return Err(IngestError::AlreadyRegistered);
        }
        if is_inside(path_key, other) {
            return Err(IngestError::NestedFolder {
                inside_existing: true,
            });
        }
        if is_inside(other, path_key) {
            return Err(IngestError::NestedFolder {
                inside_existing: false,
            });
        }
    }
    Ok(())
}

/// `child` 가 `parent` 의 하위인가 (빈 `parent` = 볼륨 루트는 모든 것을 포함)
pub(super) fn is_inside(child: &str, parent: &str) -> bool {
    child != parent && (parent.is_empty() || child.starts_with(&format!("{parent}/")))
}

pub fn set_enabled(conn: &Connection, folder_id: i64, enabled: bool) -> Result<(), IngestError> {
    let n = conn.execute(
        "UPDATE managed_folders SET enabled = ?2 WHERE folder_id = ?1",
        params![folder_id, enabled],
    )?;
    if n == 0 {
        return Err(IngestError::NotFound);
    }
    Ok(())
}

/// 숨김 토글 (보기 속성 — 스캔·감시·분석에는 영향 없음). 없는 폴더·등록 해제 중(`enabled = 0`)은 `NotFound`.
/// 같은 값 재설정은 성공(멱등).
pub fn set_hidden(conn: &Connection, folder_id: i64, hidden: bool) -> Result<(), IngestError> {
    let n = conn.execute(
        "UPDATE managed_folders SET hidden = ?2 WHERE folder_id = ?1 AND enabled = 1",
        params![folder_id, hidden],
    )?;
    if n == 0 {
        return Err(IngestError::NotFound);
    }
    Ok(())
}

/// 별칭 최대 길이 (문자 수)
pub const MAX_ALIAS_CHARS: usize = 64;

/// 별칭 설정 (표시 전용 — outbox·revision 없음). 앞뒤 공백 제거, 빈 값은 NULL(해제), 64자 초과는 `InvalidInput`.
/// 없는 폴더·등록 해제 중은 `NotFound`. 같은 값 재설정은 성공(멱등).
pub fn set_alias(
    conn: &Connection,
    folder_id: i64,
    alias: Option<&str>,
) -> Result<(), IngestError> {
    let alias = alias.map(str::trim).filter(|s| !s.is_empty());
    if alias.is_some_and(|s| s.chars().count() > MAX_ALIAS_CHARS) {
        return Err(IngestError::InvalidInput("alias"));
    }
    let n = conn.execute(
        "UPDATE managed_folders SET alias = ?2 WHERE folder_id = ?1 AND enabled = 1",
        params![folder_id, alias],
    )?;
    if n == 0 {
        return Err(IngestError::NotFound);
    }
    Ok(())
}

/// 위치(`l`)가 숨김 관리 폴더 아래인가 - 볼륨이 같고 폴더 루트 키 범위 안(루트 `''` = 볼륨 전체).
/// 폴더 수는 적어 `managed_folders` 조회는 작다. 등록 해제 중(`enabled = 0`)인 폴더도 숨김으로 본다.
macro_rules! loc_in_hidden_folder {
    () => {
        "EXISTS (SELECT 1 FROM managed_folders hf WHERE hf.hidden = 1 AND hf.volume_id = l.volume_id
           AND (hf.path_key = '' OR (l.path_key >= hf.path_key || '/' AND l.path_key < hf.path_key || '0')))"
    };
}

/// `l` 위치가 숨김 폴더 밖인 조건. 숨김 폴더가 하나도 없으면 첫 항(비상관 서브쿼리, 1회 평가)에서 끝난다.
macro_rules! not_in_hidden_folder {
    () => {
        concat!(
            "(NOT EXISTS (SELECT 1 FROM managed_folders WHERE hidden = 1) OR NOT ",
            $crate::scan::loc_in_hidden_folder!(),
            ")"
        )
    };
}
pub(crate) use {loc_in_hidden_folder, not_in_hidden_folder};

/// 사이드바 "전체 미디어" 개수의 단일 정의 — 갤러리 기본 보기(오프라인 포함, 파일 없음 제외)와 같은 기준:
/// 활성 위치(`detached_at IS NULL`) + `presence = 'present'` + 활성 자산 + 숨김 폴더 밖.
/// 위치가 모두 detach 된 고아 자산(태그·AI 결과 보존용)은 어디에도 보이지 않으므로 세지 않는다.
/// `uq_file_locations_active_asset` 때문에 자산당 활성 위치는 최대 1행이라 중복 집계가 없다.
pub(crate) const TOTAL_COUNT_SQL: &str = concat!(
    "SELECT count(*)
 FROM file_locations l
 JOIN media_assets a ON a.asset_id = l.asset_id AND a.status = 'active'
 WHERE l.detached_at IS NULL AND l.presence = 'present' AND ",
    not_in_hidden_folder!()
);

/// 검색에서 제외할 폴더 id (`hidden = 1`; 등록 해제 중인 폴더도 포함 - detach 도중 문서가 다시 보이지 않게)
pub fn hidden_folder_ids(conn: &Connection) -> Result<Vec<u64>, IngestError> {
    let mut stmt = conn.prepare_cached(
        "SELECT folder_id FROM managed_folders WHERE hidden = 1 ORDER BY folder_id",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|v| v as u64)
        .collect())
}

/// 폴더 제거의 마지막 단계: 행을 지운다 (스캔 실행·오류·트리·작업은 CASCADE). 위치 detach 는 호출자가 먼저 끝낸다.
pub fn detach_folder(conn: &Connection, folder_id: i64) -> Result<(), IngestError> {
    conn.execute(
        "DELETE FROM managed_folders WHERE folder_id = ?1",
        [folder_id],
    )?;
    Ok(())
}

/// 폴더(`root_key`) 하위의 활성 위치 수 — 인덱스 범위 조회 (`uq_file_locations_active_path`).
pub fn count_under(conn: &Connection, volume_id: i64, root_key: &str) -> Result<i64, IngestError> {
    let (lo, hi) = key_range(root_key);
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM file_locations
         WHERE volume_id = ?1 AND detached_at IS NULL AND path_key >= ?2 AND path_key < ?3",
        params![volume_id, lo, hi],
        |r| r.get(0),
    )?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Availability {
    Online,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanState {
    Idle,
    Scanning,
    Paused,
    Failed,
}

/// `list_folders` 항목 (프론트 계약 `ManagedFolder`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedFolderView {
    pub folder_id: i64,
    pub path: String,
    pub volume_kind: VolumeKind,
    pub volume_label: Option<String>,
    pub availability: Availability,
    pub asset_count: i64,
    pub last_full_scan_at: Option<i64>,
    pub scan_state: ScanState,
    pub hidden: bool,
    pub alias: Option<String>,
    pub media_types: MediaTypeSet,
    /// 이 폴더의 디스크가 "꺼내기 준비" 상태 (이때 `availability` 는 `offline`)
    pub eject_prepared: bool,
    /// 꺼낼 수 있는 디스크의 폴더인가 (USB 등 외장, 네트워크·시스템·MAMData 볼륨 아님). false 면 꺼내기 메뉴를 보이지 않는다
    pub ejectable: bool,
    /// 변경 감지·재스캔 정책 필드 6개 (JSON 에서는 최상위로 펼쳐진다)
    #[serde(flatten)]
    pub detection: super::DetectionView,
}

/// 모든 폴더의 표시용 뷰. `job_status` = 폴더별 스캔 작업 상태, `paused` = 스캔 일시정지 여부,
/// `held_volumes` = "꺼내기 준비" 로 붙들린 볼륨.
pub fn list_folder_views(
    conn: &Connection,
    job_status: &[(i64, String)],
    scan_paused: bool,
    held_volumes: &std::collections::HashSet<i64>,
    ejectable_volumes: &std::collections::HashSet<i64>,
) -> Result<Vec<ManagedFolderView>, IngestError> {
    let latest: HashMap<i64, String> = {
        let mut stmt = conn.prepare(
            "SELECT folder_id, status FROM scan_runs
             WHERE run_id IN (SELECT MAX(run_id) FROM scan_runs GROUP BY folder_id)",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut out = Vec::new();
    for f in load_all(conn)?.into_iter().filter(|f| f.enabled) {
        let jobs: Vec<&str> = job_status
            .iter()
            .filter(|(id, _)| *id == f.folder_id)
            .map(|(_, s)| s.as_str())
            .collect();
        let scan_state = if jobs.contains(&"running") || jobs.contains(&"pending") {
            if scan_paused {
                ScanState::Paused
            } else {
                ScanState::Scanning
            }
        } else if jobs.contains(&"failed")
            || latest.get(&f.folder_id).is_some_and(|s| s == "failed")
        {
            ScanState::Failed
        } else {
            ScanState::Idle
        };
        out.push(ManagedFolderView {
            folder_id: f.folder_id,
            path: path_text(&f.root_path()),
            volume_kind: f.volume_kind,
            volume_label: f.volume_label.clone(),
            availability: if f.online {
                Availability::Online
            } else {
                Availability::Offline
            },
            asset_count: count_under(conn, f.volume_id, &f.path_key)?,
            last_full_scan_at: f.last_full_scan_at,
            scan_state,
            hidden: f.hidden,
            alias: f.alias.clone(),
            media_types: f.media_types,
            eject_prepared: held_volumes.contains(&f.volume_id),
            ejectable: ejectable_volumes.contains(&f.volume_id),
            detection: Default::default(),
        });
    }
    Ok(out)
}

pub(super) fn path_text(p: &Path) -> String {
    crate::volume::strip_verbatim(&p.to_string_lossy())
}

/// 등록 입력(절대 경로 + 볼륨)에서 폴더 행에 쓸 값을 만든다: (rel_root, path_key).
/// 볼륨 루트 자체(`E:\`)는 ('', '').
pub fn root_keys(info: &VolumeInfo, abs: &Path) -> Result<(String, String), IngestError> {
    match crate::volume::relative_path(&info.mount_point, abs) {
        Ok(rel) => {
            let key = mam_db::path_key(&rel).map_err(crate::CoreError::from)?;
            Ok((rel, key))
        }
        // 볼륨 루트 자체: relative_path 는 오류지만 폴더로는 유효하다
        Err(crate::CoreError::OutsideVolume { .. }) if is_same_dir(&info.mount_point, abs) => {
            Ok((String::new(), String::new()))
        }
        Err(e) => Err(e.into()),
    }
}

fn is_same_dir(mount: &str, abs: &Path) -> bool {
    let n = |s: &str| {
        crate::volume::strip_verbatim(s)
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    n(mount) == n(&abs.to_string_lossy())
}

impl std::ops::Deref for ManagedFolderView {
    type Target = super::DetectionView;
    fn deref(&self) -> &Self::Target {
        &self.detection
    }
}

impl std::ops::DerefMut for ManagedFolderView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.detection
    }
}
