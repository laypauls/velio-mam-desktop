//! `Gateway::record_provider_output` (T-prec-*, 실행기 시험 T-pjob-04·14 의 기록 절반) — 실제 SQLite.
//! provider 반환값을 한 트랜잭션으로 기록하고 작업을 완료한다: 검증 -> 설치 -> 권한(게이트 ON) -> 총량 -> KV -> 태그 -> 완료.
//!
//! 분류: Happy / Validation(반환값) / Business rule(권한·루프 방지) / Boundary(상한) / External(강제 실패) / Time(마감)

mod common;

use std::time::Instant;

use common::*;
use mam_db::Priority;
use mam_db::plugin_settings::{INSTALLED, put};
use mam_db::plugin_tasks::{self, with_immediate_tx};
use mam_plugin_gateway::{Completion, RecordError, Recorded};
use mam_plugin_manifest::{Permission, PluginId};
use serde_json::{Value, json};

struct Setup {
    fx: Fx,
    plugin: PluginId,
    asset: i64,
    task: i64,
}

impl Setup {
    fn new() -> Self {
        Self::with_cfg(|_| {})
    }

    fn with_cfg(tweak: impl FnOnce(&mut mam_plugin_gateway::GatewayConfig)) -> Self {
        let fx = Fx::with_cfg(tweak);
        let plugin = fx.full_plugin("com.example.rec");
        install(&fx, "com.example.rec");
        let asset = fx.add_asset("image", "a/x.jpg");
        let task = add_task(&fx, "com.example.rec", asset, 1);
        Self { fx, plugin, asset, task }
    }

    fn done(&self) -> Completion {
        Completion { task_id: self.task, processed_rev: 1 }
    }

    fn record(&self, out: Value) -> Result<Recorded, RecordError> {
        self.fx.gw.record_provider_output(self.plugin.as_str(), self.asset, out, self.done(), soon())
    }

    fn kv_rows(&self) -> i64 {
        self.fx.count("SELECT COUNT(*) FROM plugin_kv")
    }

    fn tag_rows(&self) -> i64 {
        self.fx.count("SELECT COUNT(*) FROM asset_tags")
    }

    fn revision(&self) -> i64 {
        self.fx.count(&format!("SELECT revision FROM media_assets WHERE asset_id = {}", self.asset))
    }

