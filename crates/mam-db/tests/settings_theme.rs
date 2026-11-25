//! 앱 테마 설정 `theme` 통합 테스트 (T-settings-theme-01 ~ 05).
//!
//! 분류: Happy (01 기본값, 02 5모드 x 2systemDark 왕복, 04 저장 후 재로드, 05 gray 모드 수용) /
//! Validation (03 잘못된 저장값 -> 객체 전체 기본값 + 경고, 잘못된 패치 거부,
//! 05 systemDark 로 gray 는 거부).

mod common;

use common::migrated_db;
use mam_db::settings;
use mam_db::{SettingsError, ThemeMode, ThemeSetting, ThemeSystemDark};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

const KEY: &str = "theme";

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn put_raw(conn: &Connection, raw: &str) {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)",
        [KEY, raw],
    )
    .unwrap();
}

#[test]
fn t_settings_theme_01_default_is_system_with_dim_when_key_absent() {
    // Arrange
    let db = migrated_db();

    // Act
    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert_eq!(
        loaded.theme,
        ThemeSetting {
            mode: ThemeMode::System,
            system_dark: ThemeSystemDark::Dim
        }
    );
    assert_eq!(ThemeSetting::default(), loaded.theme);
    assert!(warnings.is_empty());
    let v = serde_json::to_value(&loaded).unwrap();
    assert_eq!(v["theme"], json!({"mode": "system", "systemDark": "dim"}));
}

#[test]
fn t_settings_theme_02_every_mode_and_system_dark_combination_round_trips() {
    // Arrange
    let mut db = migrated_db();
    let modes = [
        ("system", ThemeMode::System),
        ("light", ThemeMode::Light),
        ("gray", ThemeMode::Gray),
        ("dim", ThemeMode::Dim),
        ("dark", ThemeMode::Dark),
    ];
    let darks = [
        ("dim", ThemeSystemDark::Dim),
        ("dark", ThemeSystemDark::Dark),
    ];
    for (m, mode) in modes {
        for (d, system_dark) in darks {
            // Act
            let updated = settings::update(
                &mut db.conn,
                &patch(json!({"theme": {"mode": m, "systemDark": d}})),
            )
            .unwrap();
            let (loaded, warnings) = settings::load(&db.conn).unwrap();

            // Assert
            let want = ThemeSetting { mode, system_dark };
            assert_eq!(updated.theme, want, "{m}/{d}");
            assert_eq!(loaded.theme, want, "{m}/{d}");
            assert!(warnings.is_empty());
        }
    }
}

#[test]
fn t_settings_theme_03_invalid_stored_value_falls_back_to_whole_default_with_warning() {
    for raw in [
        r#"{"mode":"purple","systemDark":"dark"}"#, // 잘못된 mode (유효한 systemDark 도 버려진다)
        r#"{"mode":"dark","systemDark":"system"}"#, // 잘못된 systemDark
        r#"{"mode":"dark"}"#,                       // systemDark 누락
        r#"{"systemDark":"dark"}"#,                 // mode 누락
        r#"{"mode":1,"systemDark":"dark"}"#,        // 잘못된 타입
        r#""dark""#,                                // 객체 아님
        "null",
        "not json",
    ] {
        // Arrange
        let db = migrated_db();
        put_raw(&db.conn, raw);

        // Act
        let (loaded, warnings) = settings::load(&db.conn).unwrap();

        // Assert
        assert_eq!(loaded.theme, ThemeSetting::default(), "{raw}");
        assert_eq!(warnings.len(), 1, "{raw}");
        assert!(warnings[0].contains("theme"), "{raw}");
    }

    // 잘못된 패치는 거부되고 아무것도 저장되지 않는다.
    let mut db = migrated_db();
    for bad in [
        json!({"mode": "purple", "systemDark": "dim"}),
        json!({"mode": "dark"}),
        json!("dark"),
    ] {
        let err = settings::update(&mut db.conn, &patch(json!({"theme": bad}))).unwrap_err();
        assert!(matches!(err, SettingsError::Invalid(f) if f == "theme"));
    }
    let n: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key = 'theme'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_settings_theme_05_gray_is_a_mode_but_not_a_system_dark() {
    // Arrange
    let ok = json!({"mode": "gray", "systemDark": "dim"});
    let bad = json!({"mode": "system", "systemDark": "gray"});

    // Act / Assert: gray 모드는 파싱된다
    assert_eq!(
        ThemeSetting::parse(&ok),
        Some(ThemeSetting {
            mode: ThemeMode::Gray,
            system_dark: ThemeSystemDark::Dim
        })
    );
    assert_eq!(
        serde_json::to_value(ThemeMode::Gray).unwrap(),
        json!("gray")
    );
    assert_eq!(ThemeSetting::parse(&bad), None);

    // 저장값이 잘못되면 객체 전체가 기본값 + 경고 (T-03 과 동일 경로)
    let db = migrated_db();
    put_raw(&db.conn, &bad.to_string());
    let (loaded, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(loaded.theme, ThemeSetting::default());
    assert_eq!(warnings.len(), 1);

    // 잘못된 패치는 거부
    let mut db = migrated_db();
    let err = settings::update(&mut db.conn, &patch(json!({"theme": bad}))).unwrap_err();
    assert!(matches!(err, SettingsError::Invalid(f) if f == "theme"));
}

#[test]
fn t_settings_theme_04_saved_value_persists_after_reopening_the_database() {
    // Arrange
    let mut db = migrated_db();
    settings::update(
        &mut db.conn,
        &patch(json!({"theme": {"mode": "light", "systemDark": "dark"}})),
    )
    .unwrap();

    // Act
    drop(db.conn);
    let reopened = mam_db::open(&db.db_path).unwrap();
    let (after, warnings) = settings::load(&reopened).unwrap();

    // Assert
    assert_eq!(
        after.theme,
        ThemeSetting {
            mode: ThemeMode::Light,
            system_dark: ThemeSystemDark::Dark
        }
    );
    assert!(warnings.is_empty());
}
