//! 등록 출처(`ai_task_state.origin`) - 자동 옵션을 끄면 그 출처의 대기 단계만 정리하고, 사용자가 직접 요청한 분석은 보존한다.
//!
//! 정리는 대기 행을 **삭제**한다 (skipped 로 바꾸지 않는다): `UNANALYZED_SQL` 은 행이 있는 자산을 다시 고르지 않으므로
//! skipped 로 두면 옵션을 다시 켜도 그 자산이 분석되지 않는다.

use rusqlite::{Connection, params_from_iter};

use super::AiJobError;
use super::config::AiRuntimeSettings;
use super::executor::AiExecutor;
use super::schedule::AiScheduler;

/// 단계 등록 출처. DB 에는 [`Origin::as_str`] 문자열로 저장한다 (NULL = 구 행 = `Manual` 취급).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// 신규 자산 자동 분석 (`autoAnalyzeNew`)
    AutoNew,
    /// 미분석 백그라운드 스윕 (`backgroundAnalyze`)
    Background,
    /// 의미 검색 임베딩 백필 (`semanticSearch.enabled`)
    Backfill,
    /// 사용자 요청 (선택·일괄·재시도·편집)
    Manual,
}

impl Origin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Origin::AutoNew => "auto_new",
            Origin::Background => "background",
            Origin::Backfill => "backfill",
            Origin::Manual => "manual",
        }
    }
}

/// 정리 대상 단계 종류 범위. 자막(`stt`)은 자동 분석과 따로 켜고 끄므로 종류 단위로 나눈다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindScope {
    /// 모든 AI 단계
    All,
    /// 자막(`stt`)만
    OnlyStt,
    /// 자막을 뺀 단계
    ExceptStt,
}

impl KindScope {
    const fn sql(self) -> &'static str {
        match self {
            KindScope::All => {
                "kind IN ('describe', 'keywords', 'ocr', 'summary', 'stt', 'embedding')"
            }
            KindScope::OnlyStt => "kind = 'stt'",
            KindScope::ExceptStt => {
                "kind IN ('describe', 'keywords', 'ocr', 'summary', 'embedding')"
            }
        }
    }
}

/// (출처, 종류 범위) - 정리·실행 게이트의 단위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginScope {
    pub origin: Origin,
    pub kinds: KindScope,
}

impl OriginScope {
    pub const fn all(origin: Origin) -> Self {
        Self {
            origin,
            kinds: KindScope::All,
        }
    }
}

/// 설정이 꺼져 있어 더는 실행하면 안 되는 자동 출처들 (자동 신규는 분석·자막을 따로 판단한다).
pub(super) fn disabled_origins(st: &AiRuntimeSettings) -> Vec<OriginScope> {
    let mut v = Vec::new();
    if !(st.auto_analyze_new && st.consent_given) {
        v.push(OriginScope {
            origin: Origin::AutoNew,
            kinds: KindScope::ExceptStt,
        });
    }
    if !st.auto_subtitle_new {
        v.push(OriginScope {
            origin: Origin::AutoNew,
            kinds: KindScope::OnlyStt,
        });
    }
    if !(st.background_analyze && st.consent_given) {
        v.push(OriginScope::all(Origin::Background));
    }
    if !st.semantic_enabled {
        v.push(OriginScope::all(Origin::Backfill));
    }
    v
}

/// 해당 (출처, 종류 범위)의 단계 행을 지우고, 그 결과 처리할 단계(queued/running)가 없어진 자산의 대기(pending) `ai` 작업을 지운다.
/// `include_running` 이면 실행 중(running) 표식도 지운다 (실행기가 시작 시 고아 행을 정리할 때). 호출자의 트랜잭션 안에서 부른다.
/// 지운 단계 수를 돌려준다. 멱등.
pub(super) fn delete_origin_steps(
    conn: &Connection,
    scopes: &[OriginScope],
    asset_id: Option<i64>,
    include_running: bool,
) -> rusqlite::Result<usize> {
    if scopes.is_empty() {
        return Ok(0);
    }
    let states = if include_running {
        "('queued', 'running')"
    } else {
        "('queued')"
    };
    let asset_cond = if asset_id.is_some() {
        "AND asset_id = ?"
    } else {
        ""
    };
    let cond = scopes
        .iter()
        .map(|s| format!("(origin = ? AND {})", s.kinds.sql()))
        .collect::<Vec<_>>()
        .join(" OR ");
    let mut args: Vec<rusqlite::types::Value> = scopes
        .iter()
        .map(|s| s.origin.as_str().to_string().into())
        .collect();
    if let Some(id) = asset_id {
        args.push(id.into());
    }
    let affected: Vec<i64> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT asset_id FROM ai_task_state
             WHERE state IN {states} AND ({cond}) {asset_cond}"
        ))?;
        stmt.query_map(params_from_iter(args.iter()), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let removed = conn.execute(
        &format!(
            "DELETE FROM ai_task_state
             WHERE state IN {states} AND ({cond}) {asset_cond}"
        ),
        params_from_iter(args.iter()),
    )?;
    for id in affected {
        conn.execute(
            "DELETE FROM jobs WHERE kind = 'ai' AND asset_id = ?1 AND status = 'pending'
               AND NOT EXISTS (SELECT 1 FROM ai_task_state s
                               WHERE s.asset_id = ?1 AND s.state IN ('queued', 'running'))",
            [id],
        )?;
    }
    Ok(removed)
}

impl AiScheduler {
    /// 자동 옵션이 꺼졌을 때: 해당 출처의 대기(queued) 단계와, 처리할 단계가 남지 않은 자산의 대기 `ai` 작업을 한 트랜잭션에서
    /// 지운다. 실행 중(running) 단계와 사용자 요청(manual·구 행)은 건드리지 않는다. 지운 단계 수를 돌려준다. 멱등.
    pub fn cancel_auto_pending(&self, origins: &[Origin]) -> Result<usize, AiJobError> {
        let scopes: Vec<OriginScope> = origins.iter().copied().map(OriginScope::all).collect();
        self.cancel_auto_scopes(&scopes)
    }

    /// [`Self::cancel_auto_pending`] 의 종류 범위 버전 - 예: 자동 자막만 끄면 `auto_new` 의 `stt` 단계만 지운다.
    pub fn cancel_auto_scopes(&self, scopes: &[OriginScope]) -> Result<usize, AiJobError> {
        let scopes = scopes.to_vec();
        self.write(move |c, _| {
            let tx = c.transaction()?;
            let n = delete_origin_steps(&tx, &scopes, None, false)?;
            tx.commit()?;
            Ok(n)
        })
    }
}

impl AiExecutor {
    /// 실행 직전 안전망: 옵션이 꺼진 자동 출처의 이 자산 단계(queued, 그리고 비정상 종료로 남은 running)를 지운다.
    /// 재시작(`recover`) 직후나 정리와 실행 사이의 경합에서도 꺼진 옵션의 작업이 실행되지 않게 한다.
    pub(super) fn drop_disabled_origin_steps(&self, asset_id: i64) -> Result<usize, AiJobError> {
        let off = disabled_origins(&self.d.settings.snapshot());
        if off.is_empty() {
            return Ok(0);
        }
        self.write(move |c, _| {
            let tx = c.transaction()?;
            let n = delete_origin_steps(&tx, &off, Some(asset_id), true)?;
            tx.commit()?;
            Ok(n)
        })
    }
}
