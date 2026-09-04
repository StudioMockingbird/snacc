# Milestone 4 Implementation Plan: Call-Scoped Reference Parameters (`Ref<T>`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Two sequential tasks (checker, then backend+ABI+docs), each verified once at the end.

**Goal:** Implement Specification 011: `Ref<T>` as a parameter-only, call-scoped, automatically-dereferenced mutable reference. Establishes ABI version 4.

**Spec:** [docs/specs/011-call-scoped-reference-parameters.md](011-call-scoped-reference-parameters.md) — read the relevant section directly for exact rules; this plan only pins Rust-side shapes and task boundaries.

**Prior state:** Milestone 3 complete. `Place`/`PlaceRoot::{Local(String), SelfRef}` exist and are used for locals, `self`, and field paths; mutability lives per-binding (`let mut` locals and `self` are mutable roots); `TReceiver::{Place(Place), Value(TExpr)}` already distinguishes an addressable method receiver from a temporary one. ABI version 3.

## Global Constraint

Before handoff: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, full workspace test suite green.

## Design decisions — this milestone composes almost entirely out of Milestone 3's existing machinery

**1. `Ref<T>` never becomes a `Ty` variant.** Per spec §19 phase 1 step 2, represent it purely as a parameter-passing mode alongside an ordinary value type:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamMode { Value, Reference }

