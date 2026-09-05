# Specification 025: Deferred Calls

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Scope and status

This implementation-ready specification adds two scope-exit statements:

- `defer` schedules one no-result call on every supported normal exit from the
  current lexical scope; and
- `defer_on_error` schedules one no-result call only when an active `Error`
  result leaves the current lexical scope.

Both forms extend RFC 016's checked deterministic-cleanup plan. They do not
create a second cleanup engine, exception unwinding, anonymous functions, or
general deferred blocks.

`LANGUAGE.md` remains authoritative until this specification is accepted,
implemented, and incorporated there. This specification has no open design
questions. Section 11 fixes the implementation order.

## 2. Dependencies

This specification depends on:

- [RFC 016](archive/016-box-indirection-and-recursive-data.md) for move tracking,
  deterministic destruction, and checked scope cleanup;
- [Specification 024](archive/024-error-handling.md) for the exact predeclared `Error`
  type, error-result classification, and `return_on_error`; and
- [Specification 026](archive/026-return-statements.md) for result materialization and
  cleanup-bearing return edges.

When [Specification 022](022-concurrency-and-parallelism.md) is implemented,
its structured task joins precede the user and ownership cleanup described
here.

## 3. Motivation and boundary

Owned Snacc values already destroy themselves on scope exit. `defer` shall not
be the ordinary way to release a `Box`, `List`, `String`, file, socket, or
other owning value. Deterministic destruction remains responsible for owned
resources.

Deferred calls cover cleanup or balancing actions that are not represented by
ownership alone:

~~~snacc
fun guarded_work(lock: Ref<Lock>) do
    acquire_lock(lock)
    defer release_lock(lock)

    perform_work()
end
~~~

`defer_on_error` covers transactional rollback and removal of partially
created external state:

~~~snacc
fun replace_file(path: String, contents: String): Nil | Error do
    let mut temporary: TemporaryFile =
        return_on_error TemporaryFile.create(path)

    defer_on_error temporary.remove()

    return_on_error temporary.write(contents)
    return_on_error temporary.commit()
    return nil
end
~~~

Neither form handles a cleanup call that itself reports an `Error`. A failure
that matters shall be called and handled explicitly before exit.

## 4. Syntax

The grammar adds two block elements:

~~~ebnf
defer-statement          = "defer", postfix ;
defer-on-error-statement = "defer_on_error", postfix ;
~~~

The checker requires the parsed postfix expression to be an ordinary direct
function or method call whose resolved declaration produces no result. Its
receiver and argument expressions use ordinary call syntax.

~~~snacc
defer release_lock(lock)
defer audit(operation, status)
defer_on_error temporary.remove()
~~~

The form does not accept a non-call expression or a `do`/`end` block. It does
not accept a call returning any value, including `Error`, `Nil | Error`, or
another discardable value.

`defer` is valid in any executable lexical block, including the program's root
execution block. The root execution block is the top-level executable sequence
described by `program` in `LANGUAGE.md`; its successful fallthrough is the
successful end of program entry, after which armed `defer` calls run before
process status zero is returned. `defer_on_error` is valid only within a function or method
whose declared result directly contains exact `Error`.

## 5. Arming and evaluation

Execution reaching a deferred statement arms that one action in the current
lexical scope. Reaching the statement does not evaluate its receiver, argument
expressions, or call.

The complete call evaluates later, when an applicable exit crosses that
scope. Receiver and argument expressions evaluate then, from left to right,
under the ordinary call rules. Reads observe the values present at exit, not
the values present when the action was armed.

~~~snacc
let mut status: Int64 = 1
defer record_status(status)
status = 2
~~~

The deferred call observes `2`. A program that needs a snapshot binds it
explicitly:

~~~snacc
let mut status: Int64 = 1
let initial_status: Int64 = status
defer record_status(initial_status)
status = 2
~~~

The second deferred call observes `1`.

This exit-time rule applies uniformly to function receivers, method receivers,
arguments, nested argument expressions, and `Ref<T>` parameters. A deferred
call taking `Ref<T>` creates its reference only while the call executes at
scope exit; no reference is stored from the arming point.

