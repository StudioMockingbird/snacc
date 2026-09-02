# RFC 008: Statements, Functions Without Results, and `break`

Status: Proposed

Document kind: Feature specification (Rust-style RFC)

## Summary

Snacc gains constructs that produce no value. A function or Rust bridge may
omit its result type, `while` is a statement, and `break` exits the innermost
loop without carrying a value.

~~~snacc
fun spin() do
    while ready() do
        if done() then
            break
        end
        step()
    end
end
~~~

No result is not a type. It cannot be written, stored, passed, returned, or
compared, and it is distinct from `Nil`.

This RFC contains no open design questions. Specification 012 supplies the
final semicolon-free block grammar used below.

## Motivation

The current language forces every construct to invent a value. A `while` that
runs zero times returns a type-specific zero, and a function used only for
effects must still declare and return a value. Neither value represents a
program decision.

No-result constructs remove the fallback. Loops execute for control flow,
effectful functions omit a result type, and absence remains a data-model choice
rather than padding for a signature.

## Goals

- Permit Snacc functions and Rust bridges without results.
- Make `while` a statement with no zero-value fallback.
- Add valueless `break` for the innermost `while`.
- Permit ordinary statement-form `if` without `else`.
- Reject no-result constructs wherever a value is required.
- Make no-result control flow explicit in the checked program and LLVM IR.

## Non-goals

- A writable `Unit`, `Void`, or `()` type.
- `return`, `continue`, labelled loops, loop values, `for`, ranges, or
  iterators.
- `break` with a value.
- Changing the meaning of `Nil`; Specification 012 owns its later restriction
  to union membership.
- Declaration and assignment syntax; Specification 012 owns them.

## Terminology

A **value-producing expression** computes a value with a Snacc value type.

A **statement** performs control flow or effects and produces no value.

A **discarded expression** is a value-producing expression used as a block
element whose result is ignored.

A **value-required block** is the body of a value-returning function or method,
or a branch of a value-producing `if`. Its final reachable block element shall
be a value-producing expression.

A **no-result block** is a top-level executable block, loop body, no-result
function or method body, or branch of a statement-form `if`. It has no required
final value.

## Grammar

After Specification 012's statement refactor, the relevant rules are:

~~~ebnf
function-declaration = "fun", identifier, parameters, [ ":", value-type ],
                       "do", block, "end" ;
rust-declaration     = "extern", "rust", string-literal, "fun", identifier,
                       parameters, [ ":", bridge-value-type ] ;

block                = { block-element } ;
block-element        = variable-declaration
                     | assignment
                     | while-statement
                     | break-statement
                     | if-form
                     | expression ;

while-statement      = "while", expression, "do", block, "end" ;
break-statement      = "break" ;
if-form              = "if", condition, "then", block,
                       { "elseif", condition, "then", block },
                       [ "else", block ], "end" ;
~~~

The semicolons terminate EBNF productions; they are not Snacc tokens. Whitespace
has no grammatical meaning beyond token separation.

The parser produces one `if` form. The checker determines from context whether
it is a statement or a value-producing expression.

## Functions and methods without results

A function or method that omits `: T` produces no result:

~~~snacc
fun announce(value: Int64) do
    print(value)
end
~~~

Its body is a no-result block. Value-producing expressions in that block are
permitted and discarded. A call to the function is valid only as a block
element whose result is not consumed.

These uses are invalid:

~~~snacc
print(announce(1))
let value: Int64 = announce(1)
1 + announce(1)
~~~

A declaration with `: T` has a value-required body. Its last reachable block
element shall be an expression assignable to `T`.

Omitting the result type is the only way to declare no result. No source type
spells it.

## Rust bridges without results

An `extern rust` declaration may omit its result type:

~~~snacc
extern rust "snacc_user_log" fun log(value: Int64)
~~~

Its Rust assertion result is `()`, and its C ABI result is `void`. Its call has
the same no-result restriction as an internal no-result function.

Adding this permitted bridge signature changes ABI version 1 to 2. Compiler
metadata, runtime ABI constants, direct builds, generated assertions, object
cache identity, and mismatch tests shall change together. No runtime symbol is
added or removed.

## `while`

`while` is a statement. It evaluates its condition before each iteration,
requires `Bool`, executes its body while true, and then continues after the
loop. Its body is a no-result block.

~~~snacc
while ready() do
    step()
end
~~~

It cannot appear as an initializer, operand, argument, condition, returned
expression, or any other expression position. It produces neither a body value
nor a zero value when no iteration runs.

## `break`

`break` is a statement valid only within the body of a `while`. It immediately
branches to the exit of the innermost enclosing loop. It takes no operand.

~~~snacc
while ready() do
    if done() then
        break
    end
    step()
end
~~~

A nested function or method declaration never inherits a surrounding loop, but
functions and methods are top-level under the language contract. A `break`
outside a loop body is an error.

Because a loop condition is an expression and `break` is a statement, `break`
cannot occur in a condition. Parser recovery shall not weaken this rule.

