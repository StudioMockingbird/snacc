# Specification 024: Error Handling

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Scope and status

This implementation-ready specification defines Snacc's recoverable error
value, the `T | Error` result convention, explicit inspection, and the
`return_on_error` propagation expression.

Errors are ordinary values. Snacc has no exceptions, throwing, catching,
stack unwinding, implicit propagation, or built-in `Result<T, E>` type.
Runtime traps remain distinct from recoverable errors.

`LANGUAGE.md` remains authoritative until this specification is accepted,
implemented, and incorporated there. This specification has no open semantic
design questions of its own. Section 9 fixes the result-slot convention used
by compiler-private runtime facilities and by the I/O specification. Section
12 gives the required implementation order.

## 2. Dependencies

This specification depends on:

- [RFC 017](017-utf8-strings-and-views.md) for immutable owned `String`
  values;
- [Specification 018](archive/018-inline-sum-types.md) for `T | Error`, injection,
  decomposition, and representation; and
- [Specification 026](archive/026-return-statements.md) for early return, result
  materialization, reachability, and cleanup-bearing return edges.

[Specification 025](025-defer.md) extends the cleanup performed by an error
return but is not required to implement `Error` or `return_on_error`.

## 3. The predeclared `Error` type

The compiler predeclares one nominal struct equivalent to:

~~~snacc
type Error is struct
    category: String,
    header: String,
    message: String,
end
~~~

`Error` is not a union and has no hidden alternatives. Its three fields are
ordinary immutable struct fields and are always initialized. Programs cannot
redeclare `Error`.

An error is constructed with the ordinary named-field struct constructor:

~~~snacc
let error: Error = Error(
    category: "validation.invalid_port",
    header: "Invalid port",
    message: "Port must be between 1 and 65535"
)
~~~

User code and runtime-provided facilities may construct `Error` values. There
is no privileged constructor and no compiler-generated source location.

Because each field is an owned `String`, `Error` follows `String`'s ownership
and move rules. Propagation transfers the complete value; it does not copy or
reconstruct any field.

### 3.1 Field meanings

- `category` is the stable, machine-readable classification used by program
  logic. Standard facilities define their exact category strings. A category
  is case-sensitive. Standard categories use lowercase ASCII segments joined
  by `.`; for example, `io.not_found`.
- `header` is a short human-readable summary suitable for display.
- `message` is a detailed human-readable explanation containing relevant
  context.

Only `category` is a stable programmatic contract. Standard-library and
runtime producers may vary `header` and `message` text by platform or improve
their wording without changing the category. Programs shall not select logic
by comparing `header` or `message`.

The type does not contain a platform error number, source path, line, column,
cause, backtrace, or chained error. A platform-specific number may be included
in `message`; it has no portable semantic meaning.

## 4. Fallible result types

A callable with one or more recoverable failure reasons returns an inline sum
whose direct members include exact `Error`:

~~~snacc
fun load_config(path: String): Config | Error do
    // ...
end
~~~

An operation with no success value returns `Nil | Error`:

~~~snacc
fun save(config: Config): Nil | Error do
    // ...
end
~~~

`T | Nil` remains the convention for absence or a single reason for which no
further information is useful:

~~~snacc
fun find_user(id: UserId): User | Nil do
    // ...
end
~~~

`return_on_error` does not treat `Nil` as an error and does not propagate it.
A program handles `Nil` through ordinary truthiness, equality, or sum
decomposition.

An inline sum may have multiple success members:

~~~snacc
fun read_value(): Int64 | String | Error do
    // ...
end
~~~

The presence of an `Error` member does not change the ordinary representation,
truthiness, equality, injection, or decomposition rules of an inline sum.

## 5. Explicit handling

No construct forces propagation. A caller may inspect and handle an error with
the existing inline-sum type test:

~~~snacc
let opened: File | Error = File.open("settings.snacc")

if opened is File(file) then
    print(file.size())
elseif opened is Error(error) then
    print(error.header)
    print(error.message)
end
~~~

Testing the fields of an extracted `Error` is ordinary field access and string
equality:

~~~snacc
if error.category == "io.not_found" then
    create_default_file()
else
    report(error)
end
~~~

Under Specification 021, `File | Error` is truthy for both active members.
`if opened then` therefore does not test whether the operation succeeded.
Programs use `is` or `return_on_error` when success and error must be
distinguished.

## 6. `return_on_error`

`return_on_error` is a reserved keyword introducing a prefix expression:

