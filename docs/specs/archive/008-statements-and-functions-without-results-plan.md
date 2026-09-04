# Milestone 1 Implementation Plan: Statements, No-Result Functions, and Scalar Declarations

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Tasks are **sequential, not parallel** — each rewrites a layer the next task builds on (lexer → AST → parser → checker → backend → bridge/ABI → contract/corpus). Never dispatch two of these tasks' implementers concurrently.

**Goal:** Land RFC 008 (no-result functions/methods/bridges, statement-form `while`, `break`, contextual `if`) together with Specification 012's *scalar-only* slice (semicolons removed, `let`/`let mut` as declaration statements, assignment as a statement, blocks as ordered statement lists) as one compiler migration. This is Milestone 1 of a five-milestone rollout; see `docs/specs/012-variable-declarations-assignments-and-member-mutability.md` section 2 for the full coordinated order. Establishes ABI version 2.

**Architecture:** Snacc's parser currently has no statement/expression distinction — `let`, `while`, and `;`-sequencing are all `Expr` variants threaded through one recursive Pratt expression grammar. This plan replaces that with a real split: a `Block` is an ordered list of `BlockElement`s (`let`, `let mut`, assignment, `while`, `break`, `if`, or a bare expression); `Expr` shrinks to only value-producing forms (literals, locals, arithmetic, comparison, calls, `print`). `if` parses as **one** block-element node — never nested inside `Expr` — and the checker later classifies each occurrence as statement-form or value-form purely from where it sits (is it the final element of a value-required block?). Function/method/bridge result types become `Option<Ty>`; a `None` result lowers to an LLVM `void` function.

**Tech Stack:** Rust, chumsky 0.13 (parser combinators), Inkwell/LLVM 22.

**Spec:** [docs/specs/008-statements-and-functions-without-results.md](008-statements-and-functions-without-results.md) (primary), [docs/specs/012-variable-declarations-assignments-and-member-mutability.md](012-variable-declarations-assignments-and-member-mutability.md) sections 4-6 and 15 Phase 1 only (the scalar-only slice — do **not** implement struct/union places, `Ref<T>`, method calls, or standalone-`Nil` removal here; those land in later milestones).

## Global Constraints

- Concrete procedural Rust: explicit enums/structs, `match`, ordinary loops — matches this codebase's existing style throughout `crates/snacc-compiler`.
- No semicolon token anywhere in the language after this change. A source semicolon is a lexical/parse error with a clear "Snacc has no semicolon syntax" diagnostic, not silent recovery.
- `if` is parsed as exactly one AST node type. The parser never decides statement-vs-value; only the checker does, from block position.
- ABI version goes from 1 to 2. `snacc_compiler::ABI_VERSION` (currently `1` in `crates/snacc-compiler/src/lib.rs`), the object-cache identity hash in `apps/cargo-snacc/src/main.rs`'s `emit_cached`, and a new `snacc_runtime::ABI_VERSION` constant all move together. No runtime print symbol is added or removed.
- `LANGUAGE.md`'s EBNF is normative and must be updated in the same change as `GRAMMAR.ebnf` (byte-identical grammar fence), per `AGENTS.md`.
- Before handoff on the final task: `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and the full workspace test suite must pass.
- This repo is on `main` directly (no worktree in use this session) — every commit must stage exact files by path, never `-A`/`.`.

## Design decisions this plan makes that the spec text leaves to the implementer

These are not optional; they resolve real ambiguity so every task agrees on the same shapes.

**1. `if` is block-level only, never expression-nested.** RFC 008's grammar lists `block-element = ... | if-form | expression` — `if-form` is a *sibling* of `expression`, not part of it. This means `1 + (if x then 2 else 3 end)` and `f(if x then 1 else 2 end)` are **not** valid Snacc syntax under this milestone (or ever, per the RFC). `if` may only be a whole block element. It supplies a block's value only when it is the block's **final** element and every branch is value-producing. Do not add `if` to the Pratt expression grammar.

**2. AST shapes** (`crates/snacc-compiler/src/syntax/ast.rs`):

```rust
pub struct Block<'src> {
    pub elements: Vec<Spanned<BlockElement<'src>>>,
    pub span: Span,
}

