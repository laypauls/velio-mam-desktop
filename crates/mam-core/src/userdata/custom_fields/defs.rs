//! 필드 정의 CRUD — 생성·이름 변경·순서·삭제·목록. 타입 변경 API 는 없다 (결정 4).

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use super::validate::MAX_METADATA_FIELDS;
use super::{FieldOption, MetadataField, ValueType, options};
use crate::scan::IngestError;
use crate::userdata::{UserData, clean_name, name_key, returning_ids};

impl UserData {
    /// 모든 필드를 표시 순서로 (선택지 포함).
    pub fn list_metadata_fields(&self) -> Result<Vec<MetadataField>, IngestError> {
        self.read(load_fields)
    }

    /// 필드를 만든다. 이름은 NFC + 소문자 기준으로 유일해야 한다(`AlreadyRegistered`), 최대 [`MAX_METADATA_FIELDS`] 개.
    /// `options` 는 enum/multi_enum 일 때만 허용되며 비어 있어도 된다(나중에 추가).
    pub fn create_metadata_field(
        &self,
        name: &str,
        value_type: ValueType,
        option_labels: &[String],
    ) -> Result<MetadataField, IngestError> {
        let name = clean_name(name)?;
        let key = name_key(&name);
        if !value_type.has_options() && !option_labels.is_empty() {
            return Err(IngestError::InvalidInput("options"));
        }
        let labels = options::clean_labels(option_labels)?;
        self.write(move |tx, now| {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM metadata_fields WHERE deleting_at IS NULL",
                [],
                |r| r.get(0),
            )?;
            if usize::try_from(count).is_ok_and(|c| c >= MAX_METADATA_FIELDS) {
                return Err(IngestError::InvalidInput("too many fields"));
            }
            if name_taken(tx, &key, None)? {
                return Err(IngestError::AlreadyRegistered);
            }
            let position: i64 = tx.query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM metadata_fields",
                [],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO metadata_fields (name, name_key, value_type, position, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![name, key, value_type.as_str(), position, now],
            )?;
            let field_id = tx.last_insert_rowid();
            for (i, (label, label_key)) in labels.iter().enumerate() {
                tx.execute(
                    "INSERT INTO metadata_field_options (field_id, label, label_key, position) VALUES (?1, ?2, ?3, ?4)",
                    params![field_id, label, label_key, i as i64],
                )?;
            }
            load_field(tx, field_id)
        })
    }

    /// 이름만 바꾼다 — 색인 키는 `f{field_id}` 라 재색인·revision 증가가 없다.
    pub fn rename_metadata_field(&self, field_id: i64, name: &str) -> Result<MetadataField, IngestError> {
        let name = clean_name(name)?;
        let key = name_key(&name);
        self.write(move |tx, now| {
            super::field_type(tx, field_id)?;
            if name_taken(tx, &key, Some(field_id))? {
                return Err(IngestError::AlreadyRegistered);
            }
            tx.execute(
                "UPDATE metadata_fields SET name = ?2, name_key = ?3, updated_at = ?4 WHERE field_id = ?1",
                params![field_id, name, key, now],
            )?;
            load_field(tx, field_id)
        })
    }

    /// 표시 순서를 바꾼다. `ordered` 는 현재 모든 필드 id 를 정확히 한 번씩 포함해야 한다. 재색인 없음.
    pub fn reorder_metadata_fields(&self, ordered: &[i64]) -> Result<(), IngestError> {
        let ordered = ordered.to_vec();
        self.write(move |tx, _| {
            let current = returning_ids(tx, "SELECT field_id FROM metadata_fields WHERE deleting_at IS NULL", [])?;
            if !is_permutation(&current, &ordered) {
                return Err(IngestError::InvalidInput("field order"));
            }
            for (i, id) in ordered.iter().enumerate() {
                tx.execute(
                    "UPDATE metadata_fields SET position = ?2 WHERE field_id = ?1",
                    params![id, i as i64],
                )?;
            }
            Ok(())
        })
    }

    /// 필드 삭제를 요청한다 (결정 17, 청크 삭제) — 짧은 트랜잭션 하나로 `deleting_at` 을 찍고 `name_key` 를
    /// sentinel 로 바꾼다. 이 순간부터 목록·값 읽기·값 쓰기·20개 상한·선택지 작업에서 없는 필드로 취급되고
    /// 이름은 바로 재사용할 수 있다. 값·선택지·필드 행의 실제 삭제와 자산별 revision/outbox(priority 1)는
    /// [`UserData::purge_pending_metadata_deletions`] 가 청크 단위로 이어서 한다.
    /// 값을 가진 자산 수(영향 예정)를 돌려준다.
    pub fn delete_metadata_field(&self, field_id: i64) -> Result<usize, IngestError> {
        self.write(move |tx, now| {
            super::field_type(tx, field_id)?;
            let affected: i64 = tx.query_row(
                "SELECT count(DISTINCT asset_id) FROM asset_metadata_values WHERE field_id = ?1",
                [field_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "UPDATE metadata_fields SET deleting_at = ?2, name_key = ?3, updated_at = ?2 WHERE field_id = ?1",
                params![field_id, now, super::deleting_key(field_id)],
            )?;
            Ok(usize::try_from(affected).unwrap_or(0))
        })
    }

    /// 이 필드에 값을 가진 자산 수 (삭제 확인 AlertDialog 용)
    pub fn metadata_field_asset_count(&self, field_id: i64) -> Result<i64, IngestError> {
        self.read(|c| {
            super::field_type(c, field_id)?;
            Ok(c.query_row(
                "SELECT count(DISTINCT asset_id) FROM asset_metadata_values WHERE field_id = ?1",
                [field_id],
                |r| r.get(0),
            )?)
        })
    }
}

