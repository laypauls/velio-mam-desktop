//! 저장 위치 선택 대화상자 command (`pick_save_file`).
//!
//! `pick_directory` 와 같은 방식(자체 command 로만 대화상자를 열고 JS 대화상자 플러그인 권한은 열지 않는다).
//! 경로만 돌려주며 어떤 파일도 만들거나 쓰지 않는다 — 기록은 호출한 쪽(플러그인 provider 내보내기 등)의 몫이다.

use serde::Deserialize;
use tauri::Manager;

use crate::error::{CommandError, ErrorCode};
use crate::ingest_commands::picked_to_text;

const MAX_FILE_NAME_CHARS: usize = 255;
const MAX_FILTERS: usize = 8;
const MAX_FILTER_NAME_CHARS: usize = 64;
const MAX_EXTENSIONS: usize = 16;
const MAX_EXTENSION_CHARS: usize = 16;

/// 확장자 필터 (`{ name, extensions }`, 확장자는 점 없는 영숫자).
#[derive(Debug, Clone, Deserialize)]
pub struct SaveFileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

fn invalid(msg: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidInput, msg)
}

/// 기본 파일명 검증 — 경로 구분자·Windows 금지 문자·제어 문자·`.`/`..` 거부.
fn validate_file_name(name: &str) -> Result<(), CommandError> {
    let len = name.chars().count();
    if len == 0 || len > MAX_FILE_NAME_CHARS {
        return Err(invalid("file name length is out of range"));
    }
    if name.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(invalid("file name contains forbidden characters"));
    }
    if name.trim_matches('.').is_empty() || name.ends_with(' ') || name.ends_with('.') {
        return Err(invalid("file name is not valid"));
    }
    Ok(())
}

fn validate_filters(filters: &[SaveFileFilter]) -> Result<(), CommandError> {
    if filters.len() > MAX_FILTERS {
        return Err(invalid("too many filters"));
    }
    for f in filters {
        let name_len = f.name.chars().count();
        if name_len == 0 || name_len > MAX_FILTER_NAME_CHARS || f.name.chars().any(char::is_control)
        {
            return Err(invalid("filter name is not valid"));
        }
        if f.extensions.is_empty() || f.extensions.len() > MAX_EXTENSIONS {
            return Err(invalid("filter extension count is out of range"));
        }
        for ext in &f.extensions {
            if ext.is_empty()
                || ext.len() > MAX_EXTENSION_CHARS
                || !ext.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return Err(invalid("filter extension must be alphanumeric"));
            }
        }
    }
    Ok(())
}

/// 모든 인자를 대화상자를 열기 전에 검증한다 (프론트 입력은 신뢰하지 않는다).
fn validate_args(
    default_file_name: Option<&str>,
    filters: Option<&[SaveFileFilter]>,
) -> Result<(), CommandError> {
    if let Some(name) = default_file_name {
        validate_file_name(name)?;
    }
    if let Some(filters) = filters {
        validate_filters(filters)?;
    }
    Ok(())
}