pub enum BlockElement<'src> {
    Let {
        mutable: bool,
        name: &'src str,
        name_span: Span,
        ty: TypeName,
        value: Spanned<Expr<'src>>,
    },
    Assign {
        name: &'src str,
        name_span: Span,
        value: Spanned<Expr<'src>>,
    },
    While {
        condition: Spanned<Expr<'src>>,
        body: Block<'src>,
        span: Span,
    },
    Break(Span),
    If(IfForm<'src>),
    Expr(Spanned<Expr<'src>>),
}

pub struct IfForm<'src> {
    // First arm is the `if`; remaining arms are `elseif`s, in source order.
    pub arms: Vec<(Spanned<Expr<'src>>, Block<'src>)>,
    pub else_branch: Option<Block<'src>>,
    pub span: Span,
}
```

`Expr` loses `Let`, `Then`, and `While` entirely — it keeps `Error`, `Value`, `List`, `Local`, `Binary`, `Call`, `Print`. `Func.body` and `ExternFunc` change: `Func.ret: Option<TypeName>`, `Func.body: Block<'src>`; `ExternFunc.ret: Option<TypeName>`. `Program.body` (the top-level executable body) becomes `Block<'src>` instead of `Option<Spanned<Expr<'src>>>` — an empty program is simply a `Block` with zero elements, so `Option` is no longer needed there either. Assignment and `let` targets are bare identifiers only at this milestone (no field paths — Specification 010 hasn't landed yet, so `place = identifier` is the only reachable case even though Specification 012's *final* grammar allows dotted paths and `self`).

**3. Reference the spec text for everything else.** RFC 008 sections "Functions and methods without results," "`while`," "`break`," "Statement-form and value-form `if`," "Checked representation," and "LLVM lowering" are already precise implementation guidance — read them directly rather than having this plan restate them. Specification 012 sections 4-6 (grammar, declarations, reassignment) are the scalar-only declaration/assignment rules.

**4. ABI-version-mismatch mechanism.** RFC008 requires "runtime ABI constants... shall change together" and a test that "ABI 1↔2 mismatches fail before execution." The direct (`snacc.exe`) workflow can never mismatch — the runtime source is `include_str!`-embedded into the compiler binary at its own build time, so compiler and runtime are always the same build. Only the Cargo-hosted workflow can drift (`snacc-runtime` is a normal semver dependency in the user's `Cargo.toml`). Design: add `pub const ABI_VERSION: u32 = 2;` to `crates/snacc-runtime/src/lib.rs`. Extend the bridge-assertion file `apps/cargo-snacc/src/main.rs` already generates via `write_bridge_assertions` (from RFC 007 — piggyback on it, don't invent a second generated file) to also emit one line: `const _: () = assert!(snacc_runtime::ABI_VERSION == 2, "snacc-runtime ABI version mismatch: compiler expects 2");` (with the literal `2` coming from `snacc_compiler::ABI_VERSION`, not hand-typed) into the same file. That file is already unconditionally generated (even for zero-bridge programs) and already `include!`-d into every Cargo-hosted host behind the `#[cfg(snacc_bridge_assertions)]` gate, so this assertion runs on every Cargo-hosted build with zero new machinery. A `snacc-runtime` pinned to the wrong `ABI_VERSION` fails the host's compile with a clear message — before linking, exactly as required.

---

### Task 1: Lexer — `break`, remove semicolons

**Files:**
- Modify: `crates/snacc-compiler/src/syntax/lexer.rs`
- Test: same file's `#[cfg(test)]` (add one if none exists yet — check first)

