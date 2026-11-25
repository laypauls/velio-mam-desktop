//! P5 provider 출력 검증·오류 매핑 (T-pjob-14) — 잘못된 출력은 `output_invalid`, 철회된 권한은 `permission_denied`, 총량은 `quota_exceeded`.
//!
//! 분류: Validation(출력 모양·상한) / Business rule(권한 게이트 ON: 플러그인은 활성이고 다른 권한은 허용) / Boundary(KV 총량) /
//! External(`InvokeError` 별 처리: 재시도 vs 영구 실패). 모든 실패는 기록 전체 롤백 — KV·태그·revision 불변.
//! 출력 검증은 실제 워커가 JSON 을 돌려주는 경로로, `InvokeError` 매핑은 주입(프로세스 없이)으로 확인한다.

mod common_jobs;

use common_jobs::{Env, Plugin};
use mam_core::jobs::JobOutcome;
use mam_plugin_runtime::{AssetKind, InvokeError, Permission, ProviderKind};
use serde_json::json;

const P: &str = "com.example.outputs";

fn scripted() -> Plugin {
    Plugin::new(P, "scripted").provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
}

/// 자산 `n` 개를 만들고 편집·피드까지 마친다 (작업 행은 자산 id 순으로 실행된다)
fn queued(env: &Env, n: usize) -> Vec<i64> {
    let assets: Vec<i64> = (0..n).map(|i| env.add_asset("image", &format!("{i}.jpg"))).collect();
    assets.iter().for_each(|a| {
        env.edit(*a);
    });
    env.jobs.feed_tick();
    assets
}

fn busy(code: &str) -> InvokeError {
    InvokeError::Plugin { code: code.into(), message: "m".into() }
}

/// 영구 실패로 처리되는 호출 오류와 기대 코드 (InvokeError 는 Clone 이 아니라 호출할 때마다 만든다)
fn permanent() -> Vec<(InvokeError, &'static str)> {
    vec![
        (InvokeError::Script("x".into()), "script_error"),
        (busy("too_large"), "plugin_error"),
        (InvokeError::NotRegistered("x".into()), "not_registered"),
        (InvokeError::Limit("x".into()), "limit"),
        (InvokeError::Oversize, "oversize"),
        (InvokeError::Flood, "flood"),
        (InvokeError::Protocol("x".into()), "protocol"),
        (InvokeError::Init("x".into()), "init_failed"),
    ]
}

fn assert_failed(env: &Env, asset: i64, code: &str) {
    let t = env.task(P, "meta.run", asset).unwrap();
    assert_eq!((t.status.as_str(), t.last_error.as_deref()), ("failed", Some(code)), "asset {asset}");
}

fn assert_nothing_written(env: &Env, assets: &[i64]) {
    assert_eq!(env.kv_rows(), 0);
    assert!(assets.iter().all(|a| env.plugin_tags(*a).is_empty() && env.revision(*a) == 2), "no tag, no revision bump");
}

#[test]
fn t_pjob_14_invalid_outputs_fail_the_task_with_output_invalid_and_write_nothing() {
    // Arrange: 자산마다 다른 잘못된 출력 (알 수 없는 필드·키 33개·나쁜 태그·64 KiB 초과 값·태그 0개/21개·객체가 아님)
    let env = Env::new();
    env.install(scripted());
    let too_many_tags: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
    env.invoker.script(vec![
        json!({"out": {"unknown": 1}}),
        json!({"manyKeys": 33}),
        json!({"out": {"tags": ["\u{7}bad"]}}),
        json!({"out": {"tags": [""]}}),
        json!({"bigKv": 70_000}),
        json!({"out": {"tags": []}}),
        json!({"out": {"tags": too_many_tags}}),
        json!({"out": 5}),
    ]);
    let assets = queued(&env, 8);

    // Act
    let done = env.run(8);

    // Assert: 모두 failed(output_invalid) 이고 작업은 정상 종료, 아무것도 쓰이지 않았다
    assert!(done.iter().all(|d| matches!(d.outcome, Ok(JobOutcome::Done))));
    assets.iter().for_each(|a| assert_failed(&env, *a, "output_invalid"));
    assert_nothing_written(&env, &assets);
    assert_eq!(env.task(P, "meta.run", assets[0]).unwrap().attempts, 0);
}

#[test]
fn t_pjob_14_output_boundaries_32_keys_and_20_tags_are_accepted() {
    // Arrange
    let env = Env::new();
    env.install(scripted());
    let twenty: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
    env.invoker.script(vec![json!({"manyKeys": 32}), json!({"out": {"tags": twenty}})]);
    let assets = queued(&env, 2);

    // Act
    env.run(2);

    // Assert
    assert!(assets.iter().all(|a| env.task(P, "meta.run", *a).is_some_and(|t| t.status == "done")));
    assert_eq!(env.kv_rows(), 32);
    assert_eq!(env.plugin_tags(assets[1]).len(), 20);
}

