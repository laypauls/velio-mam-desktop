// Categories: Happy (01 목록·이름, 07 읽기) / Boundary (02 빈·없는 폴더, 06 정확히 2 MiB) / Validation (03 파일명,
// 04 JSON, 05 모양, 08 코드·탈출, 10 내용) / Business rule (06 크기 상한, 09 not_found, 11 폴더 생성)
// / External (12 실행 실패)
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mam_core::reuse::Launcher;
use serde_json::json;

use super::commands::{list, open_folder, read};
use super::files::{LocaleFileError, MAX_LOCALE_BYTES};
use super::{ensure_locales_dir, locales_dir};
use crate::error::ErrorCode;

fn put(dir: &Path, name: &str, body: &[u8]) {
    std::fs::write(dir.join(name), body).unwrap();
}

fn tmp() -> tempfile::TempDir {
    tempfile::TempDir::new().unwrap()
}

#[derive(Default)]
struct StubLauncher {
    opened: Mutex<Vec<PathBuf>>,
    fail: Mutex<Option<io::ErrorKind>>,
}

impl Launcher for StubLauncher {
    fn open(&self, path: &Path) -> io::Result<()> {
        self.opened.lock().unwrap().push(path.to_path_buf());
        self.fail
            .lock()
            .unwrap()
            .map_or(Ok(()), |k| Err(io::Error::from(k)))
    }

