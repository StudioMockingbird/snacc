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

/// Syntax retains unsupported literal forms so type checking can own their
/// diagnostics instead of leaking them into the backend.
#[derive(Clone, Debug, PartialEq)]
pub enum Value<'src> {
    Nil,
    Bool(bool),
    Num(f64, bool),
    Str(&'src str),
}

impl std::fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Nil => write!(f, "nil"),
            Self::Bool(x) => write!(f, "{x}"),
            Self::Num(x, _) => write!(f, "{x}"),
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

/// Child spans preserve source locations across parsing and type checking.
#[derive(Debug)]
pub enum Expr<'src> {
    Error,
    Value(Value<'src>),
    List(Vec<Spanned<Self>>),
    Local(&'src str),
    Let(&'src str, TypeName, Box<Spanned<Self>>, Box<Spanned<Self>>),
    Then(Box<Spanned<Self>>, Box<Spanned<Self>>),
    Binary(Box<Spanned<Self>>, BinaryOp, Box<Spanned<Self>>),
    Call(Box<Spanned<Self>>, Spanned<Vec<Spanned<Self>>>),
    If(Box<Spanned<Self>>, Box<Spanned<Self>>, Box<Spanned<Self>>),
    While(Box<Spanned<Self>>, Box<Spanned<Self>>),
    Print(Box<Spanned<Self>>),
}

#[derive(Debug)]
pub struct Func<'src> {
    pub args: Vec<Param<'src>>,
    pub ret: TypeName,
    pub span: Span,
    pub body: Spanned<Expr<'src>>,
}

#[derive(Debug)]
pub struct ExternFunc<'src> {
    pub symbol: &'src str,
    pub args: Vec<Param<'src>>,
    pub ret: TypeName,
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
    pub body: Option<Spanned<Expr<'src>>>,
}