### 5.1 Availability and borrowing

Names in a deferred call resolve at its source position. A declaration that
appears later is not visible to it.

Every place read, borrowed, or mutated by an applicable deferred call shall be
available and legally accessible on every exit on which that call runs. The
checker rejects a move, conflicting borrow, immutable mutation, or other
operation that would make this false.

For `defer`, this requirement applies to every supported exit after the action
is armed. For `defer_on_error`, it applies only to error exits. This allows a
value to move on a provably successful exit when the failure-only action is
skipped, while still rejecting a move before a later possible error exit.

The deferred call itself uses ordinary ownership rules. Passing a value by
value may move it at exit; passing it to `Ref<T>` borrows it only for that
call. No captured environment, closure value, or first-class deferred action
exists at runtime or in the source language.

## 6. Exit classes

### 6.1 Successful exits

A successful exit is:

- normal fallthrough from the current block;
- `break` leaving the current block;
- a bare return from a no-result callable; or
- an explicit or implicit result whose active value is not exact `Error`,
  including active `Nil` in `Nil | Error`.

An armed `defer` runs on a successful exit. An armed `defer_on_error` is
discarded without evaluating its call.

### 6.2 Error exits

An error exit occurs when an active exact `Error` member in the callable result
crosses the scope while leaving the callable. This includes:

- `return error` when `error` has exact type `Error` and is injected into the
  declared result;
- an explicit or implicit return of an inline sum whose active member is
  `Error`; and
- propagation by `return_on_error`.

Both armed forms run on an error exit. The active runtime tag determines the
classification when a returned sum might hold either success or `Error`.

Merely constructing or storing an `Error`, handling one and continuing, or
returning a different active member does not create an error exit. Runtime
traps are not error exits.

### 6.3 Lexical lifetime

A deferred action belongs only to the lexical scope containing its statement.
It is removed when that scope exits. A `defer_on_error` skipped during normal
fallthrough from an inner block does not remain armed for a later error from an
outer block.

~~~snacc
if needs_temporary then
    let mut temporary: TemporaryFile = return_on_error create_temporary()
    defer_on_error temporary.remove()
    return_on_error prepare(temporary)
end

// The inner action no longer exists here.
return_on_error later_operation()
~~~

## 7. Ordering

Each lexical scope has one ordered cleanup plan. Successful initialization of
an owned local appends its destruction entry. Execution reaching `defer` or
`defer_on_error` appends its deferred-call entry. Scope exit processes
applicable entries in reverse registration order.

Skipped `defer_on_error` entries do not disturb the order of other entries.

~~~snacc
defer always_a()
defer_on_error failure_a()
defer always_b()
defer_on_error failure_b()
~~~

On successful exit the call order is:

~~~text
always_b
always_a
~~~

On error exit the call order is:

~~~text
failure_b
always_b
failure_a
always_a
~~~

Local destruction participates in the same reverse order:

~~~snacc
let first: First = make_first()
defer after_first()
let second: Second = make_second()
~~~

The exit order is:

1. destroy `second`;
2. call `after_first()`; and
3. destroy `first`.

A deferred call can refer only to declarations visible where it appears, so
every referenced local has an earlier cleanup entry and remains alive while
the call runs.

Nested scopes exit from innermost to outermost, processing each scope's plan
completely before beginning its parent plan.

The interaction with an early return is explicit:

~~~snacc
fun example(): Int64 do
    let first: First = make_first()
    defer after_first()
    let second: Second = make_second()
    defer after_second()
    return 7
end
~~~

The return value is materialized first. The remaining exit plan then runs in
reverse registration order: `after_second()`, destruction of `second`,
`after_first()`, and destruction of `first`.

## 8. Return, loops, and conditionals

A return expression evaluates exactly once and its result is materialized
before any deferred call or local destruction. The active result then selects
successful or error cleanup. Cleanup cannot read or mutate the already
materialized result except through independent aliases permitted by ordinary
ownership rules.

An action in a conditional arm is armed only if execution reaches it. An
action in a loop body's lexical scope runs when that iteration exits; actions
do not accumulate until the enclosing function returns.

