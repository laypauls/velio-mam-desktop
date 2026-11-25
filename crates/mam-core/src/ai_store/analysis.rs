//! AI 분석 결과 저장·편집·조회 (P3-1/P3-3).
//!
//! - 저장·편집은 `media_assets.revision` 증가 + `search_outbox` upsert 와 **한 트랜잭션** (db.md §3).
//!   AI 저장 = 저우선 1, 사용자 편집 = 고우선 0.
//! - 재분석은 요청한 종류의 필드를 덮어쓴다. 덮어쓰는 필드에 사용자 편집본이 있었으면 `ai_analysis_replaced` 에 보존한다.

use mam_db::Clock;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

use super::{AI_PRIORITY, AiStoreError, Result, SaveOutcome, requeue_semantic_embedding};

const USER_PRIORITY: i64 = 0;
pub const DESCRIPTION_MAX_CHARS: usize = 20_000;
pub const KEYWORDS_MAX: usize = 100;
pub const KEYWORD_MAX_CHARS: usize = 100;

/// 저장할 분석 필드. `None` = 이번 요청에 없는 종류 - 기존 값을 유지한다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalysisFields {
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub ocr_text: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAnalysis {
    pub fields: AnalysisFields,
    pub language: String,
    pub provider: String,
    pub model: String,
}

/// 사용자 편집 (UI `AnalysisEdit`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalysisEdit {
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
}

/// 조회용 분석 (UI 계약 `AiAnalysis`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisView {
    pub description: String,
    pub keywords: Vec<String>,
    pub ocr_text: String,
    pub summary: String,
    pub language: String,
    pub provider: String,
    pub model: String,
    pub analyzed_at: i64,
    pub edited: bool,
}

/// 공백 정리 + 빈 값·중복 제거 (순서 유지).
fn normalize_keywords(raw: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for k in raw {
        let k = k.trim();
        if k.is_empty() || out.iter().any(|o| o == k) {
            continue;
        }
        if k.chars().count() > KEYWORD_MAX_CHARS {
            return Err(AiStoreError::Invalid("keyword too long"));
        }
        out.push(k.to_string());
    }
    if out.len() > KEYWORDS_MAX {
        return Err(AiStoreError::Invalid("too many keywords"));
    }
    Ok(out)
}

fn json(keywords: &[String]) -> String {
    serde_json::to_string(keywords).unwrap_or_else(|_| "[]".into())
}

fn parse_keywords(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

/// revision 증가 (활성 자산만) -> 새 revision.
fn bump_revision(tx: &Transaction<'_>, asset_id: i64, now: i64) -> Result<i64> {
    tx.query_row(
        "UPDATE media_assets SET revision = revision + 1, updated_at = ?2
         WHERE asset_id = ?1 AND status = 'active' RETURNING revision",
        params![asset_id, now],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(AiStoreError::NotFound(asset_id))
}

fn enqueue_outbox(
    tx: &Transaction<'_>,
    asset_id: i64,
    revision: i64,
    priority: i64,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, 'upsert', ?3, ?4)",
        params![asset_id, revision, priority, now],
    )?;
    Ok(())
}

fn row_to_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<AnalysisView> {
    Ok(AnalysisView {
        description: r.get(0)?,
        keywords: parse_keywords(&r.get::<_, String>(1)?),
        ocr_text: r.get(2)?,
        summary: r.get(3)?,
        language: r.get(4)?,
        provider: r.get(5)?,
        model: r.get(6)?,
        analyzed_at: r.get(7)?,
        edited: r.get::<_, i64>(8)? == 1,
    })
}

const SELECT_VIEW: &str = "SELECT description, keywords, ocr_text, summary, language, provider, model, analyzed_at, edited
     FROM ai_analyses WHERE asset_id = ?1";

/// 자산의 분석 결과 (없으면 `None`).
pub fn load_analysis(conn: &Connection, asset_id: i64) -> Result<Option<AnalysisView>> {
    Ok(conn
        .query_row(SELECT_VIEW, [asset_id], row_to_view)
        .optional()?)
}

fn backup_replaced(
    tx: &Transaction<'_>,
    asset_id: i64,
    old: &AnalysisView,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO ai_analysis_replaced
           (asset_id, description, keywords, ocr_text, summary, language, replaced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            asset_id,
            old.description,
            json(&old.keywords),
            old.ocr_text,
            old.summary,
            old.language,
            now
        ],
    )?;
    Ok(())
}

/// AI 분석 결과를 저장한다 (요청한 종류의 필드 덮어쓰기). 자산이 없거나 `active` 가 아니면 [`AiStoreError::NotFound`]
/// 이고 아무것도 바뀌지 않는다.
pub fn save_analysis(
    conn: &mut Connection,
    clock: &dyn Clock,
    asset_id: i64,
    new: &NewAnalysis,
) -> Result<SaveOutcome> {
    let tx = conn.transaction()?;
    let outcome = new.save_in_tx(&tx, clock, asset_id)?;
    tx.commit()?;
    Ok(outcome)
}

