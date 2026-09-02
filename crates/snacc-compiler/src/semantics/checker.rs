use crate::syntax::ast::{
    BinaryOp, Block, BlockElement, Expr, IfForm, NumLiteral, Param, Program as AstProgram, Span,
    Spanned, TypeName, Value,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Dec64,
    Int64,
    Bool,
    Nil,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
}

impl From<TypeName> for Ty {
    fn from(value: TypeName) -> Self {
        match value {
            TypeName::Dec64 => Self::Dec64,
            TypeName::Int64 => Self::Int64,
            TypeName::Bool => Self::Bool,
            TypeName::Nil => Self::Nil,
            TypeName::UInt8 => Self::UInt8,
            TypeName::UInt16 => Self::UInt16,
            TypeName::UInt32 => Self::UInt32,
            TypeName::UInt64 => Self::UInt64,
            TypeName::Float32 => Self::Float32,
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
            Self::UInt8 => write!(f, "UInt8"),
            Self::UInt16 => write!(f, "UInt16"),
            Self::UInt32 => write!(f, "UInt32"),
            Self::UInt64 => write!(f, "UInt64"),
            Self::Float32 => write!(f, "Float32"),
        }
    }
}

#[derive(Debug)]
pub struct Error {
    pub span: Span,
    pub msg: String,
}

#[derive(Debug)]
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

    /// Specification 009 section 6: a rejected operand pair names both types
    /// and the exact-match requirement.
    fn operands(span: Span, what: &str, left: Ty, right: Ty) -> Self {
        Self {
            span,
            msg: format!(
                "{what} operands must be two numbers of the same type, \
                 found '{left}' and '{right}'"
            ),
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

/// Value-producing checked nodes. Nothing here may stand for a construct
/// without a result: no sentinel type, dummy value, or fallback expression.
pub enum TExpr {
    Num(NumLiteral),
    Bool(bool),
    Nil,
    Local(String),
    Arith(Box<TExpr>, ArithOp, Box<TExpr>, Ty),
    Cmp(Box<TExpr>, CmpOp, Box<TExpr>, Ty),
    /// A call to a declaration that has a result.
    Call(String, Vec<TExpr>),
    /// An `if` classified as value-form: every branch is a value block and an
    /// `else` is present, so the form produces a value on every path.
    If(Box<TValueIf>),
    Print(Box<TExpr>, Ty),
    Cast(Box<TExpr>, Ty),
}

pub struct TValueIf {
    /// First arm is the `if`; remaining arms are `elseif`s, in source order.
    pub arms: Vec<(TExpr, TBlock)>,
    pub else_branch: TBlock,
    pub ty: Ty,
}

/// Checked statements. These perform control flow or effects and produce no
/// value, so none of them can satisfy a value-required block.
pub enum TStmt {
    Let {
        mutable: bool,
        name: String,
        ty: Ty,
        value: TExpr,
    },
    Assign {
        name: String,
        value: TExpr,
    },
    While {
        condition: TExpr,
        body: TBlock,
    },
    Break,
    /// An `if` classified as statement-form: `else` is optional and every
    /// branch is a no-result block.
    If(TStmtIf),
    /// A call to a declaration without a result.
    Call(String, Vec<TExpr>),
    /// A value-producing expression whose result is discarded.
    Expr(TExpr),
}

pub struct TStmtIf {
    pub arms: Vec<(TExpr, TBlock)>,
    pub else_branch: Option<TBlock>,
}

/// A block's ordered checked elements plus its optional resulting value.
/// `result` is `Some` only for a value-required block whose final element
/// supplied a value.
pub struct TBlock {
    pub statements: Vec<TStmt>,
    pub result: Option<TExpr>,
}

pub struct TFunc {
    pub params: Vec<(String, Ty)>,
    /// `None` is a function without a result; it lowers to LLVM `void`.
    pub result: Option<Ty>,
    pub body: TBlock,
}

pub struct TExtern {
    pub symbol: String,
    pub params: Vec<(String, Ty)>,
    /// `None` is a bridge without a result; its C ABI result is `void`.
    pub result: Option<Ty>,
    pub span: std::ops::Range<usize>,
}

pub struct Program {
    pub funcs: HashMap<String, TFunc>,
    pub externs: HashMap<String, TExtern>,
    pub body: TBlock,
}

#[derive(Clone)]
struct FuncSig {
    params: Vec<Ty>,
    result: Option<Ty>,
}

#[derive(Clone, Copy)]
struct Binding<'src> {
    name: &'src str,
    ty: Ty,
    mutable: bool,
}

struct Ctx<'src> {
    sigs: HashMap<&'src str, FuncSig>,
    /// Every name bound anywhere in the function or method being checked.
    /// Specification 012 section 5.2 makes this uniqueness rule function-wide,
    /// so nested blocks and sibling branches share one set.
    declared: Vec<&'src str>,
    /// One entry per enclosing `while` body. `break` needs a non-empty stack.
    loops: Vec<()>,
    errors: Vec<Error>,
    unknown: Option<&'static str>,
}

