//! 자산별 값 — 조회, 단건 지정, 대량 지정 (결정 9·18).

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::{Connection, params};

use super::validate::{Row, to_rows};
use super::{AssetFieldValue, FieldPatch, FieldValue, ValueType, bump_chunked, field_type};
use crate::scan::IngestError;
use crate::userdata::{
    BULK_OUTBOX_THRESHOLD, USER_EDIT_PRIORITY, UserData, clean_ids, json_ids, require_assets,
};

impl UserData {
    /// 자산 하나의 값 전체 (필드 표시 순서). 값이 없는 필드는 나오지 않는다.
    pub fn get_metadata_values(&self, asset_id: i64) -> Result<Vec<AssetFieldValue>, IngestError> {
        self.read(|c| load_asset_values(c, asset_id))
    }

    /// 자산 하나의 값을 지정한다 (고우선순위 outbox). `patches` 에 없는 필드는 그대로.
    /// 실제로 바뀐 경우에만 revision 을 올린다. 바뀐 자산 수(0 또는 1)를 돌려준다.
    pub fn set_metadata_values(
        &self,
        asset_id: i64,
        patches: &[(i64, FieldPatch)],
    ) -> Result<usize, IngestError> {
        self.bulk_set_metadata_values(&[asset_id], patches)
    }

    /// 선택한 자산들에 같은 패치를 한 트랜잭션으로 적용한다. 자산 목록은 최대 10,000건(초과 시 `invalid_input`).
    /// 패치에 없는 필드는 건드리지 않고, `Clear`(또는 빈 keyword/빈 multi_enum)는 값 삭제. 하나라도 검증에 실패하면 전체 롤백.
    /// 값이 실제로 바뀐 자산(오름차순)만 revision 증가 + outbox — 앞 1,000건은 priority 0, 초과분은 1 (결정 9).
    /// 바뀐 자산 수를 돌려준다.
    pub fn bulk_set_metadata_values(
        &self,
        asset_ids: &[i64],
        patches: &[(i64, FieldPatch)],
    ) -> Result<usize, IngestError> {
        let ids = clean_ids(asset_ids)?;
        let patches = patches.to_vec();
        let mut seen = HashSet::new();
        if patches.iter().any(|(f, _)| !seen.insert(*f)) {
            return Err(IngestError::InvalidInput("duplicate field"));
        }
        if ids.is_empty() {
            return Ok(0);
        }
        self.write(move |tx, now| {
            require_assets(tx, &ids)?;
            // 검증을 먼저 모두 끝낸다 (필드·타입·선택지) — 쓰기 전에 실패하면 아무것도 바뀌지 않는다
            let mut plans = Vec::with_capacity(patches.len());
            for (field_id, patch) in &patches {
                let vt = field_type(tx, *field_id)?;
                let rows = match patch {
                    FieldPatch::Clear => Vec::new(),
                    FieldPatch::Set(v) => to_rows(vt, v, &option_ids(tx, *field_id, vt)?)?,
                };
                plans.push((*field_id, rows));
            }
            let mut changed = BTreeSet::new();
            for (field_id, rows) in &plans {
                apply_field(tx, now, &ids, *field_id, rows, &mut changed)?;
            }
            let changed: Vec<i64> = changed.into_iter().collect();
            let split = changed.len().min(BULK_OUTBOX_THRESHOLD);
            bump_chunked(tx, now, &changed[..split], USER_EDIT_PRIORITY)?;
            super::bump_low_priority(tx, now, &changed[split..])?;
            Ok(changed.len())
        })
    }
}

