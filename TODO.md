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

3. [ ] Use a constant-time comparison for the workbench session token.
   `apps/snacc-workbench/src/lib.rs:601` compares with `==` on `String`, which
   short-circuits. RFC 004's security model requires constant-time comparison.

## Missing verification

Behavior that RFCs 004-006 specified and the implementation provides, but that
no test exercises. Each one can regress silently today.

4. [ ] Add workbench request-rejection tests. Every control is implemented and
   none is tested: missing or wrong session token, foreign `Origin`, mismatched
   `Host`, unsupported method (405 plus `Allow`), non-JSON `Content-Type`,
   malformed JSON, unknown JSON fields, each size limit, 429 while a run is
   active, and compile failure yielding a null `execution` object.

5. [ ] Add workbench process tests: supplied stdin reaches a helper child and
   then observes EOF; simultaneous stdout and stderr larger than the pipe
   buffers do not deadlock (this is why draining is threaded); timeout kills and
   waits for the child; a build path containing spaces still compiles and runs.

6. [ ] Add `snacc-runtime` ABI coverage in `crates/snacc-runtime/tests/` for
   each exported `snacc_print_*` symbol and the `force_link` retention
   contract.

## Housekeeping

7. [ ] After all acceptance criteria are verified, change RFCs 004, 005, and
   006 to `Status: Closed` and move them to `docs/specs/archive/` in the same
   change. `Status: Completed` is not a permitted terminal status.