~~~snacc
while has_work() do
    let item: Item = next_item()
    defer finish_iteration()
    process(item)
end
~~~

`finish_iteration()` runs once at the end of each reached iteration, including
an iteration exited by `break`.

## 9. Structured concurrency and traps

When a scope owns structured child tasks, it first joins every child required
by Specification 022. Only after the join completes does it process deferred
calls and local destruction. This prevents cleanup from accessing storage that
a child may still borrow. Until Specification 022 is implemented, no parallel
scope exists and this clause adds no scheduler or runtime dependency; ordinary
lexical cleanup ordering applies.

Runtime traps and process termination do not execute deferred calls or
deterministic destruction. Snacc does not add stack unwinding for these
features. If a deferred call itself traps, execution terminates and later
cleanup entries do not run.

## 10. Required diagnostics

The frontend diagnoses at least:

- either keyword followed by something other than a direct function or method
  call;
- a deferred call that declares any result;
- `defer_on_error` outside a function or method;
- `defer_on_error` in a callable whose declared result does not directly
  contain exact `Error`;
- a name in the call that is not visible at the deferred statement;
- a referenced place unavailable on any applicable exit;
- a move, immutable mutation, overlapping reference, or borrow conflict on an
  applicable exit;
- a deferred call that would use a local after its destruction;
- a compiler cleanup plan whose entries are not totally ordered; and
- a return or structured-scope exit for which required deferred cleanup cannot
  be emitted.

Diagnostics identify the deferred statement and the conflicting operation or
exit when both locations are relevant.

## 11. Detailed implementation plan

### Phase 1: syntax and parsed representation

1. Reserve `defer` and `defer_on_error` as keywords.
2. Add the two grammar productions in section 4 as block elements, preserving
   the language's whitespace-insensitive and semicolon-free syntax.
3. Parse exactly one call-shaped expression after each keyword and retain the
   full statement and call spans.
4. Add distinct parsed nodes for unconditional and error-only deferred calls;
   do not encode either as an ordinary expression statement.

### Phase 2: resolution and call checking

1. Resolve deferred receivers and arguments in the environment at the source
   statement, without evaluating or moving them there.
2. Reuse ordinary call resolution, arity, parameter, method receiver, and
   result checking.
3. Require the resolved callable to produce no result.
4. For `defer_on_error`, require an enclosing callable result with exact
   `Error` as a direct member.
5. Store a checked call plan referencing resolved place identities and checked
   expressions; do not synthesize a closure, capture tuple, or runtime
   registration object.

### Phase 3: exit-sensitive ownership analysis

1. Add an armed deferred entry to flow state only after control reaches its
   statement.
2. Extend each control-flow edge with the successful or error classification
   defined in section 6.
3. At every scope exit, filter error-only entries by that classification and
   traverse the remaining cleanup entries in reverse registration order.
4. Check availability, moves, root mutability, and reference overlap at the
   point each deferred call executes, not at its source statement.
5. Merge armed-entry state across branches only when the corresponding source
   statement was reached on that predecessor; reject a plan that cannot
   represent the path distinction explicitly.
6. Treat each loop iteration body as a fresh lexical cleanup scope and compute
   ownership state to the existing loop fixed point.
7. Permit an error-only dependency to be unavailable exclusively on proven
   successful exits, and reject it on any possible error exit.

### Phase 4: unified checked cleanup plans

1. Replace a locals-only destruction list with one ordered cleanup-entry type
   containing local destruction, `defer`, and `defer_on_error` variants.
2. Append local destruction after successful initialization and append a
   deferred entry after its statement executes.
3. Attach the fully ordered, path-specific plan to fallthrough, return,
   `return_on_error`, and `break` edges.
4. Materialize callable results before running their exit plans.
5. Place structured-concurrency joins before the cleanup plan for each exited
   parallel scope.
6. Assert exactly one destruction obligation for every live owned value and
   exactly one execution of every applicable armed deferred call.

### Phase 5: LLVM lowering