~~~ebnf
return-on-error-expression = "return_on_error", postfix ;
~~~

A parenthesized expression is a `postfix` atom, so parentheses may delimit a
larger operand. Without parentheses the keyword applies to the next `postfix`
expression, including its complete field and call chain.

~~~snacc
let file: File = return_on_error File.open(path)
let text: String = return_on_error file.read_text()
~~~

### 6.1 Static requirements

For an expression-form `return_on_error E`:

1. `E` shall have an inline-sum type with exact `Error` as one direct member.
2. Removing `Error` shall leave at least one non-`Nil` member.
3. The enclosing function or method shall declare a result type that directly
   contains exact `Error`.
4. The remaining success type shall be valid in the expression's context.

If one success member remains, the expression has that member's type. If
multiple success members remain, it has their normalized inline-sum type.

~~~snacc
fun normalize(): Int64 | String | Error do
    let value: Int64 | String = return_on_error read_value()
    value
end
~~~

The construct is invalid at top level, in a callable without `Error` in its
declared result, or on a value whose type does not directly contain `Error`.
A named type containing an `Error` field is not a fallible result and does not
satisfy the rule.

### 6.2 Statement form

When the operand has type `Nil | Error`, `return_on_error` is a statement and
produces no value:

~~~snacc
fun flush(output: Ref<Output>): Nil | Error do
    return_on_error output.flush()
    return nil
end
~~~

The statement form is valid only when the operand type is exactly
`Nil | Error`. It propagates `Error` and consumes active `Nil` without
producing a standalone value. It cannot appear where a value is required and
cannot discard a non-`Nil` success member.

~~~snacc
return_on_error flush()               // valid statement
return_on_error read_value()          // error: non-Nil success is discarded
let value: Nil = return_on_error flush() // error: success produces no value
~~~

### 6.3 Dynamic behavior

The operand evaluates exactly once.

- If its active member is not `Error`, `return_on_error` extracts that member
  without re-evaluating the operand. Expression form yields it, injecting it
  into the reduced success sum when multiple members remain. Statement form
  consumes active `Nil`.
- If its active member is `Error`, the complete `Error` value and its ownership
  obligation move into the enclosing callable's result. The callable returns
  through the same checked exit path as an explicit `return error`.

The propagated value is unchanged. `return_on_error` does not replace its
category, add context, allocate a new message, or capture a source location.

Because `Error` owns strings, every sum containing it is move-only.
`return_on_error` consumes its complete operand value and then transfers the
active payload, disarming the consumed sum. This is whole-sum elimination, not
a general move out of a union payload alias. The operand shall therefore be an
owned temporary or an available owning root that ordinary move rules permit it
to consume. A borrowed place, branch payload alias, or prohibited subplace is
rejected. The consumed root is unavailable afterwards on the success path.

The result is materialized before scope cleanup exactly as required by
Specification 026. `defer_on_error` and `defer` actions from Specification 025
then run as applicable while scopes are exited.

## 7. Errors versus runtime traps

An `Error` represents an expected condition a correct program may encounter
and handle, such as a missing file or refused connection. A runtime trap
represents an operation that cannot produce a Snacc value under its contract,
such as out-of-bounds access, allocation failure, invalid floating-point
production, or another defined fatal failure.

`return_on_error` handles only an active exact `Error` member. It does not
catch, convert, or recover from a trap. Traps do not become `Error` values
implicitly.

## 8. Interaction with return and control flow

Error propagation is a terminating edge in the current callable. Source after
an unconditional `return_on_error` is not generally unreachable because the
operand may succeed. Within the error branch generated by the checker, the
edge is terminating.

An explicit return may return an `Error` without using `return_on_error`:

~~~snacc
fun positive(value: Int64): Int64 | Error do
    if value <= 0 then
        return Error(
            category: "validation.not_positive",
            header: "Expected a positive value",
            message: "The supplied value must be greater than zero"
        )
    end

    value
end
~~~

Returning a value whose static type is a sum containing `Error` selects the
actual exit mode from its active member at runtime. An active `Error` is an
error exit; every other active member, including `Nil`, is a successful exit.
This classification controls `defer_on_error`.

Merely constructing, storing, passing, printing, or inspecting an `Error` does
not initiate propagation.

## 9. Rust bridge and ABI

### 9.1 Compiler-private fallible-result slots