type Env<'src> = Vec<Binding<'src>>;

pub fn check<'src>(program: &AstProgram<'src>) -> Result<Program, Failure> {
    let mut ctx = Ctx {
        sigs: HashMap::new(),
        declared: Vec::new(),
        loops: Vec::new(),
        errors: Vec::new(),
        unknown: None,
    };

    for (name, function) in &program.funcs {
        let params = function.args.iter().map(|param| param.ty.into()).collect();
        ctx.sigs.insert(
            *name,
            FuncSig {
                params,
                result: function.ret.map(Into::into),
            },
        );
    }
    for (name, function) in &program.externs {
        check_duplicate_params(&mut ctx, &function.args);
        let params = function.args.iter().map(|param| param.ty.into()).collect();
        ctx.sigs.insert(
            *name,
            FuncSig {
                params,
                result: function.ret.map(Into::into),
            },
        );
        if !function.symbol.starts_with("snacc_user_") {
            ctx.errors.push(Error {
                span: function.span,
                msg: "Rust bridge symbols must start with 'snacc_user_'".into(),
            });
        } else if !is_rust_identifier(function.symbol) {
            ctx.errors.push(Error {
                span: function.span,
                msg: "Rust bridge symbols must be valid Rust identifiers".into(),
            });
        }
    }

    let mut names: Vec<&str> = program.funcs.keys().copied().collect();
    names.sort_unstable();
    let mut typed_funcs = HashMap::new();
    for name in names {
        let function = &program.funcs[name];
        // Parameters and locals share one function-wide binding namespace.
        ctx.declared.clear();
        ctx.loops.clear();
        let mut env = Env::new();
        let mut params = Vec::new();
        for param in &function.args {
            let ty = param.ty.into();
            declare(&mut ctx, param.name, param.span, "Parameter");
            env.push(Binding {
                name: param.name,
                ty,
                mutable: false,
            });
            params.push((param.name.to_string(), ty));
        }
        let result = function.ret.map(Ty::from);
        let body = match result {
            Some(expected) => check_value_block(&mut ctx, &mut env, &function.body, expected),
            None => check_statement_block(&mut ctx, &mut env, &function.body),
        };
        typed_funcs.insert(
            name.to_string(),
            TFunc {
                params,
                result,
                body,
            },
        );
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
                result: function.ret.map(Into::into),
                span: function.span.into_range(),
            },
        );
    }

    // The top-level executable body is a no-result block with its own binding
    // namespace; Snacc creates no implicit global state.
    ctx.declared.clear();
    ctx.loops.clear();
    let mut env = Env::new();
    let body = check_statement_block(&mut ctx, &mut env, &program.body);

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

fn check_duplicate_params(ctx: &mut Ctx<'_>, params: &[Param<'_>]) {
    let mut seen: Vec<&str> = Vec::new();
    for param in params {
        if seen.contains(&param.name) {
            ctx.errors.push(Error {
                span: param.span,
                msg: format!("Parameter '{}' already exists", param.name),
            });
        } else {
            seen.push(param.name);
        }
    }
}

/// Records a function-wide binding, reporting a duplicate rather than creating
/// a second layer for the same name.
fn declare<'src>(ctx: &mut Ctx<'src>, name: &'src str, span: Span, kind: &str) {
    if ctx.declared.contains(&name) {
        ctx.errors.push(Error {
            span,
            msg: format!("{kind} '{name}' already exists"),
        });
    } else {
        ctx.declared.push(name);
    }
}

