//! Type identity, the type-definition table, and declaration collection.
//!
//! Specification 010 section 19 phase 2: top-level type names are collected in
//! deterministic source order and allocated stable [`TypeId`] values before any
//! body is resolved, union members are allocated afterwards, and the by-value
//! layout graph is checked for cycles before expression checking begins.

use crate::semantics::checker::{Error, TParam, Ty};
use crate::syntax::ast::{
    ExternFunc, Func, MethodDecl, Param, ParamMode, Program as AstProgram, Span, Spanned, TypeBody,
    TypeName, TypeRef,
};
use std::collections::HashMap;

/// An opaque nominal type identity, allocated in deterministic source order and
/// stable for one compilation. It indexes [`Types::defs`] directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

impl TypeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An opaque method identity, allocated in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodId(pub u32);

impl MethodId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An inline sum's identity: an index into [`SumTable`]'s interned,
/// normalized member sets (Specification 018 section 4). Never allocated
/// directly; always produced by [`SumTable::intern`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SumId(pub u32);

impl SumId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Interns each inline sum's canonical (sorted, deduplicated) member set so
/// `Byte | Nil` and `Nil | Byte` share one [`Ty::Sum`] identity (Specification
/// 018 section 4). Declaration collection (`Builder`) and per-function local
/// resolution (`resolve_type` in `checker.rs`) each hold one `SumTable` across
/// a whole compilation; `Builder`'s table moves into the finished [`Types`]
/// once collection ends, so ids stay stable and no sum is interned twice.
#[derive(Default)]
pub struct SumTable {
    members: Vec<Vec<Ty>>,
    index: HashMap<Vec<Ty>, SumId>,
}

impl SumTable {
    /// Interns an already-normalized (sorted, deduplicated, >= 2 members)
    /// member set, reusing an existing id for an identical set.
    pub fn intern(&mut self, members: Vec<Ty>) -> SumId {
        if let Some(id) = self.index.get(&members) {
            return *id;
        }
        let id = SumId(self.members.len() as u32);
        self.members.push(members.clone());
        self.index.insert(members, id);
        id
    }

    /// A sum's normalized direct members, in canonical (sorted) order. Never
    /// itself contains `Ty::Sum`: every member fed to [`Self::intern`] is
    /// already flattened by its caller.
    pub fn members(&self, id: SumId) -> &[Ty] {
        &self.members[id.index()]
    }
}

/// The result of comparing an inline sum's raw (possibly duplicated,
/// possibly unresolved) syntactic members, shared between declaration
/// collection and local resolution so both report Specification 018 section 4
/// identically.
pub(crate) struct SumOutcome {
    /// At least one member failed to resolve on its own; that failure was
    /// already reported, so the caller reports nothing further.
    pub any_unresolved: bool,
    /// Each repeated occurrence beyond a member's first, with the span to
    /// blame.
    pub duplicates: Vec<(Ty, Span)>,
    /// The distinct members, in first-occurrence order.
    pub distinct: Vec<Ty>,
}

/// Specification 018 section 4: flattening already expanded any nested sum
/// before this runs, so `raw` holds only non-`Sum` members (or `None` for one
/// that failed to resolve on its own).
pub(crate) fn dedupe_sum(raw: &[(Option<Ty>, Span)]) -> SumOutcome {
    let mut distinct: Vec<Ty> = Vec::new();
    let mut duplicates = Vec::new();
    let mut any_unresolved = false;
    for (ty, span) in raw {
        match ty {
            None => any_unresolved = true,
            Some(ty) if distinct.contains(ty) => duplicates.push((*ty, *span)),
            Some(ty) => distinct.push(*ty),
        }
    }
    SumOutcome {
        any_unresolved,
        duplicates,
        distinct,
    }
}

/// One resolved user-defined type. Every category is reached through
/// `Ty::User(TypeId)`; the category lives here, never in the `Ty` enum.
#[derive(Clone, Debug)]
pub enum TypeDef {
    /// `type N is T`. Nominally distinct from `T`.
    Represented { name: String, target: Ty },
    /// `type N is struct ... end`; fields in declaration order.
    Struct {
        name: String,
        fields: Vec<(String, Ty)>,
    },
    /// `type N is union ... end`; members in declaration order.
    Union { name: String, members: Vec<TypeId> },
    /// One union alternative, itself a nominal type. `tag` is its position in
    /// the containing union and is the deterministic runtime tag.
    UnionMember {
        /// The fully qualified name, `Union.Member`.
        name: String,
        union: TypeId,
        tag: u32,
        fields: Vec<(String, Ty)>,
        /// The special `Nil` alternative, which carries no value.
        nil: bool,
    },
}

