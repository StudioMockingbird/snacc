# Milestone 3 Implementation Plan: Nominal Types, Structs, Unions, and Methods

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Two large sequential tasks (front-end, then backend+contract), each verified once at the end — per this project's reduced-iteration convention. This is the largest milestone in the five-milestone project; read everything in this plan before starting, it carries more load-bearing design than Milestones 1-2's plans because there is more novel architecture here for the spec to leave unpinned.

**Goal:** Implement Specification 010 (`type`/`struct`/`union`/`method`/`self`/`is`-type-tests) together with Specification 012's aggregate-place slice (struct-field assignment, method receiver-write validation) as one migration. No ABI version change — user-defined types are explicitly barred from the Rust bridge boundary by this spec itself.

**Spec:** [docs/specs/010-nominal-types-structs-unions-and-methods.md](010-nominal-types-structs-unions-and-methods.md) (primary — 877 lines, unusually precise; read the relevant section directly for exact rules rather than expecting them restated here). [docs/specs/012-variable-declarations-assignments-and-member-mutability.md](012-variable-declarations-assignments-and-member-mutability.md) sections 7, 9-10 (struct members, methods/`self`, root mutability through field paths — the aggregate-place extension of what Milestone 1 already built for scalars).

**Prior state:** Milestones 1-2 complete. `Ty` is a 9-variant scalar enum (`Dec64`/`Int64`/`Bool`/`Nil`/`UInt8`/`16`/`32`/`64`/`Float32`). `Block`/`BlockElement`/`IfForm`/`TStmt`/`TBlock` exist from Milestone 1; `BlockElement::Assign`/checked `TStmt::Assign` currently only target a bare local name (no field paths — this milestone adds them). ABI version 3.

## Global Constraints

- `Ty` grows by exactly one variant: `User(TypeId)`. It does **not** grow one variant per user type category — represented/struct/union/union-member are all `Ty::User(TypeId)`, distinguished by looking up the `TypeId` in a separate type-definition table, never by the `Ty` enum shape itself. Every existing exhaustive match on `Ty` needs a `User(_)` arm; where that arm can't yet do anything meaningful (e.g. `print`'s type dispatch, since spec010 §14 says printing doesn't support user types), it should be a clear rejection, not a panic or a silent no-op.
- No ABI version change. Represented/struct/union/member types are rejected at every Rust bridge site (spec010 §16) — this is enforced during declaration collection, before checking bodies, so `render_bridge_assertions` in `apps/cargo-snacc/src/main.rs` should never actually see a `Ty::User` reach it; treat that as an internal-error case if it ever does (the spec says so explicitly in §19 phase 3 step 7).
- Field/place mutability is **root-controlled only**, never per-field, never per-type. A `let mut` variable's fields are all mutable through it; a plain `let` variable's fields are never assignable regardless of the struct's own definition. Do not consult the struct definition for mutability anywhere.
- Method dispatch is fully static — receiver type is resolved at the call site, never virtual, never a runtime vtable lookup.
- Before handoff: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, full workspace test suite green.

## Design decisions this plan makes

These pin down what spec010 deliberately leaves to the implementer (it specifies checked *behavior* precisely but not Rust *data shapes*). Get these right first; four more phases build directly on them.

**1. Type identity and the definition table.**

```rust
// crates/snacc-compiler/src/semantics/checker.rs (or a new `types.rs` submodule if that
// reads cleaner — your call, but keep it in `semantics`, not a new top-level module;
// this is checker state, not a new compiler phase)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeId(u32); // opaque; allocated in deterministic source order

pub enum TypeDef {
    Represented(Ty),                          // `type N is T`
    Struct(Vec<(String, Ty)>),                 // ordered fields, `type N is struct ... end`
    Union(Vec<TypeId>),                        // ordered member TypeIds; index IS the deterministic tag
    // a union member is its own TypeId, distinct from its containing union's TypeId
    UnionMember { union: TypeId, tag: u32, fields: Vec<(String, Ty)> },
}
```

