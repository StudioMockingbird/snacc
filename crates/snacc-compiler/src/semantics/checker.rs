use crate::semantics::types::{
    self, BoxId, CollectionDef, CollectionId, FuncSig, MethodId, MethodSig, SumId, TypeDef, TypeId,
    Types,
};
use crate::syntax::ast::{
    Arg, BinaryOp, Block, BlockElement, Condition, Expr, Func, IfForm, NumLiteral, Param,
    ParamMode, PlacePath, PlaceRootName, Program as AstProgram, Span, Spanned, TypeBody, TypeDecl,
    TypeName, TypeRef, TypeTest, UnaryOp, Value,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Every checked type. User-defined types have exactly one variant here --
/// their category (represented, struct, union, union member) lives in the type
/// table, never in this enum (Specification 010 section 19 phase 3).
/// `Sum` is an inline sum's normalized member set (Specification 018 section
/// 4); like `User`, its members live in the type table (`Types::sum_members`),
/// never here. `Ord` gives every sum's member set one canonical sorted order,
/// so `Byte | Nil` and `Nil | Byte` intern to the same id regardless of
/// source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ty {
    Float64,
    Int64,
    Bool,
    Nil,
    String,
    Unicode,
    ViewByte,
    ViewUnicode,
    Array(CollectionId),
    List(CollectionId),
    View(CollectionId),
    Map(CollectionId),
    Set(CollectionId),
    Byte,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    User(TypeId),
    Sum(SumId),
    /// `Box<T>` (Specification 016 section 4.1). The pointee lives in
    /// `Types`' box table, indexed by `BoxId`, mirroring how a `Sum`'s
    /// members live in the type table rather than here.
    Box(BoxId),
}

impl From<TypeName> for Ty {
    fn from(value: TypeName) -> Self {
        match value {
            TypeName::Float64 => Self::Float64,
            TypeName::Int64 => Self::Int64,
            TypeName::Bool => Self::Bool,
            TypeName::Nil => Self::Nil,
            TypeName::String => Self::String,
            TypeName::Unicode => Self::Unicode,
            TypeName::Byte => Self::Byte,
            TypeName::UInt16 => Self::UInt16,
            TypeName::UInt32 => Self::UInt32,
            TypeName::UInt64 => Self::UInt64,
            TypeName::Float32 => Self::Float32,
        }
    }
}

impl std::fmt::Display for Ty {
    /// A user-defined type has no name without its table, so every diagnostic
    /// renders types through [`Types::display`] instead. The placeholder below
    /// exists only so `Ty` stays printable for debugging.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Float64 => write!(f, "Float64"),
            Self::Int64 => write!(f, "Int64"),
            Self::Bool => write!(f, "Bool"),
            Self::Nil => write!(f, "Nil"),
            Self::String => write!(f, "String"),
            Self::Unicode => write!(f, "Unicode"),
            Self::ViewByte => write!(f, "View<Byte>"),
            Self::ViewUnicode => write!(f, "View<Unicode>"),
            Self::Array(_) => write!(f, "Array<...>"),
            Self::List(_) => write!(f, "List<...>"),
            Self::View(_) => write!(f, "View<...>"),
            Self::Map(_) => write!(f, "Map<...>"),
            Self::Set(_) => write!(f, "Set<...>"),
            Self::Byte => write!(f, "Byte"),
            Self::UInt16 => write!(f, "UInt16"),
            Self::UInt32 => write!(f, "UInt32"),
            Self::UInt64 => write!(f, "UInt64"),
            Self::Float32 => write!(f, "Float32"),
            Self::User(id) => write!(f, "<user type #{}>", id.0),
            Self::Sum(id) => write!(f, "<sum #{}>", id.0),
            Self::Box(id) => write!(f, "<box #{}>", id.0),
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

#[derive(Clone, Copy)]
pub enum LogicalOp {
    And,
    Or,
}

/// The root of a checked place. Local names are unique for a whole function or
/// method (Specification 012 section 5.2), so a name is the binding identity;
/// no separate ID table is needed to tell two roots apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlaceRoot {
    Local(String),
    /// The implicit method receiver.
    SelfRef,
}

impl std::fmt::Display for PlaceRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Local(name) => f.write_str(name),
            Self::SelfRef => f.write_str("self"),
        }
    }
}

/// A resolved place: a root plus field selectors. Assignment, type tests, and
/// receiver places all share this shape, and Specification 011's reference
/// arguments reuse it unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub root: PlaceRoot,
    pub root_ty: Ty,
    /// Field indices in selection order; empty selects the root itself.
    pub path: Vec<usize>,
    /// The type reached after applying `path`.
    pub ty: Ty,
}

/// Specification 016 section 12 (phase 3 step 1): the explicit mode a checked
/// place use occupies. Borrowing and mutation already have their own distinct
/// checked shapes -- a reference argument is [`TArg::Reference`], an
/// assignment target is `TStmt::Assign`'s `place`, and a receiver place is
/// [`TReceiver::Place`] -- so only the copy/consume distinction is ambiguous
/// enough to need a tag on [`TExpr::Place`] itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseMode {
    /// An ordinary read: an operand, a receiver, a print argument, or any
    /// position other than the five consuming contexts below.
    Copy,
    /// Specification 016 section 6.1: initialization, assignment's right
    /// operand, a by-value argument, a function/method result, or an
    /// aggregate constructor argument. A move-only place used this way
    /// transfers its complete value and cannot be used again (section 6.2);
    /// a copyable place used this way remains an ordinary copy.
    Consume,
}

/// One resolved parameter. The passing mode travels with the value type, so no
/// later phase re-derives it from source syntax (Specification 011 section 11).
/// A `Ref<T>` parameter's `ty` is the referent type `T`.
#[derive(Clone, Debug)]
pub struct TParam {
    pub name: String,
    pub ty: Ty,
    pub mode: ParamMode,
}

/// One checked call argument. Specification 011 section 19 phase 2 step 5 keeps
/// the two kinds apart so lowering can never copy a reference argument.
pub enum TArg {
    Value(TExpr),
    /// The referent place, resolved once. Lowering passes its address.
    Reference(Place),
}

/// How a method call reaches its receiver. A call that may write through `self`
/// requires the `Place` form; a read-only call may use a temporary, which the
/// backend gives compiler-owned storage (Specification 010 section 15.3).
pub enum TReceiver {
    Place(Place),
    /// A receiver with no addressable place of its own (a temporary, such as
    /// a fresh `box(...)` or a call result). `Ty` is the value's own checked
    /// static type, un-dereferenced -- exactly what `Place::ty` already
    /// carries for the `Place` variant, so lowering (Specification 016
    /// section 4.3) can peel the same number of `Box<T>` layers regardless
    /// of which variant a call reached, even though a box's own LLVM value
    /// never reveals its pointee type on its own.
    Value(TExpr, Ty),
}

pub struct TMethodCall {
    pub receiver: TReceiver,
    pub method: MethodId,
    pub args: Vec<TArg>,
}

pub struct TStringPart {
    pub value: TExpr,
    pub ty: Ty,
}

/// Value-producing checked nodes. Nothing here may stand for a construct
/// without a result: no sentinel type, dummy value, or fallback expression.
pub enum TExpr {
    Num(NumLiteral),
    Bool(bool),
    Nil,
    StringLiteral(String),
    Unicode(u32),
    StringClone(Box<TExpr>),
    /// One maximal concatenation/interpolation plan. Parts remain in source
    /// evaluation order and lower through one final allocation.
    StringConcat(Vec<TStringPart>),
    StringFromUnicode(Box<TExpr>),
    StringFromUtf8(Box<TExpr>, SumId),
    ViewFromString(Box<TExpr>, Ty),
    ViewLength(Box<TExpr>, Ty),
    /// A checked, bounds-safe element lookup. The runtime returns a signed
    /// sentinel so lowering can construct the specified `T | Nil` sum without
    /// exposing a private result ABI to the runtime.
    ViewAt(Box<TExpr>, Box<TExpr>, Ty, SumId),
    ViewSlice {
        value: Box<TExpr>,
        start: Box<TExpr>,
        end: Box<TExpr>,
        view_ty: Ty,
        sum: SumId,
    },
    CollectionLiteral {
        ty: Ty,
        items: Vec<TExpr>,
    },
    CollectionNew(Ty),
    CollectionLength(Box<TExpr>),
    CollectionIsEmpty(Box<TExpr>),
    CollectionCapacity(Box<TExpr>),
    CollectionView(Box<TExpr>, Ty),
    CollectionSlice {
        value: Box<TExpr>,
        start: Box<TExpr>,
        end: Box<TExpr>,
        view_ty: Ty,
        sum: SumId,
        elem: Ty,
    },
    CollectionIndex {
        collection: Box<TExpr>,
        index: Box<TExpr>,
        collection_ty: Ty,
        elem: Ty,
    },
    /// Removes and returns the last scalar element of a mutable list.
    ListPop {
        receiver: Place,
        elem: Ty,
    },
    /// Removes and returns one scalar element from a mutable list.
    ListRemove {
        receiver: Place,
        index: Box<TExpr>,
        elem: Ty,
    },
    MapContains {
        receiver: Box<TExpr>,
        key: Box<TExpr>,
        key_ty: Ty,
        value_ty: Ty,
    },
    MapInsert {
        receiver: Place,
        key: Box<TExpr>,
        value: Box<TExpr>,
        key_ty: Ty,
        value_ty: Ty,
        require_existing: bool,
    },
    MapDelete {
        receiver: Place,
        key: Box<TExpr>,
        key_ty: Ty,
        value_ty: Ty,
    },
    MapIndex {
        receiver: Box<TExpr>,
        key: Box<TExpr>,
        key_ty: Ty,
        value_ty: Ty,
    },
    MapTake {
        receiver: Place,
        key: Box<TExpr>,
        key_ty: Ty,
        value_ty: Ty,
    },
    SetContains {
        receiver: Box<TExpr>,
        value: Box<TExpr>,
        elem: Ty,
    },
    SetInsert {
        receiver: Place,
        value: Box<TExpr>,
        elem: Ty,
    },
    SetDelete {
        receiver: Place,
        value: Box<TExpr>,
        elem: Ty,
    },
    /// Reads a place. Specification 016 section 12 (phase 3 step 1): the
    /// [`UseMode`] records whether this occurrence sits in a consuming
    /// context, set by `mark_consumed` after `check_expr` produces this node
    /// and before any coercion wraps it.
    Place(Place, UseMode),
    /// Reads one field of a value that has no addressable storage. `base_ty`
    /// is `base`'s own checked type, not yet automatically dereferenced --
    /// unlike `Print`'s trailing `Ty`, `base_ty` here exists so lowering
    /// (Specification 016 section 4.3) knows how many `Box<T>` layers, if
    /// any, to peel before reading `base`'s field, since a fresh `box(...)`
    /// value (not a place) can be the base of a field chain and the backend
    /// cannot recover that from `base`'s own lowered LLVM value (every box,
    /// regardless of pointee, lowers to the same opaque pointer type).
    FieldRead {
        base: Box<TExpr>,
        base_ty: Ty,
        index: usize,
        ty: Ty,
    },
    /// Builds a struct or union-member value. Entries appear in written
    /// evaluation order; each `usize` is the destination field's declaration
    /// index, so lowering evaluates left to right and stores in field order.
    Construct {
        type_id: TypeId,
        fields: Vec<(usize, TExpr)>,
    },
    /// Adds or removes exactly one represented-type layer. Identity at runtime;
    /// the node exists so the nominal change is explicit before lowering.
    Represent {
        value: Box<TExpr>,
        ty: Ty,
    },
    /// Injects a direct member value into its containing union.
    Inject {
        member: TypeId,
        into_union: TypeId,
        value: Box<TExpr>,
    },
    /// Injects a direct member value into an inline sum (Specification 018
    /// section 5). Unlike [`Self::Inject`], `member` names the exact member
    /// type directly (a scalar, `Ty::User`, or nothing else, never another
    /// sum) rather than a `TypeId`, since a sum member need not be a
    /// user-defined type at all. Deterministic tag assignment is lowering's
    /// job, not the checker's, so no tag is recorded here.
    InjectSum {
        sum: SumId,
        member: Ty,
        value: Box<TExpr>,
    },
    /// Lifts a reduced inline sum into a larger inline sum when every source
    /// member is already a direct target member (used by successful
    /// `return_on_error` results).
    LiftSum {
        value: Box<TExpr>,
        from: SumId,
        to: SumId,
    },
    Arith(Box<TExpr>, ArithOp, Box<TExpr>, Ty),
    Cmp(Box<TExpr>, CmpOp, Box<TExpr>, Ty),
    Not(Box<TExpr>),
    Logical(Box<TExpr>, LogicalOp, Box<TExpr>),
    /// Converts a condition value to the control-flow predicate defined by
    /// Specification 021. The original type is retained because truthiness
    /// is not an ordinary Boolean conversion.
    Truthiness(Box<TExpr>, Ty),
    /// Fallible-call propagation with the operand's one non-Error success
    /// member. The enclosing result type is retained for lowering.
    ReturnOnError {
        value: Box<TExpr>,
        sum: SumId,
        success: Ty,
        result: Ty,
        cleanup: Vec<TCleanup>,
    },
    /// A call to a declaration that has a result.
    Call(String, Vec<TArg>),
    /// A method call that has a result.
    MethodCall(Box<TMethodCall>),
    /// An `if` classified as value-form: every path produces a value, either
    /// through an `else` or through a proven-exhaustive type-test chain.
    If(Box<TValueIf>),
    Print(Box<TExpr>, Ty),
    Cast(Box<TExpr>, Ty),
    /// `box(expression)` (Specification 016 section 4.2). `ty` is the whole
    /// allocation's result type `Box<T>`, not the operand's type `T` --
    /// unlike `Print`'s trailing `Ty`, which names the operand. RFC 016 Task
    /// B/C gives this a lowering strategy; the checker only produces it.
    Box(Box<TExpr>, Ty),
}

/// A proven type test against a named union. `tag` is the member's
/// deterministic union tag, so lowering compares stored tags without
/// consulting the type table.
pub struct TTypeTest {
    pub place: Place,
    pub member: TypeId,
    pub tag: u32,
    /// The name and exact member type bound on the successful edge.
    pub binding: Option<(String, Ty)>,
}

/// A proven type test against one direct member of an inline sum
/// (Specification 018 section 6). Kept separate from [`TTypeTest`] rather
/// than folded into it because a sum member need not be a `TypeId` at all
/// (it may be a bare scalar), and because deterministic tag assignment is
/// lowering's job (Task B), not the checker's, so no tag is recorded here.
pub struct TSumTypeTest {
    pub place: Place,
    pub sum: SumId,
    pub member: Ty,
    /// The name and exact member type bound on the successful edge. `Nil`
    /// carries no value and is never bound.
    pub binding: Option<(String, Ty)>,
}

/// An `if`/`elseif` condition: an ordinary `Bool` value or a type test
/// against a named union or an inline sum.
pub enum TCondition {
    Expr(TExpr),
    Test(TTypeTest),
    SumTest(TSumTypeTest),
}

pub struct TValueIf {
    /// First arm is the `if`; remaining arms are `elseif`s, in source order.
    pub arms: Vec<(TCondition, TBlock)>,
    /// `None` only when `exhaustive` proved every path is covered.
    pub else_branch: Option<TBlock>,
    /// Every arm tests the same place and every direct member of its union
    /// appears exactly once, so the fall-through edge is unreachable.
    pub exhaustive: bool,
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
        place: Place,
        value: TExpr,
        /// Specification 016 section 6.3: an assignment to a move-only place
        /// that currently holds a live value must destroy that old value
        /// before the new one is installed. False for a copyable
        /// destination (nothing to destroy) and for a whole-root
        /// destination that is currently moved (reinitializing a moved
        /// mutable local, section 6.3's closing sentence, installs a value
        /// where none was live).
        drop_before: bool,
    },
    /// Replaces one checked array/list element after validating the runtime
    /// index. The receiver remains an owning place, so a view can never reach
    /// this statement.
    SequenceIndexAssign {
        receiver: Place,
        index: TExpr,
        value: TExpr,
        elem: Ty,
    },
    While {
        condition: TExpr,
        body: TBlock,
    },
    For {
        value_name: String,
        value_ty: Ty,
        key_name: Option<String>,
        key_ty: Option<Ty>,
        iterable: TExpr,
        collection_ty: Ty,
        body: TBlock,
    },
    Break {
        cleanup: Vec<TCleanup>,
    },
    /// `return` (Specification 026 section 10). `value` is `Some` only for a
    /// value return from a result-declaring callable, already checked and
    /// assignable to its declared result -- never a sentinel for a bare
    /// return. `cleanup` is the ordered, exit-sensitive plan computed at this
    /// exact point and run after the result is materialized (section 8).
    Return {
        value: Option<TExpr>,
        result: Option<Ty>,
        cleanup: Vec<TCleanup>,
    },
    /// Statement-form `return_on_error` for an operand whose exact type is
    /// `Nil | Error`. Nil continues the current block; Error exits through
    /// the enclosing fallible callable.
    ReturnOnError {
        value: TExpr,
        sum: SumId,
        result: Ty,
        cleanup: Vec<TCleanup>,
    },
    /// An `if` classified as statement-form: `else` is optional and every
    /// branch is a no-result block.
    If(TStmtIf),
    /// A call to a declaration without a result.
    Call(String, Vec<TArg>),
    /// A call to a method without a result.
    MethodCall(TMethodCall),
    /// A built-in collection mutation lowered directly through the runtime.
    ListPush {
        receiver: Place,
        value: TExpr,
        elem: Ty,
    },
    ListClear {
        receiver: Place,
        elem: Ty,
    },
    ListInsert {
        receiver: Place,
        index: TExpr,
        value: TExpr,
        elem: Ty,
    },
    ListReserve {
        receiver: Place,
        minimum: TExpr,
        elem: Ty,
    },
    MapClear {
        receiver: Place,
        key_ty: Ty,
        value_ty: Ty,
    },
    MapReserve {
        receiver: Place,
        minimum: TExpr,
        key_ty: Ty,
        value_ty: Ty,
    },
    SetClear {
        receiver: Place,
        elem: Ty,
    },
    SetReserve {
        receiver: Place,
        minimum: TExpr,
        elem: Ty,
    },
    /// A value-producing expression whose result is discarded.
    Expr(TExpr),
}

pub struct TStmtIf {
    pub arms: Vec<(TCondition, TBlock)>,
    pub else_branch: Option<TBlock>,
    /// Same meaning as [`TValueIf::exhaustive`].
    pub exhaustive: bool,
}

/// A block's ordered checked elements plus its optional resulting value.
/// `result` is `Some` only for a value-required block whose final element
/// supplied a value.
pub struct TBlock {
    pub statements: Vec<TStmt>,
    pub result: Option<TExpr>,
    /// A single reverse-registration cleanup plan for local destruction and
    /// deferred calls (Specification 025).
    pub cleanup: Vec<TCleanup>,
}

pub enum TCleanup {
    Drop(Place),
    Deferred(Rc<TDeferred>),
}

pub struct TDeferred {
    pub on_error: bool,
    pub call: TStmt,
    /// Whole roots consumed by the deferred call at exit. The backend uses
    /// these facts to avoid dropping a value after the call transfers it.
    pub consumes: Vec<PlaceRoot>,
}

pub struct TFunc {
    pub params: Vec<TParam>,
    /// `None` is a function without a result; it lowers to LLVM `void`.
    pub result: Option<Ty>,
    pub body: TBlock,
    /// Specification 016 section 8.1: by-value parameters still available
    /// (not moved into a result or elsewhere) when the body finishes
    /// normally, in reverse parameter order, destroyed just before the
    /// function returns -- after the body's own cleanup plan, since
    /// parameters were bound before any of them.
    pub param_drops: Vec<Place>,
}

pub struct TMethod {
    pub receiver: TypeId,
    pub name: String,
    pub params: Vec<TParam>,
    pub result: Option<Ty>,
    /// The least-fixed-point receiver-write effect. Internal only: it creates
    /// no source-level method category and is not part of the signature.
    pub writes_receiver: bool,
    pub body: TBlock,
    /// Same meaning as [`TFunc::param_drops`]. `self` is never included: a
    /// receiver is always borrowed (Specification 010 section 15.3), never
    /// owned by the method body.
    pub param_drops: Vec<Place>,
}

pub struct TExtern {
    pub symbol: String,
    pub params: Vec<TParam>,
    /// `None` is a bridge without a result; its C ABI result is `void`.
    pub result: Option<Ty>,
    pub span: std::ops::Range<usize>,
}

pub struct Program {
    pub funcs: HashMap<String, TFunc>,
    pub externs: HashMap<String, TExtern>,
    /// Every resolved user-defined type, indexed by `TypeId`.
    pub types: Vec<TypeDef>,
    /// Every checked method, indexed by `MethodId`.
    pub methods: Vec<TMethod>,
    /// Every interned inline sum's normalized member list, indexed by
    /// `SumId` (Specification 018 section 4). Lowering assigns each member's
    /// deterministic tag from its position here, the same way a named
    /// union's tag is its member's declaration position.
    pub sums: Vec<Vec<Ty>>,
    /// Every interned `Box<T>`'s pointee type, indexed by `BoxId`
    /// (Specification 016 section 4.1). RFC 016 Task B/C's lowering strategy
    /// is the first consumer; Task A only records it here, mirroring `sums`.
    pub boxes: Vec<Ty>,
    pub collections: Vec<CollectionDef>,
    pub body: TBlock,
}

#[derive(Clone, Copy)]
struct Binding<'src> {
    name: &'src str,
    ty: Ty,
    mutable: bool,
    /// Specification 016 section 7.3: set only for a union- or sum-test
    /// binding. Such a binding is never an independent owning root -- it is
    /// always a branch-scoped alias to its tested place's active payload --
    /// so `mark_consumed` treats a whole-binding consuming use as a subplace
    /// move (Specification 016 section 6.4) instead of a legitimate whole-
    /// root move, even though its own path is empty.
    type_test_alias: bool,
}

/// One method call awaiting the receiver-write fixed point. Validation cannot
/// run while bodies are checked because a callee's effect may not be known yet.
struct ReceiverCall {
    method: MethodId,
    mutable_root: bool,
    receiver: String,
    span: Span,
}

/// A local immutable view borrow tracked to its source root. The scope index
/// limits last-use pruning to borrows whose view binding is alive in the block
/// being analyzed.
#[derive(Clone)]
struct ViewBorrow {
    view_name: String,
    sources: Vec<PlaceRoot>,
    scope: usize,
}

struct Ctx<'src> {
    sigs: HashMap<String, FuncSig>,
    externs: HashSet<String>,
    types: Types,
    method_sigs: Vec<MethodSig>,
    method_index: HashMap<(TypeId, String), MethodId>,
    /// Every name bound anywhere in the function or method being checked.
    /// Specification 012 section 5.2 makes this uniqueness rule function-wide,
    /// so nested blocks and sibling branches share one set.
    declared: Vec<&'src str>,
    /// One entry per enclosing `while` body. `break` needs a non-empty stack.
    loops: Vec<usize>,
    /// The receiver type while a method body is checked.
    self_ty: Option<Ty>,
    /// Specification 026 section 5: the enclosing callable's declared result
    /// while its body is checked, distinct from "no result" -- `None` means
    /// no callable currently encloses the point being checked (top level),
    /// `Some(None)` a no-result function or method, `Some(Some(ty))` one that
    /// declares `: ty`. `return`'s permitted form is checked against this
    /// fact alone, never the syntactic kind of the immediately enclosing
    /// block.
    callable_result: Option<Option<Ty>>,
    current_method: Option<MethodId>,
    direct_writes: Vec<bool>,
    effect_edges: Vec<(MethodId, MethodId)>,
    receiver_calls: Vec<ReceiverCall>,
    /// Specification 016 section 6.2: every whole move-only root that is
    /// currently moved, for the function or method body being checked, keyed
    /// by root and recording the consuming operation's span. A root absent
    /// here is available -- true for every non-move-only root, so this stays
    /// empty for a program that never uses `Box<T>` -- so no entry is made
    /// until something actually moves. Reset per function/method by
    /// `begin_region`, snapshotted and restored across sibling `if`/`elseif`/
    /// `else` arms, and merged back by unioning every reachable arm's exit
    /// (available only when available on every one, Specification 016
    /// section 6.2).
    move_state: HashMap<PlaceRoot, Span>,
    view_borrows: Vec<ViewBorrow>,
    /// Ordered cleanup entries for every currently open lexical block.
    cleanup_scopes: Vec<Vec<TCleanup>>,
    errors: Vec<Error>,
    unknown: Option<&'static str>,
    generic_funcs: HashMap<&'src str, &'src Func<'src>>,
    generic_types: HashMap<&'src str, &'src TypeDecl<'src>>,
    generic_type_finished: HashSet<String>,
    generic_type_in_progress: HashSet<String>,
    generic_type_stack: Vec<String>,
    generic_subst: HashMap<&'src str, Ty>,
    generic_queue: Vec<GenericRequest>,
    generic_seen: HashSet<String>,
    specialization_count: usize,
    generic_depth: usize,
    generic_chain: Vec<String>,
}

struct GenericRequest {
    name: String,
    args: Vec<Ty>,
    depth: usize,
    use_span: Span,
    chain: Vec<String>,
}

const MAX_SPECIALIZATION_DEPTH: usize = 128;
const MAX_SPECIALIZATIONS: usize = 4096;

impl<'src> Ctx<'src> {
    fn error(&mut self, span: Span, msg: String) {
        self.errors.push(Error { span, msg });
    }

    /// The qualified name of a type, for diagnostics.
    fn name(&self, ty: Ty) -> String {
        self.types.display(ty)
    }

    fn mismatch(&mut self, span: Span, expected: Ty, found: Ty) {
        let msg = format!(
            "expected '{}', found '{}'",
            self.name(expected),
            self.name(found)
        );
        self.error(span, msg);
    }

    /// Specification 009 section 6: a rejected operand pair names both types
    /// and the exact-match requirement.
    fn operands(&mut self, span: Span, what: &str, left: Ty, right: Ty) {
        let msg = format!(
            "{what} operands must be two numbers of the same type, found '{}' and '{}'",
            self.name(left),
            self.name(right)
        );
        self.error(span, msg);
    }

    fn method_name(&self, method: MethodId) -> String {
        self.method_sigs[method.index()].qualified(&self.types)
    }

    /// Renders a resolved place the way it was written, so an overlap or
    /// mutability diagnostic can name both argument places (Specification 011
    /// section 13).
    fn place_name(&self, place: &Place) -> String {
        let mut text = place.root.to_string();
        let mut current = place.root_ty;
        for index in &place.path {
            // Specification 016 section 4.3: a path may cross a box exactly
            // where the place itself does, so rendering it back needs the
            // same automatic dereference `walk_fields` used to build it.
            current = deref_box(&self.types, current);
            let Ty::User(id) = current else { break };
            let Some(fields) = self.types.def(id).fields() else {
                break;
            };
            let Some((name, ty)) = fields.get(*index) else {
                break;
            };
            text.push('.');
            text.push_str(name);
            current = *ty;
        }
        text
    }
}

type Env<'src> = Vec<Binding<'src>>;

pub fn check<'src>(program: &'src AstProgram<'src>) -> Result<Program, Failure> {
    let mut errors = Vec::new();
    let collected = types::collect(program, &mut errors);
    let method_count = collected.methods.len();
    let static_names = collected.static_names;
    let specialization_count = collected.specialization_count;
    let mut ctx = Ctx {
        sigs: collected.sigs,
        externs: program
            .externs
            .keys()
            .map(|name| (*name).to_string())
            .collect(),
        types: collected.types,
        method_sigs: collected.methods,
        method_index: collected.method_index,
        declared: Vec::new(),
        loops: Vec::new(),
        self_ty: None,
        callable_result: None,
        current_method: None,
        direct_writes: vec![false; method_count],
        effect_edges: Vec::new(),
        receiver_calls: Vec::new(),
        move_state: HashMap::new(),
        view_borrows: Vec::new(),
        cleanup_scopes: Vec::new(),
        errors,
        unknown: None,
        generic_funcs: program
            .funcs
            .iter()
            .filter(|(_, function)| !function.generic_params.is_empty())
            .map(|(name, function)| (*name, function))
            .collect(),
        generic_types: program
            .types
            .iter()
            .filter(|declaration| !declaration.generic_params.is_empty())
            .map(|declaration| (declaration.name, declaration))
            .collect(),
        generic_type_finished: HashSet::new(),
        generic_type_in_progress: HashSet::new(),
        generic_type_stack: Vec::new(),
        generic_subst: HashMap::new(),
        generic_queue: Vec::new(),
        generic_seen: HashSet::new(),
        specialization_count,
        generic_depth: 0,
        generic_chain: Vec::new(),
    };

    let generic_functions: Vec<&Func<'src>> = ctx.generic_funcs.values().copied().collect();
    let mut generic_errors = Vec::new();
    for function in generic_functions {
        validate_generic_function(function, &ctx, &mut generic_errors);
    }
    ctx.errors.extend(generic_errors);

    for function in program.externs.values() {
        check_duplicate_params(&mut ctx, &function.args);
        if !function.symbol.starts_with("snacc_user_") {
            ctx.error(
                function.span,
                "Rust bridge symbols must start with 'snacc_user_'".into(),
            );
        } else if !is_rust_identifier(function.symbol) {
            ctx.error(
                function.span,
                "Rust bridge symbols must be valid Rust identifiers".into(),
            );
        }
    }

    let mut names: Vec<&str> = program.funcs.keys().copied().collect();
    names.sort_unstable();
    let mut typed_funcs = HashMap::new();
    for name in names {
        let function = &program.funcs[name];
        if !function.generic_params.is_empty() {
            continue;
        }
        let signature = ctx.sigs[name].clone();
        if let Some(result) = signature.result
            && is_borrowed_type(&ctx, result)
        {
            ctx.error(
                function
                    .ret
                    .as_ref()
                    .map_or(function.span, |(_, span)| *span),
                format!(
                    "'{}' is borrowed and cannot be returned from a function",
                    ctx.name(result)
                ),
            );
        }
        let mut env = Env::new();
        let params = begin_region(&mut ctx, &mut env, &function.args, &signature.params, None);
        let result = signature.result;
        ctx.callable_result = Some(result);
        let (body, _) = check_block(&mut ctx, &mut env, &function.body, result);
        ctx.callable_result = None;
        // Specification 016 section 8.1: computed after the body, from the
        // move state at the exact point it finished, exactly like a block's
        // own `drops` -- `env` still holds only the parameters here, since
        // `check_block` always restores `env` to its entry state before
        // returning. Unused by lowering when the body always returns
        // (Specification 026 section 10), but harmless to compute regardless.
        let param_drops = compute_drops(&ctx, &env, 0);
        typed_funcs.insert(
            name.to_string(),
            TFunc {
                params,
                result,
                body,
                param_drops,
            },
        );
    }

    for (declaration, qualified) in program.statics.iter().zip(static_names) {
        let Some(qualified) = qualified else { continue };
        check_duplicate_params(&mut ctx, &declaration.args);
        let signature = ctx.sigs[&qualified].clone();
        if let Some(result) = signature.result
            && is_borrowed_type(&ctx, result)
        {
            ctx.error(
                declaration
                    .ret
                    .as_ref()
                    .map_or(declaration.span, |(_, span)| *span),
                format!(
                    "'{}' is borrowed and cannot be returned from an associated function",
                    ctx.name(result)
                ),
            );
        }
        let mut env = Env::new();
        let params = begin_region(
            &mut ctx,
            &mut env,
            &declaration.args,
            &signature.params,
            None,
        );
        ctx.callable_result = Some(signature.result);
        let (body, _) = check_block(&mut ctx, &mut env, &declaration.body, signature.result);
        ctx.callable_result = None;
        let param_drops = compute_drops(&ctx, &env, 0);
        typed_funcs.insert(
            qualified,
            TFunc {
                params,
                result: signature.result,
                body,
                param_drops,
            },
        );
    }

    // Methods are checked in declaration order so `MethodId` and the checked
    // vector agree, and so the effect analysis input is deterministic.
    let mut typed_methods = Vec::with_capacity(method_count);
    for index in 0..method_count {
        let receiver = ctx.method_sigs[index].receiver;
        let name = ctx.method_sigs[index].name.clone();
        let result = ctx.method_sigs[index].result;
        let declared_params = ctx.method_sigs[index].params.clone();
        let declaration = &program.methods[ctx.method_sigs[index].decl];
        if let Some(result) = result
            && is_borrowed_type(&ctx, result)
        {
            ctx.error(
                declaration
                    .ret
                    .as_ref()
                    .map_or(declaration.span, |(_, span)| *span),
                format!(
                    "'{}' is borrowed and cannot be returned from a method",
                    ctx.name(result)
                ),
            );
        }
        let mut env = Env::new();
        let self_ty = Ty::User(receiver);
        let params = begin_region(
            &mut ctx,
            &mut env,
            &declaration.args,
            &declared_params,
            Some(self_ty),
        );
        ctx.current_method = Some(MethodId(index as u32));
        ctx.callable_result = Some(result);
        let (body, _) = check_block(&mut ctx, &mut env, &declaration.body, result);
        ctx.callable_result = None;
        // Specification 016 section 8.1: same reasoning as a function's
        // `param_drops` above; `self` is never included since `env` never
        // held a binding for it (a receiver is reached through
        // `ctx.self_ty`/`PlaceRoot::SelfRef`, never through `env`).
        let param_drops = compute_drops(&ctx, &env, 0);
        ctx.current_method = None;
        ctx.self_ty = None;
        typed_methods.push(TMethod {
            receiver,
            name,
            params,
            result,
            writes_receiver: false,
            body,
            param_drops,
        });
    }

    let mut typed_externs = HashMap::new();
    let mut extern_names: Vec<&str> = program.externs.keys().copied().collect();
    extern_names.sort_unstable();
    for name in extern_names {
        let function = &program.externs[name];
        let signature = &ctx.sigs[name];
        let params = signature.params.clone();
        typed_externs.insert(
            name.to_string(),
            TExtern {
                symbol: function.symbol.to_string(),
                params,
                result: signature.result,
                span: function.span.into_range(),
            },
        );
    }

    // The top-level executable body is a no-result block with its own binding
    // namespace; Snacc creates no implicit global state.
    ctx.declared.clear();
    ctx.loops.clear();
    ctx.move_state.clear();
    ctx.view_borrows.clear();
    ctx.callable_result = None;
    let mut env = Env::new();
    let (body, _) = check_block(&mut ctx, &mut env, &program.body, None);

    // Generic bodies are checked only for concrete applications. Each request
    // can enqueue more requests, so this is a deterministic work queue rather
    // than recursive lowering of the source AST.
    let mut request_index = 0;
    while request_index < ctx.generic_queue.len() {
        let (request_name, request_args, request_depth, request_use_span, request_chain) = {
            let request = &ctx.generic_queue[request_index];
            (
                request.name.clone(),
                request.args.clone(),
                request.depth,
                request.use_span,
                request.chain.clone(),
            )
        };
        request_index += 1;
        let function = {
            let generic_funcs = &ctx.generic_funcs;
            let Some(function) = generic_funcs.get(request_name.as_str()).copied() else {
                continue;
            };
            function
        };
        let mangled = generic_name(&ctx, &request_name, &request_args);
        if typed_funcs.contains_key(&mangled) {
            continue;
        }
        let substitutions: HashMap<&str, Ty> = function
            .generic_params
            .iter()
            .zip(&request_args)
            .map(|((name, _), ty)| (*name, *ty))
            .collect();
        let error_start = ctx.errors.len();
        let previous = std::mem::replace(&mut ctx.generic_subst, substitutions.clone());
        let params = resolve_generic_params(&mut ctx, &function.args);
        let result = function.ret.as_ref().map(|ty| resolve_type(&mut ctx, ty));
        let signature = FuncSig {
            params: params.clone(),
            result,
        };
        ctx.generic_subst = previous;
        ctx.sigs.insert(mangled.clone(), signature);
        let previous_subst = std::mem::replace(&mut ctx.generic_subst, substitutions);
        let mut env = Env::new();
        let params = begin_region(&mut ctx, &mut env, &function.args, &params, None);
        ctx.callable_result = Some(result);
        ctx.generic_depth = request_depth;
        let previous_chain = std::mem::replace(&mut ctx.generic_chain, request_chain.clone());
        let (body, _) = check_block(&mut ctx, &mut env, &function.body, result);
        ctx.callable_result = None;
        ctx.generic_depth = 0;
        ctx.generic_chain = previous_chain;
        ctx.generic_subst = previous_subst;
        let concrete = request_args
            .iter()
            .map(|ty| ctx.name(*ty))
            .collect::<Vec<_>>()
            .join(", ");
        let chain = request_chain.join(" -> ");
        for error in &mut ctx.errors[error_start..] {
            error.msg = format!(
                "{} (while specializing {request_name}<{concrete}> declared at {}..{}; requested at {}..{}; instantiation chain: {chain})",
                error.msg,
                function.span.start,
                function.span.end,
                request_use_span.start,
                request_use_span.end,
            );
        }
        let param_drops = compute_drops(&ctx, &env, 0);
        typed_funcs.insert(
            mangled,
            TFunc {
                params,
                result,
                body,
                param_drops,
            },
        );
    }

    // Specification 010 section 19 phase 4: solve the effect to its least fixed
    // point, then validate every deferred receiver-writing call.
    let writes = solve_receiver_writes(&ctx.direct_writes, &ctx.effect_edges);
    for (index, method) in typed_methods.iter_mut().enumerate() {
        method.writes_receiver = writes[index];
    }
    let pending = std::mem::take(&mut ctx.receiver_calls);
    for call in pending {
        if writes[call.method.index()] && !call.mutable_root {
            let method = ctx.method_name(call.method);
            ctx.error(
                call.span,
                format!(
                    "'{method}' may assign through 'self', so its receiver requires a \
                     mutable root, but '{}' is not mutable",
                    call.receiver
                ),
            );
        }
    }

    if let Some(detail) = ctx.unknown {
        return Err(Failure::Unknown(detail));
    }
    if ctx.errors.is_empty() {
        // Read before `ctx.types.defs` moves out below: `all_sums`/
        // `all_boxes` borrow the whole `Types` value, which a partial move
        // would break.
        let sums = ctx.types.all_sums().to_vec();
        let boxes = ctx.types.all_boxes().to_vec();
        let collections = ctx.types.all_collections().to_vec();
        Ok(Program {
            funcs: typed_funcs,
            externs: typed_externs,
            types: ctx.types.defs,
            methods: typed_methods,
            sums,
            boxes,
            collections,
            body,
        })
    } else {
        Err(Failure::Source(ctx.errors))
    }
}

fn type_ref_is_param(ty: &TypeRef<'_>, params: &HashSet<&str>) -> bool {
    matches!(ty, TypeRef::Named(segments) if segments.len() == 1 && params.contains(segments[0].0))
}

fn type_ref_mentions_param(ty: &TypeRef<'_>, params: &HashSet<&str>) -> bool {
    match ty {
        TypeRef::Named(segments) => segments.len() == 1 && params.contains(segments[0].0),
        TypeRef::Apply { args, .. } | TypeRef::Sum(args) => args
            .iter()
            .any(|(argument, _)| type_ref_mentions_param(argument, params)),
        TypeRef::Box(inner)
        | TypeRef::View(inner)
        | TypeRef::Array(inner, _)
        | TypeRef::List(inner)
        | TypeRef::Set(inner) => type_ref_mentions_param(&inner.0, params),
        TypeRef::Map(key, value) => {
            type_ref_mentions_param(&key.0, params) || type_ref_mentions_param(&value.0, params)
        }
        TypeRef::Builtin(_) => false,
    }
}

