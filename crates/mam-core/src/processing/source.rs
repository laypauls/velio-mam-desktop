//! 처리 대상 자산의 원본 위치를 DB 에서 찾는다 (읽기 전용 경로 해석).

use std::path::PathBuf;

use mam_media::MediaKind;
use rusqlite::{Connection, OptionalExtension};

use crate::error::CoreError;

/// 작업 시작 시점의 자산 상태 (DB 값). 처리 결과를 쓸 때 이 값과 다시 대조한다.
#[derive(Debug, Clone)]
pub(crate) struct AssetSnapshot {
    pub kind: MediaKind,
    pub size: i64,
    pub modified_at: i64,
    pub hash_pending: bool,
    /// 전체 해시가 이미 있다 (`hash_state = done`) - `failed` 는 둘 다 false
    pub hash_done: bool,
}

/// 작업이 원본에 접근할 수 있는 상태인가.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// 자산이 없거나 `active` 가 아니거나 활성 위치가 없다 — 할 일이 없다
    Inactive,
    /// 볼륨이 오프라인이거나 파일이 없다(`missing`) — 처리하지 않고 나중에 다시 등록한다 (오프라인은 삭제가 아니다)
    Unavailable,
    Ready(AssetSnapshot, PathBuf),
}

/// 위치의 상대 경로를 마운트 지점에 붙인다. 빈 구성요소·`..`·`:`(대체 스트림) 이 있으면 `None` —
/// DB 값이어도 경로 탈출을 허용하지 않는다.
pub(crate) fn join_under_mount(mount: &str, rel_path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::from(mount);
    let mut any = false;
    for part in rel_path.split(['\\', '/']) {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return None;
        }
        out.push(part);
        any = true;
    }
    any.then_some(out)
}

/// 디스크의 파일이 작업 시작 시점의 DB 값(크기·수정 시각)과 같은가. 읽을 수 없으면 `true` —
/// 처리 단계가 자기 오류로 보고한다 (여기서 조용히 건너뛰지 않는다).
pub(crate) fn file_is_current(path: &std::path::Path, snap: &AssetSnapshot) -> bool {
    match crate::identity::basic_stat(path) {
        Ok((size, modified)) => {
            i64::try_from(size) == Ok(snap.size) && modified == snap.modified_at
        }
        Err(_) => true,
    }
}

fn kind_of(media_type: &str) -> Option<MediaKind> {
    match media_type {
        "image" => Some(MediaKind::Image),
        "video" => Some(MediaKind::Video),
        "audio" => Some(MediaKind::Audio),
        _ => None,
    }
}

pub(crate) fn load_source(conn: &Connection, asset_id: i64) -> Result<Source, CoreError> {
    type Row = (
        String,
        String,
        i64,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Option<Row> = conn
        .query_row(
            "SELECT a.media_type, a.status, a.file_size, a.modified_at, a.hash_state,
                    l.rel_path, l.presence, v.last_mount_point, v.availability
             FROM media_assets a
             LEFT JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
             LEFT JOIN volumes v ON v.volume_id = l.volume_id
             WHERE a.asset_id = ?1",
            [asset_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()?;
    let Some((media, status, size, modified_at, hash_state, rel, presence, mount, availability)) =
        row
    else {
        return Ok(Source::Inactive);
    };
    let (Some(kind), true, Some(rel)) = (kind_of(&media), status == "active", rel) else {
        return Ok(Source::Inactive);
    };
    if presence.as_deref() != Some("present") || availability.as_deref() != Some("online") {
        return Ok(Source::Unavailable);
    }
    let Some(path) = mount.and_then(|m| join_under_mount(&m, &rel)) else {
        return Ok(Source::Unavailable);
    };
    Ok(Source::Ready(
        AssetSnapshot {
            kind,
            size,
            modified_at,
            hash_pending: hash_state == "pending",
            hash_done: hash_state == "done",
        },
        path,
    ))
}

#[cfg(test)]
mod tests {
    //! 분류: Validation (경로 탈출 차단) / Happy
    use super::*;

    #[test]
    fn t_proc_path_01_joins_normal_relative_paths() {
        let p = join_under_mount(r"D:\", r"photos\2024/a.jpg").unwrap();
        assert_eq!(p, PathBuf::from(r"D:\photos\2024\a.jpg"));
    }

    #[test]
    fn t_proc_path_02_rejects_traversal_empty_and_stream_components() {
        for bad in [
            "",
            r"..\secret.jpg",
            r"a\..\b.jpg",
            r"a\\b.jpg",
            r"\abs.jpg",
            "a/./b.jpg",
            "a.jpg:stream",
            r"C:\x.jpg",
        ] {
            assert_eq!(join_under_mount(r"D:\", bad), None, "{bad}");
        }
    }
}