pub struct TParam { pub name: String, pub ty: Ty, pub mode: ParamMode }
```

`TFunc`/`TMethod`/`TExtern`'s `params` fields become `Vec<TParam>` (renamed/restructured from whatever `Vec<(String, Ty)>` shape they currently have — forcing every call site to handle `mode`, same pattern as Milestone 1's `ret`→`result` rename). A `Ref<T>` parameter's *bound type* inside the function body is plain `T` — never a distinct "reference type" — because automatic dereference means every read/write already operates on `T`.

**2. A `Ref<T>` parameter is bound exactly like `let mut`, with one crucial simplification for lowering.** At the checker level, binding a reference parameter into the environment is `mutable = true, ty = T` — the *same* shape as a `let mut` local's binding, just via a parameter instead of a declaration. `PlaceRoot::Local(name)` needs no new variant; the binding table entry's mutability flag is what matters, exactly as it already does for every other mutable root. This means reference-parameter reads, field access, writes, and method-receiver use all go through the *existing* place-resolution and mutable-root-checking logic Milestone 3 already built — this milestone extends the *argument-checking* side (does the call site supply a valid reference?) far more than the *body-checking* side (which mostly already works once the parameter is bound correctly).

At the LLVM level this pays off even more directly: a `let mut` local needs a fresh `alloca` because its value starts out not-yet-addressable; a `Ref<T>` parameter's incoming LLVM value **is already a pointer** (that's the whole point of passing by reference), so it needs **no alloca at all** — bind it directly as the existing mutable-local slot shape (whatever this codebase currently calls it, e.g. `Slot::Mutable(ptr, llvm_ty)`) using the incoming parameter value as the pointer. Reads, writes, field GEPs, and method-receiver-address-taking through it are then **identical code** to what already handles a `let mut` local or `self` — do not write a second code path for "reference parameter access." If Task B finds itself writing new lowering logic for *reading or writing through* a `Ref<T>` parameter (as opposed to *binding* one at function entry, or *passing* one at a call site), stop and reconsider — that logic should already exist.

**3. Checked call arguments must distinguish value from reference so lowering can never accidentally copy a reference argument** (spec §19 phase 2 step 5):

```rust
pub enum TArg { Value(TExpr), Reference(Place) }
```

Every call site (`TExpr::Call`, `TStmt::Call`/no-result call, `TExpr::MethodCall`/`TStmt::MethodCall`, bridge calls) carries `Vec<TArg>` instead of `Vec<TExpr>`. Checking a call: for each parameter, if its mode is `Value`, check the argument expression normally and wrap `TArg::Value`; if `Reference`, resolve the argument as a `Place` (reject a non-place — literal, call result, arithmetic result — same "argument requires a mutable root" family of diagnostic spec011 §13 lists), require its resolved type equals the referent type *exactly* (no widening, no represented-type equivalence), and require its root to be mutable (an ordinary local/parameter reference-argument needs `let mut`; a reference *parameter* forwarded as a reference argument to another call is always valid, since it's already a mutable root inside its own function — this is the "reborrow" case and needs no extra ceremony beyond normal place/mutability resolution).

**4. Overlap checking reuses `Place`'s existing structural equality/prefix relationship** (already built for Milestone 3's type-test-chain exhaustiveness, which needed "is this the same syntactic place"). Two reference-argument places overlap when they're structurally equal or one's field path is a prefix of the other's (same root, one path is `[]` and the other `[x, ...]` or shorter-prefix-matches-longer). Two different field indices at the same path depth on the same root (`point.x` vs `point.y`) do not overlap. Compare every pair of reference arguments in one call pairwise; for a method call, also compare every reference argument against the receiver's place *when the receiver is `TReceiver::Place`* (a `TReceiver::Value` temporary cannot overlap anything, by construction — independent storage).

**5. Bridge mapping is mechanical**: `Ref<T>` at an `extern rust` parameter maps to Rust `&mut R` where `R` is `T`'s existing scalar mapping (spec §12.2's table). At the LLVM level a bridge call with a reference argument lowers **identically** to an internal call with one — pass the place's address — because `&mut R` and `*mut R` share layout and calling-convention (spec §12.2 explains why; you don't need to re-derive it, just rely on it). `render_bridge_assertions` in `apps/cargo-snacc/src/main.rs` needs to render `&mut R` instead of bare `R` for a reference-mode bridge parameter in the generated assertion's function-pointer type.

## Task A: Syntax, resolved signatures, place/call checking (spec §§4-11, plan phases 1-2)

**Files:** `crates/snacc-compiler/src/syntax/{ast,lexer,parser}.rs`, `crates/snacc-compiler/src/semantics/checker.rs` (and `semantics/types.rs` if that's where relevant lookups live now).

Reserved word `Ref`; parse `Ref<T>` only in a direct parameter-type position (function/method/bridge parameters — not `self`), rejecting it everywhere else even under parser recovery (results, locals, fields, represented targets, union members, nested `Ref<Ref<T>>`) per spec §5. Resolve the referent through ordinary type resolution (reusing Milestone 3's qualified-path resolution — a `Ref<T>` referent can itself be a user-defined type). Implement Design Decisions 1 and 3-4 above: `ParamMode`/`TParam`, `TArg::{Value,Reference}` threaded through every call-checking path (function, method, bridge), argument mutability/exact-type/place validation, and pairwise + receiver overlap checking (spec §6.4, §13). Update every exhaustive consumer of the old parameter-list shape.

- [ ] Write tests first (TDD) per spec §16 items 1-17 at the checker level (execution proof is Task B's job — cover everything checkable here: argument validation, overlap rejection/acceptance, exact-type rejection, forwarding/reborrowing type-checks correctly, every declaration-site rejection).
- [ ] Implement.
- [ ] Run `cargo test -p snacc-compiler` once at the end. `cargo check -p snacc-compiler --all-targets` clean.
- [ ] Commit, staged by exact file path — this crate's files and new `tests/cases/parse|typecheck/` corpus only.

## Task B: LLVM lowering, bridge ABI 4, contract, corpus (spec §§10-14, plan phases 3-5)

**Files:** `crates/snacc-compiler/src/backend/llvm.rs`, `crates/snacc-runtime/src/lib.rs` (ABI constant only — spec §12.4 explicitly says no new print symbol is needed), `apps/cargo-snacc/src/main.rs`, `apps/cargo-snacc/tests/cargo_hosted.rs`, `tests/fixtures/cargo-hosted/`, `crates/snacc-driver/src/lib.rs` (ABI constant bump), `LANGUAGE.md`, `GRAMMAR.ebnf`.

Implement Design Decisions 2 and 5. Bind a `Ref<T>` parameter at function entry directly from the incoming pointer value (no alloca — see Design Decision 2's warning about not writing new read/write logic). Lower `TArg::Reference(place)` at every call site (internal, method, bridge) to the place's address; `TArg::Value` unchanged. Bump `snacc_compiler::ABI_VERSION`/`snacc_runtime::ABI_VERSION` from 3 to 4 (both existing ABI-mismatch assertions should need no change beyond the constants, per the established pattern from Milestones 1-2 — verify, don't assume). Extend `render_bridge_assertions` per Design Decision 5.

- [ ] Write execution tests first per spec §16 items 1-2, 7-15, 18-23 — real compiled-and-run programs (the `add_into` example from spec §4 is the canonical first case) plus a real bridge round trip for at least one scalar `Ref<T>` referent.
- [ ] Implement.
- [ ] Update `LANGUAGE.md`'s EBNF fence with spec §5's grammar addition, copy byte-identical to `GRAMMAR.ebnf`. Update prose for parameter-only placement, automatic dereference, exclusivity/overlap, escape prevention, reborrowing, and the bridge mapping (spec §§4-14, normative sections only).
- [ ] Run the full verification once at the end: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, `cargo test --workspace`. Iterate until green.
- [ ] Commit, staged by exact file path.

## Final check

After Task B lands, walk spec011's 9 acceptance criteria (§17) and 24 conformance-test items (§16) once, honestly, against the real code and tests.