fn generic_operation(errors: &mut Vec<Error>, span: Span, operation: &str) {
    errors.push(Error {
        span,
        msg: format!(
            "cannot use an unconstrained generic type parameter with {operation}; add a concrete type or a future capability bound"
        ),
    });
}

fn validate_generic_expr<'src>(
    expression: &Spanned<Expr<'src>>,
    generic_vars: &HashSet<&'src str>,
    opaque_vars: &HashSet<&'src str>,
    errors: &mut Vec<Error>,
) -> bool {
    match &expression.0 {
        Expr::Local(name) => generic_vars.contains(name),
        Expr::Binary(left, op, right) => {
            let uses = validate_generic_expr(left, generic_vars, opaque_vars, errors)
                || validate_generic_expr(right, generic_vars, opaque_vars, errors);
            if uses {
                let operation = match op {
                    crate::syntax::ast::BinaryOp::Add
                    | crate::syntax::ast::BinaryOp::Sub
                    | crate::syntax::ast::BinaryOp::Mul
                    | crate::syntax::ast::BinaryOp::Div => "arithmetic",
                    crate::syntax::ast::BinaryOp::Eq | crate::syntax::ast::BinaryOp::NotEq => {
                        "equality"
                    }
                    crate::syntax::ast::BinaryOp::Less
                    | crate::syntax::ast::BinaryOp::LessEq
                    | crate::syntax::ast::BinaryOp::Greater
                    | crate::syntax::ast::BinaryOp::GreaterEq => "comparison",
                    crate::syntax::ast::BinaryOp::And | crate::syntax::ast::BinaryOp::Or => {
                        "logical operations"
                    }
                };
                generic_operation(errors, expression.1, operation);
            }
            uses
        }
        Expr::Unary(_, value) => {
            let uses = validate_generic_expr(value, generic_vars, opaque_vars, errors);
            if uses {
                generic_operation(errors, expression.1, "unary operations");
            }
            uses
        }
        Expr::Print(value) => {
            let uses = validate_generic_expr(value, generic_vars, opaque_vars, errors);
            if uses {
                generic_operation(errors, expression.1, "printing");
            }
            uses
        }
        Expr::Member(base, _) => {
            let uses = validate_generic_expr(base, generic_vars, opaque_vars, errors);
            if matches!(&base.0, Expr::Local(name) if opaque_vars.contains(name)) {
                generic_operation(errors, expression.1, "field or method access");
            }
            uses
        }
        Expr::Index(base, index) => {
            let uses = validate_generic_expr(base, generic_vars, opaque_vars, errors)
                || validate_generic_expr(index, generic_vars, opaque_vars, errors);
            if matches!(&base.0, Expr::Local(name) if opaque_vars.contains(name)) {
                generic_operation(errors, expression.1, "indexing");
            }
            uses
        }
        Expr::Call(callee, args) => {
            validate_generic_expr(callee, generic_vars, opaque_vars, errors)
                || args
                    .0
                    .iter()
                    .any(|arg| validate_generic_expr(&arg.value, generic_vars, opaque_vars, errors))
        }
        Expr::GenericCall(callee, _, args) => {
            validate_generic_expr(callee, generic_vars, opaque_vars, errors)
                || args
                    .0
                    .iter()
                    .any(|arg| validate_generic_expr(&arg.value, generic_vars, opaque_vars, errors))
        }
        Expr::Interpolated(parts) => {
            let uses = parts.iter().any(|part| match part {
                crate::syntax::ast::StringPart::Literal(_) => false,
                crate::syntax::ast::StringPart::Expression(value) => {
                    validate_generic_expr(value, generic_vars, opaque_vars, errors)
                }
            });
            if uses {
                generic_operation(errors, expression.1, "string interpolation");
            }
            uses
        }
        Expr::List(items) => items
            .iter()
            .any(|item| validate_generic_expr(item, generic_vars, opaque_vars, errors)),
        Expr::ReturnOnError(value) => {
            let uses = validate_generic_expr(value, generic_vars, opaque_vars, errors);
            if uses {
                generic_operation(errors, expression.1, "error propagation");
            }
            uses
        }
        Expr::Box(value) => validate_generic_expr(value, generic_vars, opaque_vars, errors),
        Expr::Error
        | Expr::Value(_)
        | Expr::SelfRef
        | Expr::BuiltinType(_)
        | Expr::MapNew(_, _)
        | Expr::SetNew(_) => false,
    }
}

fn validate_generic_block<'src>(
    block: &Block<'src>,
    generic_params: &HashSet<&'src str>,
    ctx: &Ctx<'src>,
    generic_vars: &mut HashSet<&'src str>,
    opaque_vars: &mut HashSet<&'src str>,
    errors: &mut Vec<Error>,
) {
    for element in &block.elements {
        match &element.0 {
            BlockElement::Let {
                name, ty, value, ..
            } => {
                validate_generic_signature_type(ty, generic_params, ctx, false, errors);
                validate_generic_expr(value, generic_vars, opaque_vars, errors);
                if expr_is_literal(value) && type_ref_is_param(&ty.0, generic_params) {
                    generic_operation(errors, value.1, "a generic-parameter literal or value");
                }
                if type_ref_mentions_param(&ty.0, generic_params) {
                    generic_vars.insert(name);
                }
                if type_ref_is_param(&ty.0, generic_params) {
                    opaque_vars.insert(name);
                }
            }
            BlockElement::Assign { value, .. } | BlockElement::IndexedAssign { value, .. } => {
                validate_generic_expr(value, generic_vars, opaque_vars, errors);
            }
            BlockElement::While {
                condition, body, ..
            } => {
                if validate_generic_expr(condition, generic_vars, opaque_vars, errors) {
                    generic_operation(errors, condition.1, "truthiness");
                }
                let mut nested = generic_vars.clone();
                let mut nested_opaque = opaque_vars.clone();
                validate_generic_block(
                    body,
                    generic_params,
                    ctx,
                    &mut nested,
                    &mut nested_opaque,
                    errors,
                );
            }
            BlockElement::For { iterable, body, .. } => {
                if validate_generic_expr(iterable, generic_vars, opaque_vars, errors) {
                    generic_operation(errors, iterable.1, "iteration");
                }
                let mut nested = generic_vars.clone();
                let mut nested_opaque = opaque_vars.clone();
                validate_generic_block(
                    body,
                    generic_params,
                    ctx,
                    &mut nested,
                    &mut nested_opaque,
                    errors,
                );
            }
            BlockElement::Return(value, _span) => {
                if let Some(value) = value {
                    validate_generic_expr(value, generic_vars, opaque_vars, errors);
                }
            }
            BlockElement::Defer { call, .. } => {
                validate_generic_expr(call, generic_vars, opaque_vars, errors);
            }
            BlockElement::If(form) => {
                for (condition, body) in &form.arms {
                    match condition {
                        Condition::Expr(value) => {
                            if validate_generic_expr(value, generic_vars, opaque_vars, errors) {
                                generic_operation(errors, value.1, "truthiness");
                            }
                        }
                        Condition::TypeTest(test) => {
                            if matches!(test.place.root, PlaceRootName::Name(name) if generic_vars.contains(name))
                            {
                                generic_operation(errors, test.span, "type tests");
                            }
                        }
                    }
                    let mut nested = generic_vars.clone();
                    let mut nested_opaque = opaque_vars.clone();
                    validate_generic_block(
                        body,
                        generic_params,
                        ctx,
                        &mut nested,
                        &mut nested_opaque,
                        errors,
                    );
                }
                if let Some(body) = &form.else_branch {
                    let mut nested = generic_vars.clone();
                    let mut nested_opaque = opaque_vars.clone();
                    validate_generic_block(
                        body,
                        generic_params,
                        ctx,
                        &mut nested,
                        &mut nested_opaque,
                        errors,
                    );
                }
            }
            BlockElement::Expr(value) => {
                validate_generic_expr(value, generic_vars, opaque_vars, errors);
            }
            BlockElement::Break(_) => {}
        }
    }
}

fn expr_is_literal(expression: &Spanned<Expr<'_>>) -> bool {
    matches!(expression.0, Expr::Value(_))
}

fn validate_generic_signature_type(
    ty: &Spanned<TypeRef<'_>>,
    params: &HashSet<&str>,
    ctx: &Ctx<'_>,
    nil_member: bool,
    errors: &mut Vec<Error>,
) {
    match &ty.0 {
        TypeRef::Builtin(TypeName::Nil) if !nil_member => errors.push(Error {
            span: ty.1,
            msg: types::STANDALONE_NIL.to_string(),
        }),
        TypeRef::Builtin(_) => {}
        TypeRef::Named(path) => {
            if path.len() == 1 && params.contains(path[0].0) {
                return;
            }
            let Some(root) = ctx.types.top_level(path[0].0) else {
                errors.push(Error {
                    span: path[0].1,
                    msg: format!("Unknown type '{}'", path[0].0),
                });
                return;
            };
            if path.len() == 2 {
                if ctx.types.member(root, path[1].0).is_none() {
                    errors.push(Error {
                        span: path[1].1,
                        msg: format!("Unknown type '{}.{}'", path[0].0, path[1].0),
                    });
                }
            } else if path.len() > 2 {
                errors.push(Error {
                    span: ty.1,
                    msg: "a qualified type name has at most two components".into(),
                });
            }
        }
        TypeRef::Apply { path, args } => {
            if path.len() != 1 {
                errors.push(Error {
                    span: ty.1,
                    msg: "generic type applications must name a top-level type".into(),
                });
            } else if let Some(declaration) = ctx.generic_types.get(path[0].0) {
                if declaration.generic_params.len() != args.len() {
                    errors.push(Error {
                        span: ty.1,
                        msg: format!(
                            "generic type '{}' expects {} type arguments, found {}",
                            path[0].0,
                            declaration.generic_params.len(),
                            args.len()
                        ),
                    });
                }
            } else {
                errors.push(Error {
                    span: path[0].1,
                    msg: format!("Unknown generic type '{}'", path[0].0),
                });
            }
            for argument in args {
                validate_generic_signature_type(argument, params, ctx, false, errors);
            }
        }
        TypeRef::Sum(members) => {
            for member in members {
                validate_generic_signature_type(member, params, ctx, true, errors);
            }
        }
        TypeRef::Box(inner)
        | TypeRef::View(inner)
        | TypeRef::Array(inner, _)
        | TypeRef::List(inner)
        | TypeRef::Set(inner) => {
            validate_generic_signature_type(inner, params, ctx, false, errors);
        }
        TypeRef::Map(key, value) => {
            validate_generic_signature_type(key, params, ctx, false, errors);
            validate_generic_signature_type(value, params, ctx, false, errors);
        }
    }
}

fn validate_generic_function<'src>(
    function: &Func<'src>,
    ctx: &Ctx<'src>,
    errors: &mut Vec<Error>,
) {
    let mut generic_params = HashSet::new();
    for (name, span) in &function.generic_params {
        if !generic_params.insert(*name) {
            errors.push(Error {
                span: *span,
                msg: format!("Generic parameter '{name}' already exists"),
            });
        }
    }
    let mut generic_vars = HashSet::new();
    let mut opaque_vars = HashSet::new();
    let mut value_params = HashSet::new();
    for param in &function.args {
        if !value_params.insert(param.name) {
            errors.push(Error {
                span: param.span,
                msg: format!("Parameter '{}' already exists", param.name),
            });
        }
        validate_generic_signature_type(&param.ty, &generic_params, ctx, false, errors);
        if type_ref_mentions_param(&param.ty.0, &generic_params) {
            generic_vars.insert(param.name);
        }
        if type_ref_is_param(&param.ty.0, &generic_params) {
            opaque_vars.insert(param.name);
        }
    }
    if let Some(result) = &function.ret {
        validate_generic_signature_type(result, &generic_params, ctx, false, errors);
    }
    validate_generic_block(
        &function.body,
        &generic_params,
        ctx,
        &mut generic_vars,
        &mut opaque_vars,
        errors,
    );
}

/// Starts one function-wide binding region: clears the reserved-name set, binds
/// every parameter, and records the receiver type for a method.
fn begin_region<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    args: &[Param<'src>],
    params: &[TParam],
    self_ty: Option<Ty>,
) -> Vec<TParam> {
    ctx.declared.clear();
    ctx.loops.clear();
    ctx.move_state.clear();
    ctx.view_borrows.clear();
    ctx.self_ty = self_ty;
    for (arg, param) in args.iter().zip(params) {
        declare(ctx, arg.name, arg.span, "Parameter");
        env.push(Binding {
            name: arg.name,
            ty: param.ty,
            // Specification 012 section 8: ordinary parameters are immutable.
            // Specification 011 section 19 phase 2 step 1: a reference parameter
            // is a mutable root of type `T`, exactly like a `let mut` local, so
            // every read, write, field selection, and receiver use in the body
            // goes through the machinery that already exists for those.
            mutable: param.mode == ParamMode::Reference,
            type_test_alias: false,
        });
    }
    params.to_vec()
}

/// The least fixed point of "this method may write its receiver": a method
/// writes it when it writes directly, or when it calls -- transitively, and
/// through cycles -- a receiver-writing method on a `self`-rooted receiver.
/// Monotone and order-independent, so the result is deterministic.
fn solve_receiver_writes(direct: &[bool], edges: &[(MethodId, MethodId)]) -> Vec<bool> {
    let mut writes = direct.to_vec();
    loop {
        let mut changed = false;
        for (caller, callee) in edges {
            if writes[callee.index()] && !writes[caller.index()] {
                writes[caller.index()] = true;
                changed = true;
            }
        }
        if !changed {
            return writes;
        }
    }
}

fn check_duplicate_params(ctx: &mut Ctx<'_>, params: &[Param<'_>]) {
    let mut seen: Vec<&str> = Vec::new();
    for param in params {
        if seen.contains(&param.name) {
            ctx.error(
                param.span,
                format!("Parameter '{}' already exists", param.name),
            );
        } else {
            seen.push(param.name);
        }
    }
}

/// Records a function-wide binding, reporting a duplicate rather than creating
/// a second layer for the same name.
fn declare<'src>(ctx: &mut Ctx<'src>, name: &'src str, span: Span, kind: &str) {
    if ctx.declared.contains(&name) {
        ctx.error(span, format!("{kind} '{name}' already exists"));
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

/// The two types the one surviving implicit numeric conversion joins.
/// Specification 009 section 4.4 adds no further promotion.
fn numeric(ty: Ty) -> bool {
    matches!(ty, Ty::Float64 | Ty::Int64)
}

/// Numeric types that operate only on an exact type match (Specification 009
/// sections 4.5-4.6): they never promote, not even to each other.
fn exact_match_numeric(ty: Ty) -> bool {
    matches!(
        ty,
        Ty::Byte | Ty::UInt16 | Ty::UInt32 | Ty::UInt64 | Ty::Float32
    )
}

fn common_numeric(left: Ty, right: Ty) -> Option<Ty> {
    if !numeric(left) || !numeric(right) {
        return None;
    }
    Some(if left == Ty::Float64 || right == Ty::Float64 {
        Ty::Float64
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

/// Evaluates the closed subset of checked expressions whose floating result is
/// known without executing user code. This is deliberately not a general
/// constant folder: it only supports literals, the existing integer-to-float
/// widening conversion, transparent printing, and floating arithmetic.
fn known_float_value(value: &TExpr) -> Option<f64> {
    match value {
        TExpr::Num(NumLiteral::F64(value)) => Some(*value),
        TExpr::Num(NumLiteral::F32(value)) => Some(f64::from(*value)),
        TExpr::Cast(value, Ty::Float64) => match value.as_ref() {
            TExpr::Num(NumLiteral::Int(value)) => Some(*value as f64),
            _ => known_float_value(value),
        },
        TExpr::Print(value, ty) if matches!(ty, Ty::Float32 | Ty::Float64) => {
            known_float_value(value)
        }
        TExpr::Arith(left, op, right, Ty::Float64) => {
            let left = known_float_value(left)?;
            let right = known_float_value(right)?;
            Some(match op {
                ArithOp::Add => left + right,
                ArithOp::Sub => left - right,
                ArithOp::Mul => left * right,
                ArithOp::Div => left / right,
            })
        }
        TExpr::Arith(left, op, right, Ty::Float32) => {
            let left = known_float_value(left)? as f32;
            let right = known_float_value(right)? as f32;
            Some(f64::from(match op {
                ArithOp::Add => left + right,
                ArithOp::Sub => left - right,
                ArithOp::Mul => left * right,
                ArithOp::Div => left / right,
            }))
        }
        _ => None,
    }
}

fn reject_known_nan(ctx: &mut Ctx<'_>, value: &TExpr, ty: Ty, span: Span) {
    if matches!(ty, Ty::Float32 | Ty::Float64) && known_float_value(value).is_some_and(f64::is_nan)
    {
        ctx.error(
            span,
            "floating-point operation produces NaN, which is not a Snacc value".into(),
        );
    }
}

/// The one existing implicit scalar conversion (Specification 009 section
/// 4.4), reused unchanged as an inline sum's tier-2 injection rule
/// (Specification 018 section 5, "existing implicit conversions"). Named-union
/// member injection is deliberately excluded here: section 5 states a named
/// union's own member injects into an inline sum only once an exact expected
/// union type has already produced a union value, never directly.
fn implicit_conversion_target(from: Ty, to: Ty) -> bool {
    from == Ty::Int64 && to == Ty::Float64
}

/// Assignability. Beyond `Int64` to `Float64`, Specification 010 section 13 adds
/// exactly one implicit conversion: direct union-member injection, including
/// the contextual `nil` spelling of a union's `Nil` member. Specification 018
/// section 5 adds inline-sum injection: an exact direct member match wins;
/// otherwise exactly one existing implicit conversion must accept the value.
fn coerce(ctx: &mut Ctx<'_>, value: TExpr, from: Ty, to: Ty, span: Span) -> TExpr {
    if from == to {
        return value;
    }
    if from == Ty::String && matches!(to, Ty::ViewByte | Ty::ViewUnicode) {
        if let TExpr::Place(place, _) = value {
            return TExpr::ViewFromString(Box::new(TExpr::Place(place, UseMode::Copy)), to);
        }
        ctx.error(
            span,
            "a view can only be lent from a named String place; a temporary String would not live long enough".into(),
        );
        return TExpr::Nil;
    }
    if from == Ty::Int64 && to == Ty::Float64 {
        return TExpr::Cast(Box::new(value), Ty::Float64);
    }
    if let Ty::User(union) = to {
        if let Ty::User(member) = from
            && ctx.types.containing_union(member) == Some(union)
        {
            return TExpr::Inject {
                member,
                into_union: union,
                value: Box::new(value),
            };
        }
        // Specification 012 section 10: `nil` names the `Nil` member of one
        // expected union and never has the standalone type `Nil`.
        if from == Ty::Nil
            && let Some(member) = ctx.types.member(union, "Nil")
        {
            return TExpr::Inject {
                member,
                into_union: union,
                value: Box::new(TExpr::Construct {
                    type_id: member,
                    fields: Vec::new(),
                }),
            };
        }
    }
    if let Ty::Sum(sum) = to {
        let members = ctx.types.sum_members(sum).to_vec();
        // Tier 1: an exact direct member match, including a literal `Nil`
        // member selected by the contextual `nil` literal, which duplicate
        // rejection already guarantees is unique when present.
        if members.contains(&from) {
            return TExpr::InjectSum {
                sum,
                member: from,
                value: Box::new(value),
            };
        }
        // Tier 2: exactly one existing implicit conversion must accept the
        // value; more than one is an ambiguity, and a sum can hold at most
        // one member of any given type, so today's single scalar conversion
        // rule can never itself produce more than one candidate.
        let candidates: Vec<Ty> = members
            .iter()
            .copied()
            .filter(|member| implicit_conversion_target(from, *member))
            .collect();
        match candidates.as_slice() {
            [one] => {
                let converted = coerce(ctx, value, from, *one, span);
                return TExpr::InjectSum {
                    sum,
                    member: *one,
                    value: Box::new(converted),
                };
            }
            [] => {}
            _ => {
                let found = ctx.name(from);
                let target = ctx.name(to);
                ctx.error(
                    span,
                    format!(
                        "'{found}' could convert into more than one member of '{target}'; \
                         add an explicit conversion to pick one"
                    ),
                );
                return value;
            }
        }
    }
    ctx.mismatch(span, to, from);
    value
}

/// Bridge-only view compatibility. `View<Byte>` and `View<Unicode>` retain
/// their string-specific internal types, while a collection view of the same
/// element is a generalized descriptor. The generated bridge consumes both
/// through the identical pointer/length ABI, so this compatibility is valid
/// only for `extern rust` calls and must not leak into string operations.
fn coerce_bridge_view(ctx: &mut Ctx<'_>, value: TExpr, from: Ty, to: Ty, span: Span) -> TExpr {
    if from == to {
        return value;
    }
    let from_elem = match from {
        Ty::View(id) => match ctx.types.collection(id) {
            CollectionDef::View { elem } => Some(*elem),
            _ => None,
        },
        Ty::ViewByte => Some(Ty::Byte),
        Ty::ViewUnicode => Some(Ty::Unicode),
        _ => None,
    };
    let to_elem = match to {
        Ty::View(id) => match ctx.types.collection(id) {
            CollectionDef::View { elem } => Some(*elem),
            _ => None,
        },
        Ty::ViewByte => Some(Ty::Byte),
        Ty::ViewUnicode => Some(Ty::Unicode),
        _ => None,
    };
    if from_elem.is_some() && from_elem == to_elem {
        return value;
    }
    coerce(ctx, value, from, to, span)
}

/// A propagation expression has already selected the successful payload. It
/// may therefore be re-injected into the enclosing callable's larger result
/// sum; ordinary sum assignment remains exact and does not use this rule.
fn coerce_return_success(ctx: &mut Ctx<'_>, value: TExpr, from: Ty, to: Ty, span: Span) -> TExpr {
    if let (Ty::Sum(source), Ty::Sum(target)) = (from, to) {
        let source_members = ctx.types.sum_members(source);
        let target_members = ctx.types.sum_members(target);
        if source_members
            .iter()
            .all(|member| target_members.contains(member))
        {
            return TExpr::LiftSum {
                value: Box::new(value),
                from: source,
                to: target,
            };
        }
    }
    coerce(ctx, value, from, to, span)
}

/// A resolved place plus its root's mutability. Mutability is a property of the
/// root alone (Specification 012 section 7); the struct definition is never
/// consulted.
struct Resolved {
    place: Place,
    mutable: bool,
}

/// Whether an expression resolves to a place.
enum PlaceOutcome {
    Resolved(Resolved),
    /// The root was a binding or `self` but the field path failed; a diagnostic
    /// has already been recorded.
    Reported,
    /// The root is not a binding, so the caller may treat this as a value or a
    /// qualified type path.
    NotAPlace,
}

/// Splits `a.b.c` into its innermost atom and the field names selected from it.
fn flatten<'a, 'src>(
    expr: &'a Spanned<Expr<'src>>,
) -> (&'a Spanned<Expr<'src>>, Vec<Spanned<&'src str>>) {
    let mut fields = Vec::new();
    let mut current = expr;
    while let Expr::Member(base, name) = &current.0 {
        fields.push(*name);
        current = base;
    }
    fields.reverse();
    (current, fields)
}

/// Specification 016 section 4.3: field access and method calls automatically
/// dereference as many box layers as member resolution requires. Peeling
/// stops at the first non-box type, so a plain struct or union passes through
/// unchanged and `Box<Box<T>>` peels both layers.
fn deref_box(types: &Types, ty: Ty) -> Ty {
    let mut current = ty;
    while let Ty::Box(id) = current {
        current = types.box_pointee(id);
    }
    current
}

/// Walks a field path from a root type, reporting the first failure.
fn walk_fields(
    ctx: &mut Ctx<'_>,
    root_ty: Ty,
    fields: &[Spanned<&str>],
) -> Option<(Vec<usize>, Ty)> {
    let mut path = Vec::new();
    let mut current = root_ty;
    for (name, span) in fields {
        // Specification 016 section 4.3: cross as many box layers as needed
        // to reach the struct that actually owns this field.
        current = deref_box(&ctx.types, current);
        let Ty::User(id) = current else {
            let owner = ctx.name(current);
            ctx.error(
                *span,
                format!("'{owner}' is not a struct, so it has no field '{name}'"),
            );
            return None;
        };
        let Some((index, ty)) = ctx.types.field(id, name) else {
            let owner = ctx.types.def(id).name().to_string();
            let msg = if ctx.method_index.contains_key(&(id, (*name).to_string())) {
                format!("'{owner}.{name}' is a method; a method requires a receiver call")
            } else if ctx.types.def(id).fields().is_none() {
                format!("'{owner}' is not a struct, so it has no field '{name}'")
            } else {
                format!("'{owner}' has no field '{name}'")
            };
            ctx.error(*span, msg);
            return None;
        };
        path.push(index);
        current = ty;
    }
    Some((path, current))
}

/// Resolves the root of a place without reporting anything, so a caller can
/// fall back to value or type-path resolution.
fn place_root<'src>(
    ctx: &Ctx<'src>,
    env: &Env<'src>,
    root: &Expr<'src>,
) -> Option<(PlaceRoot, Ty, bool)> {
    match root {
        Expr::SelfRef => ctx
            .self_ty
            // Specification 012 section 9: `self` is writable inside its own
            // method body; caller permission is enforced at each call site.
            .map(|ty| (PlaceRoot::SelfRef, ty, true)),
        Expr::Local(name) => {
            env.iter()
                .rev()
                .find(|binding| binding.name == *name)
                .map(|binding| {
                    (
                        PlaceRoot::Local((*name).to_string()),
                        binding.ty,
                        binding.mutable,
                    )
                })
        }
        _ => None,
    }
}

fn as_place<'src>(
    ctx: &mut Ctx<'src>,
    env: &Env<'src>,
    expr: &Spanned<Expr<'src>>,
) -> PlaceOutcome {
    let (root, fields) = flatten(expr);
    let Some((place_root, root_ty, mutable)) = place_root(ctx, env, &root.0) else {
        return PlaceOutcome::NotAPlace;
    };
    let Some((path, ty)) = walk_fields(ctx, root_ty, &fields) else {
        return PlaceOutcome::Reported;
    };
    PlaceOutcome::Resolved(Resolved {
        place: Place {
            root: place_root,
            root_ty,
            path,
            ty,
        },
        mutable,
    })
}

/// Resolves a written place (an assignment target or an `is` subject).
fn resolve_place<'src>(
    ctx: &mut Ctx<'src>,
    env: &Env<'src>,
    path: &PlacePath<'src>,
) -> Option<Resolved> {
    let (root, root_ty, mutable) = match path.root {
        PlaceRootName::SelfRef => match ctx.self_ty {
            Some(ty) => (PlaceRoot::SelfRef, ty, true),
            None => {
                ctx.error(
                    path.root_span,
                    "'self' is only valid inside a method body".into(),
                );
                return None;
            }
        },
        PlaceRootName::Name(name) => {
            let Some(binding) = env.iter().rev().find(|binding| binding.name == name) else {
                ctx.error(
                    path.root_span,
                    format!("No such variable '{name}' in scope"),
                );
                return None;
            };
            (
                PlaceRoot::Local(name.to_string()),
                binding.ty,
                binding.mutable,
            )
        }
    };
    let (field_path, ty) = walk_fields(ctx, root_ty, &path.fields)?;
    Some(Resolved {
        place: Place {
            root,
            root_ty,
            path: field_path,
            ty,
        },
        mutable,
    })
}

/// Specification 016 section 6.1: tags a checked value with its [`UseMode`]
/// when it occupies one of the five consuming contexts (initialization,
/// assignment's right operand, a by-value argument, a function/method
/// result, or an aggregate constructor argument), and -- for a whole
/// move-only root -- runs the section 6.2 availability check this represents.
/// Every call site applies this before `coerce`, so a value later wrapped by
/// an implicit union or sum injection still carries the tag and, when
/// applicable, has already been checked. A value that is not a bare place
/// read (a call result, a fresh `box(...)`, a nested `if`, ...) has no root
/// to transfer and passes through unchanged.
///
/// Specification 016 section 6.4 rejects moving a move-only value out of a
/// field, union payload projection, or automatic box dereference -- only a
/// bare root (an empty path) may be wholly consumed. A union-test binding
/// (Specification 016 section 7.3) is the one case where an empty path is
/// still not a legitimate whole root: it is always a branch-scoped alias to
/// its tested place's payload, so `env` is consulted to reject that case too.
fn mark_consumed<'src>(ctx: &mut Ctx<'src>, env: &Env<'src>, expr: TExpr, span: Span) -> TExpr {
    let TExpr::Place(place, _) = expr else {
        return expr;
    };
    let whole_root = place.path.is_empty() && !is_type_test_alias(env, &place.root);
    if whole_root {
        check_move(ctx, &place, span);
    } else if ctx.types.is_move_only(place.ty) {
        let name = ctx.place_name(&place);
        ctx.error(
            span,
            format!("'{name}' cannot be moved out of; only a complete owning root can be moved"),
        );
    }
    TExpr::Place(place, UseMode::Consume)
}

/// Whether `root` currently names a union- or sum-test binding rather than an
/// ordinary local, parameter, or `self` (Specification 016 section 7.3).
fn is_type_test_alias<'src>(env: &Env<'src>, root: &PlaceRoot) -> bool {
    let PlaceRoot::Local(name) = root else {
        return false;
    };
    env.iter()
        .rev()
        .find(|binding| binding.name == name.as_str())
        .is_some_and(|binding| binding.type_test_alias)
}

/// Specification 016 section 6.2: a consuming use of a move-only root
/// requires availability. Reports a use-after-move diagnostic naming the root
/// if it is already moved; otherwise marks it moved from this point forward.
/// A no-op for a copyable type, which stays an ordinary copy regardless of
/// how many times it is used (Specification 016 section 5.3).
fn check_move(ctx: &mut Ctx<'_>, place: &Place, span: Span) {
    if !ctx.types.is_move_only(place.ty) {
        return;
    }
    reject_live_view_source(ctx, &place.root, span);
    if ctx.move_state.contains_key(&place.root) {
        ctx.error(
            span,
            format!("'{}' is already moved, so this use is invalid", place.root),
        );
        return;
    }
    ctx.move_state.insert(place.root.clone(), span);
}

fn is_builtin_view(ty: Ty) -> bool {
    matches!(ty, Ty::ViewByte | Ty::ViewUnicode | Ty::View(_))
}

fn is_borrowed_type(ctx: &Ctx<'_>, ty: Ty) -> bool {
    fn visit(ctx: &Ctx<'_>, ty: Ty, seen: &mut std::collections::HashSet<Ty>) -> bool {
        if !seen.insert(ty) {
            return false;
        }
        match ty {
            Ty::ViewByte | Ty::ViewUnicode => true,
            Ty::Sum(id) => ctx
                .types
                .sum_members(id)
                .iter()
                .copied()
                .any(|member| visit(ctx, member, seen)),
            Ty::User(id) => {
                if let Some(fields) = ctx.types.def(id).fields() {
                    fields.iter().any(|(_, field)| visit(ctx, *field, seen))
                } else if let Some(members) = ctx.types.union_members(id) {
                    members.iter().any(|member| {
                        ctx.types.def(*member).fields().is_some_and(|fields| {
                            fields.iter().any(|(_, field)| visit(ctx, *field, seen))
                        })
                    })
                } else {
                    ctx.types
                        .represented_target(id)
                        .is_some_and(|target| visit(ctx, target, seen))
                }
            }
            Ty::Box(id) => visit(ctx, ctx.types.box_pointee(id), seen),
            Ty::Array(id) | Ty::List(id) | Ty::View(id) => match ctx.types.collection(id) {
                CollectionDef::Array { elem, .. }
                | CollectionDef::List { elem }
                | CollectionDef::View { elem } => visit(ctx, *elem, seen),
                _ => false,
            },
            Ty::Map(id) => match ctx.types.collection(id) {
                CollectionDef::Map { key, value } => {
                    visit(ctx, *key, seen) || visit(ctx, *value, seen)
                }
                _ => false,
            },
            Ty::Set(id) => match ctx.types.collection(id) {
                CollectionDef::Set { elem } => visit(ctx, *elem, seen),
                _ => false,
            },
            _ => false,
        }
    }
    visit(ctx, ty, &mut std::collections::HashSet::new())
}

fn view_sources(ctx: &Ctx<'_>, value: &TExpr) -> Vec<PlaceRoot> {
    fn merge(into: &mut Vec<PlaceRoot>, sources: impl IntoIterator<Item = PlaceRoot>) {
        for source in sources {
            if !into.contains(&source) {
                into.push(source);
            }
        }
    }

    match value {
        TExpr::ViewFromString(value, _) => match value.as_ref() {
            TExpr::Place(place, _) => vec![place.root.clone()],
            _ => Vec::new(),
        },
        TExpr::CollectionView(value, _) => view_sources(ctx, value),
        TExpr::CollectionSlice { value, .. } => view_sources(ctx, value),
        TExpr::ViewSlice { value, .. } => view_sources(ctx, value),
        TExpr::Place(place, _) => match &place.root {
            PlaceRoot::Local(name) => ctx
                .view_borrows
                .iter()
                .find(|borrow| borrow.view_name == *name)
                .map_or_else(Vec::new, |borrow| borrow.sources.clone()),
            PlaceRoot::SelfRef => Vec::new(),
        },
        TExpr::Construct { fields, .. } => {
            let mut sources = Vec::new();
            for (_, field) in fields {
                merge(&mut sources, view_sources(ctx, field));
            }
            sources
        }
        TExpr::FieldRead { base, .. }
        | TExpr::Represent { value: base, .. }
        | TExpr::Inject { value: base, .. }
        | TExpr::InjectSum { value: base, .. }
        | TExpr::LiftSum { value: base, .. } => view_sources(ctx, base),
        TExpr::If(form) => {
            let mut sources = Vec::new();
            for (_, block) in &form.arms {
                if let Some(value) = &block.result {
                    merge(&mut sources, view_sources(ctx, value));
                }
            }
            if let Some(block) = &form.else_branch
                && let Some(value) = &block.result
            {
                merge(&mut sources, view_sources(ctx, value));
            }
            sources
        }
        TExpr::Print(value, ty) if is_builtin_view(*ty) => view_sources(ctx, value),
        _ => Vec::new(),
    }
}

fn reject_live_view_source(ctx: &mut Ctx<'_>, root: &PlaceRoot, span: Span) {
    if let Some(borrow) = ctx
        .view_borrows
        .iter()
        .find(|borrow| borrow.sources.contains(root))
    {
        ctx.error(
            span,
            format!(
                "cannot move or replace '{}': view '{}' still borrows it",
                root, borrow.view_name
            ),
        );
    }
}

fn expr_mentions_local<'src>(expression: &Expr<'src>, name: &str) -> bool {
    match expression {
        Expr::Local(local) => *local == name,
        Expr::List(items) => items.iter().any(|item| expr_mentions_local(&item.0, name)),
        Expr::Member(base, _) => expr_mentions_local(&base.0, name),
        Expr::Index(base, index) => {
            expr_mentions_local(&base.0, name) || expr_mentions_local(&index.0, name)
        }
        Expr::Binary(left, _, right) => {
            expr_mentions_local(&left.0, name) || expr_mentions_local(&right.0, name)
        }
        Expr::Unary(_, value)
        | Expr::ReturnOnError(value)
        | Expr::Print(value)
        | Expr::Box(value) => expr_mentions_local(&value.0, name),
        Expr::Call(callee, arguments) => {
            expr_mentions_local(&callee.0, name)
                || arguments
                    .0
                    .iter()
                    .any(|argument| expr_mentions_local(&argument.value.0, name))
        }
        Expr::GenericCall(callee, _, arguments) => {
            expr_mentions_local(&callee.0, name)
                || arguments
                    .0
                    .iter()
                    .any(|argument| expr_mentions_local(&argument.value.0, name))
        }
        Expr::Interpolated(parts) => parts.iter().any(|part| match part {
            crate::syntax::ast::StringPart::Literal(_) => false,
            crate::syntax::ast::StringPart::Expression(expression) => {
                expr_mentions_local(&expression.0, name)
            }
        }),
        Expr::Error
        | Expr::Value(_)
        | Expr::MapNew(_, _)
        | Expr::SetNew(_)
        | Expr::SelfRef
        | Expr::BuiltinType(_) => false,
    }
}

fn block_mentions_local<'src>(block: &Block<'src>, name: &str) -> bool {
    block
        .elements
        .iter()
        .any(|element| element_mentions_local(element, name))
}

fn element_mentions_local<'src>(element: &Spanned<BlockElement<'src>>, name: &str) -> bool {
    match &element.0 {
        BlockElement::Let { value, .. } | BlockElement::Assign { value, .. } => {
            expr_mentions_local(&value.0, name)
        }
        BlockElement::IndexedAssign { target, value } => {
            expr_mentions_local(&target.0, name) || expr_mentions_local(&value.0, name)
        }
        BlockElement::While {
            condition, body, ..
        } => expr_mentions_local(&condition.0, name) || block_mentions_local(body, name),
        BlockElement::For { iterable, body, .. } => {
            expr_mentions_local(&iterable.0, name) || block_mentions_local(body, name)
        }
        BlockElement::Return(value, _) => value
            .as_ref()
            .is_some_and(|value| expr_mentions_local(&value.0, name)),
        BlockElement::Defer { call, .. } | BlockElement::Expr(call) => {
            expr_mentions_local(&call.0, name)
        }
        BlockElement::If(form) => {
            form.arms.iter().any(|(condition, body)| {
                let condition_uses = match condition {
                    Condition::Expr(expression) => expr_mentions_local(&expression.0, name),
                    Condition::TypeTest(test) => {
                        matches!(test.place.root, PlaceRootName::Name(root) if root == name)
                    }
                };
                condition_uses || block_mentions_local(body, name)
            }) || form
                .else_branch
                .as_ref()
                .is_some_and(|body| block_mentions_local(body, name))
        }
        BlockElement::Break(_) => false,
    }
}

fn prune_view_borrows<'src>(
    ctx: &mut Ctx<'src>,
    remaining: &[Spanned<BlockElement<'src>>],
    scope: usize,
) {
    ctx.view_borrows.retain(|borrow| {
        borrow.scope < scope
            || remaining
                .iter()
                .any(|element| element_mentions_local(element, &borrow.view_name))
    });
}

/// Specification 016 section 6.2: a root is available after a merge only when
/// available on every reachable predecessor, so the merged moved-set is the
/// union of every predecessor's moved-set -- moved on even one predecessor is
/// enough to make it unavailable afterward. Keeps whichever span is found
/// first; which one survives does not matter; only whether the root is moved
/// at all does.
fn merge_moves(exits: Vec<HashMap<PlaceRoot, Span>>) -> HashMap<PlaceRoot, Span> {
    let mut merged = HashMap::new();
    for exit in exits {
        for (root, span) in exit {
            merged.entry(root).or_insert(span);
        }
    }
    merged
}

/// Specification 016 section 8.1's checked cleanup plan: every binding in
/// `env[scope..]` (a block's own locally declared bindings, or -- called
/// with `scope: 0` once a function/method body finishes -- its parameters)
/// that is move-only and still available (absent from `ctx.move_state`) at
/// this exact point, in reverse declaration order so the last one bound is
/// the first destroyed (spec: "locals drop in reverse successful-
/// initialization order"). A union-/sum-test alias is excluded even though
/// its own path is empty: it is never an independent owning root
/// (`type_test_alias`'s own doc comment), so it is never a legitimate drop
/// target any more than it is a legitimate whole-root move.
fn compute_drops(ctx: &Ctx<'_>, env: &Env<'_>, scope: usize) -> Vec<Place> {
    let mut drops: Vec<Place> = env[scope..]
        .iter()
        .filter(|binding| {
            !binding.type_test_alias
                && ctx.types.is_move_only(binding.ty)
                && !ctx
                    .move_state
                    .contains_key(&PlaceRoot::Local(binding.name.to_string()))
        })
        .map(|binding| Place {
            root: PlaceRoot::Local(binding.name.to_string()),
            root_ty: binding.ty,
            path: Vec::new(),
            ty: binding.ty,
        })
        .collect();
    drops.reverse();
    drops
}

