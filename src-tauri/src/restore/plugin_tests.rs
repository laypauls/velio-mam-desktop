// Categories: Business rule (01 설정 병합 매트릭스, 02 설치되지 않은 플러그인 데이터 삭제, 04 철회) /
// Validation (03 구 표지 호환·로그 안전, 05 V26 이전 스키마 백업) / Concurrency (06 재적용 멱등).
// 실제 SQLite 임시 파일 + 주입 시계. 사용자 복원은 표지 + `apply_pending` 으로 실제 경로를 탄다.
use std::collections::{BTreeMap, BTreeSet};

use mam_core::storage::DataPaths;
use mam_db::plugin_settings::{
    ENABLED, FEED_CURSOR, GRANTED, INSTALLED, granted_to_raw, load_prefixed, put,
};
use mam_db::{BackupInfo, BackupKind, create_backup};
use rusqlite::Connection;

use super::post::read_kept;
use super::test_support::*;
use super::{KeptState, RestoreMarker, StartupRestore, apply_pending, write_marker};
use crate::startup::Boot;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn state(
    c: &Connection,
    id: &str,
    version: &str,
    enabled: bool,
    granted: &[&str],
    cursor: Option<i64>,
) {
    put(
        c,
        INSTALLED,
        id,
        &format!("{{\"version\":\"{version}\",\"installedAt\":1}}"),
    )
    .unwrap();
    put(c, ENABLED, id, &enabled.to_string()).unwrap();
    put(c, GRANTED, id, &granted_to_raw(&set(granted))).unwrap();
    if let Some(n) = cursor {
        put(c, FEED_CURSOR, id, &n.to_string()).unwrap();
    }
}

fn wipe_plugin_settings(c: &Connection) {
    c.execute(
        "DELETE FROM settings WHERE key LIKE 'plugin\\_%' ESCAPE '\\'",
        [],
    )
    .unwrap();
}

fn plugin_data(c: &Connection, plugin: &str, assets: &[i64]) {
    let plugin_ref = mam_db::plugin_registry::ensure(c, plugin).unwrap();
    let provider_ref = mam_db::plugin_registry::ensure_provider(c, plugin_ref, "p").unwrap();
    for a in assets {
        c.execute(
            "INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (?1, ?2, 'k', '1', 0)",
            (plugin_ref, a),
        )
        .unwrap();
        c.execute(
            "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at) VALUES (?1, ?2, ?3, 1, 0)",
            (plugin_ref, provider_ref, a),
        )
        .unwrap();
    }
    c.execute(
        "INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (?1, NULL, 'g', '1', 0)",
        [plugin_ref],
    )
    .unwrap();
}

fn backup_now(paths: &DataPaths) -> BackupInfo {
    create_backup(
        &paths.db(),
        &paths.backups(),
        BackupKind::Auto,
        7,
        &[],
        &TestClock::new(),
    )
    .unwrap()
}

/// 현재 값을 읽어 표지를 쓰고 기동 경로로 복원을 적용한다
fn restore_with_current(paths: &DataPaths, info: &BackupInfo, kept: KeptState) {
    let marker = RestoreMarker {
        requested_at: T0,
        backup: paths.backups().join(&info.file_name),
        backup_file: info.file_name.clone(),
        backup_created_at: info.created_at,
        kept,
        attempts: 0,
    };
    write_marker(paths, &marker).unwrap();
    assert_eq!(run_boot(paths, &TestClock::new()), Boot::Run);
}

fn raw(paths: &DataPaths, prefix: &str) -> BTreeMap<String, String> {
    load_prefixed(&conn(paths), prefix).unwrap()
}