fn option_ids(c: &Connection, field_id: i64, vt: ValueType) -> Result<HashSet<i64>, IngestError> {
    if !vt.has_options() {
        return Ok(HashSet::new());
    }
    let mut stmt = c.prepare_cached("SELECT option_id FROM metadata_field_options WHERE field_id = ?1 AND deleting_at IS NULL",
    )?;
    let rows = stmt.query_map([field_id], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 한 필드에 대해 자산들의 현재 값을 읽어 `target` 과 다른 자산만 교체한다. 바뀐 자산은 `changed` 에 추가.
fn apply_field(
    tx: &Connection,
    now: i64,
    ids: &[i64],
    field_id: i64,
    target: &[Row],
    changed: &mut BTreeSet<i64>,
) -> Result<(), IngestError> {
    let mut current: HashMap<i64, Vec<Row>> = HashMap::new();
    {
        let mut stmt = tx.prepare_cached(
            "SELECT asset_id, ord, value_text, value_int, value_real FROM asset_metadata_values
             WHERE field_id = ?1 AND asset_id IN (SELECT value FROM json_each(?2)) ORDER BY asset_id, ord",
        )?;
        let rows = stmt.query_map(params![field_id, json_ids(ids)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Row { ord: r.get(1)?, text: r.get(2)?, int: r.get(3)?, real: r.get(4)? },
            ))
        })?;
        for row in rows {
            let (asset_id, row) = row?;
            current.entry(asset_id).or_default().push(row);
        }
    }
    let mut delete = tx.prepare_cached(
        "DELETE FROM asset_metadata_values WHERE field_id = ?1 AND asset_id = ?2",
    )?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_text, value_int, value_real, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for &asset_id in ids {
        let cur = current.get(&asset_id).map(Vec::as_slice).unwrap_or(&[]);
        if cur == target {
            continue;
        }
        delete.execute(params![field_id, asset_id])?;
        for r in target {
            insert.execute(params![asset_id, field_id, r.ord, r.text, r.int, r.real, now])?;
        }
        changed.insert(asset_id);
    }
    Ok(())
}

/// 자산 하나의 값을 타입과 함께 읽는다 (필드 표시 순서, 값 없는 필드 제외). 검색 색인 문서 구성에도 쓴다.
/// 저장된 값이 타입과 맞지 않으면(손상) `invalid_input`.
pub fn load_asset_values(c: &Connection, asset_id: i64) -> Result<Vec<AssetFieldValue>, IngestError> {
    let mut stmt = c.prepare_cached(
        "SELECT v.field_id, f.value_type, v.ord, v.value_text, v.value_int, v.value_real
         FROM asset_metadata_values v JOIN metadata_fields f ON f.field_id = v.field_id
         LEFT JOIN metadata_field_options o ON o.option_id = v.value_int AND o.field_id = v.field_id
              AND f.value_type IN ('enum', 'multi_enum')
         WHERE v.asset_id = ?1 AND f.deleting_at IS NULL AND o.deleting_at IS NULL ORDER BY f.position, v.field_id, v.ord",
    )?;
    let rows = stmt.query_map([asset_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, Option<f64>>(5)?,
        ))
    })?;
    let corrupt = || IngestError::InvalidInput("stored value");
    let mut out: Vec<AssetFieldValue> = Vec::new();
    for row in rows {
        let (field_id, vt, text, int, real) = row?;
        let value_type = ValueType::parse(&vt).ok_or(IngestError::InvalidInput("stored value type"))?;
        let value = match value_type {
            ValueType::Keyword => FieldValue::Keyword(text.ok_or_else(corrupt)?),
            ValueType::Number => FieldValue::Number(real.ok_or_else(corrupt)?),
            ValueType::Date => FieldValue::Date(int.ok_or_else(corrupt)?),
            ValueType::Boolean => FieldValue::Boolean(int.ok_or_else(corrupt)? != 0),
            ValueType::Enum => FieldValue::Enum(int.ok_or_else(corrupt)?),
            ValueType::MultiEnum => {
                let id = int.ok_or_else(corrupt)?;
                if let Some(last) = out.last_mut().filter(|l| l.field_id == field_id) {
                    if let FieldValue::MultiEnum(v) = &mut last.value {
                        v.push(id);
                    }
                    continue;
                }
                FieldValue::MultiEnum(vec![id])
            }
        };
        out.push(AssetFieldValue { field_id, value_type, value });
    }
    Ok(out)
}
