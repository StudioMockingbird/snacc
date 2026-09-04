# SNACC — Super Nibbly and Comfy Compiler

Snacc is a statically typed AOT language inspired by C, Luau, and Crystal. Its
Rust compiler uses Chumsky for the frontend and Inkwell with LLVM 22 for native
code generation; the influences are not compatibility targets.

## Language Goals & Philosophy

1. Snacc should be cohesive, coherent, consistent, uniform, and intuitive overall.
2. There should be only one obvious way to do things.
3. The language surface must remain small and clean.
4. Prioritize simplicity, elegance, expressiveness, and composability.
5. Snacc aims to be a "better C." Users should be able to do everything with
   Snacc that they can do with C, but with modern sensibilities.
6. Snacc should be statically typed.
7. The compiler should be incremental: recompile only the module that changed.
8. Compilation should be fast.
9. Concurrency and multithreading should be straightforward.
10. Keep ceremony low by minimizing boilerplate and syntactic noise.
11. Promote readability over compactness and complicated instructions.
12. Aim for no or low runtime overhead.
13. The language should have no undefined behavior.
14. "If it compiles, it runs."
15. "If it compiles, it has no memory errors." The compiler should catch every
    memory error that local analysis can detect without adding a language
    concept or disproportionate checker complexity.

## Architecture

- Keep the pipeline forward-only:
  `source -> tokens -> syntax tree -> typed program -> LLVM IR -> native code`.
- Each phase consumes validated output from the previous phase. Do not reparse,
  reinterpret invalid output, or add back channels; make downstream facts
  explicit in the preceding representation.
- Type-check before lowering. Inkwell is the only native-code backend.
- Functions are top-level and have explicit parameter and return flow. Snacc
  has no function values, nested functions, closures, or implicit shared state.
- Fail closed: reject invalid, ambiguous, and unsupported programs with
  structured diagnostics. The earliest capable phase owns the error; impossible
  compiler states are internal errors.
- Handle every supported syntax and type node explicitly and exhaustively.

## Implementation

- Treat `snacc-workbench` as a temporary, internal debugging aid for compiler
  development, not as an external product. As long as it remains functional,
  skip workbench optimization and feature work unless it is strictly required
  for compiler development or the user explicitly requests it.
- Write concrete, procedural Rust with explicit structs/enums, ownership,
  `match`, and ordinary loops. Prefer visible data flow over indirection.
- Add traits, generics, macros, dynamic dispatch, iterator pipelines, or helper
  layers only when they remove real duplication or enforce an invariant.
- Use safe Rust by default. Keep required `unsafe` blocks minimal and document
  their safety contract.
- Remove obsolete compatibility paths when a contract changes. Avoid duplicate
  representations, speculative abstractions, and premature generalization.
- Prefer maintained, widely used crates for solved infrastructure. Verify
  current maintenance, adoption, correctness, and API fit; enable only needed
  features.
- Prefer LLVM 22 capabilities when they reduce compiler complexity or size.
- Before implementing anything, ask in order:

  1. Does this need to exist? If not, skip it (YAGNI).
  2. Does the codebase already provide it? Reuse it.
  3. Does the standard library or native platform provide it? Use that.
  4. Does an existing dependency provide it? Use that.
  5. Does a suitable proven crate provide it? Prefer that to handwritten
     infrastructure.
  6. Can it be one line? Keep it to one line.
  7. Otherwise, implement the smallest complete design with the least state and
     layering.

## Documentation

- `LANGUAGE.md` is the sole normative language contract. Keep it terse, clear,
  precise, and exact. It records key language nuances and semantics that are
  difficult to capture in code; it does not replace or duplicate language and
  compiler documentation that belongs in code comments.
- Keep the formal EBNF first in `LANGUAGE.md` so syntax and semantics change
  together. Keep that grammar identical to `GRAMMAR.ebnf`, and always keep both
  documents synchronized with the parser, checker, and implemented behavior.
  A language or compiler change is incomplete until the affected contract and
  grammar text are updated in the same change.
- `TODO.md` tracks only open bugs and small tasks or features. It never defines
  language semantics. Give substantial work its own specification under
  `docs/specs/`.

## Comments

Every comment must add a non-obvious **Contract**, **Architecture**,
**Rationale**, or **Edge** fact. Delete narration and stale provenance; prefer
precise names, write comments in the present tense, and place safety reasoning
beside the operation it protects.

## Change discipline

- Follow existing structure, naming, diagnostics, and test patterns.
- Keep changes scoped and add focused tests.
- Before handoff, run `cargo fmt`, `cargo check`, and relevant `cargo test` suites.
- This directory is not a Git repository; do not run Git commands here.

## Specification format

- Use one permanent sequence across all specifications. Name files
  `NNNN-kebab-name.md`; never renumber or reuse an identifier, and always choose
  a number greater than every previously assigned number.
- Keep active specifications in `docs/specs/`. Move terminal specifications to
  `docs/specs/archive/` in the same change that gives them a terminal status.
- Name the document kind in its header:
  - Feature specification: Rust-style RFC.
  - Language semantics: ISO/IEC language-standard format.
  - Architecture decision: ADR.
  - Execution plan: use an `-plan.md` suffix and link its specification from the
    header.
- The `Status:` header is the sole completion record for an active specification.
- `Closed`, `Discarded`, `Superseded`, and `Rejected` are terminal. Before
  assigning one, verify every current-behavior claim against `LANGUAGE.md` and
  the implementation.
- Archived specifications are immutable historical records. Describe later
  changes in a new specification.
- Active specifications cite `LANGUAGE.md`, not archived specifications, for
  authoritative language rules.
- A specification is implementation-ready only when it has no open design
  questions and contains a section for detailed implementation plan.
