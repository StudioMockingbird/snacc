# Specification 026: Return Statements

Status: Proposed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Proposal state

This implementation-ready specification adds `return` statements to functions
and methods. A return may exit a callable early, with a value when that callable
declares a result and without a value when it produces no result.

`LANGUAGE.md` remains authoritative until this specification is accepted,
implemented, and incorporated there.

This specification contains no open design questions. Section 11 fixes the
implementation order and phase boundaries.

## 2. Summary

A result-declaring function or method may return a value explicitly:

~~~snacc
fun absolute(value: Int64): Int64 do
    if value < 0 then
        return 0 - value
    end
    value
end
~~~

A no-result function or method may return without a value:

~~~snacc
fun print_if_positive(value: Int64) do
    if value <= 0 then
        return
    end
    print(value)
end
~~~

The existing final-expression form remains valid and is still the shortest way
to produce a result on ordinary fallthrough:

~~~snacc
fun add(left: Int64, right: Int64): Int64 do
    left + right
end
~~~

`return` is a statement. It is not an expression, has no type, and cannot be
stored, passed, compared, printed, or nested inside another expression.

## 3. Dependencies and scope

This specification builds on the implemented block, function, method,
no-result, assignment, and control-flow contract in
[`LANGUAGE.md`](../../LANGUAGE.md).

Complete ownership-aware implementation depends on
[RFC 016](archive/016-box-indirection-and-recursive-data.md), whose checked cleanup
plans run on every scope exit. Inline-sum return examples use
[Specification 018](archive/018-inline-sum-types.md), but the return statement itself
does not depend on sums.

Specification 023 identifies missing early return as its largest error-handling
usability gap. This specification closes the fundamental syntax and
control-flow gap. Specification 024 separately builds `return_on_error` on the
return path defined here and owns the `Error` type.

This specification does not add multiple return values, named results,
`yield`, exceptions, tail-call guarantees, labelled
returns, function values, closures, or a writable `Void`, `Unit`, or `Never`
type.

## 4. Grammar and token boundaries

`return` becomes a reserved keyword. The block grammar gains one statement:

~~~ebnf
block-element         = variable-declaration
                      | assignment
                      | while-statement
                      | for-statement
                      | break-statement
                      | return-statement
                      | if-form
                      | expression ;

return-statement      = "return", [ expression ] ;
~~~

`for-statement` denotes the production added by Specification 019 when that
specification is present. Its appearance here does not make this specification
depend on collections.

Snacc has neither semicolons nor significant newlines. The optional expression
therefore follows this deterministic rule:

1. `return` is bare only when the next token closes the current block: `end`,
   `elseif`, or `else`. At the top-level program boundary, end-of-input is
   also a parser boundary so a top-level bare `return` reaches the checker and
   receives the ordinary outside-callable diagnostic.
2. Otherwise, the following tokens must begin one expression, which is consumed
   under the ordinary maximal-expression grammar.
3. A bare return cannot be followed by another element in the same block. Such
   an element would be unreachable in any case.

Consequently, these forms are unambiguous:

~~~snacc
fun stop() do
    return
end

fun choose(flag: Bool): Int64 do
    if flag then
        return 1
    else
        return 2
    end
end
~~~

In a no-result callable, `return print(1)` is parsed as a return with the
value-producing expression `print(1)` and is rejected. It is never interpreted
as a bare return followed by another statement.

## 5. Valid contexts and return forms

A return statement is valid only lexically inside a function or method body,
including a nested `if`, `while`, `for`, or other block belonging to that same
callable. It exits the nearest enclosing function or method, not merely the
nearest block or loop.

At top level, `return` and `return expression` are errors. An `extern rust`
declaration has no Snacc body and therefore contains no return statement.

The enclosing callable's declared result—not the kind of the immediately
enclosing block—selects the permitted form:

| Enclosing callable | Permitted form | Rejected form |
| --- | --- | --- |
| declares `: T` | `return expression` assignable to `T` | bare `return` |
| omits a result | bare `return` | `return expression` |

This rule permits a value return inside a no-result loop body when the loop is
inside a result-declaring function:

~~~snacc
fun first_positive(values: View<Int64>): Int64 | Nil do
    for value in values do
        if value > 0 then
            return value
        end
    end
    nil
end
~~~

The returned expression is checked against the callable's declared result using
the same assignability rules as an implicit final expression. Ordinary numeric
widening and contextual injection into a named union or inline sum therefore
apply when their owning specifications permit them. Return adds no conversion
of its own.

~~~snacc
fun find_byte(found: Bool): Byte | Nil do
    if found then
        return 1u8
    end
    nil
end
~~~

## 6. Fallthrough and control-flow checking

Executing `return` immediately terminates the current callable path. No later
block element on that path executes.

A result-declaring function or method is valid when every reachable path does
one of the following:

1. executes `return expression` with a value assignable to the declared result;
   or
2. falls through the body with a final expression assignable to the declared
   result under the existing value-required-block rule.

A no-result function or method may execute a bare return or fall through its
body normally.

~~~snacc
fun bounded(value: Int64): Int64 do
    if value < 0 then
        return 0
    elseif value > 100 then
        return 100
    end
    value
end
~~~

A path terminated by `return` does not need to supply a block value and does not
participate in a value-form `if` branch's common-result type. Other reachable
branches must still supply the value required by their context:

~~~snacc
fun normalize(value: Int64): Int64 do
    if value < 0 then
        return 0
    else
        value
    end
end
~~~

If every reachable branch of an `if` returns, the `if` itself terminates that
callable path and supplies no value:

~~~snacc
fun bit(flag: Bool): Int64 do
    if flag then
        return 1
    else
        return 0
    end
end
~~~

A result-declaring body remains invalid when any path reaches its end without
returning or producing its required final expression:

~~~snacc
fun incomplete(flag: Bool): Int64 do
    if flag then
        return 1
    end
    // error: false reaches the function end without an Int64
end
~~~

The checker does not treat `while true`, recursion, a truthy constant, or any
other computation as proof of nontermination. Only explicit terminating
statements and the existing exhaustive conditional rules affect required-value
reachability.

`return` inside a loop exits the callable. `break` continues to exit only the
nearest loop. Neither statement accepts the other's role or operand form.

## 7. Unreachable source

A block element following an unconditional return in the same block is an
unreachable-source error. The same rule applies after an `if` for which every
reachable branch returns.

The checker reports the first unreachable element and points to the return or
fully terminating conditional that prevents it from executing. It need not
perform general constant-condition analysis. A conditional return does not make
following source unreachable when another branch can fall through.

~~~snacc
fun invalid(): Int64 do
    return 1
    print(2) // error: unreachable
end
~~~

## 8. Evaluation, ownership, and cleanup

For `return expression`, the expression evaluates exactly once before any
enclosing scope is exited. Its result is converted or injected as required and
materialized as the callable result before cleanup can destroy its sources.

A copyable value is copied normally. Returning a move-only owning value
transfers the complete value and its cleanup obligation to the caller. The
source is marked moved and is excluded from the callee's cleanup. Return does
not permit a move that ordinary ownership rules reject, including a move out of
a field or borrowed place.

After the result is safely materialized, return exits every intervening lexical
scope from innermost to outermost. Each scope executes its checked cleanup plan.
Under RFC 016 this destroys live locals in reverse successful-initialization
order, skips moved values, and destroys active aggregate contents exactly once.
Temporaries follow their established full-expression rules.

A bare return performs the same scope exits without materializing a result.
Writes already made through `Ref<T>` remain visible to the caller; return does
not roll them back and does not add output-only reference semantics.

Specification 025 registers `defer` and `defer_on_error` calls in the same
checked scope-exit plan as local destruction. Return cannot bypass them. The
result is materialized first; each exited scope then follows Specification
025's reverse-registration order and its successful or error exit class.

## 9. Structured concurrency

Returning from a function called by a spawned task exits only that called
function. It does not return from the spawning function or cancel sibling
tasks.

If Specification 022's `parallel` block is present, a return written lexically
inside that block is a structured-scope exit. It is valid only once the
implementation can guarantee that every task spawned in the exited parallel
scope is joined before the callable actually returns. Until that cleanup edge
exists, the checker rejects a return that would cross a live parallel scope.