/// Builds an exit plan from currently armed entries. Returns exit every
/// currently open lexical scope; `scope_start` lets `break` retain outer
/// scopes. Parameter drops are added when no local cleanup entry represents
/// that root.
fn cleanup_for_exit(ctx: &Ctx<'_>, env: &Env<'_>, scope_start: usize) -> Vec<TCleanup> {
    let mut entries = Vec::new();
    let mut known_roots = Vec::new();
    for scope in ctx.cleanup_scopes.iter().skip(scope_start) {
        for entry in scope {
            match entry {
                TCleanup::Drop(place) => {
                    known_roots.push(place.root.clone());
                    if !ctx.move_state.contains_key(&place.root) {
                        entries.push(TCleanup::Drop(place.clone()));
                    }
                }
                TCleanup::Deferred(deferred) => {
                    entries.push(TCleanup::Deferred(Rc::clone(deferred)));
                }
            }
        }
    }
    for place in compute_drops(ctx, env, 0).into_iter().rev() {
        if !known_roots.contains(&place.root) {
            entries.push(TCleanup::Drop(place));
        }
    }
    entries.reverse();
    entries
}

fn cleanup_for_exit_from_entries(ctx: &Ctx<'_>, entries: Vec<TCleanup>) -> Vec<TCleanup> {
    entries
        .into_iter()
        .filter(|entry| {
            !matches!(entry, TCleanup::Drop(place) if ctx.move_state.contains_key(&place.root))
        })
        .rev()
        .collect()
}

enum CheckedReturnOnError {
    Expr { value: TExpr, ty: Ty },
    Statement(TStmt),
}

/// Checks both forms of `return_on_error`. The statement form is selected by
/// the block checker, not by a second parser production, so the operand is
/// still checked exactly once and retains the ordinary move rules.
fn check_return_on_error<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    value: &Spanned<Expr<'src>>,
    span: Span,
    statement_form: bool,
) -> CheckedReturnOnError {
    let (value, value_ty) = check_expr(ctx, env, value);
    let fallback = || {
        if statement_form {
            CheckedReturnOnError::Statement(TStmt::Expr(TExpr::Nil))
        } else {
            CheckedReturnOnError::Expr {
                value: TExpr::Nil,
                ty: Ty::Nil,
            }
        }
    };
    let Some(error_ty) = ctx.types.top_level("Error").map(Ty::User) else {
        ctx.error(span, "the predeclared 'Error' type is unavailable".into());
        return fallback();
    };
    let Ty::Sum(sum) = value_ty else {
        ctx.error(
            span,
            "'return_on_error' requires an inline sum containing exact 'Error'".into(),
        );
        return fallback();
    };
    let members = ctx.types.sum_members(sum);
    if !members.contains(&error_ty) {
        ctx.error(
            span,
            "'return_on_error' requires an inline sum containing exact 'Error'".into(),
        );
        return fallback();
    }
    let successes: Vec<Ty> = members
        .iter()
        .copied()
        .filter(|member| *member != error_ty)
        .collect();
    let Some(Some(result)) = ctx.callable_result else {
        ctx.error(
            span,
            "'return_on_error' is only valid inside a fallible callable".into(),
        );
        return fallback();
    };
    let Ty::Sum(result_sum) = result else {
        ctx.error(
            span,
            "'return_on_error' requires a fallible inline-sum result".into(),
        );
        return fallback();
    };
    if !ctx.types.sum_members(result_sum).contains(&error_ty) {
        ctx.error(
            span,
            "'return_on_error' requires an enclosing result containing exact 'Error'".into(),
        );
        return fallback();
    }
    if statement_form {
        if successes.as_slice() != [Ty::Nil] {
            ctx.error(
                span,
                "'return_on_error' statement form requires an operand of exactly 'Nil | Error'"
                    .into(),
            );
            return fallback();
        }
        let value = mark_consumed(ctx, env, value, span);
        let cleanup = cleanup_for_exit(ctx, env, 0);
        return CheckedReturnOnError::Statement(TStmt::ReturnOnError {
            value,
            sum,
            result,
            cleanup,
        });
    }
    if !successes.iter().any(|member| *member != Ty::Nil) {
        ctx.error(
            span,
            "'return_on_error' requires at least one non-Nil success member".into(),
        );
        return fallback();
    }
    let success = if let [success] = successes.as_slice() {
        *success
    } else {
        let mut reduced = successes;
        reduced.sort();
        Ty::Sum(ctx.types.intern_sum(reduced))
    };
    let value = mark_consumed(ctx, env, value, span);
    let cleanup = cleanup_for_exit(ctx, env, 0);
    CheckedReturnOnError::Expr {
        value: TExpr::ReturnOnError {
            value: Box::new(value),
            sum,
            success,
            result,
            cleanup,
        },
        ty: success,
    }
}

/// The first reachable block element after an unconditional callable return
/// or a conditional whose every reachable branch returns (Specification 026
/// section 7).
const UNREACHABLE_AFTER_RETURN: &str = "this code is unreachable: every path that reaches it has already returned from this \
     function or method";

/// Checks a block, threading Specification 026 section 6's flow-outcome
/// classification through statement, `return`, and `if` handling.
/// `expected` is `Some(ty)` for a value-required block (a function/method
/// body that declares a result, or a value-form `if` branch) and `None` for
/// a no-result block; every element but the last is always an ordinary
/// statement. The last element may satisfy the block's value requirement --
/// when `expected.is_some()` -- with a trailing expression, a value-form
/// `if`, or (new in this specification) by executing `return` on every
/// reachable path, which needs no value of its own (section 6). Returns the
/// checked block and whether every reachable path through it executes an
/// explicit `return`: the "callable return" outcome, which a caller excludes
/// from its own move-state and reachability merges (section 8) exactly as it
/// would an unreachable predecessor.
fn check_block<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    block: &Block<'src>,
    expected: Option<Ty>,
) -> (TBlock, bool) {
    let scope = env.len();
    ctx.cleanup_scopes.push(Vec::new());
    let mut statements = Vec::new();
    let mut result = None;
    let mut returns = false;
    let mut reported_unreachable = false;
    let last = block.elements.len().wrapping_sub(1);
    for (index, element) in block.elements.iter().enumerate() {
        prune_view_borrows(ctx, &block.elements[index..], scope);
        // Specification 026 section 7: only the first unreachable element is
        // reported; flow bookkeeping below still runs for dead code (using
        // `|=` so a later, non-returning nested construct can never make an
        // already-unreachable point look reachable again), so dead code is
        // still fully type-checked for unrelated errors.
        let already_unreachable = returns;
        if already_unreachable && !reported_unreachable {
            ctx.error(element.1, UNREACHABLE_AFTER_RETURN.into());
            reported_unreachable = true;
        }
        let is_last = index == last;
        if let BlockElement::Let { .. } = &element.0
            && !(is_last && !already_unreachable && expected.is_some())
        {
            let statement = check_stmt(ctx, env, element);
            if let TStmt::Let { name, ty, .. } = &statement
                && ctx.types.is_move_only(*ty)
            {
                ctx.cleanup_scopes
                    .last_mut()
                    .expect("a checked block owns a cleanup scope")
                    .push(TCleanup::Drop(Place {
                        root: PlaceRoot::Local(name.clone()),
                        root_ty: *ty,
                        path: Vec::new(),
                        ty: *ty,
                    }));
            }
            statements.push(statement);
            continue;
        }
        match &element.0 {
            BlockElement::Defer {
                on_error,
                call,
                span,
            } => {
                if is_last && !already_unreachable && expected.is_some() {
                    let name = ctx.name(expected.expect("guarded by expected.is_some() above"));
                    ctx.error(
                        element.1,
                        format!(
                            "this block must end in an expression of type '{name}', but it ends in a deferred call"
                        ),
                    );
                }
                if let Some(deferred) = check_defer(ctx, env, *on_error, call, *span) {
                    ctx.cleanup_scopes
                        .last_mut()
                        .expect("a checked block owns a cleanup scope")
                        .push(TCleanup::Deferred(Rc::new(deferred)));
                }
            }
            BlockElement::Return(value, span) => {
                statements.push(check_return(ctx, env, value, *span));
                returns = true;
            }
            BlockElement::If(form) => {
                // Only the block's last element may supply its required
                // value, and only when nothing above already returned.
                let value_slot = if is_last && !already_unreachable {
                    expected
                } else {
                    None
                };
                let (checked, if_returns) = check_if(ctx, env, form, value_slot);
                match checked {
                    FlowIf::Value(value) => result = Some(value),
                    FlowIf::Stmt(stmt_if) => statements.push(TStmt::If(stmt_if)),
                }
                returns |= if_returns;
            }
            BlockElement::Expr(expression)
                if is_last && !already_unreachable && expected.is_some() =>
            {
                let expected_ty = expected.expect("guarded by expected.is_some() above");
                let (value, ty) = check_expr(ctx, env, expression);
                // Specification 016 section 6.1: a value block's trailing
                // expression is always a function/method result, or feeds one
                // of the other four consuming contexts one level further out
                // (Specification 016 section 6.2's `if`-arm example).
                let value = mark_consumed(ctx, env, value, expression.1);
                result = Some(if matches!(value, TExpr::ReturnOnError { .. }) {
                    coerce_return_success(ctx, value, ty, expected_ty, expression.1)
                } else {
                    coerce(ctx, value, ty, expected_ty, expression.1)
                });
            }
            _ if is_last && !already_unreachable && expected.is_some() => {
                let name = ctx.name(expected.expect("guarded by expected.is_some() above"));
                ctx.error(
                    element.1,
                    format!(
                        "this block must end in an expression of type '{name}', \
                         but it ends in a statement"
                    ),
                );
                statements.push(check_stmt(ctx, env, element));
            }
            _ => {
                statements.push(check_stmt(ctx, env, element));
            }
        }
    }
    if !returns
        && result.is_none()
        && block.elements.is_empty()
        && let Some(expected_ty) = expected
    {
        let name = ctx.name(expected_ty);
        ctx.error(
            block.span,
            format!("this block must end in an expression of type '{name}', but it is empty"),
        );
    }
    // Specification 026 section 8: a block that always returns already
    // attached its own cleanup to each `TStmt::Return` (`check_return`), so
    // it never falls off its own end; a normal-completion drop list here
    // would simply never run and would risk destroying a value the return
    // already transferred to the caller.
    let cleanup = ctx
        .cleanup_scopes
        .pop()
        .expect("a checked block owns a cleanup scope");
    let cleanup = if returns {
        Vec::new()
    } else {
        cleanup_for_exit_from_entries(ctx, cleanup)
    };
    ctx.view_borrows.retain(|borrow| borrow.scope < scope);
    env.truncate(scope);
    (
        TBlock {
            statements,
            result,
            cleanup,
        },
        returns,
    )
}

/// Checks one arm's condition, leaving any type-test binding visible in `env`
/// for the arm body only. The caller restores `env` after the body.
/// Specification 018 section 6 extends the tested place from a named union to
/// an inline sum, so the place is resolved once here and then dispatched to
/// whichever member-lookup rules its type requires.
fn check_arm_condition<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    condition: &Condition<'src>,
) -> TCondition {
    match condition {
        Condition::Expr(expression) => TCondition::Expr(check_condition(ctx, env, expression)),
        Condition::TypeTest(test) => {
            let Some(resolved) = resolve_place(ctx, env, &test.place) else {
                return TCondition::Expr(TExpr::Bool(false));
            };
            let Resolved { mut place, mutable } = resolved;
            // Specification 016 section 4.3: `tree is Tree.Branch(branch)`
            // tests the union stored *through* a `Box<Tree>` subject exactly
            // as it would a bare `Tree`, so the subject is dereferenced
            // before checking whether it is a union or an inline sum.
            place.ty = deref_box(&ctx.types, place.ty);
            match place.ty {
                Ty::User(id) if ctx.types.union_members(id).is_some() => {
                    match check_type_test(ctx, test, place, id) {
                        Some(checked) => {
                            // Specification 016 section 7.3: the binding
                            // shares the tested place's root mutability.
                            bind_type_test(env, test.binding, &checked.binding, mutable);
                            TCondition::Test(checked)
                        }
                        None => TCondition::Expr(TExpr::Bool(false)),
                    }
                }
                Ty::Sum(sum) => match check_sum_type_test(ctx, test, place, sum) {
                    Some(checked) => {
                        bind_type_test(env, test.binding, &checked.binding, mutable);
                        TCondition::SumTest(checked)
                    }
                    None => TCondition::Expr(TExpr::Bool(false)),
                },
                other => {
                    let name = ctx.name(other);
                    ctx.error(
                        test.place.span,
                        format!(
                            "the left side of 'is' must have a union type or an inline sum \
                             type, but '{}' has type '{name}'",
                            test.place
                        ),
                    );
                    TCondition::Expr(TExpr::Bool(false))
                }
            }
        }
    }
}

/// Pushes a proven type-test binding into scope for the arm body, if the
/// syntactic test carried one. Specification 012 section 7 originally made
/// every such binding immutable; Specification 016 section 7.3 generalizes
/// that to "mutable exactly when the tested place's root is," which reduces
/// to the old rule whenever the tested root itself is immutable.
fn bind_type_test<'src>(
    env: &mut Env<'src>,
    written: Option<Spanned<&'src str>>,
    checked: &Option<(String, Ty)>,
    mutable: bool,
) {
    if let (Some((name, _)), Some((_, ty))) = (written, checked) {
        env.push(Binding {
            name,
            ty: *ty,
            mutable,
            // Specification 016 section 7.3: the binding is a branch-scoped
            // alias to the tested place's active payload, never an
            // independent owning root, so `mark_consumed` must reject
            // consuming it whole exactly as it already rejects consuming one
            // of its fields (Specification 016 section 6.4).
            type_test_alias: true,
        });
    }
}

fn check_condition<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    condition: &Spanned<Expr<'src>>,
) -> TExpr {
    let (value, ty) = check_expr(ctx, env, condition);
    if ty == Ty::Nil {
        ctx.error(
            condition.1,
            "a standalone 'nil' cannot be used as a condition; give it a sum type".into(),
        );
    }
    TExpr::Truthiness(Box::new(value), ty)
}

/// What an `if`/`elseif` chain proves about member coverage.
struct ChainFact {
    /// Every arm is a type test over one syntactic place, and every direct
    /// member of that place's union or inline sum is tested exactly once.
    exhaustive: bool,
    /// The qualified names of members a same-place chain fails to handle.
    missing: Vec<String>,
}

/// A named-union member is itself a user-defined type, so wrapping its
/// `TypeId` as `Ty::User` lets a union test and a sum test share one
/// "which member did this arm test" representation below.
fn tested_member(condition: &TCondition) -> Option<(&Place, Ty)> {
    match condition {
        TCondition::Test(test) => Some((&test.place, Ty::User(test.member))),
        TCondition::SumTest(test) => Some((&test.place, test.member)),
        TCondition::Expr(_) => None,
    }
}

/// Specification 010 section 12.4, extended by Specification 018 section 6 to
/// an inline sum. Proves -- never guesses -- whether a chain covers every
/// member, and reports unreachable duplicate branches.
fn analyze_chain(ctx: &mut Ctx<'_>, arms: &[(TCondition, TBlock)], spans: &[Span]) -> ChainFact {
    let mut subject: Option<Place> = None;
    let mut tested: Vec<Ty> = Vec::new();
    let mut chain = true;
    for ((condition, _), span) in arms.iter().zip(spans) {
        let Some((place, member)) = tested_member(condition) else {
            chain = false;
            break;
        };
        match &subject {
            None => subject = Some(place.clone()),
            Some(first) if first == place => {}
            Some(_) => {
                chain = false;
                break;
            }
        }
        if tested.contains(&member) {
            let name = ctx.name(member);
            ctx.error(
                *span,
                format!(
                    "'{name}' is already handled by an earlier branch, so this branch \
                     is unreachable"
                ),
            );
        } else {
            tested.push(member);
        }
    }
    let covered = chain
        .then(|| subject.as_ref())
        .flatten()
        .and_then(|place| match place.ty {
            Ty::User(id) => ctx.types.union_members(id).map(|members| {
                members
                    .iter()
                    .map(|member| Ty::User(*member))
                    .collect::<Vec<Ty>>()
            }),
            Ty::Sum(id) => Some(ctx.types.sum_members(id).to_vec()),
            _ => None,
        });
    let Some(members) = covered else {
        return ChainFact {
            exhaustive: false,
            missing: Vec::new(),
        };
    };
    let missing: Vec<String> = members
        .iter()
        .filter(|member| !tested.contains(member))
        .map(|member| ctx.name(*member))
        .collect();
    ChainFact {
        exhaustive: missing.is_empty() && !members.is_empty(),
        missing,
    }
}

/// What one checked `if` form produces, decided purely by whether a value
/// was requested at its position and whether it turned out to return on
/// every reachable path (Specification 026 section 6).
enum FlowIf {
    Value(TExpr),
    Stmt(TStmtIf),
}

/// Checks an `if` form as a block element and classifies its Specification
/// 026 section 6 flow outcome. `expected` is `Some(ty)` only when this `if`
/// occupies the one position that may supply a block's required value (the
/// last element of a value-required block, per `check_block`); every other
/// call site passes `None`, exactly like the pre-026 statement-form rule
/// that a mid-block `if` never produces a value. Each arm and `else` is
/// itself checked through `check_block` with that same `expected`, so a
/// nested `return` composes for free: an arm that returns supplies no value
/// of its own (`TBlock::result` stays `None`) and is excluded from this
/// `if`'s own move-state exits, since it never reaches the code after the
/// `if` (section 8). Returns whether every reachable arm -- and `else`, or
/// an exhaustive type-test chain standing in for one -- returns; a caller
/// uses this to keep tracking reachability and move availability past the
/// `if` itself.
fn check_if<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    form: &IfForm<'src>,
    expected: Option<Ty>,
) -> (FlowIf, bool) {
    // Specification 016 section 6.2: every arm branches from the same entry
    // state, so each is checked against a fresh snapshot rather than the
    // previous arm's leftover moves.
    let entry = ctx.move_state.clone();
    let mut arms = Vec::new();
    let mut spans = Vec::new();
    let mut exits = Vec::new();
    let mut arm_returns = Vec::new();
    for (condition, body) in &form.arms {
        ctx.move_state = entry.clone();
        let scope = env.len();
        let checked_condition = check_arm_condition(ctx, env, condition);
        let (checked_body, returns) = check_block(ctx, env, body, expected);
        env.truncate(scope);
        spans.push(condition.span());
        // Specification 026 section 8: a branch that unconditionally returns
        // never reaches the point after the `if`, so its exit move state is
        // not a real predecessor there and must not be merged into it.
        if !returns {
            exits.push(ctx.move_state.clone());
        }
        arm_returns.push(returns);
        arms.push((checked_condition, checked_body));
    }
    let fact = analyze_chain(ctx, &arms, &spans);
    let mut else_returns = true;
    let else_branch = match &form.else_branch {
        Some(body) => {
            if fact.exhaustive {
                ctx.error(
                    form.span,
                    "this type-test chain already covers every direct member, so the \
                     'else' branch is unreachable"
                        .into(),
                );
            }
            ctx.move_state = entry.clone();
            let (checked, returns) = check_block(ctx, env, body, expected);
            if !returns {
                exits.push(ctx.move_state.clone());
            }
            else_returns = returns;
            Some(checked)
        }
        None if fact.exhaustive => None,
        None => {
            if let Some(expected_ty) = expected {
                let name = ctx.name(expected_ty);
                let msg = if fact.missing.is_empty() {
                    format!(
                        "an 'if' that produces a value of type '{name}' requires an 'else' \
                         branch"
                    )
                } else {
                    format!(
                        "this type-test chain produces a value of type '{name}' without an \
                         'else' branch, but does not handle {}",
                        fact.missing.join(", ")
                    )
                };
                ctx.error(form.span, msg);
            }
            None
        }
    };
    let covers_every_path = else_branch.is_some() || fact.exhaustive;
    // An exhaustive type-test chain or an explicit `else` covers every path,
    // so no execution reaches the point after the `if` without entering one
    // of the recorded arms above. Otherwise the fall-through edge is
    // reachable too, carrying the entry state forward unchanged
    // (Specification 016 section 6.2's merge rule) -- when a value is
    // required (`expected.is_some()`) and coverage is missing, the branch
    // above already diagnosed it, but the merge still must not silently drop
    // moves for the rest of a malformed program.
    if !covers_every_path {
        exits.push(entry.clone());
    }
    let if_returns =
        covers_every_path && arm_returns.iter().all(|returns| *returns) && else_returns;
    ctx.move_state = if exits.is_empty() {
        entry
    } else {
        merge_moves(exits)
    };
    match expected {
        // Specification 026 section 6: every reachable branch returning
        // makes the `if` itself a callable return that supplies no value, so
        // its checked representation is a statement even in a value-needed
        // position.
        Some(ty) if !if_returns => (
            FlowIf::Value(TExpr::If(Box::new(TValueIf {
                arms,
                else_branch,
                exhaustive: fact.exhaustive,
                ty,
            }))),
            if_returns,
        ),
        _ => (
            FlowIf::Stmt(TStmtIf {
                arms,
                else_branch,
                exhaustive: fact.exhaustive,
            }),
            if_returns,
        ),
    }
}

fn check_defer<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    on_error: bool,
    call: &Spanned<Expr<'src>>,
    span: Span,
) -> Option<TDeferred> {
    let Expr::Call(callee, arguments) = &call.0 else {
        ctx.error(
            span,
            "a deferred action must be a direct function or method call".into(),
        );
        return None;
    };
    // A deferred call is checked now but executes only at scope exit. Preserve
    // the ordinary checked argument tree while restoring move availability so
    // a by-value argument is not consumed when the defer is merely armed.
    let move_state = ctx.move_state.clone();
    let Some(checked) = check_call(ctx, env, span, callee, arguments) else {
        ctx.move_state = move_state;
        return None;
    };
    ctx.move_state = move_state;
    let statement = match checked {
        CheckedCall::Function {
            name,
            args,
            result: None,
        } => TStmt::Call(name, args),
        CheckedCall::Method { call, result: None } => TStmt::MethodCall(call),
        CheckedCall::Statement(_) => {
            ctx.error(
                span,
                "a deferred action must be a direct function or method call".into(),
            );
            return None;
        }
        CheckedCall::Function {
            result: Some(_), ..
        }
        | CheckedCall::Method {
            result: Some(_), ..
        }
        | CheckedCall::Value(_, _) => {
            ctx.error(span, "a deferred call must not produce a value".into());
            return None;
        }
    };
    if on_error {
        let valid = matches!(ctx.callable_result, Some(Some(Ty::Sum(sum)))
        if ctx.types.sum_members(sum).iter().any(|ty| {
            matches!(ty, Ty::User(id) if ctx.types.def(*id).name() == "Error")
        }));
        if !valid {
            ctx.error(
                span,
                "'defer_on_error' is only valid in a callable whose result contains Error".into(),
            );
            return None;
        }
    }
    let consumes = consuming_roots(&statement);
    Some(TDeferred {
        on_error,
        call: statement,
        consumes,
    })
}

fn consuming_roots(statement: &TStmt) -> Vec<PlaceRoot> {
    let args = match statement {
        TStmt::Call(_, args) => args,
        TStmt::MethodCall(call) => &call.args,
        _ => return Vec::new(),
    };
    args.iter()
        .filter_map(|arg| match arg {
            TArg::Value(TExpr::Place(place, UseMode::Consume)) if place.path.is_empty() => {
                Some(place.root.clone())
            }
            _ => None,
        })
        .collect()
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
            let declared = resolve_type(ctx, ty);
            // The initializer is checked before the name is in scope, so it can
            // never refer to the variable being created.
            let (checked, value_ty) = if matches!(&value.0, Expr::List(_)) {
                check_collection_literal(ctx, env, value, declared)
            } else {
                check_expr(ctx, env, value)
            };
            // Specification 016 section 6.1: initialization is a consuming
            // context.
            let checked = mark_consumed(ctx, env, checked, value.1);
            let checked = coerce(ctx, checked, value_ty, declared, value.1);
            declare(ctx, name, *name_span, "Variable");
            env.push(Binding {
                name,
                ty: declared,
                mutable: *mutable,
                type_test_alias: false,
            });
            if is_borrowed_type(ctx, declared) {
                let sources = view_sources(ctx, &checked);
                if !sources.is_empty() {
                    ctx.view_borrows.push(ViewBorrow {
                        view_name: (*name).to_string(),
                        sources,
                        scope: ctx.cleanup_scopes.len().saturating_sub(1),
                    });
                }
            }
            TStmt::Let {
                mutable: *mutable,
                name: (*name).to_string(),
                ty: declared,
                value: checked,
            }
        }
        BlockElement::IndexedAssign { target, value } => {
            let Expr::Index(base, index) = &target.0 else {
                ctx.error(
                    target.1,
                    "only a map index can be an indexed assignment target".into(),
                );
                return TStmt::Expr(TExpr::Nil);
            };
            let Some(resolved) = (match as_place(ctx, env, base) {
                PlaceOutcome::Resolved(resolved) => Some(resolved),
                PlaceOutcome::Reported | PlaceOutcome::NotAPlace => None,
            }) else {
                return TStmt::Expr(TExpr::Nil);
            };
            if !resolved.mutable {
                ctx.error(
                    base.1,
                    format!(
                        "'{}' is not declared 'mut' and cannot be indexed-assigned",
                        ctx.place_name(&resolved.place)
                    ),
                );
            }
            let collection_ty = resolved.place.ty;
            reject_live_view_source(ctx, &resolved.place.root, target.1);
            if matches!(collection_ty, Ty::Array(_) | Ty::List(_)) {
                let elem = match collection_ty {
                    Ty::Array(id) | Ty::List(id) => match ctx.types.collection(id) {
                        CollectionDef::Array { elem, .. } | CollectionDef::List { elem } => *elem,
                        _ => unreachable!("sequence type has non-sequence metadata"),
                    },
                    _ => unreachable!("guarded by sequence collection type"),
                };
                let (checked_index, index_ty) = check_expr(ctx, env, index);
                if index_ty != Ty::Int64 {
                    ctx.mismatch(index.1, Ty::Int64, index_ty);
                }
                let (checked_value, value_ty) = check_expr(ctx, env, value);
                let checked_value = mark_consumed(ctx, env, checked_value, value.1);
                let checked_value = coerce(ctx, checked_value, value_ty, elem, value.1);
                return TStmt::SequenceIndexAssign {
                    receiver: resolved.place,
                    index: checked_index,
                    value: checked_value,
                    elem,
                };
            }
            let Ty::Map(id) = collection_ty else {
                ctx.error(
                    base.1,
                    format!(
                        "'{}' is not a map, so it cannot be indexed-assigned",
                        ctx.name(collection_ty)
                    ),
                );
                return TStmt::Expr(TExpr::Nil);
            };
            let (map_key_ty, map_value_ty) = match ctx.types.collection(id) {
                CollectionDef::Map { key, value } => (*key, *value),
                _ => unreachable!("map type has non-map metadata"),
            };
            let (checked_index, index_ty) = check_expr(ctx, env, index);
            let (checked_key, key_ty) = if map_key_ty == Ty::String {
                (
                    coerce(ctx, checked_index, index_ty, Ty::ViewByte, index.1),
                    Ty::ViewByte,
                )
            } else {
                if index_ty != map_key_ty {
                    ctx.mismatch(index.1, map_key_ty, index_ty);
                }
                (checked_index, index_ty)
            };
            let (checked_value, value_ty) = check_expr(ctx, env, value);
            let checked_value = mark_consumed(ctx, env, checked_value, value.1);
            let checked_value = coerce(ctx, checked_value, value_ty, map_value_ty, value.1);
            TStmt::Expr(TExpr::MapInsert {
                receiver: resolved.place,
                key: Box::new(checked_key),
                value: Box::new(checked_value),
                key_ty,
                value_ty: map_value_ty,
                require_existing: true,
            })
        }
        BlockElement::Assign { place, value } => {
            let target = resolve_place(ctx, env, place);
            if let Some(target) = &target {
                reject_live_view_source(ctx, &target.place.root, value.1);
            }
            let (checked, value_ty) = check_expr(ctx, env, value);
            // Specification 016 section 6.1: an assignment's right operand is
            // a consuming context.
            let checked = mark_consumed(ctx, env, checked, value.1);
            match target {
                Some(resolved) => {
                    if !resolved.mutable {
                        ctx.error(
                            place.root_span,
                            format!(
                                "'{}' is not declared 'mut' and cannot be assigned",
                                place.root
                            ),
                        );
                    }
                    // Specification 016 section 6.3: a move whose source
                    // overlaps its destination is rejected, including
                    // `value = value` and a projection assigned from its own
                    // owning root (e.g. `container.field = container`). A
                    // source with its own non-empty path is handled instead
                    // by `mark_consumed`'s subplace-move rejection above, so
                    // only a whole-root source is checked here.
                    if ctx.types.is_move_only(resolved.place.ty)
                        && let TExpr::Place(source, UseMode::Consume) = &checked
                        && source.path.is_empty()
                        && overlaps(&resolved.place, source)
                    {
                        let dest = ctx.place_name(&resolved.place);
                        let source_name = ctx.place_name(source);
                        ctx.error(
                            value.1,
                            format!(
                                "'{source_name}' and '{dest}' overlap, so this assignment \
                                 cannot destroy '{dest}' before '{source_name}' finishes moving \
                                 into it"
                            ),
                        );
                    }
                    let checked = coerce(ctx, checked, value_ty, resolved.place.ty, value.1);
                    if resolved.place.path.is_empty() && is_borrowed_type(ctx, resolved.place.ty) {
                        let root_name = resolved.place.root.to_string();
                        let sources = view_sources(ctx, &checked);
                        ctx.view_borrows
                            .retain(|borrow| borrow.view_name != root_name);
                        if !sources.is_empty() {
                            ctx.view_borrows.push(ViewBorrow {
                                view_name: root_name,
                                sources,
                                scope: ctx.cleanup_scopes.len().saturating_sub(1),
                            });
                        }
                    }
                    if resolved.place.root == PlaceRoot::SelfRef
                        && let Some(method) = ctx.current_method
                    {
                        ctx.direct_writes[method.index()] = true;
                    }
                    // Specification 016 section 6.3: the old destination is
                    // destroyed before the new value is installed, unless
                    // there is nothing live there to destroy -- either the
                    // destination is copyable, or it is a whole root that is
                    // currently moved (reinitializing a moved mutable local,
                    // the closing sentence below, installs a value where
                    // none was live). A field destination (non-empty path)
                    // always has a live value: the checker requires every
                    // field of a constructed aggregate to be initialized, so
                    // there is no partially-built aggregate a field
                    // assignment could be reaching into.
                    let drop_before = ctx.types.is_move_only(resolved.place.ty)
                        && !(resolved.place.path.is_empty()
                            && ctx.move_state.contains_key(&resolved.place.root));
                    // Specification 016 section 6.3's closing sentence:
                    // assigning the whole root installs a fresh value, so it
                    // is available again regardless of whether it was moved.
                    // A field assignment (a non-empty path) reinitializes no
                    // root and is left untouched.
                    if resolved.place.path.is_empty() {
                        ctx.move_state.remove(&resolved.place.root);
                    }
                    TStmt::Assign {
                        place: resolved.place,
                        value: checked,
                        drop_before,
                    }
                }
                None => TStmt::Expr(checked),
            }
        }
        BlockElement::While {
            condition, body, ..
        } => {
            let condition = check_condition(ctx, env, condition);
            // Specification 016 section 6.2: a `while` body is checked to a
            // fixed point. The body's own single checked pass (below) already
            // uses the pre-loop state, exactly as its first real iteration
            // would; what a naive single pass cannot see is a later
            // iteration reusing the same move once the first has already
            // consumed it. Since this checker's control flow never branches
            // on move-availability, a root's exit state as a function of its
            // entry state is always one of "unconditionally reinitialized",
            // "unconditionally moved", or "untouched" -- never a function
            // that depends on which one held going in -- so comparing this
            // one real pass's entry and exit finds every root the loop can
            // legitimately double-move without needing to re-run the body.
            let pre = ctx.move_state.clone();
            ctx.loops.push(ctx.cleanup_scopes.len());
            let (body, body_returns) = check_block(ctx, env, body, None);
            ctx.loops.pop();
            let post = ctx.move_state.clone();
            for (root, move_span) in &post {
                if !pre.contains_key(root) {
                    ctx.error(
                        *move_span,
                        format!(
                            "'{root}' is moved here, but this is inside a 'while' body, so a \
                             later iteration would find '{root}' already moved"
                        ),
                    );
                }
            }
            // The loop may run zero or more times, so the state after it must
            // hold on both the zero-iteration edge (`pre`) and the edge that
            // runs the body at least once (`post`); by the reasoning above,
            // `post` already reflects every later iteration too. When the
            // body always returns (Specification 026), it never falls
            // through to re-check the condition, so `post` is not a real
            // predecessor of "after the loop" -- only the zero-iteration edge
            // is (section 6's "no proof of nontermination" also applies in
            // reverse: a loop that always returns when entered still might
            // never be entered).
            ctx.move_state = if body_returns {
                pre
            } else {
                merge_moves(vec![pre, post])
            };
            TStmt::While { condition, body }
        }
        BlockElement::For {
            value,
            key,
            iterable,
            body,
            span,
        } => {
            let (checked_iterable, iterable_ty) = check_expr(ctx, env, iterable);
            let (value_ty, key_ty) = match iterable_ty {
                Ty::Array(id) | Ty::List(id) | Ty::View(id) => match ctx.types.collection(id) {
                    CollectionDef::Array { elem, .. }
                    | CollectionDef::List { elem }
                    | CollectionDef::View { elem } => (*elem, None),
                    _ => unreachable!("sequence iterable has non-sequence metadata"),
                },
                Ty::ViewByte => (Ty::Byte, None),
                Ty::ViewUnicode => (Ty::Unicode, None),
                Ty::Map(id) => match ctx.types.collection(id) {
                    CollectionDef::Map { key, value } => (*value, Some(*key)),
                    _ => unreachable!("map iterable has non-map metadata"),
                },
                Ty::Set(id) => match ctx.types.collection(id) {
                    CollectionDef::Set { elem } => (*elem, None),
                    _ => unreachable!("set iterable has non-set metadata"),
                },
                other => {
                    ctx.error(*span, format!("'{}' is not iterable", ctx.name(other)));
                    (Ty::Nil, None)
                }
            };
            if key.is_some() != key_ty.is_some() {
                let expected = if key_ty.is_some() {
                    "a key and value binding"
                } else {
                    "one value binding"
                };
                ctx.error(*span, format!("this iterable requires {expected}"));
            }
            if key_ty.is_some() {
                if !matches!(
                    key_ty,
                    Some(
                        Ty::Byte
                            | Ty::UInt16
                            | Ty::UInt32
                            | Ty::UInt64
                            | Ty::Int64
                            | Ty::Bool
                            | Ty::Unicode
                            | Ty::String
                    )
                ) {
                    ctx.error(
                        *span,
                        "map iteration requires a supported scalar or String key".into(),
                    );
                }
            } else if matches!(iterable_ty, Ty::Set(_))
                && !matches!(
                    value_ty,
                    Ty::Byte
                        | Ty::UInt16
                        | Ty::UInt32
                        | Ty::UInt64
                        | Ty::Int64
                        | Ty::Bool
                        | Ty::Unicode
                        | Ty::String
                )
            {
                ctx.error(
                    *span,
                    "set iteration currently supports scalar elements".into(),
                );
            }
            declare(ctx, value.0, value.1, "Loop variable");
            let key_string = key.map(|(name, name_span)| {
                declare(ctx, name, name_span, "Loop variable");
                name.to_string()
            });
            let scope = env.len();
            env.push(Binding {
                name: value.0,
                ty: value_ty,
                mutable: false,
                type_test_alias: true,
            });
            if let Some((name, _)) = key {
                env.push(Binding {
                    name,
                    ty: key_ty.unwrap_or(Ty::Nil),
                    mutable: false,
                    type_test_alias: true,
                });
            }
            ctx.loops.push(ctx.cleanup_scopes.len());
            let (checked_body, _) = check_block(ctx, env, body, None);
            ctx.loops.pop();
            env.truncate(scope);
            TStmt::For {
                value_name: value.0.to_string(),
                value_ty,
                key_name: key_string,
                key_ty,
                iterable: checked_iterable,
                collection_ty: iterable_ty,
                body: checked_body,
            }
        }
        BlockElement::Break(span) => {
            if ctx.loops.is_empty() {
                ctx.error(
                    *span,
                    "'break' is only valid inside a 'while' body or a 'for' body".into(),
                );
            }
            let cleanup = if let Some(scope_start) = ctx.loops.last().copied() {
                cleanup_for_exit(ctx, env, scope_start)
            } else {
                Vec::new()
            };
            TStmt::Break { cleanup }
        }
        BlockElement::Defer { .. } => {
            ctx.unknown = Some("defer reached statement checking instead of block checking");
            TStmt::Expr(TExpr::Nil)
        }
        BlockElement::Return(value, span) => check_return(ctx, env, value, *span),
        BlockElement::If(form) => match check_if(ctx, env, form, None).0 {
            FlowIf::Stmt(stmt_if) => TStmt::If(stmt_if),
            FlowIf::Value(_) => {
                unreachable!("check_if only produces a value when a value was requested")
            }
        },
        BlockElement::Expr(expression) => {
            if let Expr::ReturnOnError(value) = &expression.0 {
                return match check_return_on_error(ctx, env, value, expression.1, true) {
                    CheckedReturnOnError::Statement(statement) => statement,
                    CheckedReturnOnError::Expr { .. } => {
                        unreachable!("statement-form return_on_error produced a value")
                    }
                };
            }
            // A call to a declaration without a result is a call statement, not
            // an expression whose value is discarded.
            if let Expr::Call(callee, arguments) = &expression.0 {
                return match check_call(ctx, env, expression.1, callee, arguments) {
                    Some(CheckedCall::Function {
                        name,
                        args,
                        result: None,
                    }) => TStmt::Call(name, args),
                    Some(CheckedCall::Function { name, args, .. }) => {
                        TStmt::Expr(TExpr::Call(name, args))
                    }
                    Some(CheckedCall::Method { call, result: None }) => TStmt::MethodCall(call),
                    Some(CheckedCall::Method { call, .. }) => {
                        TStmt::Expr(TExpr::MethodCall(Box::new(call)))
                    }
                    Some(CheckedCall::Statement(statement)) => statement,
                    Some(CheckedCall::Value(value, _)) => TStmt::Expr(value),
                    None => TStmt::Expr(TExpr::Nil),
                };
            }
            if let Expr::GenericCall(callee, _type_args, arguments) = &expression.0 {
                return match check_call(ctx, env, expression.1, callee, arguments) {
                    Some(CheckedCall::Function {
                        name,
                        args,
                        result: None,
                    }) => TStmt::Call(name, args),
                    Some(CheckedCall::Function { name, args, .. }) => {
                        TStmt::Expr(TExpr::Call(name, args))
                    }
                    Some(CheckedCall::Method { call, result: None }) => TStmt::MethodCall(call),
                    Some(CheckedCall::Method { call, .. }) => {
                        TStmt::Expr(TExpr::MethodCall(Box::new(call)))
                    }
                    Some(CheckedCall::Statement(statement)) => statement,
                    Some(CheckedCall::Value(value, _)) => TStmt::Expr(value),
                    None => TStmt::Expr(TExpr::Nil),
                };
            }
            TStmt::Expr(check_expr(ctx, env, expression).0)
        }
    }
}

