# Specification 018 Implementation Plan: Inline Sum Types

Document kind: Execution plan

Specification: [Specification 018: Inline Sum Types](018-inline-sum-types.md)

Base: main @ 1e28408 (RFC 013 complete). Spec 015 (recursion) lands in parallel
and touches no shared surface, so this plan does not depend on it landing
first.

This plan exists only to fix task boundaries; the specification is the
authority on behavior.

## Prior state (verified, not assumed)

- `Ty` (crates/snacc-compiler/src/semantics/types.rs) is a flat enum:
  `Int64, Dec64, Bool, Nil, UInt8/16/32/64, Float32, User(TypeId)`. No sum
  variant exists.
- Named unions already lower to "tag + one field per member" in
  `backend/llvm.rs` and are checked via `TypeDef::Union`/`UnionMember` in
  `types.rs`. Spec 018 section 8 explicitly says to reuse this lowering
  strategy for inline sums, not invent a new one.
- contextual `nil` resolution and `CONTEXTLESS_NIL` already exist; Specification
  020 removes the former `null` compatibility spelling
  (Milestone 5, commit 6b76b54) for named unions only. Spec 018 extends the
  same contextual-injection idea to inline sums; do not duplicate the concept,
  reuse/generalize it.
- The grammar has no `|` type-operator token yet. `Ref<T>` is the only closed
  parameterized type form and already establishes the angle-bracket
  tokenization pattern this spec's own `Box<T>`/`View<T>` examples (owned by
  RFC 016/017, NOT this spec) will later reuse — 018 itself does not depend on
  Box or View existing; it only needs to *permit* closed parameterized forms
  as sum members once those specs land later. Do not implement Box or View
  here.
- No represented-type-from-sum restriction exists yet because sums don't
  exist yet.

## Task A: grammar, resolution, and checking (front end)

1. Add `|` as a type-position token/operator and parenthesized type grouping,
   parsed per spec section 3's EBNF, without touching expression parsing or
   maximal-munch call/comparison behavior.
2. Add a resolved/checked inline-sum type: a new `Ty::Sum(SumId)` (or
   equivalent interned id) alongside a canonical table mapping each `SumId` to
   its normalized, deduplicated, unordered member-type set (spec section 4).
   Flatten nested sums; reject fewer than two distinct members; reject `Nil`
   unless at least one non-`Nil` member is present; reject non-storable or
   unresolved members; reject `Ref<T>` as a member (spec section 3, "a
   reference is not itself a value-type member").
3. Reject an inline sum as a represented type's immediate representation
   (spec section 4, "invalid" example).
4. Intern normalized member sets so `Byte | Nil == Nil | Byte` and
   `(Byte | Unicode) | Nil == Byte | (Unicode | Nil)` share one `Ty::Sum` id;
   retain source order only for diagnostic rendering.
5. Injection rules (spec section 5): exact direct-member match first, then
   unique existing-implicit-conversion match, else type error, else ambiguity
   error. `nil` selects the sum's `Nil` member only when the expected
   sum contains exactly one `Nil` member — generalize the existing
   `CONTEXTLESS_NIL` machinery rather than forking it.
6. Sum-to-sum assignment requires identical normalized member sets — no
   subset/superset conversion (spec section 5, "invalid" example with
   `narrow`/`wide`).
7. Extend `is` type-test parsing/checking (spec section 6) so the test target
   may name any direct member of an inline sum (built-in, qualified name, or
   closed parameterized form), with the usual branch-scoped binding, unique
   name, mutability, move, and borrow rules already used for named-union
   tests. Extend exhaustive-`if`-chain checking to inline sums.
8. Value properties (spec section 8) computed structurally from all direct
   members: copyable iff every member copyable, move-only iff any member
   move-only, requires-destruction iff any member does, borrowed iff any
   member borrowed. NOTE: at this point in the implementation order (018
   before 016/017), no move-only or borrowed types exist yet in the language,
   so these properties are trivially "always copyable, never move-only, never
   borrowed" for now — implement the general structural computation (not a
   hardcoded true/false) so RFC 016/017 land later without revisiting this
   code, but do not add dead move/borrow-analysis machinery that has no
   caller yet. `==`/`!=` supported when every member supports equality
   (reuse existing named-union recursive equality path); ordered comparison,
   arithmetic, field access, printing, and method declarations on a whole sum
   remain unsupported (decompose first).
