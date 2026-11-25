use super::{WhenError, WhenErrorKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    True,
    False,
    Not,
    And,
    Or,
    Cmp(CmpOp),
    LParen,
    RParen,
}

#[derive(Debug, Clone)]
pub(super) struct Token {
    pub tok: Tok,
    pub offset: usize,
}

fn syntax(offset: usize) -> WhenError {
    WhenError {
        kind: WhenErrorKind::Syntax,
        offset,
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub(super) fn lex(src: &str) -> Result<Vec<Token>, WhenError> {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut out = Vec::new();
    while i < n {
        let start = i;
        let c = b[i];
        let tok = match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            b'(' => {
                i += 1;
                Tok::LParen
            }
            b')' => {
                i += 1;
                Tok::RParen
            }
            b'!' if b.get(i + 1) == Some(&b'=') => {
                i += 2;
                Tok::Cmp(CmpOp::Ne)
            }
            b'!' => {
                i += 1;
                Tok::Not
            }
            b'=' if b.get(i + 1) == Some(&b'=') => {
                i += 2;
                Tok::Cmp(CmpOp::Eq)
            }
            b'<' | b'>' => {
                let eq = b.get(i + 1) == Some(&b'=');
                i += if eq { 2 } else { 1 };
                Tok::Cmp(match (c, eq) {
                    (b'<', false) => CmpOp::Lt,
                    (b'<', true) => CmpOp::Le,
                    (_, false) => CmpOp::Gt,
                    (_, true) => CmpOp::Ge,
                })
            }
            b'&' if b.get(i + 1) == Some(&b'&') => {
                i += 2;
                Tok::And
            }
            b'|' if b.get(i + 1) == Some(&b'|') => {
                i += 2;
                Tok::Or
            }
            b'\'' | b'"' => {
                let (s, next) = lex_string(src, i)?;
                i = next;
                Tok::Str(s)
            }
            b'-' | b'0'..=b'9' => {
                let (v, next) = lex_number(src, i)?;
                i = next;
                Tok::Num(v)
            }
            _ if is_ident_start(c) => {
                i = lex_ident(b, i)?;
                match &src[start..i] {
                    "true" => Tok::True,
                    "false" => Tok::False,
                    s => Tok::Ident(s.to_owned()),
                }
            }
            _ => return Err(syntax(i)),
        };
        out.push(Token { tok, offset: start });
    }
    Ok(out)
}

fn lex_ident(b: &[u8], mut i: usize) -> Result<usize, WhenError> {
    loop {
        while i < b.len() && is_ident_char(b[i]) {
            i += 1;
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            if !b.get(i).is_some_and(|c| is_ident_start(*c)) {
                return Err(syntax(i));
            }
        } else {
            return Ok(i);
        }
    }
}

fn lex_number(src: &str, start: usize) -> Result<(f64, usize), WhenError> {
    let b = src.as_bytes();
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    if !b.get(i).is_some_and(u8::is_ascii_digit) {
        return Err(syntax(start));
    }
    if b[i] == b'0' {
        i += 1;
    } else {
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !b.get(i).is_some_and(u8::is_ascii_digit) {
            return Err(syntax(start));
        }
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if b.get(i).is_some_and(|c| is_ident_char(*c) || *c == b'.') {
        return Err(syntax(start));
    }
    let v: f64 = src[start..i].parse().map_err(|_| syntax(start))?;
    Ok((v, i))
}

fn lex_string(src: &str, start: usize) -> Result<(String, usize), WhenError> {
    let quote = src.as_bytes()[start];
    let mut out = String::new();
    let mut chars = src[start + 1..].char_indices();
    while let Some((off, c)) = chars.next() {
        let pos = start + 1 + off;
        if c == quote as char {
            return Ok((out, pos + 1));
        }
        if c.is_control() {
            return Err(syntax(pos));
        }
        if c == '\\' {
            match chars.next() {
                Some((_, e @ ('\\' | '\'' | '"'))) => out.push(e),
                _ => return Err(syntax(pos)),
            }
        } else {
            out.push(c);
        }
    }
    Err(syntax(start))
}
