# TODOs

## Known issues

1. [ ] Use a constant-time comparison for the workbench session token.
   `apps/snacc-workbench/src/lib.rs:601` compares with `==` on `String`, which
   short-circuits. RFC 004's security model requires constant-time comparison.

## Missing verification

Behavior that RFCs 004-006 specified and the implementation provides, but that
no test exercises. Each one can regress silently today.

2. [ ] Add workbench request-rejection tests. Every control is implemented and
   none is tested: missing or wrong session token, foreign `Origin`, mismatched
   `Host`, unsupported method (405 plus `Allow`), non-JSON `Content-Type`,
   malformed JSON, unknown JSON fields, each size limit, 429 while a run is
   active, and compile failure yielding a null `execution` object.

3. [ ] Add workbench process tests: supplied stdin reaches a helper child and
   then observes EOF; simultaneous stdout and stderr larger than the pipe
   buffers do not deadlock (this is why draining is threaded); timeout kills and
   waits for the child; a build path containing spaces still compiles and runs.

## Housekeeping

4. [ ] After all acceptance criteria are verified, change RFCs 004, 005, and
   006 to `Status: Closed` and move them to `docs/specs/archive/` in the same
   change. `Status: Completed` is not a permitted terminal status.