impl NewAnalysis {
    /// [`save_analysis`] 의 본체 - 호출자의 트랜잭션 안에서 저장한다 (커밋은 호출자). 저장과 같은 트랜잭션에
    /// 다른 상태 변경(예: 작업 단계 완료 표시)을 묶어야 할 때 쓴다. 오류 시 호출자가 트랜잭션을 롤백해야 한다.
    pub fn save_in_tx(
        &self,
        tx: &Transaction<'_>,
        clock: &dyn Clock,
        asset_id: i64,
    ) -> Result<SaveOutcome> {
        let now = clock.now_ms();
        let keywords = self.fields.keywords.as_deref().map(normalize_keywords);
        let keywords = keywords.transpose()?;
        let revision = bump_revision(tx, asset_id, now)?;
        self.save_rows(tx, asset_id, now, revision, keywords)?;
        Ok(SaveOutcome { revision })
    }

    fn save_rows(
        &self,
        tx: &Transaction<'_>,
        asset_id: i64,
        now: i64,
        revision: i64,
        keywords: Option<Vec<String>>,
    ) -> Result<()> {
        let old = tx
            .query_row(SELECT_VIEW, [asset_id], row_to_view)
            .optional()?;
        let overwrites_edit = old.as_ref().is_some_and(|o| {
            o.edited && (self.fields.description.is_some() || self.fields.keywords.is_some())
        });
        if overwrites_edit && let Some(o) = &old {
            backup_replaced(tx, asset_id, o, now)?;
        }
        let edited = old.as_ref().is_some_and(|o| o.edited) && !overwrites_edit;
        let pick = |n: &Option<String>, o: Option<&String>| {
            n.clone().or_else(|| o.cloned()).unwrap_or_default()
        };
        let description = pick(
            &self.fields.description,
            old.as_ref().map(|o| &o.description),
        );
        let ocr_text = pick(&self.fields.ocr_text, old.as_ref().map(|o| &o.ocr_text));
        let summary = pick(&self.fields.summary, old.as_ref().map(|o| &o.summary));
        let kw = keywords
            .or_else(|| old.as_ref().map(|o| o.keywords.clone()))
            .unwrap_or_default();

        tx.execute(
        "INSERT INTO ai_analyses
           (asset_id, description, keywords, ocr_text, summary, language, provider, model, edited, analyzed_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         ON CONFLICT (asset_id) DO UPDATE SET
           description = excluded.description, keywords = excluded.keywords,
           ocr_text = excluded.ocr_text, summary = excluded.summary, language = excluded.language,
           provider = excluded.provider, model = excluded.model, edited = excluded.edited,
           analyzed_at = excluded.analyzed_at, updated_at = excluded.updated_at",
        params![
            asset_id,
            description,
            json(&kw),
            ocr_text,
            summary,
            self.language,
            self.provider,
            self.model,
            i64::from(edited),
            now
        ],
    )?;
        enqueue_outbox(tx, asset_id, revision, AI_PRIORITY, now)?;
        Ok(())
    }
}

/// 사용자 편집. 분석이 없던 자산이면 `provider = local` 행을 만든다. 고우선(0) outbox.
pub fn update_analysis(
    conn: &mut Connection,
    clock: &dyn Clock,
    asset_id: i64,
    edit: &AnalysisEdit,
) -> Result<AnalysisView> {
    let now = clock.now_ms();
    if edit
        .description
        .as_ref()
        .is_some_and(|d| d.chars().count() > DESCRIPTION_MAX_CHARS)
    {
        return Err(AiStoreError::Invalid("description too long"));
    }
    let keywords = edit
        .keywords
        .as_deref()
        .map(normalize_keywords)
        .transpose()?;
    let tx = conn.transaction()?;
    let revision = bump_revision(&tx, asset_id, now)?;
    let old = tx
        .query_row(SELECT_VIEW, [asset_id], row_to_view)
        .optional()?;
    let view = AnalysisView {
        description: edit
            .description
            .clone()
            .or_else(|| old.as_ref().map(|o| o.description.clone()))
            .unwrap_or_default(),
        keywords: keywords
            .or_else(|| old.as_ref().map(|o| o.keywords.clone()))
            .unwrap_or_default(),
        ocr_text: old.as_ref().map(|o| o.ocr_text.clone()).unwrap_or_default(),
        summary: old.as_ref().map(|o| o.summary.clone()).unwrap_or_default(),
        language: old.as_ref().map(|o| o.language.clone()).unwrap_or_default(),
        provider: old.as_ref().map_or("local".into(), |o| o.provider.clone()),
        model: old.as_ref().map_or("user".into(), |o| o.model.clone()),
        analyzed_at: old.as_ref().map_or(now, |o| o.analyzed_at),
        edited: true,
    };
    tx.execute(
        "INSERT INTO ai_analyses
           (asset_id, description, keywords, ocr_text, summary, language, provider, model, edited, analyzed_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10)
         ON CONFLICT (asset_id) DO UPDATE SET
           description = excluded.description, keywords = excluded.keywords, edited = 1,
           updated_at = excluded.updated_at",
        params![
            asset_id,
            view.description,
            json(&view.keywords),
            view.ocr_text,
            view.summary,
            view.language,
            view.provider,
            view.model,
            view.analyzed_at,
            now
        ],
    )?;
    enqueue_outbox(&tx, asset_id, revision, USER_PRIORITY, now)?;
    // 설명·키워드가 실제로 바뀌면 낡은 의미 벡터를 의미 전용으로 다시 대기시킨다 (같은 트랜잭션)
    let text_changed = old
        .as_ref()
        .is_none_or(|o| o.description != view.description || o.keywords != view.keywords);
    if text_changed {
        requeue_semantic_embedding(&tx, now, asset_id)?;
    }
    tx.commit()?;
    Ok(view)
}
