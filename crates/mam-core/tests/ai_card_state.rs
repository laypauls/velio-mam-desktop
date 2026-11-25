//! 카드 AI 상태 `AssetSummary.ai_state` (T-aistate-*) — 실제 SQLite + 스캔으로 만든 자산.
//!
//! 분류: Happy (02, 03, 04) / Boundary (01, 07) / Business rule (05, 06, 08) / Contract (09: 직렬화) /
//! Performance (10: 쿼리 계획)

mod common;

use std::sync::atomic::AtomicBool;

use common::env::Env;
use mam_core::query::{AiCardState, PAGE_SQL, RELATED_SQL, asset_detail, hydrate_page};
use mam_core::scan::scan_folder;

fn fixture() -> (Env, i64) {
    let env = Env::new();
    env.write("lib/img0.jpg", 1);
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false)).unwrap();
    let id = env.read(|c| {
        c.query_row("SELECT asset_id FROM media_assets", [], |r| r.get::<_, i64>(0))
            .unwrap()
    });
    (env, id)
}

fn exec(env: &Env, sql: String) {
    env.pool
        .write(mam_db::Priority::Low, move |c| {
            c.execute_batch(&sql)?;
            Ok(())
        })
        .unwrap();
}

fn analysis(env: &Env, id: i64) {
    exec(
        env,
        format!(
            "INSERT INTO ai_analyses (asset_id, provider, model, analyzed_at, updated_at)
             VALUES ({id}, 'p', 'm', 1, 1)"
        ),
    );
}

fn transcript(env: &Env, id: i64) {
    exec(
        env,
        format!(
            "INSERT INTO transcripts (asset_id, text, engine, model, created_at)
             VALUES ({id}, 't', 'e', 'm', 1)"
        ),
    );
}

fn task(env: &Env, id: i64, kind: &str, state: &str) {
    exec(
        env,
        format!(
            "INSERT INTO ai_task_state (asset_id, kind, state, updated_at)
             VALUES ({id}, '{kind}', '{state}', 1)"
        ),
    );
}

/// 목록(PAGE_SQL)과 상세(RELATED_SQL) 두 경로의 값을 함께 돌려준다.
fn states(env: &Env, id: i64) -> (AiCardState, AiCardState) {
    let page = env.read(|c| hydrate_page(c, &[id]).unwrap());
    let detail = env.read(|c| asset_detail(c, id).unwrap());
    (page[0].ai_state, detail.summary.ai_state)
}

#[test]
fn t_aistate_01_no_ai_rows_is_none() {
    let (env, id) = fixture();
    assert_eq!(states(&env, id), (AiCardState::None, AiCardState::None));
}

#[test]
fn t_aistate_02_analysis_row_is_done() {
    let (env, id) = fixture();
    analysis(&env, id);
    assert_eq!(states(&env, id), (AiCardState::Done, AiCardState::Done));
}

#[test]
fn t_aistate_03_transcript_row_is_done() {
    let (env, id) = fixture();
    transcript(&env, id);
    assert_eq!(states(&env, id), (AiCardState::Done, AiCardState::Done));
}

#[test]
fn t_aistate_04_failed_describe_without_result_is_failed() {
    let (env, id) = fixture();
    task(&env, id, "describe", "failed");
    assert_eq!(states(&env, id), (AiCardState::Failed, AiCardState::Failed));
}

#[test]
fn t_aistate_05_embedding_only_failure_is_none() {
    let (env, id) = fixture();
    task(&env, id, "embedding", "failed");
    assert_eq!(states(&env, id), (AiCardState::None, AiCardState::None));
}

#[test]
fn t_aistate_06_result_plus_failed_step_is_done() {
    let (env, id) = fixture();
    analysis(&env, id);
    task(&env, id, "ocr", "failed");
    assert_eq!(states(&env, id), (AiCardState::Done, AiCardState::Done));
}

#[test]
fn t_aistate_07_queued_or_running_only_is_none() {
    let (env, id) = fixture();
    task(&env, id, "describe", "queued");
    task(&env, id, "keywords", "running");
    task(&env, id, "summary", "skipped");
    assert_eq!(states(&env, id), (AiCardState::None, AiCardState::None));
}

#[test]
fn t_aistate_08_embedding_failure_does_not_hide_other_failure() {
    let (env, id) = fixture();
    task(&env, id, "embedding", "failed");
    task(&env, id, "stt", "failed");
    assert_eq!(states(&env, id), (AiCardState::Failed, AiCardState::Failed));
}

#[test]
fn t_aistate_09_serializes_lowercase_camel_field() {
    let (env, id) = fixture();
    analysis(&env, id);
    let page = env.read(|c| hydrate_page(c, &[id]).unwrap());
    let json = serde_json::to_value(&page[0]).unwrap();
    assert_eq!(json["aiState"], "done");
    assert_eq!(serde_json::to_value(AiCardState::None).unwrap(), "none");
    assert_eq!(serde_json::to_value(AiCardState::Failed).unwrap(), "failed");
}

#[test]
fn t_aistate_10_subqueries_use_key_lookups_not_scans() {
    let (env, _) = fixture();
    for sql in [PAGE_SQL, RELATED_SQL] {
        let plan: Vec<String> = env.read(|c| {
            let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            s.query_map(["[1]"], |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        });
        let all = plan.join("\n");
        println!("{all}");
        for t in ["ai_analyses", "transcripts", "ai_task_state"] {
            assert!(!all.contains(&format!("SCAN {t}")), "{t} scan:\n{all}");
            assert!(all.contains(&format!("SEARCH {t}")), "{t} not searched:\n{all}");
        }
    }
}
