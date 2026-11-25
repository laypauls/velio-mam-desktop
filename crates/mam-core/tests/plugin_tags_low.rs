//! `UserData::add_tags_low` (T-ptag-*) — 플러그인 태그 추가: 한 트랜잭션, Low 쓰기, outbox priority 1, 상한·마감·원자성.
//!
//! 분류: Happy / Business rule(사용자 태그와 같은 tag_id, 출처 미기록) / Boundary(상한) / Validation(이름) / Time(마감) / External(강제 실패 롤백)

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::env::Env;
use common::purge::{exec, insert_assets};
use mam_core::userdata::{PluginTagError, PluginTagLimits, UpdateTagsInput, UserData, add_tags_tx};

const LIMITS: PluginTagLimits = PluginTagLimits { max_per_asset: 100, max_new_names: 200 };

fn setup(n: usize) -> (Env, UserData, Vec<i64>) {
    let env = Env::new();
    std::fs::create_dir_all(env.path("lib")).unwrap();
    env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap();
    let ids = insert_assets(&env, "vol-a", "lib", n, "image");
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    (env, ud, ids)
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

fn revision(env: &Env, id: i64) -> i64 {
    env.read(|c| c.query_row("SELECT revision FROM media_assets WHERE asset_id = ?1", [id], |r| r.get(0)).unwrap())
}

#[test]
fn t_ptag_01_adds_tags_bumps_revision_once_and_writes_one_outbox_row_with_priority_1() {
    // Arrange
    let (env, ud, ids) = setup(1);
    let (before_rev, before_outbox) = (revision(&env, ids[0]), env.count("SELECT COUNT(*) FROM search_outbox"));
    // Act
    let out = ud.add_tags_low(ids[0], &names(&["Alpha", "beta"]), LIMITS, soon()).unwrap();
    // Assert
    assert_eq!((out.added, out.new_names), (2, 2));
    assert_eq!(revision(&env, ids[0]), before_rev + 1, "두 태그라도 revision 은 한 번");
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox") - before_outbox, 1);
    let (rev, prio): (i64, i64) = env.read(|c| {
        c.query_row("SELECT revision, priority FROM search_outbox WHERE asset_id = ?1 ORDER BY event_id DESC LIMIT 1", [ids[0]], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap()))).unwrap()
    });
    assert_eq!((rev, prio), (revision(&env, ids[0]), 1));
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_tags"), 2);
}

#[test]
fn t_ptag_02_repeat_is_a_no_op_and_matches_user_tags_case_and_nfc_insensitively() {
    // Arrange: 사용자 태그 "Café"(NFC)
    let (env, ud, ids) = setup(2);
    ud.update_asset_tags(UpdateTagsInput { asset_ids: vec![ids[1]], add: vec!["Café".into()], remove: vec![] }).unwrap();
    // Act: NFD + 대문자 변형으로 플러그인이 추가
    let nfd = "CAFE\u{301}";
    let first = ud.add_tags_low(ids[0], &names(&[nfd]), LIMITS, soon()).unwrap();
    let rev = revision(&env, ids[0]);
    let outbox = env.count("SELECT COUNT(*) FROM search_outbox");
    let again = ud.add_tags_low(ids[0], &names(&[nfd, "café"]), LIMITS, soon()).unwrap();
    // Assert
    assert_eq!((first.added, first.new_names), (1, 0), "기존 사용자 태그 재사용");
    assert_eq!((again.added, again.new_names), (0, 0));
    assert_eq!(revision(&env, ids[0]), rev);
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox"), outbox);
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 1);
}

#[test]
fn t_ptag_03_invalid_names_and_counts_are_rejected_before_any_write() {
    let (env, ud, ids) = setup(1);
    let long = "a".repeat(101);
    for bad in [vec![String::new()], vec!["  ".into()], vec![long], vec!["a\nb".into()], vec!["ok".into(), "bad\u{0}".into()], vec![]] {
        let e = ud.add_tags_low(ids[0], &bad, LIMITS, soon()).unwrap_err();
        assert!(matches!(e, PluginTagError::InvalidName), "{bad:?}: {e:?}");
    }
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 0);
}

#[test]
fn t_ptag_04_missing_or_inactive_asset_is_not_found_and_writes_nothing() {
    let (env, ud, ids) = setup(2);
    exec(&env, &format!("UPDATE media_assets SET status = 'deleted' WHERE asset_id = {}", ids[1]));
    for id in [ids[1], 987_654] {
        let e = ud.add_tags_low(id, &names(&["x"]), LIMITS, soon()).unwrap_err();
        assert!(matches!(e, PluginTagError::NotFound), "{e:?}");
    }
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 0);
}