impl TypeDef {
    /// The qualified name used in diagnostics.
    pub fn name(&self) -> &str {
        match self {
            Self::Represented { name, .. }
            | Self::Struct { name, .. }
            | Self::Union { name, .. }
            | Self::UnionMember { name, .. } => name,
        }
    }

    /// The declared fields, for the two categories that have them.
    pub fn fields(&self) -> Option<&[(String, Ty)]> {
        match self {
            Self::Struct { fields, .. } | Self::UnionMember { fields, .. } => Some(fields),
            Self::Represented { .. } | Self::Union { .. } => None,
        }
    }
}

/// A method's resolved signature. Bodies are checked separately.
pub struct MethodSig {
    pub receiver: TypeId,
    pub name: String,
    pub params: Vec<TParam>,
    pub result: Option<Ty>,
    /// Index into the syntax program's `methods`, so bodies can be checked.
    pub decl: usize,
}

impl MethodSig {
    pub fn qualified(&self, types: &Types) -> String {
        format!("{}.{}", types.def(self.receiver).name(), self.name)
    }
}

/// The resolved type table plus the name maps resolution needs.
pub struct Types {
    pub defs: Vec<TypeDef>,
    top_level: HashMap<String, TypeId>,
    members: HashMap<(TypeId, String), TypeId>,
    /// Memoized `==`/`!=` support, indexed by `TypeId`.
    equality: Vec<bool>,
    /// Every inline sum interned so far, continuing the ids `Builder` already
    /// allocated during declaration collection (Specification 018 section 4).
    sums: SumTable,
}

impl Types {
    pub fn def(&self, id: TypeId) -> &TypeDef {
        &self.defs[id.index()]
    }

    pub fn top_level(&self, name: &str) -> Option<TypeId> {
        self.top_level.get(name).copied()
    }

    pub fn member(&self, union: TypeId, name: &str) -> Option<TypeId> {
        self.members.get(&(union, name.to_string())).copied()
    }

    /// The union containing `id`, when `id` is a union member.
    pub fn containing_union(&self, id: TypeId) -> Option<TypeId> {
        match self.def(id) {
            TypeDef::UnionMember { union, .. } => Some(*union),
            _ => None,
        }
    }

    pub fn union_members(&self, id: TypeId) -> Option<&[TypeId]> {
        match self.def(id) {
            TypeDef::Union { members, .. } => Some(members),
            _ => None,
        }
    }

    /// The declaration index and type of `name` in `id`'s fields.
    pub fn field(&self, id: TypeId, name: &str) -> Option<(usize, Ty)> {
        let fields = self.def(id).fields()?;
        fields
            .iter()
            .position(|(field, _)| field == name)
            .map(|index| (index, fields[index].1))
    }

    /// The immediate representation of a represented type.
    pub fn represented_target(&self, id: TypeId) -> Option<Ty> {
        match self.def(id) {
            TypeDef::Represented { target, .. } => Some(*target),
            _ => None,
        }
    }

    /// The qualified name of any type, used in every diagnostic that names one.
    /// An inline sum renders as its normalized members joined by `" | "`
    /// (Specification 018 section 4); a member is never itself a sum, so this
    /// never recurses past one level.
    pub fn display(&self, ty: Ty) -> String {
        match ty {
            Ty::User(id) => self.def(id).name().to_string(),
            Ty::Sum(id) => self
                .sums
                .members(id)
                .iter()
                .map(|member| self.display(*member))
                .collect::<Vec<_>>()
                .join(" | "),
            scalar => scalar.to_string(),
        }
    }

