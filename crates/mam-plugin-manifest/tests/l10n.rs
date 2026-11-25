//! 분류: Happy / Validation / Boundary. External·Concurrency 는 순수 crate 라 해당 없음.

use mam_plugin_manifest::{LocaleBundle, LocaleError, LocaleMap, TitleKey, parse_locale_file};

fn key(s: &str) -> TitleKey {
    TitleKey::try_from(s).expect("key")
}

fn map(json: &str) -> LocaleMap {
    parse_locale_file(json).expect("locale")
}

#[test]
fn t_pmanifest_07_resolve_chain() {
    // Arrange: 호출자 순서 = 현재 로케일 → "en" (사용자 결정)
    let mut b = LocaleBundle::default();
    b.insert("ko", map(r#"{"a":"가"}"#)).expect("ko");
    b.insert("en", map(r#"{"a":"A","only.en":"E"}"#))
        .expect("en");
    b.insert("ja-JP", map(r#"{"a":"あ"}"#)).expect("ja");
    let chain = ["ko-KR", "en"];

    // Act / Assert
    assert_eq!(b.resolve(&key("a"), &["ja-JP", "en"], "id"), "あ"); // 정확한 코드
    assert_eq!(b.resolve(&key("a"), &chain, "id"), "가"); // ko-KR -> ko
    assert_eq!(b.resolve(&key("only.en"), &chain, "id"), "E"); // 현재 로케일에 키 없음 -> en
    assert_eq!(b.resolve(&key("missing"), &chain, "plugin.id"), "plugin.id"); // 전부 없음 -> fallback
    assert_eq!(b.resolve(&key("a"), &[], "plugin.id"), "plugin.id");
}

#[test]
fn t_pmanifest_25_locale_validation() {
    assert!(matches!(
        parse_locale_file(r#"{"a":{"b":"c"}}"#),
        Err(LocaleError::NotFlatString { .. })
    ));
    assert!(matches!(
        parse_locale_file(r#"{"a":1}"#),
        Err(LocaleError::NotFlatString { .. })
    ));
    assert!(matches!(
        parse_locale_file(r#"{"bad key":"x"}"#),
        Err(LocaleError::InvalidKey { .. })
    ));
    assert!(matches!(
        parse_locale_file("[]"),
        Err(LocaleError::Json { .. })
    ));
    assert!(matches!(
        parse_locale_file("{"),
        Err(LocaleError::Json { .. })
    ));
    assert!(parse_locale_file("\u{feff}{\"a\":\"b\"}").is_ok()); // BOM
    let mut b = LocaleBundle::default();
    for code in ["../ko", "k", "KO", "ko_KR", "ko-", "ko-K"] {
        assert!(
            matches!(
                b.insert(code, LocaleMap::default()),
                Err(LocaleError::InvalidLocaleCode { .. })
            ),
            "{code}"
        );
    }
    assert!(b.insert("zh-Hant-TW", LocaleMap::default()).is_ok());
}

#[test]
fn t_pmanifest_31_locale_limits() {
    // 256 KiB 정확값 통과 / +1 TooLarge (공백 패딩)
    let base = r#"{"a":"b"}"#;
    let ok = format!("{base}{}", " ".repeat(262_144 - base.len()));
    assert_eq!(ok.len(), 262_144);
    assert!(parse_locale_file(&ok).is_ok());
    assert_eq!(
        parse_locale_file(&format!("{ok} ")),
        Err(LocaleError::TooLarge)
    );

    // 2000 / 2001 항목
    let entries = |n: usize| {
        let body: Vec<String> = (0..n).map(|i| format!("\"k{i}\":\"v\"")).collect();
        format!("{{{}}}", body.join(","))
    };
    assert_eq!(map(&entries(2000)).len(), 2000);
    assert_eq!(
        parse_locale_file(&entries(2001)),
        Err(LocaleError::TooManyEntries)
    );

    // 값 512 / 513 자
    let val = |n: usize| format!("{{\"a\":\"{}\"}}", "가".repeat(n));
    assert!(parse_locale_file(&val(512)).is_ok());
    assert!(matches!(
        parse_locale_file(&val(513)),
        Err(LocaleError::ValueTooLong { .. })
    ));
}
