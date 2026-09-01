# TODOs

## Known issues

1. [ ] Parse `Int64` literals without passing through `f64`.
   `Token::Num(f64, bool)` in `crates/snacc-compiler/src/syntax/lexer.rs:8` makes
   every literal a `f64`, and `crates/snacc-compiler/src/backend/llvm.rs:201`
   lowers it with `*value as u64`. Both losses are silent:
   `print(9007199254740993)` emits `9007199254740992`, and
   `print(9223372036854775807)` emits `-9223372036854775808`.
   Carry the integer literal as `i64` from the lexer through `TExpr::Num`,
   reject out-of-range values with a source diagnostic, and add boundary tests
   at `2^53`, `2^53 + 1`, `i64::MAX`, `i64::MIN`, and one value past each end.

2. [ ] Reject duplicate function and external-function parameter names.
   `fun f(a: Int64, a: Int64)` compiles today and silently binds the last `a`
   (`crates/snacc-compiler/src/semantics/checker.rs:160` pushes onto a `Vec`
   env without a uniqueness check). Duplicate *function* names are already
   rejected; parameters are not. Emit the same structured diagnostic shape,
   spanned on the second occurrence.

3. [ ] Make object-cache publication safe for concurrent builds.
   `apps/cargo-snacc/src/main.rs:742` and `:757` write the fixed names
   `app.tmp` and `manifest.tmp` in the shared cache directory before renaming.
   Two concurrent `cargo snacc build` runs clobber each other's temporary file
   and can publish a partially written object. Use unique temporary siblings
   (`app.<random>.tmp`) so each writer owns its file, keeping the atomic rename.

4. [ ] Use a constant-time comparison for the workbench session token.
   `apps/snacc-workbench/src/lib.rs:601` compares with `==` on `String`, which
   short-circuits. RFC 004's security model requires constant-time comparison.

## Missing verification

Behavior that RFCs 004-006 specified and the implementation provides, but that
no test exercises. Each one can regress silently today.

5. [ ] Make the conformance runner execute every `examples/*.nrs` with a
   `.stdout` sidecar. `apps/snacc/tests/conformance.rs:24` reads only
   `tests/cases/run/pass/`, and `apps/snacc-workbench/build.rs:39` checks the
   sidecar exists without running it, so the four shipped workbench snippets are
   unverified. Their expected output is correct as of this writing; nothing
   keeps it correct.

6. [ ] Add workbench request-rejection tests. Every control is implemented and
   none is tested: missing or wrong session token, foreign `Origin`, mismatched
   `Host`, unsupported method (405 plus `Allow`), non-JSON `Content-Type`,
   malformed JSON, unknown JSON fields, each size limit, 429 while a run is
   active, and compile failure yielding a null `execution` object.

7. [ ] Add workbench process tests: supplied stdin reaches a helper child and
   then observes EOF; simultaneous stdout and stderr larger than the pipe
   buffers do not deadlock (this is why draining is threaded); timeout kills and
   waits for the child; a build path containing spaces still compiles and runs.

8. [ ] Add focused `snacc-driver` tests in `crates/snacc-driver/tests/`:
   structured diagnostics for invalid source, an isolated build directory per
   call, a changed source not reusing a stale executable, runtime output landing
   on stdout rather than stderr, and filesystem or missing-`rustc` failures.

9. [ ] Add `snacc-runtime` ABI coverage in `crates/snacc-runtime/tests/` for
   each exported `snacc_print_*` symbol and the `force_link` retention
   contract.

10. [ ] Extend `tools/package-windows.ps1` so `-IncludeDirectCompiler` compiles
    and runs a Snacc program from the staged package before publication, with no
    repository runtime source, Cargo, registry, or network access available.
    Verified by hand once; not automated.

## Housekeeping

11. [ ] `LANGUAGE.md` and `GRAMMAR.ebnf` are both empty while RFCs 005 and 006
    cite `LANGUAGE.md` as the sole normative language contract and RFC 006's
    motivation describes a pipeline it claims that file defines. The real rules
    currently live in `docs/types.md`.

12. [ ] After all acceptance criteria are verified, change RFCs 004, 005, and
    006 to `Status: Closed` and move them to `docs/specs/archive/` in the same
    change. `Status: Completed` is not a permitted terminal status.

