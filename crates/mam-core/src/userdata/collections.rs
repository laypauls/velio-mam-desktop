//! 가상 컬렉션 — 원본 폴더와 무관한 자산 묶음. 삭제해도 자산은 보존한다.

use rusqlite::{Connection, OptionalExtension, params};

use super::collection_order;
use super::{
    CollectionView, UserData, bulk_priority, bump_assets, clean_ids, clean_name, dedup_keep_order,
    json_ids, name_key, require_assets, returning_ids,
};
use crate::scan::IngestError;

const COUNT_SQL: &str = "(SELECT count(*) FROM collection_assets x
        JOIN media_assets a ON a.asset_id = x.asset_id AND a.status = 'active'
        WHERE x.collection_id = c.collection_id)";

fn map_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionView> {
    Ok(CollectionView {
        collection_id: r.get(0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        asset_count: r.get(3)?,
    })
}

fn view(c: &Connection, id: i64) -> Result<CollectionView, IngestError> {
    c.query_row(
        &format!(
            "SELECT c.collection_id, c.name, c.created_at, {COUNT_SQL} FROM collections c
             WHERE c.collection_id = ?1"
        ),
        [id],
        map_view,
    )
    .optional()?
    .ok_or(IngestError::NotFound)
}

pub(super) fn require_collection(c: &Connection, id: i64) -> Result<(), IngestError> {
    let found: Option<i64> = c
        .query_row(
            "SELECT collection_id FROM collections WHERE collection_id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    found.map(|_| ()).ok_or(IngestError::NotFound)
}

fn members(c: &Connection, id: i64) -> Result<Vec<i64>, IngestError> {
    returning_ids(
        c,
        "SELECT asset_id FROM collection_assets WHERE collection_id = ?1 ORDER BY asset_id",
        [id],
    )
}

fn name_taken(c: &Connection, key: &str, except: Option<i64>) -> Result<bool, IngestError> {
    let found: Option<i64> = c
        .query_row(
            "SELECT collection_id FROM collections WHERE name_key = ?1 AND collection_id <> ?2",
            params![key, except.unwrap_or(-1)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

impl UserData {
    pub fn list_collections(&self) -> Result<Vec<CollectionView>, IngestError> {
        self.read(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT c.collection_id, c.name, c.created_at, {COUNT_SQL} FROM collections c
                 ORDER BY c.name_key, c.collection_id"
            ))?;
            let rows = stmt.query_map([], map_view)?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// 이름(대소문자·NFC 정규화 기준)이 같은 컬렉션이 있으면 `AlreadyRegistered`.
    pub fn create_collection(&self, name: &str) -> Result<CollectionView, IngestError> {
        let name = clean_name(name)?;
        let key = name_key(&name);
        self.write(move |tx, now| {
            if name_taken(tx, &key, None)? {
                return Err(IngestError::AlreadyRegistered);
            }
            tx.execute(
                "INSERT INTO collections (name, name_key, created_at) VALUES (?1, ?2, ?3)",
                params![name, key, now],
            )?;
            view(tx, tx.last_insert_rowid())
        })
    }

    /// 컬렉션 이름은 검색 문서에 들어가지 않으므로(id 만 색인) revision 은 올리지 않는다.
    pub fn rename_collection(&self, id: i64, name: &str) -> Result<CollectionView, IngestError> {
        let name = clean_name(name)?;
        let key = name_key(&name);
        self.write(move |tx, _| {
            require_collection(tx, id)?;
            if name_taken(tx, &key, Some(id))? {
                return Err(IngestError::AlreadyRegistered);
            }
            tx.execute(
                "UPDATE collections SET name = ?2, name_key = ?3 WHERE collection_id = ?1",
                params![id, name, key],
            )?;
            view(tx, id)
        })
    }

    /// 컬렉션과 소속 관계만 지운다 — 자산은 보존. 소속이었던 자산은 색인에서 컬렉션 id 가 빠지도록 revision 증가.
    pub fn delete_collection(&self, id: i64) -> Result<(), IngestError> {
        self.write(move |tx, now| {
            require_collection(tx, id)?;
            let affected = members(tx, id)?;
            tx.execute("DELETE FROM collections WHERE collection_id = ?1", [id])?; // 소속은 CASCADE
            bump_assets(tx, now, &affected, bulk_priority(affected.len()))
        })
    }

    /// 새 항목은 컬렉션 맨 뒤에 **요청한 순서대로** 붙는다. 이미 속한 항목은 위치를 유지한다 (멱등).
    pub fn add_to_collection(&self, id: i64, asset_ids: &[i64]) -> Result<(), IngestError> {
        let ids = dedup_keep_order(asset_ids)?;
        self.write(move |tx, now| {
            require_collection(tx, id)?;
            if ids.is_empty() {
                return Ok(());
            }
            require_assets(tx, &ids)?;
            let changed = collection_order::append(tx, id, &ids, now)?;
            bump_assets(tx, now, &changed, 0)
        })
    }

    pub fn remove_from_collection(&self, id: i64, asset_ids: &[i64]) -> Result<(), IngestError> {
        let ids = clean_ids(asset_ids)?;
        self.write(move |tx, now| {
            require_collection(tx, id)?;
            if ids.is_empty() {
                return Ok(());
            }
            let changed = returning_ids(
                tx,
                "DELETE FROM collection_assets WHERE collection_id = ?2
                   AND asset_id IN (SELECT value FROM json_each(?1)) RETURNING asset_id",
                params![json_ids(&ids), id],
            )?;
            bump_assets(tx, now, &changed, 0)
        })
    }
}