/// 저장 위치 선택 대화상자. 선택한 절대 경로를 돌려주고 취소하면 `null`. 파일은 쓰지 않는다.
#[tauri::command]
pub async fn pick_save_file(
    app: tauri::AppHandle,
    default_file_name: Option<String>,
    filters: Option<Vec<SaveFileFilter>>,
) -> Result<Option<String>, CommandError> {
    use tauri_plugin_dialog::DialogExt;

    validate_args(default_file_name.as_deref(), filters.as_deref())?;

    let (tx, rx) = std::sync::mpsc::channel();
    let mut dialog = app.dialog().file();
    if let Some(name) = &default_file_name {
        dialog = dialog.set_file_name(name);
    }
    for f in filters.iter().flatten() {
        let exts: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&f.name, &exts);
    }
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.save_file(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| {
            tracing::error!(%e, "save dialog task failed");
            CommandError::internal()
        })?
        .map_err(|e| {
            tracing::error!(%e, "save dialog closed without a result");
            CommandError::internal()
        })?;
    picked_to_text(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(name: &str, exts: &[&str]) -> SaveFileFilter {
        SaveFileFilter {
            name: name.into(),
            extensions: exts.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn code(r: Result<(), CommandError>) -> ErrorCode {
        r.expect_err("must be rejected").code()
    }

    #[test]
    fn t_pick_save_01_accepts_absent_and_valid_args() {
        // Happy / Boundary
        assert!(validate_args(None, None).is_ok());
        assert!(
            validate_args(
                Some("export.csv"),
                Some(&[
                    filter("CSV", &["csv"]),
                    filter("모든 파일", &["txt", "tsv"])
                ])
            )
            .is_ok()
        );
        assert!(validate_args(Some(&"a".repeat(MAX_FILE_NAME_CHARS)), Some(&[])).is_ok());
        let max_filters: Vec<_> = (0..MAX_FILTERS)
            .map(|i| filter(&format!("f{i}"), &["a1"]))
            .collect();
        assert!(validate_args(None, Some(&max_filters)).is_ok());
    }

    #[test]
    fn t_pick_save_02_rejects_bad_file_names() {
        // Validation
        for bad in [
            "",
            "..",
            ".",
            "a/b.csv",
            "a\\b.csv",
            "..\\x.csv",
            "C:x.csv",
            "a*.csv",
            "a?.csv",
            "a<b>.csv",
            "a|b",
            "a\"b",
            "a\nb",
            "a\0b",
            "name.",
            "name ",
        ] {
            assert_eq!(
                code(validate_args(Some(bad), None)),
                ErrorCode::InvalidInput,
                "{bad:?}"
            );
        }
        let too_long = "a".repeat(MAX_FILE_NAME_CHARS + 1);
        assert_eq!(
            code(validate_args(Some(&too_long), None)),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn t_pick_save_03_rejects_bad_filters() {
        // Validation / Boundary
        let too_many: Vec<_> = (0..=MAX_FILTERS)
            .map(|i| filter(&format!("f{i}"), &["a"]))
            .collect();
        let long_name = "n".repeat(MAX_FILTER_NAME_CHARS + 1);
        let too_many_exts: Vec<String> = (0..=MAX_EXTENSIONS).map(|i| format!("e{i}")).collect();
        let long_ext = "x".repeat(MAX_EXTENSION_CHARS + 1);
        let cases: Vec<Vec<SaveFileFilter>> = vec![
            too_many,
            vec![filter("", &["csv"])],
            vec![filter(&long_name, &["csv"])],
            vec![filter("a\nb", &["csv"])],
            vec![filter("CSV", &[])],
            vec![SaveFileFilter {
                name: "CSV".into(),
                extensions: too_many_exts,
            }],
            vec![filter("CSV", &[""])],
            vec![filter("CSV", &[".csv"])],
            vec![filter("CSV", &["*"])],
            vec![filter("CSV", &["c sv"])],
            vec![filter("CSV", &["cs\\v"])],
            vec![filter("CSV", &["csv", "é"])],
            vec![filter("CSV", &[&long_ext])],
        ];
        for (i, c) in cases.iter().enumerate() {
            assert_eq!(
                code(validate_args(None, Some(c))),
                ErrorCode::InvalidInput,
                "case {i}"
            );
        }
    }

    #[test]
    fn t_pick_save_04_filters_json_shape_matches_ts_contract() {
        // Contract: TS `{ name, extensions }[]`
        let parsed: Vec<SaveFileFilter> =
            serde_json::from_str(r#"[{"name":"CSV","extensions":["csv"]}]"#).expect("parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "CSV");
        assert_eq!(parsed[0].extensions, vec!["csv".to_string()]);
        assert!(serde_json::from_str::<Vec<SaveFileFilter>>(r#"[{"name":"CSV"}]"#).is_err());
    }
}
