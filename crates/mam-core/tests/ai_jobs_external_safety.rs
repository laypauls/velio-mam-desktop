//! T-aij-32 ~ 34 - 외부 AI 응답 저장 안전성: 모델 출력 정제, 저장 실패에도 사용량 유지, 저장+완료 표시 원자성.
//! 분류: Boundary / External / Business rule / Concurrency(재시도가 같은 응답을 잃지 않음)

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::{AiEnv, ok_external};
use mam_core::ai_store::{KEYWORD_MAX_CHARS, KEYWORDS_MAX, load_analysis};
use mam_core::jobs::{JobOutcome, backoff_ms};

#[test]
fn t_aij_32_oversized_model_output_is_sanitized_and_saved_with_one_request() {
    // Arrange: 상한을 넘는 키워드 (멀티바이트 포함), 빈 값·중복
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    let mut res = ok_external();
    res.keywords = vec!["  car ".into(), "".into(), "car".into(), "가".repeat(500)];
    res.keywords
        .extend((0..KEYWORDS_MAX + 50).map(|i| format!("kw{i}")));
    res.description = "d".repeat(50_000);

    e.ext.script.lock().unwrap().push_back(Ok(res));

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert
    assert_eq!(out, Ok(JobOutcome::Done));
    assert_eq!(e.ext.calls.lock().unwrap().len(), 1);
    assert_eq!(e.st(id, "describe"), "done");
    let a = e.env.read(|c| load_analysis(c, id).unwrap().unwrap());
    assert_eq!(a.keywords.len(), KEYWORDS_MAX);
    assert_eq!(a.keywords[0], "car");
    assert_eq!(a.keywords[1].chars().count(), KEYWORD_MAX_CHARS);
    assert_eq!(a.description.chars().count(), 20_000);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_usage WHERE ok = 1"), 1);
}

const BLOCK_ANALYSIS_INSERT: &str = "CREATE TRIGGER boom BEFORE INSERT ON ai_analyses
    BEGIN SELECT RAISE(ABORT, 'boom'); END;";
const BLOCK_STEP_DONE: &str = "CREATE TRIGGER boom BEFORE UPDATE ON ai_task_state
    WHEN NEW.state = 'done' BEGIN SELECT RAISE(ABORT, 'boom'); END;";

fn drop_boom(e: &AiEnv) {
    e.w(|c, _| c.execute_batch("DROP TRIGGER boom").unwrap());
}

#[test]
fn t_aij_33_usage_is_kept_when_saving_the_result_fails_and_error_is_retryable() {
    // Arrange: 저장 시점에 DB 오류 (일시 오류 모사)
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.w(|c, _| c.execute_batch(BLOCK_ANALYSIS_INSERT).unwrap());

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert: 요청은 나갔으므로 사용량은 남고, 결과는 저장되지 않았으며, 일시 DB 오류는 재시도 대상이다
    assert!(out.unwrap_err().retryable);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_usage WHERE ok = 1"), 1);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 0);
    assert_ne!(e.st(id, "describe"), "done");

    // 복구 후 재시도는 성공한다
    drop_boom(&e);
    e.env.clock.advance(backoff_ms(1));
    assert_eq!(e.run_next(&|| false).unwrap(), Ok(JobOutcome::Done));
    assert_eq!(e.st(id, "describe"), "done");
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 1);
}

#[test]
fn t_aij_34_result_save_and_step_completion_are_one_transaction() {
    // Arrange: 단계 완료 표시만 실패하게 만든다
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    let rev0 = e.env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {id}"
    ));
    e.w(|c, _| c.execute_batch(BLOCK_STEP_DONE).unwrap());

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert: 저장만 된 채 단계가 미완료로 남지 않는다 (같은 요청을 다시 보내야 하는 어긋남 방지)
    assert!(out.unwrap_err().retryable);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 0);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM search_outbox WHERE priority = 1"),
        0
    );
    assert_eq!(
        e.env.count(&format!(
            "SELECT revision FROM media_assets WHERE asset_id = {id}"
        )),
        rev0
    );
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_usage WHERE ok = 1"), 1);

    // 복구 후에는 저장과 완료 표시가 함께 반영된다
    drop_boom(&e);
    e.env.clock.advance(backoff_ms(1));
    assert_eq!(e.run_next(&|| false).unwrap(), Ok(JobOutcome::Done));
    assert_eq!(e.st(id, "describe"), "done");
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 1);
}