    /// Specification 010 sections 7.3, 8.4, and 9.2: equality is supported when
    /// every contained type supports it. Memoized by resolved type ID.
    /// Specification 018 section 8 extends the same rule structurally to an
    /// inline sum's direct members.
    pub fn supports_equality(&self, ty: Ty) -> bool {
        match ty {
            Ty::User(id) => self.equality[id.index()],
            Ty::Sum(id) => self
                .sums
                .members(id)
                .iter()
                .all(|member| self.supports_equality(*member)),
            // Every scalar compares with itself.
            _ => true,
        }
    }

    /// An inline sum's normalized direct members (Specification 018 section
    /// 4), in canonical order.
    pub fn sum_members(&self, id: SumId) -> &[Ty] {
        self.sums.members(id)
    }

    /// Interns a normalized member set discovered during local `let`
    /// resolution, continuing the same table declaration collection built.
    pub fn intern_sum(&mut self, members: Vec<Ty>) -> SumId {
        self.sums.intern(members)
    }
}

/// Everything declaration collection produces.
pub struct Collected {
    pub types: Types,
    pub methods: Vec<MethodSig>,
    pub method_index: HashMap<(TypeId, String), MethodId>,
    /// Resolved signatures for `fun` and `extern rust` declarations.
    pub sigs: HashMap<String, FuncSig>,
}

#[derive(Clone)]
pub struct FuncSig {
    pub params: Vec<TParam>,
    pub result: Option<Ty>,
}

/// A partially built table: the name maps exist before any body is resolved so
/// field and represented types can refer to types declared later.
struct Builder {
    defs: Vec<Option<TypeDef>>,
    spans: Vec<Span>,
    top_level: HashMap<String, TypeId>,
    members: HashMap<(TypeId, String), TypeId>,
    /// Interned inline sums, moved into the finished [`Types`] once collection
    /// ends (Specification 018 section 4).
    sums: SumTable,
}

impl Builder {
    fn allocate(&mut self, name: String, span: Span) -> TypeId {
        let id = TypeId(self.defs.len() as u32);
        self.defs.push(None);
        self.spans.push(span);
        self.top_level.insert(name, id);
        id
    }

    fn allocate_member(&mut self, union: TypeId, simple: &str, span: Span) -> TypeId {
        let id = TypeId(self.defs.len() as u32);
        self.defs.push(None);
        self.spans.push(span);
        self.members.insert((union, simple.to_string()), id);
        id
    }
}

/// Specification 012 section 10 and section 13's "Standalone `Nil` type" row.
/// `Nil` is spelled only as a union member, which the parser recognizes on its
/// own (`Token::TyNil` in the union-member rule) and never routes through type
/// resolution, so every `TypeRef::Builtin(TypeName::Nil)` that reaches a
/// resolver is a standalone use and is rejected here.
pub const STANDALONE_NIL: &str = "'Nil' is not a standalone type; it is permitted only as a \
                                  member of a union that also declares a non-Nil member";

/// Specification 012 section 10 and section 13's "`nil` without one expected
/// Nil-containing union" row. `nil` names a union's `Nil` member, so a use with
/// no expected union type behind it -- `print(nil)`, `nil == nil` -- has no type
/// at all.
pub const CONTEXTLESS_NIL: &str = "'nil' has no type of its own; it is valid only where one \
                                   expected union type directly contains 'Nil'";

/// Specification 018 section 4: a source sum must contain at least two
/// distinct member types.
pub const SUM_TOO_FEW_MEMBERS: &str = "a sum type requires at least two distinct member types";

/// Specification 018 section 4: `Nil` is valid inside a sum type only
/// alongside at least one non-`Nil` member -- the same "needs a sibling" rule
/// Specification 012 section 10 already applies to a named union.
pub const NIL_NEEDS_A_SUM_SIBLING: &str =
    "'Nil' is valid in a sum type only alongside at least one non-'Nil' member";

/// Specification 018 section 4: a represented type is opened by calling its
/// named immediate representation type, and an inline sum has no type name
/// that can serve as that call head.
pub const SUM_AS_REPRESENTED_TARGET: &str = "an inline sum cannot be a represented type's immediate representation; \
     it has no callable type name to open it with";