fn is_rust_identifier(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The two types the one surviving implicit conversion joins. Specification
/// 009 section 4.4 adds no further promotion, so nothing else belongs here.
fn numeric(ty: Ty) -> bool {
    match ty {
        Ty::Dec64 | Ty::Int64 => true,
        Ty::Bool | Ty::Nil | Ty::UInt8 | Ty::UInt16 | Ty::UInt32 | Ty::UInt64 | Ty::Float32 => {
            false
        }
    }
}

/// Numeric types that operate only on an exact type match (Specification 009
/// sections 4.5-4.6): they never promote, not even to each other.
fn exact_match_numeric(ty: Ty) -> bool {
    match ty {
        Ty::UInt8 | Ty::UInt16 | Ty::UInt32 | Ty::UInt64 | Ty::Float32 => true,
        Ty::Dec64 | Ty::Int64 | Ty::Bool | Ty::Nil => false,
    }
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

/// The type an arithmetic or ordered-comparison pair shares, or `None` when the
/// operands cannot be combined at all.
fn operand_numeric(left: Ty, right: Ty) -> Option<Ty> {
    common_numeric(left, right)
        .or_else(|| (left == right && exact_match_numeric(left)).then_some(left))
}

/// The one implicit conversion Snacc has. Every other assignment, argument,
/// result, and branch requires an exact type match (Specification 009 4.4).
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

fn check_condition<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    condition: &Spanned<Expr<'src>>,
) -> TExpr {
    let (value, ty) = check_expr(ctx, env, condition);
    if ty != Ty::Bool {
        ctx.errors.push(Error::mismatch(condition.1, Ty::Bool, ty));
    }
    value
}

/// Checks a block that must supply a value of `expected`. Every element but
/// the last is a statement; the last shall be a value-producing expression or
/// a value-form `if`.
fn check_value_block<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    block: &Block<'src>,
    expected: Ty,
) -> TBlock {
    let scope = env.len();
    let mut statements = Vec::new();
    let mut result = None;
    let last = block.elements.len().wrapping_sub(1);
    for (index, element) in block.elements.iter().enumerate() {
        if index != last {
            statements.push(check_stmt(ctx, env, element));
            continue;
        }
        match &element.0 {
            BlockElement::Expr(expression) => {
                let (value, ty) = check_expr(ctx, env, expression);
                result = Some(coerce(ctx, value, ty, expected, expression.1));
            }
            BlockElement::If(form) => {
                result = Some(check_value_if(ctx, env, form, expected));
            }
            _ => {
                ctx.errors.push(Error {
                    span: element.1,
                    msg: format!(
                        "this block must end in an expression of type '{expected}', \
                         but it ends in a statement"
                    ),
                });
                statements.push(check_stmt(ctx, env, element));
            }
        }
    }
    if result.is_none() && block.elements.is_empty() {
        ctx.errors.push(Error {
            span: block.span,
            msg: format!(
                "this block must end in an expression of type '{expected}', but it is empty"
            ),
        });
    }
    env.truncate(scope);
    TBlock { statements, result }
}

/// Checks a block with no required final value. Every element is a statement;
/// a value-producing expression used as an element is simply discarded.
fn check_statement_block<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    block: &Block<'src>,
) -> TBlock {
    let scope = env.len();
    let statements = block
        .elements
        .iter()
        .map(|element| check_stmt(ctx, env, element))
        .collect();
    env.truncate(scope);
    TBlock {
        statements,
        result: None,
    }
}

fn check_value_if<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    form: &IfForm<'src>,
    expected: Ty,
) -> TExpr {
    let mut arms = Vec::new();
    for (condition, body) in &form.arms {
        let condition = check_condition(ctx, env, condition);
        let body = check_value_block(ctx, env, body, expected);
        arms.push((condition, body));
    }
    // Specification 010 will add an exhaustive union type-test chain as the
    // second way to cover every path; until unions exist, `else` is the only
    // one.
    let else_branch = match &form.else_branch {
        Some(body) => check_value_block(ctx, env, body, expected),
        None => {
            ctx.errors.push(Error {
                span: form.span,
                msg: format!(
                    "an 'if' that produces a value of type '{expected}' requires an 'else' branch"
                ),
            });
            TBlock {
                statements: Vec::new(),
                result: None,
            }
        }
    };
    TExpr::If(Box::new(TValueIf {
        arms,
        else_branch,
        ty: expected,
    }))
}