#[test]
fn t_pjob_14_kv_output_with_kv_write_revoked_is_permission_denied_while_the_plugin_stays_enabled() {
    // Arrange: 게이트 ON — 플러그인은 활성이고 asset.read/tag.write 등 나머지 권한은 허용, kv.write 만 철회
    let env = Env::new();
    env.install(scripted());
    let without_kv_write: Vec<Permission> = Permission::ALL.into_iter().filter(|p| *p != Permission::KvWrite).collect();
    env.set_granted(P, &without_kv_write);
    env.invoker.script(vec![json!({"out": {"kv": {"a": 1}, "tags": ["x"]}}), json!({"out": {"tags": ["y"]}})]);
    let assets = queued(&env, 2);

    // Act
    env.run(2);

    // Assert: kv 가 든 출력은 거부되고 같은 출력의 태그도 쓰이지 않았다(원자성). 태그만 있는 출력은 허용돼 완료
    assert_failed(&env, assets[0], "permission_denied");
    assert_eq!((env.kv_rows(), env.plugin_tags(assets[0])), (0, Vec::<String>::new()));
    assert!(env.task(P, "meta.run", assets[1]).is_some_and(|t| t.status == "done"));
    assert_eq!(env.plugin_tags(assets[1]), vec!["y".to_owned()]);
}

#[test]
fn t_pjob_14_tag_output_with_tag_write_revoked_is_permission_denied() {
    // Arrange
    let env = Env::new();
    env.install(scripted());
    let without_tag_write: Vec<Permission> = Permission::ALL.into_iter().filter(|p| *p != Permission::TagWrite).collect();
    env.set_granted(P, &without_tag_write);
    env.invoker.script(vec![json!({"out": {"tags": ["x"]}})]);
    let assets = queued(&env, 1);

    // Act
    env.run(1);

    // Assert
    assert_failed(&env, assets[0], "permission_denied");
    assert_nothing_written(&env, &assets);
}

#[test]
fn t_pjob_14_kv_total_over_the_quota_fails_with_quota_exceeded_and_keeps_earlier_data() {
    // Arrange: 플러그인당 KV 총량 300 바이트 (행 비용 = 키 + 값 + 64)
    let env = Env::with(|cfg| cfg.kv_quota_bytes = 300);
    env.install(scripted());
    env.invoker.script(vec![json!({"out": {"kv": {"a": 1}}}), json!({"bigKv": 1_000})]);
    let assets = queued(&env, 2);

    // Act
    env.run(2);

    // Assert
    assert!(env.task(P, "meta.run", assets[0]).is_some_and(|t| t.status == "done"));
    assert_failed(&env, assets[1], "quota_exceeded");
    assert_eq!(env.kv_rows(), 1, "the earlier row stays, the oversized write left nothing");
}

#[test]
fn t_pjob_14_transient_invoke_errors_stay_pending_and_permanent_ones_fail_with_a_fixed_code() {
    // Arrange: 호출 오류를 주입 (프로세스 없이) — 자산 순서대로 소비된다
    let env = Env::new();
    env.install(scripted());
    let transient = vec![InvokeError::Timeout, InvokeError::Crashed, InvokeError::Start("io".into()), busy("busy"), busy("unavailable")];
    let codes: Vec<&str> = permanent().into_iter().map(|(_, c)| c).collect();
    let (n_t, n_p) = (transient.len(), codes.len());
    let mut all = transient;
    all.extend(permanent().into_iter().map(|(e, _)| e));
    env.invoker.inject(all);
    let assets = queued(&env, n_t + n_p);

    // Act
    let done = env.run(n_t + n_p);

    // Assert: 일시 오류는 대기 유지 + 시도 1 + 작업은 재시도 가능 오류, 영구 오류는 고정 코드로 failed + 작업은 Done
    for (i, a) in assets.iter().enumerate() {
        let t = env.task(P, "meta.run", *a).unwrap();
        if i < n_t {
            assert_eq!((t.status.as_str(), t.attempts), ("pending", 1), "transient #{i}");
            assert!(done[i].outcome.as_ref().is_err_and(|e| e.retryable), "transient #{i}");
        } else {
            assert_eq!((t.status.as_str(), t.last_error.as_deref(), t.attempts), ("failed", Some(codes[i - n_t]), 0), "permanent #{i}");
            assert!(matches!(done[i].outcome, Ok(JobOutcome::Done)), "permanent #{i}");
        }
    }
    assert_eq!(env.kv_rows(), 0);
}