## Statement-form and value-form `if`

An `if` used as a block element in a no-result context is a statement. Its
branches are no-result blocks, and `else` is optional regardless of condition
kind:

~~~snacc
if ready() then
    step()
end
~~~

An `if` used where a value is required shall produce a value on every possible
path. It therefore requires either:

- an `else` branch; or
- an exhaustive union type-test chain under Specification 010.

Every reachable branch of a value-form `if` shall end in a value-producing
expression with a common assignable type. A no-result call, `while`, `break`,
declaration, or assignment cannot supply that branch value.

## Evaluation and control flow

Block elements execute in source order. A discarded expression still evaluates
completely, including its side effects. Conditions and call arguments retain
their existing left-to-right evaluation rules.

`break` terminates the current loop-body path. The lowering shall not emit a
fallthrough branch after a terminated LLVM block. Other branch paths remain
independent.

This RFC does not require a diagnostic for statically unreachable source after
`break`; if accepted, that source shall not be lowered as reachable code.

## Checked representation

The checked program shall distinguish values from no-result control flow:

- function, method, and bridge signatures store `result: Option<Ty>`, where
  `Ty` remains the checked representation of every value-producing type;
- `while`, `break`, assignment, and declaration are checked statements;
- a no-result call becomes a checked call statement;
- a value-returning call remains a checked expression;
- `if` is represented as a checked statement or checked expression after
  contextual checking; and
- blocks record their ordered checked elements and optional resulting value.

No sentinel `Ty`, `Nil`, dummy value, or fallback expression shall represent no
result.

## LLVM lowering

A no-result function or method lowers to an LLVM `void` function. A no-result
bridge declaration lowers to a C ABI `void` declaration. Calls emit no result
instruction value.

A loop lowers to condition, body, and exit blocks. False leaves the condition
for the exit; normal body completion branches back to the condition; `break`
branches to the exit. There is no loop-carried result phi.

Statement-form `if` lowers branch control flow and merges only control. A
value-form `if` lowers one result phi when its value is not already materialized
through an addressable place.

The existing loop `default_value` path and its only helper shall be removed.

## Diagnostics

| Condition | Required information |
| --- | --- |
| No-result call used as a value | The called declaration and missing result |
| `while` used as an expression | That `while` is a statement |
| `break` outside a loop body | That only a `while` body establishes a target |
| Value-required block ending in a statement | The required result type and final construct |
| Value-form `if` missing a path | The missing `else` or unhandled union members |
| Branch that produces no value | The affected branch and required common value |
| Non-`Bool` condition | The expected and actual types |

Syntax-shape errors belong to parsing. Result-context, branch, and condition
errors belong to checking. A no-result node reaching value lowering is an
internal compiler error.

## Compatibility and migration

This RFC is source-breaking:

- `while` no longer produces a value;
- a zero-iteration loop no longer creates a zero value; and
- `break` becomes reserved.

Existing result-declaring functions remain valid. Existing effect-only
functions may remove their artificial result type and final value.

The current corpus sites that print or return a loop value shall instead use a
statement loop followed by an explicit value:

~~~snacc
fun zero_after_loop(value: Int64): Int64 do
    while false do
        print(value)
    end
    0
end
~~~

Specification 012 removes semicolon sequencing in the same parser migration.
No migration example in this RFC uses semicolon syntax.

## Detailed implementation plan

Primary implementation surfaces are
`crates/snacc-compiler/src/syntax/{ast,lexer,parser}.rs`,
`crates/snacc-compiler/src/semantics/checker.rs`,
`crates/snacc-compiler/src/backend/llvm.rs`, checked bridge declarations
exported through `crates/snacc-compiler/src/lib.rs`, assertion generation in
`apps/cargo-snacc/src/main.rs`, `crates/snacc-runtime/src/lib.rs`, and the
language, phase, conformance, runtime, driver, and Cargo-hosted test suites.

### Phase 1: shared syntax model

1. Add `break` to `Token`, display, keyword mapping, recovery boundaries, and
   reserved-word contract tests.
2. Introduce `Block` and distinct syntax statement nodes shared with
   Specification 012. Represent `while` and `break` as statements and preserve
   the source span of every block and element.
3. Parse function, method, and bridge result types as optional. Parse branch and
   loop bodies as blocks.
4. Parse one optional-`else` `if` form; leave statement/value classification to
   the checker.
5. Remove the expression-valued `while` AST variant. Do not retain a
   compatibility node or synthesize `nil`.

### Phase 2: signature collection and checking

1. Change collected signatures to `Option<Ty>` results and update every
   exhaustive signature consumer. Specification 010 extends this same `Ty`
   representation with `User(TypeId)`; it does not introduce a second checked
   type representation.
2. Add explicit checking entry points for value-required blocks and no-result
   blocks rather than a boolean threaded through arbitrary expressions.
