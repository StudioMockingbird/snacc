# Milestone 2 Implementation Plan: Fixed-Width Unsigned Integers and Float32

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Two large sequential tasks, not many small ones — verify once per task, not per sub-phase, per this project's reduced-iteration convention.

**Goal:** Implement Specification 009 in full: `UInt8`, `UInt16`, `UInt32`, `UInt64`, `Float32`, their literals, operations, printing, and Rust bridge mappings. Establishes ABI version 3.

**Spec:** [docs/specs/009-fixed-width-unsigned-and-float32.md](009-fixed-width-unsigned-and-float32.md) — this spec is unusually precise (exact bit-width tables, exact diagnostics, exact conformance test list in section 9). This plan only adds the Rust-side shapes and task boundaries the spec leaves to the implementer; read the spec directly for everything else rather than expecting it restated here.

**Prior state:** Milestone 1 (RFC 008 + Specification 012 scalar slice) is complete — `crates/snacc-compiler` has `Block`/`BlockElement`/`IfForm`, `TStmt`/`TBlock`, optional function/bridge results, and ABI version 2. This milestone extends the same architecture with five new scalar types; it does not change the statement/block model.

## Global Constraints

- `Ty` (checked, `crates/snacc-compiler/src/semantics/checker.rs`) and `TypeName` (syntax, `crates/snacc-compiler/src/syntax/ast.rs`) both grow from 4 variants to 9: add `UInt8`, `UInt16`, `UInt32`, `UInt64`, `Float32` to each, keeping `Dec64`, `Int64`, `Bool`, `Nil`. Every existing exhaustive `match` on either type must be extended, not wildcarded — the whole point of an exhaustive match here is that a missing case fails to compile.
- No implicit conversion between any new type and anything else, including each other. Only the pre-existing `Int64 -> Dec64` conversion survives (spec section 4.4).
- Exact-match operations only: arithmetic/comparison require identical operand types (spec section 4.5-4.6) — do not route the new types through the existing `common_numeric`/`assignable` helpers designed for the `Int64`/`Dec64` promotion pair; those helpers are for that one specific conversion and nothing else.
- ABI version 3 (from 2). `snacc_compiler::ABI_VERSION`, the new `snacc_runtime::ABI_VERSION`, both ABI-mismatch assertions (Cargo-hosted and direct — both now exist from Milestone 1), and `emit_cached`'s cache identity all move together.
- Before handoff: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, full workspace test suite green.

## Design decisions this plan makes

**1. `NumLiteral` (`crates/snacc-compiler/src/syntax/ast.rs`) grows to carry every literal form exactly:**

```rust
pub enum NumLiteral {
    Int(i64),
    Dec(f64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
}
```

**2. Lexer maximal munch.** The current `num` rule (`text::int(10).then(just('.').then(text::digits(10)).or_not()).to_slice().try_map(...)`) only ever sees digits and an optional decimal point — it never looks past them, so `1u8` today lexes as `Int(1)` followed by a separate `Ident("u8")` token. Spec section 4.2 requires the *opposite*: a numeric token must greedily consume any immediately-following ASCII letter/digit/underscore run as part of the same token before classifying it, so `1u9` is one malformed token, not two valid ones. Rewrite the `num` combinator to capture `digits [ '.' digits ] [ trailing-alphanumeric-run ]` as one slice, then classify the trailing run against exactly `""` (plain `Int64`/`Dec64`), `"u8"`/`"u16"`/`"u32"`/`"u64"` (unsigned, only when there was no decimal point), or `"f32"` (float32, with or without a decimal point) — anything else is one lexical error naming the complete invalid token, per spec section 4.2's exact examples (`1u9`, `1u8x`, `1f64`, `1.0u8` are all errors).
Parse each accepted numeric magnitude with the narrowest correct integer type for its class (`u8`/`u16`/`u32`/`u64` via `str::parse`, which already range-checks) and reject out-of-range with the literal's exact text in the message. Parse `f32` literals with `str::parse::<f32>()` directly on the decimal source text (never via an intermediate `f64` parse — that would round twice) and reject a result that parses to infinity.

**3. Checked `Ty` gains no new payload shape** — it's a plain 9-variant enum exactly like today's 4-variant one; unsigned width and float vs. double are fully determined by which variant, so no separate "signedness" or "width" field is needed on `Ty` itself. Do carry the *operand* type explicitly on typed arithmetic/comparison nodes (`TExpr::Arith`/`TExpr::Cmp` already do this via their trailing `Ty` field — confirm it flows through for every new type too) so the backend never has to infer semantics from an LLVM bit width, per spec section 8 phase 2 step 4.

**4. Bridge/ABI mapping is the literal table in spec section 5.2** — `UInt8->u8/i8`, `UInt16->u16/i16`, `UInt32->u32/i32`, `UInt64->u64/i64`, `Float32->f32/float`. Extend `rust_abi_type` in `apps/cargo-snacc/src/main.rs` (currently a 4-arm exhaustive match over `Ty`) with the five new arms — it stays exhaustive, so the compiler forces every call site to handle the addition.

## Task 1: Lexer, AST, checker, LLVM backend (spec sections 4, 4.1-4.7, spec plan phases 1-3)

**Files:** `crates/snacc-compiler/src/syntax/{ast,lexer,parser}.rs`, `crates/snacc-compiler/src/semantics/checker.rs`, `crates/snacc-compiler/src/backend/llvm.rs`, plus this crate's own tests.

