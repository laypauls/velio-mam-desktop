//! 태그 — 목록, 일괄 추가/제거, 이름 변경, 삭제.

use rusqlite::{Connection, OptionalExtension, params};

use super::{
    MAX_TAGS_PER_CALL, TagView, UpdateTagsInput, UserData, bulk_priority, bump_assets, clean_ids,
    clean_name, json_ids, name_key, require_assets, returning_ids,
};
use crate::scan::{IngestError, loc_in_hidden_folder};

/// 숨김 폴더의 자산은 세지 않는다 (갤러리 기본 보기와 같은 기준).
const COUNT_SQL: &str = concat!(
    "(SELECT count(*) FROM asset_tags x
        JOIN media_assets a ON a.asset_id = x.asset_id AND a.status = 'active'
        WHERE x.tag_id = t.tag_id AND (NOT EXISTS (SELECT 1 FROM managed_folders WHERE hidden = 1)
          OR NOT EXISTS (SELECT 1 FROM file_locations l WHERE l.asset_id = x.asset_id AND l.detached_at IS NULL AND ",
    loc_in_hidden_folder!(),
    ")))"
);

fn map_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<TagView> {
    Ok(TagView {
        tag_id: r.get(0)?,
        name: r.get(1)?,
        asset_count: r.get(2)?,
    })
}

fn view(c: &Connection, tag_id: i64) -> Result<TagView, IngestError> {
    c.query_row(
        &format!("SELECT t.tag_id, t.name, {COUNT_SQL} FROM tags t WHERE t.tag_id = ?1"),
        [tag_id],
        map_view,
    )
    .optional()?
    .ok_or(IngestError::NotFound)
}

/// 태그가 붙은 자산 id (활성 여부 무관 — 색인에서 빠져야 하는 자산도 revision 은 올린다)
fn assets_of(c: &Connection, tag_id: i64) -> Result<Vec<i64>, IngestError> {
    returning_ids(
        c,
        "SELECT asset_id FROM asset_tags WHERE tag_id = ?1 ORDER BY asset_id",
        [tag_id],
    )
}

impl UserData {
    pub fn list_tags(&self) -> Result<Vec<TagView>, IngestError> {
        self.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT t.tag_id, t.name, {COUNT_SQL} FROM tags t ORDER BY t.name_key, t.tag_id"
            ))?;
            let rows = stmt.query_map([], map_view)?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// 선택한 자산들에 태그를 붙이고(`add`: 없으면 만든다) 뗀다(`remove`). 한 트랜잭션.
    /// 자산이 0개면 아무것도 하지 않는다 (태그도 만들지 않는다).
    pub fn update_asset_tags(&self, input: UpdateTagsInput) -> Result<(), IngestError> {
        let ids = clean_ids(&input.asset_ids)?;
        if input.add.len() > MAX_TAGS_PER_CALL || input.remove.len() > MAX_TAGS_PER_CALL {
            return Err(IngestError::InvalidInput("too many tags"));
        }
        let mut names: Vec<(String, String)> = Vec::new();
        for raw in &input.add {
            let name = clean_name(raw)?;
            let key = name_key(&name);
            if !names.iter().any(|(_, k)| *k == key) {
                names.push((name, key));
            }
        }
        if ids.is_empty() {
            return Ok(());
        }
        let remove = input.remove;
        self.write(move |tx, now| {
            require_assets(tx, &ids)?;
            let json = json_ids(&ids);
            let mut changed: Vec<i64> = Vec::new();
            for (name, key) in &names {
                let existing: Option<i64> = tx
                    .query_row("SELECT tag_id FROM tags WHERE name_key = ?1", [key], |r| {
                        r.get(0)
                    })
                    .optional()?;
                let tag_id = match existing {
                    Some(id) => id,
                    None => {
                        tx.execute(
                            "INSERT INTO tags (name, name_key) VALUES (?1, ?2)",
                            params![name, key],
                        )?;
                        tx.last_insert_rowid()
                    }
                };
                changed.extend(returning_ids(
                    tx,
                    "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id, created_at)
                     SELECT value, ?2, ?3 FROM json_each(?1) WHERE true RETURNING asset_id",
                    params![json, tag_id, now],
                )?);
            }
            for tag_id in &remove {
                changed.extend(returning_ids(
                    tx,
                    "DELETE FROM asset_tags WHERE tag_id = ?2
                       AND asset_id IN (SELECT value FROM json_each(?1)) RETURNING asset_id",
                    params![json, tag_id],
                )?);
            }
            changed.sort_unstable();
            changed.dedup();
            bump_assets(tx, now, &changed, 0)
        })
    }

    /// 이름 변경. 다른 태그와 이름(대소문자·NFC 정규화 기준)이 같아지면 `AlreadyRegistered`.
    /// 이 태그가 붙은 모든 자산의 revision 을 올려 검색에 반영한다.
    pub fn rename_tag(&self, tag_id: i64, name: &str) -> Result<TagView, IngestError> {
        let name = clean_name(name)?;
        let key = name_key(&name);
        self.write(move |tx, now| {
            let old_name: Option<String> = tx
                .query_row("SELECT name FROM tags WHERE tag_id = ?1", [tag_id], |r| {
                    r.get(0)
                })
                .optional()?;
            let Some(old_name) = old_name else {
                return Err(IngestError::NotFound);
            };
            let clash: Option<i64> = tx
                .query_row(
                    "SELECT tag_id FROM tags WHERE name_key = ?1 AND tag_id <> ?2",
                    params![key, tag_id],
                    |r| r.get(0),
                )
                .optional()?;
            if clash.is_some() {
                return Err(IngestError::AlreadyRegistered);
            }
            if old_name != name {
                tx.execute(
                    "UPDATE tags SET name = ?2, name_key = ?3 WHERE tag_id = ?1",
                    params![tag_id, name, key],
                )?;
                let affected = assets_of(tx, tag_id)?;
                bump_assets(tx, now, &affected, bulk_priority(affected.len()))?;
            }
            view(tx, tag_id)
        })
    }

    /// 태그를 지운다 (자산은 그대로, 붙어 있던 자산의 revision 증가).
    pub fn delete_tag(&self, tag_id: i64) -> Result<(), IngestError> {
        self.write(move |tx, now| {
            let exists: Option<i64> = tx
                .query_row("SELECT tag_id FROM tags WHERE tag_id = ?1", [tag_id], |r| {
                    r.get(0)
                })
                .optional()?;
            if exists.is_none() {
                return Err(IngestError::NotFound);
            }
            let affected = assets_of(tx, tag_id)?;
            tx.execute("DELETE FROM tags WHERE tag_id = ?1", [tag_id])?; // asset_tags 는 CASCADE
            bump_assets(tx, now, &affected, bulk_priority(affected.len()))
        })
    }
}
