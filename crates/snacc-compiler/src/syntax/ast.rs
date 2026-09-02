use std::collections::HashMap;

pub type Span = chumsky::span::SimpleSpan;
pub type Spanned<T> = (T, Span);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeName {
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

impl std::fmt::Display for TypeName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let name = match self {
            Self::Dec64 => "Dec64",
            Self::Int64 => "Int64",
            Self::Bool => "Bool",
            Self::Nil => "Nil",
            Self::UInt8 => "UInt8",
            Self::UInt16 => "UInt16",
            Self::UInt32 => "UInt32",
            Self::UInt64 => "UInt64",
            Self::Float32 => "Float32",
        };
        f.write_str(name)
    }
}

/// A written type: either a built-in name or a qualified path naming a
/// user-defined type (`Point`, `Shape.Circle`). Specification 010 section 5
/// replaces the built-in-only type position with this path form; resolution,
/// not the parser, decides what a path denotes.
#[derive(Clone, Debug)]
pub enum TypeRef<'src> {
    Builtin(TypeName),
    Named(Vec<Spanned<&'src str>>),
}

impl std::fmt::Display for TypeRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Builtin(name) => write!(f, "{name}"),
            Self::Named(segments) => {
                let path = segments
                    .iter()
                    .map(|(segment, _)| *segment)
                    .collect::<Vec<_>>()
                    .join(".");
                f.write_str(&path)
            }
        }
    }
}

/// A numeric literal's exact value. Every literal form has its own variant so
/// no magnitude is ever narrowed or re-rounded on its way to the backend: an
/// integer never passes through `f64`, and a `Float32` is rounded once, from
/// the source decimal, by the lexer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumLiteral {
    Int(i64),
    Dec(f64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
}

impl std::fmt::Display for NumLiteral {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Int(x) => write!(f, "{x}"),
            Self::Dec(x) => write!(f, "{x}"),
            Self::U8(x) => write!(f, "{x}u8"),
            Self::U16(x) => write!(f, "{x}u16"),
            Self::U32(x) => write!(f, "{x}u32"),
            Self::U64(x) => write!(f, "{x}u64"),
            Self::F32(x) => write!(f, "{x}f32"),
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

/// One call argument. `name` is present for the `identifier : expression` form,
/// which Specification 010 section 5 permits only for struct construction; the
/// parser records it without deciding what the call head denotes.
#[derive(Debug)]
pub struct Arg<'src> {
    pub name: Option<Spanned<&'src str>>,
    pub value: Spanned<Expr<'src>>,
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
    /// The implicit method receiver. Rejected outside a method body.
    SelfRef,
    /// A built-in type name in expression position. Only valid as a call head,
    /// where it unwraps one represented-type layer (Specification 010 7.2).
    BuiltinType(TypeName),
    /// `base . name`. Resolution decides whether this is a field, a qualified
    /// type path, or a method that was used without a call.
    Member(Box<Spanned<Self>>, Spanned<&'src str>),
    Binary(Box<Spanned<Self>>, BinaryOp, Box<Spanned<Self>>),
    Call(Box<Spanned<Self>>, Spanned<Vec<Arg<'src>>>),
    Print(Box<Spanned<Self>>),
}

/// The syntactic root of a place: a named binding or `self`.
#[derive(Clone, Copy, Debug)]
pub enum PlaceRootName<'src> {
    Name(&'src str),
    SelfRef,
}

impl std::fmt::Display for PlaceRootName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::SelfRef => f.write_str("self"),
        }
    }
}

/// `place = ( identifier | "self" ), { ".", identifier }` -- Specification 012
/// section 4. Assignment targets and `is` subjects are both this shape.
#[derive(Clone, Debug)]
pub struct PlacePath<'src> {
    pub root: PlaceRootName<'src>,
    pub root_span: Span,
    pub fields: Vec<Spanned<&'src str>>,
    pub span: Span,
}

impl std::fmt::Display for PlacePath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.root)?;
        for (field, _) in &self.fields {
            write!(f, ".{field}")?;
        }
        Ok(())
    }
}