**Interfaces:**
- Produces: `Token::Break` variant; `Token::Ctrl` no longer accepts `;` — the ctrl-char set `one_of("()[];,:")` drops `;`. A bare `;` in source must produce a lex/parse error naming that Snacc has no semicolon syntax (a `Rich::custom` diagnostic at the semicolon's span is the simplest place for this — the ctrl-char set already fails to match unknown characters, so removing `;` from it is likely sufficient by itself given the existing `recover_with` error path; confirm what error message that currently produces for an unmatched character and decide whether it's clear enough or needs an explicit case).

- [ ] **Step 1: Write the failing tests**

Add lexer tests: `break` lexes to `Token::Break`; `break` is unavailable as an identifier (i.e. `Token::Ident("break")` never occurs — it must lex to `Token::Break` regardless of surrounding context, matching how `while`/`if`/etc. already work); a source containing `;` produces a lex or parse error (check both `crate::parse("let x: Int64 = 1;")` style — decide during implementation whether this is best caught at the lexer or left to the parser now that `;` is an unmatched character, and write the test against whichever layer actually produces the diagnostic).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p snacc-compiler lexer::`

- [ ] **Step 3: Implement**

Add `Break` to `Token`, its `Display` impl (`"break"`), and the `ident` closure's keyword-mapping match arm (`"break" => Token::Break`). Remove `';'` from the `ctrl` combinator's `one_of("()[];,:")`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p snacc-compiler lexer::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/snacc-compiler/src/syntax/lexer.rs
git commit -m "feat(compiler): add break token and remove semicolons from the lexer"
```

---

### Task 2: AST — `Block`, `BlockElement`, `IfForm`, optional results

**Files:**
- Modify: `crates/snacc-compiler/src/syntax/ast.rs`

**Interfaces:**
- Consumes: nothing new (pure data-shape change).
- Produces: the exact types in "Design decisions" item 2 above. `Func.ret: Option<TypeName>`, `Func.body: Block<'src>`. `ExternFunc.ret: Option<TypeName>`. `Program.body: Block<'src>` (not `Option`).

This task only changes type definitions — it will not compile standalone since `parser.rs` still constructs the old shapes. That's expected; Task 3 fixes it. Do not attempt to make this task's diff compile in isolation; verify by reading, not by `cargo check`, and say so in the report.

- [ ] **Step 1: Implement the new types**

Replace the relevant portions of `ast.rs` with the shapes from "Design decisions" item 2. Remove `Expr::Let`, `Expr::Then`, `Expr::While`. Keep `NumLiteral`, `Value`, `BinaryOp`, `Param`, `TypeName` unchanged. Update `Func`, `ExternFunc`, `Program` per the interfaces above.

- [ ] **Step 2: Commit**

```bash
git add crates/snacc-compiler/src/syntax/ast.rs
git commit -m "feat(compiler): introduce Block/BlockElement/IfForm AST, optional results"
```

Note in the report that this commit does not compile standalone (expected, `cargo check` will fail here) — Task 3 restores compilation.

---

### Task 3: Parser — block-element grammar

**Files:**
- Modify: `crates/snacc-compiler/src/syntax/parser.rs`
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `Block`, `BlockElement`, `IfForm`, `Token::Break` (Tasks 1-2).
- Produces: `pub fn block_parser<'tokens, 'src: 'tokens, I>() -> impl Parser<..., Block<'src>, ...>` (or fold this directly into `expr_parser`/`program_parser`'s replacement — your call on the exact function boundary, but the *program* itself is now parsed as one `Block` of top-level items, since `program = { top-level-declaration | block-element }` per spec012 section 4 interleaves function/type/bridge declarations with executable block-elements at the top level, matching the current parser's existing interleaving of `Item::Func`/`Item::Extern`/`Item::Expr`).

**Design notes for this task specifically:**