/// Checks a `return` statement (Specification 026 section 5). `value`'s
/// presence is checked against `ctx.callable_result` alone -- never the
/// syntactic kind of the immediately enclosing block -- distinguishing three
/// facts: outside every callable (`None`), a no-result callable
/// (`Some(None)`), and a result-declaring callable (`Some(Some(ty))`).
fn check_return<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    value: &Option<Spanned<Expr<'src>>>,
    span: Span,
) -> TStmt {
    let Some(expected) = ctx.callable_result else {
        ctx.error(
            span,
            "'return' is only valid inside a function or method body".into(),
        );
        // Still type-checked for its own internal errors, even though there
        // is no callable result to check it against.
        if let Some(expr) = value {
            check_expr(ctx, env, expr);
        }
        return TStmt::Return {
            value: None,
            result: None,
            cleanup: Vec::new(),
        };
    };
    let checked_value = match (expected, value) {
        (Some(result_ty), Some(expr)) => {
            let (checked, ty) = check_expr(ctx, env, expr);
            // Specification 026 section 8: the returned expression is a
            // consuming context, exactly like a value block's trailing
            // expression -- a move-only value transfers to the caller.
            let checked = mark_consumed(ctx, env, checked, expr.1);
            Some(coerce(ctx, checked, ty, result_ty, expr.1))
        }
        (Some(result_ty), None) => {
            let name = ctx.name(result_ty);
            ctx.error(
                span,
                format!(
                    "bare 'return' is not valid here; this callable declares a result of \
                     type '{name}', so 'return' needs a value"
                ),
            );
            None
        }
        (None, Some(expr)) => {
            ctx.error(
                expr.1,
                "this callable declares no result, so 'return' cannot return a value; use \
                 bare 'return' instead"
                    .into(),
            );
            check_expr(ctx, env, expr);
            None
        }
        (None, None) => None,
    };
    // Specification 026 section 8: the result above is fully materialized
    // (and, for a moved root, marked unavailable) before this cleanup plan is
    // computed, exactly like a function's own `param_drops` -- every scope
    // still open at this point, innermost first, is included in one flat
    // list because `env`'s declaration order already nests that way.
    let cleanup = cleanup_for_exit(ctx, env, 0);
    TStmt::Return {
        value: checked_value,
        result: expected,
        cleanup,
    }
}

/// Resolves a written type in a `let`. Function, method, field, and bridge
/// types are resolved once during declaration collection.
fn resolve_generic_params(ctx: &mut Ctx<'_>, params: &[Param<'_>]) -> Vec<TParam> {
    params
        .iter()
        .map(|param| TParam {
            name: param.name.to_string(),
            ty: resolve_type(ctx, &param.ty),
            mode: param.mode,
        })
        .collect()
}

fn generic_name(ctx: &Ctx<'_>, name: &str, args: &[Ty]) -> String {
    let encoded = args
        .iter()
        .map(|ty| {
            ctx.name(*ty)
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("$");
    format!("$snacc$generic${name}${encoded}")
}

fn instantiate_generic_type(
    ctx: &mut Ctx<'_>,
    name: &str,
    args: &[Ty],
    span: Span,
) -> Option<TypeId> {
    let (generic_params, body, declaration_span) = {
        let declaration = ctx.generic_types.get(name).copied()?;
        (
            declaration.generic_params.clone(),
            declaration.body.clone(),
            declaration.span,
        )
    };
    if generic_params.len() != args.len() {
        ctx.error(
            span,
            format!(
                "generic type '{name}' expects {} type arguments, found {}",
                generic_params.len(),
                args.len()
            ),
        );
        return None;
    }
    let TypeBody::Struct(fields) = body else {
        ctx.error(span, format!("generic type '{name}' must be a struct"));
        return None;
    };
    let key = format!("{name}<{args:?}>");
    if let Some(id) = ctx.types.generic_specialization(name, args) {
        return Some(id);
    }
    let display = format!(
        "{name}<{}>",
        args.iter()
            .map(|ty| ctx.name(*ty))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut chain = ctx.generic_chain.clone();
    chain.extend(ctx.generic_type_stack.iter().cloned());
    chain.push(display.clone());
    if ctx.generic_depth + ctx.generic_type_stack.len() >= MAX_SPECIALIZATION_DEPTH {
        ctx.error(
            declaration_span,
            format!(
                "generic specialization depth exceeds {MAX_SPECIALIZATION_DEPTH}; requested at {}..{}; instantiation chain: {}",
                span.start,
                span.end,
                chain.join(" -> ")
            ),
        );
        return None;
    }
    let already_in_progress = ctx.generic_type_in_progress.contains(&key);
    if !already_in_progress && ctx.specialization_count >= MAX_SPECIALIZATIONS {
        ctx.error(
            declaration_span,
            format!(
                "generic specialization limit exceeded (maximum {MAX_SPECIALIZATIONS}); requested at {}..{}; instantiation chain: {}",
                span.start,
                span.end,
                chain.join(" -> ")
            ),
        );
        return None;
    }
    let id = ctx
        .types
        .reserve_generic_struct(key.clone(), display.clone());
    if already_in_progress {
        return Some(id);
    }
    ctx.specialization_count += 1;
    ctx.generic_type_in_progress.insert(key.clone());
    ctx.generic_type_stack.push(display);
    let substitutions: HashMap<&str, Ty> = generic_params
        .iter()
        .zip(args)
        .map(|((param, _), ty)| (*param, *ty))
        .collect();
    let previous = std::mem::replace(&mut ctx.generic_subst, substitutions);
    let resolved = fields
        .iter()
        .map(|field| (field.name.to_string(), resolve_type(ctx, &field.ty)))
        .collect();
    ctx.generic_subst = previous;
    ctx.types.finish_generic_struct(id, resolved);
    ctx.generic_type_stack.pop();
    ctx.generic_type_in_progress.remove(&key);
    if let Some(cycle) = ctx.types.generic_layout_cycle(id) {
        ctx.error(
            span,
            format!(
                "Type '{}' has an infinite value layout: {}",
                ctx.types.def(id).name(),
                cycle.join(" -> ")
            ),
        );
        return None;
    }
    ctx.types.refresh_generic_properties();
    ctx.generic_type_finished.insert(key);
    Some(id)
}

fn resolve_type(ctx: &mut Ctx<'_>, ty: &Spanned<TypeRef<'_>>) -> Ty {
    match &ty.0 {
        // Specification 012 section 10: a local declaration is an ordinary
        // value-type position, so a standalone `Nil` is rejected here exactly
        // as it is during declaration collection.
        TypeRef::Builtin(TypeName::Nil) => {
            ctx.error(ty.1, types::STANDALONE_NIL.to_string());
            Ty::Nil
        }
        TypeRef::Builtin(name) => Ty::from(*name),
        TypeRef::Named(segments) => {
            let (first, first_span) = segments[0];
            if segments.len() == 1
                && let Some(ty) = ctx.generic_subst.get(first)
            {
                return *ty;
            }
            let Some(root) = ctx.types.top_level(first) else {
                ctx.error(first_span, format!("Unknown type '{first}'"));
                return Ty::Nil;
            };
            match segments.len() {
                1 => Ty::User(root),
                2 => {
                    let (member, span) = segments[1];
                    match ctx.types.member(root, member) {
                        Some(id) => Ty::User(id),
                        None => {
                            ctx.error(span, format!("Unknown type '{first}.{member}'"));
                            Ty::Nil
                        }
                    }
                }
                _ => {
                    ctx.error(
                        ty.1,
                        "a qualified type name has at most two components".into(),
                    );
                    Ty::Nil
                }
            }
        }
        TypeRef::Apply { path, args } => {
            if path.len() != 1 {
                ctx.error(
                    ty.1,
                    "generic type applications must name a top-level type".into(),
                );
                return Ty::Nil;
            }
            let resolved: Vec<Ty> = args.iter().map(|arg| resolve_type(ctx, arg)).collect();
            match ctx
                .types
                .generic_specialization(path[0].0, &resolved)
                .or_else(|| instantiate_generic_type(ctx, path[0].0, &resolved, ty.1))
            {
                Some(id) => Ty::User(id),
                None => {
                    ctx.error(
                        ty.1,
                        format!("unknown generic type specialization '{}'", ty.0),
                    );
                    Ty::Nil
                }
            }
        }
        TypeRef::Sum(members) => resolve_sum(ctx, members, ty.1),
        // Specification 016 section 4.1: the pointee resolves through
        // ordinary type resolution, exactly like declaration collection's
        // `resolve` in `types.rs`. Neither `Ref<T>` nor a no-result type has
        // a `TypeRef` spelling that reaches here, so every pointee is already
        // a storable value type.
        TypeRef::Box(inner) => {
            let pointee = resolve_type(ctx, inner);
            Ty::Box(ctx.types.intern_box(pointee))
        }
        TypeRef::View(inner) => match resolve_type(ctx, inner) {
            Ty::Byte => Ty::ViewByte,
            Ty::Unicode => Ty::ViewUnicode,
            other => Ty::View(
                ctx.types
                    .intern_collection(CollectionDef::View { elem: other }),
            ),
        },
        TypeRef::Array(inner, len) => {
            let elem = resolve_type(ctx, inner);
            Ty::Array(
                ctx.types
                    .intern_collection(CollectionDef::Array { elem, len: *len }),
            )
        }
        TypeRef::List(inner) => {
            let elem = resolve_type(ctx, inner);
            Ty::List(ctx.types.intern_collection(CollectionDef::List { elem }))
        }
        TypeRef::Map(key, value) => {
            let key = resolve_type(ctx, key);
            let value = resolve_type(ctx, value);
            Ty::Map(
                ctx.types
                    .intern_collection(CollectionDef::Map { key, value }),
            )
        }
        TypeRef::Set(inner) => {
            let elem = resolve_type(ctx, inner);
            Ty::Set(ctx.types.intern_collection(CollectionDef::Set { elem }))
        }
    }
}

/// Every built-in type keyword's segment spelling, as accepted by a type
/// test naming a direct sum member (Specification 018 section 6). Shares no
/// code with `resolve_type`'s `TypeRef::Builtin` arm because a type test
/// receives a bare name string from the parser, not a `TypeName`.
fn builtin_type_name(name: &str) -> Option<TypeName> {
    Some(match name {
        "Float64" => TypeName::Float64,
        "Int64" => TypeName::Int64,
        "Bool" => TypeName::Bool,
        "Nil" => TypeName::Nil,
        "Byte" => TypeName::Byte,
        "UInt16" => TypeName::UInt16,
        "UInt32" => TypeName::UInt32,
        "UInt64" => TypeName::UInt64,
        "Float32" => TypeName::Float32,
        _ => return None,
    })
}

/// Specification 018 section 4: resolves every syntactic member, expanding a
/// nested sum (from a parenthesized group) into its own already-flattened
/// members, then applies the member-set rules shared with declaration
/// collection (`resolve_sum` in `types.rs`). A member that itself reports an
/// error is dropped rather than kept as `resolve_type`'s `Ty::Nil` filler, so
/// a genuinely unrelated failure never masquerades as a repeated or lone
/// `Nil` member.
fn resolve_sum(ctx: &mut Ctx<'_>, members: &[Spanned<TypeRef<'_>>], span: Span) -> Ty {
    let mut raw: Vec<(Option<Ty>, Span)> = Vec::new();
    for member in members {
        // `resolve_type`'s `TypeRef::Builtin(TypeName::Nil)` arm always
        // rejects a standalone `Nil` because that arm is normally reached
        // only by one; `Nil` as a sum member is the valid, expected spelling
        // this specification adds, so it bypasses that rejection here.
        if let TypeRef::Builtin(TypeName::Nil) = &member.0 {
            raw.push((Some(Ty::Nil), member.1));
            continue;
        }
        let before = ctx.errors.len();
        let resolved = resolve_type(ctx, member);
        if ctx.errors.len() != before {
            raw.push((None, member.1));
            continue;
        }
        match resolved {
            Ty::Sum(id) => {
                for flattened in ctx.types.sum_members(id).to_vec() {
                    raw.push((Some(flattened), member.1));
                }
            }
            other => raw.push((Some(other), member.1)),
        }
    }
    let outcome = types::dedupe_sum(&raw);
    if outcome.any_unresolved {
        return Ty::Nil;
    }
    for (ty, dup_span) in &outcome.duplicates {
        let name = ctx.name(*ty);
        ctx.error(
            *dup_span,
            format!("'{name}' is repeated in this sum type; each member must be distinct"),
        );
    }
    if outcome.distinct.len() < 2 {
        let msg = if outcome.distinct == [Ty::Nil] {
            types::NIL_NEEDS_A_SUM_SIBLING
        } else {
            types::SUM_TOO_FEW_MEMBERS
        };
        ctx.error(span, msg.to_string());
        return Ty::Nil;
    }
    let mut distinct = outcome.distinct;
    distinct.sort();
    Ty::Sum(ctx.types.intern_sum(distinct))
}

/// What a checked call turned out to be.
enum CheckedCall {
    Function {
        name: String,
        args: Vec<TArg>,
        result: Option<Ty>,
    },
    Method {
        call: TMethodCall,
        result: Option<Ty>,
    },
    Statement(TStmt),
    /// A constructor, wrap, or unwrap. These always produce a value.
    Value(TExpr, Ty),
}

/// Resolves an expression used as the namespace of an associated-function
/// call. A local binding wins over a type name, matching ordinary call-head
/// resolution, and this never turns values into first-class type objects.
fn static_receiver_type(
    ctx: &Ctx<'_>,
    env: &Env<'_>,
    expression: &Spanned<Expr<'_>>,
) -> Option<Ty> {
    match &expression.0 {
        Expr::BuiltinType(name) => Some(Ty::from(*name)),
        Expr::Local(name) if !env.iter().any(|binding| binding.name == *name) => {
            ctx.types.top_level(name).map(Ty::User)
        }
        Expr::Member(base, (member, _)) => {
            let Expr::Local(root_name) = &base.0 else {
                return None;
            };
            if env.iter().any(|binding| binding.name == *root_name) {
                return None;
            }
            let root = ctx.types.top_level(root_name)?;
            ctx.types.member(root, member).map(Ty::User)
        }
        _ => None,
    }
}

/// Specification 010 section 6.1: resolves one call head. A qualified path
/// whose first component is an in-scope local, parameter, or `self` is a
/// receiver access; that test runs before any type or callable lookup. A bare
/// `name(...)` never calls a local, because Snacc has no function values.
fn check_call<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    callee: &Spanned<Expr<'src>>,
    arguments: &Spanned<Vec<Arg<'src>>>,
) -> Option<CheckedCall> {
    let args = &arguments.0;
    match &callee.0 {
        // `Int64(id)` removes one represented layer.
        Expr::BuiltinType(name) => Some(check_convert(ctx, env, span, Ty::from(*name), args)),
        // A bare `name(...)` resolves only a top-level callable or a type
        // constructor: it never reaches the local binding namespace, because
        // Snacc has no function values.
        Expr::Local(name) => {
            if ctx.sigs.contains_key(*name) {
                return check_function_call(ctx, env, span, name, args);
            }
            if let Some(id) = ctx.types.top_level(*name) {
                return Some(check_type_call(ctx, env, span, id, args, callee.1));
            }
            let msg = if env.iter().any(|binding| binding.name == *name) {
                format!(
                    "'{name}' is a variable; Snacc has no function values, so it cannot be called"
                )
            } else {
                format!("'{name}' is not callable")
            };
            ctx.error(callee.1, msg);
            None
        }
        Expr::GenericCall(callee, type_args, arguments) => {
            check_generic_function_call(ctx, env, span, callee, type_args, arguments)
        }
        Expr::Member(base, (member, member_span)) => {
            if let Expr::BuiltinType(TypeName::String) = &base.0 {
                if matches!(*member, "from_utf8" | "from_unicode") {
                    return check_string_static(ctx, env, span, member, *member_span, args);
                }
            }
            if let Some(receiver) = static_receiver_type(ctx, env, base) {
                let qualified = format!("{}.{}", ctx.name(receiver), member);
                if ctx.sigs.contains_key(&qualified) {
                    return check_function_call(ctx, env, span, &qualified, args);
                }
            }
            match as_place(ctx, env, base) {
                PlaceOutcome::Resolved(resolved) => {
                    if ctx.move_state.contains_key(&resolved.place.root) {
                        ctx.error(
                            base.1,
                            format!(
                                "'{}' is already moved, so this use is invalid",
                                resolved.place.root
                            ),
                        );
                        return None;
                    }
                    let receiver_ty = resolved.place.ty;
                    if let Some(statement) = check_list_mutation(
                        ctx,
                        env,
                        span,
                        resolved.place.clone(),
                        resolved.mutable,
                        member,
                        args,
                    ) {
                        return Some(statement);
                    }
                    if let Some(statement) = check_map_set_method(
                        ctx,
                        env,
                        span,
                        resolved.place.clone(),
                        resolved.mutable,
                        member,
                        args,
                    ) {
                        return Some(statement);
                    }
                    if let Some(call) = check_builtin_method(
                        ctx,
                        env,
                        span,
                        TExpr::Place(resolved.place.clone(), UseMode::Copy),
                        receiver_ty,
                        member,
                        args,
                    ) {
                        return Some(call);
                    }
                    let self_rooted = resolved.place.root == PlaceRoot::SelfRef;
                    let description = resolved.place.root.to_string();
                    check_method_call(
                        ctx,
                        env,
                        span,
                        TReceiver::Place(resolved.place),
                        receiver_ty,
                        resolved.mutable,
                        self_rooted,
                        description,
                        member,
                        *member_span,
                        args,
                    )
                }
                PlaceOutcome::Reported => None,
                PlaceOutcome::NotAPlace => {
                    // `Union.Member(...)` names a member constructor.
                    if let Expr::Local(first) = &base.0
                        && let Some(root) = ctx.types.top_level(first)
                    {
                        return match ctx.types.member(root, member) {
                            Some(id) => Some(check_type_call(ctx, env, span, id, args, callee.1)),
                            None => {
                                let owner = ctx.types.def(root).name().to_string();
                                ctx.error(
                                    *member_span,
                                    format!("'{owner}' has no member type '{member}'"),
                                );
                                None
                            }
                        };
                    }
                    // Otherwise a method on a computed value.
                    let before = ctx.errors.len();
                    let (value, ty) = check_expr(ctx, env, base);
                    if ctx.errors.len() != before {
                        return None;
                    }
                    if matches!(
                        ty,
                        Ty::String
                            | Ty::ViewByte
                            | Ty::ViewUnicode
                            | Ty::Array(_)
                            | Ty::List(_)
                            | Ty::View(_)
                            | Ty::Map(_)
                            | Ty::Set(_)
                    ) {
                        return check_builtin_method(ctx, env, span, value, ty, member, args);
                    }
                    check_method_call(
                        ctx,
                        env,
                        span,
                        TReceiver::Value(value, ty),
                        ty,
                        false,
                        false,
                        "a temporary".into(),
                        member,
                        *member_span,
                        args,
                    )
                }
            }
        }
        _ => {
            ctx.error(
                callee.1,
                "only calling a function, type, or method by name is supported".into(),
            );
            None
        }
    }
}

/// Checks the closed map/set operation surface. Keys remain restricted by the
/// collection contract, while values may be any fully storable non-borrowed
/// type; the backend selects typed or opaque-byte runtime entry points from
/// the checked value type.
fn check_map_set_method<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    receiver: Place,
    mutable: bool,
    name: &str,
    args: &[Arg<'src>],
) -> Option<CheckedCall> {
    let (kind, key_ty, value_ty) = match receiver.ty {
        Ty::Map(id) => match ctx.types.collection(id) {
            CollectionDef::Map { key, value } => (0u8, *key, Some(*value)),
            _ => unreachable!("map type has non-map metadata"),
        },
        Ty::Set(id) => match ctx.types.collection(id) {
            CollectionDef::Set { elem } => (1u8, *elem, None),
            _ => unreachable!("set type has non-set metadata"),
        },
        _ => return None,
    };
    let supported_key = matches!(
        key_ty,
        Ty::Byte
            | Ty::UInt16
            | Ty::UInt32
            | Ty::UInt64
            | Ty::Int64
            | Ty::Bool
            | Ty::Unicode
            | Ty::String
    );
    if !supported_key {
        ctx.error(
            span,
            if kind == 0 {
                "Map operations require a supported scalar or String key".into()
            } else {
                "Set operations require a supported scalar or String element".into()
            },
        );
        return Some(CheckedCall::Value(TExpr::Nil, Ty::Nil));
    }
    match (kind, name) {
        (0, "insert") => {
            reject_named_args(ctx, "Map.insert", args);
            if !mutable {
                ctx.error(span, "Map.insert requires a mutable map receiver".into());
            }
            if args.len() != 2 {
                ctx.error(
                    span,
                    format!("Map.insert expects 2 arguments, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Value(TExpr::Bool(false), Ty::Bool));
            }
            let (key, key_found) = check_expr(ctx, env, &args[0].value);
            if key_found != key_ty {
                ctx.mismatch(args[0].value.1, key_ty, key_found);
            }
            let key = mark_consumed(ctx, env, key, args[0].value.1);
            let (value, found) = check_expr(ctx, env, &args[1].value);
            let value_ty = value_ty.expect("map operation has a value type");
            let value = coerce(ctx, value, found, value_ty, args[1].value.1);
            Some(CheckedCall::Value(
                TExpr::MapInsert {
                    receiver,
                    key: Box::new(key),
                    value: Box::new(value),
                    key_ty,
                    value_ty,
                    require_existing: false,
                },
                Ty::Bool,
            ))
        }
        (0, "contains") | (0, "delete") | (0, "take") => {
            reject_named_args(ctx, "a map operation", args);
            if args.len() != 1 {
                ctx.error(
                    span,
                    format!("Map.{name} expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Value(TExpr::Bool(false), Ty::Bool));
            }
            let (key, found) = check_expr(ctx, env, &args[0].value);
            let (query, query_ty) = if key_ty == Ty::String {
                (
                    coerce(ctx, key, found, Ty::ViewByte, args[0].value.1),
                    Ty::ViewByte,
                )
            } else {
                if found != key_ty {
                    ctx.mismatch(args[0].value.1, key_ty, found);
                }
                (key, found)
            };
            if name != "contains" && !mutable {
                ctx.error(span, format!("Map.{name} requires a mutable map receiver"));
            }
            if name == "contains" {
                Some(CheckedCall::Value(
                    TExpr::MapContains {
                        receiver: Box::new(TExpr::Place(receiver.clone(), UseMode::Copy)),
                        key: Box::new(query),
                        key_ty: query_ty,
                        value_ty: value_ty.expect("map operation has a value type"),
                    },
                    Ty::Bool,
                ))
            } else if name == "delete" {
                Some(CheckedCall::Value(
                    TExpr::MapDelete {
                        receiver,
                        key: Box::new(query),
                        key_ty: query_ty,
                        value_ty: value_ty.expect("map operation has a value type"),
                    },
                    Ty::Bool,
                ))
            } else {
                Some(CheckedCall::Value(
                    TExpr::MapTake {
                        receiver,
                        key: Box::new(query),
                        key_ty: query_ty,
                        value_ty: value_ty.expect("map operation has a value type"),
                    },
                    value_ty.expect("map operation has a value type"),
                ))
            }
        }
        (0, "clear") => {
            reject_named_args(ctx, "Map.clear", args);
            if !mutable {
                ctx.error(span, "Map.clear requires a mutable map receiver".into());
            }
            if !args.is_empty() {
                ctx.error(
                    span,
                    format!("Map.clear expects no arguments, found {}", args.len()),
                );
            }
            Some(CheckedCall::Statement(TStmt::MapClear {
                receiver,
                key_ty,
                value_ty: value_ty.expect("map operation has a value type"),
            }))
        }
        (0, "reserve") => {
            reject_named_args(ctx, "Map.reserve", args);
            if !mutable {
                ctx.error(span, "Map.reserve requires a mutable map receiver".into());
            }
            if args.len() != 1 {
                ctx.error(
                    span,
                    format!("Map.reserve expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Statement(TStmt::MapReserve {
                    receiver,
                    minimum: TExpr::Num(NumLiteral::Int(0)),
                    key_ty,
                    value_ty: value_ty.expect("map operation has a value type"),
                }));
            }
            let (minimum, minimum_ty) = check_expr(ctx, env, &args[0].value);
            if minimum_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, minimum_ty);
            }
            Some(CheckedCall::Statement(TStmt::MapReserve {
                receiver,
                minimum,
                key_ty,
                value_ty: value_ty.expect("map operation has a value type"),
            }))
        }
        (1, "insert") | (1, "delete") | (1, "contains") => {
            reject_named_args(ctx, "a set operation", args);
            if args.len() != 1 {
                ctx.error(
                    span,
                    format!("Set.{name} expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Value(TExpr::Bool(false), Ty::Bool));
            }
            let (value, found) = check_expr(ctx, env, &args[0].value);
            let value = if key_ty == Ty::String {
                if found != Ty::String {
                    ctx.mismatch(args[0].value.1, Ty::String, found);
                }
                if name == "insert" {
                    mark_consumed(ctx, env, value, args[0].value.1)
                } else {
                    value
                }
            } else {
                if found != key_ty {
                    ctx.mismatch(args[0].value.1, key_ty, found);
                }
                value
            };
            if name != "contains" && !mutable {
                ctx.error(span, format!("Set.{name} requires a mutable set receiver"));
            }
            if name == "contains" || name == "delete" {
                let query = if key_ty == Ty::String {
                    coerce(ctx, value, found, Ty::ViewByte, args[0].value.1)
                } else {
                    value
                };
                if name == "contains" {
                    Some(CheckedCall::Value(
                        TExpr::SetContains {
                            receiver: Box::new(TExpr::Place(receiver, UseMode::Copy)),
                            value: Box::new(query),
                            elem: key_ty,
                        },
                        Ty::Bool,
                    ))
                } else {
                    Some(CheckedCall::Value(
                        TExpr::SetDelete {
                            receiver,
                            value: Box::new(query),
                            elem: key_ty,
                        },
                        Ty::Bool,
                    ))
                }
            } else if name == "insert" {
                Some(CheckedCall::Value(
                    TExpr::SetInsert {
                        receiver,
                        value: Box::new(value),
                        elem: key_ty,
                    },
                    Ty::Bool,
                ))
            } else {
                unreachable!("set operation name was checked above")
            }
        }
        (1, "clear") => {
            reject_named_args(ctx, "Set.clear", args);
            if !mutable {
                ctx.error(span, "Set.clear requires a mutable set receiver".into());
            }
            Some(CheckedCall::Statement(TStmt::SetClear {
                receiver,
                elem: key_ty,
            }))
        }
        (1, "reserve") => {
            reject_named_args(ctx, "Set.reserve", args);
            if !mutable {
                ctx.error(span, "Set.reserve requires a mutable set receiver".into());
            }
            if args.len() != 1 {
                ctx.error(
                    span,
                    format!("Set.reserve expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Statement(TStmt::SetReserve {
                    receiver,
                    minimum: TExpr::Num(NumLiteral::Int(0)),
                    elem: key_ty,
                }));
            }
            let (minimum, minimum_ty) = check_expr(ctx, env, &args[0].value);
            if minimum_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, minimum_ty);
            }
            Some(CheckedCall::Statement(TStmt::SetReserve {
                receiver,
                minimum,
                elem: key_ty,
            }))
        }
        _ => None,
    }
}

/// Checks the deliberately closed mutation surface currently exposed for
/// lists. The runtime path copies scalar bytes, so accepting only scalar
/// elements keeps `push` and `clear` correct until element destruction and
/// general collection ownership are implemented.
fn check_list_mutation<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    receiver: Place,
    mutable: bool,
    name: &str,
    args: &[Arg<'src>],
) -> Option<CheckedCall> {
    let Ty::List(id) = receiver.ty else {
        return None;
    };
    let elem = match ctx.types.collection(id) {
        CollectionDef::List { elem } => *elem,
        _ => unreachable!("list type has non-list metadata"),
    };
    match name {
        "push" => {
            reject_named_args(ctx, "a List.push call", args);
            if !mutable {
                ctx.error(span, "List.push requires a mutable list receiver".into());
            }
            if args.len() != 1 {
                ctx.error(
                    span,
                    format!("List.push expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Statement(TStmt::ListPush {
                    receiver,
                    value: TExpr::Nil,
                    elem,
                }));
            }
            let (value, value_ty) = check_expr(ctx, env, &args[0].value);
            let value = mark_consumed(ctx, env, value, args[0].value.1);
            let value = coerce(ctx, value, value_ty, elem, args[0].value.1);
            Some(CheckedCall::Statement(TStmt::ListPush {
                receiver,
                value,
                elem,
            }))
        }
        "pop" => {
            reject_named_args(ctx, "a List.pop call", args);
            if !mutable {
                ctx.error(span, "List.pop requires a mutable list receiver".into());
            }
            if !args.is_empty() {
                ctx.error(
                    span,
                    format!("List.pop expects no arguments, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
            }
            Some(CheckedCall::Value(TExpr::ListPop { receiver, elem }, elem))
        }
        "remove" => {
            reject_named_args(ctx, "a List.remove call", args);
            if !mutable {
                ctx.error(span, "List.remove requires a mutable list receiver".into());
            }
            if args.len() != 1 {
                ctx.error(
                    span,
                    format!("List.remove expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Value(
                    TExpr::ListRemove {
                        receiver,
                        index: Box::new(TExpr::Num(NumLiteral::Int(0))),
                        elem,
                    },
                    elem,
                ));
            }
            let (index, index_ty) = check_expr(ctx, env, &args[0].value);
            if index_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, index_ty);
            }
            Some(CheckedCall::Value(
                TExpr::ListRemove {
                    receiver,
                    index: Box::new(index),
                    elem,
                },
                elem,
            ))
        }
        "insert" => {
            reject_named_args(ctx, "a List.insert call", args);
            if !mutable {
                ctx.error(span, "List.insert requires a mutable list receiver".into());
            }
            if args.len() != 2 {
                ctx.error(
                    span,
                    format!("List.insert expects 2 arguments, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Statement(TStmt::ListInsert {
                    receiver,
                    index: TExpr::Num(NumLiteral::Int(0)),
                    value: TExpr::Nil,
                    elem,
                }));
            }
            let (index, index_ty) = check_expr(ctx, env, &args[0].value);
            if index_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, index_ty);
            }
            let (value, value_ty) = check_expr(ctx, env, &args[1].value);
            let value = mark_consumed(ctx, env, value, args[1].value.1);
            let value = coerce(ctx, value, value_ty, elem, args[1].value.1);
            Some(CheckedCall::Statement(TStmt::ListInsert {
                receiver,
                index,
                value,
                elem,
            }))
        }
        "reserve" => {
            reject_named_args(ctx, "a List.reserve call", args);
            if !mutable {
                ctx.error(span, "List.reserve requires a mutable list receiver".into());
            }
            if args.len() != 1 {
                ctx.error(
                    span,
                    format!("List.reserve expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
                return Some(CheckedCall::Statement(TStmt::ListReserve {
                    receiver,
                    minimum: TExpr::Num(NumLiteral::Int(0)),
                    elem,
                }));
            }
            let (minimum, minimum_ty) = check_expr(ctx, env, &args[0].value);
            if minimum_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, minimum_ty);
            }
            Some(CheckedCall::Statement(TStmt::ListReserve {
                receiver,
                minimum,
                elem,
            }))
        }
        "clear" => {
            reject_named_args(ctx, "a List.clear call", args);
            if !mutable {
                ctx.error(span, "List.clear requires a mutable list receiver".into());
            }
            if !args.is_empty() {
                ctx.error(
                    span,
                    format!("List.clear expects no arguments, found {}", args.len()),
                );
                for arg in args {
                    check_expr(ctx, env, &arg.value);
                }
            }
            Some(CheckedCall::Statement(TStmt::ListClear { receiver, elem }))
        }
        _ => None,
    }
}

fn check_builtin_method<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    receiver: TExpr,
    receiver_ty: Ty,
    name: &str,
    args: &[Arg<'src>],
) -> Option<CheckedCall> {
    if !matches!(
        receiver_ty,
        Ty::String
            | Ty::ViewByte
            | Ty::ViewUnicode
            | Ty::Array(_)
            | Ty::List(_)
            | Ty::View(_)
            | Ty::Map(_)
            | Ty::Set(_)
    ) {
        return None;
    }
    match name {
        "bytes" if receiver_ty == Ty::String && args.is_empty() => Some(CheckedCall::Value(
            TExpr::ViewFromString(Box::new(receiver), Ty::ViewByte),
            Ty::ViewByte,
        )),
        "unicode" if receiver_ty == Ty::String && args.is_empty() => Some(CheckedCall::Value(
            TExpr::ViewFromString(Box::new(receiver), Ty::ViewUnicode),
            Ty::ViewUnicode,
        )),
        "length" if matches!(receiver_ty, Ty::ViewByte | Ty::ViewUnicode) && args.is_empty() => {
            Some(CheckedCall::Value(
                TExpr::ViewLength(Box::new(receiver), receiver_ty),
                Ty::Int64,
            ))
        }
        "length"
            if matches!(
                receiver_ty,
                Ty::Array(_) | Ty::List(_) | Ty::View(_) | Ty::Map(_) | Ty::Set(_)
            ) && args.is_empty() =>
        {
            Some(CheckedCall::Value(
                TExpr::CollectionLength(Box::new(receiver)),
                Ty::Int64,
            ))
        }
        "is_empty"
            if matches!(
                receiver_ty,
                Ty::Array(_) | Ty::List(_) | Ty::View(_) | Ty::Map(_) | Ty::Set(_)
            ) && args.is_empty() =>
        {
            Some(CheckedCall::Value(
                TExpr::CollectionIsEmpty(Box::new(receiver)),
                Ty::Bool,
            ))
        }
        "capacity" if matches!(receiver_ty, Ty::List(_)) && args.is_empty() => Some(
            CheckedCall::Value(TExpr::CollectionCapacity(Box::new(receiver)), Ty::Int64),
        ),
        "view" if matches!(receiver_ty, Ty::Array(_) | Ty::List(_)) && args.is_empty() => {
            let elem = match receiver_ty {
                Ty::Array(id) | Ty::List(id) => match ctx.types.collection(id) {
                    CollectionDef::Array { elem, .. } | CollectionDef::List { elem } => *elem,
                    _ => unreachable!("sequence type has non-sequence metadata"),
                },
                _ => unreachable!("guarded by receiver type"),
            };
            let view = Ty::View(ctx.types.intern_collection(CollectionDef::View { elem }));
            Some(CheckedCall::Value(
                TExpr::CollectionView(Box::new(receiver), view),
                view,
            ))
        }
        "at" if receiver_ty == Ty::ViewByte && args.len() == 1 && args[0].name.is_none() => {
            let (index, index_ty) = check_expr(ctx, env, &args[0].value);
            if index_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, index_ty);
            }
            let sum = ctx.types.intern_sum(vec![Ty::Nil, Ty::Byte]);
            Some(CheckedCall::Value(
                TExpr::ViewAt(Box::new(receiver), Box::new(index), Ty::ViewByte, sum),
                Ty::Sum(sum),
            ))
        }
        "scalar_at"
            if receiver_ty == Ty::ViewUnicode && args.len() == 1 && args[0].name.is_none() =>
        {
            let (index, index_ty) = check_expr(ctx, env, &args[0].value);
            if index_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, index_ty);
            }
            let sum = ctx.types.intern_sum(vec![Ty::Nil, Ty::Unicode]);
            Some(CheckedCall::Value(
                TExpr::ViewAt(Box::new(receiver), Box::new(index), Ty::ViewUnicode, sum),
                Ty::Sum(sum),
            ))
        }
        "slice"
            if matches!(receiver_ty, Ty::ViewByte | Ty::ViewUnicode)
                && args.len() == 2
                && args.iter().all(|arg| arg.name.is_none()) =>
        {
            let (start, start_ty) = check_expr(ctx, env, &args[0].value);
            let (end, end_ty) = check_expr(ctx, env, &args[1].value);
            if start_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, start_ty);
            }
            if end_ty != Ty::Int64 {
                ctx.mismatch(args[1].value.1, Ty::Int64, end_ty);
            }
            let sum = ctx.types.intern_sum(vec![Ty::Nil, receiver_ty]);
            Some(CheckedCall::Value(
                TExpr::ViewSlice {
                    value: Box::new(receiver),
                    start: Box::new(start),
                    end: Box::new(end),
                    view_ty: receiver_ty,
                    sum,
                },
                Ty::Sum(sum),
            ))
        }
        "slice"
            if matches!(receiver_ty, Ty::Array(_) | Ty::List(_) | Ty::View(_))
                && args.len() == 2
                && args.iter().all(|arg| arg.name.is_none()) =>
        {
            let elem = match receiver_ty {
                Ty::Array(id) | Ty::List(id) | Ty::View(id) => match ctx.types.collection(id) {
                    CollectionDef::Array { elem, .. }
                    | CollectionDef::List { elem }
                    | CollectionDef::View { elem } => *elem,
                    _ => unreachable!("sequence view metadata is not a sequence"),
                },
                _ => unreachable!("guarded by collection receiver type"),
            };
            let view_ty = match receiver_ty {
                Ty::View(_) => receiver_ty,
                Ty::Array(_) | Ty::List(_) => {
                    Ty::View(ctx.types.intern_collection(CollectionDef::View { elem }))
                }
                _ => unreachable!("guarded by collection receiver type"),
            };
            let (start, start_ty) = check_expr(ctx, env, &args[0].value);
            let (end, end_ty) = check_expr(ctx, env, &args[1].value);
            if start_ty != Ty::Int64 {
                ctx.mismatch(args[0].value.1, Ty::Int64, start_ty);
            }
            if end_ty != Ty::Int64 {
                ctx.mismatch(args[1].value.1, Ty::Int64, end_ty);
            }
            let sum = ctx.types.intern_sum(vec![Ty::Nil, view_ty]);
            Some(CheckedCall::Value(
                TExpr::CollectionSlice {
                    value: Box::new(receiver),
                    start: Box::new(start),
                    end: Box::new(end),
                    view_ty,
                    sum,
                    elem,
                },
                Ty::Sum(sum),
            ))
        }
        "clone" if receiver_ty == Ty::String && args.is_empty() => Some(CheckedCall::Value(
            TExpr::StringClone(Box::new(receiver)),
            Ty::String,
        )),
        "concat" if receiver_ty == Ty::String && args.len() == 1 && args[0].name.is_none() => {
            let (value, ty) = check_expr(ctx, env, &args[0].value);
            let accepted = matches!(
                ty,
                Ty::String
                    | Ty::ViewUnicode
                    | Ty::Unicode
                    | Ty::Byte
                    | Ty::UInt16
                    | Ty::UInt32
                    | Ty::UInt64
                    | Ty::Int64
                    | Ty::Float32
                    | Ty::Float64
                    | Ty::Bool
            );
            if !accepted {
                ctx.error(
                    args[0].value.1,
                    format!(
                        "String.concat does not accept '{}'; expected a text or scalar part",
                        ctx.name(ty)
                    ),
                );
            }
            let mut parts = match receiver {
                TExpr::StringConcat(parts) => parts,
                value => vec![TStringPart {
                    value,
                    ty: Ty::String,
                }],
            };
            parts.push(TStringPart { value, ty });
            Some(CheckedCall::Value(TExpr::StringConcat(parts), Ty::String))
        }
        "clone" | "concat" => {
            ctx.error(span, format!("String.{name} called with invalid arguments"));
            Some(CheckedCall::Value(TExpr::Nil, Ty::String))
        }
        _ => None,
    }
}