#[test]
fn t_restore_plugin_01_settings_merge_follows_the_most_restrictive_side() {
    // Arrange: 백업 시점 상태 -> 백업 -> 현재 상태 (설치 목록·버전·활성·권한이 다르다)
    let (_d, paths) = data_dir();
    {
        let c = conn(&paths);
        state(&c, "tt", "1.0.0", true, &["a", "b", "c"], Some(42));
        state(&c, "tf", "1.0.0", false, &["a"], Some(7)); // 백업은 꺼짐
        state(&c, "ft", "1.0.0", true, &["a"], None); // 현재는 꺼짐
        state(&c, "ff", "1.0.0", false, &[], None);
        state(&c, "old", "1.0.0", true, &["a"], Some(9)); // 현재 설치되지 않음
    }
    let info = backup_now(&paths);
    {
        let c = conn(&paths);
        wipe_plugin_settings(&c);
        state(&c, "tt", "2.0.0", true, &["b", "c", "d"], Some(1000)); // 현재: 새 버전, 권한 b c d
        state(&c, "tf", "2.0.0", true, &["a"], None);
        state(&c, "ft", "2.0.0", false, &["a"], None);
        state(&c, "ff", "2.0.0", false, &[], None);
        state(&c, "newer", "1.0.0", true, &["x"], None); // 백업에 없는 플러그인
    }
    let kept = read_kept(&conn(&paths)).unwrap();

    // Act
    restore_with_current(&paths, &info, kept);

    // Assert: installed = 현재 값, enabled = 현재 && 백업, granted = 현재 ∩ 백업, cursor = 백업 값
    let installed = raw(&paths, INSTALLED);
    assert_eq!(
        installed.keys().cloned().collect::<Vec<_>>(),
        ["ff", "ft", "newer", "tf", "tt"]
    );
    assert!(installed["tt"].contains("2.0.0") && installed["newer"].contains("1.0.0"));
    let enabled: BTreeMap<_, _> = raw(&paths, ENABLED).into_iter().collect();
    assert_eq!(
        enabled,
        BTreeMap::from([
            ("ff".to_owned(), "false".to_owned()),
            ("ft".to_owned(), "false".to_owned()),
            ("newer".to_owned(), "false".to_owned()), // 백업에 키 없음 = false
            ("tf".to_owned(), "false".to_owned()),
            ("tt".to_owned(), "true".to_owned()),
        ])
    );
    let granted = raw(&paths, GRANTED);
    assert_eq!(granted["tt"], "[\"b\",\"c\"]");
    assert_eq!(granted["tf"], "[\"a\"]");
    assert_eq!(granted["newer"], "[]", "nothing was granted in the backup");
    assert_eq!(granted["ff"], "[]");
    let cursors = raw(&paths, FEED_CURSOR);
    assert_eq!(cursors.get("tt").map(String::as_str), Some("42"));
    assert_eq!(cursors.get("tf").map(String::as_str), Some("7"));
    assert!(!cursors.contains_key("old") && !cursors.contains_key("newer"));
    assert!(
        !installed.contains_key("old")
            && !enabled.contains_key("old")
            && !granted.contains_key("old")
    );
}

#[test]
fn t_restore_plugin_02_rows_of_plugins_that_are_not_installed_now_are_deleted_after_restore() {
    // Arrange: 백업에는 old·tt 의 KV·작업이 있다 (자산 3개, 전역 KV 포함). 현재 설치: tt 만
    let (_d, paths) = data_dir();
    {
        let c = conn(&paths);
        let assets: Vec<i64> = (0..3).map(|_| insert_asset(&c)).collect();
        plugin_data(&c, "old", &assets);
        plugin_data(&c, "tt", &assets[..2]);
        state(&c, "old", "1.0.0", true, &[], None);
        state(&c, "tt", "1.0.0", true, &[], None);
    }
    let info = backup_now(&paths);
    {
        let c = conn(&paths);
        wipe_plugin_settings(&c);
        state(&c, "tt", "1.0.0", true, &[], None);
    }
    let kept = read_kept(&conn(&paths)).unwrap();

    // Act
    restore_with_current(&paths, &info, kept);

    // Assert: old 는 두 표에서 모두 사라지고 tt 는 그대로
    for table in ["plugin_kv", "plugin_tasks"] {
        assert_eq!(
            count(
                &paths,
                &format!(
                    "SELECT COUNT(*) FROM {table} JOIN plugins USING (plugin_ref) WHERE plugin_id = 'old'"
                )
            ),
            0,
            "{table}"
        );
    }
    assert_eq!(
        count(
            &paths,
            "SELECT COUNT(*) FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = 'tt'"
        ),
        3,
        "2 asset rows + 1 global"
    );
    assert_eq!(
        count(
            &paths,
            "SELECT COUNT(*) FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = 'tt'"
        ),
        2
    );
    assert_eq!(
        count(&paths, "SELECT COUNT(*) FROM media_assets"),
        3,
        "assets are never touched"
    );
}