/// Resolves one written type. Returns `None` after reporting an unresolved or
/// malformed path, a standalone `Nil`, or an invalid sum.
fn resolve(
    builder: &mut Builder,
    ty: &Spanned<TypeRef<'_>>,
    errors: &mut Vec<Error>,
) -> Option<Ty> {
    match &ty.0 {
        TypeRef::Builtin(TypeName::Nil) => {
            errors.push(Error {
                span: ty.1,
                msg: STANDALONE_NIL.to_string(),
            });
            None
        }
        TypeRef::Builtin(name) => Some(Ty::from(*name)),
        TypeRef::Named(segments) => {
            let first = segments[0].0;
            let Some(root) = builder.top_level.get(first).copied() else {
                errors.push(Error {
                    span: segments[0].1,
                    msg: format!("Unknown type '{first}'"),
                });
                return None;
            };
            match segments.len() {
                1 => Some(Ty::User(root)),
                2 => {
                    let (member, span) = segments[1];
                    match builder.members.get(&(root, member.to_string())) {
                        Some(id) => Some(Ty::User(*id)),
                        None => {
                            errors.push(Error {
                                span,
                                msg: format!("Unknown type '{first}.{member}'"),
                            });
                            None
                        }
                    }
                }
                _ => {
                    errors.push(Error {
                        span: ty.1,
                        msg: format!(
                            "Unknown type '{}'; a qualified type name has at most two components",
                            ty.0
                        ),
                    });
                    None
                }
            }
        }
        TypeRef::Sum(members) => resolve_sum(builder, members, ty.1, errors),
    }
}

/// The qualified name of any resolved type, for a builder whose declarations
/// are not yet all resolved (`None` still stands in for one being built).
fn defs_display(defs: &[Option<TypeDef>], sums: &SumTable, ty: Ty) -> String {
    match ty {
        Ty::User(id) => defs[id.index()]
            .as_ref()
            .map(|def| def.name().to_string())
            .unwrap_or_else(|| format!("<type #{}>", id.0)),
        Ty::Sum(id) => sums
            .members(id)
            .iter()
            .map(|member| defs_display(defs, sums, *member))
            .collect::<Vec<_>>()
            .join(" | "),
        scalar => scalar.to_string(),
    }
}

