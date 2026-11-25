//! 언어 파일 나열·읽기·검증 (폴더 경로는 인자 — 테스트는 임시 폴더를 쓴다).

use std::io::Read;
use std::path::Path;

use mam_db::locale_code::is_valid_locale_code;
use serde::Serialize;
use serde_json::Value;

/// 언어 파일 1개 최대 크기.
pub const MAX_LOCALE_BYTES: u64 = 2 * 1024 * 1024;

/// 목록에 표시되는 파일 단위 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocaleFileError {
    InvalidJson,
    InvalidShape,
    TooLarge,
    ReadFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleEntry {
    pub code: String,
    pub name: Option<String>,
    pub error: Option<LocaleFileError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleListing {
    pub dir: String,
    pub locales: Vec<LocaleEntry>,
}

/// 읽기 실패 — `Io` 는 원인 종류를 보존한다(`not_found` 구분용).
#[derive(Debug)]
pub enum ReadError {
    Invalid(LocaleFileError),
    Io(std::io::Error),
}

impl ReadError {
    fn list_error(&self) -> LocaleFileError {
        match self {
            Self::Invalid(e) => *e,
            Self::Io(_) => LocaleFileError::ReadFailed,
        }
    }
}

/// 값이 재귀적으로 문자열 또는 객체뿐이면 `true`.
fn shape_ok(v: &Value) -> bool {
    match v {
        Value::String(_) => true,
        Value::Object(m) => m.values().all(shape_ok),
        _ => false,
    }
}

/// `<dir>/<code>.json` 을 검증해 읽는다. `code` 형식은 호출자가 검증해야 한다.
pub fn read_file(dir: &Path, code: &str) -> Result<Value, ReadError> {
    debug_assert!(is_valid_locale_code(code));
    let path = dir.join(format!("{code}.json"));
    let meta = std::fs::metadata(&path).map_err(ReadError::Io)?;
    if !meta.is_file() {
        return Err(ReadError::Io(std::io::ErrorKind::NotFound.into()));
    }
    if meta.len() > MAX_LOCALE_BYTES {
        return Err(ReadError::Invalid(LocaleFileError::TooLarge));
    }
    // 메타데이터 확인 후 파일이 커졌을 수 있으므로 읽는 양도 제한한다
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|f| f.take(MAX_LOCALE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > MAX_LOCALE_BYTES {
        return Err(ReadError::Invalid(LocaleFileError::TooLarge));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ReadError::Invalid(LocaleFileError::InvalidJson))?;
    if value.is_object() && shape_ok(&value) {
        Ok(value)
    } else {
        Err(ReadError::Invalid(LocaleFileError::InvalidShape))
    }
}

/// 폴더 최상위의 `<code>.json` 을 코드순으로 나열한다. 폴더가 없으면 빈 목록.
pub fn list_dir(dir: &Path) -> std::io::Result<Vec<LocaleEntry>> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for item in rd {
        let Ok(item) = item else { continue };
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(code) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_valid_locale_code(code) || !path.is_file() {
            continue;
        }
        let (name, error) = match read_file(dir, code) {
            Ok(v) => (
                v.pointer("/_meta/name")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                None,
            ),
            Err(e) => {
                if let ReadError::Io(ref io) = e {
                    tracing::warn!(%io, path = %path.display(), "cannot read a locale file");
                } else {
                    tracing::warn!(error = ?e.list_error(), path = %path.display(), "invalid locale file");
                }
                (None, Some(e.list_error()))
            }
        };
        out.push(LocaleEntry {
            code: code.to_owned(),
            name,
            error,
        });
    }
    out.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(out)
}
