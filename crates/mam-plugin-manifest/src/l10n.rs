//! 로케일 파일(`l10n/<code>.json`) — 평면 문자열 객체. 중복 키는 serde_json last-wins(문서화만).

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::error::LocaleError;
use crate::ids::{TitleKey, title_key_ok};
use crate::limits::{MAX_LOCALE_BYTES, MAX_LOCALE_ENTRIES, MAX_LOCALE_VALUE_LEN};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocaleMap(HashMap<String, String>);

impl LocaleMap {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub fn parse_locale_file(json: &str) -> Result<LocaleMap, LocaleError> {
    if json.len() > MAX_LOCALE_BYTES {
        return Err(LocaleError::TooLarge);
    }
    let s = json.strip_prefix('\u{feff}').unwrap_or(json);
    let value: Value = serde_json::from_str(s).map_err(|e| LocaleError::Json {
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    })?;
    let Value::Object(obj) = value else {
        return Err(LocaleError::Json {
            line: 1,
            column: 1,
            message: "root must be a JSON object".to_owned(),
        });
    };
    if obj.len() > MAX_LOCALE_ENTRIES {
        return Err(LocaleError::TooManyEntries);
    }
    let mut map = HashMap::with_capacity(obj.len());
    for (k, v) in obj {
        if !title_key_ok(&k) {
            return Err(LocaleError::InvalidKey { key: k });
        }
        let Value::String(text) = v else {
            return Err(LocaleError::NotFlatString { key: k });
        };
        if text.chars().count() > MAX_LOCALE_VALUE_LEN {
            return Err(LocaleError::ValueTooLong { key: k });
        }
        map.insert(k, text);
    }
    Ok(LocaleMap(map))
}

/// `[a-z]{2,3}(-[A-Za-z0-9]{2,8})*`, ≤35 바이트.
fn locale_code_ok(code: &str) -> bool {
    if code.len() > 35 {
        return false;
    }
    let mut parts = code.split('-');
    let lang_ok = parts
        .next()
        .is_some_and(|p| (2..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_lowercase()));
    lang_ok
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

#[derive(Debug, Clone, Default)]
pub struct LocaleBundle {
    maps: BTreeMap<String, LocaleMap>,
}

impl LocaleBundle {
    pub fn insert(&mut self, code: &str, map: LocaleMap) -> Result<(), LocaleError> {
        if !locale_code_ok(code) {
            return Err(LocaleError::InvalidLocaleCode {
                code: code.to_owned(),
            });
        }
        self.maps.insert(code.to_owned(), map);
        Ok(())
    }

    /// `locales` 순서대로: 정확한 코드 → 기본 하위 태그("ko-KR" → "ko"). 어디에도 없으면 `fallback`.
    pub fn resolve<'a>(&'a self, key: &TitleKey, locales: &[&str], fallback: &'a str) -> &'a str {
        for loc in locales {
            let base = loc.split('-').next().unwrap_or(loc);
            for code in [*loc, base] {
                if let Some(text) = self.maps.get(code).and_then(|m| m.get(key.as_str())) {
                    return text;
                }
            }
        }
        fallback
    }
}
