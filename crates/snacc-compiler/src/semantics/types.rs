//! Type identity, the type-definition table, and declaration collection.
//!
//! Specification 010 section 19 phase 2: top-level type names are collected in
//! deterministic source order and allocated stable [`TypeId`] values before any
//! body is resolved, union members are allocated afterwards, and the by-value
//! layout graph is checked for cycles before expression checking begins.

use crate::semantics::checker::{Error, TParam, Ty};
use crate::syntax::ast::{
    ExternFunc, Func, MethodDecl, Param, ParamMode, Program as AstProgram, Span, Spanned, TypeBody,
    TypeRef,
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
    pub fn display(&self, ty: Ty) -> String {
        match ty {
            Ty::User(id) => self.def(id).name().to_string(),
            scalar => scalar.to_string(),
        }
    }

    /// Specification 010 sections 7.3, 8.4, and 9.2: equality is supported when
    /// every contained type supports it. Memoized by resolved type ID.
    pub fn supports_equality(&self, ty: Ty) -> bool {
        match ty {
            Ty::User(id) => self.equality[id.index()],
            // Every scalar compares with itself.
            _ => true,
        }
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

/// Resolves one written type. Returns `None` after reporting an unresolved or
/// malformed path.
fn resolve(builder: &Builder, ty: &Spanned<TypeRef<'_>>, errors: &mut Vec<Error>) -> Option<Ty> {
    match &ty.0 {
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
    }
}

/// The user types reachable by value from `def`, for layout cycle detection.
fn contained(def: &TypeDef) -> Vec<TypeId> {
    let user = |ty: &Ty| match ty {
        Ty::User(id) => Some(*id),
        _ => None,
    };
    match def {
        TypeDef::Represented { target, .. } => user(target).into_iter().collect(),
        TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => {
            fields.iter().filter_map(|(_, ty)| user(ty)).collect()
        }
        TypeDef::Union { members, .. } => members.clone(),
    }
}

/// Three-state depth-first traversal over the by-value layout graph. Reports
/// the complete first cycle it finds, in traversal order.
fn reject_layout_cycles(defs: &[TypeDef], spans: &[Span], errors: &mut Vec<Error>) -> bool {
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
        for next in contained(&defs[id.index()]) {
            if !visit(next, defs, spans, state, stack, errors) {
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
fn equality_support(defs: &[TypeDef]) -> Vec<bool> {
    let mut memo: Vec<Option<bool>> = vec![None; defs.len()];

    fn solve(id: TypeId, defs: &[TypeDef], memo: &mut Vec<Option<bool>>) -> bool {
        if let Some(known) = memo[id.index()] {
            return known;
        }
        // A cyclic layout is rejected before this runs; the guard keeps a
        // rejected program from recursing forever while other errors report.
        memo[id.index()] = Some(true);
        let supported = match &defs[id.index()] {
            TypeDef::Represented { target, .. } => match target {
                Ty::User(inner) => solve(*inner, defs, memo),
                _ => true,
            },
            TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => {
                fields.iter().all(|(_, ty)| match ty {
                    Ty::User(inner) => solve(*inner, defs, memo),
                    _ => true,
                })
            }
            TypeDef::Union { members, .. } => {
                members.iter().all(|member| solve(*member, defs, memo))
            }
        };
        memo[id.index()] = Some(supported);
        supported
    }

    (0..defs.len())
        .map(|index| solve(TypeId(index as u32), defs, &mut memo))
        .collect()
}

/// Resolves a parameter list. Duplicate names belong to the function-wide
/// binding check in the checker, not here.
fn resolve_params(builder: &Builder, params: &[Param<'_>], errors: &mut Vec<Error>) -> Vec<TParam> {
    params
        .iter()
        .map(|param| {
            // Specification 011 section 19 phase 1 step 4: the referent resolves
            // through ordinary type resolution, and the passing mode is stored
            // beside the resolved value type.
            let ty = resolve(builder, &param.ty, errors).unwrap_or(Ty::Nil);
            TParam {
                name: param.name.to_string(),
                ty,
                mode: param.mode,
            }
        })
        .collect()
}

/// Specification 010 section 16: no user-defined type crosses the Rust bridge.
/// Rejected here, during declaration collection, so nothing downstream sees one.
fn reject_bridge_user_types(
    defs: &[Option<TypeDef>],
    params: &[TParam],
    result: Option<Ty>,
    declaration: &ExternFunc<'_>,
    errors: &mut Vec<Error>,
) {
    let name = |ty: Ty| match ty {
        Ty::User(id) => defs[id.index()]
            .as_ref()
            .map(|def| def.name().to_string())
            .unwrap_or_else(|| "a user-defined type".to_string()),
        scalar => scalar.to_string(),
    };
    let crossing = params
        .iter()
        .map(|param| param.ty)
        .chain(result)
        .filter(|ty| matches!(ty, Ty::User(_)));
    for ty in crossing {
        errors.push(Error {
            span: declaration.span,
            msg: format!(
                "'{}' is a user-defined type; only the ABI's permitted types \
                 may cross a Rust bridge",
                name(ty)
            ),
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
            TypeBody::Represented(target) => {
                let target = resolve(&builder, target, errors).unwrap_or(Ty::Nil);
                TypeDef::Represented { name, target }
            }
            TypeBody::Struct(fields) => TypeDef::Struct {
                name,
                fields: resolve_fields(&builder, declaration.name, fields, errors),
            },
            TypeBody::Union(declared) => {
                // Tags are the member's source position, assigned deterministically
                // from zero (Specification 010 section 15.2).
                for (tag, (member, member_id)) in declared.iter().zip(members).enumerate() {
                    let Some(member_id) = *member_id else {
                        continue;
                    };
                    let qualified = format!("{}.{}", declaration.name, member.name);
                    let fields = resolve_fields(&builder, &qualified, &member.fields, errors);
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
    let acyclic = reject_layout_cycles(&defs, &builder.spans, errors);
    let equality = if acyclic {
        equality_support(&defs)
    } else {
        vec![false; defs.len()]
    };

    // Step 5: callable signatures, bridge rejection, and call-head conflicts.
    let mut sigs: HashMap<String, FuncSig> = HashMap::new();
    let mut func_names: Vec<&str> = program.funcs.keys().copied().collect();
    func_names.sort_unstable();
    for name in func_names {
        let function: &Func<'_> = &program.funcs[name];
        let params = resolve_params(&builder, &function.args, errors);
        let result = function
            .ret
            .as_ref()
            .and_then(|ty| resolve(&builder, ty, errors));
        sigs.insert(name.to_string(), FuncSig { params, result });
    }
    let mut extern_names: Vec<&str> = program.externs.keys().copied().collect();
    extern_names.sort_unstable();
    for name in extern_names {
        let function: &ExternFunc<'_> = &program.externs[name];
        let params = resolve_params(&builder, &function.args, errors);
        let result = function
            .ret
            .as_ref()
            .and_then(|ty| resolve(&builder, ty, errors));
        reject_bridge_user_types(&builder.defs, &params, result, function, errors);
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
        let params = resolve_params(&builder, &declaration.args, errors);
        let result = declaration
            .ret
            .as_ref()
            .and_then(|ty| resolve(&builder, ty, errors));
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
        },
        methods,
        method_index,
        sigs,
    }
}

fn resolve_fields(
    builder: &Builder,
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
