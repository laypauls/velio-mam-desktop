//! 재귀 하강 파서 + 정적 타입 검사. 비교는 비결합, `!` 는 비교보다 먼저 묶는다(JS 우선순위).

use super::lexer::{CmpOp, Tok, Token};
use super::{IDENTIFIERS, IdentType, WhenError, WhenErrorKind};
use crate::limits::MAX_WHEN_DEPTH;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Lit {
    Bool(bool),
    Num(f64),
    Str(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Expr {
    Lit(Lit),
    Ident(&'static str, IdentType),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cmp(CmpOp, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Ty {
    Bool,
    Num,
    Str,
    Enum(&'static [&'static str]),
}

struct Node {
    expr: Expr,
    ty: Ty,
}

fn err(kind: WhenErrorKind, offset: usize) -> WhenError {
    WhenError { kind, offset }
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
    end: usize,
}

pub(super) fn parse(toks: Vec<Token>, end: usize) -> Result<Expr, WhenError> {
    let mut p = Parser {
        toks,
        pos: 0,
        depth: 0,
        end,
    };
    let node = p.or()?;
    if p.pos < p.toks.len() {
        return Err(err(WhenErrorKind::Syntax, p.offset()));
    }
    if node.ty != Ty::Bool {
        return Err(err(WhenErrorKind::TypeMismatch, 0));
    }
    Ok(node.expr)
}

fn need_bool(n: &Node, off: usize) -> Result<(), WhenError> {
    if n.ty == Ty::Bool {
        Ok(())
    } else {
        Err(err(WhenErrorKind::TypeMismatch, off))
    }
}

fn lit_in(e: &Expr, values: &[&str]) -> bool {
    matches!(e, Expr::Lit(Lit::Str(s)) if values.contains(&s.as_str()))
}

fn check_cmp(op: CmpOp, l: &Node, r: &Node, off: usize) -> Result<(), WhenError> {
    let ok = match op {
        CmpOp::Eq | CmpOp::Ne => match (l.ty, r.ty) {
            (a, b) if a == b => true,
            (Ty::Enum(v), Ty::Str) => lit_in(&r.expr, v),
            (Ty::Str, Ty::Enum(v)) => lit_in(&l.expr, v),
            _ => false,
        },
        _ => l.ty == Ty::Num && r.ty == Ty::Num,
    };
    if ok {
        Ok(())
    } else {
        Err(err(WhenErrorKind::TypeMismatch, off))
    }
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn offset(&self) -> usize {
        self.toks.get(self.pos).map_or(self.end, |t| t.offset)
    }

    fn enter(&mut self, off: usize) -> Result<(), WhenError> {
        self.depth += 1;
        if self.depth > MAX_WHEN_DEPTH {
            Err(err(WhenErrorKind::TooDeep, off))
        } else {
            Ok(())
        }
    }

    fn or(&mut self) -> Result<Node, WhenError> {
        let mut left = self.and()?;
        while matches!(self.peek(), Some(Tok::Or)) {
            let off = self.offset();
            self.pos += 1;
            need_bool(&left, off)?;
            let right = self.and()?;
            need_bool(&right, off)?;
            left = Node {
                expr: Expr::Or(Box::new(left.expr), Box::new(right.expr)),
                ty: Ty::Bool,
            };
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, WhenError> {
        let mut left = self.cmp()?;
        while matches!(self.peek(), Some(Tok::And)) {
            let off = self.offset();
            self.pos += 1;
            need_bool(&left, off)?;
            let right = self.cmp()?;
            need_bool(&right, off)?;
            left = Node {
                expr: Expr::And(Box::new(left.expr), Box::new(right.expr)),
                ty: Ty::Bool,
            };
        }
        Ok(left)
    }

    fn cmp(&mut self) -> Result<Node, WhenError> {
        let left = self.unary()?;
        let op = match self.peek() {
            Some(Tok::Cmp(op)) => *op,
            _ => return Ok(left),
        };
        let off = self.offset();
        self.pos += 1;
        let right = self.unary()?;
        check_cmp(op, &left, &right, off)?;
        if matches!(self.peek(), Some(Tok::Cmp(_))) {
            return Err(err(WhenErrorKind::Syntax, self.offset()));
        }
        Ok(Node {
            expr: Expr::Cmp(op, Box::new(left.expr), Box::new(right.expr)),
            ty: Ty::Bool,
        })
    }

    fn unary(&mut self) -> Result<Node, WhenError> {
        if matches!(self.peek(), Some(Tok::Not)) {
            let off = self.offset();
            self.pos += 1;
            self.enter(off)?;
            let inner = self.unary()?;
            self.depth -= 1;
            need_bool(&inner, off)?;
            return Ok(Node {
                expr: Expr::Not(Box::new(inner.expr)),
                ty: Ty::Bool,
            });
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, WhenError> {
        let off = self.offset();
        let Some(token) = self.toks.get(self.pos).cloned() else {
            return Err(err(WhenErrorKind::Syntax, off));
        };
        self.pos += 1;
        let lit = |l: Lit, ty: Ty| Node {
            expr: Expr::Lit(l),
            ty,
        };
        match token.tok {
            Tok::True => Ok(lit(Lit::Bool(true), Ty::Bool)),
            Tok::False => Ok(lit(Lit::Bool(false), Ty::Bool)),
            Tok::Num(n) => Ok(lit(Lit::Num(n), Ty::Num)),
            Tok::Str(s) => Ok(lit(Lit::Str(s), Ty::Str)),
            Tok::Ident(name) => {
                let Some((n, t)) = IDENTIFIERS.iter().find(|(n, _)| *n == name) else {
                    return Err(err(WhenErrorKind::UnknownIdentifier, off));
                };
                let ty = match t {
                    IdentType::Number => Ty::Num,
                    IdentType::Bool => Ty::Bool,
                    IdentType::Enum(v) => Ty::Enum(v),
                };
                Ok(Node {
                    expr: Expr::Ident(n, *t),
                    ty,
                })
            }
            Tok::LParen => {
                self.enter(off)?;
                let inner = self.or()?;
                if !matches!(self.peek(), Some(Tok::RParen)) {
                    return Err(err(WhenErrorKind::Syntax, self.offset()));
                }
                self.pos += 1;
                self.depth -= 1;
                Ok(inner)
            }
            _ => Err(err(WhenErrorKind::Syntax, off)),
        }
    }
}
