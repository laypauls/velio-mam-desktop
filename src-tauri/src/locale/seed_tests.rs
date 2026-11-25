// Categories: Happy (21 복사) / Business rule (22 덮어쓰기 금지, 23 이름·종류 필터) / Boundary (24 원본 폴더 없음)
// / External (25 대상 쓰기 실패) / Concurrency (26 멱등 재실행)
use std::path::Path;

use super::seed::seed_bundled_locales;

fn tmp() -> tempfile::TempDir {
    tempfile::TempDir::new().unwrap()
}

fn put(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

#[test]
fn t_locale_21_copies_missing_valid_files() {
    // Arrange
    let (src, dst) = (tmp(), tmp());
    put(src.path(), "en.json", "{\"a\":\"1\"}");
    put(src.path(), "pt-BR.json", "{\"a\":\"2\"}");
    // Act
    let r = seed_bundled_locales(src.path(), dst.path());
    // Assert
    assert_eq!(r.copied, ["en.json", "pt-BR.json"]);
    assert!(r.failed.is_empty());
    assert_eq!(
        std::fs::read_to_string(dst.path().join("en.json")).unwrap(),
        "{\"a\":\"1\"}"
    );
}

#[test]
fn t_locale_22_never_overwrites_existing_user_file() {
    // Arrange
    let (src, dst) = (tmp(), tmp());
    put(src.path(), "en.json", "bundled");
    put(src.path(), "ja.json", "bundled-ja");
    put(dst.path(), "en.json", "user-edited");
    // Act
    let r = seed_bundled_locales(src.path(), dst.path());
    // Assert
    assert_eq!(r.copied, ["ja.json"]);
    assert_eq!(
        std::fs::read_to_string(dst.path().join("en.json")).unwrap(),
        "user-edited"
    );
}

#[test]
fn t_locale_23_skips_invalid_names_non_json_and_subdirs() {
    // Arrange
    let (src, dst) = (tmp(), tmp());
    put(src.path(), "bad name.json", "x");
    put(src.path(), "..json", "x");
    put(src.path(), ".json", "x");
    put(src.path(), "notes.txt", "x");
    put(src.path(), "fr.JSON", "x");
    std::fs::create_dir(src.path().join("de.json")).unwrap();
    std::fs::create_dir(src.path().join("sub")).unwrap();
    put(&src.path().join("sub"), "es.json", "x");
    // Act
    let r = seed_bundled_locales(src.path(), dst.path());
    // Assert
    assert!(r.copied.is_empty() && r.failed.is_empty());
    assert_eq!(std::fs::read_dir(dst.path()).unwrap().count(), 0);
}

#[test]
fn t_locale_24_missing_source_dir_is_noop() {
    // Arrange
    let (base, dst) = (tmp(), tmp());
    // Act
    let r = seed_bundled_locales(&base.path().join("nope"), dst.path());
    // Assert
    assert!(r.copied.is_empty() && r.failed.is_empty());
}

#[test]
fn t_locale_25_unwritable_destination_is_reported_without_panic() {
    // Arrange: 대상 경로가 폴더가 아니라 파일 / 대상 폴더가 없음
    let (src, base) = (tmp(), tmp());
    put(src.path(), "en.json", "x");
    let file_as_dir = base.path().join("file");
    std::fs::write(&file_as_dir, "x").unwrap();
    // Act
    let r1 = seed_bundled_locales(src.path(), &file_as_dir);
    let r2 = seed_bundled_locales(src.path(), &base.path().join("missing"));
    // Assert
    for r in [r1, r2] {
        assert!(r.copied.is_empty());
        assert_eq!(r.failed, ["en.json"]);
    }
}

#[test]
fn t_locale_26_second_run_copies_nothing() {
    // Arrange
    let (src, dst) = (tmp(), tmp());
    put(src.path(), "en.json", "x");
    let first = seed_bundled_locales(src.path(), dst.path());
    // Act
    let second = seed_bundled_locales(src.path(), dst.path());
    // Assert
    assert_eq!(first.copied, ["en.json"]);
    assert!(second.copied.is_empty() && second.failed.is_empty());
}
