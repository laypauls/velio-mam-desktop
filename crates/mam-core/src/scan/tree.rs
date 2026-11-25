//! 폴더 탐색 트리(지연 로딩) — 스캐너가 유지하는 `folder_dirs` 에서 한 단계씩 읽는다.
//! 자산 수는 `file_locations` 의 경로 키 범위 조회(커버링 인덱스)로 세므로 감시 이벤트로 바뀐 값도 항상 정확하다.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::IngestError;
use super::folders::{FolderRow, count_under};
use crate::keyrange::join_key;

/// `list_subfolders` 항목 (프론트 계약 `FolderNode`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    /// 폴더 루트 기준 상대 경로, `/` 구분
    pub rel_path: String,
    pub name: String,
    pub has_children: bool,
    /// 하위 폴더 포함 자산 수
    pub asset_count: i64,
}

/// `rel_path` 의 바로 아래 폴더들 (`None`/빈 문자열 = 폴더 루트 바로 아래). 이름 순.
pub fn list_subfolders(
    conn: &Connection,
    folder: &FolderRow,
    rel_path: Option<&str>,
) -> Result<Vec<FolderNode>, IngestError> {
    let parent_key = match rel_path.map(str::trim).filter(|p| !p.is_empty()) {
        None => String::new(),
        Some(p) => {
            if p.split(['/', '\\']).any(|s| s == "..") || p.contains('\0') {
                return Err(IngestError::InvalidInput("relPath"));
            }
            let key = mam_db::path_key(p).map_err(|_| IngestError::InvalidInput("relPath"))?;
            let known = conn
                .query_row(
                    "SELECT 1 FROM folder_dirs WHERE folder_id = ?1 AND dir_key = ?2",
                    params![folder.folder_id, key],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !known {
                return Err(IngestError::NotFound);
            }
            key
        }
    };
    let mut stmt = conn.prepare(
        "SELECT d.dir_key, d.name, d.rel_path,
                EXISTS (SELECT 1 FROM folder_dirs c INDEXED BY idx_folder_dirs_parent
                        WHERE c.folder_id = d.folder_id AND c.parent_key = d.dir_key)
         FROM folder_dirs d INDEXED BY idx_folder_dirs_parent
         WHERE d.folder_id = ?1 AND d.parent_key = ?2
         ORDER BY d.dir_key",
    )?;
    let rows = stmt.query_map(params![folder.folder_id, parent_key], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, bool>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (dir_key, name, rel_path, has_children) = row?;
        out.push(FolderNode {
            asset_count: count_under(
                conn,
                folder.volume_id,
                &join_key(&folder.path_key, &dir_key),
            )?,
            rel_path,
            name,
            has_children,
        });
    }
    Ok(out)
}