The return expression is still evaluated at the return statement before scope
exit. Specification 022's live-task borrow rules therefore reject reading or
moving a place still borrowed by a spawned task. Joining is not moved earlier
merely to make such an expression legal.

Nested parallel scopes are joined from innermost to outermost along the return
edge. Return performs no cancellation and cannot allow a task to outlive its
structured scope.

## 10. Checked representation, LLVM, and ABI

The checked program represents return as a statement containing:

- the enclosing callable identity and declared result, if any;
- an optional checked result expression already assignable to that result;
- a terminating control-flow fact; and
- the ordered cleanup and structured-scope exits required before the callable
  exit.

No `Nil`, dummy expression, sentinel type, or fallback value represents a bare
return. Checked block flow distinguishes reachable fallthrough, fallthrough
with a value, loop-local termination, and callable return explicitly.

LLVM lowering evaluates and materializes an explicit result before emitting its
checked exit plan, then emits the appropriate `ret value` or `ret void`.
Implicit final-expression returns use the same result and exit path. A backend
may share one epilogue or emit equivalent exits directly, but it must not add a
fallthrough branch or second terminator after `ret`.

Return changes no function signature, value layout, calling convention, bridge
type, runtime symbol, or physical ABI. It therefore requires no ABI-version
advance by itself. A separate ownership, defer, or concurrency specification
may advance the ABI for its own runtime changes.

## 11. Detailed implementation plan

### Phase 1: syntax

1. Reserve `return` and add a spanned syntax statement with an optional
   expression.
2. Parse a bare return only immediately before `end`, `elseif`, or `else`; parse
   one maximal ordinary expression otherwise.
3. Add parser tests for function and method bodies, every nested block, bare and
   valued forms, whitespace variation, block boundaries, and ambiguous-looking
   no-result examples.
4. Reject malformed return tokens through the normal parser recovery path
   without inventing a placeholder value.

### Phase 2: checking and flow

1. Track the current callable identity and optional declared result while
   checking its complete body.
2. Check the return form against that signature and check a return expression
   with the exact expected result type, including ordinary conversion and sum
   injection.
3. Replace binary value/no-result block checking with an explicit flow outcome
   that distinguishes reachable fallthrough, value fallthrough, loop-local
   termination, and callable return.
4. Exclude returned branches from common-result computation while requiring
   every remaining reachable branch to satisfy its value context.
5. Reject the first reachable block element after an unconditional callable
   return or a conditional whose every reachable branch returns.
6. Add checker tests for all result/no-result combinations, top-level returns,
   methods, loops, nested conditionals, exhaustive tests, missing fallthrough
   values, and unreachable source.

### Phase 3: ownership and scope exits

1. Evaluate and type-adjust the result before computing exit cleanup.
2. Transfer move-only return roots to the caller, disarm their source cleanup,
   and reject partial, borrowed, overlapping, or already-moved sources.
3. Attach every intervening scope's existing cleanup plan to the checked return
   in innermost-to-outermost order.
4. Preserve completed `Ref<T>` writes and reject no additional alias pattern
   beyond the ordinary expression and ownership rules.
5. Integrate parallel-scope joins when Specification 022 supplies their cleanup
   edge; otherwise reject returns that cross a live parallel scope.
6. Add instrumented tests proving exactly-once destruction, correct local order,
   moved-result survival, temporary cleanup, nested-scope cleanup, and task
   joining.

### Phase 4: lowering

1. Lower explicit and implicit results through one shared result-materialization
   path.
2. Emit each checked cleanup and structured-scope exit before `ret`, without
   reconstructing ownership or reachability in the backend.
3. Emit `ret value` for result callables and `ret void` for no-result callables;
   never emit a second terminator or reachable instruction afterwards.
4. Verify every generated LLVM module and treat a mismatched return kind,
   absent checked result, invalid cleanup obligation, or reachable fallthrough
   as an internal compiler error.

### Phase 5: contract and conformance

1. Add the unchanged grammar context and new return production to the formal
   EBNF first in `LANGUAGE.md`, then copy it identically to `GRAMMAR.ebnf`.