1. Lower only the checked cleanup plan; do not reconstruct scope, ownership,
   error classification, or call ordering in the backend.
2. Emit the deferred call at each applicable exit using ordinary call
   lowering, evaluating its receiver and arguments at that exit.
3. Branch on the materialized result's sum tag when the success/error class is
   dynamic, and emit the corresponding checked plans before their return
   terminators.
4. Reuse cleanup blocks when their checked ordered plans are identical, without
   reordering observable calls.
5. Verify every LLVM module and treat a missing dependency, invalid call plan,
   double cleanup, or unclassified exit as an internal error.

### Phase 6: tests and contract synchronization

1. Add lexer and parser tests for both keywords, functions, methods, malformed
   calls, and rejection of blocks and non-call expressions.
2. Add checker tests for every diagnostic in section 10, including late
   moves, mutable receivers, `Ref<T>`, branch-dependent arming, and
   error-only path sensitivity.
3. Add conformance cases for fallthrough, return, `return_on_error`, explicit
    `Error` returns, nested scopes, branches, loops, and `break`.
4. Add observable ordering tests that interleave both deferred forms with
   local destruction and prove the exact orders in section 7.
5. Add tests proving exit-time evaluation, explicit snapshots, no evaluation
   of skipped error-only calls, and exactly-once execution.
6. Add structured-concurrency tests proving joins precede deferred calls and
   destruction when Specification 022 lands.
7. Update the formal grammar first in `LANGUAGE.md`, copy it identically to
   `GRAMMAR.ebnf`, and update the terse normative block, scope, ownership,
   return, error, and trap text in the same implementation change.
8. Run formatting, workspace checks, and the complete workspace test suite.

## 12. Acceptance criteria

Implementation is complete only when:

1. `defer` and `defer_on_error` accept exactly one no-result direct call;
2. reaching a statement arms its action without evaluating the call;
3. an applicable exit evaluates the complete call using values at exit;
4. ordinary `Ref<T>`, mutation, move, and overlap rules apply at call time;
5. `defer` runs on every supported successful and error exit;
6. `defer_on_error` runs only while an active exact `Error` result crosses its
   lexical scope;
7. both forms and owned local destruction follow one reverse-registration
   cleanup order;
8. conditional and loop arming is path-correct and iteration-local;
9. results are materialized and structured child tasks are joined before user
   cleanup accesses the exiting scope;
10. runtime traps execute no deferred actions;
11. unused features impose no runtime registration, allocation, or binary
    dependency; and
12. `LANGUAGE.md`, both grammar copies, implementation comments, diagnostics,
    and tests agree.

## 13. Rejected alternatives

### General deferred blocks

Blocks admit return, propagation, declarations, fallible operations, and
arbitrary control flow during an already active exit. One no-result call covers
the intended balancing action without creating a second miniature function
body.

### Arming-time argument capture

Implicit capture either moves owning values too early or stores references
beyond `Ref<T>`'s call boundary. Exit-time evaluation uses existing place,
ownership, and call rules. An explicit local provides snapshot behavior.

### Calls whose errors are discarded

Silently discarding a cleanup error violates the purpose of recoverable error
values. Replacing or combining an already pending error would require a second
error policy. Fallible cleanup remains an explicit ordinary call.

### Function-scoped accumulation

A defer inside a loop would accumulate until function return and retain every
iteration's state. Lexical scope exit is predictable, bounded, and consistent
with deterministic destruction.

### Running during traps

That requires stack unwinding and gives traps observable recovery-like
behavior. Traps terminate without executing language cleanup.

### The name `errdefer`

`defer_on_error` follows the explicit naming of `return_on_error` and states
the condition without an abbreviation.

## 14. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 016: Box Indirection and Recursive Data](archive/016-box-indirection-and-recursive-data.md)
- [Specification 021: Truthiness and Equality](archive/021-truthiness-and-equality.md)
- [Specification 022: Concurrency and Parallelism](022-concurrency-and-parallelism.md)
- [Specification 024: Error Handling](archive/024-error-handling.md)
- [Specification 026: Return Statements](archive/026-return-statements.md)