#[test]
fn t_restore_plugin_03_old_markers_without_plugins_still_deserialize_and_debug_hides_values() {
    // Arrange: `plugins` 키가 없는 구 표지 (ai_keys 도 없는 더 오래된 형태)
    let json = r#"{"passwordHash":null,"authExemptMinutes":null,"consent":{"given":false,"at":null},"sequences":{}}"#;

    // Act
    let kept: KeptState = serde_json::from_str(json).unwrap();

    // Assert: 설치 없음 -> 복원 시 모든 플러그인 데이터가 정리 대상이다 (fail closed)
    assert!(kept.plugins.installed.is_empty() && kept.plugins.enabled.is_empty());
    let mut with = kept.clone();
    with.plugins
        .installed
        .insert("p".into(), "{\"version\":\"SECRET-1.2.3\"}".into());
    let shown = format!("{with:?}");
    assert!(!shown.contains("SECRET-1.2.3"), "{shown}");
    // 직렬화 왕복
    let back: KeptState = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
    assert_eq!(back, with);
}

#[test]
fn t_restore_plugin_04_auto_recovery_disables_every_plugin_and_clears_grants_keeping_installs() {
    // Arrange: 활성 + 권한이 있는 플러그인 2개를 담은 백업, 그 뒤 DB 손상
    let (_d, paths) = data_dir();
    {
        let c = conn(&paths);
        state(&c, "a", "1.0.0", true, &["x"], Some(5));
        state(&c, "b", "1.0.0", false, &["y"], None);
    }
    backup_now(&paths);
    overwrite_with_garbage(&paths);

    // Act
    assert_eq!(run_boot(&paths, &TestClock::new()), Boot::Run);

    // Assert
    assert_eq!(
        raw(&paths, ENABLED),
        BTreeMap::from([("a".into(), "false".into()), ("b".into(), "false".into())])
    );
    assert!(raw(&paths, GRANTED).is_empty());
    assert_eq!(
        raw(&paths, INSTALLED).len(),
        2,
        "installs and cursors stay; the user re-enables and re-grants"
    );
    assert_eq!(raw(&paths, FEED_CURSOR).len(), 1);
}

#[test]
fn t_restore_plugin_05_backup_from_before_the_plugin_tables_restores_without_error() {
    // T-pref-10 (pre-V26 backup: purge_uninstalled is a no-op; all four plugin tables are dropped below)
    // Arrange: V26 표가 없는 백업을 흉내 낸다 (표를 지운 뒤 백업) — 현재는 플러그인이 설치돼 있다
    let (_d, paths) = data_dir();
    {
        let c = conn(&paths);
        c.execute_batch("DROP TABLE plugin_tasks; DROP TABLE plugin_kv; DROP TABLE plugin_providers; DROP TABLE plugins;").unwrap();
    }
    let info = backup_now(&paths);
    let kept = {
        let c = conn(&paths);
        state(&c, "p", "1.0.0", true, &["x"], None);
        read_kept(&c).unwrap()
    };

    // Act: 복원 후처리는 표 유무를 확인하고 건너뛴다
    let marker = RestoreMarker {
        requested_at: T0,
        backup: paths.backups().join(&info.file_name),
        backup_file: info.file_name.clone(),
        backup_created_at: info.created_at,
        kept,
        attempts: 0,
    };
    write_marker(&paths, &marker).unwrap();
    let applied = apply_pending(&paths).unwrap();

    // Assert
    assert_eq!(applied, StartupRestore::Applied(info.file_name));
    assert_eq!(
        raw(&paths, INSTALLED).keys().cloned().collect::<Vec<_>>(),
        ["p"]
    );
    assert_eq!(
        raw(&paths, ENABLED).get("p").map(String::as_str),
        Some("false"),
        "not enabled in the backup"
    );
}

#[test]
fn t_restore_plugin_06_applying_the_same_marker_twice_gives_the_same_result() {
    // Arrange
    let (_d, paths) = data_dir();
    {
        let c = conn(&paths);
        let a = insert_asset(&c);
        plugin_data(&c, "old", &[a]);
        state(&c, "tt", "1.0.0", true, &["a", "b"], Some(3));
    }
    let info = backup_now(&paths);
    let kept = {
        let c = conn(&paths);
        wipe_plugin_settings(&c);
        state(&c, "tt", "1.0.0", true, &["b"], None);
        read_kept(&c).unwrap()
    };
    let marker = RestoreMarker {
        requested_at: T0,
        backup: paths.backups().join(&info.file_name),
        backup_file: info.file_name.clone(),
        backup_created_at: info.created_at,
        kept: kept.clone(),
        attempts: 0,
    };

    // Act: 두 번 적용 (중단 후 재시도 흉내) — 두 번째는 이미 복원된 DB 위에 후처리만 다시
    write_marker(&paths, &marker).unwrap();
    apply_pending(&paths).unwrap();
    let first = (
        raw(&paths, GRANTED),
        raw(&paths, ENABLED),
        raw(&paths, FEED_CURSOR),
    );
    super::post::finish_user_restore(&paths, &kept).unwrap();

    // Assert
    assert_eq!(
        (
            raw(&paths, GRANTED),
            raw(&paths, ENABLED),
            raw(&paths, FEED_CURSOR)
        ),
        first
    );
    assert_eq!(first.0["tt"], "[\"b\"]");
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM plugin_kv"), 0);
}