Compiler-private runtime calls that produce an inline-sum result use a
caller-provided result slot. The caller reserves compiler-sized and
compiler-aligned storage, passes its address as the final hidden argument, and
the runtime writes the inline-sum tag plus the active payload into that slot.
The caller owns the initialized payload and destroys it exactly once after the
call. No inline-sum value or payload copy crosses the runtime-import boundary.
For `Nil | Error`, the `Nil` tag leaves no payload initialized; the `Error` tag
initializes the complete `Error` payload. Exact size, alignment, tag encoding,
and payload offsets come from the shared inline-sum ABI implementation.

Specification 023's fallible I/O operations use this same convention; it does
not define a second result ABI.

The first version rejects `Error` and any inline sum containing it in user
`extern rust` parameters and results. `Error` owns three Snacc strings and the
inline-sum representation is a private Snacc ABI; neither is a Rust or C ABI
type.

Runtime-provided standard facilities may construct and return `Error` through
compiler-private runtime entry points. Their ABI uses the same checked
representation as internal Snacc calls. If those entry points change the
physical compiler/runtime ABI, implementation assigns the explicit ABI
successor under the shared ABI policy; source-only checker changes do not bump
the ABI.

## 10. Required diagnostics

The frontend diagnoses at least:

- a user declaration that collides with predeclared `Error`;
- missing, duplicate, unknown, positionally supplied, or mistyped `Error`
  constructor fields under ordinary struct-construction rules;
- `return_on_error` outside a function or method;
- an operand that is not an inline sum directly containing exact `Error`;
- an enclosing callable whose declared result does not directly contain
  exact `Error`;
- expression-form use when removal of `Error` leaves only `Nil`;
- statement-form use when any non-`Nil` success member remains;
- a remaining success type incompatible with its context;
- use of statement-form `return_on_error` where a value is required;
- any move, borrow, or cleanup conflict caused by propagating a move-only
  success or error value;
- an operand that is a borrowed place, payload alias, or prohibited subplace
  rather than a consumable owning value; and
- `Error` in a user Rust bridge signature.

These are checker errors before LLVM lowering.

## 11. Examples

### 11.1 Propagating several operations

~~~snacc
fun load_config(path: String): Config | Error do
    let file: File = return_on_error File.open(path)
    let text: String = return_on_error file.read_text()
    parse_config(text)
end
~~~

### 11.2 Handling one category

~~~snacc
fun load_or_default(path: String): Config | Error do
    let opened: File | Error = File.open(path)

    if opened is Error(error) then
        if error.category == "io.not_found" then
            return default_config()
        end
    end

    let file: File = return_on_error opened
    let text: String = return_on_error file.read_text()
    parse_config(text)
end
~~~

The type-test binding permits inspection but does not move the `Error` payload.
After the handled category returns, `return_on_error opened` consumes the whole
owning sum and propagates any remaining error. General consuming union
decomposition remains outside this specification.

### 11.3 No-value success

~~~snacc
fun write_message(output: Ref<Output>, text: String): Nil | Error do
    return_on_error output.write(text)
    return_on_error output.flush()
    return nil
end
~~~

## 12. Detailed implementation plan

### Phase 1: syntax and predeclared type

1. Reserve `return_on_error` as a keyword and add its token without changing
   identifier treatment for other underscore-containing names.
2. Add the grammar production in section 6 at postfix-prefix precedence and
   add a distinct parsed expression node carrying the keyword and operand
   spans.
3. Predeclare nominal `Error` as the exact three-field struct in section 3.
   Reuse ordinary struct construction, field access, type identity, ownership,
   and diagnostics; do not add an error-specific constructor path.
4. Reject user declarations named `Error` through the existing predeclared-name
   collision check.

### Phase 2: resolution and checking

1. Add resolved and checked `return_on_error` nodes rather than desugaring into
   source-level `if`; preserve one operand and its evaluation span.
2. Require a direct exact `Error` member in both operand and enclosing result.
3. Compute the normalized remaining success type after removing `Error`.
4. Distinguish expression form from statement form using the existing
   value-required and statement block entry points; require exact
   `Nil | Error` in statement form.
5. Reuse inline-sum projection and injection checks for the success and error
   paths.
6. Mark the generated error path as terminating and attach the same lexical
   scope-exit obligations as an explicit return.
7. Model the operation as consumption of the whole sum followed by transfer of
   its active payload. Disarm the source obligation without enabling general
   subplace moves.
8. Apply ownership analysis to both projections. Transfer the error obligation
   on propagation and consume `Nil` in statement form.

### Phase 3: checked exits and defer integration

