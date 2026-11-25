//! 실행 의미: `&&`·`||` 단락, 키 없음 = Null(bool 값 false), 타입이 틀린 컨텍스트는 오류.

use super::lexer::CmpOp;
use super::parser::{Expr, Lit};
use super::{IdentType, WhenContext, WhenEvalError, WhenValue};

pub(super) fn truthy(v: &WhenValue) -> bool {
    matches!(v, WhenValue::Bool(true))
}

fn type_ok(t: &IdentType, v: &WhenValue) -> bool {
    matches!(
        (t, v),
        (IdentType::Number, WhenValue::Num(_))
            | (IdentType::Bool, WhenValue::Bool(_))
            | (IdentType::Enum(_), WhenValue::Str(_))
    )
}

fn equal(l: &WhenValue, r: &WhenValue) -> bool {
    match (l, r) {
        (WhenValue::Null, WhenValue::Null) => true,
        (WhenValue::Bool(a), WhenValue::Bool(b)) => a == b,
        (WhenValue::Num(a), WhenValue::Num(b)) => a == b,
        (WhenValue::Str(a), WhenValue::Str(b)) => a == b,
        _ => false,
    }
}

fn compare(op: CmpOp, l: &WhenValue, r: &WhenValue) -> bool {
    match op {
        CmpOp::Eq => equal(l, r),
        CmpOp::Ne => !equal(l, r),
        _ => match (l, r) {
            (WhenValue::Num(a), WhenValue::Num(b)) => match op {
                CmpOp::Lt => a < b,
                CmpOp::Le => a <= b,
                CmpOp::Gt => a > b,
                _ => a >= b,
            },
            _ => false,
        },
    }
}

pub(super) fn eval(e: &Expr, ctx: &WhenContext) -> Result<WhenValue, WhenEvalError> {
    Ok(match e {
        Expr::Lit(Lit::Bool(b)) => WhenValue::Bool(*b),
        Expr::Lit(Lit::Num(n)) => WhenValue::Num(*n),
        Expr::Lit(Lit::Str(s)) => WhenValue::Str(s.clone()),
        Expr::Ident(name, t) => match ctx.get(name) {
            None | Some(WhenValue::Null) => WhenValue::Null,
            Some(v) if type_ok(t, v) => v.clone(),
            Some(_) => {
                return Err(WhenEvalError::ContextType {
                    identifier: (*name).to_owned(),
                });
            }
        },
        Expr::Not(i) => WhenValue::Bool(!truthy(&eval(i, ctx)?)),
        Expr::And(a, b) => WhenValue::Bool(truthy(&eval(a, ctx)?) && truthy(&eval(b, ctx)?)),
        Expr::Or(a, b) => WhenValue::Bool(truthy(&eval(a, ctx)?) || truthy(&eval(b, ctx)?)),
        Expr::Cmp(op, a, b) => {
            let (l, r) = (eval(a, ctx)?, eval(b, ctx)?);
            WhenValue::Bool(compare(*op, &l, &r))
        }
    })
}