#[test]
fn t_pref_08_restore_uses_the_backups_own_registry_even_when_the_integer_refs_differ_from_the_current_db()
 {
    // Arrange: 백업 레지스트리 B=1, A=2 (둘 다 데이터 있음). 현재 DB 는 A=1, B=2 로 다시 만든 상태 — 현재 설치: A 만
    let (_d, paths) = data_dir();
    {
        let c = conn(&paths);
        let asset = insert_asset(&c);
        for id in ["com.b", "com.a"] {
            plugin_data(&c, id, &[asset]);
            mam_db::plugin_kv::upsert(&c, id, None, "marker", "\"backup\"", 0).unwrap();
        }
        state(&c, "com.a", "1.0.0", true, &[], None);
        assert_eq!(
            mam_db::plugin_registry::lookup(&c, "com.b").unwrap(),
            Some(1)
        );
        assert_eq!(
            mam_db::plugin_registry::lookup(&c, "com.a").unwrap(),
            Some(2)
        );
    }
    let info = backup_now(&paths);
    let kept = {
        let c = conn(&paths);
        c.execute_batch("DELETE FROM plugin_tasks; DELETE FROM plugin_kv; DELETE FROM plugin_providers; DELETE FROM plugins;").unwrap();
        for id in ["com.a", "com.b"] {
            mam_db::plugin_kv::upsert(&c, id, None, "marker", "\"current\"", 0).unwrap();
        }
        assert_eq!(
            mam_db::plugin_registry::lookup(&c, "com.a").unwrap(),
            Some(1)
        );
        wipe_plugin_settings(&c);
        state(&c, "com.a", "1.0.0", true, &[], None);
        read_kept(&c).unwrap()
    };

    // Act
    restore_with_current(&paths, &info, kept);

    // Assert: A 는 백업 데이터(문자열 API 로 읽는다), B 는 지워지고, 레지스트리 행은 남으며, FK 위반 없음
    let c = conn(&paths);
    assert_eq!(
        mam_db::plugin_kv::get(&c, "com.a", None, "marker")
            .unwrap()
            .as_deref(),
        Some("\"backup\"")
    );
    assert_eq!(
        mam_db::plugin_kv::get(&c, "com.b", None, "marker").unwrap(),
        None
    );
    assert_eq!(
        mam_db::plugin_registry::lookup(&c, "com.a").unwrap(),
        Some(2),
        "백업 DB 의 정수 키 그대로"
    );
    assert_eq!(
        mam_db::plugin_data::data_plugin_ids(&c).unwrap(),
        set(&["com.a"])
    );
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM plugin_tasks"), 1);
    assert_eq!(
        count(&paths, "SELECT COUNT(*) FROM pragma_foreign_key_check"),
        0
    );
}

#[test]
fn t_pref_09_a_plugin_installed_now_but_absent_from_the_backup_registry_gets_a_fresh_ref_on_first_write()
 {
    // Arrange: 백업에는 레지스트리 행이 없다 (플러그인 데이터 없음). 현재는 `new` 가 설치돼 있다
    let (_d, paths) = data_dir();
    let info = backup_now(&paths);
    let kept = {
        let c = conn(&paths);
        state(&c, "new", "1.0.0", true, &["kv.write"], None);
        read_kept(&c).unwrap()
    };
    restore_with_current(&paths, &info, kept);

    // Act: 복원 뒤 첫 쓰기 (게이트웨이 `kv.set` 과 같은 함수)
    let c = conn(&paths);
    assert_eq!(
        mam_db::plugin_registry::lookup(&c, "new").unwrap(),
        None,
        "읽기만으로는 행이 생기지 않는다"
    );
    mam_db::plugin_kv::upsert(&c, "new", None, "k", "1", 0).unwrap();

    // Assert
    assert_eq!(
        mam_db::plugin_kv::get(&c, "new", None, "k")
            .unwrap()
            .as_deref(),
        Some("1")
    );
    assert_eq!(
        count(&paths, "SELECT COUNT(*) FROM pragma_foreign_key_check"),
        0
    );
}