fn check_stmt<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    element: &Spanned<BlockElement<'src>>,
) -> TStmt {
    match &element.0 {
        BlockElement::Let {
            mutable,
            name,
            name_span,
            ty,
            value,
        } => {
            let declared = Ty::from(*ty);
            // The initializer is checked before the name is in scope, so it can
            // never refer to the variable being created.
            let (checked, value_ty) = check_expr(ctx, env, value);
            let checked = coerce(ctx, checked, value_ty, declared, value.1);
            declare(ctx, name, *name_span, "Variable");
            env.push(Binding {
                name,
                ty: declared,
                mutable: *mutable,
            });
            TStmt::Let {
                mutable: *mutable,
                name: (*name).to_string(),
                ty: declared,
                value: checked,
            }
        }
        BlockElement::Assign {
            name,
            name_span,
            value,
        } => {
            let target = env
                .iter()
                .rev()
                .find(|binding| binding.name == *name)
                .copied();
            let (checked, value_ty) = check_expr(ctx, env, value);
            let checked = match target {
                Some(binding) => {
                    if !binding.mutable {
                        ctx.errors.push(Error {
                            span: *name_span,
                            msg: format!("'{name}' is not declared 'mut' and cannot be assigned"),
                        });
                    }
                    coerce(ctx, checked, value_ty, binding.ty, value.1)
                }
                None => {
                    ctx.errors.push(Error {
                        span: *name_span,
                        msg: format!("No such variable '{name}' in scope"),
                    });
                    checked
                }
            };
            TStmt::Assign {
                name: (*name).to_string(),
                value: checked,
            }
        }
        BlockElement::While {
            condition, body, ..
        } => {
            let condition = check_condition(ctx, env, condition);
            ctx.loops.push(());
            let body = check_statement_block(ctx, env, body);
            ctx.loops.pop();
            TStmt::While { condition, body }
        }
        BlockElement::Break(span) => {
            if ctx.loops.is_empty() {
                ctx.errors.push(Error {
                    span: *span,
                    msg: "'break' is only valid inside a 'while' body, which is the only \
                          construct that establishes a loop target"
                        .into(),
                });
            }
            TStmt::Break
        }
        BlockElement::If(form) => {
            let mut arms = Vec::new();
            for (condition, body) in &form.arms {
                let condition = check_condition(ctx, env, condition);
                let body = check_statement_block(ctx, env, body);
                arms.push((condition, body));
            }
            let else_branch = form
                .else_branch
                .as_ref()
                .map(|body| check_statement_block(ctx, env, body));
            TStmt::If(TStmtIf { arms, else_branch })
        }
        BlockElement::Expr(expression) => {
            // A call to a declaration without a result is a call statement, not
            // an expression whose value is discarded.
            if let Expr::Call(function, arguments) = &expression.0
                && let Some((name, args, None)) =
                    check_call(ctx, env, expression.1, function, arguments)
            {
                return TStmt::Call(name, args);
            }
            TStmt::Expr(check_expr(ctx, env, expression).0)
        }
    }
}

