//! 검증된 식별자 뉴타입. 생성 시 검증하고 문자열로 직렬화한다.

use crate::error::ManifestError;
use crate::limits::{MAX_COMMAND_ID_LEN, MAX_ID_LEN, MAX_TITLE_KEY_LEN};

macro_rules! string_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
pub(crate) use string_newtype;

string_newtype!(PluginId);
string_newtype!(CommandId);
string_newtype!(ProviderId);
string_newtype!(TitleKey);

/// Windows 예약 장치 이름(확장자 제외 stem, 대소문자 무시, 끝 공백 무시) —
/// `CON PRN AUX NUL CONIN$ CONOUT$`, `COM1..9`·`LPT1..9` 와 위 첨자 `COM¹²³`·`LPT¹²³`.
/// 내보내기 대상 경로 검사(`mam-plugin-runtime`)도 이 함수를 쓴다.
pub fn is_reserved_name(stem: &str) -> bool {
    let u = stem.trim_end().to_uppercase();
    if matches!(u.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let chars: Vec<char> = u.chars().collect();
    chars.len() == 4 && (u.starts_with("COM") || u.starts_with("LPT")) && matches!(chars[3], '1'..='9' | '¹' | '²' | '³')
}

pub(crate) fn check_len(v: &str, path: &str, max: usize) -> Result<(), ManifestError> {
    if v.len() > max {
        return Err(ManifestError::TooLong {
            path: path.to_owned(),
            max,
        });
    }
    Ok(())
}

fn plugin_id_ok(s: &str) -> bool {
    let segs: Vec<&str> = s.split('.').collect();
    segs.len() >= 2
        && !is_reserved_name(segs[0])
        && segs.iter().all(|g| {
            g.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                && g.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !g.ends_with('-')
        })
}

fn dotted_id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|g| {
            g.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && g.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        })
}

/// `[A-Za-z0-9_][A-Za-z0-9_.-]*`, 1~128.
pub(crate) fn title_key_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_TITLE_KEY_LEN
        && (b[0].is_ascii_alphanumeric() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

fn invalid_id(path: &str, v: &str) -> ManifestError {
    ManifestError::InvalidId {
        path: path.to_owned(),
        value: v.to_owned(),
    }
}

impl PluginId {
    pub(crate) fn parse(v: &str, path: &str) -> Result<Self, ManifestError> {
        check_len(v, path, MAX_ID_LEN)?;
        if plugin_id_ok(v) {
            Ok(Self(v.to_owned()))
        } else {
            Err(invalid_id(path, v))
        }
    }
}

impl CommandId {
    pub(crate) fn parse(v: &str, path: &str) -> Result<Self, ManifestError> {
        check_len(v, path, MAX_COMMAND_ID_LEN)?;
        if dotted_id_ok(v) {
            Ok(Self(v.to_owned()))
        } else {
            Err(invalid_id(path, v))
        }
    }
}

impl ProviderId {
    pub(crate) fn parse(v: &str, path: &str) -> Result<Self, ManifestError> {
        check_len(v, path, MAX_COMMAND_ID_LEN)?;
        if dotted_id_ok(v) {
            Ok(Self(v.to_owned()))
        } else {
            Err(invalid_id(path, v))
        }
    }
}

impl TitleKey {
    pub(crate) fn parse(v: &str, path: &str) -> Result<Self, ManifestError> {
        check_len(v, path, MAX_TITLE_KEY_LEN)?;
        if title_key_ok(v) {
            Ok(Self(v.to_owned()))
        } else {
            Err(ManifestError::InvalidTitleKey {
                path: path.to_owned(),
                value: v.to_owned(),
            })
        }
    }
}

macro_rules! try_from_str {
    ($name:ident) => {
        impl TryFrom<&str> for $name {
            type Error = ManifestError;
            fn try_from(v: &str) -> Result<Self, Self::Error> {
                Self::parse(v, "")
            }
        }
    };
}
try_from_str!(PluginId);
try_from_str!(CommandId);
try_from_str!(ProviderId);
try_from_str!(TitleKey);