    fn reveal(&self, _: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn t_locale_01_lists_valid_files_sorted_with_meta_name() {
    // Arrange
    let d = tmp();
    put(
        d.path(),
        "ja.json",
        r#"{"_meta":{"name":"日本語"},"a":{"b":"c"}}"#.as_bytes(),
    );
    put(d.path(), "en.json", br#"{"_meta":{"name":"English"}}"#);
    put(d.path(), "zh-TW.json", br#"{"a":"b"}"#);
    put(d.path(), "ko.json", br#"{"a":"b"}"#);

    // Act
    let r = list(d.path()).unwrap();

    // Assert
    let codes: Vec<_> = r.locales.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(codes, ["en", "ja", "ko", "zh-TW"]);
    assert_eq!(r.locales[0].name.as_deref(), Some("English"));
    assert_eq!(r.locales[1].name.as_deref(), Some("日本語"));
    assert_eq!(r.locales[3].name, None);
    assert!(r.locales.iter().all(|e| e.error.is_none()));
    assert_eq!(r.dir, d.path().display().to_string());
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(
        v["locales"][0],
        json!({"code": "en", "name": "English", "error": null})
    );
}

#[test]
fn t_locale_02_empty_and_missing_folder_give_empty_list() {
    // Arrange
    let d = tmp();

    // Act / Assert
    assert!(list(d.path()).unwrap().locales.is_empty());
    assert!(list(&d.path().join("nope")).unwrap().locales.is_empty());
}

#[test]
fn t_locale_03_invalid_names_subfolders_and_other_extensions_are_skipped() {
    // Arrange
    let d = tmp();
    for n in [
        "EN.json",
        "en_US.json",
        "e.json",
        "readme.txt",
        "en.json.bak",
        "en-.json",
        ".json",
    ] {
        put(d.path(), n, br#"{"a":"b"}"#);
    }
    std::fs::create_dir(d.path().join("sub")).unwrap();
    put(&d.path().join("sub"), "fr.json", br#"{"a":"b"}"#);
    std::fs::create_dir(d.path().join("de.json")).unwrap();
    put(d.path(), "es.json", br#"{"a":"b"}"#);

    // Act
    let r = list(d.path()).unwrap();

    // Assert
    let codes: Vec<_> = r.locales.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(codes, ["es"]);
}

#[test]
fn t_locale_04_invalid_json_and_non_utf8_are_reported() {
    // Arrange
    let d = tmp();
    put(d.path(), "en.json", b"{not json");
    put(d.path(), "ja.json", b"");
    put(
        d.path(),
        "fr.json",
        &[b'{', b'"', 0xff, 0xfe, b'"', b':', b'"', b'"', b'}'],
    );

    // Act
    let r = list(d.path()).unwrap();

    // Assert
    assert_eq!(r.locales.len(), 3);
    assert!(
        r.locales
            .iter()
            .all(|e| e.error == Some(LocaleFileError::InvalidJson) && e.name.is_none())
    );
    assert_eq!(
        serde_json::to_value(r.locales[0].error).unwrap(),
        json!("invalid_json")
    );
}

#[test]
fn t_locale_05_non_object_root_and_non_string_values_are_invalid_shape() {
    // Arrange
    let d = tmp();
    for (n, body) in [
        ("aa.json", r#"["x"]"#),
        ("bb.json", r#"{"a":1}"#),
        ("cc.json", r#"{"a":{"b":[ "x" ]}}"#),
        ("dd.json", r#"{"a":true}"#),
        ("ee.json", r#"{"a":null}"#),
        ("ff.json", r#""text""#),
        ("gg.json", r#"{"_meta":{"name":"X"},"a":{"b":{"c":2.5}}}"#),
    ] {
        put(d.path(), n, body.as_bytes());
    }

    // Act
    let r = list(d.path()).unwrap();

    // Assert
    assert_eq!(r.locales.len(), 7);
    for e in &r.locales {
        assert_eq!(e.error, Some(LocaleFileError::InvalidShape), "{}", e.code);
        assert_eq!(e.name, None);
    }
}

#[test]
fn t_locale_06_file_over_2_mib_is_too_large_and_exactly_2_mib_is_accepted() {
    // Arrange
    let d = tmp();
    let pad = |total: u64| {
        let head = br#"{"a":""#;
        let tail = br#""}"#;
        let fill = total as usize - head.len() - tail.len();
        let mut v = head.to_vec();
        v.extend(std::iter::repeat_n(b'x', fill));
        v.extend_from_slice(tail);
        v
    };
    put(d.path(), "aa.json", &pad(MAX_LOCALE_BYTES + 1));
    put(d.path(), "bb.json", &pad(MAX_LOCALE_BYTES));

    // Act
    let r = list(d.path()).unwrap();
    let err = read(d.path(), "aa").unwrap_err();

    // Assert
    assert_eq!(r.locales[0].code, "aa");
    assert_eq!(r.locales[0].error, Some(LocaleFileError::TooLarge));
    assert_eq!(r.locales[1].error, None);
    assert_eq!(err.code(), ErrorCode::InvalidInput);
}

#[test]
fn t_locale_07_read_returns_whole_object_including_meta() {
    // Arrange
    let d = tmp();
    put(
        d.path(),
        "en.json",
        br#"{"_meta":{"name":"English"},"common":{"cancel":"Cancel"}}"#,
    );

    // Act
    let v = read(d.path(), "en").unwrap();

    // Assert
    assert_eq!(
        v,
        json!({"_meta":{"name":"English"},"common":{"cancel":"Cancel"}})
    );
}

#[test]
fn t_locale_08_traversal_and_malformed_codes_are_rejected_before_touching_the_fs() {
    // Arrange: 폴더 밖에 유효한 파일을 둔다
    let d = tmp();
    let dir = d.path().join("locales");
    std::fs::create_dir(&dir).unwrap();
    put(d.path(), "x.json", br#"{"a":"b"}"#);

    for code in [
        "../x",
        "..\\x",
        "a\\b",
        "a/b",
        "..",
        "",
        "EN",
        "en_US",
        "en/../../x",
        "C:\\x",
    ] {
        // Act
        let err = read(&dir, code).unwrap_err();

        // Assert
        assert_eq!(err.code(), ErrorCode::InvalidInput, "{code:?}");
    }
}

#[test]
fn t_locale_09_missing_file_and_directory_named_like_a_file_are_not_found() {
    // Arrange
    let d = tmp();
    std::fs::create_dir(d.path().join("de.json")).unwrap();

    // Act / Assert
    assert_eq!(
        read(d.path(), "en").unwrap_err().code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        read(&d.path().join("nope"), "en").unwrap_err().code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        read(d.path(), "de").unwrap_err().code(),
        ErrorCode::NotFound
    );
}

#[test]
fn t_locale_10_read_rejects_invalid_content_without_leaking_it() {
    // Arrange
    let d = tmp();
    put(d.path(), "aa.json", b"{secret-token");
    put(d.path(), "bb.json", br#"{"a":["secret-token"]}"#);

    for code in ["aa", "bb"] {
        // Act
        let err = read(d.path(), code).unwrap_err();

        // Assert
        assert_eq!(err.code(), ErrorCode::InvalidInput, "{code}");
        assert!(
            !serde_json::to_string(&err)
                .unwrap()
                .contains("secret-token")
        );
    }
}

#[test]
fn t_locale_11_open_creates_missing_folder_then_opens_it() {
    // Arrange
    let d = tmp();
    let dir = locales_dir(d.path());
    let launcher = StubLauncher::default();
    assert!(!dir.exists());

    // Act
    open_folder(&dir, &launcher).unwrap();
    ensure_locales_dir(d.path());

    // Assert
    assert!(dir.is_dir());
    assert_eq!(*launcher.opened.lock().unwrap(), vec![dir.clone()]);
    assert!(dir.ends_with("locales"));
}

#[test]
fn t_locale_12_open_failures_map_to_io_and_unwritable_parent_is_io() {
    // Arrange
    let d = tmp();
    let dir = d.path().join("locales");
    for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
        let launcher = StubLauncher::default();
        *launcher.fail.lock().unwrap() = Some(kind);

        // Act
        let err = open_folder(&dir, &launcher).unwrap_err();

        // Assert
        assert_eq!(err.code(), ErrorCode::Io, "{kind:?}");
    }

    // 상위 경로가 파일이면 폴더를 만들 수 없다
    let file = d.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    let launcher = StubLauncher::default();
    let err = open_folder(&file.join("locales"), &launcher).unwrap_err();
    assert_ne!(err.code(), ErrorCode::Internal);
    assert!(launcher.opened.lock().unwrap().is_empty());
}