9. Reject inline sums in `extern rust` parameters/results (spec section 10).
10. Diagnostics: implement every item in spec section 12.
11. Tests: parser tests (whitespace, grouping, nested parameterized forms,
    malformed separators, one-member forms, expression/type boundary) in
    `crates/snacc-compiler/src/syntax/parser.rs` tests module; checker tests
    (every positive/negative rule above) in
    `crates/snacc-compiler/src/semantics/checker.rs` tests module, following
    existing test naming/style in both files.
12. Verify: `cargo fmt --all`, `cargo check --workspace --all-targets`,
    `cargo test -p snacc-compiler`. Do not touch ABI, `backend/llvm.rs`,
    `LANGUAGE.md`, or `GRAMMAR.ebnf` in this task — that's Task B.

## Task B: lowering, ABI, docs, and conformance (back end)

Depends on Task A landing first.

1. Lower `Ty::Sum` by reusing named-union aggregate construction: one
   deterministic internal tag (assigned from canonical resolved member order)
   plus one correctly typed storage field per direct member; a `Nil` member
   has no payload but keeps its tag. Zero-initialize the complete aggregate
   before installing the active member (spec section 8, "Value properties" +
   Phase 4 item 3) so inactive fields/padding never carry stale data through
   compiler-generated copies.
2. Lower injection, `is` tests/bindings, equality, copies, and active-member
   cleanup exclusively from checked nodes (no backend-side re-derivation).
3. Advance `snacc_compiler::ABI_VERSION` / `snacc_runtime::ABI_VERSION` from 5
   to 6, update `crates/snacc-runtime/tests/abi.rs`'s hard-coded assertion,
   and add the `apps/cargo-snacc/tests/cargo_hosted.rs`
   `abi_5_cache_manifests_are_never_reused_after_the_abi_6_bump` test
   mirroring the existing ABI-bump precedents (grep for
   `abi_4_cache_manifests_are_never_reused_after_the_abi_5_bump` for the
   pattern to copy). Reject inline sums at every `extern rust`
   parameter/result in the bridge-assertion renderer
   (`apps/cargo-snacc/src/main.rs`) the same way standalone `Nil` is already
   rejected there.
4. Update `LANGUAGE.md` (main body, in the same style/location as the
   `Nil`-as-union-member and `Ref<T>` sections) and `GRAMMAR.ebnf` (must stay
   byte-identical in grammar content to LANGUAGE.md's leading grammar block)
   to document `T | U` inline sums per spec section 3-10.
5. Conformance tests, following existing repo conventions:
   - `tests/cases/typecheck/pass` / `tests/cases/typecheck/fail` `.nrs`/
     `.stderr` pairs for every positive rule and every diagnostic in spec
     section 12 (mirror the style of the existing
     `tests/cases/typecheck/fail/standalone_nil.nrs` etc. from Milestone 5).
   - At least one real compiled-and-executed `tests/cases/run/pass/*.nrs` +
     `.stdout` program exercising: a function returning `Byte | Nil` (or
     similar scalar sum), contextual `nil` injection, an exhaustive `if`/
     `elseif` decomposition with `is`, and a named-union-as-one-sum-member
     case (`Shape | Nil` where `Shape` is a small existing-style named union)
     — proving the reused union lowering actually executes correctly, not
     just type-checks.
   - `crates/snacc-compiler/tests/phases.rs`: one IR-level test proving the
     zero-initialize-before-install property from Task B item 1 (analogous to
     the existing `unions_lower_to_a_tag_and_one_storage_field_per_member`
     test — grep for it and extend/mirror it for an inline sum).
6. Full verification: `cargo fmt --all`, `cargo check --workspace
   --all-targets`, `cargo test --workspace`. All green.
7. Do not touch `docs/specs/013-*` (done), `docs/specs/015-*` (separate
   parallel task), or the untracked `016-*`/`017-*`/`014-*` drafts.

## Explicit non-goals for both tasks (per spec section 14/"Non-goals")

- `Box<T>` and `View<T>` do not exist yet; do not add them or forward-declare
  them. Sums simply need to *permit* a closed parameterized type as a member
  once one exists — verify the grammar/resolution code path is written
  generally enough (not hardcoded to reject all parameterized forms), but do
  not implement Box/View themselves.
- No `Option<T>` type.
- No structural union subtyping / implicit widening.
- No runtime-visible tags, layout, or address.