fn is_permutation(current: &[i64], ordered: &[i64]) -> bool {
    let a: HashSet<i64> = current.iter().copied().collect();
    let b: HashSet<i64> = ordered.iter().copied().collect();
    ordered.len() == current.len() && b.len() == ordered.len() && a == b
}

fn name_taken(c: &Connection, key: &str, except: Option<i64>) -> Result<bool, IngestError> {
    let found: Option<i64> = c
        .query_row(
            "SELECT field_id FROM metadata_fields WHERE name_key = ?1 AND field_id <> ?2",
            params![key, except.unwrap_or(-1)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub(super) fn load_field(c: &Connection, field_id: i64) -> Result<MetadataField, IngestError> {
    load_fields(c)?
        .into_iter()
        .find(|f| f.field_id == field_id)
        .ok_or(IngestError::NotFound)
}

pub(crate) fn load_fields(c: &Connection) -> Result<Vec<MetadataField>, IngestError> {
    let mut opts: BTreeMap<i64, Vec<FieldOption>> = BTreeMap::new();
    let mut stmt = c.prepare_cached(
        "SELECT field_id, option_id, label, position FROM metadata_field_options
         WHERE deleting_at IS NULL ORDER BY field_id, position, option_id",
    )?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            FieldOption { option_id: r.get(1)?, label: r.get(2)?, position: r.get(3)? },
        ))
    })? {
        let (field_id, opt) = row?;
        opts.entry(field_id).or_default().push(opt);
    }
    let mut stmt = c.prepare_cached(
        "SELECT field_id, name, value_type, position, created_at, updated_at FROM metadata_fields
         WHERE deleting_at IS NULL ORDER BY position, field_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (field_id, name, vt, position, created_at, updated_at) = row?;
        let value_type = ValueType::parse(&vt).ok_or(IngestError::InvalidInput("stored value type"))?;
        out.push(MetadataField {
            field_id,
            name,
            value_type,
            position,
            options: opts.remove(&field_id).unwrap_or_default(),
            created_at,
            updated_at,
        });
    }
    Ok(out)
}