fn check_string_static<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    name: &str,
    member_span: Span,
    args: &[Arg<'src>],
) -> Option<CheckedCall> {
    if args.len() != 1 || args[0].name.is_some() {
        ctx.error(
            member_span,
            format!("String.{name} expects one positional view argument"),
        );
        return Some(CheckedCall::Value(TExpr::Nil, Ty::String));
    }
    let (value, ty) = check_expr(ctx, env, &args[0].value);
    match (name, ty) {
        ("from_unicode", Ty::ViewUnicode) => Some(CheckedCall::Value(
            TExpr::StringFromUnicode(Box::new(value)),
            Ty::String,
        )),
        ("from_utf8", Ty::ViewByte) => {
            let sum = ctx.types.intern_sum(vec![Ty::Nil, Ty::String]);
            Some(CheckedCall::Value(
                TExpr::StringFromUtf8(Box::new(value), sum),
                Ty::Sum(sum),
            ))
        }
        (_, found) => {
            ctx.error(
                span,
                format!("String.{name} received unsupported argument type '{}', expected a matching view", ctx.name(found)),
            );
            Some(CheckedCall::Value(TExpr::Nil, Ty::String))
        }
    }
}

/// Rejects the named-argument form outside struct construction.
fn reject_named_args(ctx: &mut Ctx<'_>, what: &str, args: &[Arg<'_>]) {
    for arg in args {
        if let Some((name, span)) = arg.name {
            ctx.error(
                span,
                format!(
                    "named argument '{name}' is not valid for {what}; named arguments \
                     are only used to construct a struct"
                ),
            );
        }
    }
}

fn check_function_call<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    name: &str,
    args: &[Arg<'src>],
) -> Option<CheckedCall> {
    let signature = ctx.sigs.get(name).cloned()?;
    reject_named_args(ctx, "a function call", args);
    if signature.params.len() != args.len() {
        ctx.error(
            span,
            format!(
                "'{name}' called with wrong number of arguments (expected {}, found {})",
                signature.params.len(),
                args.len()
            ),
        );
    }
    let values = check_args(
        ctx,
        env,
        args,
        &signature.params,
        name,
        None,
        ctx.externs.contains(name),
    );
    Some(CheckedCall::Function {
        name: name.to_string(),
        args: values,
        result: signature.result,
    })
}

fn check_generic_function_call<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    callee: &Spanned<Expr<'src>>,
    type_args: &Spanned<Vec<Spanned<TypeRef<'src>>>>,
    args: &Spanned<Vec<Arg<'src>>>,
) -> Option<CheckedCall> {
    let Expr::Local(name) = &callee.0 else {
        ctx.error(
            callee.1,
            "generic calls require an unqualified top-level function name".into(),
        );
        return None;
    };
    let function = {
        let generic_funcs = &ctx.generic_funcs;
        generic_funcs.get(name).copied()
    };
    let Some(function) = function else {
        let concrete: Vec<Ty> = type_args.0.iter().map(|ty| resolve_type(ctx, ty)).collect();
        if let Some(id) = instantiate_generic_type(ctx, name, &concrete, type_args.1) {
            return Some(check_type_call(ctx, env, span, id, &args.0, callee.1));
        }
        ctx.error(
            callee.1,
            format!("'{name}' is not a generic function or type"),
        );
        return None;
    };
    if function.generic_params.len() != type_args.0.len() {
        ctx.error(
            type_args.1,
            format!(
                "generic function '{name}' expects {} type arguments, found {}",
                function.generic_params.len(),
                type_args.0.len()
            ),
        );
        for arg in &args.0 {
            check_expr(ctx, env, &arg.value);
        }
        return None;
    }
    let concrete: Vec<Ty> = type_args.0.iter().map(|ty| resolve_type(ctx, ty)).collect();
    let substitutions: HashMap<&str, Ty> = function
        .generic_params
        .iter()
        .zip(&concrete)
        .map(|((param, _), ty)| (*param, *ty))
        .collect();
    let previous = std::mem::replace(&mut ctx.generic_subst, substitutions.clone());
    let params = resolve_generic_params(ctx, &function.args);
    let result = function.ret.as_ref().map(|ty| resolve_type(ctx, ty));
    ctx.generic_subst = previous;
    let mangled = generic_name(ctx, name, &concrete);
    let display = format!(
        "{name}<{}>",
        concrete
            .iter()
            .map(|ty| ctx.name(*ty))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut chain = ctx.generic_chain.clone();
    chain.push(display);
    let signature = FuncSig {
        params: params.clone(),
        result,
    };
    ctx.sigs.insert(mangled.clone(), signature);
    if !ctx.generic_seen.contains(&mangled) {
        if ctx.generic_depth >= MAX_SPECIALIZATION_DEPTH {
            ctx.error(
                span,
                format!(
                    "generic specialization depth exceeds {MAX_SPECIALIZATION_DEPTH}; instantiation chain: {}",
                    chain.join(" -> ")
                ),
            );
        } else if ctx.specialization_count >= MAX_SPECIALIZATIONS {
            ctx.error(
                span,
                format!(
                    "generic specialization limit exceeded (maximum {MAX_SPECIALIZATIONS}); instantiation chain: {}",
                    chain.join(" -> ")
                ),
            );
        } else {
            ctx.generic_seen.insert(mangled.clone());
            ctx.specialization_count += 1;
            ctx.generic_queue.push(GenericRequest {
                name: (*name).to_string(),
                args: concrete,
                depth: ctx.generic_depth + 1,
                use_span: span,
                chain,
            });
        }
    }
    reject_named_args(ctx, "a function call", &args.0);
    let values = check_args(ctx, env, &args.0, &params, &mangled, None, false);
    Some(CheckedCall::Function {
        name: mangled,
        args: values,
        result,
    })
}

/// Specification 011 sections 6.1-6.4. Arguments are processed left to right;
/// a value argument is checked and coerced as before, and a reference argument
/// resolves to exactly one place, which is then validated for mutability, exact
/// referent type, and disjointness from every other reference in the same call.
fn check_args<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    args: &[Arg<'src>],
    params: &[TParam],
    callee: &str,
    receiver: Option<&Place>,
    bridge: bool,
) -> Vec<TArg> {
    let mut checked = Vec::with_capacity(args.len());
    let mut references: Vec<(String, Place, Span)> = Vec::new();
    let mut moves: Vec<(String, Place, Span)> = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        // An argument with no parameter is already reported as an arity error;
        // it is still checked so its own diagnostics are not swallowed.
        let Some(param) = params.get(index) else {
            checked.push(TArg::Value(check_expr(ctx, env, &arg.value).0));
            continue;
        };
        match param.mode {
            ParamMode::Value => {
                let (value, ty) = check_expr(ctx, env, &arg.value);
                // Specification 016 section 6.1: a by-value argument is a
                // consuming context.
                let value = mark_consumed(ctx, env, value, arg.value.1);
                // Specification 016 section 7.2's closing sentence: a
                // borrowed allocation cannot be simultaneously moved, so a
                // whole-root move-only argument joins the same overlap
                // check as a reference argument in this same call.
                if let TExpr::Place(place, UseMode::Consume) = &value
                    && place.path.is_empty()
                    && ctx.types.is_move_only(place.ty)
                {
                    moves.push((param.name.clone(), place.clone(), arg.value.1));
                }
                let value = if bridge {
                    coerce_bridge_view(ctx, value, ty, param.ty, arg.value.1)
                } else {
                    coerce(ctx, value, ty, param.ty, arg.value.1)
                };
                checked.push(TArg::Value(value));
            }
            ParamMode::Reference => match check_reference_arg(ctx, env, arg, param, callee) {
                Some(place) => {
                    references.push((param.name.clone(), place.clone(), arg.value.1));
                    checked.push(TArg::Reference(place));
                }
                None => checked.push(TArg::Value(TExpr::Nil)),
            },
        }
    }
    reject_overlap(ctx, &references, &moves, receiver);
    checked
}

/// Resolves one reference argument. A reference parameter forwarded from an
/// enclosing signature needs no special case: it is already a mutable root of
/// its referent type, so it reborrows through this same path.
fn check_reference_arg<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    arg: &Arg<'src>,
    param: &TParam,
    callee: &str,
) -> Option<Place> {
    let span = arg.value.1;
    match as_place(ctx, env, &arg.value) {
        PlaceOutcome::Resolved(resolved) => {
            if !resolved.mutable {
                let root = resolved.place.root.to_string();
                let name = &param.name;
                ctx.error(
                    span,
                    format!(
                        "'{root}' is not declared 'mut', so it cannot be passed to the \
                         reference parameter '{name}' of '{callee}'"
                    ),
                );
            }
            let mut place = resolved.place;
            // Specification 016 section 7.2: a `Box<T>` argument place lends
            // its pointee to a `Ref<T>` parameter automatically. A `Box<T>`
            // argument binding to a declared `Ref<Box<T>>` parameter instead
            // is already an exact match below and needs no special case --
            // the expected parameter type alone disambiguates, with no new
            // inference. The overlap check afterward still compares this
            // place's unchanged root and path, so two lends of the same
            // allocation (as the box or as its pointee) still overlap.
            if place.ty != param.ty
                && let Ty::Box(id) = place.ty
                && ctx.types.box_pointee(id) == param.ty
            {
                place.ty = param.ty;
            } else if place.ty != param.ty {
                // Specification 011 sections 6.1 and 9: exactly `T`. Neither
                // the `Int64`-to-`Float64` widening nor represented-type
                // equivalence applies, because the callee addresses the
                // caller's own storage.
                let expected = ctx.name(param.ty);
                let found = ctx.name(place.ty);
                let name = &param.name;
                ctx.error(
                    span,
                    format!(
                        "reference parameter '{name}' of '{callee}' requires a place of \
                         exactly type '{expected}', found '{found}'"
                    ),
                );
            }
            // Handing a `self`-rooted place to a reference parameter may write
            // it, exactly as an assignment to that place would, so it feeds the
            // same receiver-write fixed point (Specification 010 section 19
            // phase 4). Without this a caller could pass an immutable receiver.
            if place.root == PlaceRoot::SelfRef
                && let Some(method) = ctx.current_method
            {
                ctx.direct_writes[method.index()] = true;
            }
            Some(place)
        }
        PlaceOutcome::Reported => None,
        PlaceOutcome::NotAPlace => {
            // A malformed argument reports its own error first; only a
            // well-formed value -- a literal, a call result, arithmetic -- needs
            // the "not a place" diagnostic.
            let before = ctx.errors.len();
            check_expr(ctx, env, &arg.value);
            if ctx.errors.len() == before {
                let expected = ctx.name(param.ty);
                let name = &param.name;
                ctx.error(
                    span,
                    format!(
                        "reference parameter '{name}' of '{callee}' requires an initialized \
                         mutable place of type '{expected}', but this argument is a value \
                         with no storage"
                    ),
                );
            }
            None
        }
    }
}

/// Specification 011 section 3: two places overlap when they are identical or
/// one is reached by selecting fields from the other. Two paths that first
/// differ at sibling field indices are disjoint.
fn overlaps(left: &Place, right: &Place) -> bool {
    if left.root != right.root {
        return false;
    }
    let shared = left.path.len().min(right.path.len());
    left.path[..shared] == right.path[..shared]
}