/// `place is Member` or `place is Member(binding)`. Valid only as the complete
/// condition of an `if` or `elseif` (Specification 010 section 12.2).
#[derive(Debug)]
pub struct TypeTest<'src> {
    pub place: PlacePath<'src>,
    /// The tested member's written path. `Nil` appears as one segment.
    pub member: Vec<Spanned<&'src str>>,
    pub member_span: Span,
    pub binding: Option<Spanned<&'src str>>,
    pub span: Span,
}

/// An `if`/`elseif` condition: an ordinary `Bool` expression or a type test.
#[derive(Debug)]
pub enum Condition<'src> {
    Expr(Spanned<Expr<'src>>),
    TypeTest(TypeTest<'src>),
}

impl Condition<'_> {
    pub fn span(&self) -> Span {
        match self {
            Self::Expr((_, span)) => *span,
            Self::TypeTest(test) => test.span,
        }
    }
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
        ty: Spanned<TypeRef<'src>>,
        value: Spanned<Expr<'src>>,
    },
    Assign {
        place: PlacePath<'src>,
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
    pub arms: Vec<(Condition<'src>, Block<'src>)>,
    pub else_branch: Option<Block<'src>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Func<'src> {
    pub args: Vec<Param<'src>>,
    /// `None` declares a function without a result.
    pub ret: Option<Spanned<TypeRef<'src>>>,
    pub span: Span,
    pub body: Block<'src>,
}

#[derive(Debug)]
pub struct ExternFunc<'src> {
    pub symbol: &'src str,
    pub args: Vec<Param<'src>>,
    /// `None` declares a bridge without a result.
    pub ret: Option<Spanned<TypeRef<'src>>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Param<'src> {
    pub name: &'src str,
    pub ty: Spanned<TypeRef<'src>>,
    pub span: Span,
}

/// `type N is ...`. Declaration order is preserved so `TypeId` allocation is
/// deterministic (Specification 010 section 19 phase 2).
#[derive(Debug)]
pub struct TypeDecl<'src> {
    pub name: &'src str,
    pub name_span: Span,
    pub body: TypeBody<'src>,
    pub span: Span,
}

#[derive(Debug)]
pub enum TypeBody<'src> {
    Represented(Spanned<TypeRef<'src>>),
    Struct(Vec<FieldDecl<'src>>),
    Union(Vec<UnionMemberDecl<'src>>),
}

#[derive(Debug)]
pub struct FieldDecl<'src> {
    pub name: &'src str,
    pub name_span: Span,
    pub ty: Spanned<TypeRef<'src>>,
}

/// One union alternative. A bare alternative is exactly an empty inline struct
/// (Specification 010 section 6.2), so both spell out to the same shape here.
#[derive(Debug)]
pub struct UnionMemberDecl<'src> {
    /// `Nil` for the special member; otherwise the declared identifier.
    pub name: &'src str,
    pub name_span: Span,
    pub nil: bool,
    pub fields: Vec<FieldDecl<'src>>,
}

/// `method Receiver.name(...) do ... end`. `path` holds every written
/// component; the last is the method name and the rest name the receiver type.
#[derive(Debug)]
pub struct MethodDecl<'src> {
    pub path: Vec<Spanned<&'src str>>,
    pub args: Vec<Param<'src>>,
    pub ret: Option<Spanned<TypeRef<'src>>>,
    pub span: Span,
    pub body: Block<'src>,
}

impl<'src> MethodDecl<'src> {
    /// The receiver path and the method name, or `None` when fewer than two
    /// components were written.
    pub fn split(&self) -> Option<(&[Spanned<&'src str>], Spanned<&'src str>)> {
        let (name, receiver) = self.path.split_last()?;
        (!receiver.is_empty()).then_some((receiver, *name))
    }
}

pub struct Program<'src> {
    pub funcs: HashMap<&'src str, Func<'src>>,
    pub externs: HashMap<&'src str, ExternFunc<'src>>,
    /// Type declarations in source order; their order fixes `TypeId` values.
    pub types: Vec<TypeDecl<'src>>,
    /// Method declarations in source order; their order fixes `MethodId` values.
    pub methods: Vec<MethodDecl<'src>>,
    /// The top-level executable block. An empty program is a block with zero
    /// elements, so this is never optional.
    pub body: Block<'src>,
}