`Program` (the checked-program struct, already public via `snacc_compiler`) gains `pub types: Vec<TypeDef>` (indexed directly by `TypeId`'s inner `u32` — this is simplest and matches the spec's "allocate stable `TypeId` values" language) and enough metadata to answer "what is this `TypeId`'s qualified display name" for diagnostics (a parallel `Vec<String>` of rendered names, or store the name inside each `TypeDef` variant — your call). A union member's `TypeId` is allocated *after* all top-level types are allocated but the exact global numbering scheme is yours to design as long as it's deterministic and stable within one compilation (spec010 §19 phase 2 step 1-2).

**2. Places.** A place is a root plus zero or more field selectors:

```rust
pub struct Place {
    pub root: PlaceRoot,      // a resolved binding ID (local, parameter, or `self`)
    pub path: Vec<usize>,     // field indices, in selection order; empty = the root itself
}
```

Both `TStmt::Assign` (Milestone 1, currently bare-name-only) and the new struct-field-write case extend to use `Place` uniformly — do not keep a separate bare-name `Assign` variant alongside a new field-path one; unify them now, since Specification 011 (Milestone 4) needs this exact same `Place` shape for reference arguments and this plan's own §10 explicitly says so ("Reuse the identical place and root capability for Specification 011 reference arguments"). Mutability is a property of the *root* (looked up from the binding table — mutable only for `let mut` locals; ordinary parameters and plain `let` locals are immutable roots), never of the path.

**3. Constructors and union injection as checked expression nodes**, not statements — they produce a value:

```rust
// additions to TExpr (crates/snacc-compiler/src/semantics/checker.rs)
Construct { type_id: TypeId, fields: Vec<TExpr> },   // in declaration-field order, after checking in written order
Inject { member: TypeId, into_union: TypeId, value: Box<TExpr> },  // direct member -> union injection
FieldRead { base: Box<TExpr>, field_index: usize, ty: Ty },
```

**4. Method calls.**