/// Specification 011 section 6.4: every pair of reference arguments, and each
/// reference argument against an addressable method receiver. A temporary
/// receiver has independent storage and cannot overlap a caller place.
/// Specification 016 section 7.2's closing sentence extends this through
/// boxes: `moves` is every whole-root move-only by-value argument in the same
/// call (Specification 016 Task B's `check_args` collects it alongside
/// `references`), and none of them may overlap a reference argument either --
/// a borrowed allocation cannot also be moved out from under the call.
fn reject_overlap(
    ctx: &mut Ctx<'_>,
    references: &[(String, Place, Span)],
    moves: &[(String, Place, Span)],
    receiver: Option<&Place>,
) {
    for (index, (name, place, span)) in references.iter().enumerate() {
        if let Some(receiver) = receiver
            && overlaps(place, receiver)
        {
            let argument = ctx.place_name(place);
            let subject = ctx.place_name(receiver);
            ctx.error(
                *span,
                format!(
                    "reference argument '{argument}' for parameter '{name}' overlaps the \
                     receiver '{subject}', which the method may access through 'self'"
                ),
            );
        }
        for (other_name, other, _) in &references[index + 1..] {
            if overlaps(place, other) {
                let left = ctx.place_name(place);
                let right = ctx.place_name(other);
                ctx.error(
                    *span,
                    format!(
                        "reference arguments '{left}' and '{right}' overlap, so parameters \
                         '{name}' and '{other_name}' cannot both have exclusive access"
                    ),
                );
            }
        }
        for (moved_name, moved_place, _) in moves {
            if overlaps(place, moved_place) {
                let argument = ctx.place_name(place);
                let moved = ctx.place_name(moved_place);
                ctx.error(
                    *span,
                    format!(
                        "reference argument '{argument}' for parameter '{name}' overlaps the \
                         moved argument '{moved}' for parameter '{moved_name}', so it cannot \
                         be borrowed in the same call that moves it"
                    ),
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_method_call<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    receiver: TReceiver,
    receiver_ty: Ty,
    mutable_root: bool,
    self_rooted: bool,
    description: String,
    name: &str,
    name_span: Span,
    args: &[Arg<'src>],
) -> Option<CheckedCall> {
    // Specification 016 section 4.3: a method call on a box automatically
    // dereferences to the pointee before resolution, exactly like field
    // access. `receiver_ty` itself is left as the caller's static type;
    // `TReceiver::Place`'s place is unaffected, so lowering (Task C) still
    // knows the receiver storage is boxed.
    let receiver_ty = deref_box(&ctx.types, receiver_ty);
    let Ty::User(id) = receiver_ty else {
        let owner = ctx.name(receiver_ty);
        ctx.error(name_span, format!("'{owner}' has no method '{name}'"));
        return None;
    };
    let Some(method) = ctx.method_index.get(&(id, name.to_string())).copied() else {
        let owner = ctx.types.def(id).name().to_string();
        let msg = if ctx.types.field(id, name).is_some() {
            format!("'{owner}.{name}' is a field, not a method, so it cannot be called")
        } else {
            format!("'{owner}' has no method '{name}'")
        };
        ctx.error(name_span, msg);
        return None;
    };
    reject_named_args(ctx, "a method call", args);
    let signature_params = ctx.method_sigs[method.index()].params.clone();
    let result = ctx.method_sigs[method.index()].result;
    if signature_params.len() != args.len() {
        let qualified = ctx.method_name(method);
        ctx.error(
            span,
            format!(
                "'{qualified}' called with wrong number of arguments (expected {}, found {})",
                signature_params.len(),
                args.len()
            ),
        );
    }
    // Specification 011 section 6.4: an addressable receiver participates in
    // overlap checking for the complete call, whether or not the method writes.
    let receiver_place = match &receiver {
        TReceiver::Place(place) => Some(place.clone()),
        TReceiver::Value(..) => None,
    };
    let qualified = ctx.method_name(method);
    let values = check_args(
        ctx,
        env,
        args,
        &signature_params,
        &qualified,
        receiver_place.as_ref(),
        false,
    );
    // The effect is not known yet, so the receiver check is deferred to the
    // fixed point (Specification 010 section 19 phase 4).
    ctx.receiver_calls.push(ReceiverCall {
        method,
        mutable_root: mutable_root && matches!(receiver, TReceiver::Place(_)),
        receiver: description,
        span,
    });
    if self_rooted && let Some(caller) = ctx.current_method {
        ctx.effect_edges.push((caller, method));
    }
    Some(CheckedCall::Method {
        call: TMethodCall {
            receiver,
            method,
            args: values,
        },
        result,
    })
}

/// Calling a user type: struct or member construction, or one represented
/// wrap/unwrap layer.
fn check_type_call<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    id: TypeId,
    args: &[Arg<'src>],
    head_span: Span,
) -> CheckedCall {
    enum Head {
        Represented,
        Union(String),
        Fields,
    }
    let head = match ctx.types.def(id) {
        TypeDef::Represented { .. } => Head::Represented,
        TypeDef::Union { name, .. } => Head::Union(name.clone()),
        TypeDef::Struct { .. } | TypeDef::UnionMember { .. } => Head::Fields,
    };
    match head {
        Head::Represented => check_convert(ctx, env, span, Ty::User(id), args),
        Head::Union(name) => {
            ctx.error(
                head_span,
                format!(
                    "'{name}' is a union; construction names one member type, not the \
                     union itself"
                ),
            );
            for arg in args {
                check_expr(ctx, env, &arg.value);
            }
            CheckedCall::Value(TExpr::Nil, Ty::Nil)
        }
        Head::Fields => check_construct(ctx, env, span, id, args),
    }
}

/// Specification 010 section 7.2: exactly one layer, exact immediate type, no
/// named arguments, and no general numeric cast.
fn check_convert<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    to: Ty,
    args: &[Arg<'src>],
) -> CheckedCall {
    reject_named_args(ctx, "a represented-type conversion", args);
    let [arg] = args else {
        let name = ctx.name(to);
        ctx.error(
            span,
            format!(
                "'{name}' converts exactly one positional value, but {} were supplied",
                args.len()
            ),
        );
        for arg in args {
            check_expr(ctx, env, &arg.value);
        }
        return CheckedCall::Value(TExpr::Nil, to);
    };
    let (value, from) = check_expr(ctx, env, &arg.value);
    let wraps = matches!(to, Ty::User(id) if ctx.types.represented_target(id) == Some(from));
    let unwraps = matches!(from, Ty::User(id) if ctx.types.represented_target(id) == Some(to));
    if !wraps && !unwraps {
        let target = ctx.name(to);
        let found = ctx.name(from);
        let msg = match to {
            Ty::User(id) => {
                let target_ty = ctx.types.represented_target(id);
                let immediate = target_ty
                    .map(|ty| ctx.name(ty))
                    .unwrap_or_else(|| target.clone());
                format!(
                    "'{target}' wraps exactly its immediate representation '{immediate}', \
                     found '{found}'"
                )
            }
            _ => format!(
                "'{target}' unwraps exactly one value of a type represented by \
                 '{target}', found '{found}'"
            ),
        };
        ctx.error(arg.value.1, msg);
    }
    CheckedCall::Value(
        TExpr::Represent {
            value: Box::new(value),
            ty: to,
        },
        to,
    )
}

/// Specification 010 sections 8.2-8.3: named fields, each exactly once, checked
/// and evaluated in written order.
fn check_construct<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    span: Span,
    id: TypeId,
    args: &[Arg<'src>],
) -> CheckedCall {
    let type_name = ctx.types.def(id).name().to_string();
    let fields: Vec<(String, Ty)> = ctx
        .types
        .def(id)
        .fields()
        .expect("a constructed type has fields")
        .to_vec();
    if fields.is_empty() {
        if !args.is_empty() {
            ctx.error(
                span,
                format!("'{type_name}' has no fields, so it is constructed with '()'"),
            );
            for arg in args {
                check_expr(ctx, env, &arg.value);
            }
        }
        return CheckedCall::Value(
            TExpr::Construct {
                type_id: id,
                fields: Vec::new(),
            },
            Ty::User(id),
        );
    }

    let mut checked: Vec<(usize, TExpr)> = Vec::new();
    let mut filled = vec![false; fields.len()];
    for arg in args {
        let Some((name, name_span)) = arg.name else {
            ctx.error(
                arg.value.1,
                format!(
                    "'{type_name}' requires named fields; positional construction of a \
                     non-empty struct is invalid"
                ),
            );
            check_expr(ctx, env, &arg.value);
            continue;
        };
        let Some(index) = fields.iter().position(|(field, _)| field == name) else {
            ctx.error(name_span, format!("'{type_name}' has no field '{name}'"));
            check_expr(ctx, env, &arg.value);
            continue;
        };
        // Arguments evaluate left to right in written order; the destination
        // index travels with the value so lowering can store in field order.
        let (value, ty) = check_expr(ctx, env, &arg.value);
        if filled[index] {
            ctx.error(
                name_span,
                format!("Field '{type_name}.{name}' is supplied more than once"),
            );
            continue;
        }
        filled[index] = true;
        let expected = fields[index].1;
        // Specification 016 section 6.1: an aggregate constructor argument is
        // a consuming context.
        let value = mark_consumed(ctx, env, value, arg.value.1);
        let value = coerce(ctx, value, ty, expected, arg.value.1);
        checked.push((index, value));
    }

    let missing: Vec<&str> = fields
        .iter()
        .zip(&filled)
        .filter(|(_, supplied)| !**supplied)
        .map(|((name, _), _)| name.as_str())
        .collect();
    if !missing.is_empty() {
        ctx.error(
            span,
            format!(
                "'{type_name}' is missing field {}",
                missing
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    CheckedCall::Value(
        TExpr::Construct {
            type_id: id,
            fields: checked,
        },
        Ty::User(id),
    )
}

/// Specification 010 section 12: the subject is a place with a union type and
/// the tested type is one direct member of that union. The caller
/// (`check_arm_condition`) has already resolved `place` and confirmed `union`
/// names a union.
fn check_type_test<'src>(
    ctx: &mut Ctx<'src>,
    test: &TypeTest<'src>,
    place: Place,
    union: TypeId,
) -> Option<TTypeTest> {
    let subject_name = ctx.name(place.ty);
    let member = match test.member.as_slice() {
        [(name, _)] => match ctx.types.member(union, name) {
            Some(id) => id,
            None => {
                let msg = if ctx.types.top_level(name) == Some(union) {
                    format!(
                        "'{}' already has type '{subject_name}', so this test is always true",
                        test.place
                    )
                } else {
                    format!("'{name}' is not a direct member of '{subject_name}'")
                };
                ctx.error(test.member_span, msg);
                return None;
            }
        },
        [(first, first_span), (second, _)] => {
            let Some(root) = ctx.types.top_level(first) else {
                ctx.error(*first_span, format!("Unknown type '{first}'"));
                return None;
            };
            let Some(id) = ctx.types.member(root, second) else {
                let owner = ctx.types.def(root).name().to_string();
                ctx.error(
                    test.member_span,
                    format!("'{owner}' has no member type '{second}'"),
                );
                return None;
            };
            if root != union {
                let named = ctx.types.def(id).name().to_string();
                ctx.error(
                    test.member_span,
                    format!("'{named}' is not a direct member of '{subject_name}'"),
                );
                return None;
            }
            id
        }
        _ => {
            ctx.error(
                test.member_span,
                "a tested member name has at most two components".into(),
            );
            return None;
        }
    };

    let (tag, nil) = match ctx.types.def(member) {
        TypeDef::UnionMember { tag, nil, .. } => (*tag, *nil),
        _ => (0, false),
    };
    let binding = match test.binding {
        Some((name, name_span)) if nil => {
            let _ = name;
            ctx.error(
                name_span,
                "'Nil' carries no value, so it cannot be bound by a type test".into(),
            );
            None
        }
        Some((name, name_span)) => {
            declare(ctx, name, name_span, "Binding");
            Some((name.to_string(), Ty::User(member)))
        }
        None => None,
    };
    Some(TTypeTest {
        place,
        member,
        tag,
        binding,
    })
}

/// Specification 018 section 6: the tested member is one direct member of an
/// inline sum, named by exactly one segment -- a built-in keyword or a
/// top-level type name. A test target is never a two-segment path: an inline
/// sum's members are never namespaced, and testing a member inside a
/// named-union member requires a second test after binding that union. The
/// caller (`check_arm_condition`) has already resolved `place`.
fn check_sum_type_test<'src>(
    ctx: &mut Ctx<'src>,
    test: &TypeTest<'src>,
    place: Place,
    sum: SumId,
) -> Option<TSumTypeTest> {
    let subject_name = ctx.name(place.ty);
    let [(name, name_span)] = test.member.as_slice() else {
        ctx.error(
            test.member_span,
            "a type test on an inline sum names exactly one direct member; testing a \
             member inside a named-union member requires a second test after binding \
             that union"
                .into(),
        );
        return None;
    };
    let member = match builtin_type_name(name) {
        Some(builtin) => Ty::from(builtin),
        None => match ctx.types.top_level(name) {
            Some(id) => Ty::User(id),
            None => {
                ctx.error(*name_span, format!("Unknown type '{name}'"));
                return None;
            }
        },
    };
    if !ctx.types.sum_members(sum).contains(&member) {
        ctx.error(
            *name_span,
            format!("'{name}' is not a direct member of '{subject_name}'"),
        );
        return None;
    }
    let binding = match test.binding {
        Some((name, name_span)) if member == Ty::Nil => {
            let _ = name;
            ctx.error(
                name_span,
                "'Nil' carries no value, so it cannot be bound by a type test".into(),
            );
            None
        }
        Some((name, name_span)) => {
            declare(ctx, name, name_span, "Binding");
            Some((name.to_string(), member))
        }
        None => None,
    };
    Some(TSumTypeTest {
        place,
        sum,
        member,
        binding,
    })
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
                NumLiteral::F64(_) => Ty::Float64,
                NumLiteral::U8(_) => Ty::Byte,
                NumLiteral::U16(_) => Ty::UInt16,
                NumLiteral::U32(_) => Ty::UInt32,
                NumLiteral::U64(_) => Ty::UInt64,
                NumLiteral::F32(_) => Ty::Float32,
            };
            (TExpr::Num(*literal), ty)
        }
        Expr::Value(Value::Bool(value)) => (TExpr::Bool(*value), Ty::Bool),
        Expr::Value(Value::Nil) => (TExpr::Nil, Ty::Nil),
        Expr::Value(Value::Str(value)) => (TExpr::StringLiteral(value.clone()), Ty::String),
        Expr::Value(Value::Unicode(value)) => (TExpr::Unicode(*value), Ty::Unicode),
        Expr::Interpolated(parts) => {
            let mut checked_parts = Vec::with_capacity(parts.len());
            for part in parts {
                let (part, part_ty) = match part {
                    crate::syntax::ast::StringPart::Literal(text) => {
                        (TExpr::StringLiteral(text.clone()), Ty::String)
                    }
                    crate::syntax::ast::StringPart::Expression(expression) => {
                        check_expr(ctx, env, expression)
                    }
                };
                let accepted = matches!(
                    part_ty,
                    Ty::String
                        | Ty::ViewUnicode
                        | Ty::Unicode
                        | Ty::Byte
                        | Ty::UInt16
                        | Ty::UInt32
                        | Ty::UInt64
                        | Ty::Int64
                        | Ty::Float32
                        | Ty::Float64
                        | Ty::Bool
                );
                if !accepted {
                    ctx.error(
                        span,
                        format!(
                            "string interpolation does not accept '{}'; expected a text or scalar part",
                            ctx.name(part_ty)
                        ),
                    );
                }
                checked_parts.push(TStringPart {
                    value: part,
                    ty: part_ty,
                });
            }
            (TExpr::StringConcat(checked_parts), Ty::String)
        }
        Expr::List(items) => {
            for item in items {
                check_expr(ctx, env, item);
            }
            ctx.error(span, "lists are not supported by the AOT backend".into());
            (TExpr::Nil, Ty::Nil)
        }
        Expr::MapNew(key, value) => {
            let key = resolve_type(ctx, key);
            let value = resolve_type(ctx, value);
            let id = ctx
                .types
                .intern_collection(CollectionDef::Map { key, value });
            (TExpr::CollectionNew(Ty::Map(id)), Ty::Map(id))
        }
        Expr::SetNew(inner) => {
            let elem = resolve_type(ctx, inner);
            let id = ctx.types.intern_collection(CollectionDef::Set { elem });
            (TExpr::CollectionNew(Ty::Set(id)), Ty::Set(id))
        }
        Expr::SelfRef => match ctx.self_ty {
            Some(ty) => (
                TExpr::Place(
                    Place {
                        root: PlaceRoot::SelfRef,
                        root_ty: ty,
                        path: Vec::new(),
                        ty,
                    },
                    UseMode::Copy,
                ),
                ty,
            ),
            None => {
                ctx.error(span, "'self' is only valid inside a method body".into());
                (TExpr::Nil, Ty::Nil)
            }
        },
        Expr::BuiltinType(name) => {
            ctx.error(
                span,
                format!("'{name}' is a type name; a type name alone is never a value"),
            );
            (TExpr::Nil, Ty::Nil)
        }
        Expr::Local(name) => {
            for binding in env.iter().rev() {
                if binding.name == *name {
                    let root = PlaceRoot::Local((*name).to_string());
                    if ctx.move_state.contains_key(&root) {
                        ctx.error(
                            span,
                            format!("'{root}' is already moved, so this use is invalid"),
                        );
                    }
                    return (
                        TExpr::Place(
                            Place {
                                root,
                                root_ty: binding.ty,
                                path: Vec::new(),
                                ty: binding.ty,
                            },
                            UseMode::Copy,
                        ),
                        binding.ty,
                    );
                }
            }
            if ctx.sigs.contains_key(*name) {
                ctx.error(
                    span,
                    format!("'{name}' is a function; functions cannot be used as values"),
                );
            } else if ctx.types.top_level(name).is_some() {
                ctx.error(
                    span,
                    format!("'{name}' is a type name; a type name alone is never a value"),
                );
            } else {
                ctx.error(span, format!("No such variable '{name}' in scope"));
            }
            (TExpr::Nil, Ty::Nil)
        }
        Expr::Member(base, (field, field_span)) => {
            match as_place(ctx, env, expression) {
                PlaceOutcome::Resolved(resolved) => {
                    if ctx.move_state.contains_key(&resolved.place.root) {
                        ctx.error(
                            expression.1,
                            format!(
                                "'{}' is already moved, so this use is invalid",
                                resolved.place.root
                            ),
                        );
                    }
                    let ty = resolved.place.ty;
                    (TExpr::Place(resolved.place, UseMode::Copy), ty)
                }
                PlaceOutcome::Reported => (TExpr::Nil, Ty::Nil),
                PlaceOutcome::NotAPlace => {
                    // A qualified type path is not a value on its own.
                    if let Expr::Local(first) = &base.0
                        && let Some(root) = ctx.types.top_level(first)
                    {
                        let owner = ctx.types.def(root).name().to_string();
                        let msg = if ctx.types.member(root, field).is_some() {
                            format!(
                                "'{owner}.{field}' is a type name; a type name alone is never a value"
                            )
                        } else {
                            format!("'{owner}' has no member type '{field}'")
                        };
                        ctx.error(span, msg);
                        return (TExpr::Nil, Ty::Nil);
                    }
                    let before = ctx.errors.len();
                    let (value, raw_base_ty) = check_expr(ctx, env, base);
                    if ctx.errors.len() != before {
                        return (TExpr::Nil, Ty::Nil);
                    }
                    // Specification 016 section 4.3: automatic dereference
                    // applies here too, since a fresh `box(...)` value (not a
                    // place) can still be the base of a field chain.
                    // `raw_base_ty` (un-dereferenced) is kept for the checked
                    // node itself, so lowering knows how many box layers to
                    // peel; `base_ty` (dereferenced) is used only to resolve
                    // the field here.
                    let base_ty = deref_box(&ctx.types, raw_base_ty);
                    let Ty::User(id) = base_ty else {
                        let owner = ctx.name(base_ty);
                        ctx.error(
                            *field_span,
                            format!("'{owner}' is not a struct, so it has no field '{field}'"),
                        );
                        return (TExpr::Nil, Ty::Nil);
                    };
                    let Some((index, ty)) = ctx.types.field(id, field) else {
                        let owner = ctx.types.def(id).name().to_string();
                        let msg = if ctx.method_index.contains_key(&(id, (*field).to_string())) {
                            format!(
                                "'{owner}.{field}' is a method; a method requires a receiver call"
                            )
                        } else if ctx.types.def(id).fields().is_none() {
                            format!("'{owner}' is not a struct, so it has no field '{field}'")
                        } else {
                            format!("'{owner}' has no field '{field}'")
                        };
                        ctx.error(*field_span, msg);
                        return (TExpr::Nil, Ty::Nil);
                    };
                    (
                        TExpr::FieldRead {
                            base: Box::new(value),
                            base_ty: raw_base_ty,
                            index,
                            ty,
                        },
                        ty,
                    )
                }
            }
        }
        Expr::Index(base, index) => {
            let (collection, collection_ty) = check_expr(ctx, env, base);
            let (checked_index, index_ty) = check_expr(ctx, env, index);
            if let Ty::Map(id) = collection_ty {
                let (key_ty, value_ty) = match ctx.types.collection(id) {
                    CollectionDef::Map { key, value } => (*key, *value),
                    _ => unreachable!("map type has non-map metadata"),
                };
                if ctx.types.is_move_only(value_ty) {
                    ctx.error(
                        span,
                        "an indexed map read cannot move a move-only value out of its map; use take()"
                            .into(),
                    );
                    return (TExpr::Nil, Ty::Nil);
                }
                let (key, query_ty) = if key_ty == Ty::String {
                    (
                        coerce(ctx, checked_index, index_ty, Ty::ViewByte, index.1),
                        Ty::ViewByte,
                    )
                } else {
                    if index_ty != key_ty {
                        ctx.mismatch(index.1, key_ty, index_ty);
                    }
                    (checked_index, index_ty)
                };
                return (
                    TExpr::MapIndex {
                        receiver: Box::new(collection),
                        key: Box::new(key),
                        key_ty: query_ty,
                        value_ty,
                    },
                    value_ty,
                );
            }
            if index_ty != Ty::Int64 {
                ctx.mismatch(index.1, Ty::Int64, index_ty);
            }
            let elem = match collection_ty {
                Ty::Array(id) | Ty::List(id) | Ty::View(id) => match ctx.types.collection(id) {
                    CollectionDef::Array { elem, .. }
                    | CollectionDef::List { elem }
                    | CollectionDef::View { elem } => Some(*elem),
                    _ => None,
                },
                Ty::ViewByte => Some(Ty::Byte),
                _ => None,
            };
            let Some(elem) = elem else {
                ctx.error(
                    base.1,
                    format!(
                        "'{}' is not an indexable collection",
                        ctx.name(collection_ty)
                    ),
                );
                return (TExpr::Nil, Ty::Nil);
            };
            if ctx.types.is_move_only(elem) {
                ctx.error(
                    span,
                    "an indexed read cannot move a move-only element out of its collection".into(),
                );
            }
            (
                TExpr::CollectionIndex {
                    collection: Box::new(collection),
                    index: Box::new(checked_index),
                    collection_ty,
                    elem,
                },
                elem,
            )
        }
        Expr::Unary(UnaryOp::Not, value) => {
            let (value, ty) = check_expr(ctx, env, value);
            if ty != Ty::Bool {
                ctx.error(
                    span,
                    format!("'!' requires a Bool operand, found '{}'", ctx.name(ty)),
                );
            }
            (TExpr::Not(Box::new(value)), Ty::Bool)
        }
        Expr::ReturnOnError(value) => {
            let CheckedReturnOnError::Expr { value, ty } =
                check_return_on_error(ctx, env, value, span, false)
            else {
                unreachable!("expression-form return_on_error produced a statement")
            };
            (value, ty)
        }
        Expr::Binary(left, op, right) => {
            let (left, left_ty) = check_expr(ctx, env, left);
            let (right, right_ty) = check_expr(ctx, env, right);
            match op {
                BinaryOp::And | BinaryOp::Or => {
                    if left_ty != Ty::Bool {
                        ctx.error(
                            span,
                            format!(
                                "'{}' requires Bool operands, found '{}' and '{}'",
                                if matches!(op, BinaryOp::And) {
                                    "and"
                                } else {
                                    "or"
                                },
                                ctx.name(left_ty),
                                ctx.name(right_ty)
                            ),
                        );
                    } else if right_ty != Ty::Bool {
                        ctx.error(
                            span,
                            format!(
                                "'{}' requires Bool operands, found '{}' and '{}'",
                                if matches!(op, BinaryOp::And) {
                                    "and"
                                } else {
                                    "or"
                                },
                                ctx.name(left_ty),
                                ctx.name(right_ty)
                            ),
                        );
                    }
                    let logical = if matches!(op, BinaryOp::And) {
                        LogicalOp::And
                    } else {
                        LogicalOp::Or
                    };
                    (
                        TExpr::Logical(Box::new(left), logical, Box::new(right)),
                        Ty::Bool,
                    )
                }
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    let operation = match op {
                        BinaryOp::Add => ArithOp::Add,
                        BinaryOp::Sub => ArithOp::Sub,
                        BinaryOp::Mul => ArithOp::Mul,
                        _ => ArithOp::Div,
                    };
                    // A rejected pair keeps its own operand types rather than
                    // being coerced to a guessed one, so one mixed-type
                    // expression reports one diagnostic.
                    let Some(ty) = operand_numeric(left_ty, right_ty) else {
                        ctx.operands(span, "arithmetic", left_ty, right_ty);
                        return (
                            TExpr::Arith(Box::new(left), operation, Box::new(right), left_ty),
                            left_ty,
                        );
                    };
                    let left = coerce(ctx, left, left_ty, ty, span);
                    let right = coerce(ctx, right, right_ty, ty, span);
                    let value = TExpr::Arith(Box::new(left), operation, Box::new(right), ty);
                    reject_known_nan(ctx, &value, ty, span);
                    (value, ty)
                }
                BinaryOp::Eq | BinaryOp::NotEq => {
                    let operation = if matches!(op, BinaryOp::Eq) {
                        CmpOp::Eq
                    } else {
                        CmpOp::NotEq
                    };
                    // Specification 012 section 10: `nil == nil` has no union
                    // operand to take its type from.
                    if left_ty == Ty::Nil && right_ty == Ty::Nil {
                        ctx.error(span, types::CONTEXTLESS_NIL.to_string());
                        return (
                            TExpr::Cmp(Box::new(left), operation, Box::new(right), Ty::Nil),
                            Ty::Bool,
                        );
                    }
                    // Equality joins the `Int64`/`Float64` promotion pair; a
                    // contextual `nil` joins one Nil-containing union; every
                    // other type compares only against itself.
                    let operand_ty = match common_numeric(left_ty, right_ty) {
                        Some(ty) => ty,
                        None if left_ty == right_ty => left_ty,
                        None => match nil_union(ctx, left_ty, right_ty) {
                            Some(ty) => ty,
                            None => {
                                ctx.mismatch(span, left_ty, right_ty);
                                return (
                                    TExpr::Cmp(Box::new(left), operation, Box::new(right), left_ty),
                                    Ty::Bool,
                                );
                            }
                        },
                    };
                    if !ctx.types.supports_equality(operand_ty) {
                        let name = ctx.name(operand_ty);
                        ctx.error(
                            span,
                            format!(
                                "'{name}' does not support equality because one of the \
                                 types it contains does not"
                            ),
                        );
                    }
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
                        _ => CmpOp::GreaterEq,
                    };
                    let Some(operand_ty) = operand_numeric(left_ty, right_ty) else {
                        ctx.operands(span, "ordered comparison", left_ty, right_ty);
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
        Expr::Call(callee, arguments) => {
            let Some(call) = check_call(ctx, env, span, callee, arguments) else {
                return (TExpr::Nil, Ty::Nil);
            };
            match call {
                CheckedCall::Value(value, ty) => (value, ty),
                CheckedCall::Function {
                    name,
                    args,
                    result: Some(ty),
                } => (TExpr::Call(name, args), ty),
                CheckedCall::Function { name, .. } => {
                    ctx.error(
                        span,
                        format!(
                            "'{name}' declares no result, so its call cannot be used as a value"
                        ),
                    );
                    (TExpr::Nil, Ty::Nil)
                }
                CheckedCall::Method {
                    call,
                    result: Some(ty),
                } => (TExpr::MethodCall(Box::new(call)), ty),
                CheckedCall::Method { call, .. } => {
                    let name = ctx.method_name(call.method);
                    ctx.error(
                        span,
                        format!(
                            "'{name}' declares no result, so its call cannot be used as a value"
                        ),
                    );
                    (TExpr::Nil, Ty::Nil)
                }
                CheckedCall::Statement(_) => {
                    ctx.error(span, "a list mutation does not produce a value".into());
                    (TExpr::Nil, Ty::Nil)
                }
            }
        }
        Expr::GenericCall(callee, type_args, arguments) => {
            let Some(call) =
                check_generic_function_call(ctx, env, span, callee, type_args, arguments)
            else {
                return (TExpr::Nil, Ty::Nil);
            };
            match call {
                CheckedCall::Function {
                    name,
                    args,
                    result: Some(ty),
                } => (TExpr::Call(name, args), ty),
                CheckedCall::Function { name, .. } => {
                    ctx.error(
                        span,
                        format!(
                            "'{name}' declares no result, so its call cannot be used as a value"
                        ),
                    );
                    (TExpr::Nil, Ty::Nil)
                }
                CheckedCall::Value(value, ty) => (value, ty),
                CheckedCall::Method {
                    call,
                    result: Some(ty),
                } => (TExpr::MethodCall(Box::new(call)), ty),
                CheckedCall::Method { .. } | CheckedCall::Statement(_) => {
                    ctx.error(span, "generic call does not produce a value".into());
                    (TExpr::Nil, Ty::Nil)
                }
            }
        }
        Expr::Print(value) => {
            let before = ctx.errors.len();
            let (value, ty) = check_expr(ctx, env, value);
            // Specification 012 section 10 and 12: there is no standalone `Nil`
            // value and no `snacc_print_nil` import, so `print(nil)` is a
            // context-free `nil`. Only report when the operand checked cleanly;
            // `Ty::Nil` is also this checker's error-recovery type.
            if ty == Ty::Nil && ctx.errors.len() == before {
                ctx.error(span, types::CONTEXTLESS_NIL.to_string());
            }
            // Specification 010 section 14: printing a user-defined type is a
            // separate future feature, not a silent no-op. Specification 018
            // section 8 extends the same restriction to a whole inline sum:
            // a program decomposes it with `is` before printing a member.
            match ty {
                Ty::User(_) => {
                    let name = ctx.name(ty);
                    ctx.error(
                        span,
                        format!(
                            "'print' does not support the user-defined type '{name}'; print a \
                             scalar field or unwrap a represented scalar"
                        ),
                    );
                }
                Ty::Sum(_) => {
                    let name = ctx.name(ty);
                    ctx.error(
                        span,
                        format!(
                            "'print' does not support the inline sum type '{name}'; decompose \
                             it with 'is' and print the bound member"
                        ),
                    );
                }
                // Specification 016 section 8.3: direct printing of a box is
                // unsupported initially.
                Ty::Box(_) => {
                    let name = ctx.name(ty);
                    ctx.error(
                        span,
                        format!("'print' does not support the box type '{name}'"),
                    );
                }
                _ => {}
            }
            (TExpr::Print(Box::new(value), ty), ty)
        }
        Expr::Box(operand) => {
            // Specification 016 section 4.2: the operand is an ordinary
            // expression, evaluated exactly once; its checked type becomes
            // the pointee `T`. Every checked expression already has a
            // storable value type (`Ref<T>` and a no-result type are never
            // an expression's type), so no separate "storable pointee"
            // validation is needed here.
            let (value, pointee) = check_expr(ctx, env, operand);
            if is_borrowed_type(ctx, pointee) {
                ctx.error(
                    operand.1,
                    format!(
                        "'{}' is borrowed and cannot be stored in a Box",
                        ctx.name(pointee)
                    ),
                );
            }
            // Specification 016 section 6.1: allocating a box transfers its
            // operand's complete value into the new allocation exactly like
            // an aggregate constructor argument does, so a move-only operand
            // is a consuming use here too -- otherwise the same already-boxed
            // place could be boxed again, producing two owners of one
            // allocation.
            let value = mark_consumed(ctx, env, value, operand.1);
            let ty = Ty::Box(ctx.types.intern_box(pointee));
            (TExpr::Box(Box::new(value), ty), ty)
        }
    }
}

fn check_collection_literal<'src>(
    ctx: &mut Ctx<'src>,
    env: &mut Env<'src>,
    expression: &Spanned<Expr<'src>>,
    expected: Ty,
) -> (TExpr, Ty) {
    let Expr::List(items) = &expression.0 else {
        unreachable!("collection literal helper received a non-list expression")
    };
    let (elem, length, is_array) = match expected {
        Ty::Array(id) => match ctx.types.collection(id) {
            CollectionDef::Array { elem, len } => (*elem, *len, true),
            _ => unreachable!("array type has non-array collection metadata"),
        },
        Ty::List(id) => match ctx.types.collection(id) {
            CollectionDef::List { elem } => (*elem, 0, false),
            _ => unreachable!("list type has non-list collection metadata"),
        },
        _ => {
            ctx.error(
                expression.1,
                "a collection literal requires an expected Array<T, N> or List<T> type".into(),
            );
            for item in items {
                check_expr(ctx, env, item);
            }
            return (TExpr::Nil, Ty::Nil);
        }
    };
    if is_array && items.len() != length as usize {
        ctx.error(
            expression.1,
            format!(
                "array literal has {} elements, but the expected array length is {length}",
                items.len()
            ),
        );
    }
    let mut checked = Vec::with_capacity(items.len());
    for item in items {
        let (value, value_ty) = check_expr(ctx, env, item);
        let value = mark_consumed(ctx, env, value, item.1);
        checked.push(coerce(ctx, value, value_ty, elem, item.1));
    }
    (
        TExpr::CollectionLiteral {
            ty: expected,
            items: checked,
        },
        expected,
    )
}

/// The aggregate type -- a named union or an inline sum -- a `value == nil`
/// comparison takes, when exactly one operand is `nil` and the other directly
/// contains `Nil`.
fn nil_union(ctx: &Ctx<'_>, left: Ty, right: Ty) -> Option<Ty> {
    let pair = |aggregate: Ty, other: Ty| match (aggregate, other) {
        (Ty::User(id), Ty::Nil) => ctx.types.member(id, "Nil").map(|_| aggregate),
        (Ty::Sum(id), Ty::Nil) => ctx
            .types
            .sum_members(id)
            .contains(&Ty::Nil)
            .then_some(aggregate),
        _ => None,
    };
    pair(left, right).or_else(|| pair(right, left))
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

    fn assert_rejected_by_parser(source: &str) {
        assert!(
            crate::parse(source).is_err(),
            "expected a parse error for: {source}"
        );
    }

    #[test]
    fn parser_recovery_nodes_are_compiler_bugs_after_parsing() {
        let span: Span = (0..0).into();
        let mut funcs = HashMap::new();
        funcs.insert(
            "recovered",
            crate::syntax::ast::Func {
                generic_params: Vec::new(),
                args: Vec::new(),
                ret: Some((TypeRef::Builtin(TypeName::Nil), span)),
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
            types: Vec::new(),
            methods: Vec::new(),
            statics: Vec::new(),
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
            "extern rust \"snacc_user_bad-name\" fun bad(): Int64\nprint(0)",
            "valid Rust identifiers",
        );
    }

    #[test]
    fn accepts_bridge_symbols_with_digits_and_underscores() {
        assert_checks("extern rust \"snacc_user_v2_ok\" fun ok(): Int64\nprint(0)");
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
    fn truthiness_accepts_values_but_rejects_standalone_nil() {
        assert_checks("while 1 do print(1) end");
        assert_checks("if 1 then print(1) end");
        assert_error_contains("if nil then print(1) end", "standalone 'nil'");
    }

    #[test]
    fn predeclared_error_and_return_on_error_are_checked() {
        assert_checks(
            "fun fail(): Int64 | Error do return Error(category: \"x\", header: \"h\", message: \"m\") end\nfun caller(): Int64 | Error do let value: Int64 = return_on_error fail() value end",
        );
    }

    #[test]
    fn return_on_error_supports_nil_statement_form() {
        let program = assert_checks(
            "fun flush(): Nil | Error do return nil end\nfun caller(): Int64 | Error do return_on_error flush() return 1 end",
        );
        assert!(matches!(
            program.funcs["caller"].body.statements[0],
            TStmt::ReturnOnError { .. }
        ));
    }

    #[test]
    fn return_on_error_rejects_nil_expression_form_and_discarded_values() {
        assert_error_contains(
            "fun flush(): Nil | Error do return nil end\nfun caller(): Nil | Error do return_on_error flush() end",
            "at least one non-Nil success member",
        );
        assert_error_contains(
            "fun read(): Int64 | Error do return 1 end\nfun caller(): Int64 | Error do print(0) return_on_error read() return 1 end",
            "statement form requires an operand of exactly 'Nil | Error'",
        );
    }

    #[test]
    fn return_on_error_consumes_the_complete_source_sum() {
        assert_error_contains(
            "fun fail(): Int64 | Error do return Error(category: \"x\", header: \"h\", message: \"m\") end\nfun caller(): Int64 | Error do let value: Int64 | Error = fail() let result: Int64 = return_on_error value value end",
            "already moved",
        );
    }

    #[test]
    fn return_on_error_can_propagate_into_a_larger_result_sum() {
        assert_checks(
            "fun narrow(): Int64 | Error do return 1 end\nfun widen(): Int64 | String | Error do let value: Int64 = return_on_error narrow() value end",
        );
        crate::emit_llvm_ir(
            "fun narrow(): Int64 | Error do return 1 end\nfun widen(): Int64 | String | Error do let value: Int64 = return_on_error narrow() value end\nprint(0)",
        )
        .expect("return_on_error should retag an Error into a larger result sum");
    }

    #[test]
    fn defer_requires_a_no_result_call_and_is_accepted_in_a_block() {
        assert_checks("fun cleanup() do end defer cleanup()");
        assert_error_contains(
            "fun cleanup() do end fun value(): Int64 do defer cleanup() end",
            "must end in an expression",
        );
    }

    #[test]
    fn defer_cleanup_lowers_on_fallthrough_and_return() {
        crate::emit_llvm_ir(
            "fun cleanup() do print(9) end\nfun fallthrough(): Int64 do defer cleanup() 1 end\nfun early(): Int64 do defer cleanup() return 2 end\nprint(fallthrough()) print(early())",
        )
        .expect("defer should lower on both normal and explicit-return exits");
    }

    #[test]
    fn defer_on_error_lowers_on_propagation_only() {
        crate::emit_llvm_ir(
            "fun cleanup() do print(9) end\nfun fail(): Int64 | Error do return Error(category: \"x\", header: \"h\", message: \"m\") end\nfun caller(): Int64 | Error do defer_on_error cleanup() return_on_error fail() end\nprint(0)",
        )
        .expect("defer_on_error should lower alongside return_on_error");
    }

    #[test]
    fn deferred_by_value_arguments_move_only_at_scope_exit() {
        crate::emit_llvm_ir(
            "fun consume(text: String) do print(text) end\nfun caller() do let text: String = \"deferred\" defer consume(text) end\ncaller()",
        )
        .expect("a deferred by-value argument should remain available until exit");
    }

    #[test]
    fn string_views_have_closed_element_types_and_builtin_lengths() {
        let program = assert_checks(
            "let text: String = \"café\" let bytes: View<Byte> = text.bytes() let count: Int64 = bytes.length()",
        );
        assert!(program.body.statements.len() >= 2);
        assert_error_contains(
            "let text: String = \"x\" let bad: View<Int64> = text.bytes()",
            "expected 'View<Int64>', found 'View<Byte>'",
        );
    }

    #[test]
    fn collection_sequences_and_empty_map_set_constructors_check() {
        assert_checks(
            "let coordinates: Array<Int64, 3> = [10, 20, 30] let first: Int64 = coordinates[0] let size: Int64 = coordinates.length()",
        );
        assert_checks(
            "let numbers: List<Int64> = [1, 2, 3] let last: Int64 = numbers[2] let size: Int64 = numbers.length()",
        );
        assert_checks(
            "let scores: Map<String, Int64> = Map<String, Int64>() let seen: Set<Int64> = Set<Int64>()",
        );
    }

    #[test]
    fn collection_literal_and_index_lower_to_valid_llvm() {
        crate::emit_llvm_ir(
            "let values: Array<Int64, 3> = [4, 5, 6] let item: Int64 = values[1] print(item)",
        )
        .expect("array literal and indexing should lower");
    }

    #[test]
    fn map_and_set_operations_lower_through_private_runtime_descriptors() {
        crate::emit_llvm_ir(
            "let mut scores: Map<String, Int64> = Map<String, Int64>()
             let name: String = \"Alice\"
             scores.reserve(4)
             let added: Bool = scores.insert(name.clone(), 10)
             let found: Bool = scores.contains(name)
             let score: Int64 = scores[name]
             let removed: Bool = scores.delete(name)
             scores.clear()
             let mut seen: Set<Int64> = Set<Int64>()
             seen.reserve(4)
             let inserted: Bool = seen.insert(3)
             let present: Bool = seen.contains(3)
             let deleted: Bool = seen.delete(3)
             seen.clear()",
        )
        .expect("map and set operations should lower");
    }

    #[test]
    fn integer_map_and_set_iteration_lower_through_private_runtime_descriptors() {
        crate::emit_llvm_ir(
            "let mut scores: Map<Int64, Int64> = Map<Int64, Int64>()
             scores.insert(1, 10)
             scores.insert(2, 20)
             for key, value in scores do print(key) print(value) end
             let mut seen: Set<Int64> = Set<Int64>()
             seen.insert(3)
             for member in seen do print(member) end",
        )
        .expect("integer map and set iteration should lower");
    }

    #[test]
    fn scalar_map_and_set_key_types_lower_through_private_runtime_descriptors() {
        crate::emit_llvm_ir(
            "let mut widths: Map<UInt16, Int64> = Map<UInt16, Int64>()
             widths.insert(1u16, 16)
             let width: Int64 = widths[1u16]
             let mut flags: Set<Bool> = Set<Bool>()
             flags.insert(true)
             let present: Bool = flags.contains(true)",
        )
        .expect("scalar map and set key types should lower");
    }

    #[test]
    fn string_map_and_set_iteration_uses_borrowed_loop_descriptors() {
        crate::emit_llvm_ir(
            "let mut bag: Map<String, Int64> = Map<String, Int64>()
             let name: String = \"Alice\"
             bag.insert(name.clone(), 10)
             for key, score in bag do print(key) print(score) end
             let mut words: Set<String> = Set<String>()
             words.insert(name.clone())
             for word in words do print(word) end",
        )
        .expect("String map and set iteration should lower through borrowed descriptors");
    }

    #[test]
    fn mutable_scalar_lists_support_push_and_clear() {
        let program = assert_checks(
            "let mut values: List<Int64> = [1, 2]
             values.push(3)
             let size: Int64 = values.length()
             values.clear()
             print(size)",
        );
        assert!(matches!(program.body.statements[1], TStmt::ListPush { .. }));
        assert!(matches!(
            program.body.statements[3],
            TStmt::ListClear { .. }
        ));
        crate::emit_llvm_ir(
            "let mut values: List<Int64> = [1, 2]
             values.push(3)
             values.clear()",
        )
        .expect("scalar list mutation should lower");
    }

    #[test]
    fn same_typed_sequences_and_views_support_equality() {
        crate::emit_llvm_ir(
            "let left: Array<Int64, 2> = [1, 2]
             let right: Array<Int64, 2> = [1, 2]
             let list_left: List<Int64> = [1, 2]
             let list_right: List<Int64> = [1, 2]
             let array_equal: Bool = left == right
             let list_equal: Bool = list_left == list_right
             let left_view: View<Int64> = left.view()
             let right_view: View<Int64> = right.view()
             let view_equal: Bool = left_view == right_view
             let text_left: List<String> = [\"a\"]
             let text_right: List<String> = [\"a\"]
             let text_equal: Bool = text_left == text_right
             print(array_equal)
             print(list_equal)
             print(view_equal)
             print(text_equal)",
        )
        .expect("same-typed sequence equality should lower");
        assert_error_contains(
            "let values: Map<Int64, Int64> = Map<Int64, Int64>() let other: Map<Int64, Int64> = Map<Int64, Int64>() print(values == other)",
            "does not support equality",
        );
    }

    #[test]
    fn list_mutation_requires_mutability_and_storable_elements() {
        assert_error_contains(
            "let values: List<Int64> = [1] values.push(2)",
            "requires a mutable list receiver",
        );
        assert_checks("let mut values: List<Bool | Nil> = [true] values.push(true)");
    }

    #[test]
    fn sequence_for_loop_checks_and_lowers() {
        assert_checks(
            "let values: Array<Int64, 3> = [1, 2, 3]
             for value in values do print(value) end",
        );
        crate::emit_llvm_ir(
            "let values: Array<Int64, 3> = [1, 2, 3]
             for value in values do print(value) end",
        )
        .expect("sequence for loop should lower");
    }

    #[test]
    fn unicode_view_for_loop_uses_scalar_iteration() {
        crate::emit_llvm_ir(
            "let text: String = \"hé\"
             let scalars: View<Unicode> = text.unicode()
             for scalar in scalars do print(scalar) end",
        )
        .expect("Unicode view iteration should lower");
    }

    #[test]
    fn string_view_lookup_and_multi_success_propagation_lower_to_valid_llvm() {
        crate::emit_llvm_ir(
            "fun read(): Int64 | String | Error do return 1 end
             fun caller(): Int64 | String | Error do return_on_error read() end
             let text: String = \"hé\"
             let bytes: View<Byte> = text.bytes()
             let byte: Byte | Nil = bytes.at(0)
             let scalars: View<Unicode> = text.unicode()
             let scalar: Unicode | Nil = scalars.scalar_at(1)
             print(text)",
        )
        .expect("view lookup and multi-success propagation should lower");
    }

    #[test]
    fn view_slicing_and_checked_string_construction_lower_to_valid_llvm() {
        crate::emit_llvm_ir(
            "let text: String = \"héllo\"
             let bytes: View<Byte> = text.bytes()
             let part: View<Byte> | Nil = bytes.slice(0, 2)
             let scalars: View<Unicode> = text.unicode()
             let scalar_part: View<Unicode> | Nil = scalars.slice(1, 3)
             let copy: String = String.from_unicode(scalars)
             let checked: String | Nil = String.from_utf8(bytes)
             print(copy)",
        )
        .expect("view slicing and string construction should lower");
    }

    #[test]
    fn string_concat_accepts_the_closed_scalar_part_set() {
        crate::emit_llvm_ir("let text: String = \"count: \".concat(3).concat(true).concat('!')")
            .expect("String.concat scalar parts should lower");
        assert_error_contains(
            "let text: String = \"x\".concat([1])",
            "String.concat does not accept",
        );
    }

    #[test]
    fn interpolated_strings_use_the_normal_expression_checker_and_lowering() {
        let source = "let name: String = \"Ada\"\n\
                      let count: Int64 = 3\n\
                      let message: String = \"Hello, {{name}}. Count: {{count}}\"\n\
                      print(message)";
        assert_checks(source);
        crate::emit_llvm_ir(source).expect("interpolated strings should lower");
    }

    #[test]
    fn interpolated_strings_reject_empty_and_unclosed_expressions() {
        for source in [
            "let message: String = \"bad {{}}\"",
            "let message: String = \"bad {{1\"",
            "let message: String = \"bad }}\"",
        ] {
            assert!(
                crate::parse(source).is_err(),
                "expected rejection: {source}"
            );
        }
    }

    #[test]
    fn string_views_end_at_their_last_local_use_before_reassignment() {
        assert_checks(
            "let mut text: String = \"hello\"\n\
             let bytes: View<Byte> = text.bytes()\n\
             print(bytes.length())\n\
             text = \"goodbye\"",
        );
        assert_error_contains(
            "let mut text: String = \"hello\"\n\
             let bytes: View<Byte> = text.bytes()\n\
             text = \"goodbye\"\n\
             print(bytes.length())",
            "still borrows it",
        );
    }

    #[test]
    fn strings_lend_the_expected_view_type_to_calls_but_views_do_not_escape() {
        assert_checks(
            "fun count(bytes: View<Byte>): Int64 do bytes.length() end\n\
             let text: String = \"hello\"\n\
             let result: Int64 = count(text)",
        );
        assert_error_contains(
            "fun leak(text: String): View<Byte> do text.bytes() end",
            "cannot be returned",
        );
        assert_error_contains(
            "let text: String = \"hello\"\nlet boxed: Box<View<Byte>> = box(text.bytes())",
            "cannot be stored in a Box",
        );
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

    const NEW_TYPES: [(&str, &str); 5] = [
        ("Byte", "1u8"),
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
            assert_eq!(program.funcs["identity"].params[0].ty, expected.unwrap());
            assert_eq!(program.externs["edge"].result, expected);
        }
    }

    #[test]
    fn rejects_every_implicit_conversion_the_new_types_prohibit() {
        for (source, needle) in [
            ("let byte: Byte = 1", "expected 'Byte', found 'Int64'"),
            ("let byte: Byte = 1u16", "expected 'Byte', found 'UInt16'"),
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
                "expected 'Float32', found 'Byte'",
            ),
            (
                "let ratio: Float32 = 1.5",
                "expected 'Float32', found 'Float64'",
            ),
            (
                "let wide: Float64 = 1.5f32",
                "expected 'Float64', found 'Float32'",
            ),
            (
                "let wide: Float64 = 1u8",
                "expected 'Float64', found 'Byte'",
            ),
        ] {
            assert_error_contains(source, needle);
        }
    }

    #[test]
    fn the_int64_to_float64_conversion_still_works() {
        assert_checks("let wide: Float64 = 1\nprint(wide + 1)");
    }

    #[test]
    fn accepts_same_type_arithmetic_and_comparison_for_every_new_type() {
        for (name, literal) in NEW_TYPES {
            for operator in ["+", "-", "*", "/"] {
                assert_checks(&format!(
                    "let result: {name} = {literal} {operator} {literal}"
                ));
            }
            for operator in ["<", "<=", ">", ">=", "==", "!="] {
                assert_checks(&format!("let flag: Bool = {literal} {operator} {literal}"));
            }
        }
    }

    #[test]
    fn rejects_mixed_operands_in_every_category() {
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
    fn statically_known_nan_is_rejected_for_both_float_widths() {
        assert_error_contains("print(0.0 / 0.0)", "floating-point operation produces NaN");
        assert_error_contains(
            "print(0f32 / 0f32)",
            "floating-point operation produces NaN",
        );
        assert_error_contains(
            "print(1.0 / 0.0 - 1.0 / 0.0)",
            "floating-point operation produces NaN",
        );
    }

    #[test]
    fn infinity_remains_an_admitted_non_nan_float_result() {
        assert_checks("print(1.0 / 0.0)");
        assert_checks("print(1f32 / 0f32)");
    }

    #[test]
    fn arithmetic_and_comparison_keep_the_exact_operand_type() {
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

    // ---------------------------------------------------------------------
    // Specification 010: nominal types, structs, unions, and methods.
    // ---------------------------------------------------------------------

    const POINT: &str = "type Point is struct x: Float64, y: Float64, end\n";
    const SHAPE: &str = "type Shape is union\n\
         | Circle is struct radius: Int64, end\n\
         | Rectangle is struct length: Int64, width: Int64, end\n\
         end\n";
    const DIRECTION: &str = "type Direction is union | East | West end\n";

    /// Conformance 1: represented types are nominal and wrap/unwrap one layer.
    #[test]
    fn a_represented_type_wraps_and_unwraps_exactly_one_layer() {
        assert_checks(
            "type UserId is Int64\n\
             let id: UserId = UserId(42)\n\
             let number: Int64 = Int64(id)\n\
             print(number)",
        );
    }

    #[test]
    fn a_represented_type_is_not_its_representation() {
        assert_error_contains(
            "type UserId is Int64\nlet id: UserId = 42",
            "expected 'UserId', found 'Int64'",
        );
        assert_error_contains(
            "type UserId is Int64\nlet id: UserId = UserId(42)\nlet n: Int64 = id",
            "expected 'Int64', found 'UserId'",
        );
        assert_error_contains(
            "type UserId is Int64\nlet id: UserId = UserId(42)\nprint(id + id)",
            "operands must be two numbers of the same type",
        );
    }

    #[test]
    fn represented_conversion_names_the_required_immediate_type() {
        assert_error_contains(
            "type UserId is Int64\nlet id: UserId = UserId(1.5)",
            "wraps exactly its immediate representation 'Int64', found 'Float64'",
        );
        assert_error_contains(
            "type UserId is Int64\nprint(Int64(7))",
            "unwraps exactly one value of a type represented by 'Int64', found 'Int64'",
        );
        assert_error_contains(
            "type UserId is Int64\nlet id: UserId = UserId(1, 2)",
            "converts exactly one positional value",
        );
        assert_error_contains(
            "type UserId is Int64\nlet id: UserId = UserId(value: 1)",
            "named arguments are only used to construct a struct",
        );
    }

    /// Conformance 1: unwrapping skips no layer.
    #[test]
    fn represented_conversion_does_not_skip_a_layer() {
        assert_error_contains(
            "type Inner is Int64\ntype Outer is Inner\n\
             let value: Outer = Outer(Inner(1))\n\
             let flat: Int64 = Int64(value)",
            "unwraps exactly one value of a type represented by 'Int64', found 'Outer'",
        );
        assert_checks(
            "type Inner is Int64\ntype Outer is Inner\n\
             let value: Outer = Outer(Inner(1))\n\
             let one: Inner = Inner(value)\n\
             let flat: Int64 = Int64(one)\n\
             print(flat)",
        );
    }

    /// Conformance 2: same-representation nominal types stay distinct.
    #[test]
    fn same_representation_nominal_types_are_not_assignable_or_comparable() {
        assert_error_contains(
            "type UserId is Int64\ntype OrderId is Int64\n\
             let id: UserId = UserId(1)\nlet other: OrderId = id",
            "expected 'OrderId', found 'UserId'",
        );
        assert_error_contains(
            "type UserId is Int64\ntype OrderId is Int64\n\
             let id: UserId = UserId(1)\nlet other: OrderId = OrderId(1)\n\
             print(id == other)",
            "expected 'UserId', found 'OrderId'",
        );
    }

    /// Conformance 7: represented equality compares represented values.
    #[test]
    fn represented_values_compare_with_their_own_type_only() {
        assert_checks(
            "type UserId is Int64\n\
             let a: UserId = UserId(1)\nlet b: UserId = UserId(2)\nprint(a == b)",
        );
        assert_error_contains(
            "type UserId is Int64\nlet a: UserId = UserId(1)\nprint(a == 1)",
            "expected 'UserId', found 'Int64'",
        );
    }

    /// Conformance 3: reordered named fields and a trailing comma, checked and
    /// stored so evaluation stays in written order.
    #[test]
    fn struct_construction_accepts_reordered_named_fields() {
        let program = assert_checks(&format!(
            "{POINT}let point: Point = Point(y: 4.0, x: 3.0,)\nprint(point.x)"
        ));
        let TStmt::Let { value, .. } = &program.body.statements[0] else {
            panic!("expected a declaration");
        };
        let TExpr::Construct { fields, .. } = value else {
            panic!("expected a constructor");
        };
        assert_eq!(
            fields.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            vec![1, 0],
            "constructor entries keep written evaluation order with declaration indices"
        );
    }

    /// Conformance 4: missing, duplicate, unknown, and positional fields.
    #[test]
    fn rejects_malformed_struct_construction() {
        for (source, needle) in [
            ("let p: Point = Point(x: 1.0)", "is missing field 'y'"),
            (
                "let p: Point = Point(x: 1.0, x: 2.0, y: 3.0)",
                "Field 'Point.x' is supplied more than once",
            ),
            (
                "let p: Point = Point(x: 1.0, y: 2.0, z: 3.0)",
                "'Point' has no field 'z'",
            ),
            (
                "let p: Point = Point(1.0, 2.0)",
                "positional construction of a non-empty struct is invalid",
            ),
        ] {
            assert_error_contains(&format!("{POINT}{source}"), needle);
        }
    }

    /// Conformance 5: empty top-level and union-member structs.
    #[test]
    fn empty_structs_construct_with_parentheses_and_stay_nominal() {
        assert_checks(
            "type Marker is struct end\ntype Other is struct end\n\
             let marker: Marker = Marker()\nlet second: Marker = Marker()\n\
             print(marker == second)",
        );
        assert_error_contains(
            "type Marker is struct end\ntype Other is struct end\n\
             let marker: Marker = Marker()\nlet other: Other = Other()\n\
             print(marker == other)",
            "expected 'Marker', found 'Other'",
        );
        assert_error_contains(
            "type Marker is struct end\nlet marker: Marker = Marker(x: 1)",
            "has no fields, so it is constructed with '()'",
        );
        assert_error_contains(
            "type Marker is struct end\nlet marker: Marker = Marker",
            "a type name alone is never a value",
        );
    }

    /// Conformance 6: member names live only in their union's namespace.
    #[test]
    fn union_member_names_resolve_only_through_their_union() {
        assert_checks(&format!(
            "{SHAPE}let shape: Shape = Shape.Circle(radius: 10)\nprint(1)"
        ));
        assert_error_contains(
            &format!("{SHAPE}let c: Circle = 1"),
            "Unknown type 'Circle'",
        );
        assert_error_contains(
            &format!("{SHAPE}print(Circle(radius: 1))"),
            "'Circle' is not callable",
        );
        assert_error_contains(
            &format!("{SHAPE}let s: Shape = Shape.Triangle()"),
            "'Shape' has no member type 'Triangle'",
        );
    }

    /// Conformance 8: a union takes each direct member and nothing else.
    #[test]
    fn a_union_accepts_its_direct_members_and_rejects_others() {
        assert_checks(&format!(
            "{SHAPE}{DIRECTION}\
             let a: Shape = Shape.Circle(radius: 1)\n\
             let b: Shape = Shape.Rectangle(length: 1, width: 2)\n\
             let c: Direction = Direction.East()\nprint(1)"
        ));
        assert_error_contains(
            &format!("{SHAPE}{DIRECTION}let bad: Shape = Direction.East()"),
            "expected 'Shape', found 'Direction.East'",
        );
        assert_error_contains(
            &format!("{SHAPE}let bad: Shape = Shape(1)"),
            "construction names one member type, not the union itself",
        );
    }

    /// Conformance 7: struct and union equality.
    #[test]
    fn struct_and_union_equality_follow_nominal_identity() {
        assert_checks(&format!(
            "{POINT}let a: Point = Point(x: 1.0, y: 2.0)\n\
             let b: Point = Point(x: 1.0, y: 2.0)\nprint(a == b)"
        ));
        assert_checks(&format!(
            "{SHAPE}let a: Shape = Shape.Circle(radius: 1)\n\
             let b: Shape = Shape.Rectangle(length: 1, width: 2)\nprint(a != b)"
        ));
        // A union never compares directly with one of its member types.
        assert_error_contains(
            &format!(
                "{SHAPE}let a: Shape = Shape.Circle(radius: 1)\n\
                 let b: Shape.Circle = Shape.Circle(radius: 1)\nprint(a == b)"
            ),
            "expected 'Shape', found 'Shape.Circle'",
        );
        assert_error_contains(
            &format!("{POINT}let a: Point = Point(x: 1.0, y: 2.0)\nprint(a < a)"),
            "operands must be two numbers of the same type",
        );
    }

    #[test]
    fn union_tags_follow_source_order_from_zero() {
        let program = assert_checks(&format!("{SHAPE}print(1)"));
        let tags: Vec<(String, u32)> = program
            .types
            .iter()
            .filter_map(|def| match def {
                TypeDef::UnionMember { name, tag, .. } => Some((name.clone(), *tag)),
                _ => None,
            })
            .collect();
        assert_eq!(
            tags,
            vec![("Shape.Circle".into(), 0), ("Shape.Rectangle".into(), 1)]
        );
    }

    /// Conformance 10: methods read `self` and work on values and temporaries.
    #[test]
    fn methods_read_self_and_return_values() {
        let program = assert_checks(&format!(
            "{POINT}\
             method Point.sum(): Float64 do self.x + self.y end\n\
             method Point.scaled(factor: Float64): Point do \
                Point(x: self.x * factor, y: self.y * factor) end\n\
             let point: Point = Point(x: 3.0, y: 4.0)\n\
             print(point.sum())\n\
             print(Point(x: 1.0, y: 2.0).sum())\n\
             print(point.scaled(2.0).sum())"
        ));
        assert_eq!(program.methods.len(), 2);
        assert!(program.methods.iter().all(|m| !m.writes_receiver));
    }

    /// Conformance 13: lookup is exact, namespaced, and non-overloaded.
    #[test]
    fn method_lookup_is_exact_and_namespaced_by_receiver_type() {
        assert_checks(&format!(
            "{POINT}type Other is struct x: Float64, end\n\
             method Point.width(): Float64 do self.x end\n\
             method Other.width(): Float64 do self.x end\n\
             let p: Point = Point(x: 1.0, y: 2.0)\n\
             let o: Other = Other(x: 1.0)\n\
             print(p.width())\nprint(o.width())"
        ));
        assert_error_contains(
            &format!(
                "{POINT}method Point.a(): Float64 do 1.0 end\nmethod Point.a(): Float64 do 2.0 end"
            ),
            "Method 'Point.a' already exists; methods are not overloaded",
        );
        assert_error_contains(
            &format!("{POINT}let p: Point = Point(x: 1.0, y: 2.0)\nprint(p.missing())"),
            "'Point' has no method 'missing'",
        );
        assert_error_contains(
            &format!(
                "{POINT}method Point.sum(): Float64 do self.x end\n\
                 let p: Point = Point(x: 1.0, y: 2.0)\nprint(p.sum)"
            ),
            "'Point.sum' is a method; a method requires a receiver call",
        );
        assert_error_contains(
            &format!("{POINT}let p: Point = Point(x: 1.0, y: 2.0)\nprint(p.x())"),
            "'Point.x' is a field, not a method, so it cannot be called",
        );
        assert_error_contains(
            "method Missing.thing(): Int64 do 1 end",
            "Unknown type 'Missing'",
        );
    }

    /// Conformance 12: `self` is method-only.
    #[test]
    fn self_is_rejected_outside_a_method() {
        assert_error_contains("print(self)", "'self' is only valid inside a method body");
        assert_error_contains(
            "fun f(): Int64 do self end",
            "'self' is only valid inside a method body",
        );
        assert_error_contains("self = 1", "'self' is only valid inside a method body");
        // `self` is a keyword, so it cannot be spelled as a binding at all.
        assert_rejected_by_parser("let self: Int64 = 1");
        assert_rejected_by_parser("fun f(self: Int64) do end");
    }

    /// Conformance 11 and 15, with Specification 012 sections 7 and 9.
    #[test]
    fn field_and_receiver_assignment_follow_root_mutability_only() {
        assert_checks(&format!(
            "{POINT}let mut point: Point = Point(x: 3.0, y: 4.0)\n\
             point.x = 5.0\npoint = Point(x: 0.0, y: 0.0)"
        ));
        assert_error_contains(
            &format!("{POINT}let point: Point = Point(x: 3.0, y: 4.0)\npoint.x = 5.0"),
            "'point' is not declared 'mut' and cannot be assigned",
        );
        assert_error_contains(
            &format!(
                "{POINT}let point: Point = Point(x: 3.0, y: 4.0)\npoint = Point(x: 0.0, y: 0.0)"
            ),
            "'point' is not declared 'mut' and cannot be assigned",
        );
        // Ordinary parameters and their fields are immutable roots.
        assert_error_contains(
            &format!("{POINT}fun shift(point: Point) do point.x = 1.0 end"),
            "'point' is not declared 'mut' and cannot be assigned",
        );
        // Root mutability applies through the complete field path, and never
        // consults the struct definition.
        assert_checks(&format!(
            "{POINT}type Entity is struct position: Point, end\n\
             let mut entity: Entity = Entity(position: Point(x: 1.0, y: 2.0))\n\
             entity.position.x = 1.0"
        ));
        assert_error_contains(
            &format!(
                "{POINT}type Entity is struct position: Point, end\n\
                 let entity: Entity = Entity(position: Point(x: 1.0, y: 2.0))\n\
                 entity.position.x = 1.0"
            ),
            "'entity' is not declared 'mut' and cannot be assigned",
        );
        assert_error_contains(
            &format!("{POINT}let mut point: Point = Point(x: 1.0, y: 2.0)\npoint.z = 1.0"),
            "'Point' has no field 'z'",
        );
    }

    /// Conformance 11: receiver-writing calls need a mutable root; whole-`self`
    /// replacement is one of them, and no method-level marker exists.
    #[test]
    fn receiver_writing_calls_require_a_mutable_receiver_root() {
        let program = assert_checks(&format!(
            "{POINT}\
             method Point.translate(dx: Float64, dy: Float64) do \
                self.x = self.x + dx self.y = self.y + dy end\n\
             method Point.reset() do self = Point(x: 0.0, y: 0.0) end\n\
             method Point.sum(): Float64 do self.x + self.y end\n\
             let mut point: Point = Point(x: 3.0, y: 4.0)\n\
             point.translate(1.0, 2.0)\npoint.reset()\nprint(point.sum())"
        ));
        let effects: Vec<(String, bool)> = program
            .methods
            .iter()
            .map(|m| (m.name.clone(), m.writes_receiver))
            .collect();
        assert_eq!(
            effects,
            vec![
                ("translate".into(), true),
                ("reset".into(), true),
                ("sum".into(), false)
            ]
        );
        for call in ["point.translate(1.0, 2.0)", "point.reset()"] {
            assert_error_contains(
                &format!(
                    "{POINT}\
                     method Point.translate(dx: Float64, dy: Float64) do self.x = self.x + dx end\n\
                     method Point.reset() do self = Point(x: 0.0, y: 0.0) end\n\
                     let point: Point = Point(x: 3.0, y: 4.0)\n{call}"
                ),
                "requires a mutable root, but 'point' is not mutable",
            );
        }
        // A read-only method is callable on a temporary; a writing one is not.
        assert_error_contains(
            &format!(
                "{POINT}method Point.reset() do self = Point(x: 0.0, y: 0.0) end\n\
                 Point(x: 1.0, y: 2.0).reset()"
            ),
            "requires a mutable root, but 'a temporary' is not mutable",
        );
    }

    /// Conformance 27: the effect reaches its least fixed point through direct,
    /// transitive, recursive, and mutually recursive calls, and marks nothing
    /// for unrelated local writes.
    #[test]
    fn receiver_write_effects_reach_a_least_fixed_point() {
        let source = format!(
            "{POINT}\
             method Point.bump() do self.x = self.x + 1.0 end\n\
             method Point.outer() do self.middle() end\n\
             method Point.middle() do self.bump() end\n\
             method Point.ping() do self.pong() end\n\
             method Point.pong() do self.ping() end\n\
             method Point.local_only() do let mut n: Float64 = 1.0 n = n + 1.0 end\n\
             method Point.selfish() do self.selfish() end\n\
             let mut point: Point = Point(x: 1.0, y: 2.0)\npoint.outer()"
        );
        let program = assert_checks(&source);
        let effects: Vec<(String, bool)> = program
            .methods
            .iter()
            .map(|m| (m.name.clone(), m.writes_receiver))
            .collect();
        assert_eq!(
            effects,
            vec![
                ("bump".into(), true),
                // Transitive through one intermediate method.
                ("outer".into(), true),
                ("middle".into(), true),
                // A mutually recursive pair that never writes stays unmarked.
                ("ping".into(), false),
                ("pong".into(), false),
                // An unrelated local write is not a receiver write.
                ("local_only".into(), false),
                ("selfish".into(), false),
            ]
        );
        // The transitive fact is what rejects the call, not the direct one.
        assert_error_contains(
            &source.replace("let mut point", "let point"),
            "'Point.outer' may assign through 'self', so its receiver requires a mutable root",
        );
        // Calling on a field of a mutable root is allowed.
        assert_checks(&format!(
            "{POINT}type Entity is struct position: Point, end\n\
             method Point.bump() do self.x = self.x + 1.0 end\n\
             let mut entity: Entity = Entity(position: Point(x: 1.0, y: 2.0))\n\
             entity.position.bump()"
        ));
    }

    /// A type-test binding is an immutable root (Specification 012 section 7).
    #[test]
    fn a_type_test_binding_is_an_immutable_root() {
        assert_error_contains(
            &format!(
                "{SHAPE}let shape: Shape = Shape.Circle(radius: 1)\n\
                 if shape is Shape.Circle(circle) then circle.radius = 2 end"
            ),
            "'circle' is not declared 'mut' and cannot be assigned",
        );
    }

    /// Conformance 16: a no-result method call is a statement only.
    #[test]
    fn no_result_method_calls_are_rejected_in_value_positions() {
        assert_checks(&format!(
            "{POINT}method Point.noop() do print(1) end\n\
             let mut p: Point = Point(x: 1.0, y: 2.0)\np.noop()"
        ));
        assert_error_contains(
            &format!(
                "{POINT}method Point.noop() do print(1) end\n\
                 let mut p: Point = Point(x: 1.0, y: 2.0)\nlet n: Int64 = p.noop()"
            ),
            "'Point.noop' declares no result, so its call cannot be used as a value",
        );
    }

    /// Conformance 26: call-head conflicts and section 6.1 resolution.
    #[test]
    fn a_type_name_may_not_share_a_call_head_with_a_callable() {
        assert_error_contains(
            "type Point is struct x: Int64, end\nfun Point(): Int64 do 1 end",
            "shares a call head with the function or Rust bridge of the same name",
        );
        assert_error_contains(
            "type Point is struct x: Int64, end\n\
             extern rust \"snacc_user_point\" fun Point(): Int64",
            "shares a call head with the function or Rust bridge of the same name",
        );
        assert_error_contains(
            "type Point is struct x: Int64, end\ntype Point is Int64",
            "Type 'Point' already exists",
        );
    }

    /// Conformance 26: an in-scope binding wins a qualified call head, and a
    /// bare `name(...)` never calls a local.
    #[test]
    fn a_binding_wins_a_qualified_call_head_over_a_type_path() {
        // `Case` is both a union type and a parameter; the parameter wins.
        // (Specification 016 reserves `Box`, so this uses an unreserved name
        // that still exercises the same type-path-versus-binding shadowing.)
        assert_checks(
            "type Case is union | Item is struct n: Int64, end end\n\
             method Case.Item.get(): Int64 do self.n end\n\
             fun read(Case: Case.Item): Int64 do Case.get() end\n\
             print(read(Case.Item(n: 1)))",
        );
        // Without a binding of that name the same path is a constructor.
        assert_checks(
            "type Case is union | Item is struct n: Int64, end end\n\
             let held: Case = Case.Item(n: 1)\nprint(1)",
        );
        assert_error_contains(
            "fun f(value: Int64): Int64 do value(1) end",
            "'value' is a variable; Snacc has no function values, so it cannot be called",
        );
        assert_error_contains("print(missing(1))", "'missing' is not callable");
        // A bare call head skips the local namespace entirely, so a local that
        // shares a callable's name does not hide it.
        assert_checks(
            "fun double(value: Int64): Int64 do value * 2 end\n\
             fun use(double: Int64): Int64 do double(double) end\n\
             print(use(3))",
        );
        // The same holds for a type constructor sharing a local's name.
        assert_checks(
            "type Wrapper is struct n: Int64, end\n\
             fun build(Wrapper: Int64): Int64 do Wrapper(n: Wrapper).n end\n\
             print(build(3))",
        );
    }

    /// Conformance 17 and 18: `is` produces `Bool`, and its binding has the
    /// exact member type only inside the successful branch.
    #[test]
    fn type_tests_narrow_only_inside_their_own_branch() {
        assert_checks(&format!(
            "{SHAPE}let shape: Shape = Shape.Circle(radius: 10)\n\
             if shape is Shape.Circle(circle) then print(circle.radius) \
             elseif shape is Shape.Rectangle(rectangle) then \
             print(rectangle.length * rectangle.width) end"
        ));
        assert_error_contains(
            &format!(
                "{SHAPE}let shape: Shape = Shape.Circle(radius: 10)\n\
                 if shape is Shape.Circle(circle) then print(1) else print(circle.radius) end"
            ),
            "No such variable 'circle' in scope",
        );
        assert_error_contains(
            &format!(
                "{SHAPE}let shape: Shape = Shape.Circle(radius: 10)\n\
                 if shape is Shape.Circle(circle) then print(1) end\nprint(circle.radius)"
            ),
            "No such variable 'circle' in scope",
        );
        // Specification 012 section 5.2: the binding name is function-wide.
        assert_error_contains(
            &format!(
                "{SHAPE}let circle: Int64 = 1\nlet shape: Shape = Shape.Circle(radius: 10)\n\
                 if shape is Shape.Circle(circle) then print(1) end"
            ),
            "Binding 'circle' already exists",
        );
        assert_error_contains(
            &format!(
                "{SHAPE}let shape: Shape = Shape.Circle(radius: 10)\n\
                 if shape is Shape.Circle(c) then print(1) elseif shape is Shape.Rectangle(c) \
                 then print(2) end"
            ),
            "Binding 'c' already exists",
        );
    }

    #[test]
    fn a_bare_type_test_supplies_a_bool_condition() {
        let program = assert_checks(&format!(
            "{DIRECTION}let d: Direction = Direction.East()\n\
             if d is Direction.East then print(1) elseif d is Direction.West then print(2) end"
        ));
        let TStmt::If(form) = &program.body.statements[1] else {
            panic!("expected an if statement");
        };
        let tags: Vec<u32> = form
            .arms
            .iter()
            .map(|(condition, _)| match condition {
                TCondition::Test(test) => test.tag,
                TCondition::Expr(_) | TCondition::SumTest(_) => panic!("expected a union test"),
            })
            .collect();
        assert_eq!(tags, vec![0, 1]);
        assert!(form.exhaustive);
    }

    /// Conformance 19: unrelated, always-true, and non-place tests.
    #[test]
    fn invalid_type_tests_are_rejected() {
        assert_error_contains(
            &format!(
                "{SHAPE}{DIRECTION}let shape: Shape = Shape.Circle(radius: 1)\n\
                 if shape is Direction.East then print(1) end"
            ),
            "'Direction.East' is not a direct member of 'Shape'",
        );
        assert_error_contains(
            &format!(
                "{SHAPE}let shape: Shape = Shape.Circle(radius: 1)\n\
                 if shape is Shape then print(1) end"
            ),
            "already has type 'Shape', so this test is always true",
        );
        assert_error_contains(
            &format!(
                "{POINT}let p: Point = Point(x: 1.0, y: 2.0)\nif p is Point then print(1) end"
            ),
            "the left side of 'is' must have a union type",
        );
        assert_error_contains(
            &format!(
                "{SHAPE}let c: Shape.Circle = Shape.Circle(radius: 1)\n\
                 if c is Shape.Circle then print(1) end"
            ),
            "the left side of 'is' must have a union type",
        );
        // A call result is not a place, so it cannot be the subject of `is`.
        assert_rejected_by_parser(&format!(
            "{SHAPE}fun make(): Shape do Shape.Circle(radius: 1) end\n\
             if make() is Shape.Circle then print(1) end"
        ));
    }

    /// Conformance 20: exhaustive chains produce values without `else`, for
    /// both empty and data-carrying members.
    #[test]
    fn an_exhaustive_chain_produces_a_value_without_an_else() {
        let program = assert_checks(&format!(
            "{DIRECTION}\
             fun pick(d: Direction): Int64 do \
                if d is Direction.East then 1 elseif d is Direction.West then 2 end end\n\
             print(pick(Direction.East()))"
        ));
        let result = program.funcs["pick"]
            .body
            .result
            .as_ref()
            .expect("pick produces a value");
        let TExpr::If(form) = result else {
            panic!("expected a value-form if");
        };
        assert!(form.exhaustive);
        assert!(form.else_branch.is_none());

        assert_checks(&format!(
            "{SHAPE}\
             fun area(shape: Shape): Int64 do \
                if shape is Shape.Circle(circle) then circle.radius * circle.radius \
                elseif shape is Shape.Rectangle(rectangle) then \
                rectangle.length * rectangle.width end end\n\
             print(area(Shape.Circle(radius: 2)))"
        ));
    }

    /// Conformance 21: missing members, duplicates, mixed places, and an
    /// unreachable `else`.
    #[test]
    fn non_exhaustive_and_unreachable_chains_are_rejected() {
        let three = "type Light is union | Red | Amber | Green end\n";
        assert_error_contains(
            &format!(
                "{three}fun pick(l: Light): Int64 do \
                 if l is Light.Red then 1 elseif l is Light.Amber then 2 end end"
            ),
            "does not handle Light.Green",
        );
        assert_error_contains(
            &format!(
                "{DIRECTION}fun pick(d: Direction): Int64 do \
                 if d is Direction.East then 1 elseif d is Direction.East then 2 \
                 elseif d is Direction.West then 3 end end"
            ),
            "'Direction.East' is already handled by an earlier branch",
        );
        // Two different places never form one chain, so `else` stays required.
        assert_error_contains(
            &format!(
                "{DIRECTION}fun pick(a: Direction, b: Direction): Int64 do \
                 if a is Direction.East then 1 elseif b is Direction.West then 2 end end"
            ),
            "requires an 'else' branch",
        );
        // An ordinary condition mixed into the chain also requires `else`.
        assert_error_contains(
            &format!(
                "{DIRECTION}fun pick(d: Direction, flag: Bool): Int64 do \
                 if d is Direction.East then 1 elseif flag then 2 end end"
            ),
            "requires an 'else' branch",
        );
        // A covered chain rejects `else` in both statement and value form.
        for source in [
            format!(
                "{DIRECTION}fun pick(d: Direction): Int64 do \
                 if d is Direction.East then 1 elseif d is Direction.West then 2 else 3 end end"
            ),
            format!(
                "{DIRECTION}let d: Direction = Direction.East()\n\
                 if d is Direction.East then print(1) elseif d is Direction.West then print(2) \
                 else print(3) end"
            ),
        ] {
            assert_error_contains(&source, "so the 'else' branch is unreachable");
        }
    }

    /// Section 12.3: the tested place may be a field path rooted at a local,
    /// and exhaustiveness compares the complete syntactic place.
    #[test]
    fn an_exhaustive_chain_may_test_a_field_path() {
        let types =
            format!("{DIRECTION}type Entity is struct heading: Direction, spare: Direction, end\n");
        assert_checks(&format!(
            "{types}fun rank(entity: Entity): Int64 do \
             if entity.heading is Direction.East then 1 \
             elseif entity.heading is Direction.West then 2 end end"
        ));
        // Two different field paths under one root are two different places.
        assert_error_contains(
            &format!(
                "{types}fun rank(entity: Entity): Int64 do \
                 if entity.heading is Direction.East then 1 \
                 elseif entity.spare is Direction.West then 2 end end"
            ),
            "requires an 'else' branch",
        );
    }

    /// Section 11.2 and conformance 14: the receiver evaluates once, before the
    /// explicit arguments, and arguments keep their written order.
    #[test]
    fn a_method_call_keeps_its_receiver_and_arguments_in_order() {
        let program = assert_checks(&format!(
            "{POINT}method Point.combine(a: Float64, b: Float64): Float64 do self.x + a + b end\n\
             let p: Point = Point(x: 1.0, y: 2.0)\nprint(p.combine(3.0, 4.0))"
        ));
        let TStmt::Expr(TExpr::Print(value, _)) = &program.body.statements[1] else {
            panic!("expected a print statement");
        };
        let TExpr::MethodCall(call) = value.as_ref() else {
            panic!("expected a method call");
        };
        assert!(
            matches!(&call.receiver, TReceiver::Place(place) if place.root == PlaceRoot::Local("p".into())),
            "a place receiver keeps its addressable storage"
        );
        assert_eq!(call.args.len(), 2);
    }

    /// Conformance 22: adding a member breaks a formerly exhaustive chain.
    #[test]
    fn adding_a_union_member_breaks_an_exhaustive_chain() {
        let chain = "fun pick(d: Direction): Int64 do \
             if d is Direction.East then 1 elseif d is Direction.West then 2 end end";
        assert_checks(&format!("{DIRECTION}{chain}"));
        assert_error_contains(
            &format!("type Direction is union | East | West | North end\n{chain}"),
            "does not handle Direction.North",
        );
    }

    /// Conformance 23: direct and indirect recursive layouts.
    #[test]
    fn recursive_value_layouts_are_rejected() {
        assert_error_contains(
            "type Node is struct next: Node, end",
            "Type 'Node' has an infinite value layout: Node -> Node",
        );
        assert_error_contains(
            "type A is struct b: B, end\ntype B is struct a: A, end",
            "has an infinite value layout: A -> B -> A",
        );
        assert_error_contains(
            "type A is B\ntype B is A",
            "has an infinite value layout: A -> B -> A",
        );
        assert_error_contains(
            "type Tree is union | Leaf | Branch is struct child: Tree, end end",
            "has an infinite value layout",
        );
    }

    /// Conformance 24: no user-defined type crosses the Rust bridge.
    #[test]
    fn user_defined_types_are_rejected_at_every_bridge_site() {
        for declaration in [
            "extern rust \"snacc_user_take\" fun take(value: Point)",
            "extern rust \"snacc_user_make\" fun make(): Point",
        ] {
            assert_error_contains(
                &format!("{POINT}{declaration}"),
                "only the ABI's permitted types may cross a Rust bridge",
            );
        }
        assert_error_contains(
            &format!("{SHAPE}extern rust \"snacc_user_take\" fun take(value: Shape.Circle)"),
            "'Shape.Circle' is a user-defined type",
        );
        assert_error_contains(
            "type UserId is Int64\nextern rust \"snacc_user_take\" fun take(value: UserId)",
            "'UserId' is a user-defined type",
        );
        // Internal Snacc functions accept and return them freely.
        assert_checks(&format!(
            "{POINT}fun identity(point: Point): Point do point end\n\
             print(identity(Point(x: 1.0, y: 2.0)).x)"
        ));
    }

    /// Conformance 14 (checking half): a union type flows from an expected
    /// type through branches and arguments.
    #[test]
    fn common_union_types_come_from_the_expected_type() {
        assert_checks(&format!(
            "{DIRECTION}\
             fun pick(flag: Bool): Direction do \
                if flag then Direction.East() else Direction.West() end end\n\
             fun take(d: Direction): Int64 do 1 end\n\
             print(take(Direction.East()))\nprint(take(pick(true)))"
        ));
        assert_error_contains(
            &format!(
                "{DIRECTION}{SHAPE}\
                 fun pick(flag: Bool): Direction do \
                    if flag then Direction.East() else Shape.Circle(radius: 1) end end"
            ),
            "expected 'Direction', found 'Shape.Circle'",
        );
        assert_error_contains(
            &format!(
                "{DIRECTION}fun pick(flag: Bool): Int64 do \
                 if flag then Direction.East() else 1 end end"
            ),
            "expected 'Int64', found 'Direction.East'",
        );
    }

    /// Specification 012 section 10: `Nil` is a union member and `nil` names it
    /// from an expected union type.
    #[test]
    fn nil_is_available_as_a_union_member() {
        assert_checks(
            "type UserId is Int64\n\
             type MaybeUser is union | User is struct id: UserId, end | Nil end\n\
             let missing: MaybeUser = nil\n\
             let present: MaybeUser = MaybeUser.User(id: UserId(10))\n\
             if missing is Nil then print(1) elseif missing is MaybeUser.User(user) then \
             print(Int64(user.id)) end\n\
             print(missing == nil)",
        );
        assert_error_contains(
            "type MaybeUser is union | User is struct id: Int64, end | Nil end\n\
             let missing: MaybeUser = nil\n\
             if missing is Nil(value) then print(1) end",
            "'Nil' carries no value, so it cannot be bound by a type test",
        );
        assert_error_contains(
            "type Empty is union | Nil end",
            "contains only 'Nil'; 'Nil' requires another member type",
        );
    }

    /// Specification 012 conformance 16: standalone `Nil` is rejected in every
    /// type and bridge position.
    #[test]
    fn standalone_nil_is_rejected_in_every_type_position() {
        for source in [
            "let value: Nil = nil",
            "fun consume(value: Nil) do print(1) end",
            "fun produce(): Nil do nil end",
            "fun update(value: Ref<Nil>) do print(1) end",
            "type Empty is Nil",
            "type Holder is struct value: Nil, end",
            "type Maybe is union | Held is struct value: Nil, end | Nil end",
            "extern rust \"snacc_user_take\" fun take(value: Nil)\nprint(0)",
            "extern rust \"snacc_user_make\" fun make(): Nil\nprint(0)",
            "extern rust \"snacc_user_ref\" fun update(value: Ref<Nil>)\nprint(0)",
            &format!("{POINT}method Point.at(value: Nil) do print(1) end"),
            &format!("{POINT}method Point.at(): Nil do nil end"),
        ] {
            assert_error_contains(source, "'Nil' is not a standalone type");
        }
    }

    /// Specification 012 conformance 17: `nil` needs one expected
    /// Nil-containing union. Specification 020 section 8 removes `null` as an
    /// alternate spelling entirely.
    #[test]
    fn contextual_nil_requires_one_nil_containing_union() {
        assert_checks(
            "type Maybe is union | Some is struct value: Int64, end | Nil end\n\
             let missing: Maybe = nil\n\
             fun absent(): Maybe do nil end\n\
             fun take(value: Maybe): Bool do value == nil end\n\
             print(take(absent()))",
        );
        for source in ["print(nil)", "print(nil == nil)"] {
            assert_error_contains(source, "'nil' has no type of its own");
        }
        assert_error_contains("let value: Int64 = nil", "expected 'Int64', found 'Nil'");
    }

    /// Specification 020 section 8: `null` is an ordinary identifier with no
    /// built-in meaning, so an unresolved use gets the same diagnostic as any
    /// other undeclared name -- never a `nil`-specific one.
    #[test]
    fn null_is_an_ordinary_undeclared_identifier_not_a_nil_spelling() {
        assert_error_contains("print(null)", "No such variable 'null' in scope");
        assert_checks("let null: Int64 = 10\nprint(null)");
    }

    /// Conformance 4 and 17 diagnostics: duplicate fields and members.
    #[test]
    fn duplicate_fields_and_union_members_are_rejected() {
        assert_error_contains(
            "type Point is struct x: Float64, x: Float64, end",
            "Field 'Point.x' already exists",
        );
        assert_error_contains(
            "type Shape is union | Circle | Circle end",
            "Union member 'Shape.Circle' already exists",
        );
    }

    /// Conformance 14 and 18: field access needs a struct, and printing a
    /// user-defined type is rejected rather than silently accepted.
    #[test]
    fn field_access_and_printing_reject_unsupported_receivers() {
        assert_error_contains(
            "let value: Int64 = 1\nprint(value.field)",
            "'Int64' is not a struct, so it has no field 'field'",
        );
        assert_error_contains(
            &format!("{SHAPE}let shape: Shape = Shape.Circle(radius: 1)\nprint(shape.radius)"),
            "'Shape' is not a struct, so it has no field 'radius'",
        );
        assert_error_contains(
            &format!("{POINT}print(Point(x: 1.0, y: 2.0))"),
            "'print' does not support the user-defined type 'Point'",
        );
        assert_error_contains(
            "type UserId is Int64\nprint(UserId(1))",
            "'print' does not support the user-defined type 'UserId'",
        );
        assert_checks("type UserId is Int64\nprint(Int64(UserId(1)))");
    }

    /// Conformance 25: user-defined types combine with RFC 008 no-result
    /// functions and Specification 009 scalar fields.
    #[test]
    fn user_types_combine_with_no_result_functions_and_fixed_width_scalars() {
        assert_checks(
            "type Pixel is struct red: Byte, green: Byte, blue: Byte, ratio: Float32, end\n\
             method Pixel.brightest(): Byte do self.red end\n\
             fun announce(value: Byte) do print(value) end\n\
             let pixel: Pixel = Pixel(red: 1u8, green: 2u8, blue: 3u8, ratio: 0.5f32)\n\
             announce(pixel.brightest())\n\
             print(pixel.ratio)",
        );
    }

    /// A method on a union receiver takes the whole union value.
    #[test]
    fn methods_attach_to_unions_and_to_their_members() {
        assert_checks(&format!(
            "{SHAPE}\
             method Shape.Circle.area(): Int64 do self.radius * self.radius end\n\
             method Shape.describe(): Int64 do \
                if self is Shape.Circle(circle) then circle.area() \
                elseif self is Shape.Rectangle(rectangle) then \
                rectangle.length * rectangle.width end end\n\
             let shape: Shape = Shape.Circle(radius: 2)\nprint(shape.describe())"
        ));
        // A member method is not callable on the union.
        assert_error_contains(
            &format!(
                "{SHAPE}method Shape.Circle.area(): Int64 do self.radius end\n\
                 let shape: Shape = Shape.Circle(radius: 2)\nprint(shape.area())"
            ),
            "'Shape' has no method 'area'",
        );
    }

    // ---------------------------------------------------------------------
    // Specification 011: call-scoped reference parameters.
    // ---------------------------------------------------------------------

    const ADD_INTO: &str =
        "fun add_into(x: Int64, y: Int64, result: Ref<Int64>) do result = x + y end\n";
    const EXCHANGE: &str = "fun exchange(left: Ref<Float64>, right: Ref<Float64>) do \
                            let saved: Float64 = left left = right right = saved end\n";

    /// The checked arguments of the first top-level call statement.
    fn top_level_args(program: &Program) -> &[TArg] {
        program
            .body
            .statements
            .iter()
            .find_map(|statement| match statement {
                TStmt::Call(_, args) | TStmt::Expr(TExpr::Call(_, args)) => Some(args.as_slice()),
                TStmt::MethodCall(call) => Some(call.args.as_slice()),
                _ => None,
            })
            .expect("the top-level body contains a call")
    }

    /// Conformance 1-2: the canonical example checks, and its reference
    /// argument survives into the checked program as a resolved place.
    #[test]
    fn a_reference_parameter_carries_a_resolved_place_to_lowering() {
        let program = assert_checks(&format!(
            "{ADD_INTO}let x: Int64 = 20\nlet y: Int64 = 22\n\
             let mut z: Int64 = 0\nadd_into(x, y, z)\nprint(z)"
        ));
        assert_eq!(
            program.funcs["add_into"].params[2].mode,
            ParamMode::Reference
        );
        assert_eq!(program.funcs["add_into"].params[2].ty, Ty::Int64);
        assert_eq!(program.funcs["add_into"].params[0].mode, ParamMode::Value);
        let args = top_level_args(&program);
        assert!(matches!(args[0], TArg::Value(_)));
        assert!(matches!(args[1], TArg::Value(_)));
        let TArg::Reference(place) = &args[2] else {
            panic!("the third argument should be a reference place");
        };
        assert_eq!(place.root, PlaceRoot::Local("z".into()));
        assert_eq!(place.ty, Ty::Int64);
        assert!(place.path.is_empty());
    }

    /// Conformance 1 and 7.2: inside the callee a reference parameter is an
    /// ordinary mutable root, so it reads, writes, and selects fields through
    /// the same machinery a `let mut` local uses.
    #[test]
    fn a_reference_parameter_is_a_mutable_root_of_its_referent_type() {
        assert_checks("fun increment(value: Ref<Int64>) do value = value + 1 end\nprint(0)");
        // A by-value parameter is still immutable, so the mode -- not the
        // parameter position -- is what grants the capability.
        assert_error_contains(
            "fun increment(value: Int64) do value = value + 1 end\nprint(0)",
            "'value' is not declared 'mut' and cannot be assigned",
        );
    }

    /// Conformance 3: only an initialized mutable place can establish a
    /// reference.
    #[test]
    fn rejects_every_reference_argument_that_is_not_a_mutable_place() {
        assert_error_contains(
            &format!("{ADD_INTO}let total: Int64 = 0\nadd_into(20, 22, total)"),
            "'total' is not declared 'mut', so it cannot be passed to the reference \
             parameter 'result' of 'add_into'",
        );
        for argument in ["0", "make_total()", "1 + 2"] {
            assert_error_contains(
                &format!(
                    "{ADD_INTO}fun make_total(): Int64 do 0 end\nadd_into(20, 22, {argument})"
                ),
                "requires an initialized mutable place of type 'Int64', but this argument \
                 is a value with no storage",
            );
        }
    }

    /// Conformance 4: there is no uninitialized declaration, and an initialized
    /// mutable local is the operational output form.
    #[test]
    fn a_referent_is_always_an_initialized_mutable_local() {
        assert_rejected_by_parser("let result: Int64");
        assert_checks(&format!(
            "{ADD_INTO}let mut result: Int64 = 0\nadd_into(20, 22, result)\nprint(result)"
        ));
    }

    /// Conformance 5 and 9: a field place rooted at a mutable variable is a
    /// valid referent, and the callee reaches the caller's field through it.
    #[test]
    fn struct_fields_rooted_at_a_mutable_variable_are_valid_referents() {
        let program = assert_checks(&format!(
            "{POINT}fun move_right(point: Ref<Point>, amount: Float64) do \
             point.x = point.x + amount end\n\
             fun bump(value: Ref<Float64>) do value = value + 1.0 end\n\
             let mut point: Point = Point(x: 1.0, y: 2.0)\n\
             bump(point.x)\n\
             move_right(point, 1.0)"
        ));
        let TArg::Reference(place) = &top_level_args(&program)[0] else {
            panic!("expected a reference argument");
        };
        assert_eq!(place.root, PlaceRoot::Local("point".into()));
        assert_eq!(place.path, vec![0]);
        assert_eq!(place.ty, Ty::Float64);
        // An immutable root is rejected even when the field itself is written.
        assert_error_contains(
            &format!(
                "{POINT}fun bump(value: Ref<Float64>) do value = value + 1.0 end\n\
                 let point: Point = Point(x: 1.0, y: 2.0)\nbump(point.x)"
            ),
            "'point' is not declared 'mut'",
        );
    }

    /// Conformance 6: the referent type is exact. Neither the `Int64`-to-`Float64`
    /// widening nor represented-type equivalence establishes a reference.
    #[test]
    fn a_reference_argument_requires_the_exact_referent_type() {
        assert_error_contains(
            "fun scale(value: Ref<Float64>) do value = value * 2.0 end\n\
             let mut count: Int64 = 1\nscale(count)",
            "reference parameter 'value' of 'scale' requires a place of exactly type \
             'Float64', found 'Int64'",
        );
        // The same value widens happily when the parameter is by value.
        assert_checks(
            "fun scale(value: Float64): Float64 do value * 2.0 end\n\
             let count: Int64 = 1\nprint(scale(count))",
        );
        assert_error_contains(
            "type UserId is Int64\nfun bump(value: Ref<Int64>) do value = value + 1 end\n\
             let mut id: UserId = UserId(1)\nbump(id)",
            "requires a place of exactly type 'Int64', found 'UserId'",
        );
        assert_error_contains(
            "type UserId is Int64\nfun bump(value: Ref<UserId>) do value = UserId(1) end\n\
             let mut raw: Int64 = 1\nbump(raw)",
            "requires a place of exactly type 'UserId', found 'Int64'",
        );
    }

    /// Conformance 7 and 11: an automatic read produces `T`, after which every
    /// ordinary rule for `T` applies -- including widening and copying into a
    /// by-value parameter.
    #[test]
    fn an_automatic_read_behaves_exactly_like_a_loaded_value() {
        assert_checks(
            "fun show(value: Int64) do print(value) end\n\
             fun widen(value: Float64): Float64 do value end\n\
             fun use_all(value: Ref<Int64>): Bool do\n\
                 print(value)\n\
                 show(value)\n\
                 print(widen(value))\n\
                 value > 0\n\
             end\n\
             let mut count: Int64 = 1\nprint(use_all(count))",
        );
    }

    /// Conformance 8: a whole-value assignment through a reference works for
    /// every category of referent.
    #[test]
    fn assignment_through_a_reference_replaces_a_complete_value() {
        assert_checks(&format!(
            "{POINT}{SHAPE}type UserId is Int64\n\
             fun set_scalar(value: Ref<Int64>) do value = 7 end\n\
             fun set_represented(value: Ref<UserId>) do value = UserId(7) end\n\
             fun set_struct(value: Ref<Point>) do value = Point(x: 0.0, y: 0.0) end\n\
             fun set_union(value: Ref<Shape>) do value = Shape.Circle(radius: 1) end\n\
             let mut scalar: Int64 = 0\n\
             let mut id: UserId = UserId(0)\n\
             let mut point: Point = Point(x: 1.0, y: 1.0)\n\
             let mut shape: Shape = Shape.Circle(radius: 0)\n\
             set_scalar(scalar)\nset_represented(id)\nset_struct(point)\nset_union(shape)"
        ));
        assert_error_contains(
            "fun set(value: Ref<Int64>) do value = true end\nprint(0)",
            "expected 'Int64', found 'Bool'",
        );
    }

    /// Conformance 10 and 7.3: forwarding is a reborrow with no extra ceremony
    /// -- the parameter is already a mutable root, so the ordinary place and
    /// mutability rules accept it.
    #[test]
    fn forwarding_a_reference_parameter_reborrows_it() {
        let program = assert_checks(
            "fun increment(value: Ref<Int64>) do value = value + 1 end\n\
             fun twice(value: Ref<Int64>) do increment(value) increment(value) end\n\
             let mut count: Int64 = 0\ntwice(count)",
        );
        let TStmt::Call(_, args) = &program.funcs["twice"].body.statements[0] else {
            panic!("expected a forwarded call");
        };
        let TArg::Reference(place) = &args[0] else {
            panic!("a forwarded reference parameter stays a reference argument");
        };
        assert_eq!(place.root, PlaceRoot::Local("value".into()));
        // Forwarding a field of a referenced struct reborrows the same way.
        assert_checks(&format!(
            "{POINT}fun bump(value: Ref<Float64>) do value = value + 1.0 end\n\
             fun bump_x(point: Ref<Point>) do bump(point.x) end\nprint(0)"
        ));
        // A by-value parameter still cannot supply a reference.
        assert_error_contains(
            "fun increment(value: Ref<Int64>) do value = value + 1 end\n\
             fun twice(value: Int64) do increment(value) end\nprint(0)",
            "'value' is not declared 'mut'",
        );
    }

    /// Conformance 11: supplying a reference parameter to a by-value parameter
    /// copies the current referent instead of forwarding the reference.
    #[test]
    fn a_reference_parameter_supplied_by_value_is_copied() {
        let program = assert_checks(
            "fun show(value: Int64) do print(value) end\n\
             fun relay(value: Ref<Int64>) do show(value) end\nprint(0)",
        );
        let TStmt::Call(_, args) = &program.funcs["relay"].body.statements[0] else {
            panic!("expected a value call");
        };
        assert!(matches!(args[0], TArg::Value(_)));
    }

    /// Conformance 12-13: overlap is decided from resolved roots and field
    /// paths. Identical places and prefix relationships overlap; sibling fields
    /// do not.
    #[test]
    fn overlapping_reference_arguments_are_rejected_and_siblings_are_accepted() {
        assert_error_contains(
            &format!("{EXCHANGE}let mut value: Float64 = 1.0\nexchange(value, value)"),
            "reference arguments 'value' and 'value' overlap, so parameters 'left' and \
             'right' cannot both have exclusive access",
        );
        assert_error_contains(
            &format!(
                "{POINT}fun use_both(whole: Ref<Point>, part: Ref<Float64>) do \
                 part = whole.y end\n\
                 let mut point: Point = Point(x: 1.0, y: 2.0)\nuse_both(point, point.x)"
            ),
            "reference arguments 'point' and 'point.x' overlap",
        );
        // Two statically distinct fields of the same mutable struct are
        // disjoint, so both may be referenced at once.
        assert_checks(&format!(
            "{POINT}{EXCHANGE}let mut point: Point = Point(x: 1.0, y: 2.0)\n\
             exchange(point.x, point.y)"
        ));
        // Two different mutable roots are always disjoint.
        assert_checks(&format!(
            "{EXCHANGE}let mut a: Float64 = 1.0\nlet mut b: Float64 = 2.0\nexchange(a, b)"
        ));
    }

    /// Nested field paths overlap only through a common prefix.
    #[test]
    fn overlap_compares_complete_field_paths() {
        const NESTED: &str = "type Point is struct x: Float64, y: Float64, end\n\
                              type Entity is struct position: Point, velocity: Point, end\n";
        assert_checks(&format!(
            "{NESTED}{EXCHANGE}let mut entity: Entity = Entity(\
             position: Point(x: 0.0, y: 0.0), velocity: Point(x: 0.0, y: 0.0))\n\
             exchange(entity.position.x, entity.velocity.x)"
        ));
        assert_error_contains(
            &format!(
                "{NESTED}fun use_both(whole: Ref<Point>, part: Ref<Float64>) do \
                 part = whole.y end\n\
                 let mut entity: Entity = Entity(\
                 position: Point(x: 0.0, y: 0.0), velocity: Point(x: 0.0, y: 0.0))\n\
                 use_both(entity.position, entity.position.x)"
            ),
            "reference arguments 'entity.position' and 'entity.position.x' overlap",
        );
    }

    /// Conformance 14: an addressable receiver participates in overlap checking
    /// for the whole call, whether or not the method writes through `self`; a
    /// temporary receiver cannot overlap a caller place.
    #[test]
    fn a_method_receiver_participates_in_overlap_checking() {
        const READ_ONLY: &str = "type Point is struct x: Float64, y: Float64, end\n\
                                 method Point.give(other: Ref<Float64>) do other = self.x end\n";
        const WRITING: &str = "type Point is struct x: Float64, y: Float64, end\n\
                               method Point.take(other: Ref<Float64>) do self.x = other end\n";
        for source in [READ_ONLY, WRITING] {
            let name = if std::ptr::eq(source, READ_ONLY) {
                "give"
            } else {
                "take"
            };
            assert_error_contains(
                &format!(
                    "{source}let mut point: Point = Point(x: 1.0, y: 2.0)\npoint.{name}(point.y)"
                ),
                "overlaps the receiver 'point', which the method may access through 'self'",
            );
        }
        // An unrelated mutable place is accepted for either method.
        assert_checks(&format!(
            "{READ_ONLY}let mut total: Float64 = 0.0\n\
             let point: Point = Point(x: 1.0, y: 2.0)\npoint.give(total)"
        ));
        // A temporary receiver has independent storage.
        assert_checks(&format!(
            "{READ_ONLY}let mut total: Float64 = 0.0\nPoint(x: 1.0, y: 2.0).give(total)"
        ));
    }

    /// A `self`-rooted reference argument may be written by the callee, so it
    /// feeds the same receiver-write effect an assignment to `self` would.
    #[test]
    fn passing_a_self_rooted_place_by_reference_is_a_receiver_write() {
        assert_error_contains(
            &format!(
                "{POINT}fun bump(value: Ref<Float64>) do value = value + 1.0 end\n\
                 method Point.grow() do bump(self.x) end\n\
                 let point: Point = Point(x: 1.0, y: 2.0)\npoint.grow()"
            ),
            "may assign through 'self', so its receiver requires a mutable root",
        );
        assert_checks(&format!(
            "{POINT}fun bump(value: Ref<Float64>) do value = value + 1.0 end\n\
             method Point.grow() do bump(self.x) end\n\
             let mut point: Point = Point(x: 1.0, y: 2.0)\npoint.grow()"
        ));
    }

    /// Section 7.2: method lookup uses the referent type `T`, and a
    /// receiver-writing method is valid because the referent is a mutable root.
    #[test]
    fn methods_resolve_through_a_reference_parameter() {
        assert_checks(&format!(
            "{POINT}method Point.length(): Float64 do self.x + self.y end\n\
             method Point.reset() do self.x = 0.0 end\n\
             fun report(point: Ref<Point>) do\n\
                 print(point.length())\n\
                 point.reset()\n\
             end\n\
             let mut point: Point = Point(x: 1.0, y: 2.0)\nreport(point)"
        ));
        // A by-value parameter still cannot receive a receiver-writing method.
        assert_error_contains(
            &format!(
                "{POINT}method Point.reset() do self.x = 0.0 end\n\
                 fun report(point: Point) do point.reset() end\nprint(0)"
            ),
            "may assign through 'self', so its receiver requires a mutable root",
        );
    }

    /// Conformance 15: a value argument is read before the callee starts, so
    /// reading a place by value and referencing it in the same call is valid.
    #[test]
    fn the_same_place_may_be_read_by_value_and_referenced_in_one_call() {
        let program = assert_checks(
            "fun replace(previous: Int64, value: Ref<Int64>) do value = previous + 1 end\n\
             let mut number: Int64 = 4\nreplace(number, number)",
        );
        let args = top_level_args(&program);
        assert!(matches!(args[0], TArg::Value(_)));
        assert!(matches!(args[1], TArg::Reference(_)));
    }

    /// Conformance 16-17: a reference is not storable and cannot be built,
    /// returned, or named anywhere but a parameter.
    #[test]
    fn a_reference_is_never_storable_or_constructible() {
        for source in [
            "fun f(value: Ref<Int64>): Ref<Int64> do value end",
            "let saved: Ref<Int64> = 1",
            "type Holder is struct value: Ref<Int64>, end",
            "type Alias is Ref<Int64>",
            "type Shape is union | Circle is struct radius: Ref<Int64>, end | Nil end",
            "fun f(value: Ref<Ref<Int64>>) do print(1) end",
            "method Point.f(self: Ref<Point>) do print(1) end",
            // No constructor, and `Ref` is reserved so it is not a call head.
            "let saved: Int64 = Ref(1)",
        ] {
            assert_rejected_by_parser(source);
        }
        // Returning a reference parameter returns a copy of its referent, which
        // is an ordinary value result.
        assert_checks(
            "fun read(value: Ref<Int64>): Int64 do value end\n\
             let mut count: Int64 = 1\nprint(read(count))",
        );
    }

    /// Reference parameters compose with methods and with Rust bridges.
    #[test]
    fn reference_parameters_are_permitted_on_methods_and_bridges() {
        assert_checks(&format!(
            "{POINT}method Point.give(other: Ref<Float64>) do other = self.x end\n\
             extern rust \"snacc_user_bump\" fun rust_bump(value: Ref<Int64>)\n\
             let mut total: Float64 = 0.0\nlet mut count: Int64 = 0\n\
             let point: Point = Point(x: 1.0, y: 2.0)\n\
             point.give(total)\nrust_bump(count)"
        ));
        // Specification 011 section 12.1: a bridge referent is an ABI scalar.
        assert_error_contains(
            &format!(
                "{POINT}extern rust \"snacc_user_move\" fun rust_move(point: Ref<Point>)\n\
                 print(0)"
            ),
            "only the ABI's permitted types may cross a Rust bridge",
        );
    }

    // ---------------------------------------------------------------------
    // Specification 018: inline sum types.
    // ---------------------------------------------------------------------

    /// Conformance 2-3: normalized member order and grouping never affect
    /// identity, so a differently written but member-set-identical sum is
    /// exactly assignable.
    #[test]
    fn sum_identity_ignores_member_order() {
        assert_checks("let a: Byte | Nil = nil\nlet b: Nil | Byte = a\nprint(0)");
    }

    #[test]
    fn sum_identity_ignores_parenthesized_grouping() {
        assert_checks("let a: Bool | Nil | Byte = nil\nlet b: (Byte | Bool) | Nil = a\nprint(0)");
    }

    /// Conformance 3: duplicates and fewer than two members are rejected.
    #[test]
    fn rejects_fewer_than_two_distinct_sum_members() {
        assert_error_contains(
            "let x: Byte | Byte = 1u8",
            "at least two distinct member types",
        );
    }

    #[test]
    fn rejects_a_repeated_member_after_flattening() {
        assert_error_contains(
            "let x: (Byte | Bool) | Byte = 1u8",
            "is repeated in this sum type",
        );
    }

    #[test]
    fn rejects_a_lone_nil_member() {
        assert_error_contains(
            "let x: Nil | Nil = nil",
            "valid in a sum type only alongside",
        );
    }

    /// Specification 018 section 4: an unresolved member is reported once,
    /// through the same "Unknown type" diagnostic every other position uses.
    #[test]
    fn an_unresolved_sum_member_is_reported_once() {
        assert_error_contains("let x: Foo | Nil = nil", "Unknown type 'Foo'");
    }

    /// Specification 020 section 11/12: `Dec64` has no built-in meaning or
    /// compatibility alias and is diagnosed like any other unknown type name,
    /// unless a user declaration independently defines it.
    #[test]
    fn stale_dec64_name_has_no_built_in_meaning() {
        assert_error_contains("let x: Dec64 = 1.0", "Unknown type 'Dec64'");
        assert_checks("type Dec64 is Int64\nlet x: Dec64 = Dec64(1)\nprint(0)");
    }

    /// Conformance 5: an inline sum has no callable type name, so it cannot be
    /// a represented type's immediate representation.
    #[test]
    fn an_inline_sum_cannot_be_a_represented_types_target() {
        assert_error_contains(
            "type MaybeByte is Byte | Nil",
            "cannot be a represented type's immediate representation",
        );
    }

    /// Specification 018 section 3: a reference is not a value-type member,
    /// so it fails to parse as one -- the parser establishes this, and the
    /// checker never sees a `Ref<T>` sum member.
    #[test]
    fn a_reference_cannot_be_a_sum_member() {
        assert_rejected_by_parser("let value: Ref<Byte> | Nil = nil");
    }

    /// Specification 018 section 8: a self-referential inline sum field has
    /// no more indirection than a plain self-referential field, so it is
    /// still an infinite value layout.
    #[test]
    fn a_self_referential_inline_sum_field_is_an_infinite_layout() {
        assert_error_contains(
            "type Node is struct next: Node | Nil, end",
            "has an infinite value layout",
        );
    }

    /// Conformance 8: a named union is one opaque direct member; its own
    /// members do not flatten into the inline sum.
    #[test]
    fn a_named_union_is_one_opaque_member_of_an_inline_sum() {
        assert_checks(&format!(
            "{SHAPE}fun classify(value: Shape | Nil): Int64 do \
             if value is Shape(shape) then \
                 if shape is Shape.Circle(circle) then circle.radius \
                 elseif shape is Shape.Rectangle(rectangle) then rectangle.length end \
             elseif value is Nil then 0 end end\n\
             print(classify(nil))"
        ));
    }

    #[test]
    fn a_sum_type_test_cannot_name_a_member_inside_a_named_union_member() {
        assert_error_contains(
            &format!(
                "{SHAPE}fun f(value: Shape | Nil): Int64 do \
                 if value is Shape.Circle(circle) then circle.radius \
                 elseif value is Nil then 0 end end"
            ),
            "a type test on an inline sum names exactly one direct member",
        );
    }

    /// Conformance 4: a direct value and a contextual `nil` both inject.
    #[test]
    fn direct_values_and_contextual_nil_inject_into_an_expected_sum() {
        let program =
            assert_checks("let present: Byte | Nil = 1u8\nlet absent: Byte | Nil = nil\nprint(0)");
        let TStmt::Let { value, .. } = &program.body.statements[0] else {
            panic!("expected a declaration");
        };
        assert!(matches!(
            value,
            TExpr::InjectSum {
                member: Ty::Byte,
                ..
            }
        ));
        let TStmt::Let { value, .. } = &program.body.statements[1] else {
            panic!("expected a declaration");
        };
        assert!(matches!(
            value,
            TExpr::InjectSum {
                member: Ty::Nil,
                ..
            }
        ));
    }

    /// Specification 018 section 5: an exact direct member match wins over an
    /// available widening conversion.
    #[test]
    fn an_exact_member_match_wins_over_widening() {
        let program = assert_checks("let x: Float64 | Int64 = 1\nprint(0)");
        let TStmt::Let { value, .. } = &program.body.statements[0] else {
            panic!("expected a declaration");
        };
        assert!(matches!(
            value,
            TExpr::InjectSum {
                member: Ty::Int64,
                ..
            }
        ));
    }

    /// With no exact `Int64` member, the value still widens through the one
    /// existing implicit conversion.
    #[test]
    fn an_int64_value_widens_into_the_sums_float64_member() {
        let program = assert_checks("let x: Float64 | Nil = 1\nprint(0)");
        let TStmt::Let { value, .. } = &program.body.statements[0] else {
            panic!("expected a declaration");
        };
        let TExpr::InjectSum { member, value, .. } = value else {
            panic!("expected a sum injection");
        };
        assert_eq!(*member, Ty::Float64);
        assert!(matches!(**value, TExpr::Cast(_, Ty::Float64)));
    }

    #[test]
    fn rejects_a_value_with_no_matching_sum_member() {
        assert_error_contains("let x: Bool | Byte = 1.5", "found 'Float64'");
    }

    /// Section 5: a named union's own member does not directly inject into an
    /// inline sum; only an already-`Shape`-typed value does.
    #[test]
    fn a_named_unions_member_value_does_not_directly_inject_into_an_inline_sum() {
        assert_error_contains(
            &format!("{SHAPE}let combined: Shape | Nil = Shape.Circle(radius: 1)"),
            "found 'Shape.Circle'",
        );
        assert_checks(&format!(
            "{SHAPE}let shape: Shape = Shape.Circle(radius: 1)\n\
             let combined: Shape | Nil = shape\nprint(0)"
        ));
    }

    /// Conformance 6: sum-to-sum assignment requires identical normalized
    /// member sets; there is no subset-to-superset conversion.
    #[test]
    fn sum_to_sum_assignment_requires_identical_member_sets() {
        assert_checks("let a: Byte | Nil = nil\nlet b: Byte | Nil = a\nprint(0)");
        assert_error_contains(
            "let narrow: Byte | Nil = nil\nlet wide: Bool | Nil | Byte = narrow",
            "expected 'Bool | Nil | Byte', found 'Nil | Byte'",
        );
    }

    /// Conformance 7: type tests bind the exact tested member and support an
    /// exhaustive chain with no `else`.
    #[test]
    fn is_tests_bind_the_exact_member_and_support_exhaustive_chains() {
        let program = assert_checks(
            "fun describe(value: Byte | Nil): Byte do \
             if value is Byte(byte) then byte elseif value is Nil then 0u8 end end\n\
             print(describe(nil))",
        );
        let result = program.funcs["describe"]
            .body
            .result
            .as_ref()
            .expect("describe produces a value");
        let TExpr::If(form) = result else {
            panic!("expected a value-form if");
        };
        assert!(form.exhaustive);
        assert!(form.else_branch.is_none());
        let TCondition::SumTest(first) = &form.arms[0].0 else {
            panic!("expected a sum type test");
        };
        assert_eq!(first.member, Ty::Byte);
        assert_eq!(first.binding.as_ref().map(|(_, ty)| *ty), Some(Ty::Byte));
    }

    #[test]
    fn a_non_exhaustive_sum_chain_without_an_else_is_rejected() {
        assert_error_contains(
            "fun describe(value: Byte | Nil): Byte do \
             if value is Byte(byte) then byte end end",
            "does not handle Nil",
        );
    }

    #[test]
    fn a_duplicate_sum_branch_is_rejected() {
        assert_error_contains(
            "fun describe(value: Byte | Nil): Byte do \
             if value is Byte(byte) then byte \
             elseif value is Byte(other) then other \
             elseif value is Nil then 0u8 end end",
            "'Byte' is already handled by an earlier branch",
        );
    }

    #[test]
    fn an_exhaustive_sum_chain_rejects_a_redundant_else() {
        assert_error_contains(
            "fun describe(value: Byte | Nil): Byte do \
             if value is Byte(byte) then byte \
             elseif value is Nil then 0u8 \
             else 1u8 end end",
            "already covers every direct member",
        );
    }

    #[test]
    fn a_sum_type_test_cannot_name_a_nonmember() {
        assert_error_contains(
            "fun f(value: Byte | Nil): Int64 do \
             if value is Bool(b) then 1 elseif value is Nil then 0 end end",
            "'Bool' is not a direct member of",
        );
    }

    #[test]
    fn nil_cannot_be_bound_in_a_sum_type_test() {
        assert_error_contains(
            "fun f(value: Byte | Nil): Byte do \
             if value is Byte(byte) then byte elseif value is Nil(x) then 0u8 end end",
            "'Nil' carries no value, so it cannot be bound by a type test",
        );
    }

    /// Section 7: an explicit expected sum injects different branch values
    /// without synthesizing a new type.
    #[test]
    fn an_expected_sum_permits_different_branch_types_without_synthesis() {
        assert_checks(
            "fun maybe_byte(found: Bool): Byte | Nil do \
             if found then 1u8 else nil end end\n\
             print(0)",
        );
    }

    /// Conformance 9: equality follows every member; unsupported operations on
    /// the whole sum decompose first.
    #[test]
    fn sums_support_equality_when_every_member_does() {
        assert_checks(
            "let a: Byte | Nil = nil\nlet b: Byte | Nil = 1u8\nprint(a == b)\nprint(a != b)",
        );
    }

    #[test]
    fn a_sum_value_compares_against_contextual_nil() {
        assert_checks("let a: Byte | Nil = 1u8\nprint(a == nil)");
    }

    #[test]
    fn unsupported_operations_on_a_whole_sum_are_rejected() {
        let base = "let a: Byte | Nil = nil\nlet b: Byte | Nil = nil\n";
        assert_error_contains(
            &format!("{base}print(a < b)"),
            "operands must be two numbers of the same type",
        );
        assert_error_contains(
            &format!("{base}print(a + b)"),
            "operands must be two numbers of the same type",
        );
        assert_error_contains(
            &format!("{base}print(a.value)"),
            "is not a struct, so it has no field",
        );
        assert_error_contains(
            &format!("{base}print(a)"),
            "'print' does not support the inline sum type",
        );
    }

    /// Conformance 11: an inline sum cannot cross a Rust bridge, even when
    /// every member individually could.
    #[test]
    fn an_inline_sum_cannot_cross_a_rust_bridge() {
        assert_error_contains(
            "extern rust \"snacc_user_maybe\" fun maybe(): Byte | Nil\nprint(0)",
            "no inline sum may cross a Rust bridge",
        );
    }

    /// A struct field may hold an inline sum, and construction injects into it
    /// exactly like any other expected-sum position.
    #[test]
    fn a_struct_field_may_be_an_inline_sum() {
        assert_checks(
            "type CacheEntry is struct value: Byte | Nil, end\n\
             let empty: CacheEntry = CacheEntry(value: nil)\n\
             let full: CacheEntry = CacheEntry(value: 1u8)\nprint(0)",
        );
    }

    // RFC 016 Task A: `Box<T>` syntax, resolved types, and layout.

    /// Specification 016 sections 4.1 and 12 (phase 1): `Box<T>` resolves as
    /// an ordinary storable value type in every position a plain value type
    /// can occupy -- field, parameter, local, and result -- and round-trips
    /// through the full `check()` pipeline into a resolved `Ty::Box`.
    #[test]
    fn a_box_type_resolves_as_a_field_parameter_local_and_result_type() {
        let program = assert_checks(
            "type Point is struct x: Int64, end\n\
             type Holder is struct value: Box<Point>, end\n\
             fun make(value: Box<Point>): Box<Point> do let local: Box<Point> = value local end\n\
             print(0)",
        );
        let holder = program
            .types
            .iter()
            .find(|def| def.name() == "Holder")
            .expect("Holder exists");
        assert!(
            matches!(holder.fields().unwrap()[0].1, Ty::Box(_)),
            "Holder.value should resolve to a 'Ty::Box'"
        );
        let make = &program.funcs["make"];
        assert!(
            matches!(make.params[0].ty, Ty::Box(_)),
            "make's parameter should resolve to a 'Ty::Box'"
        );
        assert!(
            matches!(make.result, Some(Ty::Box(_))),
            "make's result should resolve to a 'Ty::Box'"
        );
    }

    /// Specification 016 section 11: the wrong number of `Box` type
    /// arguments is diagnosed. `Box<T>` accepts exactly one type argument
    /// (section 4.1), so zero or more than one both fail to parse, the same
    /// way a malformed `Ref<T>` would.
    #[test]
    fn rejects_the_wrong_number_of_box_type_arguments() {
        assert_rejected_by_parser("let value: Box = box(1)");
        assert_rejected_by_parser("let value: Box<> = box(1)");
        assert_rejected_by_parser("let value: Box<Int64, Bool> = box(1)");
    }

    /// Specification 016 section 4.1: `Ref<T>` is not storable, so it cannot
    /// be a box's pointee -- like a reference nested inside a sum member
    /// (`a_reference_cannot_be_a_sum_member` above), this fails to parse
    /// rather than reaching resolution.
    #[test]
    fn a_reference_cannot_be_a_box_pointee() {
        assert_rejected_by_parser("let value: Box<Ref<Int64>> = box(1)");
    }

    /// Specification 016 section 4.1: a no-result type is not storable, so
    /// it cannot be a box's pointee. There is no `TypeRef` spelling for a
    /// no-result type, so the only way to attempt boxing one is
    /// `box(call-to-a-no-result-function())` -- rejected by the pre-existing
    /// no-result-call-as-value diagnostic (RFC 008 conformance 2) before any
    /// box-specific checking runs.
    #[test]
    fn a_no_result_calls_result_cannot_be_boxed() {
        assert_error_contains(
            "fun log(value: Int64) do print(value) end\n\
             let boxed: Box<Int64> = box(log(1))\nprint(0)",
            "declares no result, so its call cannot be used as a value",
        );
    }

    /// Specification 016 section 4.1: `Box<Box<T>>` is valid.
    #[test]
    fn nested_box_is_accepted() {
        assert_checks("let value: Box<Box<Int64>> = box(box(1))\nprint(0)");
    }

    /// Specification 016 section 5.1's worked example (also section 3's
    /// motivation): a recursive union crossing a `Box<T>` edge has a finite
    /// layout and checks; the identical shape with the edge unbroken is
    /// still an infinite value layout, exactly as before this RFC (compare
    /// `recursive_value_layouts_are_rejected` above).
    #[test]
    fn a_box_edge_breaks_an_otherwise_infinite_recursive_union_layout() {
        assert_checks(
            "type IntLink is union | Empty | Item is struct value: Int64, \
             next: Box<IntLink>, end end\nprint(0)",
        );
        assert_error_contains(
            "type IntLink is union | Empty | Item is struct value: Int64, \
             next: IntLink, end end\nprint(0)",
            "has an infinite value layout",
        );
    }

    /// Specification 016 section 5.3: a struct with a `Box<T>` field is
    /// move-only. `move_only_support`'s own fixed-point computation is
    /// already unit-tested directly in `types.rs`; this proves the property
    /// is reachable and correct on a realistic program through the same
    /// declaration-collection phase `check()` itself starts from (`check`
    /// calls `types::collect` as its first step).
    #[test]
    fn a_box_field_makes_its_struct_move_only_through_the_shared_pipeline() {
        let source = "type Holder is struct value: Box<Int64>, end\nprint(0)";
        assert_checks(source);
        let syntax =
            crate::parse(source).unwrap_or_else(|d| panic!("{source} should parse: {d:?}"));
        let mut errors = Vec::new();
        let collected = types::collect(&syntax, &mut errors);
        assert!(
            errors.is_empty(),
            "unexpected collection errors: {errors:?}"
        );
        let holder = collected.types.top_level("Holder").expect("Holder exists");
        assert!(
            collected.types.is_move_only(Ty::User(holder)),
            "a struct with a 'Box<T>' field must be move-only"
        );
    }

    /// Specification 016 section 4.1: `Box` is reserved, so it cannot be a
    /// user-declared type, callable, parameter, or local name -- like `self`
    /// (`self_is_rejected_outside_a_method` above), this fails to parse
    /// rather than reaching the checker at all.
    #[test]
    fn box_is_reserved_and_cannot_be_a_declared_name() {
        assert_rejected_by_parser("type Box is Int64");
        assert_rejected_by_parser("fun Box(): Int64 do 1 end");
        assert_rejected_by_parser("fun f(Box: Int64): Int64 do 1 end");
        assert_rejected_by_parser("fun f(): Int64 do let Box: Int64 = 1 1 end");
    }

    /// Specification 016 section 10: `Box<T>` and every type transitively
    /// containing one are rejected in `extern rust` parameters and results.
    /// A direct `Box<T>` parameter or result gets its own diagnostic naming
    /// the box type; a struct containing a `Box<T>` field is already caught
    /// by the pre-existing "no user-defined type crosses the bridge" rule
    /// (`user_defined_types_are_rejected_at_every_bridge_site` above), which
    /// rejects every `Ty::User` unconditionally regardless of its fields --
    /// this proves the transitive case is actually enforced, not merely
    /// claimed by the diagnostic text.
    #[test]
    fn box_and_every_type_transitively_containing_one_are_rejected_at_the_bridge() {
        assert_error_contains(
            "extern rust \"snacc_user_take\" fun take(value: Box<Int64>)",
            "is a box type; 'Box<T>' and every type transitively containing one",
        );
        assert_error_contains(
            "extern rust \"snacc_user_make\" fun make(): Box<Int64>",
            "is a box type; 'Box<T>' and every type transitively containing one",
        );
        assert_error_contains(
            "type Holder is struct value: Box<Int64>, end\n\
             extern rust \"snacc_user_take\" fun take(value: Holder)",
            "is a user-defined type",
        );
    }

    // RFC 016 Task B (first half): consuming-context classification and the
    // available/moved control-flow analysis (Specification 016 sections 6.1
    // and 6.2). `Box<Int64>` stands in for every move-only type here since
    // `Types::is_move_only` already has its own direct unit tests; these
    // exercise the dataflow pass, not move-only classification itself.

    /// Specification 016 section 6.1's worked example: a move-only value
    /// moves out of its root on a consuming use, and a later consuming use of
    /// the same root is rejected.
    #[test]
    fn using_a_moved_box_again_is_rejected() {
        assert_error_contains(
            "let first: Box<Int64> = box(1)\n\
             let second: Box<Int64> = first\n\
             let third: Box<Int64> = first\n\
             print(0)",
            "is already moved",
        );
    }

    /// Specification 016 section 6.2: a move confined to one non-exhaustive
    /// `if` arm, with no later use, is fine -- there is nothing after the
    /// merge to reject.
    #[test]
    fn a_move_inside_one_if_arm_with_no_later_use_is_accepted() {
        assert_checks(
            "let first: Box<Int64> = box(1)\n\
             if true then\n\
             \x20   let second: Box<Int64> = first\n\
             end\n\
             print(0)",
        );
    }

    /// Specification 016 section 6.2: a root moved on only one arm is not
    /// available on the un-taken fall-through predecessor, so -- per "available
    /// only when available on every reachable predecessor" -- it is unavailable
    /// after the merge and a later use is rejected.
    #[test]
    fn a_move_inside_one_if_arm_is_unavailable_after_the_merge() {
        assert_error_contains(
            "let first: Box<Int64> = box(1)\n\
             if true then\n\
             \x20   let second: Box<Int64> = first\n\
             end\n\
             let third: Box<Int64> = first\n\
             print(0)",
            "is already moved",
        );
    }

    /// Specification 016 section 6.2: a root moved on every arm of an
    /// exhaustive `if`/`else` is moved on every reachable predecessor, so it
    /// is definitely gone after the merge.
    #[test]
    fn a_move_on_every_if_else_arm_is_unavailable_after_the_merge() {
        assert_error_contains(
            "let first: Box<Int64> = box(1)\n\
             if true then\n\
             \x20   let second: Box<Int64> = first\n\
             else\n\
             \x20   let third: Box<Int64> = first\n\
             end\n\
             let fourth: Box<Int64> = first\n\
             print(0)",
            "is already moved",
        );
    }

    /// Specification 016 section 6.3's closing sentence: assigning a fresh
    /// value to a moved mutable local reinitializes it, so a later use
    /// succeeds.
    #[test]
    fn reassigning_a_moved_mutable_local_restores_availability() {
        assert_checks(
            "let mut first: Box<Int64> = box(1)\n\
             let second: Box<Int64> = first\n\
             first = box(2)\n\
             let third: Box<Int64> = first\n\
             print(0)",
        );
    }

    /// Specification 016 section 6.2: a move inside a `while` body that is
    /// never reinitialized would already be moved on a later iteration, so it
    /// is rejected even though the single pass through the body that moves it
    /// starts from an available root.
    #[test]
    fn a_move_inside_a_while_body_that_would_double_move_is_rejected() {
        assert_error_contains(
            "let first: Box<Int64> = box(1)\n\
             while true do\n\
             \x20   let second: Box<Int64> = first\n\
             end\n\
             print(0)",
            "already moved",
        );
    }

    /// Specification 016 section 6.3's closing sentence, inside a loop: a
    /// `while` body that reinitializes the moved root before the body ends is
    /// safe to repeat, so no double-move diagnostic fires.
    #[test]
    fn a_while_body_that_reinitializes_after_moving_is_accepted() {
        assert_checks(
            "let mut first: Box<Int64> = box(1)\n\
             while true do\n\
             \x20   let second: Box<Int64> = first\n\
             \x20   first = box(2)\n\
             end\n\
             print(0)",
        );
    }

    /// Specification 016 section 5.3: a copyable type is never move-only, so
    /// this analysis leaves it alone -- reusing the same local repeatedly is
    /// an ordinary copy, exactly as it was before this task existed.
    #[test]
    fn copyable_values_may_be_used_repeatedly_without_move_tracking() {
        assert_checks(
            "let first: Int64 = 1\n\
             let second: Int64 = first\n\
             let third: Int64 = first\n\
             print(third)",
        );
    }

    // RFC 016 Task B (second half): subplace-move rejection, overlapping
    // source/destination, union-test-binding aliasing through boxes, and
    // `Ref<T>` lending from `Box<T>` (Specification 016 sections 6.3, 6.4,
    // 7.2, and 7.3).

    const NODE: &str = "type Node is struct value: Int64, end\n";
    // `box(...)`'s operand is checked with no expected type (Specification
    // 016 section 4.2's evaluation rule takes whatever the operand
    // synthesizes), so `box(Tree.Branch(...))` directly would allocate a
    // `Box<Tree.Branch>`, not the intended `Box<Tree>` -- injecting a member
    // into its union already works before a box ever gets involved, so
    // `leaf` binds the plain `Tree` value (letting that existing injection
    // run) and boxes the already-widened result.
    const TREE: &str = "type Tree is union\n\
         | Empty\n\
         | Branch is struct value: Int64, left: Box<Tree>, right: Box<Tree>, end\n\
         end\n\
         fun leaf(): Box<Tree> do let payload: Tree = Tree.Empty() box(payload) end\n";

    /// Specification 016 section 6.4: a move-only struct field cannot be
    /// moved out of; only the complete root may be consumed.
    #[test]
    fn moving_a_move_only_struct_field_out_is_rejected() {
        assert_error_contains(
            "type Holder is struct value: Box<Int64>, end\n\
             let holder: Holder = Holder(value: box(1))\n\
             let taken: Box<Int64> = holder.value\n\
             print(0)",
            "cannot be moved out of",
        );
    }

    /// Specification 016 section 6.4, composed with section 4.3's automatic
    /// box dereference (Task B's item 6 verification): a move-only field
    /// reached only by crossing a box is still a subplace, not a root.
    #[test]
    fn moving_a_move_only_field_through_an_automatic_box_dereference_is_rejected() {
        assert_error_contains(
            "type Holder is struct value: Box<Int64>, end\n\
             let boxed: Box<Holder> = box(Holder(value: box(1)))\n\
             let taken: Box<Int64> = boxed.value\n\
             print(0)",
            "cannot be moved out of",
        );
    }

    /// Specification 016 section 7.3: a union-test binding is a branch-scoped
    /// alias to its tested place's active payload, never an independent
    /// owning root, so consuming it whole is a subplace move exactly like
    /// consuming one of its fields (below) already was.
    #[test]
    fn moving_a_union_test_binding_whole_is_rejected() {
        assert_error_contains(
            &format!(
                "{TREE}let payload: Tree = Tree.Branch(value: 1, left: leaf(), right: leaf())\n\
                 let tree: Box<Tree> = box(payload)\n\
                 if tree is Tree.Branch(branch) then\n\
                 let taken: Tree.Branch = branch\n\
                 end\n\
                 print(0)"
            ),
            "cannot be moved out of",
        );
    }

    /// Specification 016 section 7.3: a move-only field of a union-test
    /// binding is a subplace of the tested root, so moving it out is
    /// rejected the same way a field of an ordinary place is (Specification
    /// 016 section 6.4). This is the field-level counterpart to
    /// `moving_a_union_test_binding_whole_is_rejected` above.
    #[test]
    fn moving_a_field_out_of_a_box_wrapped_union_binding_is_rejected() {
        assert_error_contains(
            &format!(
                "{TREE}let payload: Tree = Tree.Branch(value: 1, left: leaf(), right: leaf())\n\
                 let mut tree: Box<Tree> = box(payload)\n\
                 if tree is Tree.Branch(branch) then\n\
                 let taken: Box<Tree> = branch.left\n\
                 end\n\
                 print(0)"
            ),
            "cannot be moved out of",
        );
    }

    /// Specification 016 section 6.4: rejecting a move out of a subplace
    /// does not prevent reading a sibling copyable field, borrowing or
    /// mutating the move-only subplace itself, or any of that through an
    /// automatic box dereference.
    #[test]
    fn reading_borrowing_and_mutating_a_move_only_subplace_is_still_accepted() {
        assert_checks(
            "type Holder is struct tag: Int64, value: Box<Int64>, end\n\
             fun touch(value: Ref<Box<Int64>>) do print(1) end\n\
             let mut holder: Holder = Holder(tag: 1, value: box(2))\n\
             print(holder.tag)\n\
             touch(holder.value)\n\
             holder.value = box(3)\n\
             let mut boxed: Box<Holder> = box(Holder(tag: 1, value: box(2)))\n\
             print(boxed.tag)\n\
             touch(boxed.value)\n\
             boxed.value = box(4)\n\
             print(0)",
        );
    }

    /// Specification 016 section 6.3: `value = value` is a move whose source
    /// overlaps its destination -- the source is available (this is its
    /// first use), so nothing else would reject it.
    #[test]
    fn assigning_a_move_only_place_to_itself_is_rejected() {
        assert_error_contains(
            "let mut first: Box<Int64> = box(1)\n\
             first = first\n\
             print(0)",
            "overlap",
        );
    }

    /// Specification 016 section 6.3: a destination that is a projection of
    /// its own source overlaps it too, not just an identical place. (The
    /// source here also fails the assignment's ordinary type check, since a
    /// field can never share its containing struct's exact type without
    /// going through a box and this RFC gives boxing its own consuming node
    /// rather than a bare place -- but the overlap diagnostic is independent
    /// of that and still fires.)
    #[test]
    fn assigning_a_container_from_its_own_projection_is_rejected() {
        assert_error_contains(
            "type Wrapper is struct inner: Box<Int64>, other: Box<Int64>, end\n\
             let mut w: Wrapper = Wrapper(inner: box(1), other: box(2))\n\
             w.inner = w\n\
             print(0)",
            "overlap",
        );
    }

    /// Specification 016 section 6.3: two different roots never overlap, so
    /// an ordinary reassignment between them is unaffected by the new checks.
    #[test]
    fn a_non_overlapping_move_only_reassignment_is_still_accepted() {
        assert_checks(
            "let mut a: Box<Int64> = box(1)\n\
             let b: Box<Int64> = box(2)\n\
             a = b\n\
             print(0)",
        );
    }

    /// Specification 016 section 7.3's worked example: testing a
    /// `Box<Tree>`-typed union member binds a branch-scoped alias whose
    /// fields may be read, mutated (the tested root is `mut`), and lent to a
    /// `Ref<T>` parameter -- `branch.left`/`branch.right` are `Box<Tree>`
    /// fields passed to a plain `Ref<Tree>` parameter, exercising Specification
    /// 016 section 7.2's automatic pointee lending through the alias.
    #[test]
    fn a_box_wrapped_union_binding_permits_read_borrow_and_mutation_through_a_mutable_root() {
        assert_checks(&format!(
            "{TREE}fun touch(node: Ref<Tree>) do print(1) end\n\
             let payload: Tree = Tree.Branch(value: 1, left: leaf(), right: leaf())\n\
             let mut tree: Box<Tree> = box(payload)\n\
             if tree is Tree.Branch(branch) then\n\
             print(branch.value)\n\
             touch(branch.left)\n\
             touch(branch.right)\n\
             branch.value = 2\n\
             end\n\
             print(0)"
        ));
    }

    /// Specification 016 section 7.3: the binding is mutable only when the
    /// tested place's root is. This is the same rule
    /// `a_type_test_binding_is_an_immutable_root` already covers for an
    /// unboxed union; this proves it still holds through a `Box<T>` subject.
    #[test]
    fn a_box_wrapped_union_binding_is_immutable_when_its_root_is_immutable() {
        assert_error_contains(
            &format!(
                "{TREE}let payload: Tree = Tree.Branch(value: 1, left: leaf(), right: leaf())\n\
                 let tree: Box<Tree> = box(payload)\n\
                 if tree is Tree.Branch(branch) then\n\
                 branch.value = 2\n\
                 end\n\
                 print(0)"
            ),
            "'branch' is not declared 'mut' and cannot be assigned",
        );
    }

    /// Specification 016 section 7.2's worked example: a `Box<T>` argument
    /// place automatically lends its pointee to a `Ref<T>` parameter, and the
    /// call may both read and mutate through the lent reference without
    /// consuming the box -- `node` is still usable afterward.
    #[test]
    fn a_box_argument_automatically_lends_its_pointee_to_a_ref_parameter() {
        let program = assert_checks(&format!(
            "{NODE}fun increment(node: Ref<Node>) do node.value = node.value + 1 end\n\
             let mut node: Box<Node> = box(Node(value: 10))\n\
             increment(node)\n\
             print(node.value)"
        ));
        let args = top_level_args(&program);
        let TArg::Reference(place) = &args[0] else {
            panic!("a boxed argument lent to 'Ref<T>' should stay a reference argument");
        };
        assert_eq!(place.root, PlaceRoot::Local("node".into()));
        assert!(place.path.is_empty());
        // The lent place's type is the pointee, matching the parameter's
        // referent type exactly, not the argument's own `Box<Node>` type.
        assert_eq!(place.ty, program.funcs["increment"].params[0].ty);
    }

    /// Specification 016 section 7.2: a `Box<T>` argument may instead bind to
    /// a declared `Ref<Box<T>>` parameter, borrowing the box itself rather
    /// than lending its pointee -- the declared parameter type disambiguates
    /// with no new inference, and this is just an exact-type match, so it
    /// needs no special-casing beyond what already exists.
    #[test]
    fn a_box_argument_binds_to_a_declared_ref_of_box_parameter_instead_of_lending_its_pointee() {
        let program = assert_checks(&format!(
            "{NODE}fun replace(node: Ref<Box<Node>>) do node = box(Node(value: 0)) end\n\
             let mut node: Box<Node> = box(Node(value: 10))\n\
             replace(node)\n\
             print(node.value)"
        ));
        let args = top_level_args(&program);
        let TArg::Reference(place) = &args[0] else {
            panic!("a 'Ref<Box<T>>' parameter should still receive a reference argument");
        };
        assert!(matches!(place.ty, Ty::Box(_)));
        assert_eq!(place.ty, program.funcs["replace"].params[0].ty);
    }

    /// Specification 016 section 7.2: "mutation requires a mutable
    /// originating root" applies to a lent pointee exactly as it already
    /// does to an ordinary `Ref<T>` argument.
    #[test]
    fn lending_a_box_argument_still_requires_its_root_to_be_declared_mut() {
        assert_error_contains(
            &format!(
                "{NODE}fun increment(node: Ref<Node>) do node.value = node.value + 1 end\n\
                 let node: Box<Node> = box(Node(value: 10))\n\
                 increment(node)\n\
                 print(0)"
            ),
            "'node' is not declared 'mut', so it cannot be passed to the reference parameter",
        );
    }

    /// Specification 011 section 6.4, extended through boxes: lending the
    /// same box's pointee to two parameters in the same call still overlaps,
    /// exactly like passing the same plain place twice already did.
    #[test]
    fn passing_the_same_boxed_pointee_twice_in_one_call_is_rejected_as_overlapping() {
        assert_error_contains(
            &format!(
                "{NODE}fun swap_values(a: Ref<Node>, b: Ref<Node>) do print(1) end\n\
                 let mut node: Box<Node> = box(Node(value: 10))\n\
                 swap_values(node, node)\n\
                 print(0)"
            ),
            "reference arguments 'node' and 'node' overlap",
        );
    }

    /// Specification 016 section 7.2's closing sentence: a borrowed
    /// allocation cannot also be moved out from under the same call.
    #[test]
    fn moving_a_boxed_argument_while_borrowing_its_pointee_in_the_same_call_is_rejected() {
        assert_error_contains(
            &format!(
                "{NODE}fun consume_and_peek(taken: Box<Node>, peek: Ref<Node>) do print(1) end\n\
                 let mut node: Box<Node> = box(Node(value: 10))\n\
                 consume_and_peek(node, node)\n\
                 print(0)"
            ),
            "overlaps the moved argument",
        );
    }

    // Specification 026: return statements. A small representative net, not
    // the specification's full conformance matrix.

    #[test]
    fn bare_return_exits_a_no_result_function_early() {
        let program = assert_checks(
            "fun print_if_positive(value: Int64) do\n\
             if value <= 0 then\n\
             return\n\
             end\n\
             print(value)\n\
             end",
        );
        let TStmt::If(form) = &program.funcs["print_if_positive"].body.statements[0] else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            form.arms[0].1.statements.as_slice(),
            [TStmt::Return { value: None, .. }]
        ));
    }

    #[test]
    fn valued_return_exits_a_result_declaring_function_early() {
        let program = assert_checks(
            "fun absolute(value: Int64): Int64 do\n\
             if value < 0 then\n\
             return 0 - value\n\
             end\n\
             value\n\
             end",
        );
        let TStmt::If(form) = &program.funcs["absolute"].body.statements[0] else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            form.arms[0].1.statements.as_slice(),
            [TStmt::Return { value: Some(_), .. }]
        ));
        // The trailing `value` still supplies the function's own result on
        // the path that never returns early.
        assert!(program.funcs["absolute"].body.result.is_some());
    }

    /// Section 6's `normalize` example: a returning branch supplies no value
    /// of its own and does not participate in the value-form `if`'s common
    /// result type, while the other branch still must.
    #[test]
    fn a_returning_if_branch_is_excluded_from_the_value_form_common_result_type() {
        let program = assert_checks(
            "fun normalize(value: Int64): Int64 do\n\
             if value < 0 then\n\
             return 0\n\
             else\n\
             value\n\
             end\n\
             end",
        );
        // Only the `else` branch reaches the callable end without a value of
        // its own, so this stays a value-form `if` (`TExpr::If`) in the
        // block's `result`, not a plain statement.
        let Some(TExpr::If(form)) = &program.funcs["normalize"].body.result else {
            panic!("expected a value-form if");
        };
        assert!(form.arms[0].1.result.is_none());
        assert!(form.else_branch.as_ref().unwrap().result.is_some());
    }

    /// Section 6's `bit` example: every reachable branch returning makes the
    /// `if` itself a callable return that supplies no value, even though it
    /// is the last element of a value-required body.
    #[test]
    fn an_if_whose_every_branch_returns_becomes_a_statement_with_no_result() {
        let program = assert_checks(
            "fun bit(flag: Bool): Int64 do\n\
             if flag then\n\
             return 1\n\
             else\n\
             return 0\n\
             end\n\
             end",
        );
        let body = &program.funcs["bit"].body;
        assert!(body.result.is_none());
        assert!(matches!(body.statements.as_slice(), [TStmt::If(_)]));
    }

    /// Specification 026 section 8: a value moved out through a conditional
    /// early return is unavailable only on the path that took it -- the
    /// fall-through path never moved it, so a later use on that path is
    /// still valid. This exercises the `check_if` move-state fix this
    /// specification requires: a returning arm's exit state must not be
    /// merged into the state after the `if`.
    #[test]
    fn a_move_confined_to_a_returning_if_branch_leaves_the_fallthrough_path_available() {
        let program = assert_checks(
            "fun grab(held: Bool, value: Box<Int64>): Box<Int64> do\n\
             if held then\n\
             return value\n\
             end\n\
             value\n\
             end",
        );
        assert!(program.funcs["grab"].body.result.is_some());
    }

    #[test]
    fn a_while_body_that_always_returns_still_leaves_the_loop_exit_reachable() {
        assert_checks(
            "fun first(): Int64 do\n\
             while true do\n\
             return 1\n\
             end\n\
             0\n\
             end",
        );
    }

    #[test]
    fn rejects_return_outside_a_function_or_method() {
        assert_error_contains(
            "return",
            "'return' is only valid inside a function or method",
        );
        assert_error_contains(
            "return 1",
            "'return' is only valid inside a function or method",
        );
    }

    #[test]
    fn rejects_a_bare_return_from_a_result_declaring_callable() {
        assert_error_contains(
            "fun f(): Int64 do return end",
            "declares a result of type 'Int64', so 'return' needs a value",
        );
    }

    #[test]
    fn rejects_a_value_return_from_a_no_result_callable() {
        assert_error_contains(
            "fun f() do return 1 end",
            "declares no result, so 'return' cannot return a value",
        );
    }

    #[test]
    fn rejects_a_returned_expression_not_assignable_to_the_declared_result() {
        assert_error_contains(
            "fun f(): Int64 do return true end",
            "expected 'Int64', found 'Bool'",
        );
    }

    /// Section 6's `incomplete` example: the path that does not return still
    /// reaches the callable end without a value.
    #[test]
    fn rejects_a_result_declaring_path_that_reaches_the_end_without_returning() {
        assert_error_contains(
            "fun incomplete(flag: Bool): Int64 do\n\
             if flag then\n\
             return 1\n\
             end\n\
             end",
            "requires an 'else' branch",
        );
    }

    #[test]
    fn rejects_source_after_an_unconditional_return() {
        assert_error_contains(
            "fun invalid(): Int64 do\n\
             return 1\n\
             print(2)\n\
             end",
            "unreachable",
        );
    }

    /// The same unreachable-source rule applies after an `if` for which
    /// every reachable branch returns, not only after a bare `return`.
    #[test]
    fn rejects_source_after_an_if_whose_every_branch_returns() {
        assert_error_contains(
            "fun invalid(flag: Bool): Int64 do\n\
             if flag then\n\
             return 1\n\
             else\n\
             return 0\n\
             end\n\
             print(2)\n\
             end",
            "unreachable",
        );
    }

    #[test]
    fn rejects_an_invalid_move_through_return() {
        assert_error_contains(
            "type Holder is struct value: Box<Int64>, end\n\
             fun take(holder: Holder): Box<Int64> do\n\
             return holder.value\n\
             end",
            "cannot be moved out of",
        );
    }
}
