//! 검색 조회 서비스 (P2-5) — 검색 결과 페이지 구성·자산 상세·최근 검색. 전부 SQLite 이고 Tantivy 를 모른다
//! (조정기는 `mam-search`, 둘을 묶는 것은 앱 계층). 목록 조회는 읽기 연결, 최근 검색 쓰기는 사용자 편집 우선순위다.
//!
//! 최근 검색은 자산 데이터가 아니므로 revision·outbox 를 건드리지 않는다. 원본 파일은 열지 않는다.

mod collection_order;
mod hydrate;
mod types;
mod user_field_filter;

use std::sync::Arc;

use mam_db::{DbPool, Priority};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(crate) use hydrate::{availability_of, file_name_of, media_kind_of};
pub use hydrate::{
    MAX_DUPLICATES, NEWER_SQL, PAGE_SQL, PREVIOUS_SQL, RELATED_SQL, asset_detail, hydrate_page,
};
pub use types::*;
pub use user_field_filter::{MAX_USER_FIELD_FILTERS, UserFieldFilter, validate_user_field_filters};

use crate::SharedClock;
use crate::scan::IngestError;

/// 최근 검색 보관 개수
pub const MAX_RECENT_SEARCHES: i64 = 50;
/// 질의 문자열 최대 글자 수
pub const MAX_QUERY_CHARS: usize = 500;

/// 최근 검색 쓰기: 같은 (query, filters_json) 은 `used_at` 만 갱신한다.
const UPSERT_RECENT_SQL: &str =
    "INSERT INTO recent_searches (query, filters_json, used_at) VALUES (?1, ?2, ?3)
 ON CONFLICT (query, filters_json) DO UPDATE SET used_at = excluded.used_at";

/// 오래된 행 정리 (`idx_recent_searches_used`)
const PRUNE_RECENT_SQL: &str = "DELETE FROM recent_searches WHERE recent_id IN (
   SELECT recent_id FROM recent_searches ORDER BY used_at DESC, recent_id DESC LIMIT -1 OFFSET ?1)";

const LIST_RECENT_SQL: &str = "SELECT query, filters_json, used_at FROM recent_searches
 ORDER BY used_at DESC, recent_id DESC LIMIT ?1";

pub struct QueryService {
    pool: Arc<DbPool>,
    clock: SharedClock,
}

fn clean_query(raw: &str) -> Result<String, IngestError> {
    let q = raw.trim();
    if q.chars().count() > MAX_QUERY_CHARS || q.contains('\0') {
        return Err(IngestError::InvalidInput("query"));
    }
    Ok(q.to_string())
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String, IngestError> {
    serde_json::to_string(v).map_err(|_| IngestError::InvalidInput("json"))
}

impl QueryService {
    pub fn new(pool: Arc<DbPool>, clock: SharedClock) -> Self {
        Self { pool, clock }
    }

    fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, IngestError>,
    ) -> Result<T, IngestError> {
        self.pool.read(|c| Ok(f(c)))?
    }

    fn write<T, F>(&self, f: F) -> Result<T, IngestError>
    where
        T: Send + 'static,
        F: FnOnce(&mut rusqlite::Transaction<'_>, i64) -> Result<T, IngestError> + Send + 'static,
    {
        let now = self.clock.now_ms();
        self.pool.write(Priority::High, move |c| {
            Ok((|| {
                let mut tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let out = f(&mut tx, now)?;
                tx.commit()?;
                Ok(out)
            })())
        })?
    }

    // ---- 조회 ----

    /// 검색 결과 페이지 구성 — 질의 1회.
    pub fn hydrate(&self, ids: &[i64]) -> Result<Vec<AssetSummary>, IngestError> {
        self.read(|c| hydrate_page(c, ids))
    }

    pub fn detail(&self, asset_id: i64) -> Result<AssetDetail, IngestError> {
        self.read(|c| asset_detail(c, asset_id))
    }

    /// 지금 연결된 볼륨 id (`includeOffline = false` 필터용).
    pub fn online_volume_ids(&self) -> Result<Vec<u64>, IngestError> {
        self.read(|c| {
            let mut stmt =
                c.prepare_cached("SELECT volume_id FROM volumes WHERE availability = 'online'")?;
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
            Ok(rows
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|v| v as u64)
                .collect())
        })
    }

    /// 검색의 사용자 필드 조건을 현재 정의와 대조한다 (조건이 없으면 DB 를 읽지 않는다 — 검색당 정의 조회는 최대 1회).
    pub fn validate_user_fields(&self, conds: &[UserFieldFilter]) -> Result<(), IngestError> {
        if conds.is_empty() {
            return Ok(());
        }
        let fields = self.read(crate::userdata::load_metadata_fields)?;
        validate_user_field_filters(&fields, conds)
    }

    /// 검색·전체 보기에서 제외할 숨김 폴더 id (`hidden = 1`, 등록 해제 중 포함).
    pub fn hidden_folder_ids(&self) -> Result<Vec<u64>, IngestError> {
        self.read(crate::scan::hidden_folder_ids)
    }

    /// 폴더 범위 -> (관리 폴더 id, 폴더 루트 기준 `path_key`). 없는 폴더는 `NotFound`.
    pub fn resolve_folder(&self, scope: &FolderScope) -> Result<(u64, String), IngestError> {
        let exists = self.read(|c| {
            Ok(c.query_row(
                "SELECT 1 FROM managed_folders WHERE folder_id = ?1",
                [scope.folder_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
        })?;
        if !exists {
            return Err(IngestError::NotFound);
        }
        let rel = scope.rel_path.as_deref().unwrap_or("").trim();
        let dir_key = if rel.is_empty() || rel == "/" {
            String::new()
        } else {
            mam_db::path_key(rel).map_err(|_| IngestError::InvalidInput("relPath"))?
        };
        Ok((scope.folder_id as u64, dir_key))
    }

    // ---- 최근 검색 ----

    pub fn list_recent(&self) -> Result<Vec<RecentSearch>, IngestError> {
        self.read(|c| {
            let mut stmt = c.prepare_cached(LIST_RECENT_SQL)?;
            let rows = stmt.query_map([MAX_RECENT_SEARCHES], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (query, json, used_at) = row?;
                // 읽을 수 없는 행(다른 버전이 쓴 형태 등)은 건너뛴다
                if let Ok(filters) = serde_json::from_str(&json) {
                    out.push(RecentSearch {
                        query,
                        filters,
                        used_at,
                    });
                }
            }
            Ok(out)
        })
    }

    /// 최근 검색에 기록한다. 질의도 없고 필터도 기본이면 기록하지 않는다. 최대 50개를 유지한다.
    pub fn record_recent(&self, query: &str, filters: &SearchFilters) -> Result<(), IngestError> {
        let query = clean_query(query)?;
        if query.is_empty() && filters.is_default() {
            return Ok(());
        }
        let json = to_json(filters)?;
        self.write(move |tx, now| {
            tx.execute(UPSERT_RECENT_SQL, params![query, json, now])?;
            tx.execute(PRUNE_RECENT_SQL, [MAX_RECENT_SEARCHES])?;
            Ok(())
        })
    }

    pub fn clear_recent(&self) -> Result<(), IngestError> {
        self.write(|tx, _| {
            tx.execute("DELETE FROM recent_searches", [])?;
            Ok(())
        })
    }
}
