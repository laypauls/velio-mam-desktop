//! 메모·즐겨찾기 (`asset_user_data`). 행이 없으면 메모 '' / 즐겨찾기 아님.

use rusqlite::{OptionalExtension, params};

use super::{
    MAX_MEMO_CHARS, MAX_TITLE_CHARS, UserData, bump_assets, clean_ids, json_ids, require_assets, returning_ids,
};
use crate::scan::IngestError;

impl UserData {
    /// 메모를 바꾼다 (빈 문자열 = 지우기). 값이 그대로면 revision 을 올리지 않는다.
    pub fn set_memo(&self, asset_id: i64, memo: &str) -> Result<(), IngestError> {
        if memo.chars().count() > MAX_MEMO_CHARS || memo.contains('\0') {
            return Err(IngestError::InvalidInput("memo"));
        }
        let memo = memo.to_string();
        self.write(move |tx, now| {
            require_assets(tx, &[asset_id])?;
            let current: Option<String> = tx
                .query_row(
                    "SELECT memo FROM asset_user_data WHERE asset_id = ?1",
                    [asset_id],
                    |r| r.get(0),
                )
                .optional()?;
            if current.as_deref().unwrap_or("") == memo {
                return Ok(());
            }
            tx.execute(
                "INSERT INTO asset_user_data (asset_id, memo, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (asset_id) DO UPDATE
                   SET memo = excluded.memo, updated_at = excluded.updated_at",
                params![asset_id, memo, now],
            )?;
            bump_assets(tx, now, &[asset_id], 0)
        })
    }

    /// 선택한 자산의 즐겨찾기를 한 트랜잭션으로 설정/해제한다. 실제로 바뀐 자산만 revision 증가.
    pub fn set_favorite(&self, asset_ids: &[i64], favorite: bool) -> Result<(), IngestError> {
        let ids = clean_ids(asset_ids)?;
        if ids.is_empty() {
            return Ok(());
        }
        self.write(move |tx, now| {
            require_assets(tx, &ids)?;
            let json = json_ids(&ids);
            let changed = if favorite {
                returning_ids(
                    tx,
                    "INSERT INTO asset_user_data (asset_id, favorite, updated_at)
                     SELECT value, 1, ?2 FROM json_each(?1) WHERE true
                     ON CONFLICT (asset_id) DO UPDATE
                       SET favorite = 1, updated_at = excluded.updated_at WHERE favorite = 0
                     RETURNING asset_id",
                    params![json, now],
                )?
            } else {
                returning_ids(
                    tx,
                    "UPDATE asset_user_data SET favorite = 0, updated_at = ?2
                     WHERE favorite = 1 AND asset_id IN (SELECT value FROM json_each(?1))
                     RETURNING asset_id",
                    params![json, now],
                )?
            };
            bump_assets(tx, now, &changed, 0)
        })
    }
}

impl UserData {
    /// 콘텐츠 제목을 바꾼다 (MAM 메타데이터 — 원본 파일명은 건드리지 않는다). 앞뒤 공백 제거 후 비면 `None`/빈 값 = 지우기(NULL).
    /// 설정 `titleEditEnabled` 가 꺼져 있으면 `invalid_input`. 값이 그대로면 revision 을 올리지 않는다.
    pub fn set_title(&self, asset_id: i64, title: Option<&str>) -> Result<(), IngestError> {
        let title = clean_title(title)?;
        self.write(move |tx, now| {
            require_assets(tx, &[asset_id])?;
            let enabled = mam_db::settings::load(tx)
                .map_err(IngestError::from)?
                .0
                .title_edit_enabled;
            if !enabled {
                return Err(IngestError::InvalidInput("title editing is disabled"));
            }
            let current: Option<String> = tx
                .query_row(
                    "SELECT title FROM asset_user_data WHERE asset_id = ?1",
                    [asset_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            if current == title {
                return Ok(());
            }
            tx.execute(
                "INSERT INTO asset_user_data (asset_id, title, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (asset_id) DO UPDATE
                   SET title = excluded.title, updated_at = excluded.updated_at",
                params![asset_id, title, now],
            )?;
            bump_assets(tx, now, &[asset_id], 0)
        })
    }
}

/// 제목 정리: trim, 비면 `None`. 제어 문자 또는 [`MAX_TITLE_CHARS`] 초과는 `invalid_input`.
fn clean_title(raw: Option<&str>) -> Result<Option<String>, IngestError> {
    let t = raw.map(str::trim).unwrap_or("");
    if t.is_empty() {
        return Ok(None);
    }
    if t.chars().count() > MAX_TITLE_CHARS || t.chars().any(char::is_control) {
        return Err(IngestError::InvalidInput("title"));
    }
    Ok(Some(t.to_string()))
}
