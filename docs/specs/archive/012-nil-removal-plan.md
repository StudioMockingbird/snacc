# Milestone 5 Implementation Plan: Standalone `Nil` Removal (ABI 5)

**Goal:** Implement Specification 012's final phase (spec §15 Phase 6-7): remove
standalone `Nil` as a usable value type everywhere except as one member of a
union. Establishes ABI version 5.

**Spec:** [docs/specs/012-variable-declarations-assignments-and-member-mutability.md](012-variable-declarations-assignments-and-member-mutability.md),
§10 (normative rule), §12 (bridge/ABI), §15 Phase 6-7 (the only unimplemented
phases — Phases 1-5 landed across Milestones 1, 3, and 4).

**Prior state, verified directly against the current tree, not assumed:**
- Blocks, declarations, assignments, root mutability, `Ref<T>` (Milestones 1,
  3, 4) are complete and match spec §§4-9, 11 already.
- `Nil` as a union member, the contextual `nil`/`null` literal resolved
  against one expected Nil-containing union, `is Nil` (no binding), and
  duplicate/Nil-only union rejection are **already implemented and tested**
  (`crates/snacc-compiler/src/semantics/checker.rs`, search `Specification 012
  section 10`; `TypeDef::UnionMember { nil: bool, .. }` in `types.rs`).
- What is genuinely missing: `resolve()` in
  `crates/snacc-compiler/src/semantics/types.rs:215-217` maps
  `TypeRef::Builtin(TypeName::Nil)` to `Ty::Nil` unconditionally, with no
  positional check — so `let x: Nil`, a `Nil` parameter, a `Nil` result, a
  `Nil` represented-type target, and (confirmed by an existing passing test at
  `checker.rs:2379`, `extern rust "snacc_user_v2_ok" fun ok(): Nil"`) a
  standalone `Nil` **bridge result are all currently accepted**, contradicting
  spec §10 and §12. `snacc_runtime::snacc_print_nil` and its force-link/bridge
  mapping still exist. ABI is still 4.

## Global Constraint

Before handoff: `cargo fmt --all -- --check`, `cargo check --workspace
--all-targets`, full workspace test suite green.

## Scope: one task

This phase is bounded and mostly rejection/deletion work layered on
machinery that already exists and is already tested (union-member `Nil`,
contextual `nil`). It does not split naturally into front-end/back-end the way
earlier milestones did. Do it as one task, verified once at the end.

**Files:** `crates/snacc-compiler/src/semantics/{types,checker}.rs`,
`crates/snacc-compiler/src/backend/llvm.rs`, `crates/snacc-runtime/src/lib.rs`,
`apps/cargo-snacc/src/main.rs`, `apps/cargo-snacc/tests/cargo_hosted.rs`,
`tests/fixtures/cargo-hosted/`, `crates/snacc-driver/src/lib.rs` (ABI constant
bump only), `LANGUAGE.md`, `GRAMMAR.ebnf`, plus corpus/test updates anywhere a
standalone `Nil` currently appears in a test fixture and must become invalid
or be rewritten as a Nil-containing union.

### What to do

1. **Reject standalone `Nil` in every value-type position** (spec §10, §13
   "Standalone `Nil` type"). The natural fix point is `resolve()` in
   `types.rs`: a `TypeRef::Builtin(TypeName::Nil)` reached from a value-type
   position (local declaration, parameter, function/method/bridge result,
   struct field, represented-type target — every existing caller of
   `resolve()`) must report a diagnostic and return `None`/an error type,
   *except* the one caller that resolves a union member's own type list, which
   must keep accepting `Nil` there (that's the existing, tested, working
   path — do not touch it). Read every call site of `resolve()` before
   changing its signature or behavior, since one of them is the path that must
   keep working. `Ref<Nil>` is already rejected by Specification 011's own
   check (confirmed in `types.rs` around line 438) — leave that as-is, no
   double rejection needed.
2. **Bridge and runtime removal** (spec §12): reject a standalone `Nil`
   `extern rust` parameter or result (the existing test at `checker.rs:2379`
   currently asserts this *succeeds* — it must become a `tests/cases/*/fail`
   or in-file rejection case instead, and any other bridge test relying on a
   bare `Nil` bridge signature needs the same treatment). Remove
   `snacc_print_nil` from `snacc-runtime`, its force-link retention list, its
   `apps/cargo-snacc` bridge-mapping/assertion entry, and its LLVM
   declaration/call-lowering path in `backend/llvm.rs`. Search for
   `snacc_print_nil` and any standalone-`Nil` case in `print`'s lowering
   (`Ty::Nil` arms) across the whole workspace — remove every one; there
   should be no `Ty::Nil`-shaped runtime value left after this task, only the
   union-tagged member.
3. **ABI 5.** Bump `snacc_compiler::ABI_VERSION` and
   `snacc_runtime::ABI_VERSION` from 4 to 5 (both existing ABI-mismatch
   assertions read the constant dynamically per the pattern established at
   every prior bump — verify, don't assume). Update the one hard-coded
   assertion in `crates/snacc-runtime/tests/abi.rs`. Add an ABI-4-cache-not-
   reused-after-the-bump test mirroring the ABI-1/2/3/4 precedents already in
   `apps/cargo-snacc/tests/cargo_hosted.rs`.
4. **Contract.** Update `LANGUAGE.md`'s EBNF fence and prose for: standalone
   `Nil` rejection, the union-member-only rule, contextual `nil`/`null`
   resolution (much of this prose may already be correct from Milestone 3 —
   verify against spec §10 and only change what's actually wrong or missing),
   and the ABI-5 bridge/runtime change. Copy the EBNF fence byte-identical to
   `GRAMMAR.ebnf`.
5. **Tests.** Add tests per spec §16 items 16-18 (standalone `Nil` rejected in
   every type/bridge position; Nil-containing unions still accept contextual
   `nil`/`null` — this should already pass, don't re-implement it, just add a
   regression test if coverage is thin; ABI 5 removes `snacc_print_nil`,
   rejects version-4 combinations, invalidates version-4 caches). Search the
   existing corpus (`tests/cases/`, fixtures, workbench snippets, doc examples
   in `LANGUAGE.md`) for any standalone `Nil` currently used as a passing
   example and either rewrite it as a `Nil`-containing union or move it to a
   `fail` case, per spec §15 Phase 7 item 3.
6. Run the full verification once at the end: `cargo fmt --all -- --check`,
   `cargo check --workspace --all-targets`, `cargo test --workspace`. Iterate
   until green.
7. Commit, staged by exact file path. Do not touch anything under
   `docs/specs/` other than this plan and `LANGUAGE.md`/`GRAMMAR.ebnf` — the
   repo has several unrelated untracked spec drafts (`013`-`016`) that are out
   of scope.

## Final check

After landing, walk spec012's 9 acceptance criteria (§17) and the Nil-relevant
conformance items (§16 items 16-18, plus 19 as the whole-suite check) once,
honestly, against the real code and tests. This closes out the full "008
to 012" implementation arc — record that in the ledger when done.
