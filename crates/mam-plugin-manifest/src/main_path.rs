//! `main` 경로 — 순수 문자열 검사. canonicalize·링크 탈출·존재 확인은 설치기(P6) 몫.

use std::path::PathBuf;

use crate::error::{MainPathError, ManifestError};
use crate::ids::{is_reserved_name, string_newtype};
use crate::limits::{MAX_MAIN_COMPONENTS, MAX_MAIN_LEN};

string_newtype!(MainPath);

impl MainPath {
    /// 검사 후 앞 `./` 1회를 제거한 정규형을 저장한다.
    pub fn parse(value: &str) -> Result<Self, MainPathError> {
        if value.is_empty() {
            return Err(MainPathError::Empty);
        }
        if value.len() > MAX_MAIN_LEN {
            return Err(MainPathError::TooLong);
        }
        if value.contains('\\') {
            return Err(MainPathError::Backslash);
        }
        if value.contains(':') {
            return Err(MainPathError::Colon);
        }
        if value.starts_with('/') {
            return Err(MainPathError::Absolute);
        }
        let rest = value.strip_prefix("./").unwrap_or(value);
        if rest.is_empty() {
            return Err(MainPathError::Empty);
        }
        if rest
            .chars()
            .any(|c| c.is_control() || "<>\"|?*".contains(c))
        {
            return Err(MainPathError::IllegalChar);
        }
        let comps: Vec<&str> = rest.split('/').collect();
        if comps.len() > MAX_MAIN_COMPONENTS {
            return Err(MainPathError::TooManyComponents);
        }
        for c in &comps {
            match *c {
                "" => return Err(MainPathError::EmptyComponent),
                "." => return Err(MainPathError::DotComponent),
                ".." => return Err(MainPathError::ParentEscape),
                _ => {}
            }
            if c.ends_with('.') || c.ends_with(' ') {
                return Err(MainPathError::TrailingDotOrSpace);
            }
            let stem = c.split('.').next().unwrap_or(c);
            if is_reserved_name(stem) {
                return Err(MainPathError::ReservedName);
            }
        }
        let file = comps.last().copied().unwrap_or("");
        if !file.ends_with(".js") || file.len() <= 3 {
            return Err(MainPathError::NotJs);
        }
        Ok(Self(rest.to_owned()))
    }

    /// `/` 구분 정규형을 상대 `PathBuf` 로 바꾼다(앞 `./` 없음).
    pub fn to_relative_path(&self) -> PathBuf {
        self.0.split('/').collect()
    }
}

impl TryFrom<&str> for MainPath {
    type Error = MainPathError;

    fn try_from(v: &str) -> Result<Self, Self::Error> {
        Self::parse(v)
    }
}

impl MainPath {
    pub(crate) fn parse_manifest(value: &str) -> Result<Self, ManifestError> {
        Self::parse(value).map_err(|reason| ManifestError::InvalidMain {
            value: value.to_owned(),
            reason,
        })
    }
}
