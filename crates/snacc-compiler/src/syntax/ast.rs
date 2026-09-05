use std::collections::HashMap;

pub type Span = chumsky::span::SimpleSpan;
pub type Spanned<T> = (T, Span);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeName {
    Float64,
    Int64,
    Bool,
    Nil,
    String,
    Unicode,
    Byte,
    UInt16,
    UInt32,
    UInt64,
    Float32,
}

impl std::fmt::Display for TypeName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let name = match self {
            Self::Float64 => "Float64",
            Self::Int64 => "Int64",
            Self::Bool => "Bool",
            Self::Nil => "Nil",
            Self::String => "String",
            Self::Unicode => "Unicode",
            Self::Byte => "Byte",
            Self::UInt16 => "UInt16",
            Self::UInt32 => "UInt32",
            Self::UInt64 => "UInt64",
            Self::Float32 => "Float32",
        };
        f.write_str(name)
    }
}

/// A written type: a built-in name, a qualified path naming a user-defined
/// type (`Point`, `Shape.Circle`), or an inline sum of two or more primary
/// member types (`Byte | Nil`, Specification 018 section 3). Resolution, not
/// the parser, decides what a path denotes, flattens nested sums, and
/// enforces member-set rules; the parser only records what was written, in
/// source order. A parenthesized sum member (`(A | B) | C`) is not a
/// distinct syntax node: the grouped sum-type is parsed recursively and
/// simply appears as one member's [`TypeRef::Sum`] here, so flattening is a
/// resolution concern, never a parser one.
#[derive(Clone, Debug)]
pub enum TypeRef<'src> {
    Builtin(TypeName),
    Named(Vec<Spanned<&'src str>>),
    /// An explicitly applied generic type such as `Pair<Int64, Bool>`.
    Apply {
        path: Vec<Spanned<&'src str>>,
        args: Vec<Spanned<Self>>,
    },
    Sum(Vec<Spanned<Self>>),
    /// `Box<T>` (Specification 016 section 4.1): a closed, single-argument
    /// built-in parameterized type, parsed with the same closed-angle-bracket
    /// tokenization already established for `Ref<T>`. Unlike `Ref<T>`, which
    /// is never a [`TypeRef`] at all (Specification 011 represents it as a
    /// [`ParamMode`] instead), `Box<T>` is an ordinary storable value type: it
    /// can appear as a sum member, a nested argument (`Box<Box<T>>`), and
    /// anywhere else this enum's other variants can.
    Box(std::boxed::Box<Spanned<Self>>),
    /// Closed built-in immutable view family: `View<Byte>` or
    /// `View<Unicode>`.
    View(std::boxed::Box<Spanned<Self>>),
    Array(std::boxed::Box<Spanned<Self>>, u64),
    List(std::boxed::Box<Spanned<Self>>),
    Map(
        std::boxed::Box<Spanned<Self>>,
        std::boxed::Box<Spanned<Self>>,
    ),
    Set(std::boxed::Box<Spanned<Self>>),
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
            Self::Apply { path, args } => {
                let path = path
                    .iter()
                    .map(|(segment, _)| *segment)
                    .collect::<Vec<_>>()
                    .join(".");
                let args = args
                    .iter()
                    .map(|(arg, _)| arg.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{path}<{args}>")
            }
            Self::Sum(members) => {
                let joined = members
                    .iter()
                    .map(|(member, _)| member.to_string())
                    .collect::<Vec<_>>()
                    .join(" | ");
                f.write_str(&joined)
            }
            Self::Box(inner) => write!(f, "Box<{}>", inner.0),
            Self::View(inner) => write!(f, "View<{}>", inner.0),
            Self::Array(inner, length) => write!(f, "Array<{}, {}>", inner.0, length),
            Self::List(inner) => write!(f, "List<{}>", inner.0),
            Self::Map(key, value) => write!(f, "Map<{}, {}>", key.0, value.0),
            Self::Set(inner) => write!(f, "Set<{}>", inner.0),
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
    F64(f64),
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
            Self::F64(x) => write!(f, "{x}"),
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
pub enum Value {
    Nil,
    Bool(bool),
    Num(NumLiteral),
    /// An interpreted UTF-8 string after escape decoding. The source token is
    /// borrowed, but the checked literal owns its normalized bytes.
    Str(String),
    Unicode(u32),
}

/// One ordered part of an interpreted string. Literal text is decoded by the
/// lexer; embedded expressions are parsed from the same token stream as every
/// other expression and never reinterpreted during lowering.
#[derive(Debug)]
pub enum StringPart<'src> {
    Literal(String),
    Expression(Spanned<Expr<'src>>),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Nil => write!(f, "nil"),
            Self::Bool(x) => write!(f, "{x}"),
            Self::Num(x) => write!(f, "{x}"),
            Self::Str(x) => write!(f, "{x}"),
            Self::Unicode(x) => write!(f, "U+{x:04X}"),
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
    And,
    Or,
}

#[derive(Clone, Copy, Debug)]
pub enum UnaryOp {
    Not,
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
    Value(Value),
    Interpolated(Vec<StringPart<'src>>),
    List(Vec<Spanned<Self>>),
    MapNew(Spanned<TypeRef<'src>>, Spanned<TypeRef<'src>>),
    SetNew(Spanned<TypeRef<'src>>),
    Local(&'src str),
    /// The implicit method receiver. Rejected outside a method body.
    SelfRef,
    /// A built-in type name in expression position. Only valid as a call head,
    /// where it unwraps one represented-type layer (Specification 010 7.2).
    BuiltinType(TypeName),
    /// `base . name`. Resolution decides whether this is a field, a qualified
    /// type path, or a method that was used without a call.
    Member(Box<Spanned<Self>>, Spanned<&'src str>),
    Index(Box<Spanned<Self>>, Box<Spanned<Self>>),
    Binary(Box<Spanned<Self>>, BinaryOp, Box<Spanned<Self>>),
    Unary(UnaryOp, Box<Spanned<Self>>),
    /// `return_on_error expression` propagates an active Error alternative
    /// through the enclosing fallible callable.
    ReturnOnError(Box<Spanned<Self>>),
    Call(Box<Spanned<Self>>, Spanned<Vec<Arg<'src>>>),
    /// An explicitly monomorphized call such as `identity<Int64>(42)`.
    GenericCall(
        Box<Spanned<Self>>,
        Spanned<Vec<Spanned<TypeRef<'src>>>>,
        Spanned<Vec<Arg<'src>>>,
    ),
    Print(Box<Spanned<Self>>),
    /// `box(expression)` (Specification 016 section 4.2): a reserved
    /// allocation expression, not a call. Its operand is evaluated exactly
    /// once and its checked result is `Box<T>`, where `T` is the operand's
    /// checked type.
    Box(Box<Spanned<Self>>),
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
    /// An indexed assignment target. It is kept as an expression until type
    /// checking because only map indexing is writable in the first collection
    /// slice; ordinary field/root assignments retain `Assign`'s resolved path.
    IndexedAssign {
        target: Spanned<Expr<'src>>,
        value: Spanned<Expr<'src>>,
    },
    While {
        condition: Spanned<Expr<'src>>,
        body: Block<'src>,
        span: Span,
    },
    For {
        value: Spanned<&'src str>,
        key: Option<Spanned<&'src str>>,
        iterable: Spanned<Expr<'src>>,
        body: Block<'src>,
        span: Span,
    },
    Break(Span),
    /// `"return", [ expression ]` (Specification 026 section 4). `None` is a
    /// bare return; the parser admits it only immediately before a block
    /// boundary (`end`, `elseif`, `else`, or top-level end of input), so a
    /// bare return can never be followed by another element of the same
    /// block (section 4 rule 3).
    Return(Option<Spanned<Expr<'src>>>, Span),
    /// A deferred direct call, executed when the containing lexical block
    /// exits. The checker validates the call shape and result type.
    Defer {
        on_error: bool,
        call: Spanned<Expr<'src>>,
        span: Span,
    },
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
    /// Explicit type parameters declared between the function name and `(`.
    pub generic_params: Vec<Spanned<&'src str>>,
    pub args: Vec<Param<'src>>,
    /// `None` declares a function without a result.
    pub ret: Option<Spanned<TypeRef<'src>>>,
    pub span: Span,
    pub body: Block<'src>,
}

/// `static Type.name(...) do ... end`. Associated functions have no implicit
/// receiver; the receiver type only provides their namespace.
#[derive(Debug)]
pub struct StaticDecl<'src> {
    pub receiver: Spanned<TypeRef<'src>>,
    pub name: Spanned<&'src str>,
    pub args: Vec<Param<'src>>,
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

/// How one parameter is passed. Specification 011 section 19 phase 1 step 2
/// represents `Ref<T>` as a passing mode beside an ordinary value type rather
/// than as a type of its own, so no reference type is ever storable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamMode {
    Value,
    Reference,
}

#[derive(Debug)]
pub struct Param<'src> {
    pub name: &'src str,
    pub mode: ParamMode,
    /// The written value type. For [`ParamMode::Reference`] this is the
    /// referent type `T` of `Ref<T>`, never the reference itself.
    pub ty: Spanned<TypeRef<'src>>,
    pub span: Span,
}

/// `type N is ...`. Declaration order is preserved so `TypeId` allocation is
/// deterministic (Specification 010 section 19 phase 2).
#[derive(Clone, Debug)]
pub struct TypeDecl<'src> {
    pub name: &'src str,
    pub name_span: Span,
    pub generic_params: Vec<Spanned<&'src str>>,
    pub body: TypeBody<'src>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeBody<'src> {
    Represented(Spanned<TypeRef<'src>>),
    Struct(Vec<FieldDecl<'src>>),
    Union(Vec<UnionMemberDecl<'src>>),
}

#[derive(Clone, Debug)]
pub struct FieldDecl<'src> {
    pub name: &'src str,
    pub name_span: Span,
    pub ty: Spanned<TypeRef<'src>>,
}

/// One union alternative. A bare alternative is exactly an empty inline struct
/// (Specification 010 section 6.2), so both spell out to the same shape here.
#[derive(Clone, Debug)]
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
    /// Static associated-function declarations in source order.
    pub statics: Vec<StaticDecl<'src>>,
    /// The top-level executable block. An empty program is a block with zero
    /// elements, so this is never optional.
    pub body: Block<'src>,
}
