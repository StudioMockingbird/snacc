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

    /// Every interned sum's normalized members, indexed by `SumId`. Used once,
    /// at the end of checking, to hand the backend (Specification 018 Task B)
    /// a lowering-only snapshot of the table alongside the checked `Program`.
    pub fn all(&self) -> &[Vec<Ty>] {
        &self.members
    }
}

/// A boxed pointee's identity: an index into [`BoxTable`]'s interned pointee
/// types (Specification 016 section 4.1). Never allocated directly; always
/// produced by [`BoxTable::intern`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BoxId(pub u32);

impl BoxId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Interns each `Box<T>`'s pointee type, mirroring [`SumTable`] above: a
/// box's whole identity is its one pointee type (Specification 016 section
/// 4.1), so two occurrences of `Box<Int64>` share one [`Ty::Box`] id and no
/// separate member-set normalization is needed the way a sum needs.
#[derive(Default)]
pub struct BoxTable {
    pointees: Vec<Ty>,
    index: HashMap<Ty, BoxId>,
}

impl BoxTable {
    /// Interns a pointee type, reusing an existing id for an identical one.
    pub fn intern(&mut self, pointee: Ty) -> BoxId {
        if let Some(id) = self.index.get(&pointee) {
            return *id;
        }
        let id = BoxId(self.pointees.len() as u32);
        self.pointees.push(pointee);
        self.index.insert(pointee, id);
        id
    }

    /// The pointee type a box's members were built from.
    pub fn pointee(&self, id: BoxId) -> Ty {
        self.pointees[id.index()]
    }

