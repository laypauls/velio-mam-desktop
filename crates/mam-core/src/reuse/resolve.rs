//! 자산 id -> 원본 절대 경로·접근 상태 (+ 내보내기 매니페스트용 메타). 배치 질의 1회 (N+1 없음).

use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, params};

use crate::scan::IngestError;
use crate::userdata::json_ids;

/// 원본 접근 상태
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// 경로가 만들어졌다 (디스크 존재 여부는 호출자가 확인한다)
    Ready(PathBuf),
    /// 볼륨 연결 끊김 — 삭제가 아니다
    Offline,
    /// 자산·위치가 없거나, 볼륨은 연결됐는데 파일이 없다
    Missing,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub asset_id: i64,
    pub access: Access,
    pub tags: Vec<String>,
    pub memo: String,
    /// UTC Unix ms
    pub captured_at: Option<i64>,
}

/// `?1` = 요청 id JSON 배열. 요청 순서·중복 없이 `json_each` 가 구동 테이블.
const SQL: &str = "SELECT j.value, a.asset_id, a.status, l.rel_path, l.presence,
        v.last_mount_point, v.availability,
        (SELECT group_concat(t.name, char(31)) FROM asset_tags x
           JOIN tags t ON t.tag_id = x.tag_id WHERE x.asset_id = a.asset_id),
        u.memo, m.captured_at
     FROM json_each(?1) j
     LEFT JOIN media_assets a ON a.asset_id = j.value
     LEFT JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
     LEFT JOIN volumes v ON v.volume_id = l.volume_id
     LEFT JOIN asset_user_data u ON u.asset_id = a.asset_id
     LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
     ORDER BY j.key";

pub const RESOLVE_SQL: &str = SQL;

/// 요청한 id 마다 결과 1건 (순서 유지). 알 수 없는 id 는 `Missing`.
pub fn resolve(conn: &Connection, ids: &[i64]) -> Result<Vec<Resolved>, IngestError> {
    let mut stmt = conn.prepare_cached(SQL)?;
    let rows = stmt.query_map(params![json_ids(ids)], |r| {
        let requested: i64 = r.get(0)?;
        let asset_id: Option<i64> = r.get(1)?;
        let status: Option<String> = r.get(2)?;
        let rel: Option<String> = r.get(3)?;
        let presence: Option<String> = r.get(4)?;
        let mount: Option<String> = r.get(5)?;
        let availability: Option<String> = r.get(6)?;
        let tags: Option<String> = r.get(7)?;
        let access = match (asset_id, status.as_deref(), rel, availability.as_deref()) {
            (None, ..) | (_, Some("deleted"), ..) | (_, _, None, _) => Access::Missing,
            (_, _, Some(_), Some("offline")) => Access::Offline,
            (_, _, Some(rel), _) => match (mount, presence.as_deref()) {
                (None, _) => Access::Offline,
                (Some(_), Some("missing")) => Access::Missing,
                (Some(mount), _) => join_rel(&mount, &rel).map_or(Access::Missing, Access::Ready),
            },
        };
        Ok(Resolved {
            asset_id: requested,
            access,
            tags: tags
                .map(|t| t.split('\u{1f}').map(str::to_string).collect())
                .unwrap_or_default(),
            memo: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
            captured_at: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 마운트 지점 + 상대 경로. DB 값도 신뢰하지 않는다 — `..`·드라이브·대체 스트림·예약 문자가 있으면 거부.
fn join_rel(mount: &str, rel: &str) -> Option<PathBuf> {
    let mut out = PathBuf::from(crate::volume::strip_verbatim(mount));
    let mut any = false;
    for seg in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
        if seg == "."
            || seg == ".."
            || seg.contains([':', '<', '>', '"', '|', '?', '*', '\0'])
            || seg.ends_with(' ')
        {
            return None;
        }
        out.push(seg);
        any = true;
    }
    (any && out.components().all(|c| !matches!(c, Component::ParentDir))).then_some(out)
}

/// 기본 프로그램으로 여는 것이 위험한 확장자 (스캐너는 미디어 확장자만 등록하지만 방어한다)
pub fn is_executable_name(path: &Path) -> bool {
    const DENY: &[&str] = &[
        "exe", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi",
        "msp", "scr", "lnk", "url", "hta", "cpl", "reg", "jar", "dll", "pif", "appx",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| DENY.contains(&e.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    // Categories: Validation (경로 구성 거부) / Boundary (구분자 혼용)
    use super::*;

    #[test]
    fn t_reuse_08_join_rel_rejects_traversal_and_reserved_names() {
        // Arrange / Act / Assert
        assert_eq!(
            join_rel(r"D:\media", "a/b\\c.jpg"),
            Some(PathBuf::from(r"D:\media").join("a").join("b").join("c.jpg"))
        );
        for bad in [
            "../x.jpg",
            "a/../../x.jpg",
            "C:/x.jpg",
            "a/b.jpg:stream",
            "",
            "a/*.jpg",
        ] {
            assert_eq!(join_rel(r"D:\media", bad), None, "{bad}");
        }
    }

    #[test]
    fn t_reuse_09_executable_extensions_are_flagged() {
        assert!(is_executable_name(Path::new(r"D:\a\x.EXE")));
        assert!(is_executable_name(Path::new("x.lnk")));
        assert!(!is_executable_name(Path::new("x.jpg")));
        assert!(!is_executable_name(Path::new("noext")));
    }
}