/// Checks a call's callee and arguments. Returns `None` when the callee is not
/// a resolvable declaration (the diagnostic has already been recorded).
fn check_call<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    function: &Spanned<Expr<'src>>,
    arguments: &Spanned<Vec<Spanned<Expr<'src>>>>,
) -> Option<(String, Vec<TExpr>, Option<Ty>)> {
    let Expr::Local(name) = &function.0 else {
        ctx.errors.push(Error {
            span: function.1,
            msg: "only calling a function by name is supported".into(),
        });
        return None;
    };
    let Some(signature) = ctx.sigs.get(name).cloned() else {
        ctx.errors.push(Error {
            span: function.1,
            msg: format!("'{name}' is not callable"),
        });
        return None;
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
    Some(((*name).to_string(), args, signature.result))
}

fn check_expr<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    expression: &Spanned<Expr<'src>>,
) -> (TExpr, Ty) {
    let span = expression.1;
    match &expression.0 {
        Expr::Error => {
            ctx.unknown = Some("a parser recovery node escaped into type checking");
            (TExpr::Num(NumLiteral::Int(0)), Ty::Int64)
        }
        Expr::Value(Value::Num(literal)) => {
            let ty = match literal {
                NumLiteral::Int(_) => Ty::Int64,
                NumLiteral::Dec(_) => Ty::Dec64,
                NumLiteral::U8(_) => Ty::UInt8,
                NumLiteral::U16(_) => Ty::UInt16,
                NumLiteral::U32(_) => Ty::UInt32,
                NumLiteral::U64(_) => Ty::UInt64,
                NumLiteral::F32(_) => Ty::Float32,
            };
            (TExpr::Num(*literal), ty)
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
            for binding in env.iter().rev() {
                if binding.name == *name {
                    return (TExpr::Local((*name).to_string()), binding.ty);
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
        Expr::Binary(left, op, right) => {
            let (left, left_ty) = check_expr(ctx, env, left);
            let (right, right_ty) = check_expr(ctx, env, right);
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    let operation = match op {
                        BinaryOp::Add => ArithOp::Add,
                        BinaryOp::Sub => ArithOp::Sub,
                        BinaryOp::Mul => ArithOp::Mul,
                        BinaryOp::Div => ArithOp::Div,
                        _ => unreachable!(),
                    };
                    // A rejected pair keeps its own operand types rather than
                    // being coerced to a guessed one, so one mixed-type
                    // expression reports one diagnostic.
                    let Some(ty) = operand_numeric(left_ty, right_ty) else {
                        ctx.errors
                            .push(Error::operands(span, "arithmetic", left_ty, right_ty));
                        return (
                            TExpr::Arith(Box::new(left), operation, Box::new(right), left_ty),
                            left_ty,
                        );
                    };
                    let left = coerce(ctx, left, left_ty, ty, span);
                    let right = coerce(ctx, right, right_ty, ty, span);
                    (
                        TExpr::Arith(Box::new(left), operation, Box::new(right), ty),
                        ty,
                    )
                }
                BinaryOp::Eq | BinaryOp::NotEq => {
                    let operation = if matches!(op, BinaryOp::Eq) {
                        CmpOp::Eq
                    } else {
                        CmpOp::NotEq
                    };
                    // Equality joins the `Int64`/`Dec64` promotion pair; every
                    // other type compares only against itself.
                    let operand_ty = match common_numeric(left_ty, right_ty) {
                        Some(ty) => ty,
                        None if left_ty == right_ty => left_ty,
                        None => {
                            ctx.errors.push(Error::mismatch(span, left_ty, right_ty));
                            return (
                                TExpr::Cmp(Box::new(left), operation, Box::new(right), left_ty),
                                Ty::Bool,
                            );
                        }
                    };
                    let left = coerce(ctx, left, left_ty, operand_ty, span);
                    let right = coerce(ctx, right, right_ty, operand_ty, span);
                    (
                        TExpr::Cmp(Box::new(left), operation, Box::new(right), operand_ty),
                        Ty::Bool,
                    )
                }
                _ => {
                    let operation = match op {
                        BinaryOp::Less => CmpOp::Less,
                        BinaryOp::LessEq => CmpOp::LessEq,
                        BinaryOp::Greater => CmpOp::Greater,
                        BinaryOp::GreaterEq => CmpOp::GreaterEq,
                        _ => unreachable!(),
                    };
                    let Some(operand_ty) = operand_numeric(left_ty, right_ty) else {
                        ctx.errors.push(Error::operands(
                            span,
                            "ordered comparison",
                            left_ty,
                            right_ty,
                        ));
                        return (
                            TExpr::Cmp(Box::new(left), operation, Box::new(right), left_ty),
                            Ty::Bool,
                        );
                    };
                    let left = coerce(ctx, left, left_ty, operand_ty, span);
                    let right = coerce(ctx, right, right_ty, operand_ty, span);
                    (
                        TExpr::Cmp(Box::new(left), operation, Box::new(right), operand_ty),
                        Ty::Bool,
                    )
                }
            }
        }
        Expr::Call(function, arguments) => {
            let Some((name, args, result)) = check_call(ctx, env, span, function, arguments) else {
                return (TExpr::Nil, Ty::Nil);
            };
            match result {
                Some(ty) => (TExpr::Call(name, args), ty),
                None => {
                    ctx.errors.push(Error {
                        span,
                        msg: format!(
                            "'{name}' declares no result, so its call cannot be used as a value"
                        ),
                    });
                    (TExpr::Nil, Ty::Nil)
                }
            }
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
    use crate::syntax::ast::Block;

    fn errors(source: &str) -> Vec<Error> {
        let syntax =
            crate::parse(source).unwrap_or_else(|d| panic!("{source} should parse: {d:?}"));
        match check(&syntax) {
            Err(Failure::Source(errors)) => errors,
            Err(Failure::Unknown(detail)) => panic!("unexpected compiler bug: {detail}"),
            Ok(_) => panic!("expected a type error for: {source}"),
        }
    }

    fn assert_checks(source: &str) -> Program {
        let syntax =
            crate::parse(source).unwrap_or_else(|d| panic!("{source} should parse: {d:?}"));
        check(&syntax).unwrap_or_else(|failure| panic!("{source} should check: {failure:?}"))
    }

    fn assert_error_contains(source: &str, needle: &str) {
        let errors = errors(source);
        assert!(
            errors.iter().any(|error| error.msg.contains(needle)),
            "expected an error containing {needle:?} for {source}, got: {errors:?}"
        );
    }

    #[test]
    fn parser_recovery_nodes_are_compiler_bugs_after_parsing() {
        let span: Span = (0..0).into();
        let mut funcs = HashMap::new();
        funcs.insert(
            "recovered",
            crate::syntax::ast::Func {
                args: Vec::new(),
                ret: Some(TypeName::Nil),
                span,
                body: Block {
                    elements: vec![(BlockElement::Expr((Expr::Error, span)), span)],
                    span,
                },
            },
        );
        let program = AstProgram {
            funcs,
            externs: HashMap::new(),
            body: Block {
                elements: Vec::new(),
                span,
            },
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
        let program = assert_checks(
            "extern rust \"snacc_user_double\" fun rust_double(value: Int64): Int64\nprint(rust_double(2))",
        );
        assert_eq!(program.externs["rust_double"].symbol, "snacc_user_double");
        assert_eq!(program.externs["rust_double"].result, Some(Ty::Int64));
    }

    #[test]
    fn checked_externs_carry_their_declaration_span() {
        let source = "extern rust \"snacc_user_double\" fun rust_double(value: Int64): Int64\nprint(rust_double(2))";
        let program = assert_checks(source);
        let span = &program.externs["rust_double"].span;
        assert_eq!(span.start, 0);
        assert!(span.end > span.start && span.end <= source.find('\n').unwrap());
    }

    #[test]
    fn rejects_bridge_symbols_that_are_not_rust_identifiers() {
        assert_error_contains(
            "extern rust \"snacc_user_bad-name\" fun bad(): Nil\nprint(0)",
            "valid Rust identifiers",
        );
    }

    #[test]
    fn accepts_bridge_symbols_with_digits_and_underscores() {
        assert_checks("extern rust \"snacc_user_v2_ok\" fun ok(): Nil\nprint(0)");
    }

    #[test]
    fn rejects_duplicate_function_parameter_names() {
        let source = "fun f(a: Int64, a: Int64): Int64 do a end";
        let second_a = source.find(", a: Int64)").map(|i| i + 2).unwrap();
        let errors = errors(source);
        let error = errors
            .iter()
            .find(|error| error.msg.contains("Parameter 'a' already exists"))
            .unwrap_or_else(|| {
                panic!("expected a duplicate-parameter diagnostic, got: {errors:?}")
            });
        assert_eq!(
            error.span.start, second_a,
            "diagnostic should span the second 'a'"
        );
        assert_eq!(error.span.end, second_a + 1);
    }

    #[test]
    fn accepts_functions_with_distinct_parameter_names() {
        assert_checks("fun f(a: Int64, b: Int64): Int64 do a + b end");
    }

    // RFC 008 conformance 1: declarations with and without results.

    #[test]
    fn checks_functions_and_bridges_with_and_without_results() {
        let program = assert_checks(
            "extern rust \"snacc_user_log\" fun log(value: Int64)\n\
             extern rust \"snacc_user_double\" fun rust_double(value: Int64): Int64\n\
             fun announce(value: Int64) do print(value) end\n\
             fun double(value: Int64): Int64 do value * 2 end\n\
             announce(1)\n\
             log(2)\n\
             print(double(3))\n\
             print(rust_double(4))",
        );
        assert_eq!(program.funcs["announce"].result, None);
        assert_eq!(program.funcs["double"].result, Some(Ty::Int64));
        assert_eq!(program.externs["log"].result, None);
        assert_eq!(program.externs["rust_double"].result, Some(Ty::Int64));
    }

    // RFC 008 conformance 2: no-result calls as block elements, never as values.

    #[test]
    fn accepts_a_no_result_call_as_a_block_element() {
        let program = assert_checks("fun announce(value: Int64) do print(value) end\nannounce(1)");
        assert!(matches!(program.body.statements[0], TStmt::Call(_, _)));
        assert!(program.body.result.is_none());
    }

    #[test]
    fn rejects_a_no_result_call_in_every_expression_position() {
        let declaration = "fun announce(value: Int64) do print(value) end\n";
        for use_site in [
            "print(announce(1))",
            "let value: Int64 = announce(1)",
            "print(1 + announce(1))",
            "fun wrap(): Int64 do announce(1) end",
            "if announce(1) then print(0) end",
        ] {
            assert_error_contains(
                &format!("{declaration}{use_site}"),
                "declares no result, so its call cannot be used as a value",
            );
        }
    }

    // RFC 008 conformance 3: value-required bodies reject statements.

    #[test]
    fn rejects_a_value_required_body_ending_in_a_statement() {
        for body in ["let value: Int64 = 1", "while false do print(1) end"] {
            assert_error_contains(
                &format!("fun f(): Int64 do {body} end"),
                "must end in an expression of type 'Int64'",
            );
        }
    }

    #[test]
    fn rejects_a_value_required_body_ending_in_an_assignment() {
        assert_error_contains(
            "fun f(): Int64 do let mut x: Int64 = 1 x = 2 end",
            "must end in an expression of type 'Int64'",
        );
    }

    #[test]
    fn accepts_a_value_required_body_with_a_leading_statement() {
        assert_checks("fun f(value: Int64): Int64 do let result: Int64 = value * value result end");
    }

    #[test]
    fn accepts_a_statement_loop_followed_by_an_explicit_value() {
        // RFC 008's migration pattern for the old loop zero-value fallback.
        assert_checks(
            "fun zero_after_loop(value: Int64): Int64 do while false do print(value) end 0 end",
        );
    }

    // RFC 008 conformance 6: break targets and placement.

    #[test]
    fn rejects_break_outside_a_loop() {
        assert_error_contains("break", "only valid inside a 'while' body");
        assert_error_contains(
            "fun f() do if true then break end end",
            "only valid inside a 'while' body",
        );
    }

    #[test]
    fn accepts_break_inside_a_nested_loop_body() {
        assert_checks("while true do while true do break end break end");
    }

    #[test]
    fn a_loop_target_does_not_outlive_its_body() {
        // The stack must pop when the body ends, so a `break` after the loop
        // is still rejected.
        assert_error_contains(
            "while true do print(1) end break",
            "only valid inside a 'while' body",
        );
    }

    // RFC 008 conformance 7: statement-form vs value-form `if`.

    #[test]
    fn statement_form_if_accepts_an_omitted_else() {
        let program = assert_checks("if true then print(1) end");
        assert!(matches!(program.body.statements[0], TStmt::If(_)));
    }

    #[test]
    fn value_form_if_requires_an_else() {
        assert_error_contains(
            "fun f(): Int64 do if true then 1 end end",
            "requires an 'else' branch",
        );
    }

    #[test]
    fn value_form_if_checks_every_branch_against_the_required_type() {
        assert_checks("fun f(c: Bool): Int64 do if c then 1 elseif c then 2 else 3 end end");
        assert_error_contains(
            "fun f(c: Bool): Int64 do if c then 1 else true end end",
            "expected 'Int64', found 'Bool'",
        );
    }

    #[test]
    fn value_form_if_rejects_a_branch_that_ends_in_a_statement() {
        assert_error_contains(
            "fun f(c: Bool): Int64 do if c then print(1) else while false do print(2) end end end",
            "must end in an expression of type 'Int64'",
        );
    }

    #[test]
    fn rejects_a_non_bool_condition() {
        assert_error_contains("while 1 do print(1) end", "expected 'Bool', found 'Int64'");
        assert_error_contains("if 1 then print(1) end", "expected 'Bool', found 'Int64'");
    }

    // Specification 012 sections 5-6: declarations and root mutability.

    #[test]
    fn rejects_a_duplicate_local_declaration() {
        assert_error_contains(
            "let x: Int64 = 10\nlet x: Int64 = 20",
            "Variable 'x' already exists",
        );
    }

    #[test]
    fn rejects_a_duplicate_local_declared_in_a_nested_branch() {
        // Specification 012 section 5.2: uniqueness is function-wide, so a
        // nested block cannot reuse an outer name.
        assert_error_contains(
            "fun f(ready: Bool) do let value: Int64 = 1 if ready then let value: Int64 = 2 print(value) end end",
            "Variable 'value' already exists",
        );
    }

    #[test]
    fn rejects_a_local_that_shadows_a_parameter() {
        assert_error_contains(
            "fun f(value: Int64): Int64 do let value: Int64 = 1 value end",
            "Variable 'value' already exists",
        );
    }

    #[test]
    fn accepts_the_same_local_name_in_different_functions() {
        assert_checks(
            "fun f(): Int64 do let value: Int64 = 1 value end\n\
             fun g(): Int64 do let value: Int64 = 2 value end",
        );
    }

    #[test]
    fn an_initializer_cannot_refer_to_the_variable_being_declared() {
        assert_error_contains("let count: Int64 = count + 1", "No such variable 'count'");
    }

    #[test]
    fn rejects_assignment_to_an_immutable_root() {
        assert_error_contains(
            "let count: Int64 = 1\ncount = 2",
            "'count' is not declared 'mut' and cannot be assigned",
        );
    }

    #[test]
    fn accepts_assignment_to_a_mutable_root() {
        let program = assert_checks("let mut count: Int64 = 1\ncount = count + 1\nprint(count)");
        assert!(matches!(
            program.body.statements[0],
            TStmt::Let { mutable: true, .. }
        ));
        assert!(matches!(program.body.statements[1], TStmt::Assign { .. }));
    }

    #[test]
    fn rejects_an_assignment_type_mismatch() {
        assert_error_contains(
            "let mut count: Int64 = 1\ncount = true",
            "expected 'Int64', found 'Bool'",
        );
    }

    #[test]
    fn rejects_assignment_to_an_undeclared_name() {
        assert_error_contains("missing = 1", "No such variable 'missing' in scope");
    }

    #[test]
    fn a_declaration_does_not_escape_its_block() {
        assert_error_contains(
            "if true then let inner: Int64 = 1 print(inner) end\nprint(inner)",
            "No such variable 'inner' in scope",
        );
    }

    #[test]
    fn an_empty_value_required_body_is_rejected() {
        assert_error_contains("fun f(): Int64 do end", "but it is empty");
    }

    #[test]
    fn a_no_result_function_body_may_be_empty() {
        assert_checks("fun nothing() do end");
    }

    // Specification 009 sections 4.4-4.7: exact-match types.

    /// Each new type paired with a literal of that exact type.
    const NEW_TYPES: [(&str, &str); 5] = [
        ("UInt8", "1u8"),
        ("UInt16", "1u16"),
        ("UInt32", "1u32"),
        ("UInt64", "1u64"),
        ("Float32", "1.5f32"),
    ];

    #[test]
    fn accepts_every_new_type_in_every_declaration_position() {
        for (name, literal) in NEW_TYPES {
            let program = assert_checks(&format!(
                "extern rust \"snacc_user_edge\" fun edge(value: {name}): {name}\n\
                 fun identity(value: {name}): {name} do value end\n\
                 let bound: {name} = {literal}\n\
                 print(identity(bound))"
            ));
            let expected = program.funcs["identity"].result;
            assert!(expected.is_some());
            assert_eq!(program.funcs["identity"].params[0].1, expected.unwrap());
            assert_eq!(program.externs["edge"].result, expected);
        }
    }

    #[test]
    fn rejects_every_implicit_conversion_the_new_types_prohibit() {
        // Specification 009 section 4.4: no width converts to another width, to
        // or from Int64, to a float, and Float32 does not meet Dec64.
        for (source, needle) in [
            ("let byte: UInt8 = 1", "expected 'UInt8', found 'Int64'"),
            ("let byte: UInt8 = 1u16", "expected 'UInt8', found 'UInt16'"),
            (
                "let wide: UInt64 = 1u32",
                "expected 'UInt64', found 'UInt32'",
            ),
            (
                "let count: Int64 = 1u64",
                "expected 'Int64', found 'UInt64'",
            ),
            (
                "let ratio: Float32 = 1u8",
                "expected 'Float32', found 'UInt8'",
            ),
            (
                "let ratio: Float32 = 1.5",
                "expected 'Float32', found 'Dec64'",
            ),
            (
                "let wide: Dec64 = 1.5f32",
                "expected 'Dec64', found 'Float32'",
            ),
            ("let wide: Dec64 = 1u8", "expected 'Dec64', found 'UInt8'"),
        ] {
            assert_error_contains(source, needle);
        }
    }

    #[test]
    fn the_int64_to_dec64_conversion_still_works() {
        assert_checks("let wide: Dec64 = 1\nprint(wide + 1)");
    }

    #[test]
    fn accepts_same_type_arithmetic_and_comparison_for_every_new_type() {
        for (name, literal) in NEW_TYPES {
            for operator in ["+", "-", "*", "/"] {
                let source = format!("let result: {name} = {literal} {operator} {literal}");
                assert_checks(&source);
            }
            for operator in ["<", "<=", ">", ">=", "==", "!="] {
                let source = format!("let flag: Bool = {literal} {operator} {literal}");
                assert_checks(&source);
            }
        }
    }

    #[test]
    fn rejects_mixed_operands_in_every_category() {
        // Every pairing of a new type with a different type, in arithmetic,
        // ordered comparison, and equality.
        let others = ["1", "1.5", "1u8", "1u16", "1u32", "1u64", "1.5f32", "true"];
        for (_, literal) in NEW_TYPES {
            for other in others {
                if other == literal {
                    continue;
                }
                assert_error_contains(
                    &format!("print({literal} + {other})"),
                    "operands must be two numbers of the same type",
                );
                assert_error_contains(
                    &format!("print({literal} < {other})"),
                    "operands must be two numbers of the same type",
                );
                assert_error_contains(&format!("print({literal} == {other})"), "expected");
            }
        }
    }

    #[test]
    fn a_mixed_operand_pair_reports_one_diagnostic() {
        assert_eq!(errors("print(1u8 + 1)").len(), 1);
        assert_eq!(errors("print(1u8 < 1u16)").len(), 1);
        assert_eq!(errors("print(1u8 == 1)").len(), 1);
    }

    #[test]
    fn arithmetic_and_comparison_keep_the_exact_operand_type() {
        // The backend reads signedness and float width from this type, so it
        // must survive checking rather than being inferred from a bit width.
        for (name, literal) in NEW_TYPES {
            let program = assert_checks(&format!(
                "fun combine(): Bool do {literal} + {literal} < {literal} end\n\
                 let bound: {name} = {literal}"
            ));
            let TStmt::Let { ty, .. } = &program.body.statements[0] else {
                panic!("expected a let statement");
            };
            let result = program.funcs["combine"]
                .body
                .result
                .as_ref()
                .expect("combine produces a value");
            let TExpr::Cmp(left, _, _, operand_ty) = result else {
                panic!("expected a comparison");
            };
            assert_eq!(operand_ty, ty, "{name} comparison lost its operand type");
            let TExpr::Arith(_, _, _, arith_ty) = left.as_ref() else {
                panic!("expected arithmetic");
            };
            assert_eq!(arith_ty, ty, "{name} arithmetic lost its operand type");
        }
    }

    #[test]
    fn print_accepts_every_new_type_and_returns_it() {
        for (name, literal) in NEW_TYPES {
            assert_checks(&format!("let echoed: {name} = print({literal})"));
        }
    }
}
