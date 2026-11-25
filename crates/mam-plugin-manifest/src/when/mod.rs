//! `when` 식 — 파싱 시 정적 검사(잘못된 식은 설치 실패), 실행은 fail-closed.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::limits::MAX_WHEN_LEN;

mod eval;
mod lexer;
mod parser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhenErrorKind {
    Syntax,
    UnknownIdentifier,
    TypeMismatch,
    TooLong,
    TooDeep,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid when expression ({kind:?}) at byte {offset}")]
pub struct WhenError {
    pub kind: WhenErrorKind,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WhenEvalError {
    #[error("context value for `{identifier}` has the wrong type")]
    ContextType { identifier: String },
}

/// 식별자의 정적 타입.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentType {
    Number,
    Bool,
    Enum(&'static [&'static str]),
}

pub const ASSET_KIND_VALUES: &[&str] = &["image", "video", "audio"];

/// 닫힌 식별자 표(`vectors/when.json` 의 `identifiers` 와 같아야 한다).
pub const IDENTIFIERS: &[(&str, IdentType)] = &[
    ("selection.count", IdentType::Number),
    ("asset.kind", IdentType::Enum(ASSET_KIND_VALUES)),
    ("asset.offline", IdentType::Bool),
];

#[derive(Debug, Clone, PartialEq)]
pub enum WhenValue {
    Bool(bool),
    Num(f64),
    Str(String),
    Null,
}

/// 키 없음 = `Null`.
#[derive(Debug, Clone, Default)]
pub struct WhenContext(HashMap<String, WhenValue>);

impl WhenContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: &str, value: WhenValue) {
        self.0.insert(key.to_owned(), value);
    }

    pub fn with(mut self, key: &str, value: WhenValue) -> Self {
        self.insert(key, value);
        self
    }

    pub(crate) fn get(&self, key: &str) -> Option<&WhenValue> {
        self.0.get(key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenExpr {
    src: String,
    ast: parser::Expr,
}

impl WhenExpr {
    pub fn parse(src: &str) -> Result<Self, WhenError> {
        if src.len() > MAX_WHEN_LEN {
            return Err(WhenError {
                kind: WhenErrorKind::TooLong,
                offset: 0,
            });
        }
        let tokens = lexer::lex(src)?;
        let ast = parser::parse(tokens, src.len())?;
        Ok(Self {
            src: src.to_owned(),
            ast,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.src
    }

    pub fn try_eval(&self, ctx: &WhenContext) -> Result<bool, WhenEvalError> {
        Ok(eval::truthy(&eval::eval(&self.ast, ctx)?))
    }

    /// 실행 오류는 false(메뉴 숨김) — fail-closed.
    pub fn eval(&self, ctx: &WhenContext) -> bool {
        self.try_eval(ctx).unwrap_or(false)
    }
}

impl Serialize for WhenExpr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.src)
    }
}

impl<'de> Deserialize<'de> for WhenExpr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}
