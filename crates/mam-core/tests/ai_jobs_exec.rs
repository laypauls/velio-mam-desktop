//! T-aij-06, 08, 10, 17, 21~25 - AI 작업 실행기: 단계 순서, 동의 게이트, 멱등, 일시정지 양보, 음성 처리, 원본 보호.
//! 분류: Happy / Business rule / Concurrency(멱등) / Time(제한 시간 공식) / Boundary.

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::StepKind;
use mam_core::jobs::JobOutcome;

fn done_all(e: &AiEnv, id: i64, kinds: &[&str]) {
    for k in kinds {
        assert_eq!(e.st(id, k), "done", "{k}");
    }
}

#[test]
fn t_aij_06_video_runs_stt_then_external_then_embedding_and_stores_each_result() {
    // Arrange
    let e = AiEnv::new();
    let id = e.add_asset("video");
    let q = e.queue_all(id, None, false);
    let rev0 = e.env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {id}"
    ));

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert
    assert_eq!(out, Ok(JobOutcome::Done));
    assert_eq!(q.queued, 1);
    assert_eq!(e.order(), vec!["stt", "external", "embedding"]);
    done_all(
        &e,
        id,
        &["stt", "describe", "keywords", "ocr", "summary", "embedding"],
    );
    let ext = e.ext.calls.lock().unwrap();
    assert_eq!(ext.len(), 1, "외부 요청은 1회로 묶는다");
    assert_eq!(ext[0].kinds.len(), 4);
    assert_eq!(ext[0].images.len(), 5);
    assert!(
        ext[0].images[0].path.starts_with(&e.cache),
        "이미지는 MAMData 썸네일만"
    );
    let emb = e.emb.calls.lock().unwrap();
    assert_eq!(emb[0].images.len(), 5);
    let text = emb[0].text_parts.join("|");
    assert!(
        text.contains("a red car") && text.contains("hello world"),
        "AI 설명과 전사가 의미 벡터 입력에 포함: {text}"
    );
    assert_eq!(e.env.count("SELECT COUNT(*) FROM transcripts"), 1);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 1);
    assert_eq!(
        e.env.count(
            "SELECT COUNT(*) FROM ai_usage WHERE ok = 1 AND kind = 'describe+keywords+ocr+summary'"
        ),
        1
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE ok = 1"),
        6
    );
    assert_eq!(
        e.env.count("SELECT COUNT(*) FROM jobs"),
        0,
        "완료 작업 행은 삭제"
    );
    // 전사·분석 저장마다 revision 증가 + 저우선 outbox
    let rev1 = e.env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {id}"
    ));
    assert_eq!(rev1, rev0 + 2);
    assert_eq!(e.env.count(&format!("SELECT COUNT(*) FROM search_outbox WHERE asset_id = {id} AND priority = 1 AND revision > {rev0}")), 2);
}

#[test]
fn t_aij_08_without_consent_external_is_skipped_but_local_steps_run() {
    // Arrange: 동의 상태에서 등록한 뒤 실행 전에 철회 (게이트 ON 경로)
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    e.set(|s| s.consent_given = false);

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert: 어떤 것도 전송되지 않고(요청 0, 사용량 0), 로컬 단계는 실행
    assert_eq!(out, Ok(JobOutcome::Done));
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_usage"), 0);
    for k in ["describe", "keywords", "ocr", "summary"] {
        assert_eq!(
            e.step(id, k),
            Some(("skipped".into(), Some("consent_required".into())))
        );
    }
    done_all(&e, id, &["stt", "embedding"]);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE ok = 0"),
        0,
        "건너뜀은 실패 이력이 아니다"
    );
}

#[test]
fn t_aij_10_rerunning_the_same_job_skips_done_steps_without_duplicates() {
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    e.run_next(&|| false).unwrap().unwrap();
    let (stt_n, ext_n, emb_n) = (
        e.stt.calls.lock().unwrap().len(),
        e.ext.calls.lock().unwrap().len(),
        e.emb.calls.lock().unwrap().len(),
    );

    // Act: 같은 자산의 작업을 다시 등록·실행 (force 아님 / 직접 enqueue)
    let again = e.queue_all(id, None, false);
    e.queue
        .enqueue(mam_core::jobs::NewJob::for_asset(
            mam_core::jobs::JobKind::Ai,
            id,
        ))
        .unwrap();
    let out = e.run_next(&|| false).unwrap();

    // Assert
    assert_eq!(again.queued, 0, "완료 단계는 다시 등록되지 않는다");
    assert_eq!(out, Ok(JobOutcome::Done));
    assert_eq!(
        (
            e.stt.calls.lock().unwrap().len(),
            e.ext.calls.lock().unwrap().len(),
            e.emb.calls.lock().unwrap().len()
        ),
        (stt_n, ext_n, emb_n)
    );
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_task_state"), 6);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 1);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM analysis_runs"), 6);
}

#[test]
fn t_aij_10b_force_reanalysis_overwrites_without_duplicate_rows() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.run_next(&|| false).unwrap().unwrap();
    e.ext.script.lock().unwrap().push_back(Ok({
        let mut r = ai_env::ok_external();
        r.description = "second".into();
        r
    }));

    let q = e.queue_all(id, Some(&[StepKind::Describe]), true);
    e.run_next(&|| false).unwrap().unwrap();

    assert_eq!(q.queued, 1);
    let d: String = e.env.read(|c| {
        c.query_row("SELECT description FROM ai_analyses", [], |r| r.get(0))
            .unwrap()
    });
    assert_eq!(d, "second");
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 1);
    assert_eq!(
        e.env.count("SELECT COUNT(*) FROM ai_task_state"),
        5,
        "이미지: describe keywords ocr summary embedding"
    );
}

#[test]
fn t_aij_17_pause_yields_between_steps_and_keeps_completed_steps() {
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    let log = e.log.clone();

    // Act: stt 가 끝난 뒤 일시정지 신호
    let out = e
        .run_next(&move || log.lock().unwrap().iter().any(|s| s == "stt"))
        .unwrap();

    // Assert: 양보, 시도 횟수 불변, stt 결과 보존, 나머지는 queued
    assert_eq!(out, Ok(JobOutcome::Yielded));
    assert_eq!(e.st(id, "stt"), "done");
    assert_eq!(e.st(id, "describe"), "queued");
    assert_eq!(e.env.count("SELECT COUNT(*) FROM transcripts"), 1);
    assert_eq!(e.env.count("SELECT attempts FROM jobs"), 0);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM jobs WHERE status = 'pending'"),
        1
    );

    // 재개: 이어서 끝낸다 (stt 는 다시 실행되지 않는다)
    assert_eq!(e.run_next(&|| false).unwrap(), Ok(JobOutcome::Done));
    assert_eq!(e.stt.calls.lock().unwrap().len(), 1);
    done_all(&e, id, &["describe", "embedding"]);
}

#[test]
fn t_aij_17b_stop_before_first_step_runs_nothing() {
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);

    let out = e.run_next(&|| true).unwrap();

    assert_eq!(out, Ok(JobOutcome::Yielded));
    assert!(e.order().is_empty());
    assert_eq!(e.st(id, "stt"), "queued");
}
