use crate::syntax::ast::{BinaryOp, Expr, Program as AstProgram, Span, Spanned, TypeName, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Dec64,
    Int64,
    Bool,
    Nil,
}

impl From<TypeName> for Ty {
    fn from(value: TypeName) -> Self {
        match value {
            TypeName::Dec64 => Self::Dec64,
            TypeName::Int64 => Self::Int64,
            TypeName::Bool => Self::Bool,
            TypeName::Nil => Self::Nil,
        }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Dec64 => write!(f, "Dec64"),
            Self::Int64 => write!(f, "Int64"),
            Self::Bool => write!(f, "Bool"),
            Self::Nil => write!(f, "Nil"),
        }
    }
}

pub struct Error {
    pub span: Span,
    pub msg: String,
}

pub enum Failure {
    Source(Vec<Error>),
    Unknown(&'static str),
}

impl Error {
    fn mismatch(span: Span, expected: Ty, found: Ty) -> Self {
        Self {
            span,
            msg: format!("expected '{expected}', found '{found}'"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy)]
pub enum CmpOp {
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
}

pub enum TExpr {
    Num(f64, Ty),
    Bool(bool),
    Nil,
    Local(String),
    Let(String, Box<TExpr>, Box<TExpr>),
    Then(Box<TExpr>, Box<TExpr>),
    Arith(Box<TExpr>, ArithOp, Box<TExpr>, Ty),
    Cmp(Box<TExpr>, CmpOp, Box<TExpr>, Ty),
    Call(String, Vec<TExpr>),
    If(Box<TExpr>, Box<TExpr>, Box<TExpr>, Ty),
    While(Box<TExpr>, Box<TExpr>, Ty),
    Print(Box<TExpr>, Ty),
    Cast(Box<TExpr>, Ty),
}

pub struct TFunc {
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub body: TExpr,
}

pub struct TExtern {
    pub symbol: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
}

pub struct Program {
    pub funcs: HashMap<String, TFunc>,
    pub externs: HashMap<String, TExtern>,
    pub body: Option<TExpr>,
}

#[derive(Clone)]
struct FuncSig {
    params: Vec<Ty>,
    ret: Ty,
}

struct Ctx<'src> {
    sigs: HashMap<&'src str, FuncSig>,
    errors: Vec<Error>,
    unknown: Option<&'static str>,
}

pub fn check<'src>(program: &AstProgram<'src>) -> Result<Program, Failure> {
    let mut ctx = Ctx {
        sigs: HashMap::new(),
        errors: Vec::new(),
        unknown: None,
    };

    for (name, function) in &program.funcs {
        let params = function.args.iter().map(|param| param.ty.into()).collect();
        ctx.sigs.insert(
            *name,
            FuncSig {
                params,
                ret: function.ret.into(),
            },
        );
    }
    for (name, function) in &program.externs {
        let params = function.args.iter().map(|param| param.ty.into()).collect();
        ctx.sigs.insert(
            *name,
            FuncSig {
                params,
                ret: function.ret.into(),
            },
        );
        if !function.symbol.starts_with("snacc_user_") {
            ctx.errors.push(Error {
                span: function.span,
                msg: "Rust bridge symbols must start with 'snacc_user_'".into(),
            });
        }
    }

    let mut names: Vec<&str> = program.funcs.keys().copied().collect();
    names.sort_unstable();
    let mut typed_funcs = HashMap::new();
    for name in names {
        let function = &program.funcs[name];
        let mut env = Vec::new();
        let mut params = Vec::new();
        for param in &function.args {
            let ty = param.ty.into();
            env.push((param.name, ty));
            params.push((param.name.to_string(), ty));
        }
        let (body, body_ty) = check_expr(&mut ctx, &mut env, &function.body);
        let ret = function.ret.into();
        let body = coerce(&mut ctx, body, body_ty, ret, function.body.1);
        typed_funcs.insert(name.to_string(), TFunc { params, ret, body });
    }

    let mut typed_externs = HashMap::new();
    let mut extern_names: Vec<&str> = program.externs.keys().copied().collect();
    extern_names.sort_unstable();
    for name in extern_names {
        let function = &program.externs[name];
        let params = function
            .args
            .iter()
            .map(|param| (param.name.to_string(), param.ty.into()))
            .collect();
        typed_externs.insert(
            name.to_string(),
            TExtern {
                symbol: function.symbol.to_string(),
                params,
                ret: function.ret.into(),
            },
        );
    }

    let body = program.body.as_ref().map(|expression| {
        let mut env = Vec::new();
        check_expr(&mut ctx, &mut env, expression).0
    });

    if let Some(detail) = ctx.unknown {
        return Err(Failure::Unknown(detail));
    }
    if ctx.errors.is_empty() {
        Ok(Program {
            funcs: typed_funcs,
            externs: typed_externs,
            body,
        })
    } else {
        Err(Failure::Source(ctx.errors))
    }
}

fn numeric(ty: Ty) -> bool {
    matches!(ty, Ty::Dec64 | Ty::Int64)
}

fn common_numeric(left: Ty, right: Ty) -> Option<Ty> {
    if !numeric(left) || !numeric(right) {
        return None;
    }
    Some(if left == Ty::Dec64 || right == Ty::Dec64 {
        Ty::Dec64
    } else {
        Ty::Int64
    })
}

fn assignable(from: Ty, to: Ty) -> bool {
    from == to || (from == Ty::Int64 && to == Ty::Dec64)
}

fn coerce(ctx: &mut Ctx<'_>, value: TExpr, from: Ty, to: Ty, span: Span) -> TExpr {
    if from == to {
        value
    } else if from == Ty::Int64 && to == Ty::Dec64 {
        TExpr::Cast(Box::new(value), Ty::Dec64)
    } else {
        ctx.errors.push(Error::mismatch(span, to, from));
        value
    }
}

fn check_expr<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Vec<(&'src str, Ty)>,
    expression: &Spanned<Expr<'src>>,
) -> (TExpr, Ty) {
    let span = expression.1;
    match &expression.0 {
        Expr::Error => {
            ctx.unknown = Some("a parser recovery node escaped into type checking");
            (TExpr::Num(0.0, Ty::Int64), Ty::Int64)
        }
        Expr::Value(Value::Num(value, is_float)) => {
            let ty = if *is_float { Ty::Dec64 } else { Ty::Int64 };
            (TExpr::Num(*value, ty), ty)
        }
        Expr::Value(Value::Bool(value)) => (TExpr::Bool(*value), Ty::Bool),
        Expr::Value(Value::Nil) => (TExpr::Nil, Ty::Nil),
        Expr::Value(Value::Str(_)) => {
            ctx.errors.push(Error {
                span,
                msg: "strings are not supported by the AOT backend".into(),
            });
            (TExpr::Nil, Ty::Nil)
        }
        Expr::List(items) => {
            for item in items {
                check_expr(ctx, env, item);
            }
            ctx.errors.push(Error {
                span,
                msg: "lists are not supported by the AOT backend".into(),
            });
            (TExpr::Nil, Ty::Nil)
        }
        Expr::Local(name) => {
            for (bound_name, ty) in env.iter().rev() {
                if bound_name == name {
                    return (TExpr::Local((*name).to_string()), *ty);
                }
            }
            if ctx.sigs.contains_key(name) {
                ctx.errors.push(Error {
                    span,
                    msg: format!("'{name}' is a function; functions cannot be used as values"),
                });
            } else {
                ctx.errors.push(Error {
                    span,
                    msg: format!("No such variable '{name}' in scope"),
                });
            }
            (TExpr::Nil, Ty::Nil)
        }
        Expr::Let(name, declared, value, body) => {
            let declared = (*declared).into();
            let value_span = value.1;
            let (value, value_ty) = check_expr(ctx, env, value);
            let value = coerce(ctx, value, value_ty, declared, value_span);
            env.push((name, declared));
            let (body, body_ty) = check_expr(ctx, env, body);
            env.pop();
            (
                TExpr::Let((*name).to_string(), Box::new(value), Box::new(body)),
                body_ty,
            )
        }
        Expr::Then(first, second) => {
            let (first, _) = check_expr(ctx, env, first);
            let (second, ty) = check_expr(ctx, env, second);
            (TExpr::Then(Box::new(first), Box::new(second)), ty)
        }
        Expr::Binary(left, op, right) => {
            let (left, left_ty) = check_expr(ctx, env, left);
            let (right, right_ty) = check_expr(ctx, env, right);
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    let ty = common_numeric(left_ty, right_ty).unwrap_or_else(|| {
                        ctx.errors.push(Error {
                            span,
                            msg: "arithmetic operands must be numeric".into(),
                        });
                        Ty::Int64
                    });
                    let left = coerce(ctx, left, left_ty, ty, span);
                    let right = coerce(ctx, right, right_ty, ty, span);
                    let operation = match op {
                        BinaryOp::Add => ArithOp::Add,
                        BinaryOp::Sub => ArithOp::Sub,
                        BinaryOp::Mul => ArithOp::Mul,
                        BinaryOp::Div => ArithOp::Div,
                        _ => unreachable!(),
                    };
                    (
                        TExpr::Arith(Box::new(left), operation, Box::new(right), ty),
                        ty,
                    )
                }
                BinaryOp::Eq | BinaryOp::NotEq => {
                    let operand_ty = common_numeric(left_ty, right_ty).unwrap_or(left_ty);
                    if !assignable(left_ty, operand_ty)
                        || !assignable(right_ty, operand_ty)
                        || (!numeric(left_ty) && left_ty != right_ty)
                    {
                        ctx.errors.push(Error::mismatch(span, left_ty, right_ty));
                    }
                    let left = coerce(ctx, left, left_ty, operand_ty, span);
                    let right = coerce(ctx, right, right_ty, operand_ty, span);
                    let operation = if matches!(op, BinaryOp::Eq) {
                        CmpOp::Eq
                    } else {
                        CmpOp::NotEq
                    };
                    (
                        TExpr::Cmp(Box::new(left), operation, Box::new(right), operand_ty),
                        Ty::Bool,
                    )
                }
                _ => {
                    let operand_ty = common_numeric(left_ty, right_ty).unwrap_or_else(|| {
                        ctx.errors.push(Error {
                            span,
                            msg: "ordered comparison operands must be numeric".into(),
                        });
                        Ty::Int64
                    });
                    let left = coerce(ctx, left, left_ty, operand_ty, span);
                    let right = coerce(ctx, right, right_ty, operand_ty, span);
                    let operation = match op {
                        BinaryOp::Less => CmpOp::Less,
                        BinaryOp::LessEq => CmpOp::LessEq,
                        BinaryOp::Greater => CmpOp::Greater,
                        BinaryOp::GreaterEq => CmpOp::GreaterEq,
                        _ => unreachable!(),
                    };
                    (
                        TExpr::Cmp(Box::new(left), operation, Box::new(right), operand_ty),
                        Ty::Bool,
                    )
                }
            }
        }
        Expr::Call(function, arguments) => {
            let Expr::Local(name) = &function.0 else {
                ctx.errors.push(Error {
                    span: function.1,
                    msg: "only calling a function by name is supported".into(),
                });
                return (TExpr::Nil, Ty::Nil);
            };
            let Some(signature) = ctx.sigs.get(name).cloned() else {
                ctx.errors.push(Error {
                    span: function.1,
                    msg: format!("'{name}' is not callable"),
                });
                return (TExpr::Nil, Ty::Nil);
            };
            let mut checked = Vec::new();
            for argument in &arguments.0 {
                let (value, value_ty) = check_expr(ctx, env, argument);
                checked.push((value, value_ty, argument.1));
            }
            if signature.params.len() != checked.len() {
                ctx.errors.push(Error {
                    span,
                    msg: format!(
                        "'{name}' called with wrong number of arguments (expected {}, found {})",
                        signature.params.len(),
                        checked.len()
                    ),
                });
            }
            let mut args = Vec::new();
            for (index, (value, value_ty, arg_span)) in checked.into_iter().enumerate() {
                if let Some(expected) = signature.params.get(index) {
                    args.push(coerce(ctx, value, value_ty, *expected, arg_span));
                } else {
                    args.push(value);
                }
            }
            (TExpr::Call((*name).to_string(), args), signature.ret)
        }
        Expr::If(condition, then_branch, else_branch) => {
            let condition_span = condition.1;
            let (condition, condition_ty) = check_expr(ctx, env, condition);
            if condition_ty != Ty::Bool {
                ctx.errors
                    .push(Error::mismatch(condition_span, Ty::Bool, condition_ty));
            }
            let (then_branch, then_ty) = check_expr(ctx, env, then_branch);
            let (else_branch, else_ty) = check_expr(ctx, env, else_branch);
            let ty = common_numeric(then_ty, else_ty).unwrap_or(then_ty);
            if !assignable(then_ty, ty) || !assignable(else_ty, ty) {
                ctx.errors.push(Error::mismatch(span, then_ty, else_ty));
            }
            let then_branch = coerce(ctx, then_branch, then_ty, ty, span);
            let else_branch = coerce(ctx, else_branch, else_ty, ty, span);
            (
                TExpr::If(
                    Box::new(condition),
                    Box::new(then_branch),
                    Box::new(else_branch),
                    ty,
                ),
                ty,
            )
        }
        Expr::While(condition, body) => {
            let condition_span = condition.1;
            let (condition, condition_ty) = check_expr(ctx, env, condition);
            if condition_ty != Ty::Bool {
                ctx.errors
                    .push(Error::mismatch(condition_span, Ty::Bool, condition_ty));
            }
            let (body, body_ty) = check_expr(ctx, env, body);
            (
                TExpr::While(Box::new(condition), Box::new(body), body_ty),
                body_ty,
            )
        }
        Expr::Print(value) => {
            let (value, ty) = check_expr(ctx, env, value);
            (TExpr::Print(Box::new(value), ty), ty)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_recovery_nodes_are_compiler_bugs_after_parsing() {
        let span: Span = (0..0).into();
        let mut funcs = HashMap::new();
        funcs.insert(
            "recovered",
            crate::syntax::ast::Func {
                args: Vec::new(),
                ret: TypeName::Nil,
                span,
                body: (Expr::Error, span),
            },
        );
        let program = AstProgram {
            funcs,
            externs: HashMap::new(),
            body: None,
        };

        match check(&program) {
            Err(Failure::Unknown(detail)) => {
                assert_eq!(detail, "a parser recovery node escaped into type checking");
            }
            _ => panic!("recovery node was not classified as a compiler bug"),
        }
    }

    #[test]
    fn checks_typed_rust_bridge_calls() {
        let source = "extern rust \"snacc_user_double\" fun rust_double(value: Int64): Int64\nprint(rust_double(2))";
        let syntax = crate::parse(source).expect("bridge declaration should parse");
        let program = match check(&syntax) {
            Ok(program) => program,
            Err(_) => panic!("bridge call should type check"),
        };
        assert_eq!(program.externs["rust_double"].symbol, "snacc_user_double");
    }
}