2. Update the terse normative callable, block, reachability, ownership, cleanup,
   and control-flow text in `LANGUAGE.md`; remove the statement that every
   value-required path must end in an expression.
3. Update Specification 023's early-return gap and deferred-work entry to point
   to this completed design. Update Specifications 022 and 025 when their
   substantive contracts are finalized so their exit paths agree.
4. Add positive and negative conformance programs for every rule and diagnostic
   in this specification, including returned inline sums and move-only values
   as their owning specifications land.
5. Run formatting, workspace checking, and the complete workspace test suite.

## 12. Required diagnostics

The implementation diagnoses at least:

- a return at top level or otherwise outside a function or method;
- a bare return from a result-declaring callable;
- a value return from a no-result callable;
- a returned expression not assignable to the declared result;
- a result-declaring path that reaches the callable end without returning or
  producing its required final expression;
- source following an unconditional return or fully returning conditional;
- a bare return not immediately followed by its current block boundary;
- a move-only return that violates an ownership, move, or borrow rule; and
- a return that crosses a live structured-concurrency scope before its required
  join cleanup is available.

Diagnostics identify the return statement, the enclosing callable signature,
and both expected and actual result types when a value is incompatible.

## 13. Rejected alternatives

### Require `return` for every result

Final expressions are already concise, implemented, and composable with
value-form conditionals. Requiring `return` on every path would add ceremony.
Explicit return exists for early exits and for cases where it improves clarity.

### Make `return` an expression

Return transfers control out of the current callable and cannot produce a value
for its surrounding expression. Treating it as an expression would require a
bottom or `Never` type and complicate every expression-combination rule without
making another program possible.

### Permit bare return from a result callable

Snacc has no default values and no uninitialized results. Every successful
result path supplies exactly one value of the declared type.

### Permit a value on return from a no-result callable

Silently discarding that value would hide mistakes and create a second spelling
for a discarded expression. A no-result callable uses bare return.

### Use newline or semicolon to distinguish bare return

Snacc is whitespace-insensitive and has no semicolon token. Requiring either
would violate the uniform block grammar. Restricting bare return to a block
boundary is unambiguous and loses no useful reachable program.

### Add automatic error propagation

Early return is a fundamental control-flow operation useful outside error
handling. Specification 024 therefore specifies `return_on_error` separately,
including its exact `Error` member, success type, and cleanup behavior, without
changing the general `return` statement defined here.

## 14. Acceptance criteria

Implementation is complete only when:

1. `return` is reserved and parsed as one statement with an optional value;
2. bare return is unambiguous without semicolons or significant newlines;
3. result callables accept only value returns assignable to their declared type,
   while no-result callables accept only bare returns;
4. return is rejected outside functions and methods and exits the nearest
   enclosing callable from every supported nested block;
5. implicit final expressions remain valid and every reachable result path
   either returns explicitly or falls through with its required value;
6. returned branches do not participate in a value-form conditional's common
   result type, while all remaining reachable branches do;
7. unreachable source following a definite return is rejected;
8. result expressions evaluate exactly once before scope exit;
9. move-only results transfer ownership to the caller and every other live
   owned value is destroyed exactly once along the return edge;
10. existing `Ref<T>` writes remain visible and no reference can be returned;
11. a return cannot bypass a live structured-concurrency join or any completed
    scope-exit cleanup facility;
12. lowering consumes checked result, flow, and cleanup facts and emits no
    instruction after a terminating return;
13. return introduces no standalone no-result type or ABI change;
14. `LANGUAGE.md`, both grammar copies, implementation comments, diagnostics,
    active dependent specifications, and tests agree; and
15. formatting, workspace checks, and all conformance tests pass.

## 15. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 016: Box Indirection and Recursive Data Structures](archive/016-box-indirection-and-recursive-data.md)
- [Specification 018: Inline Sum Types](archive/018-inline-sum-types.md)
- [Specification 019: Collections and Iteration](019-collections-and-iteration.md)
- [Specification 022: Concurrency and Parallelism](022-concurrency-and-parallelism.md)
- [Specification 023: Input and Output](023-input-and-output.md)
- [Specification 024: Error Handling](024-error-handling.md)
- [Specification 025: Defer](025-defer.md)