3. Convert no-result calls used as block elements to checked call statements;
   reject them from expression checking.
4. Check `while` bodies with a loop-target stack. Push only while checking the
   body, pop on exit, and resolve `break` to the innermost target.
5. Contextually classify `if`. Require total value paths only for value-form
   `if`; integrate Specification 010's exhaustive union result.
6. Ensure declarations and assignments from Specification 012 remain
   statements and cannot satisfy a value-required block.

### Phase 3: checked IR and LLVM

1. Split checked statements from checked expressions; remove result types from
   loop and break nodes.
2. Change function declaration and call lowering to LLVM `void` when the
   checked result is absent.
3. Lower loop targets with a stack of exit blocks so nested `break` selects the
   nearest loop.
4. Track whether each LLVM block is terminated before adding fallthrough or
   merge branches.
5. Delete the loop-result phi and `default_value`; search the backend for every
   remaining zero-fallback call.

### Phase 4: bridge ABI 2

1. Map an absent bridge result to Rust `()` in generated assertions and to LLVM
   `void` in declarations and calls.
2. Change the checked bridge declarations exported by `snacc-compiler` to carry
   an optional result. Update `render_bridge_assertions` in
   `apps/cargo-snacc/src/main.rs` to render an absent result as an
   `unsafe extern "C" fn(...)` pointer returning Rust `()` while retaining its
   deterministic declaration ordering.
3. Advance compiler and runtime ABI constants from 1 to 2.
4. Include ABI 2 in cache identity and reject compiler/runtime 1↔2 mismatches
   for programs with and without user bridges.
5. Update generated Cargo-host guidance and assertion snapshots.

### Phase 5: contract and corpus

1. Update the formal grammar first in `LANGUAGE.md`, then copy it identically
   to `GRAMMAR.ebnf`.
2. Update the no-result, function, bridge, `while`, `break`, `if`, ABI, and
   runtime-symbol rules in `LANGUAGE.md`.
3. Migrate every loop-value corpus case and remove obsolete expected output.
4. Confirm no retired standalone loop-semantics document remains or is
   referenced; `LANGUAGE.md` is the only normative contract.

### Phase 6: verification

1. Add parser cases for optional results, optional `else`, `while`, and
   valueless `break`.
2. Add checker cases for every value/no-result boundary and nested-loop target.
3. Add LLVM execution tests for zero, one, and multiple iterations; early
   `break`; nested `break`; and statement/value `if`.
4. Add Cargo-hosted no-result bridge assertion, execution, ABI mismatch, and
   cache invalidation tests.
5. Run formatting, workspace checking, and the complete workspace test suite.

## Conformance tests

A conforming implementation shall test at least:

1. Internal functions, methods, and Rust bridges with and without results.
2. No-result calls accepted as block elements and rejected in every expression
   position.
3. Value-required bodies ending in expressions and rejecting statements.
4. `while` execution for zero, one, and multiple iterations without a value.
5. `while` rejected from initializer, operand, argument, condition, branch
   result, and returned-expression positions.
6. `break` exits the innermost loop and is rejected outside a loop.
7. Statement-form `if` accepts omitted `else`; value-form `if` requires `else`
   or an exhaustive union chain.
8. Terminated LLVM blocks receive no second terminator.
9. No loop-result phi, fallback zero, dummy `Nil`, or no-result `Ty` remains.
10. No-result bridges assert against Rust `()` and execute successfully.
11. ABI 1↔2 mismatches fail before execution and ABI 1 cache objects are not
    reused.
12. The migrated corpus and examples produce their expected output.
13. Formatting, workspace checking, and all workspace tests pass.

## Acceptance criteria

1. No result is unspellable and distinct from every value type.
2. `while`, `break`, declarations, and assignments are statements.
3. Functions, methods, and bridges may omit a result type.
4. Statement-form `if` may omit `else`; value-form `if` is total.
5. Checked IR and LLVM contain no invented value for a no-result construct.
6. No-result bridge support establishes ABI version 2 consistently.
7. `LANGUAGE.md`, `GRAMMAR.ebnf`, parser, checker, lowering, bridge assertions,
   runtime version, cache identity, and tests agree.

## Rejected alternatives

### Keep loop zero values

They report a value that the program never chose and make a skipped loop
indistinguishable from a loop that computed zero.

### Use `Nil` or a unit type for effects

Either choice creates a padding value and allows it to be stored or passed.
No-result control flow needs no value at all.

### Require `else` on statement-form `if`

An omitted path in a no-result context means “do nothing” and needs no invented
value. Only value-form branching must cover every path.

### Add loop values and `break value`

They require result unification and phi construction without a current use
case. A later loop-expression feature can be designed independently.

## References

- [Language contract](../../LANGUAGE.md)
- [Specification 010](010-nominal-types-structs-unions-and-methods.md)
- [Specification 012](012-variable-declarations-assignments-and-member-mutability.md)
- [Archived bridge signature RFC](archive/007-bridge-signature-verification.md)