Implement spec009 phases 1-3 in full, in one continuous pass (these layers don't compile independently, same reasoning as Milestone 1). Cover:
- Reserved words, keyword tokens, `Display`, parser type-name sites for all five new types.
- Exact literal lexing per Design Decision 2.
- Exhaustive `Ty`/`TypeName` extension (Design Decision 3) across every match site: assignability, common-type selection (new types never participate in it — only `Int64`/`Dec64` do), equality, printing eligibility, signature checking.
- Arithmetic: modular wrapping for `UIntN` (`+`/`-`/`*` produce `UIntN` mod 2^N — LLVM's ordinary `add`/`sub`/`mul` on an N-bit integer already wraps modulo 2^N by definition, so no special wrapping logic is needed beyond using the correct bit width; do not add `nsw`/`nuw` no-wrap flags), unsigned `udiv` (division by zero is explicitly undefined behavior per spec 4.5 — no guard, no diagnostic, may use LLVM's `udiv` directly and accept its poison result), binary32 arithmetic at `float` precision (Inkwell float ops on `context.f32_type()`, never promoted to `f64` mid-computation).
- Comparison: `UIntN` uses unsigned predicates (`ULT`/`ULE`/`UGT`/`UGE` etc., not the signed ones `Int64` already uses); `Float32` uses the same ordered/NaN semantics `Dec64` already has, just at `float` width.
- Printing: extend whatever mechanism currently restricts `print` to the 4 existing types (or perhaps it doesn't restrict at all today and printing "just works" per-`Ty` — check) to accept and correctly format all five new types.
- Backend: five new `llvm_ty`/`default_value`-equivalent mappings (`i8`/`i16`/`i32`/`i64`/`float`), unsigned comparison predicates, `udiv`, float32 constant/arithmetic lowering. Per spec section 5.2's ABI note: the backend must match Rust/Clang zero-extension calling-convention behavior for `UInt8`/`UInt16` sub-word bridge parameters/results — check what Inkwell attribute (`zeroext` param/return attribute) is needed on bridge-facing function declarations and calls for these two widths specifically, and apply the same audit to the *existing* `Bool` (`u8`) bridge mapping while you're in this code, per the spec's explicit instruction to do so.

- [ ] Write tests first (TDD) for the checker/backend layers per spec section 9 items 1-10 (lexing/range/suffix cases can be tested directly against the lexer; arithmetic/comparison/modular/binary32-precision cases need real LLVM execution — check whether this codebase's existing pattern is `tests/cases/run/pass/*.nrs` corpus cases consumed by `apps/snacc/tests/conformance.rs`, or checker-level unit tests, or both, and follow it).
- [ ] Implement.
- [ ] Run `cargo test -p snacc-compiler` and `cargo test -p snacc --test conformance` once, at the end. Iterate until green. `cargo check -p snacc-compiler --all-targets` clean.
- [ ] Commit, staged by exact file path (this crate's files plus any new `tests/cases/` corpus files only — nothing in `apps/cargo-snacc`, `crates/snacc-runtime`, or docs, that's Task 2).

## Task 2: Runtime, bridges, ABI 3, contract, corpus (spec sections 5.3, 6-7, spec plan phases 4-5)

**Files:** `crates/snacc-runtime/src/lib.rs`, `apps/cargo-snacc/src/main.rs`, `apps/cargo-snacc/tests/cargo_hosted.rs`, `tests/fixtures/cargo-hosted/`, `crates/snacc-driver/src/lib.rs` (ABI constant bump only), `LANGUAGE.md`, `GRAMMAR.ebnf`, and any remaining corpus/example files.

- Add the five runtime print functions (`snacc_print_u8`, `_u16`, `_u32`, `_u64`, `_f32`) to `crates/snacc-runtime/src/lib.rs` with the exact signatures in spec section 5.3, and add their addresses to `force_link`'s retention list alongside the existing four.
- Extend `apps/cargo-snacc/src/main.rs`'s `rust_abi_type` (or equivalent) with the five new mappings per Design Decision 4; extend the checked bridge declaration type consumption anywhere else it's exhaustively matched.
- Bump `snacc_compiler::ABI_VERSION` and `snacc_runtime::ABI_VERSION` from 2 to 3. Both existing ABI-mismatch assertions (the Cargo-hosted generated-assertion-file one from Milestone 1's Task 6, and the direct-workflow one in `crates/snacc-driver/src/lib.rs`) already read the constant rather than a hardcoded number, so they should need no further change beyond the constants themselves — verify this is actually true rather than assuming it.
- Update `LANGUAGE.md`'s EBNF fence with spec section 4.2's grammar, copy byte-identical to `GRAMMAR.ebnf`. Update `LANGUAGE.md` prose: the five new types, their literal forms and ranges, the exact-match conversion rule, arithmetic/comparison/printing behavior, and the bridge/ABI table.
- Add focused parse/typecheck/run/bridge/runtime/mismatch/cache tests per spec section 9's full list (17 items) — treat this as the authoritative checklist, not a suggestion.
- Add real bridge parameter/result round trips for all five new types (spec section 9 item 12) using the same fixture pattern Milestone 1's ABI-2 no-result-bridge test used.

- [ ] Write tests first where practical; implement; run the full verification once at the end: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, `cargo test --workspace`.
- [ ] Commit, staged by exact file path, in as many commits as make sense scoped by sub-area (runtime+bridges+ABI vs. docs+corpus) — your judgment.

## Final check

After Task 2 lands, walk spec009's 10 acceptance criteria (section 10) and 17 conformance-test items (section 9) once, honestly, against the real code and tests — report any gap rather than assuming completeness.
