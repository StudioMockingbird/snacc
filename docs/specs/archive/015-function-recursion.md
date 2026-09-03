# RFC 015: Function and Method Recursion

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## Proposal state

This implementation-ready specification makes the existing function and method
recursion behavior an explicit, tested language contract. It introduces no
syntax or compiler mechanism and does not change recursive data layouts.

`LANGUAGE.md` remains authoritative until this proposal is accepted and its
decisions are incorporated there.

This specification contains no open design questions. The implementation plan
below is limited to verifying the existing mechanism, updating the normative
contract, and adding focused conformance coverage.

## Summary

Snacc functions and methods may call themselves or other callable declarations
recursively. Direct recursion, mutual recursion, and forward calls are valid
when the ordinary call and result-type rules are satisfied.

The compiler does not prove termination, impose a source-level recursion-depth
limit, or promise tail-call optimization. An unbounded computation may exhaust
the native call stack at runtime.

## Motivation

Recursion is a natural way to express factorials, tree algorithms, recursive
descent helpers, and mutually recursive state machines. Snacc already has the
declaration collection and LLVM declaration order needed to support it:

- callable signatures are known before function bodies are checked;
- declarations are visible independently of source order;
- LLVM functions are declared before their bodies are lowered.

This RFC records that behavior explicitly and adds focused conformance
coverage rather than adding a separate recursion mechanism.

## Proposed semantics

### Direct recursion

A function may call itself by its declared name:

~~~snacc
fun factorial(n: Int64): Int64 do
    if n == 0 then
        1
    else
        n * factorial(n - 1)
    end
end
~~~

The recursive call is an ordinary statically resolved call. Its arguments are
evaluated using the normal argument evaluation order, and its result must be
used according to the ordinary result-type rules.

### Mutual recursion

Two or more functions may call one another:

~~~snacc
fun even(n: Int64): Bool do
    if n == 0 then true else odd(n - 1) end
end

fun odd(n: Int64): Bool do
    if n == 0 then false else even(n - 1) end
end
~~~

Mutual recursion does not require a special declaration or forward-reference
syntax. All participating declarations must still have valid signatures and
function bodies.

### Methods

Methods may recursively call themselves or other statically resolved methods.
Existing receiver mutability rules continue to apply to every call. The
receiver-write analysis must remain conservative through recursive and mutual
method-call cycles.

Methods remain top-level declarations. Recursion does not create nested
functions, closures, function values, or dynamic dispatch.

### Result and termination rules

A recursive function with a result must produce a value of its declared result
type on every reachable return path. A function without a result remains a
statement-only callable. Recursion does not create an implicit result or a
special bottom type.

Snacc does not require a syntactic base case or statically prove termination.
For example, this is well-typed even though it does not terminate:

~~~snacc
fun loop() do
    loop()
end
~~~

The runtime behavior of unbounded recursion is platform-dependent native stack
exhaustion or another process-level failure. Tail-call optimization is an
implementation choice and is not observable language behavior.

Generic recursion and specialization termination belong exclusively to the
generic-programming specification. This specification neither permits nor
rejects syntax that is not yet part of the language.

## Detailed implementation plan

No new runtime mechanism is required.

### Phase 1: verify and preserve frontend behavior

1. Keep declaration collection ahead of body checking so direct, mutual, and
   forward calls resolve through the ordinary callable table.
2. Confirm that recursive calls use the same argument, result, no-result, and
   diagnostic paths as nonrecursive calls; add no recursion-specific syntax or
   checked node.
3. Exercise the receiver-write call graph with self-recursive and mutually
   recursive method cycles. Preserve the conservative fixed-point result at
   every call site.
4. Treat a failure of any existing behavior as an implementation defect fixed
   in its owning phase rather than adding a recursion fallback.

### Phase 2: verify lowering

1. Keep declaration of every concrete LLVM function ahead of lowering any
   function body.
2. Lower recursive edges as ordinary statically resolved native calls.
3. Verify direct, mutual, forward, no-result, result-producing, and
   receiver-writing recursion in debug and release builds.
4. Add no heap continuation, recursion counter, stack probe beyond the target's
   ordinary policy, or mandatory tail-call marker.

### Phase 3: contract and conformance

1. Add the direct recursion, mutual recursion, termination, stack, and
   tail-call rules to `LANGUAGE.md`. No grammar change is required.
2. Add positive conformance cases for every acceptance criterion and negative
   cases proving ordinary call diagnostics remain active inside recursive
   bodies.
3. Add a recursive method test whose receiver-write effect crosses a call-graph
   cycle and is enforced at the original call site.
4. Run formatting, workspace checking, and the complete workspace test suite.

The compiler must not add a recursion-specific fallback expression, fake result,
or backend-only interpretation of an invalid recursive call.

## Diagnostics and failures

Recursion itself is not an error. Existing diagnostics apply when a recursive
call has:

- an unknown callable;
- the wrong number of arguments;
- an incompatible argument type;
- a no-result call in a value position;
- an invalid receiver or receiver mutability;
- an invalid result path in the containing function.

The compiler is not required to diagnose nontermination or likely stack
overflow during compilation.

## Non-goals

- Compile-time termination proofs.
- A guaranteed tail-call or heap-allocated continuation model.
- Dynamic dispatch or function values.
- Recursive by-value type layouts.
- A runtime recursion API or explicit recursion keyword.
- Changing the native stack or process failure model.
- Generic specialization or generic-recursion rules.
- A source option that configures recursion depth or stack size.
- A standardized recursion-specific stack-trace or debug-name format.

## Rejected alternatives

### Require a syntactic base case

A syntactic check cannot prove termination and would reject valid recursion in
which termination is established through another function, input invariant, or
state transition. Ordinary typing remains the compile-time contract.

### Add a recursion keyword or forward declaration

Signatures are already collected before bodies, so recursion and forward calls
need no additional syntax. A marker would add ceremony without information.

### Guarantee tail-call elimination

Mandatory tail-call elimination would constrain debugging, calling conventions,
and backend choices. The optimizer may eliminate a tail call whenever doing so
preserves observable behavior, but programs cannot depend on it.

## Acceptance criteria

This RFC is implemented when the language contract and tests explicitly cover:

1. Direct result-producing function recursion.
2. Mutual result-producing function recursion.
3. Direct no-result function recursion.
4. Forward recursive calls independent of source order.
5. Recursive and mutually recursive methods, including receiver-write effects.
6. Ordinary call diagnostics inside recursive bodies.
7. The absence of a termination requirement and tail-call guarantee.

## Deferred work

Future specifications may introduce a configurable stack policy or standardized
debug information if concrete tooling requirements justify them. An optimizer
may perform tail-call elimination whenever it preserves observable behavior;
that permission requires no language extension. Generic recursion remains part
of generic programming rather than deferred work in this specification.

## References

- [`LANGUAGE.md`](../../LANGUAGE.md)