    /// (status, target, done, attempts)
    fn task_row(&self) -> (String, i64, i64, i64) {
        let t = self.task;
        self.fx.read(|c| c.query_row("SELECT status, target_revision, done_revision, attempts FROM plugin_tasks WHERE task_id = ?1", [t], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap())
    }

    /// 아무것도 쓰지 않았고 작업도 그대로
    fn assert_untouched(&self, outbox: i64) {
        assert_eq!((self.kv_rows(), self.tag_rows(), self.revision()), (0, 0, 1));
        assert_eq!(self.fx.count("SELECT COUNT(*) FROM search_outbox"), outbox);
        assert_eq!(self.task_row(), ("pending".into(), 1, 0, 0));
    }
}

fn install(fx: &Fx, id: &str) {
    let id = id.to_owned();
    fx.pool.write(Priority::High, move |c| { put(c, INSTALLED, &id, r#"{"version":"1.0.0","installedAt":1}"#).unwrap(); Ok(()) }).unwrap();
}

fn add_task(fx: &Fx, plugin: &str, asset: i64, target: i64) -> i64 {
    fx.exec(&format!("INSERT OR IGNORE INTO plugins (plugin_id) VALUES ('{plugin}')"));
    fx.exec(&format!("INSERT OR IGNORE INTO plugin_providers (plugin_ref, name) SELECT plugin_ref, 'meta' FROM plugins WHERE plugin_id = '{plugin}'"));
    fx.exec(&format!("INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at) SELECT p.plugin_ref, v.provider_ref, {asset}, {target}, 0 FROM plugins p JOIN plugin_providers v USING (plugin_ref) WHERE p.plugin_id = '{plugin}' AND v.name = 'meta'"));
    fx.count(&format!("SELECT task_id FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = '{plugin}' AND asset_id = {asset}"))
}

fn outbox(s: &Setup) -> i64 {
    s.fx.count("SELECT COUNT(*) FROM search_outbox")
}

#[test]
fn t_prec_01_kv_tags_and_completion_are_recorded_atomically() {
    // Arrange
    let s = Setup::new();
    let before = outbox(&s);

    // Act
    let r = s.record(json!({"kv": {"mood": {"a": [1, "가"]}, "n": 3}, "tags": ["Alpha", "beta"]})).unwrap();

    // Assert
    assert_eq!((r.kv_entries, r.tags_added), (2, 2));
    assert_eq!(s.kv_rows(), 2);
    let v: String = s.fx.read(|c| mam_db::plugin_kv::get(c, "com.example.rec", Some(s.asset), "mood").unwrap().unwrap());
    assert_eq!(serde_json::from_str::<Value>(&v).unwrap(), json!({"a": [1, "가"]}));
    assert_eq!(s.tag_rows(), 2);
    assert_eq!((s.revision(), outbox(&s) - before), (2, 1), "태그 쓰기로 revision 1회 증가·outbox 1행");
    assert_eq!((r.done_rev, s.task_row()), (2, ("done".into(), 1, 2, 0)));
    assert_eq!(s.fx.count("SELECT priority FROM search_outbox ORDER BY event_id DESC LIMIT 1"), 1);
}

#[test]
fn t_prec_02_null_and_empty_object_outputs_just_complete_the_task() {
    for out in [Value::Null, json!({}), json!({"kv": {}})] {
        let s = Setup::new();
        let r = s.record(out.clone()).unwrap();
        assert_eq!((r.kv_entries, r.tags_added, r.done_rev), (0, 0, 1), "{out}");
        assert_eq!((s.kv_rows(), s.tag_rows(), s.revision()), (0, 0, 1));
        assert_eq!(s.task_row(), ("done".into(), 1, 1, 0));
    }
}

#[test]
fn t_prec_03_invalid_outputs_are_rejected_before_any_write_and_leave_the_task_untouched() {
    // Arrange
    let s = Setup::new();
    let too_many: serde_json::Map<String, Value> = (0..33).map(|i| (format!("k{i}"), json!(i))).collect();
    let big = "x".repeat(65_535); // compact JSON 은 따옴표 포함 65,537 바이트
    let cases = [
        ("unknown field", json!({"foo": 1})),
        ("unknown field next to a valid one", json!({"kv": {"a": 1}, "extra": true})),
        ("array", json!([1])),
        ("string", json!("tags")),
        ("33 keys", json!({"kv": Value::Object(too_many)})),
        ("empty kv key", json!({"kv": {"": 1}})),
        ("control char in key", json!({"kv": {"a\nb": 1}})),
        ("value over 64 KiB", json!({"kv": {"k": big}})),
        ("bad tag name", json!({"tags": ["ok", ""]})),
        ("control char in tag", json!({"tags": ["a\tb"]})),
        ("empty tags", json!({"tags": []})),
        ("21 tags", json!({"tags": (0..21).map(|i| format!("t{i}")).collect::<Vec<_>>()})),
        ("non-string tag", json!({"tags": [1]})),
        ("tags not an array", json!({"tags": "a"})),
        ("kv not an object", json!({"kv": [1]})),
    ];
    let before = outbox(&s);

    // Act / Assert
    for (name, out) in cases {
        let e = s.record(out).unwrap_err();
        assert!(matches!(e, RecordError::OutputInvalid(_)), "{name}: {e:?}");
        s.assert_untouched(before);
    }
}

#[test]
fn t_prec_04_boundaries_32_keys_20_tags_and_64_kib_values_are_accepted() {
    let s = Setup::new();
    let kv: serde_json::Map<String, Value> = (0..32).map(|i| (format!("k{i}"), json!(i))).collect();
    let big = "x".repeat(65_534); // 따옴표 포함 정확히 65,536 바이트
    let mut kv = kv;
    kv.insert("k0".into(), json!(big));
    let tags: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();

    let r = s.record(json!({"kv": Value::Object(kv), "tags": tags})).unwrap();

    assert_eq!((r.kv_entries, r.tags_added, s.kv_rows(), s.tag_rows()), (32, 20, 32, 20));
}

#[test]
fn t_prec_05_kv_output_with_kv_write_revoked_is_permission_denied_and_writes_nothing() {
    // Arrange: 게이트 ON — 활성이고 kv.write 만 허용하지 않았다 (나머지 권한은 허용)
    let s = Setup::new();
    let granted: Vec<Permission> = Permission::ALL.into_iter().filter(|p| *p != Permission::KvWrite).collect();
    s.fx.set_settings("com.example.rec", Some("true"), Some(&granted_json(&granted)));
    s.fx.gw.grants().refresh(&s.fx.pool, &s.plugin, soon()).unwrap();
    let before = outbox(&s);

    // Act / Assert: kv + tags 가 같이 있어도 태그까지 쓰지 않는다
    let e = s.record(json!({"kv": {"a": 1}, "tags": ["x"]})).unwrap_err();
    assert_eq!(e, RecordError::PermissionDenied(Permission::KvWrite));
    s.assert_untouched(before);
    // tags 만이면 tag.write 가 있으므로 통과, {} 는 권한이 필요 없다
    assert!(s.record(json!({"tags": ["x"]})).is_ok());
}

#[test]
fn t_prec_06_tags_with_tag_write_revoked_and_disabled_plugin_are_permission_denied() {
    let s = Setup::new();
    let granted: Vec<Permission> = Permission::ALL.into_iter().filter(|p| *p != Permission::TagWrite).collect();
    s.fx.set_settings("com.example.rec", Some("true"), Some(&granted_json(&granted)));
    s.fx.gw.grants().refresh(&s.fx.pool, &s.plugin, soon()).unwrap();
    let before = outbox(&s);
    assert_eq!(s.record(json!({"tags": ["x"]})).unwrap_err(), RecordError::PermissionDenied(Permission::TagWrite));
    s.assert_untouched(before);

    // 비활성이면 모든 쓰기 권한이 0
    s.fx.set_settings("com.example.rec", Some("false"), Some(&granted_json(&Permission::ALL)));
    s.fx.gw.grants().refresh(&s.fx.pool, &s.plugin, soon()).unwrap();
    assert_eq!(s.record(json!({"kv": {"a": 1}})).unwrap_err(), RecordError::PermissionDenied(Permission::KvWrite));
    s.assert_untouched(before);
}

#[test]
fn t_prec_07_kv_quota_is_enforced_with_the_delta_and_a_failed_write_does_not_move_the_meter() {
    // Arrange: 키 3자 + 값 52바이트 + 64 = 119 바이트/행, 상한 150
    let s = Setup::with_cfg(|c| c.kv_quota_bytes = 150);
    let v = "y".repeat(50);

    // Act / Assert
    assert!(s.record(json!({"kv": {"aaa": v}})).is_ok());
    let second = add_task(&s.fx, "com.example.rec", s.fx.add_asset("image", "a/y.jpg"), 1);
    let other_asset = s.fx.count("SELECT MAX(asset_id) FROM media_assets");
    let e = s.fx.gw.record_provider_output("com.example.rec", other_asset, json!({"kv": {"bbb": v}}), Completion { task_id: second, processed_rev: 1 }, soon()).unwrap_err();
    assert_eq!(e, RecordError::QuotaExceeded);
    assert_eq!(s.kv_rows(), 1, "거부된 쓰기는 남지 않는다");
    let status: String = s.fx.read(|c| c.query_row("SELECT status FROM plugin_tasks WHERE task_id = ?1", [second], |r| r.get(0)).unwrap());
    assert_eq!(status, "pending");
    // 같은 키를 줄여 덮어쓰기는 통과 (delta 만 센다)
    assert!(s.fx.gw.record_provider_output("com.example.rec", s.asset, json!({"kv": {"aaa": "z"}}), Completion { task_id: s.task, processed_rev: 1 }, soon()).is_ok());
}

#[test]
fn t_prec_08_a_tag_limit_failure_after_the_kv_write_rolls_the_kv_back_too() {
    // Arrange: 자산당 태그 상한 1 — kv 를 먼저 쓰고 태그에서 실패한다
    let s = Setup::with_cfg(|c| c.max_asset_tags = 1);
    let before = outbox(&s);

    // Act
    let e = s.record(json!({"kv": {"a": 1}, "tags": ["t1", "t2"]})).unwrap_err();

    // Assert
    assert_eq!(e, RecordError::QuotaExceeded);
    s.assert_untouched(before);
}

#[test]
fn t_prec_09_forced_db_failure_rolls_back_everything_and_the_meter_is_not_moved() {
    // Arrange: outbox 기록이 실패하도록 트리거 (KV·태그 부착·revision 증가 뒤에 실패). 상한 130 = 행 1개(119)만 들어간다
    let s = Setup::with_cfg(|c| c.kv_quota_bytes = 130);
    s.fx.exec("CREATE TRIGGER fail_outbox BEFORE INSERT ON search_outbox BEGIN SELECT RAISE(ABORT, 'forced'); END");
    let v = "y".repeat(50);

    // Act
    let e = s.record(json!({"kv": {"aaa": v}, "tags": ["x"]})).unwrap_err();

    // Assert: 롤백 — KV·태그·revision·작업 모두 그대로, 계량기도 그대로(트리거를 지우면 같은 쓰기가 상한 안에 든다)
    assert!(matches!(e, RecordError::Db(_)), "{e:?}");
    s.assert_untouched(0);
    s.fx.exec("DROP TRIGGER fail_outbox");
    assert!(s.record(json!({"kv": {"aaa": v}})).is_ok(), "실패한 쓰기가 계량기를 올렸다면 여기서 QuotaExceeded");
}

#[test]
fn t_prec_10_plugin_without_installed_key_is_not_installed_and_nothing_is_written() {
    // Arrange: 제거 경합 — plugin_installed 키가 없다 (권한 레지스트리는 아직 남아 있을 수 있다)
    let s = Setup::new();
    s.fx.exec("DELETE FROM settings WHERE key = 'plugin_installed:com.example.rec'");
    let before = outbox(&s);

    // Act / Assert: 출력 종류와 무관하게 NotInstalled, 쓰기·완료 없음
    for out in [Value::Null, json!({"kv": {"a": 1}, "tags": ["x"]})] {
        assert_eq!(s.record(out).unwrap_err(), RecordError::NotInstalled);
        s.assert_untouched(before);
    }
}

#[test]
fn t_prec_11_own_tag_event_does_not_repend_this_plugins_task_but_other_plugins_get_one() {
    // Arrange: 같은 자산에 플러그인 A·B 의 작업 (대상 rev 1)
    let s = Setup::new();
    let b_task = {
        s.fx.full_plugin("com.example.other");
        install(&s.fx, "com.example.other");
        add_task(&s.fx, "com.example.other", s.asset, 1)
    };

    // Act: A 가 태그를 붙여 완료 -> revision 2, outbox 이벤트 1건 -> 피드가 그 이벤트를 두 플러그인에 공급
    let r = s.record(json!({"tags": ["auto"]})).unwrap();
    let upto = s.fx.count("SELECT MAX(event_id) FROM search_outbox");
    let feed = |plugin: &'static str| {
        s.fx.pool.write(Priority::Low, move |c| Ok(with_immediate_tx(c, |tx| plugin_tasks::feed_batch(tx, plugin, &[("meta", &["image"])], 0, upto, 7))?)).unwrap()
    };
    let (a_pending, b_pending) = (feed("com.example.rec"), feed("com.example.other"));

    // Assert
    assert_eq!(r.done_rev, 2);
    assert!(a_pending.is_empty(), "자기 이벤트는 자기 작업을 다시 대기로 만들지 않는다");
    assert_eq!(s.task_row(), ("done".into(), 2, 2, 0));
    assert_eq!(b_pending, vec![s.asset]);
    let b: (String, i64) = s.fx.read(|c| c.query_row("SELECT status, target_revision FROM plugin_tasks WHERE task_id = ?1", [b_task], |r| Ok((r.get(0)?, r.get(1)?))).unwrap());
    assert_eq!(b, ("pending".into(), 2));
}

#[test]
fn t_prec_12_done_revision_never_exceeds_processed_without_a_tag_write_and_target_above_done_stays_pending() {
    // Arrange: 태그를 이미 사용자가 붙여 둠 -> 플러그인 태그는 no-op (revision 불변). 그 사이 target 이 7 로 올랐다
    let s = Setup::new();
    s.record(json!({"tags": ["same"]})).unwrap();
    s.fx.exec(&format!("UPDATE plugin_tasks SET status = 'pending', target_revision = 7, done_revision = 2 WHERE task_id = {}", s.task));
    let (rev, before) = (s.revision(), outbox(&s));

    // Act: 같은 태그 재기록 (processed rev 5)
    let r = s.fx.gw.record_provider_output("com.example.rec", s.asset, json!({"tags": ["same"]}), Completion { task_id: s.task, processed_rev: 5 }, soon()).unwrap();

    // Assert: 붙인 것이 없으니 done_rev = processed(5) — 자산 revision 으로 올리지 않는다. target 7 > 5 -> 계속 pending
    assert_eq!(r.tags_added, 0);
    assert_eq!((r.done_rev, s.revision(), outbox(&s)), (5, rev, before));
    assert_eq!(s.task_row(), ("pending".into(), 7, 5, 0));
    // 늦은 낮은 revision 완료는 done 을 낮추지 않는다
    s.fx.gw.record_provider_output("com.example.rec", s.asset, Value::Null, Completion { task_id: s.task, processed_rev: 3 }, soon()).unwrap();
    assert_eq!(s.task_row(), ("pending".into(), 7, 5, 0));
}

#[test]
fn t_prec_13_missing_task_inactive_asset_and_expired_deadline_write_nothing() {
    let s = Setup::new();
    let before = outbox(&s);

    // 작업 행이 없다 (provider 제거로 삭제됨) -> TaskGone, KV·태그 롤백
    let e = s.fx.gw.record_provider_output("com.example.rec", s.asset, json!({"kv": {"a": 1}, "tags": ["x"]}), Completion { task_id: 9_999, processed_rev: 1 }, soon()).unwrap_err();
    assert_eq!(e, RecordError::TaskGone);
    s.assert_untouched(before);

    // 마감이 이미 지났다 -> Busy, 이후에도 실행되지 않는다
    let e = s.fx.gw.record_provider_output("com.example.rec", s.asset, json!({"kv": {"a": 1}}), s.done(), Instant::now()).unwrap_err();
    assert_eq!(e, RecordError::Busy);
    s.assert_untouched(before);

    // 병합·삭제된 자산 -> AssetInactive
    s.fx.exec(&format!("UPDATE media_assets SET status = 'superseded' WHERE asset_id = {}", s.asset));
    assert_eq!(s.record(json!({"kv": {"a": 1}})).unwrap_err(), RecordError::AssetInactive);
    assert_eq!((s.kv_rows(), s.task_row().0), (0, "pending".to_owned()));
}

#[test]
fn t_prec_14_an_edit_made_while_the_provider_ran_is_not_swallowed_by_the_loop_guard() {
    // Arrange: provider 는 revision 1 을 보고 실행 중이다. 그 사이 사용자 편집이 revision 2 와 outbox 이벤트를 만들었다
    let s = Setup::new();
    s.fx.exec(&format!("UPDATE media_assets SET revision = 2 WHERE asset_id = {}", s.asset));
    s.fx.exec(&format!("INSERT INTO search_outbox (asset_id, revision, event_type, created_at) VALUES ({}, 2, 'upsert', 0)", s.asset));

    // Act: provider 가 태그를 붙여 완료 (자기 이벤트 revision 3)
    let r = s.record(json!({"tags": ["auto"]})).unwrap();
    let upto = s.fx.count("SELECT MAX(event_id) FROM search_outbox");
    let pending = s.fx.pool.write(Priority::Low, move |c| Ok(with_immediate_tx(c, |tx| plugin_tasks::feed_batch(tx, "com.example.rec", &[("meta", &["image"])], 0, upto, 7))?)).unwrap();

    // Assert: 예전에는 done_rev = 3 이라 revision 2 의 사용자 편집을 provider 가 다시 보지 못했다
    assert_eq!((r.tags_added, r.done_rev, s.revision()), (1, 1, 3));
    assert_eq!(pending, vec![s.asset], "the unseen edit re-pends the task");
    assert_eq!(s.task_row(), ("pending".into(), 3, 1, 0));
}
