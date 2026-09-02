use std::collections::HashMap;

pub type Span = chumsky::span::SimpleSpan;
pub type Spanned<T> = (T, Span);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeName {
    Dec64,
    Int64,
    Bool,
    Nil,
}

/// A numeric literal's exact value, carried without ever passing through
/// `f64` for an integer literal (see TODO item on Int64 precision).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumLiteral {
    Int(i64),
    Dec(f64),
}

impl std::fmt::Display for NumLiteral {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Int(x) => write!(f, "{x}"),
            Self::Dec(x) => write!(f, "{x}"),
        }
    }
}

/// Syntax retains unsupported literal forms so type checking can own their
/// diagnostics instead of leaking them into the backend.
#[derive(Clone, Debug, PartialEq)]
pub enum Value<'src> {
    Nil,
    Bool(bool),
    Num(NumLiteral),
    Str(&'src str),
}

impl std::fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Nil => write!(f, "nil"),
            Self::Bool(x) => write!(f, "{x}"),
            Self::Num(x) => write!(f, "{x}"),
            Self::Str(x) => write!(f, "{x}"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
}

/// Only value-producing forms live here. Statements and `if` are block
/// elements (see [`BlockElement`]) so no construct without a value can ever
/// reach an expression position.
///
/// Child spans preserve source locations across parsing and type checking.
#[derive(Debug)]
pub enum Expr<'src> {
    Error,
    Value(Value<'src>),
    List(Vec<Spanned<Self>>),
    Local(&'src str),
    Binary(Box<Spanned<Self>>, BinaryOp, Box<Spanned<Self>>),
    Call(Box<Spanned<Self>>, Spanned<Vec<Spanned<Self>>>),
    Print(Box<Spanned<Self>>),
}

/// An ordered list of block elements. Whether the block must produce a value
/// is decided by its position, not its syntax, so the parser records no such
/// distinction.
#[derive(Debug)]
pub struct Block<'src> {
    pub elements: Vec<Spanned<BlockElement<'src>>>,
    pub span: Span,
}

#[derive(Debug)]
pub enum BlockElement<'src> {
    Let {
        mutable: bool,
        name: &'src str,
        name_span: Span,
        ty: TypeName,
        value: Spanned<Expr<'src>>,
    },
    Assign {
        name: &'src str,
        name_span: Span,
        value: Spanned<Expr<'src>>,
    },
    While {
        condition: Spanned<Expr<'src>>,
        body: Block<'src>,
        span: Span,
    },
    Break(Span),
    If(IfForm<'src>),
    Expr(Spanned<Expr<'src>>),
}

/// One `if` syntax form. The checker classifies each occurrence as
/// statement-form or value-form purely from where it sits.
#[derive(Debug)]
pub struct IfForm<'src> {
    /// First arm is the `if`; remaining arms are `elseif`s, in source order.
    pub arms: Vec<(Spanned<Expr<'src>>, Block<'src>)>,
    pub else_branch: Option<Block<'src>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Func<'src> {
    pub args: Vec<Param<'src>>,
    /// `None` declares a function without a result.
    pub ret: Option<TypeName>,
    pub span: Span,
    pub body: Block<'src>,
}

#[derive(Debug)]
pub struct ExternFunc<'src> {
    pub symbol: &'src str,
    pub args: Vec<Param<'src>>,
    /// `None` declares a bridge without a result.
    pub ret: Option<TypeName>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Param<'src> {
    pub name: &'src str,
    pub ty: TypeName,
    pub span: Span,
}

pub struct Program<'src> {
    pub funcs: HashMap<&'src str, Func<'src>>,
    pub externs: HashMap<&'src str, ExternFunc<'src>>,
    /// The top-level executable block. An empty program is a block with zero
    /// elements, so this is never optional.
    pub body: Block<'src>,
}
