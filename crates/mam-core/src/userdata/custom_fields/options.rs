//! enum / multi_enum 선택지 — 추가·이름 변경·순서·삭제·사용 건수.
//! 선택지 삭제만 재색인을 만든다 (이름·순서 변경은 색인에 `option_id` 만 있으므로 없음).

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use super::FieldOption;
use crate::scan::IngestError;
use crate::userdata::{UserData, clean_name, name_key, returning_ids};

/// 선택지 이름 정리 + 같은 목록 안의 중복(`label_key` 기준) 거부. `(label, label_key)` 를 돌려준다.
pub(super) fn clean_labels(raw: &[String]) -> Result<Vec<(String, String)>, IngestError> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for r in raw {
        let label = clean_name(r)?;
        let key = name_key(&label);
        if !seen.insert(key.clone()) {
            return Err(IngestError::AlreadyRegistered);
        }
        out.push((label, key));
    }
    Ok(out)
}

/// 선택지가 속한 필드 id. 없으면 `NotFound`.
fn option_field(c: &Connection, option_id: i64) -> Result<i64, IngestError> {
    c.query_row(
        "SELECT o.field_id FROM metadata_field_options o JOIN metadata_fields f ON f.field_id = o.field_id
         WHERE o.option_id = ?1 AND o.deleting_at IS NULL AND f.deleting_at IS NULL",
        [option_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(IngestError::NotFound)
}

fn require_option_field(c: &Connection, field_id: i64) -> Result<(), IngestError> {
    if !super::field_type(c, field_id)?.has_options() {
        return Err(IngestError::InvalidInput("field has no options"));
    }
    Ok(())
}

fn label_taken(c: &Connection, field_id: i64, key: &str, except: Option<i64>) -> Result<bool, IngestError> {
    let found: Option<i64> = c
        .query_row(
            "SELECT option_id FROM metadata_field_options WHERE field_id = ?1 AND label_key = ?2 AND option_id <> ?3",
            params![field_id, key, except.unwrap_or(-1)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn load_option(c: &Connection, option_id: i64) -> Result<FieldOption, IngestError> {
    Ok(c.query_row(
        "SELECT option_id, label, position FROM metadata_field_options WHERE option_id = ?1 AND deleting_at IS NULL",
        [option_id],
        |r| Ok(FieldOption { option_id: r.get(0)?, label: r.get(1)?, position: r.get(2)? }),
    )?)
}

impl UserData {
    /// 선택지를 맨 뒤에 추가한다. 같은 필드 안에서 이름(NFC + 소문자)이 겹치면 `AlreadyRegistered`.
    pub fn add_metadata_option(&self, field_id: i64, label: &str) -> Result<FieldOption, IngestError> {
        let label = clean_name(label)?;
        let key = name_key(&label);
        self.write(move |tx, _| {
            require_option_field(tx, field_id)?;
            if label_taken(tx, field_id, &key, None)? {
                return Err(IngestError::AlreadyRegistered);
            }
            let position: i64 = tx.query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM metadata_field_options WHERE field_id = ?1",
                [field_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO metadata_field_options (field_id, label, label_key, position) VALUES (?1, ?2, ?3, ?4)",
                params![field_id, label, key, position],
            )?;
            load_option(tx, tx.last_insert_rowid())
        })
    }

    /// 선택지 이름을 바꾼다 — 재색인 없음.
    pub fn rename_metadata_option(&self, option_id: i64, label: &str) -> Result<FieldOption, IngestError> {
        let label = clean_name(label)?;
        let key = name_key(&label);
        self.write(move |tx, _| {
            let field_id = option_field(tx, option_id)?;
            if label_taken(tx, field_id, &key, Some(option_id))? {
                return Err(IngestError::AlreadyRegistered);
            }
            tx.execute(
                "UPDATE metadata_field_options SET label = ?2, label_key = ?3 WHERE option_id = ?1",
                params![option_id, label, key],
            )?;
            load_option(tx, option_id)
        })
    }

    /// 선택지 순서를 바꾼다. `ordered` 는 그 필드의 모든 option_id 를 정확히 한 번씩 포함해야 한다. 재색인 없음.
    pub fn reorder_metadata_options(&self, field_id: i64, ordered: &[i64]) -> Result<(), IngestError> {
        let ordered = ordered.to_vec();
        self.write(move |tx, _| {
            require_option_field(tx, field_id)?;
            let current = returning_ids(
                tx,
                "SELECT option_id FROM metadata_field_options WHERE field_id = ?1 AND deleting_at IS NULL",
                [field_id],
            )?;
            let cur: HashSet<i64> = current.iter().copied().collect();
            let new: HashSet<i64> = ordered.iter().copied().collect();
            if ordered.len() != current.len() || new.len() != ordered.len() || cur != new {
                return Err(IngestError::InvalidInput("option order"));
            }
            for (i, id) in ordered.iter().enumerate() {
                tx.execute(
                    "UPDATE metadata_field_options SET position = ?2 WHERE option_id = ?1",
                    params![id, i as i64],
                )?;
            }
            Ok(())
        })
    }

    /// 선택지 삭제를 요청한다 (결정 13, 청크 삭제) — 짧은 트랜잭션 하나로 `deleting_at` 을 찍고 `label_key` 를
    /// sentinel 로 바꾼다. 이 순간부터 읽기에서는 값이 없는 것(enum)·그 선택지만 빠진 것(multi_enum)으로 보이고,
    /// 같은 이름을 다시 추가할 수 있다. 값 삭제·자산별 revision/outbox(priority 1)·선택지 행 삭제는
    /// [`UserData::purge_pending_metadata_deletions`] 가 청크 단위로 한다. 영향 예정 자산 수를 돌려준다.
    pub fn delete_metadata_option(&self, option_id: i64) -> Result<usize, IngestError> {
        self.write(move |tx, now| {
            let field_id = option_field(tx, option_id)?;
            let affected: i64 = tx.query_row(
                "SELECT count(DISTINCT asset_id) FROM asset_metadata_values WHERE field_id = ?1 AND value_int = ?2",
                params![field_id, option_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "UPDATE metadata_field_options SET deleting_at = ?2, label_key = ?3 WHERE option_id = ?1",
                params![option_id, now, super::deleting_key(option_id)],
            )?;
            Ok(usize::try_from(affected).unwrap_or(0))
        })
    }

    /// 이 선택지를 고른 자산 수 (삭제 확인 AlertDialog 용)
    pub fn metadata_option_usage(&self, option_id: i64) -> Result<i64, IngestError> {
        self.read(|c| {
            let field_id = option_field(c, option_id)?;
            Ok(c.query_row(
                "SELECT count(DISTINCT asset_id) FROM asset_metadata_values WHERE field_id = ?1 AND value_int = ?2",
                params![field_id, option_id],
                |r| r.get(0),
            )?)
        })
    }
}