#[test]
fn t_ptag_05_per_asset_limit_counts_only_new_attachments_and_rolls_back() {
    // Arrange: 상한 3
    let (env, ud, ids) = setup(1);
    let lim = PluginTagLimits { max_per_asset: 3, max_new_names: 200 };
    ud.add_tags_low(ids[0], &names(&["a", "b"]), lim, soon()).unwrap();
    // Act / Assert: 이미 붙은 것 + 새 2개 = 4 > 3 → 거부, 새 태그도 남지 않는다
    let e = ud.add_tags_low(ids[0], &names(&["a", "c", "d"]), lim, soon()).unwrap_err();
    assert!(matches!(e, PluginTagError::AssetTagLimit), "{e:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 2);
    // 상한까지는 허용(경계)
    assert_eq!(ud.add_tags_low(ids[0], &names(&["a", "c"]), lim, soon()).unwrap().added, 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_tags"), 3);
}

#[test]
fn t_ptag_06_new_name_budget_only_counts_names_that_do_not_exist_yet() {
    let (env, ud, ids) = setup(2);
    ud.add_tags_low(ids[1], &names(&["old"]), LIMITS, soon()).unwrap();
    let lim = PluginTagLimits { max_per_asset: 100, max_new_names: 1 };
    let e = ud.add_tags_low(ids[0], &names(&["n1", "n2"]), lim, soon()).unwrap_err();
    assert!(matches!(e, PluginTagError::NewNameBudget), "{e:?}");
    let ok = ud.add_tags_low(ids[0], &names(&["old", "n1"]), lim, soon()).unwrap();
    assert_eq!((ok.added, ok.new_names), (2, 1));
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 2);
}

#[test]
fn t_ptag_07_expired_deadline_is_busy_and_writes_nothing() {
    let (env, ud, ids) = setup(1);
    let e = ud.add_tags_low(ids[0], &names(&["x"]), LIMITS, Instant::now()).unwrap_err();
    assert!(matches!(e, PluginTagError::Busy), "{e:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 0);
}

#[test]
fn t_ptag_08_forced_failure_after_tag_insert_rolls_back_tags_links_revision_and_outbox() {
    // Arrange: outbox 기록이 실패하도록 트리거를 둔다 (태그·부착·revision 증가 뒤에 실패)
    let (env, ud, ids) = setup(1);
    exec(&env, "CREATE TRIGGER fail_outbox BEFORE INSERT ON search_outbox BEGIN SELECT RAISE(ABORT, 'forced'); END");
    let (rev, outbox) = (revision(&env, ids[0]), env.count("SELECT COUNT(*) FROM search_outbox"));
    // Act
    let e = ud.add_tags_low(ids[0], &names(&["x", "y"]), LIMITS, soon()).unwrap_err();
    // Assert
    assert!(matches!(e, PluginTagError::Internal(_)), "{e:?}");
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_tags"), 0);
    assert_eq!(revision(&env, ids[0]), rev);
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox"), outbox);
}

#[test]
fn t_ptag_09_add_tags_tx_joins_the_callers_transaction_and_rolls_back_with_it() {
    // Arrange: 호출자 트랜잭션에서 태그 + 호출자의 다른 쓰기, 한 번은 롤백·한 번은 커밋
    let (env, _ud, ids) = setup(1);
    let (rev, outbox) = (revision(&env, ids[0]), env.count("SELECT COUNT(*) FROM search_outbox"));
    let run = |commit: bool| {
        let id = ids[0];
        env.pool
            .write(mam_db::Priority::Low, move |c| {
                let tx = c.transaction()?;
                let out = add_tags_tx(&tx, id, &names(&["Alpha", "beta"]), LIMITS, 5).unwrap();
                tx.execute("INSERT INTO settings (key, value) VALUES ('caller_marker', '1') ON CONFLICT(key) DO NOTHING", [])?;
                if commit {
                    tx.commit()?;
                }
                Ok(out.added)
            })
            .unwrap()
    };

    // Act / Assert: 롤백 — 태그·부착·revision·outbox·호출자 쓰기 모두 없음
    assert_eq!(run(false), 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(revision(&env, ids[0]), rev);
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox"), outbox);
    assert_eq!(env.count("SELECT COUNT(*) FROM settings WHERE key = 'caller_marker'"), 0);
    // 커밋 — 같이 반영, revision 1회 증가·outbox 1행
    assert_eq!(run(true), 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_tags"), 2);
    assert_eq!(revision(&env, ids[0]), rev + 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox"), outbox + 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM settings WHERE key = 'caller_marker'"), 1);
}

#[test]
fn t_ptag_10_add_tags_tx_rejects_invalid_names_and_inactive_assets_without_writing() {
    let (env, _ud, ids) = setup(1);
    env.pool
        .write(mam_db::Priority::Low, move |c| {
            let tx = c.transaction()?;
            let bad = add_tags_tx(&tx, ids[0], &names(&["ok", ""]), LIMITS, 5).unwrap_err();
            assert!(matches!(bad, PluginTagError::InvalidName), "{bad:?}");
            let missing = add_tags_tx(&tx, 9_999, &names(&["ok"]), LIMITS, 5).unwrap_err();
            assert!(matches!(missing, PluginTagError::NotFound), "{missing:?}");
            Ok(())
        })
        .unwrap();
    assert_eq!(env.count("SELECT COUNT(*) FROM tags"), 0);
}