- A block is `{ block-element }` — parse block-elements one at a time until the enclosing terminator (`end`, `elseif`, `else`, or end-of-input at the top level). Chumsky's `.repeated()` combined with a `.and_is(terminator.not())` guard, or simply looping until one of the terminator tokens is peeked, are both reasonable — pick whichever reads cleanest against this codebase's existing combinator style.
- Distinguish `let x: Int64 = expr` (declaration) from `x = expr` (assignment) from a bare expression by looking at the leading tokens: `Token::Let` starts a declaration; `Token::Ident(name)` immediately followed by `Token::Op("=")` (not `Token::Op("==")` — these are already distinct tokens from the lexer, since `op` lexes greedily via `one_of("+*-/!=<>").repeated().at_least(1)`, so `=` and `==` are already different `Token::Op` payloads) starts an assignment; anything else is parsed as an ordinary expression. Use `chumsky`'s lookahead/backtracking as needed (e.g. try assignment, fall back to expression, or peek two tokens before committing) — whichever approach this chumsky version supports cleanly; check chumsky 0.13's actual combinators rather than assuming.
- `while`, `break`, `if` each parse as their own `BlockElement` variant. `if`'s `else_tail`-style recursive elseif-chain from the current parser is a reasonable pattern to adapt — but note branches are now `Block`, not `Expr`, and `else` is *optional* (unlike today).
- Function/method/bridge declarations parse an *optional* `: type` (use `.or_not()` where the current parser has `.then_ignore(just(Token::Ctrl(':'))).then(type_name)` unconditionally).
- Remove the `';'`-driven `Expr::Then` fold entirely — there is no more sequencing operator; a block's elements are just whatever the repeated block-element parser collects.
- Keep the existing duplicate-function/duplicate-link-symbol validation in `program_parser`'s `.validate(...)` — it doesn't need to change shape, just needs to build a `Block` for the top-level executable elements instead of folding an `Expr::Then` chain.

- [ ] **Step 1: Write the failing tests**

Port every existing parser test in this file to the new grammar (they currently use semicolons and `while`-as-expression — e.g. `parses_while_do_as_an_expression` must become something like `parses_while_as_a_statement`, dropping the `print(while ...)` case since that's no longer valid syntax, and dropping the `;`-separated case). Add new tests: `break` inside a `while` body parses; a function with no `: type` parses with `ret: None`; a bridge with no `: type` parses; an `if` with no `else` parses as one block element; two block-elements on one line with no separator parse identically to two lines (mirroring spec012 section 4's `let x: Int64 = 10 print(x)` vs. multi-line example — port this exact case); a `;` in source fails to parse.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p snacc-compiler parser::` (expect compile errors first, since Tasks 1-2 already landed but nothing yet produces the new AST correctly from source — that's the RED state).

- [ ] **Step 3: Implement the block-element parser and rewire `program_parser`**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p snacc-compiler parser::`
Expected: PASS, including every ported test.

- [ ] **Step 5: Commit**

```bash
git add crates/snacc-compiler/src/syntax/parser.rs
git commit -m "feat(compiler): parse blocks, statements, and optional results"
```

---

### Task 4: Checker — statement/expression split, `Option<Ty>` signatures, loop targets