    /// Every interned box's pointee type, indexed by `BoxId`. Used once, at
    /// the end of checking, to hand the backend (RFC 016 Task B/C) a
    /// lowering-only snapshot of the table alongside the checked `Program`,
    /// the same way [`SumTable::all`] does for inline sums.
    pub fn all(&self) -> &[Ty] {
        &self.pointees
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
    /// Memoized move-only status, indexed by `TypeId` (Specification 016
    /// section 5.3).
    move_only: Vec<bool>,
    /// Every inline sum interned so far, continuing the ids `Builder` already
    /// allocated during declaration collection (Specification 018 section 4).
    sums: SumTable,
    /// Every `Box<T>` pointee interned so far, continuing the ids `Builder`
    /// already allocated during declaration collection (Specification 016
    /// section 4.1).
    boxes: BoxTable,
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
            Ty::Box(id) => format!("Box<{}>", self.display(self.boxes.pointee(id))),
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
            // Specification 016 section 8.3: direct equality involving a box
            // is unsupported initially, regardless of whether the pointee
            // itself would support it.
            Ty::Box(_) => false,
            // Every scalar compares with itself.
            _ => true,
        }
    }

    /// Specification 016 section 5.3: `Box<T>` is unconditionally move-only
    /// regardless of `T`, and a struct or union is move-only when any field
    /// or member is -- computed once as a structural fixed point over the
    /// type dependency graph (`move_only_support`), the same way
    /// `supports_equality` is. Task A only records this property; ownership
    /// analysis (RFC 016 Task B) is the first consumer.
    pub fn is_move_only(&self, ty: Ty) -> bool {
        match ty {
            Ty::User(id) => self.move_only[id.index()],
            Ty::Sum(id) => self
                .sums
                .members(id)
                .iter()
                .any(|member| self.is_move_only(*member)),
            Ty::Box(_) => true,
            _ => false,
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

    /// Every interned inline sum's normalized member list, indexed by
    /// `SumId` (Specification 018 Task B): a lowering-only snapshot handed to
    /// the checked `Program` once checking finishes, so the backend does not
    /// need the rest of the type table -- only each sum's own member order --
    /// to build its LLVM layout and deterministic tags.
    pub fn all_sums(&self) -> &[Vec<Ty>] {
        self.sums.all()
    }

    /// A box's pointee type (Specification 016 section 4.1).
    pub fn box_pointee(&self, id: BoxId) -> Ty {
        self.boxes.pointee(id)
    }

    /// Interns a pointee type discovered during local `let` resolution,
    /// continuing the same table declaration collection built.
    pub fn intern_box(&mut self, pointee: Ty) -> BoxId {
        self.boxes.intern(pointee)
    }

    /// Every interned box's pointee type, indexed by `BoxId` (RFC 016 Task
    /// B/C): a lowering-only snapshot handed to the checked `Program` once
    /// checking finishes, mirroring [`Self::all_sums`].
    pub fn all_boxes(&self) -> &[Ty] {
        self.boxes.all()
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
    /// Interned box pointees, moved into the finished [`Types`] once
    /// collection ends (Specification 016 section 4.1).
    boxes: BoxTable,
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
        // Specification 016 section 4.1: the pointee resolves through
        // ordinary type resolution. `Ref<T>` and a no-result type have no
        // `TypeRef` spelling that reaches here at all (the parser's
        // `sum_type_parser` never accepts `Ref`, and a no-result type is only
        // ever the absence of a `: type` clause), so every pointee that
        // reaches this arm is already a storable value type; a pointee that
        // failed to resolve on its own already reported its own error and
        // falls back to `Ty::Nil` like every other filler here.
        TypeRef::Box(inner) => {
            let pointee = resolve(builder, inner, errors).unwrap_or(Ty::Nil);
            Some(Ty::Box(builder.boxes.intern(pointee)))
        }
    }
}

/// The qualified name of any resolved type, for a builder whose declarations
/// are not yet all resolved (`None` still stands in for one being built).
fn defs_display(defs: &[Option<TypeDef>], sums: &SumTable, boxes: &BoxTable, ty: Ty) -> String {
    match ty {
        Ty::User(id) => defs[id.index()]
            .as_ref()
            .map(|def| def.name().to_string())
            .unwrap_or_else(|| format!("<type #{}>", id.0)),
        Ty::Sum(id) => sums
            .members(id)
            .iter()
            .map(|member| defs_display(defs, sums, boxes, *member))
            .collect::<Vec<_>>()
            .join(" | "),
        Ty::Box(id) => format!(
            "Box<{}>",
            defs_display(defs, sums, boxes, boxes.pointee(id))
        ),
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
        let name = defs_display(&builder.defs, &builder.sums, &builder.boxes, *ty);
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
///
/// Specification 016 section 5.1: a `Box<T>` occurrence is an indirection
/// edge that this by-value layout graph must not traverse through, so a
/// `Ty::Box` field or sum member contributes no edge at all -- it falls
/// through to the wildcard below, deliberately unlike `Ty::User`/`Ty::Sum`.
/// This is a separate, box-excluding edge function from the complete
/// semantic dependency graph a box's pointee still needs for resolution and
/// (eventually) destruction; nothing here collapses the two.
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
            // `Ty::Box(_)` terminates the edge (see above); every other
            // scalar was never a layout edge either.
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
            // Specification 016 section 8.3: a box never supports equality,
            // so a struct or union containing one does not either, regardless
            // of the pointee.
            Ty::Box(_) => false,
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

/// Computes move-only status for every type once, after layout is known
/// finite -- the same structural fixed point `equality_support` above
/// computes, with the opposite propagation rule (Specification 016 section
/// 5.3): a struct is move-only when *any* field is, a union when *any*
/// member is, and `Box<T>` unconditionally, regardless of `T`. RFC 016 Task A
/// only computes and records this property; ownership analysis (Task B) is
/// its first consumer.
fn move_only_support(defs: &[TypeDef], sums: &SumTable) -> Vec<bool> {
    let mut memo: Vec<Option<bool>> = vec![None; defs.len()];

    fn ty_move_only(
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
                .any(|member| ty_move_only(member, defs, sums, memo)),
            Ty::Box(_) => true,
            _ => false,
        }
    }

    fn solve(id: TypeId, defs: &[TypeDef], sums: &SumTable, memo: &mut Vec<Option<bool>>) -> bool {
        if let Some(known) = memo[id.index()] {
            return known;
        }
        // A cyclic layout is rejected before this runs, so this recursion
        // cannot actually revisit an in-progress id; the guard exists only
        // for defensive symmetry with `equality_support`'s.
        memo[id.index()] = Some(false);
        let move_only = match &defs[id.index()] {
            TypeDef::Represented { target, .. } => ty_move_only(target, defs, sums, memo),
            TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => fields
                .iter()
                .any(|(_, ty)| ty_move_only(ty, defs, sums, memo)),
            TypeDef::Union { members, .. } => members
                .iter()
                .any(|member| solve(*member, defs, sums, memo)),
        };
        memo[id.index()] = Some(move_only);
        move_only
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
/// whose members individually have bridge representations. Specification 016
/// section 10 extends it again to `Box<T>` and every type transitively
/// containing one: a struct or union field of type `Box<T>` is already
/// rejected here because the *containing* struct or union is itself a
/// `Ty::User` and every such type is unconditionally rejected below,
/// regardless of its fields, so only a `Box<T>` used directly (by value or
/// as a `Ref<T>` referent) needs its own arm. Rejected here, during
/// declaration collection, so nothing downstream sees any of the three.
fn reject_bridge_user_types(
    defs: &[Option<TypeDef>],
    sums: &SumTable,
    boxes: &BoxTable,
    params: &[TParam],
    result: Option<Ty>,
    declaration: &ExternFunc<'_>,
    errors: &mut Vec<Error>,
) {
    let name = |ty: Ty| defs_display(defs, sums, boxes, ty);
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
            Ty::Box(_) => format!(
                "'{}' is a box type; 'Box<T>' and every type transitively containing one \
                 are rejected in 'extern rust' parameters and results",
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
        boxes: BoxTable::default(),
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
    // Specification 016 section 5.3: computed only once layout is proven
    // finite, exactly like `equality` above. The default for an already
    // layout-rejected program is inconsequential (nothing consumes it before
    // `errors` is reported), so `true` is chosen only for symmetry with
    // `equality`'s "assume the restrictive answer" default.
    let move_only = if acyclic {
        move_only_support(&defs, &builder.sums)
    } else {
        vec![true; defs.len()]
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
            &builder.boxes,
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
            move_only,
            sums: builder.sums,
            boxes: builder.boxes,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects declarations for `source`, asserting there were no
    /// declaration-collection errors -- these tests are about the resulting
    /// [`Types`] table, not about rejecting malformed input.
    fn collected(source: &str) -> Collected {
        let program =
            crate::parse(source).unwrap_or_else(|d| panic!("{source} should parse: {d:?}"));
        let mut errors = Vec::new();
        let collected = collect(&program, &mut errors);
        assert!(
            errors.is_empty(),
            "expected no declaration errors for {source}, got: {errors:?}"
        );
        collected
    }

    // Specification 016 section 4.1: `Box<T>`'s identity is exactly its
    // pointee type, interned the same way `SumTable` interns member sets.

    #[test]
    fn box_pointees_intern_to_the_same_id_for_the_same_pointee_type() {
        let collected = collected(
            "type A is struct value: Box<Int64>, end\n\
             type B is struct value: Box<Int64>, end",
        );
        let types = &collected.types;
        let a = types.top_level("A").expect("A exists");
        let b = types.top_level("B").expect("B exists");
        let a_ty = types.def(a).fields().expect("A is a struct")[0].1;
        let b_ty = types.def(b).fields().expect("B is a struct")[0].1;
        assert_eq!(
            a_ty, b_ty,
            "two 'Box<Int64>' fields must intern to the same 'Ty::Box' id"
        );
        let Ty::Box(id) = a_ty else {
            panic!("expected a Box field, got {a_ty:?}")
        };
        assert_eq!(types.box_pointee(id), Ty::Int64);
    }

    // Specification 016 section 5.1: a box occurrence terminates the by-value
    // layout graph, so a recursive type crossing a box edge has a finite
    // layout while an unbroken direct cycle still does not.

    #[test]
    fn a_box_edge_breaks_an_otherwise_infinite_value_layout() {
        let program = crate::parse("type Node is struct next: Node, end")
            .expect("a self-referential field parses");
        let mut errors = Vec::new();
        collect(&program, &mut errors);
        assert!(
            !errors.is_empty(),
            "an unbroken direct value-layout cycle must still be rejected"
        );

        // The identical shape, broken by one Box edge, has a finite layout.
        collected("type Node is struct next: Box<Node>, end");
    }

    // Specification 016 section 5.3: `Box<T>` is unconditionally move-only,
    // and a struct or union propagates move-only status from any field or
    // member, transitively through further nesting -- the same structural
    // fixed point `supports_equality` already computes for equality.

    #[test]
    fn a_box_is_always_move_only_regardless_of_a_copyable_pointee() {
        let collected = collected("type Holder is struct value: Box<Int64>, end");
        let types = &collected.types;
        let holder = types.top_level("Holder").expect("Holder exists");
        let box_ty = types.def(holder).fields().expect("Holder is a struct")[0].1;
        assert!(
            types.is_move_only(box_ty),
            "'Box<Int64>' must be move-only even though 'Int64' is copyable"
        );
    }

    #[test]
    fn move_only_is_structural_and_transitive() {
        let collected = collected(
            "type Leaf is struct value: Int64, end\n\
             type Boxy is struct payload: Box<Leaf>, end\n\
             type Wrapper is struct inner: Boxy, end\n\
             type Choice is union\n\
             \x20   | A is struct value: Box<Leaf>, end\n\
             \x20   | B is struct value: Int64, end\n\
             end",
        );
        let types = &collected.types;
        let leaf = Ty::User(types.top_level("Leaf").expect("Leaf exists"));
        let boxy = Ty::User(types.top_level("Boxy").expect("Boxy exists"));
        let wrapper = Ty::User(types.top_level("Wrapper").expect("Wrapper exists"));
        let choice = types.top_level("Choice").expect("Choice exists");
        let b_member = Ty::User(types.member(choice, "B").expect("Choice.B exists"));

        assert!(
            !types.is_move_only(leaf),
            "a plain 'Int64' field is copyable"
        );
        assert!(
            types.is_move_only(boxy),
            "a direct 'Box<T>' field makes its struct move-only"
        );
        assert!(
            types.is_move_only(wrapper),
            "move-only propagates transitively through further struct nesting"
        );
        assert!(
            types.is_move_only(Ty::User(choice)),
            "a union is move-only when any member's payload is"
        );
        assert!(
            !types.is_move_only(b_member),
            "one move-only member does not make every member move-only"
        );
    }
}
