//! SQLite `asset_metadata_values` -> `UserFieldValue` (사용자 정의 메타데이터 필드, V19).
//!
//! 값 컬럼이 곧 색인 형식이다: `value_text` = keyword, `value_real` = number, `value_int` = date/boolean/enum/multi_enum.
//! 삭제 중인 필드(`deleting_at`)와 삭제 중이거나 사라진 선택지의 enum/multi_enum 값은 제외한다 — 재색인이 끝나기 전에도
//! 새 문서에는 실리지 않는다 (필드 삭제는 값 정리가 청크로 나뉘어 진행된다).
//! mam-search 는 mam-core 에 의존하지 않으므로 이 질의는 자체 SQL 이다.

use std::collections::HashMap;

use rusqlite::{Connection, Row, Rows, params};

use super::load::json_ids;
use crate::userfield::{UserFieldValue, UserValue};

/// 공통 본문 (`v` = asset_metadata_values). 컬럼: asset_id, field_id, value_text, value_int, value_real.
macro_rules! values_select {
    ($from:literal, $order:literal) => {
        concat!(
            "SELECT v.asset_id, v.field_id, v.value_text, v.value_int, v.value_real
 FROM ",
            $from,
            "
 JOIN metadata_fields mf ON mf.field_id = v.field_id AND mf.deleting_at IS NULL
 WHERE (mf.value_type NOT IN ('enum', 'multi_enum') OR EXISTS (
         SELECT 1 FROM metadata_field_options o
         WHERE o.field_id = v.field_id AND o.option_id = v.value_int AND o.deleting_at IS NULL))",
            $order
        )
    };
}

/// 전체 스트리밍 (자산 id 순) — 재구축에서 `ALL_DOCS_SQL` 과 합쳐 읽는다. 기본키 순서라 정렬이 필요 없다.
pub const ALL_VALUES_SQL: &str = values_select!(
    "asset_metadata_values v",
    " ORDER BY v.asset_id, v.field_id, v.ord"
);

/// 배치: `?1` = asset_id JSON 배열.
pub const VALUES_BY_IDS_SQL: &str = values_select!(
    "json_each(?1) j CROSS JOIN asset_metadata_values v ON v.asset_id = j.value",
    ""
);

fn value_from_row(r: &Row) -> rusqlite::Result<(u64, Option<UserFieldValue>)> {
    let asset_id = r.get::<_, i64>(0)? as u64;
    let field_id = r.get::<_, i64>(1)? as u64;
    let value = if let Some(s) = r.get::<_, Option<String>>(2)? {
        Some(UserValue::Keyword(s))
    } else if let Some(x) = r.get::<_, Option<f64>>(4)? {
        Some(UserValue::Number(x))
    } else {
        r.get::<_, Option<i64>>(3)?.map(UserValue::Int)
    };
    Ok((
        asset_id,
        value.map(|value| UserFieldValue { field_id, value }),
    ))
}

/// 여러 자산의 값을 질의 1회로 읽는다 (N+1 없음). 값이 없는 자산은 결과에 없다.
pub fn load_user_values(
    conn: &Connection,
    ids: &[u64],
) -> rusqlite::Result<HashMap<u64, Vec<UserFieldValue>>> {
    let mut out: HashMap<u64, Vec<UserFieldValue>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut stmt = conn.prepare_cached(VALUES_BY_IDS_SQL)?;
    let rows = stmt.query_map(params![json_ids(ids)], value_from_row)?;
    for row in rows {
        if let (asset_id, Some(v)) = row? {
            out.entry(asset_id).or_default().push(v);
        }
    }
    Ok(out)
}

/// 자산 id 오름차순 값 스트림 — 자산 id 오름차순 문서 스트림과 병합한다 (재구축). 문서가 되지 않는 자산(비활성)의 값은 건너뛴다.
pub struct UserValueStream<'s> {
    rows: Rows<'s>,
    pending: Option<(u64, UserFieldValue)>,
    done: bool,
}

impl<'s> UserValueStream<'s> {
    pub fn new(rows: Rows<'s>) -> Self {
        Self {
            rows,
            pending: None,
            done: false,
        }
    }

    fn advance(&mut self) -> rusqlite::Result<()> {
        loop {
            match self.rows.next()? {
                None => {
                    self.done = true;
                    self.pending = None;
                    return Ok(());
                }
                Some(row) => {
                    if let (asset_id, Some(v)) = value_from_row(row)? {
                        self.pending = Some((asset_id, v));
                        return Ok(());
                    }
                }
            }
        }
    }

    /// `asset_id` 의 값을 모두 꺼낸다. 호출은 asset_id 오름차순이어야 한다.
    pub fn take_for(&mut self, asset_id: u64) -> rusqlite::Result<Vec<UserFieldValue>> {
        let mut out = Vec::new();
        loop {
            if self.pending.is_none() && !self.done {
                self.advance()?;
            }
            match self.pending.take() {
                None => return Ok(out),
                Some((id, _)) if id < asset_id => {} // 문서가 아닌 자산의 값 — 버린다
                Some((id, v)) if id == asset_id => out.push(v),
                Some(later) => {
                    self.pending = Some(later);
                    return Ok(out);
                }
            }
        }
    }
}