1. Extend the checked return-exit classification with `Success` and `Error`.
2. Classify explicit and implicit returns of an inline sum containing `Error`
   from the active runtime tag; classify exact non-sum returns statically.
3. Make `return_on_error` construct a checked `Error` exit with the projected
   value already materialized.
4. Feed that exit classification into Specification 025's cleanup plan when
   `defer_on_error` is implemented; keep the representation usable before then.
5. Preserve structured-concurrency joins and other existing exit obligations.

### Phase 4: LLVM and runtime ABI

1. Evaluate the operand once into checked temporary storage.
2. Branch on the existing inline-sum tag without rebuilding type information
   in the backend.
3. On success, project or transfer the active member and continue.
4. On error, transfer the exact `Error` payload into the callable result,
   execute the checked exit plan, and emit the return terminator.
5. Add compiler-private runtime construction entry points needed by standard
   facilities. If their physical signatures or representations change the
   bridge, assign the explicit ABI successor under the shared ABI policy.
6. Verify every generated module and treat a missing tag, member projection,
   ownership obligation, or exit classification as an internal error.

### Phase 5: tests and contract synchronization

1. Add lexer and parser tests for keyword recognition, postfix binding,
   parentheses, expression form, and statement form.
2. Add checker tests for every diagnostic in section 10.
3. Add conformance cases for manual handling, one and multiple success members,
   `Nil | Error`, nested scopes, methods, explicit error returns, and unchanged
   propagation of all three strings.
4. Add instrumented ownership tests proving each `String` field and each
   move-only success payload is destroyed exactly once on success, handling,
   explicit return, and propagation.
5. Test interaction with `defer`, `defer_on_error`, return, and structured task
   joins when their specifications are implemented.
6. Update the formal grammar first in `LANGUAGE.md`, copy it identically to
   `GRAMMAR.ebnf`, and update the terse normative error, expression, ownership,
   truthiness, and return text in the same implementation change.
7. Update runtime and compiler comments only where they carry a non-obvious
   contract, then run formatting, workspace checks, and all tests.

## 13. Acceptance criteria

Implementation is complete only when:

1. `Error` is one predeclared nominal struct with exactly three immutable
   `String` fields;
2. user code can construct, return, inspect, and compare those fields through
   ordinary language operations;
3. `T | Error` and `Nil | Error` use ordinary inline-sum representation;
4. `return_on_error` evaluates its operand once and propagates only an active
   exact `Error` member;
5. propagation transfers the original value without changing or copying it;
6. propagation and success extraction consume the whole owning sum without
   enabling moves from borrowed payload aliases or arbitrary subplaces;
7. one or multiple success members continue with the correct normalized type;
8. `Nil | Error` works in statement form and cannot produce a standalone `Nil`
   expression;
9. explicit and propagated errors execute the same checked return and cleanup
   path;
10. runtime traps remain outside recoverable error handling;
11. `Error` cannot cross a user Rust bridge;
12. every diagnostic in section 10 has a focused negative test; and
13. `LANGUAGE.md`, both grammar copies, implementation comments, diagnostics,
    and tests agree.

## 14. Rejected alternatives

### `Error` as a union

A union freezes a global member list because adding a member breaks exhaustive
programs. String categories keep the data shape fixed while allowing standard
facilities and libraries to introduce categories without changing the type.

### Exceptions and stack unwinding

They introduce invisible control flow, a second failure channel, and unwinding
requirements for every frame. Errors remain visible in callable result types.

### `Result<T, E>`

It requires generic machinery and would duplicate the direct sum notation
Snacc already supports. `T | Error` is the sole multi-reason recoverable-error
convention.

### Propagating `Nil`

`Nil` represents absence without an error value. Treating it as an error would
erase the semantic distinction and make `return_on_error`'s name inaccurate.

### The keyword `try`

`try` does not state whether it catches, converts, retries, or returns. The
longer `return_on_error` spelling makes its control-flow effect explicit.

### Automatic source locations

File paths and positions enlarge every error, can disclose build-machine
details, and do not constitute a useful call stack. Programs may place source
information in `message` when it is relevant.

## 15. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 017: UTF-8 Strings and Views](017-utf8-strings-and-views.md)
- [Specification 018: Inline Sum Types](archive/018-inline-sum-types.md)
- [Specification 021: Truthiness and Equality](archive/021-truthiness-and-equality.md)
- [Specification 023: Input and Output](023-input-and-output.md)
- [Specification 025: Defer](025-defer.md)
- [Specification 026: Return Statements](archive/026-return-statements.md)
