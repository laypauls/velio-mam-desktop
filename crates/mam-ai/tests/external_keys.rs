//! 키 보관 테스트 — 분류: Happy / Business rule / External / Validation. DPAPI 는 Windows 에서만 실제 왕복한다.

use mam_ai::external::Provider;
use mam_ai::keys::{KeyError, KeyStorage, KeyVault};

const KEY: &str = "sk-test-SECRET-0123456789abcdef";

#[cfg(windows)]
#[test]
fn t_ext_01_session_and_persist_storage_and_dpapi_round_trip() {
    // Arrange
    let v = KeyVault::new();
    // Act
    assert_eq!(
        v.set(Provider::OpenAi, KEY, false).unwrap(),
        KeyStorage::Session
    );
    assert_eq!(
        v.set(Provider::Claude, KEY, true).unwrap(),
        KeyStorage::Persisted
    );
    // Assert: 상태
    assert_eq!(v.storage(Provider::OpenAi), KeyStorage::Session);
    assert_eq!(v.storage(Provider::Gemini), KeyStorage::None);
    assert!(v.export_persisted(Provider::OpenAi).is_none());
    // Assert: 새 vault 로 blob 왕복
    let blob = v.export_persisted(Provider::Claude).unwrap();
    let v2 = KeyVault::new();
    let r = v2.load_persisted([(Provider::Claude, blob.as_str())]);
    assert!(r[0].1.is_ok());
    assert_eq!(v2.storage(Provider::Claude), KeyStorage::Persisted);
    assert_eq!(v2.with_key(Provider::Claude, |k| k == KEY), Some(true));
    // clear
    v2.clear(Provider::Claude);
    assert_eq!(v2.storage(Provider::Claude), KeyStorage::None);
    assert!(v2.with_key(Provider::Claude, |_| ()).is_none());
}

#[test]
fn t_ext_01_set_validation_rejects_empty_and_malformed_keys() {
    let v = KeyVault::new();
    assert!(matches!(
        v.set(Provider::OpenAi, "   ", false),
        Err(KeyError::Invalid(_))
    ));
    assert!(matches!(
        v.set(Provider::OpenAi, "a b", false),
        Err(KeyError::Invalid(_))
    ));
    assert!(matches!(
        v.set(Provider::OpenAi, "bad\nkey", false),
        Err(KeyError::Invalid(_))
    ));
    assert_eq!(v.storage(Provider::OpenAi), KeyStorage::None);
}

#[cfg(not(windows))]
#[test]
fn t_ext_01_persist_unsupported_off_windows_keeps_nothing() {
    let v = KeyVault::new();
    assert_eq!(
        v.set(Provider::OpenAi, KEY, true),
        Err(KeyError::Unsupported)
    );
    assert_eq!(v.storage(Provider::OpenAi), KeyStorage::None);
}

#[cfg(windows)]
#[test]
fn t_ext_02_key_never_in_debug_errors_or_blob() {
    // Arrange
    let v = KeyVault::new();
    v.set(Provider::Gemini, KEY, true).unwrap();
    let blob = v.export_persisted(Provider::Gemini).unwrap();
    // Assert: Debug·blob·오류 문자열에 평문 없음
    assert!(!format!("{v:?}").contains(KEY));
    assert!(!format!("{v:#?}").contains(KEY));
    assert!(!blob.contains(KEY));
    let errs = [
        v.set(Provider::Gemini, "", false).unwrap_err(),
        v.load_persisted([(Provider::Gemini, "dpapi1:@@@")])[0]
            .1
            .clone()
            .unwrap_err(),
        KeyError::Crypto("x"),
    ];
    for e in errs {
        assert!(!e.to_string().contains(KEY) && !format!("{e:?}").contains(KEY));
    }
}

#[cfg(windows)]
#[test]
fn t_ext_03_corrupted_blob_fails_only_that_provider() {
    // Arrange: 정상 blob 하나 + 손상 blob 여러 형태
    let src = KeyVault::new();
    src.set(Provider::Claude, KEY, true).unwrap();
    let good = src.export_persisted(Provider::Claude).unwrap();
    let garbage_cipher = format!("dpapi1:{}", "QUJDREVGR0g=");
    let v = KeyVault::new();
    // Act
    let results = v.load_persisted([
        (Provider::OpenAi, "not-a-blob"),
        (Provider::Gemini, garbage_cipher.as_str()),
        (Provider::Claude, good.as_str()),
    ]);
    // Assert
    assert_eq!(results[0].1, Err(KeyError::BadBlob));
    assert!(results[1].1.is_err());
    assert!(results[2].1.is_ok());
    assert_eq!(v.storage(Provider::OpenAi), KeyStorage::None);
    assert_eq!(v.storage(Provider::Gemini), KeyStorage::None);
    assert_eq!(v.storage(Provider::Claude), KeyStorage::Persisted);
}