```rust
Call { name: String, args: Vec<TExpr> },   // unchanged, top-level function/bridge call (Milestone 1 shape)
MethodCall { receiver: Box<TExpr_or_Place>, type_id: TypeId, method: MethodId, args: Vec<TExpr> },
```
(`MethodId` — same pattern as `TypeId`, an opaque allocated-in-order ID into a method table keyed by `(receiver TypeId, method name)`.) Decide concretely whether the receiver in a `MethodCall` is represented as a `Place` (when the call might write through it) or a plain `TExpr` (when it's read-only) — spec010 §15.3 says "a read-only call may instead use compiler-owned temporary storage," implying the checked IR should distinguish "this call's receiver is a place I can take the address of" from "this call's receiver is a value with no addressable storage." Design this explicitly rather than papering over it with one shape that only works for one case.

**5. Type-test (`is`) checked node:**

```rust
IsTest { place: Place, member: TypeId, binding: Option<(String, Ty)> },  // as a value (Bool) — the no-binding form
```
appearing as an `IfForm` arm's condition when that arm is a type test (vs. an ordinary `Bool` expression) — the checker needs to tell these apart when validating exhaustiveness (spec010 §12.4). Decide whether `IfForm`'s checked representation carries an explicit "this chain is a type-test chain over place P, exhaustive: yes/no" fact alongside its arms, or reconstructs it by inspecting each arm's condition — the former is almost certainly cleaner given exhaustiveness must be *proven*, not guessed, and the fact is needed again during LLVM lowering (Task B) to know whether to emit a tag switch or ordinary branches. Whichever you choose, thread it through consistently — Task B (LLVM lowering) will need to reconstruct or reuse it.

**6. Receiver-write effect.** A per-method `bool` (or three-state during the fixed-point solve: unknown/writes/doesn't-write) computed once via the least-fixed-point algorithm spec010 §19 phase 4 describes, stored keyed by `MethodId`, consulted only at call-checking time to validate the caller's receiver root — it is not part of the method's checked signature type and creates no source-level distinction.

## Task A: Syntax, type resolution, checked values/places/bodies, receiver-write effect (spec §§4-14, plan phases 1-4)

**Files:** `crates/snacc-compiler/src/syntax/{ast,lexer,parser}.rs`, `crates/snacc-compiler/src/semantics/checker.rs` (or `semantics/checker.rs` + a new `semantics/types.rs` if you split it — your call), plus this crate's tests. No LLVM work in this task.

This is the entire front-end: reserved words and grammar (`type`/`is`/`struct`/`union`/`method`/`self`/`.`/`|`), qualified type paths, struct/union/method/field/type-test AST nodes, declaration collection with deterministic `TypeId` allocation and cycle detection (spec §19 phase 2), the `Ty`/`TypeDef` extension from Design Decision 1, place/constructor/method-call/type-test checking (Design Decisions 2-5), the common-union-type algorithm (spec §13), and the receiver-write fixed-point analysis (Design Decision 6, spec §19 phase 4). Read spec sections 4-14 and 17 (diagnostics) directly — they are precise enough to implement from directly rather than needing this plan to restate them. Section 6.1's call-head resolution rule (is `name(...)` a receiver-local, a type constructor, or a top-level callable?) and section 12 (type-test exhaustiveness) are the two trickiest correctness spots — read them twice.

Extend Specification 012's aggregate-place work here too (not a separate task): struct field assignment validated against root mutability (spec012 §7), method receiver mutation validated against a mutable receiver root (spec012 §9), reusing exactly the `Place`/mutable-root machinery Milestone 1 already built for scalar locals.

- [ ] Write tests first (TDD) per spec §20 items 1-27 (skip items needing LLVM execution or bridge rejection you can't observe purely at the checker level — cover those at the checker/diagnostic level here, defer *execution* proof to Task B).
- [ ] Implement.
- [ ] Run `cargo test -p snacc-compiler` once at the end. `cargo check -p snacc-compiler --all-targets` clean. Iterate until green.
- [ ] Commit, staged by exact file path — this crate's files and any new `tests/cases/parse|typecheck/` corpus only. Nothing in `backend/llvm.rs`, nothing outside `crates/snacc-compiler`.

## Task B: LLVM lowering, contract, corpus (spec §§15-16, plan phases 5-6)

**Files:** `crates/snacc-compiler/src/backend/llvm.rs`, `LANGUAGE.md`, `GRAMMAR.ebnf`, remaining corpus/example migration, `apps/cargo-snacc/src/main.rs` (only if `Ty::User` reaching bridge rendering needs an explicit internal-error arm — it should never be reachable in practice per Task A's declaration-collection rejection, but the exhaustive match still needs *a* arm).

Implement spec §15 directly: named LLVM types per resolved user type in dependency order, structs in field order, unions as `{i32 tag, member_0, ..., member_n}` with deterministic zero-then-write construction, GEP-based field places, methods as internal functions with a hidden receiver pointer, tag-based `is`-test lowering and equality dispatch. Read §15.1-15.4 directly.

- [ ] Write execution tests first per spec §20 items 5, 7-11, 17-18, 20, 23, 29 (real LLVM execution — check whether this codebase's existing `tests/cases/run/pass/*.nrs` + `apps/snacc/tests/conformance.rs` pattern is still the right home, per Milestones 1-2's precedent).
- [ ] Implement.
- [ ] Update `LANGUAGE.md`'s EBNF fence with spec §5's grammar, copy byte-identical to `GRAMMAR.ebnf`. Update prose for §§6-14's normative rules — not the non-normative rationale, not the rejected-alternatives discussion.
- [ ] Migrate any remaining corpus/example files that need it for this milestone's tests to exist (none should need *migration* for old syntax at this point — Milestone 1's Task 7 already swept the whole workspace — but new corpus cases for this milestone's own features go here).
- [ ] Run the full verification once at the end: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, `cargo test --workspace`. Iterate until green.
- [ ] Commit, staged by exact file path.

## Final check

After Task B lands, walk spec010's 10 acceptance criteria (§21) and 30 conformance-test items (§20) once, honestly, against the real code and tests. Also confirm spec012's §7 (struct members) and §9 (methods/`self`) rules are genuinely satisfied, since this milestone carries them — they don't have their own separate acceptance-criteria section here, spec010's does the job jointly per its own §2.