/// Specification 018 section 4: resolves every syntactic member, expanding a
/// nested sum (from a parenthesized group) into its own already-flattened
/// members, then applies the member-set rules shared with local `let`
/// resolution (`resolve_type` in `checker.rs`).
fn resolve_sum(
    builder: &mut Builder,
    members: &[Spanned<TypeRef<'_>>],
    span: Span,
    errors: &mut Vec<Error>,
) -> Option<Ty> {
    let mut raw: Vec<(Option<Ty>, Span)> = Vec::new();
    for member in members {
        // `resolve`'s `TypeRef::Builtin(TypeName::Nil)` arm always rejects a
        // standalone `Nil` because that arm is normally reached only by one;
        // `Nil` as a sum member is the valid, expected spelling this
        // specification adds, so it bypasses that rejection here instead of
        // going through `resolve`.
        if let TypeRef::Builtin(TypeName::Nil) = &member.0 {
            raw.push((Some(Ty::Nil), member.1));
            continue;
        }
        match resolve(builder, member, errors) {
            Some(Ty::Sum(id)) => {
                for flattened in builder.sums.members(id).to_vec() {
                    raw.push((Some(flattened), member.1));
                }
            }
            other => raw.push((other, member.1)),
        }
    }
    let outcome = dedupe_sum(&raw);
    if outcome.any_unresolved {
        return None;
    }
    for (ty, dup_span) in &outcome.duplicates {
        let name = defs_display(&builder.defs, &builder.sums, *ty);
        errors.push(Error {
            span: *dup_span,
            msg: format!("'{name}' is repeated in this sum type; each member must be distinct"),
        });
    }
    if outcome.distinct.len() < 2 {
        let msg = if outcome.distinct == [Ty::Nil] {
            NIL_NEEDS_A_SUM_SIBLING.to_string()
        } else {
            SUM_TOO_FEW_MEMBERS.to_string()
        };
        errors.push(Error { span, msg });
        return None;
    }
    let mut distinct = outcome.distinct;
    distinct.sort();
    Some(Ty::Sum(builder.sums.intern(distinct)))
}

/// The user types reachable by value from `def`, for layout cycle detection.
/// A sum-typed field reaches every one of its direct members that is itself a
/// user-defined type (Specification 018 section 8's layout requirement), so a
/// self-referential inline sum is caught exactly like a self-referential
/// field of the plain named-union form already is.
fn contained(def: &TypeDef, sums: &SumTable) -> Vec<TypeId> {
    let user = |ty: &Ty| -> Vec<TypeId> {
        match ty {
            Ty::User(id) => vec![*id],
            Ty::Sum(id) => sums
                .members(*id)
                .iter()
                .filter_map(|member| match member {
                    Ty::User(id) => Some(*id),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    match def {
        TypeDef::Represented { target, .. } => user(target),
        TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => {
            fields.iter().flat_map(|(_, ty)| user(ty)).collect()
        }
        TypeDef::Union { members, .. } => members.clone(),
    }
}

/// Three-state depth-first traversal over the by-value layout graph. Reports
/// the complete first cycle it finds, in traversal order.
fn reject_layout_cycles(
    defs: &[TypeDef],
    spans: &[Span],
    sums: &SumTable,
    errors: &mut Vec<Error>,
) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        InProgress,
        Done,
    }
    let mut state = vec![State::Unvisited; defs.len()];
    let mut stack: Vec<TypeId> = Vec::new();

    fn visit(
        id: TypeId,
        defs: &[TypeDef],
        spans: &[Span],
        sums: &SumTable,
        state: &mut Vec<State>,
        stack: &mut Vec<TypeId>,
        errors: &mut Vec<Error>,
    ) -> bool {
        match state[id.index()] {
            State::Done => return true,
            State::InProgress => {
                let start = stack
                    .iter()
                    .position(|entry| *entry == id)
                    .expect("an in-progress type is on the stack");
                let cycle = stack[start..]
                    .iter()
                    .chain(std::iter::once(&id))
                    .map(|entry| defs[entry.index()].name())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                errors.push(Error {
                    span: spans[id.index()],
                    msg: format!(
                        "Type '{}' has an infinite value layout: {cycle}",
                        defs[id.index()].name()
                    ),
                });
                return false;
            }
            State::Unvisited => {}
        }
        state[id.index()] = State::InProgress;
        stack.push(id);
        let mut ok = true;
        for next in contained(&defs[id.index()], sums) {
            if !visit(next, defs, spans, sums, state, stack, errors) {
                ok = false;
                break;
            }
        }
        stack.pop();
        state[id.index()] = State::Done;
        ok
    }

    let mut acyclic = true;
    for index in 0..defs.len() {
        if !visit(
            TypeId(index as u32),
            defs,
            spans,
            sums,
            &mut state,
            &mut stack,
            errors,
        ) {
            acyclic = false;
            break;
        }
    }
    acyclic
}

/// Computes equality support for every type once, after layout is known finite.
/// Specification 018 section 8 extends the same structural rule to a sum-typed
/// field: it supports equality only when every one of its direct members does.
fn equality_support(defs: &[TypeDef], sums: &SumTable) -> Vec<bool> {
    let mut memo: Vec<Option<bool>> = vec![None; defs.len()];

    // A sum member is never itself a sum (every member fed to `SumTable` is
    // already flattened), so this never recurses past one level for a sum.
    fn ty_supports(
        ty: &Ty,
        defs: &[TypeDef],
        sums: &SumTable,
        memo: &mut Vec<Option<bool>>,
    ) -> bool {
        match ty {
            Ty::User(inner) => solve(*inner, defs, sums, memo),
            Ty::Sum(inner) => sums
                .members(*inner)
                .to_vec()
                .iter()
                .all(|member| ty_supports(member, defs, sums, memo)),
            _ => true,
        }
    }

    fn solve(id: TypeId, defs: &[TypeDef], sums: &SumTable, memo: &mut Vec<Option<bool>>) -> bool {
        if let Some(known) = memo[id.index()] {
            return known;
        }
        // A cyclic layout is rejected before this runs; the guard keeps a
        // rejected program from recursing forever while other errors report.
        memo[id.index()] = Some(true);
        let supported = match &defs[id.index()] {
            TypeDef::Represented { target, .. } => ty_supports(target, defs, sums, memo),
            TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => fields
                .iter()
                .all(|(_, ty)| ty_supports(ty, defs, sums, memo)),
            TypeDef::Union { members, .. } => members
                .iter()
                .all(|member| solve(*member, defs, sums, memo)),
        };
        memo[id.index()] = Some(supported);
        supported
    }

    (0..defs.len())
        .map(|index| solve(TypeId(index as u32), defs, sums, &mut memo))
        .collect()
}

/// Resolves a parameter list. Duplicate names belong to the function-wide
/// binding check in the checker, not here.
fn resolve_params(
    builder: &mut Builder,
    params: &[Param<'_>],
    errors: &mut Vec<Error>,
) -> Vec<TParam> {
    let mut resolved = Vec::with_capacity(params.len());
    for param in params {
        // Specification 011 section 19 phase 1 step 4: the referent resolves
        // through ordinary type resolution, and the passing mode is stored
        // beside the resolved value type.
        let ty = resolve(builder, &param.ty, errors).unwrap_or(Ty::Nil);
        resolved.push(TParam {
            name: param.name.to_string(),
            ty,
            mode: param.mode,
        });
    }
    resolved
}

/// Specification 010 section 16: no user-defined type crosses the Rust bridge.
/// Specification 018 section 10 extends this to every inline sum, even one
/// whose members individually have bridge representations. Rejected here,
/// during declaration collection, so nothing downstream sees either.
fn reject_bridge_user_types(
    defs: &[Option<TypeDef>],
    sums: &SumTable,
    params: &[TParam],
    result: Option<Ty>,
    declaration: &ExternFunc<'_>,
    errors: &mut Vec<Error>,
) {
    let name = |ty: Ty| defs_display(defs, sums, ty);
    let crossing = params.iter().map(|param| param.ty).chain(result);
    for ty in crossing {
        let msg = match ty {
            Ty::User(_) => format!(
                "'{}' is a user-defined type; only the ABI's permitted types \
                 may cross a Rust bridge",
                name(ty)
            ),
            Ty::Sum(_) => format!(
                "'{}' is an inline sum type; no inline sum may cross a Rust bridge, even \
                 when every member individually has a bridge representation",
                name(ty)
            ),
            _ => continue,
        };
        errors.push(Error {
            span: declaration.span,
            msg,
        });
    }
    // Specification 011 section 12.1: a bridge reference refers only to a
    // permitted by-value scalar. Standalone `Nil` has no storage to refer to.
    for param in params
        .iter()
        .filter(|param| param.mode == ParamMode::Reference)
    {
        if param.ty == Ty::Nil {
            errors.push(Error {
                span: declaration.span,
                msg: format!(
                    "'Ref<Nil>' cannot cross a Rust bridge; a bridge reference refers \
                     only to a permitted ABI scalar, and '{}' is not one",
                    name(param.ty)
                ),
            });
        }
    }
}

/// Runs Specification 010's phase 2 in order: allocate, resolve, check layout,
/// then build the lookup tables.
pub fn collect(program: &AstProgram<'_>, errors: &mut Vec<Error>) -> Collected {
    let mut builder = Builder {
        defs: Vec::new(),
        spans: Vec::new(),
        top_level: HashMap::new(),
        members: HashMap::new(),
        sums: SumTable::default(),
    };

    // Step 1: every top-level type name, in source order.
    let mut declaration_ids = Vec::with_capacity(program.types.len());
    for declaration in &program.types {
        if let Some(existing) = builder.top_level.get(declaration.name) {
            let previous = builder.spans[existing.index()];
            errors.push(Error {
                span: declaration.name_span,
                msg: format!(
                    "Type '{}' already exists (first declared at {}..{})",
                    declaration.name, previous.start, previous.end
                ),
            });
            declaration_ids.push(None);
            continue;
        }
        let id = builder.allocate(declaration.name.to_string(), declaration.name_span);
        declaration_ids.push(Some(id));
    }

    // Step 2: every union member, after all top-level names exist.
    let mut member_ids: Vec<Vec<Option<TypeId>>> = Vec::with_capacity(program.types.len());
    for (declaration, id) in program.types.iter().zip(&declaration_ids) {
        let TypeBody::Union(members) = &declaration.body else {
            member_ids.push(Vec::new());
            continue;
        };
        let Some(union) = *id else {
            member_ids.push(members.iter().map(|_| None).collect());
            continue;
        };
        if members.len() > u32::MAX as usize {
            errors.push(Error {
                span: declaration.name_span,
                msg: format!(
                    "Union '{}' declares more than 2^32 members, which exceeds the 32-bit tag limit",
                    declaration.name
                ),
            });
        }
        let mut ids = Vec::with_capacity(members.len());
        for member in members {
            if builder
                .members
                .contains_key(&(union, member.name.to_string()))
            {
                errors.push(Error {
                    span: member.name_span,
                    msg: format!(
                        "Union member '{}.{}' already exists",
                        declaration.name, member.name
                    ),
                });
                ids.push(None);
                continue;
            }
            ids.push(Some(builder.allocate_member(
                union,
                member.name,
                member.name_span,
            )));
        }
        // Specification 012 section 10: `Nil` needs another member beside it.
        if members.iter().all(|member| member.nil) {
            errors.push(Error {
                span: declaration.name_span,
                msg: format!(
                    "Union '{}' contains only 'Nil'; 'Nil' requires another member type",
                    declaration.name
                ),
            });
        }
        member_ids.push(ids);
    }

    // Step 3: resolve every body now that all names exist.
    for ((declaration, id), members) in program.types.iter().zip(&declaration_ids).zip(&member_ids)
    {
        let Some(id) = *id else { continue };
        let name = declaration.name.to_string();
        let def = match &declaration.body {
            TypeBody::Represented(target_ref) => {
                let resolved = resolve(&mut builder, target_ref, errors);
                // Specification 018 section 4: a represented type is opened by
                // calling its named immediate representation type, and an
                // inline sum has no callable type name to open it with.
                if let Some(Ty::Sum(_)) = resolved {
                    errors.push(Error {
                        span: target_ref.1,
                        msg: SUM_AS_REPRESENTED_TARGET.to_string(),
                    });
                }
                let target = resolved.unwrap_or(Ty::Nil);
                TypeDef::Represented { name, target }
            }
            TypeBody::Struct(fields) => TypeDef::Struct {
                name,
                fields: resolve_fields(&mut builder, declaration.name, fields, errors),
            },
            TypeBody::Union(declared) => {
                // Tags are the member's source position, assigned deterministically
                // from zero (Specification 010 section 15.2).
                for (tag, (member, member_id)) in declared.iter().zip(members).enumerate() {
                    let Some(member_id) = *member_id else {
                        continue;
                    };
                    let qualified = format!("{}.{}", declaration.name, member.name);
                    let fields = resolve_fields(&mut builder, &qualified, &member.fields, errors);
                    builder.defs[member_id.index()] = Some(TypeDef::UnionMember {
                        name: qualified,
                        union: id,
                        tag: tag as u32,
                        fields,
                        nil: member.nil,
                    });
                }
                TypeDef::Union {
                    name,
                    members: members.iter().copied().flatten().collect(),
                }
            }
        };
        builder.defs[id.index()] = Some(def);
    }

    // A name that failed to resolve leaves a hole; fill it with an empty struct
    // so later phases still index the table safely.
    let defs: Vec<TypeDef> = builder
        .defs
        .iter()
        .enumerate()
        .map(|(index, def)| {
            def.clone().unwrap_or(TypeDef::Struct {
                name: format!("<unresolved type #{index}>"),
                fields: Vec::new(),
            })
        })
        .collect();

    // Step 4: infinite layout, before any expression is checked.
    let acyclic = reject_layout_cycles(&defs, &builder.spans, &builder.sums, errors);
    let equality = if acyclic {
        equality_support(&defs, &builder.sums)
    } else {
        vec![false; defs.len()]
    };

    // Step 5: callable signatures, bridge rejection, and call-head conflicts.
    let mut sigs: HashMap<String, FuncSig> = HashMap::new();
    let mut func_names: Vec<&str> = program.funcs.keys().copied().collect();
    func_names.sort_unstable();
    for name in func_names {
        let function: &Func<'_> = &program.funcs[name];
        let params = resolve_params(&mut builder, &function.args, errors);
        let result = function
            .ret
            .as_ref()
            .and_then(|ty| resolve(&mut builder, ty, errors));
        sigs.insert(name.to_string(), FuncSig { params, result });
    }
    let mut extern_names: Vec<&str> = program.externs.keys().copied().collect();
    extern_names.sort_unstable();
    for name in extern_names {
        let function: &ExternFunc<'_> = &program.externs[name];
        let params = resolve_params(&mut builder, &function.args, errors);
        let result = function
            .ret
            .as_ref()
            .and_then(|ty| resolve(&mut builder, ty, errors));
        reject_bridge_user_types(
            &builder.defs,
            &builder.sums,
            &params,
            result,
            function,
            errors,
        );
        sigs.insert(name.to_string(), FuncSig { params, result });
    }

    // Specification 010 section 6.1: one call head cannot mean two things.
    for declaration in &program.types {
        if sigs.contains_key(declaration.name) {
            errors.push(Error {
                span: declaration.name_span,
                msg: format!(
                    "Type '{}' shares a call head with the function or Rust bridge \
                     of the same name",
                    declaration.name
                ),
            });
        }
    }

    // Step 6: method receivers and signatures.
    let mut methods: Vec<MethodSig> = Vec::new();
    let mut method_index: HashMap<(TypeId, String), MethodId> = HashMap::new();
    for (index, declaration) in program.methods.iter().enumerate() {
        let Some(receiver) = method_receiver(&builder, declaration, errors) else {
            continue;
        };
        let (_, (name, name_span)) = declaration.split().expect("the receiver resolved");
        if method_index.contains_key(&(receiver, name.to_string())) {
            errors.push(Error {
                span: name_span,
                msg: format!(
                    "Method '{}.{name}' already exists; methods are not overloaded",
                    defs[receiver.index()].name()
                ),
            });
            continue;
        }
        let params = resolve_params(&mut builder, &declaration.args, errors);
        let result = declaration
            .ret
            .as_ref()
            .and_then(|ty| resolve(&mut builder, ty, errors));
        let id = MethodId(methods.len() as u32);
        method_index.insert((receiver, name.to_string()), id);
        methods.push(MethodSig {
            receiver,
            name: name.to_string(),
            params,
            result,
            decl: index,
        });
    }

    Collected {
        types: Types {
            defs,
            top_level: builder.top_level,
            members: builder.members,
            equality,
            sums: builder.sums,
        },
        methods,
        method_index,
        sigs,
    }
}

fn resolve_fields(
    builder: &mut Builder,
    owner: &str,
    fields: &[crate::syntax::ast::FieldDecl<'_>],
    errors: &mut Vec<Error>,
) -> Vec<(String, Ty)> {
    let mut resolved: Vec<(String, Ty)> = Vec::new();
    for field in fields {
        if resolved.iter().any(|(name, _)| name == field.name) {
            errors.push(Error {
                span: field.name_span,
                msg: format!("Field '{}.{}' already exists", owner, field.name),
            });
            continue;
        }
        let ty = resolve(builder, &field.ty, errors).unwrap_or(Ty::Nil);
        resolved.push((field.name.to_string(), ty));
    }
    resolved
}

/// Resolves a method's receiver path: `Type.method` or `Union.Member.method`.
fn method_receiver(
    builder: &Builder,
    declaration: &MethodDecl<'_>,
    errors: &mut Vec<Error>,
) -> Option<TypeId> {
    let Some((receiver, _)) = declaration.split() else {
        errors.push(Error {
            span: declaration.span,
            msg: "A method name is a receiver type followed by one method name".into(),
        });
        return None;
    };
    let (first, first_span) = receiver[0];
    let Some(root) = builder.top_level.get(first).copied() else {
        errors.push(Error {
            span: first_span,
            msg: format!("Unknown type '{first}'"),
        });
        return None;
    };
    match receiver.len() {
        1 => Some(root),
        2 => {
            let (member, span) = receiver[1];
            match builder.members.get(&(root, member.to_string())) {
                Some(id) => Some(*id),
                None => {
                    errors.push(Error {
                        span,
                        msg: format!("Unknown type '{first}.{member}'"),
                    });
                    None
                }
            }
        }
        _ => {
            errors.push(Error {
                span: declaration.span,
                msg: "A method receiver names a top-level type or one union member".into(),
            });
            None
        }
    }
}