**Files:**
- Modify: `crates/snacc-compiler/src/semantics/checker.rs`
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `Block`, `BlockElement`, `IfForm` (Task 3's parser output via `AstProgram`).
- Produces: the checked-IR shapes described in RFC 008's "Checked representation" section — read it directly. Concretely, this task needs (design these consistently with the existing `TExpr`/`TFunc`/`TExtern`/`Program` shapes already in this file, extending rather than replacing what still applies):
  - `TFunc.result: Option<Ty>` (was `ret: Ty`), `TExtern.result: Option<Ty>` (was `ret: Ty`) — **rename the field**, don't just change its type, so every call site is forced to update rather than silently compiling against a wrong-shaped `Option`.
  - A checked statement type (e.g. `TStmt`) separate from `TExpr`, covering: `Let { mutable: bool, name: String, ty: Ty, value: TExpr }`, `Assign { name: String, value: TExpr }`, `While { condition: TExpr, body: TBlock }`, `Break`, `If(TIfForm)` (a checked if that may or may not produce a value — see below), and `CallStatement` for a no-result call used as a block element (a no-result call is **not** a `TExpr` — `TExpr::Call` should now only represent a value-producing call; add a distinct `TStmt::Call` or similar for the no-result case, per RFC008: "a no-result call becomes a checked call statement").
  - A checked block type (e.g. `TBlock { statements: Vec<TStmt>, result: Option<TExpr> }`) — `result` is `Some` only when the block is value-required and its final element supplied a value.
  - Two checking entry points, not one boolean threaded everywhere: e.g. `fn check_value_block(ctx, env, block: &Block, expected: Ty) -> TBlock` and `fn check_statement_block(ctx, env, block: &Block) -> TBlock`. RFC008 phase 2 step 2 explicitly asks for this shape ("explicit checking entry points... rather than a boolean threaded through arbitrary expressions").
  - A loop-target stack (`Vec<()>` or similar — it just needs to exist and be non-empty when checking a `break`) pushed only while checking a `while` body, popped on exit, so `break` outside any loop is a clean error via an empty stack.
  - Contextual `if` classification exactly as RFC008 describes: a statement-form `if` (not the final element of a value-required block) checks each branch as a `check_statement_block`; a value-form `if` (the final element of a value-required block) requires either an `else` or — this milestone has no unions yet, so in practice **requires `else`** unconditionally; Specification 010 adds the exhaustive-union-chain alternative later. Do not build the exhaustive-chain machinery now — it has no union type to exhaustively check against yet, and doing so would be speculative work this milestone's tests can't exercise.
  - Extend the existing `is_rust_identifier`/duplicate-param logic's *pattern* (a function-wide reserved-name check) — do not implement Specification 012's full "one name per function including nested blocks and type-test bindings" rule yet (that needs struct/union bindings that don't exist until Specification 010); for this milestone, keep whatever the current duplicate-parameter check already covers and add duplicate-*local*-declaration rejection using the same function-wide reserved-name-set approach, scoped to what's reachable now: parameters and `let`-declared locals within one function/method body. Read spec012 section 5.2 for the exact rule shape to follow even though its full scope (type-test bindings, etc.) isn't reachable until later milestones.

- [ ] **Step 1: Write the failing tests**

Cover RFC008's own "Conformance tests" list items 1-3, 6-7 (items 4-5, 8-13 need LLVM execution or bridge/ABI machinery from later tasks) at the checker level: functions/bridges with and without results type-check; a no-result call is accepted as a block element and rejected in an expression position (argument, arithmetic operand, let-initializer); a value-required body ending in a statement (assignment, `let`, bare `while`, bare no-result call) is rejected; `break` outside a loop is rejected; statement-form `if` accepts omitted `else`; value-form `if` without `else` is rejected (no union exhaustive-chain exists yet, so this is unconditional at this milestone). Also cover spec012's declaration rules reachable now: duplicate local name in the same function is rejected (including across a nested `if` branch vs. the outer block — spec012's `if ready() then let value... end` example); assignment to a name that isn't declared `mut` is rejected; assignment type-mismatch is rejected.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p snacc-compiler checker::`

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p snacc-compiler`
Expected: PASS, including every pre-existing test not superseded by this migration (update/remove tests that assert old semicolon/expression-`while`/expression-`let` behavior — they test syntax that no longer exists).

- [ ] **Step 5: Commit**

```bash
git add crates/snacc-compiler/src/semantics/checker.rs
git commit -m "feat(compiler): check statements, no-result blocks, and loop targets"
```

---

### Task 5: LLVM backend — void functions, statement lowering, loop targets

**Files:**
- Modify: `crates/snacc-compiler/src/backend/llvm.rs`

**Interfaces:**
- Consumes: `TStmt`, `TBlock`, `TFunc.result: Option<Ty>`, `TExtern.result: Option<Ty>` (Task 4).

**Design notes:**

- A function/method with `result: None` lowers to an LLVM function returning `void` (Inkwell: `context.void_type().fn_type(...)`, and its body emits `builder.build_return(None)` rather than a value return). A call to it emits no result `BasicValueEnum` — lower it as a statement (`builder.build_call(...)` and discard/ignore the returned `Either<BasicValueEnum, InstructionValue>`, which Inkwell's `build_call` already returns in a form that doesn't force you to unwrap a value).
- Lower each `TBlock` by iterating its statements in order, tracking whether the current LLVM basic block has already been terminated (i.e. hit a `break` or an unconditional return) — **do not emit any instruction into a block after it's been terminated**, and do not add a fallthrough/merge branch from an already-terminated block. This is the single most likely source of an LLVM verifier failure in this task; track it explicitly (a simple `bool` or a helper that checks the current block's terminator) rather than assuming structured control flow always leaves you in a live block.
- Loop lowering: three blocks (condition, body, exit), matching RFC008's "LLVM lowering" section exactly. Maintain a stack of exit blocks (parallel to the checker's loop-target stack, but this one is real Inkwell `BasicBlock` values) so a `break` inside a nested loop branches to the *innermost* exit block. Push when entering a loop body's lowering, pop on exit.
- `break` lowers to an unconditional branch to the top of the current exit-block stack, then marks its containing LLVM block terminated (nothing after a `break` in the same block should ever lower — the checker doesn't need to prove unreachability, but the backend must not emit dead instructions past a terminator either way, per RFC008: "This RFC does not require a diagnostic for statically unreachable source after `break`; if accepted, that source shall not be lowered as reachable code").
- Statement-form `if` lowers ordinary branch control flow merging only control (a merge block with no phi). Value-form `if` lowers one result phi **only when the value isn't already materialized through addressable storage** — at this milestone there's no addressable-storage concept yet (that's spec012's aggregate-place work in Milestone 3), so just implement the phi path; RFC008's caveat about avoiding a phi is here so later milestones can skip materializing one for values that already live in memory, which isn't reachable yet.
- Delete `default_value` (currently `crates/snacc-compiler/src/backend/llvm.rs:23`) and the loop-result phi it fed (currently around `llvm.rs:481-500`, the `TExpr::While` arm). Search the rest of the file for any other zero-fallback call before considering this task done — RFC008 explicitly calls this out as required cleanup, not optional.
- `TExpr::Let`/`TExpr::Then`/`TExpr::While` no longer exist in `TExpr` (Task 4 already removed them from the checked IR) — this task's `lower` function for `TExpr` shrinks accordingly; the removed cases move to a new `lower_stmt`/`lower_block`-shaped function pair.

- [ ] **Step 1: Write the failing tests**

Add execution tests (likely in `crates/snacc-compiler/tests/` alongside existing phase tests, or `tests/cases/run/pass/` corpus cases consumed by `apps/snacc/tests/conformance.rs` — check which this codebase already uses for LLVM-execution-level tests and follow that pattern) covering RFC008's conformance list items 4, 8, 9: `while` executes correctly for zero/one/multiple iterations with no value; early `break`; nested `break` selecting the correct (innermost) loop; a terminated LLVM block never receives a second terminator (this is best proven indirectly — by every other execution test actually running rather than hitting an LLVM verifier panic — rather than a single dedicated test; note this in the report).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p snacc-compiler` and `cargo test -p snacc --test conformance`

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p snacc-compiler && cargo test -p snacc --test conformance`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/snacc-compiler/src/backend/llvm.rs
git commit -m "feat(compiler): lower void functions, statements, and loop targets to LLVM"
```

At this point `cargo check --workspace --all-targets` will still fail outside `snacc-compiler` — `apps/cargo-snacc`, `apps/snacc`, `apps/snacc-workbench`, and the test corpus all still assume the old `TFunc.ret: Ty` / `TExtern.ret: Ty` shape and semicolon syntax. That's expected; Tasks 6-8 fix it. Note this explicitly in the report.

---

### Task 6: Bridge ABI 2 — no-result bridges, ABI version bump, runtime constant

**Files:**
- Modify: `crates/snacc-compiler/src/lib.rs` (`ABI_VERSION`)
- Modify: `crates/snacc-runtime/src/lib.rs` (new `ABI_VERSION` constant)
- Modify: `apps/cargo-snacc/src/main.rs` (`render_bridge_assertions`, `rust_abi_type`-equivalent for results, `emit_cached`'s identity hash, the ABI-mismatch assertion described in this plan's "Design decisions" item 4)
- Test: `apps/cargo-snacc/tests/cargo_hosted.rs`, `tests/fixtures/cargo-hosted/`

**Interfaces:**
- Consumes: `TExtern.result: Option<Ty>` (Task 4), `snacc_compiler::Ty`/`Program` exports (already public from RFC 007).

**Design notes:**

- `snacc_compiler::ABI_VERSION` goes from `1` to `2` (`crates/snacc-compiler/src/lib.rs`).
- Add `pub const ABI_VERSION: u32 = 2;` to `crates/snacc-runtime/src/lib.rs`.
- In `apps/cargo-snacc/src/main.rs`, `render_bridge_assertions` (from RFC 007) currently always writes a return type via `rust_abi_type(extern_decl.ret)`. With `TExtern.result: Option<Ty>`, render `None` as Rust `()` in the generated `const _: unsafe extern "C" fn(...) -> R = ...;` line — write `-> ()` explicitly (matching the spec's exact wording: "Its Rust assertion result is `()`"), not by omitting the arrow (an omitted `-> R` in Rust already implicitly means `()`, but writing it explicitly keeps every generated line the same shape and makes the assertion's intent legible from a diagnostic).
- Add the ABI-version assertion line described in "Design decisions" item 4 to the same generated file, using the *current* `snacc_compiler::ABI_VERSION` value (don't hardcode `2` in `cargo-snacc`'s own source beyond one place — read it from the constant so a future ABI bump only requires changing `snacc_compiler::ABI_VERSION` and the assertion follows automatically).
- `emit_cached`'s identity hash already includes `snacc_compiler::ABI_VERSION.to_le_bytes()` — confirm this still fires correctly with the new value (no code change needed there beyond the constant itself changing, but verify with a test that an ABI-1-identity cached object is never reused after this change, per RFC008 conformance test 11's "ABI 1 cache objects are not reused").
- The fixture at `tests/fixtures/cargo-hosted/` (`src/main.nrs`, `src/interop.rs`) needs its Snacc source migrated to the new semicolon-free syntax (it currently has two top-level `print(...)` statements separated by `;` — check current content and update). It does not need a no-result bridge added for this task alone, but Step 3 below asks for one to prove the ABI-2 path end to end.

- [ ] **Step 1: Write the failing tests**

In `apps/cargo-snacc/tests/cargo_hosted.rs`: a bridge declared `extern rust "snacc_user_log" fun log(value: Int64)` (no result) round-trips through a real `cargo snacc run` — the generated assertion asserts `-> ()`, the Rust side is a `fn(i64)` with no return type, and the program compiles/links/runs correctly calling it as a statement. Also: an `snacc-runtime` version deliberately pinned/patched to report the wrong `ABI_VERSION` (you'll need a way to construct this — e.g. a fixture variant whose `Cargo.toml` patches `snacc-runtime` to a local copy with a modified constant, or a unit-level test of the generated assertion content asserting the `assert!(snacc_runtime::ABI_VERSION == 2, ...)` line is present and correctly worded, if a full ABI-mismatch integration test proves impractical to construct cleanly — use your judgment and explain the choice in the report) fails the host build with a clear diagnostic before linking.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cargo-snacc --test cargo_hosted`

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cargo-snacc` (full crate — unit + integration)
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/snacc-compiler/src/lib.rs crates/snacc-runtime/src/lib.rs apps/cargo-snacc/src/main.rs apps/cargo-snacc/tests/cargo_hosted.rs tests/fixtures/cargo-hosted/src/main.nrs tests/fixtures/cargo-hosted/src/interop.rs
git commit -m "feat(cargo-snacc): no-result bridges and ABI version 2"
```

---

### Task 7: Contract and corpus migration

**Files:**
- Modify: `LANGUAGE.md`, `GRAMMAR.ebnf`
- Modify: every `.nrs` file under `tests/cases/`, `examples/`, `apps/snacc-workbench/snippets.json`-referenced examples, and any inline Snacc source string in test files across the workspace (search, don't guess which ones)

**Interfaces:** none new — this is a documentation and corpus-migration task.

**Design notes:**

- Update `LANGUAGE.md`'s EBNF fence first (the grammar productions from RFC008 section "Grammar" and spec012 section 4, scalar-only), then copy it byte-identical into `GRAMMAR.ebnf`, per `AGENTS.md`'s rule that the two must always match.
- Update `LANGUAGE.md`'s prose: no-result functions/bridges, `while` as a statement, `break`, statement-form vs. value-form `if`, declaration/assignment statements, root mutability (`let` vs `let mut`), no semicolons. Do not describe struct/union/`Ref<T>`/method syntax here — those aren't implemented yet.
- Search the whole repository for `;` inside `.nrs` files and inline Snacc source strings (Rust string literals containing Snacc source, e.g. in checker/parser/llvm tests, `apps/snacc/tests/`, `apps/snacc-workbench/`) and migrate every one to semicolon-free block syntax. Search for expression-form `let` and expression-form `while` usage the same way.
- RFC008's own migration example (`fun zero_after_loop... while false do print(value) end 0 end`) is the pattern for any corpus case that currently relies on a loop's old zero-value fallback — check `tests/cases/` and `examples/` for any such case and rewrite it to an explicit trailing value per that pattern.

- [ ] **Step 1: Update `LANGUAGE.md` then `GRAMMAR.ebnf`**

- [ ] **Step 2: Migrate the corpus**

Search: `grep -rn ';' tests/cases/ examples/ apps/snacc-workbench/` and every Rust test file with an inline `.nrs`-shaped string literal (`crates/snacc-compiler/tests/`, `crates/snacc-compiler/src/**/*.rs` test modules, `apps/snacc/tests/`, `apps/cargo-snacc/tests/`, `apps/snacc-workbench/src/lib.rs` test module). Migrate each.

- [ ] **Step 3: Verify**

Run: `cargo test --workspace` — expect it to still fail at this point only where Task 8's new/updated conformance-suite assertions haven't landed yet; every corpus/example file itself should now parse and check under the new grammar.

- [ ] **Step 4: Commit**

```bash
git add LANGUAGE.md GRAMMAR.ebnf
git commit -m "docs: update the language contract for statements and no-result functions"
```

(Commit the corpus migration separately, scoped to exactly the files changed — list them explicitly rather than a wildcard, since this repo has substantial unrelated pre-existing uncommitted work elsewhere.)

---

### Task 8: Final verification

**Files:** none new.

- [ ] **Step 1:** `cargo fmt --all -- --check` (fix and include if dirty)
- [ ] **Step 2:** `cargo check --workspace --all-targets`
- [ ] **Step 3:** `cargo test --workspace` — full green
- [ ] **Step 4:** Walk RFC 008's 13 conformance-test items and 7 acceptance criteria one by one against the actual code/tests; report any gap honestly rather than assuming Tasks 1-7 covered everything.
- [ ] **Step 5:** Commit any final fixup, scoped by exact file path.

## Execution

Dispatch via `superpowers:subagent-driven-development`, one implementer per task, **strictly sequential** (never parallel — every task depends on the previous one's compiling state). Task-review each before proceeding, per that skill's process.
