# Specification 021: Truthiness and Equality

Status: Proposed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Proposal state

This implementation-ready specification permits every value type in an `if`, `elseif`, or `while`
condition and defines one complete equality contract. Truthiness recursively
follows represented-type layers and active sum alternatives. It is false only
when that process reaches exact `Bool(false)` or `Nil`. Every other value is
truthy.

`LANGUAGE.md` remains authoritative until this specification is accepted,
implemented, and incorporated there.

Section 15 fixes the implementation order and phase boundaries.

Specification 024 preserves the uniform rule for `T | Error`: both active
members are truthy. Programs use `is` or `return_on_error` when they need to
distinguish success from error.

### 1.1 Dependencies and implementation order

The scalar truthiness and equality rules build on the implemented contract in
[`LANGUAGE.md`](../../LANGUAGE.md). Complete implementation additionally
depends on:

- [Specification 018](archive/018-inline-sum-types.md) for structural sum identity,
  active alternatives, and contextual `nil`;
- [Specification 020](020-literal-cleanup-and-numeric-radices.md) for the
  `Float64` name and final numeric literal surface;
- [RFC 016](archive/016-box-indirection-and-recursive-data.md) for box truthiness and
  the explicit absence of box equality;
- [RFC 017](017-utf8-strings-and-views.md) for string and initial view equality;
  and
- [Specification 019](019-collections-and-iteration.md) for collection and
  generalized-view truthiness and equality.

RFC 016 and Specification 018 are already-established prerequisites. The
remaining landing order is Specification 020 and RFC 017 as one coordinated
literal/type migration, then Specification 019, and this specification after
those types and collections exist. Implementing an owning type earlier does
not require a temporary duplicate rule: its own specification remains
authoritative until this specification replaces the affected truthiness or
equality clause.

This specification explicitly supersedes the current `LANGUAGE.md` rule that
admits NaN and defines comparisons involving it, originally established by
archived Specification 009. It also replaces the current exact-`Bool`
restriction on ordinary conditions. `LANGUAGE.md` is updated only when these
new rules are implemented.

## 2. Summary

Truthiness is a control-flow interpretation of a value, not a conversion:

~~~snacc
if false then
    print(1) // not executed
end

if 0 then
    print(2) // executed
end

let byte: Byte | Nil = 0u8
if byte then
    print(3) // executed: zero is not false or Nil
end

let missing: Byte | Nil = nil
if missing then
    print(4) // not executed
end
~~~

Equality remains explicit and type-directed:

~~~snacc
let found: Byte | Nil = 0u8
let missing: Byte | Nil = nil

print(found == nil)   // false
print(missing == nil) // true
print(0 == 0)         // true
print(false == false) // true
~~~

Truthiness and equality are separate language operations. In particular,
`if value` is not rewritten as `value != false`, `value != nil`, or any other
source comparison.

## 3. Terminology

A **condition value** is the value produced by the ordinary expression that is
the complete condition of an `if`, `elseif`, or `while`.

A value is **falsey** when its truthiness test selects the non-taken path. A
value is **truthy** when its truthiness test selects the taken path.

An **active alternative** is the direct member identified by the runtime tag of
a named union or inline sum.

An **exact type** is one canonical semantic type, including nominal identity,
inline-sum normalization, collection element types, and fixed-array length.

## 4. Condition grammar and admissible expressions

This specification adds no token, operator, precedence rule, or grammar
production. The existing grammar already admits:

~~~ebnf
while-statement      = "while", expression, "do", block, "end" ;
if-form              = "if", condition, "then", block,
                       { "elseif", condition, "then", block },
                       [ "else", block ], "end" ;
condition            = type-test | expression ;
~~~

The semantic restriction that an ordinary condition expression have type
`Bool` is replaced by the truthiness rules in section 5. A condition must still
be a value-producing expression. A declaration, assignment, `while`, `break`,
or call with no result cannot be a condition.

Type tests remain complete conditions that produce `Bool`. Their syntax,
bindings, exhaustiveness rules, and left-place restriction do not change. They
are not expression operands for `!`, `and`, or `or`; Specification 027
explicitly preserves this condition-only boundary in version one.

## 5. Truthiness

### 5.1 Complete rule

Truthiness is computed recursively by the first applicable rule:

1. the exact built-in `Bool` value `false` is falsey and `true` is truthy;
2. a represented value has the truthiness of its immediate representation;
3. a named-union or inline-sum value has the truthiness of its active direct
   alternative; an active `Nil` alternative is falsey; and
4. every other value is truthy.

Represented-type declarations are finite and inline sums are flattened, so
this recursion always reaches one terminal rule. There is no user-defined
truthiness operation and no type may override these rules.

`Nil` has no standalone type. Consequently, a bare `if nil then ... end` or
`while nil do ... end` is invalid because the condition supplies no expected
sum type into which the literal can be injected. Falsey `Nil` is observed
through a value of a named union or inline sum that directly contains `Nil`.

### 5.2 Scalar values

`true` is truthy and `false` is falsey.

Every numeric value is truthy, including:

- signed and unsigned integer zero;
- positive and negative floating-point zero;
- floating-point infinity produced by arithmetic.

NaN is not a Snacc value. Section 7.1 requires every operation or bridge result
that would introduce NaN to fail before the value becomes observable.

`Unicode` values are truthy, including the scalar with numeric value zero.

~~~snacc
if 0 then
    print(1) // executed
end

if 0.0 then
    print(2) // executed
end
~~~

### 5.3 Named represented types, structs, and union members

A represented value retains its nominal type but has the truthiness of its
immediate representation. This rule applies recursively through any number of
represented-type layers:

~~~snacc
type Flag is Bool

let disabled: Flag = Flag(false)
if disabled then
    print(1) // not executed
end
~~~

This does not make `Flag` assignable to, comparable with, or implicitly
convertible to `Bool`. It is one closed control-flow observation, analogous to
the existing rules that give a represented value its representation's layout
and define same-type represented equality through that representation.

A struct value is always truthy. Its fields are not inspected. Empty structs
are truthy.

A named union has the truthiness of its active direct alternative. Every
declared empty member and member struct is truthy because structs are truthy;
their fields are not inspected. An active `Nil` alternative is falsey. A
declared member that is itself a represented or sum value follows that value's
ordinary recursive truthiness.

~~~snacc
type State is union
    | Disabled is struct
        value: Bool,
      end
    | Unknown
    | Nil
end

let disabled: State = State.Disabled(value: false)
let unknown: State = State.Unknown()
let absent: State = nil

if disabled then
    print(1) // executed
end
if unknown then
    print(2) // executed
end
if absent then
    print(3) // not executed
end
~~~

`State.Unknown` is an ordinary nominal empty type. Its name does not give it a
special Boolean or falsey meaning.

### 5.4 Inline sums

An inline sum has the truthiness of its active direct member. Thus an active
`Nil` is falsey, an active `Bool(false)` is falsey, and an active represented
type follows its immediate representation. Other member kinds use their own
rules: numeric zero is truthy, a struct's fields are not inspected, and a box's
pointee is not inspected.

~~~snacc
fun show(value: Bool | Byte | Nil) do
    if value then
        print(1)
    else
        print(0)
    end
end

show(false) // 0
show(nil)   // 0
show(0u8)  // 1
show(true)  // 1
~~~

A represented type over `Bool` follows the underlying Boolean value. A struct
containing `Bool`, or a named-union member struct whose field is `Bool`, remains
truthy because field values do not define the containing struct's truthiness.

### 5.5 Strings, collections, views, and boxes

Every `String` is truthy, including the empty string. Every array, list, map,
set, and view is truthy, including an empty one. Every `Box<T>` is truthy; a box
is never null, and its pointee is not inspected.

An implementation may optimize a condition whose static type has no falsey
value, but the condition expression must still be evaluated exactly once and
all its effects must occur.

### 5.6 No implicit Boolean conversion

Truthiness is permitted only where this specification asks control flow to
select a path. It does not make one type assignable or convertible to `Bool`:

~~~snacc
let number: Int64 = 1

if number then       // valid and taken
    print(number)
end

let flag: Bool = number // invalid
accepts_bool(number)    // invalid when the parameter is Bool
~~~

Function and method overload resolution, assignment, return checking, union
injection, and Rust bridge signatures receive no truthiness conversion.

### 5.7 No implicit decomposition or narrowing

A truthiness condition neither binds a sum payload nor changes the static type
of a place in either branch. The `is` condition remains the sole sum
decomposition form:

~~~snacc
let result: Byte | Nil = read_byte(bytes, index)

if result then
    print(result) // invalid: result still has type Byte | Nil
end

if result is Byte(byte) then
    print(byte) // valid: byte has type Byte
elseif result is Nil then
    print(0u8)
end
~~~

For `Bool | Nil`, a falsey result intentionally does not distinguish active
`Bool(false)` from active `Nil`. A program that needs the distinction uses an
`is` test or equality with `nil`.

## 6. Equality operators

`left == right` tests equality. `left != right` is defined as the exact logical
negation of `left == right` for the same operands and admitted operand types.
Both operators produce `Bool`.

Operands evaluate exactly once, left before right. Equality observes operand
values without moving from either operand. An owning temporary remains alive
through the comparison and is destroyed afterwards.

Equality is available only through `==` and `!=`. A user method, including a
method named `equal`, `equals`, or any similar name, is an ordinary method and
cannot implement, replace, or overload these operators.

Except for the two explicit cases below, equality requires two operands of the
same exact type:

1. `Int64` and `Float64` may compare in either order after converting the
   `Int64` operand to `Float64`; and
2. the contextual `nil` comparison in section 8 compares a sum value with its
   `Nil` alternative.

No other numeric conversion, represented-type conversion, member injection,
sum widening, string conversion, or collection conversion occurs for equality.

## 7. Scalar equality

### 7.1 NaN is rejected

`Float32` and `Float64` use IEEE 754 finite values, signed zero, subnormal
values, and infinities, but a successful Snacc execution never contains a NaN
value. This supersedes the current `LANGUAGE.md` NaN comparison behavior and
refines Specification 020's proposed IEEE arithmetic contract only for a NaN
result; every admitted non-NaN result retains its specified IEEE value.

When every input to a built-in floating-point operation is compile-time known,
the compiler checks the operation and rejects a NaN result with a diagnostic at
that operation. It also rejects every other NaN result that ordinary constant
evaluation proves; general theorem proving or value propagation is not
required. When the compiler cannot prove the result statically, generated code
tests it immediately after the operation and terminates with a defined
invalid-floating-operation error if it is NaN. The invalid result is never
stored, returned, compared, printed, used as a condition, or passed to another
operation.

Every `Float32` or `Float64` result crossing from a Rust bridge into Snacc is
validated before admission and causes the same runtime error when it is NaN.
Future native imports, bit conversions, deserialization facilities, and other
ways of constructing floats must establish the same invariant at their entry
boundary.

This requirement covers every operation whose IEEE result is NaN, including
invalid forms involving zero or infinity. It is not limited to division, and
future floating-point library operations are subject to the same rule. All such
failures use one common fatal runtime entry with a stable
`InvalidFloatingOperation` reason; the implementation does not add one runtime
symbol per floating-point operation.
Compiler optimization must preserve this observable failure; fast-math modes
or transformations that assume away, defer, or erase the check are invalid.

The chosen invariant has a permanent runtime cost when non-NaN cannot be
proved: one unordered floating-point comparison and one conditional failure
edge after each floating-point-producing operation and each floating bridge
result. It also increases generated IR and may inhibit vectorization or
reassociation. It performs no allocation. The compiler may omit or combine a
check only when it proves that the exact transformation preserves the point and
behavior of every possible invalid-floating-operation failure. Benchmarking
floating-heavy programs and recording code-size and runtime deltas is required
as an implementation and release-readiness task, but performance does not
weaken the fail-closed semantic rule chosen here.

There is no NaN literal spelling. A compile-time rejection is a compiler
diagnostic. A value-dependent rejection is a defined runtime error because its
invalidity cannot in general be known before execution.

### 7.2 Equality rules

Two `Bool` values are equal exactly when they are the same Boolean value.

Two integer values of the same exact integer type are equal exactly when their
mathematical values are equal. Unsigned types do not compare with one another
across widths and do not compare with signed or floating-point types.

Two `Float32` values or two `Float64` values use IEEE 754 equality for admitted
Snacc values. Positive and negative zero compare equal. NaN has no equality
case because section 7.1 prevents it from becoming a Snacc value.

Ordered comparison of admitted floating-point values follows IEEE 754 numeric
ordering. Positive infinity is greater than every finite value and negative
infinity is less than every finite value; the two infinities compare equal only
to the same sign. Signed zero compares neither less nor greater than the other
zero. NaN has no ordered-comparison case because section 7.1 rejects it before
it becomes a Snacc value.

`Float32` compares only with `Float32`. Mixed `Int64` and `Float64` equality
first converts the `Int64` value to `Float64` using the ordinary widening rule.
Large integers that are not exactly representable in binary64 consequently
compare using their rounded `Float64` value.

Two `Unicode` values are equal exactly when they denote the same Unicode scalar
value.

Truthiness never participates in scalar equality. For example, `0 == false`
and `1 == true` are type errors even though `0` and `1` are both truthy.

Consequently, mixed equality can report equality for adjacent large integers
whose `Int64` values round to the same binary64 value:

~~~snacc
print(9007199254740993 == 9007199254740992.0) // true
~~~

### 7.3 Mixed ordered comparison

`Int64` and `Float64` may be compared in either order. The checker widens the
`Int64` operand to `Float64` before applying IEEE 754 ordered comparison:

~~~snacc
let lower: Bool = 0 < 1.0
let upper: Bool = 2.0 >= 1
~~~

This is the same widening rule used by mixed equality. `Float32` remains exact-
type-only and does not mix with `Int64` or `Float64`.

## 8. Equality with `nil`

One operand may be the contextual literal `nil` when the other operand has a
named-union or inline-sum type that directly contains `Nil`.

~~~snacc
let item: Byte | Nil = read_byte(bytes, index)

if item == nil then
    print(0)
elseif item != nil then
    print(1)
end
~~~

This comparison tests only whether the sum's active direct alternative is
`Nil`. It does not require equality support from any other member. Therefore,
`Box<Node> | Nil` may compare with `nil` even though `Box<Node>` does not
support direct equality.

The literal may appear on either side. The non-literal sum expression evaluates
once in the position dictated by normal left-to-right operand evaluation.

`nil == nil` and `nil != nil` are invalid because neither operand supplies an
expected sum type. A nested `Nil` inside a direct named member does not qualify:
the complete compared sum must itself contain `Nil` as a direct alternative.

Comparing a complete sum with any non-`Nil` member value remains invalid. A
program decomposes the sum first:

~~~snacc
let item: Byte | Nil = read_byte(bytes, index)

print(item == 1u8) // invalid

if item is Byte(byte) then
    print(byte == 1u8) // valid
end
~~~

## 9. User-defined and sum equality

### 9.1 Represented types

Two values of the same nominal represented type support equality when their
immediate representation supports equality. The operator compares the
immediate representations recursively under this specification.

A represented value never compares directly with its representation or with a
different represented type, even when both have identical storage:

~~~snacc
type UserId is Int64
type OrderId is Int64

let user: UserId = UserId(1)
let order: OrderId = OrderId(1)

print(user == UserId(1)) // valid and true
print(user == 1)         // invalid
print(user == order)     // invalid
~~~

### 9.2 Structs

Two values of the same nominal struct type support equality when every field
type supports equality. Fields compare in declaration order and comparison
stops at the first unequal field. Empty structs of the same type are equal.
Different nominal struct types never compare, even when their fields match.

### 9.3 Named unions

Two values of the same nominal union type support complete equality when every
direct member type supports equality. Different active member tags compare
unequal. Equal tags compare the active member payloads; two active `Nil`
alternatives are equal. Inactive payload storage is never read.

Named unions of different types never compare. A named union never compares
directly with one of its declared member values, except for the contextual
`nil` tag test in section 8.

### 9.4 Inline sums

Two inline-sum values support complete equality when they have the same
normalized direct member set and every member supports equality. Different
active member types compare unequal. Equal active member types use that
member's equality; two active `Nil` members are equal.

Member order and grouping do not affect normalized inline-sum identity, so
`Byte | Nil` and `Nil | Byte` are the same equality operand type. Distinct
member sets do not compare, and no subset-to-superset conversion is performed.

As with named unions, section 8 permits comparison against contextual `nil`
without requiring complete equality support from the other members.

## 10. Strings, collections, views, and boxes

Two strings are equal exactly when they contain the same Unicode scalar
sequence. Because valid UTF-8 has a unique encoding for a scalar sequence, an
implementation may compare byte lengths and bytes. Equality performs no
Unicode normalization, case folding, locale processing, or allocation.

Arrays of the same complete type, lists of the same element type, and views of
the same element type support equality when their element type does. They
compare lengths first and then elements in increasing index order, stopping at
the first unequal element. Views compare element sequences rather than source
identity. An array and a list never compare, and arrays with different lengths
have different types and never compare.

Maps and sets do not support whole-value equality in their first version.
`Box<T>` does not support direct equality in its first version; pointer identity
is not observable. Any represented type, struct, named union, inline sum, or
sequence whose recursive equality requirement reaches an unsupported type also
lacks complete equality.

## 11. Relationship between equality and truthiness

Equality never coerces operands according to truthiness. Truthiness never calls
equality. These examples are deliberately different operations:

~~~snacc
let zero: Int64 = 0
let absent: Byte | Nil = nil

if zero then
    print(1) // executed
end

if zero == 0 then
    print(2) // executed
end

if absent then
    print(3) // not executed
end

if absent == nil then
    print(4) // executed
end
~~~

A statically always-truthy condition is valid. It is not an error merely
because the compiler can determine that its branch is always selected.

## 12. Evaluation, ownership, and lowering contract

Every ordinary condition expression evaluates exactly once per condition
check. A `while` reevaluates its condition once before each attempted
iteration. A condition observes its result without moving from a place.

An owning temporary produced for a condition remains alive through the
truthiness observation and is then destroyed before the selected branch begins,
unless ordinary lifetime rules require a longer lifetime for another reason.
The compiler must preserve condition-expression effects even when the value's
static type is always truthy.

Checked truthiness carries one of these lowering facts:

| Checked fact | Runtime test |
| --- | --- |
| exact `Bool` | test the Boolean value |
| represented type | apply the checked truthiness fact of its immediate representation |
| named union or inline sum | select the active member by its checked tag, then apply that member's checked truthiness fact; `Nil` selects false |
| every other value type | evaluate the expression, then select true |

Lowering must consume that checked fact. It must not reconstruct type rules,
infer sum membership, inspect inactive storage, call source methods, allocate,
or reinterpret an arbitrary value as a machine Boolean.

Equality likewise carries a complete checked comparison plan: scalar compare,
mixed `Int64`/`Float64` compare, contextual-`nil` tag test, represented compare,
struct field sequence, named-union tag and payload compare, inline-sum tag and
payload compare, string bytes, or sequence elements. Lowering must not redo
overload resolution or type compatibility.

Both truthiness and equality are non-consuming observations. Existing borrow,
move, initialization, mutability, and cleanup rules continue to apply to every
subexpression and temporary.

Every checked floating-point-producing operation additionally records whether
it was rejected as a constant NaN or requires an immediate runtime NaN check.
Lowering must emit the latter check before making the result available to any
following checked node.

## 13. Rust bridge and ABI

Truthiness is not a bridge conversion. Rust parameters and results continue to
use their exact declared bridge types; a Rust integer, pointer, string, or
container is never admitted as `Bool` because it would be truthy in a Snacc
condition.

Equality operators are evaluated inside compiled Snacc code and add no bridge
type. The NaN invariant requires validation of every floating bridge result and
a defined invalid-floating-operation failure path. The runtime exposes one
common fatal entry with a stable `InvalidFloatingOperation` reason. If adding
that reason changes the physical runtime ABI, the implementation advances the
ABI version assigned to this change and rejects older cached objects and hosts.

## 14. Required diagnostics

The implementation diagnoses at least:

- a condition that produces no value;
- a contextless `nil` condition;
- a compile-time floating-point operation whose result is NaN;
- equality operands with incompatible exact types;
- equality between unsigned integers of different widths;
- equality between `Float32` and any non-`Float32` type;
- equality between a represented type and its representation or another
  represented type;
- equality between different nominal struct or union types;
- equality between different normalized inline sums;
- equality between a sum and a non-`Nil` member value;
- complete equality for a type whose recursive equality requirement reaches an
  unsupported field, member, element, or pointee type;
- `nil == nil` and `nil != nil` without an expected sum;
- direct equality of maps, sets, or boxes; and
- any attempt to use a no-result declaration, `Ref<T>` itself, a function, or a
  method as an equality value.

Diagnostics identify both operand types for an incompatible comparison and the
first recursive field, member, element, or pointee type that prevents equality.

## 15. Detailed implementation plan

### Phase 1: semantic inventory and checked representation

1. Audit condition and equality handling across syntax, type resolution,
   checking, ownership analysis, checked IR, LLVM lowering, diagnostics, and
   conformance tests; remove parallel or downstream-only rules.
2. Replace the checked ordinary-condition requirement of exact `Bool` with an
   exhaustive, recursive truthiness classification carrying the facts listed
   in section 12. Retain type tests as their existing explicit checked form.
3. Add one centralized equality-admission function that returns a complete
   checked comparison plan rather than a Boolean “supports equality” answer.
4. Make recursive equality capability computation cycle-safe and deterministic;
   reject unsupported leaves before lowering.
5. Add unit tests for classification of every built-in, represented, aggregate,
   sum, box, string, collection, view, and no-result category, including nested
   represented types and represented or sum members inside sums.
6. Inventory the implemented NaN comparison behavior and remove it in the same
   change that introduces the non-NaN invariant; do not leave compatibility
   paths that admit NaN into equality or truthiness.

### Phase 2: condition checking and ownership

1. Admit every value-producing expression as an ordinary `if`, `elseif`, or
   `while` condition while rejecting statements, no-result calls, and
   contextless `nil`.
2. Preserve existing source spans and evaluation order and record whether the
   condition is exact `Bool`, Nil-tagged, Bool-or-Nil-tagged, or always truthy.
3. Apply non-consuming place observation and temporary lifetime rules before
   lowering; verify that move-only places remain initialized after a condition.
4. Do not add branch narrowing, payload bindings, implicit `Bool` conversion,
   user-defined hooks, or warnings for always-truthy static types.
5. Add checker tests for false, numeric zero, signed zero, represented false,
   empty structs, named unions, every relevant inline-sum active member, empty
   collections, empty strings, boxes, and invalid bare nil.

### Phase 3: equality checking

1. Implement the exact-type matrix and the sole mixed `Int64`/`Float64`
   exception, including deterministic diagnostics for every rejected pair.
2. Implement contextual `nil` comparison as a dedicated sum-tag plan that does
   not require complete equality from other members.
3. Implement recursive equality plans for represented types, structs, named
   unions, inline sums, strings, arrays, lists, and views; reject maps, sets,
   boxes, and every aggregate that transitively contains an unsupported type.
4. Ensure normalized inline-sum identity, nominal user-type identity, fixed
   array length, and collection element types participate in exact matching.
5. Mark equality operands as observed rather than moved and retain owning
   temporaries until the comparison completes.
6. Add checker tests for every admitted and rejected pair, including sums that
   support `== nil` but not complete sum-to-sum equality.

### Phase 4: LLVM lowering

1. Reject every statically evaluated NaN-producing floating operation with its
   checked source diagnostic.
2. After every runtime floating-point-producing operation and every floating
   Rust bridge result, test for NaN and enter the defined failure path before
   exposing an invalid result. Do not enable floating optimizations that can
   erase or defer this behavior.
3. Lower exact `Bool` truthiness through its Boolean bit, represented
   truthiness through its checked representation plan, and sum truthiness by
   dispatching on the checked active tag and applying the member's plan.
4. Lower always-truthy static types to a true branch only after fully evaluating
   and cleaning up the condition expression as required.
5. Lower equality exclusively from checked comparison plans, using tag and
   length checks before payload or element checks and never reading inactive
   union storage.
6. Preserve admitted IEEE equality, left-to-right operand evaluation,
   short-circuit field and element comparison, and exact `!=` negation.
7. Add the runtime failure entry or reason. If that changes the physical runtime
   ABI, assign the ABI successor under the shared ABI policy and validate cache
   and host compatibility; in all cases test that invalid results never escape.
8. Benchmark representative scalar and vectorizable floating workloads before
   and after checks, record runtime and code-size deltas, and retain the checks
   regardless of cost unless this specification is superseded.
9. Verify every generated module and treat an impossible truthiness kind,
   equality plan, tag, payload, or cleanup state as an internal compiler error.

### Phase 5: contract and conformance

1. Keep the unchanged formal EBNF first in `LANGUAGE.md` and identical to
   `GRAMMAR.ebnf`; update the terse semantic text for conditions, values,
   equality, ownership, and each affected type.
2. Remove every statement that ordinary conditions require exact `Bool` and
   consolidate scattered equality clauses so they agree with this contract.
3. Add positive execution tests covering every falsey form and representative
   truthy scalar, aggregate, string, collection, view, and box value, plus both
   Boolean states through one and multiple represented layers, as their owning
   specifications land.
4. Add equality execution tests for scalars, valid floating-point edges,
   contextual nil, nominal types, normalized sums, strings, and sequences, plus
   negative conformance programs for every diagnostic in section 14.
5. Add constant and runtime tests for each practical NaN-producing operation,
   values dependent on runtime input, Rust bridge NaN results, error reporting,
   and optimization builds.
6. Add side-effect and ownership tests proving once-only left-to-right
   evaluation, no move from conditions or equality operands, temporary cleanup,
   while reevaluation, and short-circuit comparison.
7. Run formatting, workspace checking, and the complete workspace test suite.

## 16. Rejected alternatives

### Require `Bool` conditions

This would retain explicit Boolean-only control flow but would reject the
chosen Crystal-like rule and make optional-value presence tests unnecessarily
ceremonious.

### Convert truthy values to `Bool`

Truthiness is needed only for control-flow selection. A general conversion
would broaden assignment, calls, results, overload resolution, and bridge
behavior without adding expressive power.

### Treat zero and empty values as falsey

Doing so would make truthiness depend on each type's contents and would conflate
valid zero, empty string, and empty collection values with absence. The complete
rule remains “only false and Nil.”

### Admit NaN and merely define its truthiness

NaN would force every arithmetic, comparison, formatting, and interop contract
to account for an unordered exceptional value. Rejecting it when statically
known and trapping it immediately otherwise gives Snacc one observable numeric
invariant and prevents invalid floating results from propagating far from their
cause.

### Recursively inspect aggregate contents

Inspecting struct fields, box pointees, strings, or collection elements would
make truthiness depend on unrelated contents and private layout. Represented
types are deliberately different: their declaration explicitly says that the
whole value is represented by one other value, so truthiness follows that
representation without weakening nominal identity anywhere else.

### Narrow sums after a truthiness condition

Truthiness can merge several states, most visibly `Bool(false)` and `Nil` in
`Bool | Nil`. Keeping static types unchanged makes `is` the one explicit and
uniform decomposition mechanism and avoids hidden flow-sensitive types.

### Permit user-defined truthiness or equality

Hooks would make control flow and `==` invoke arbitrary code and would require
an operator protocol, dispatch rules, failure behavior, and generic bounds.
The closed built-in rules keep both operations visible and predictable.

### Use pointer or allocation identity for equality

Snacc values expose semantic contents rather than storage identity. Strings and
views compare sequences, while boxes deliberately have no equality until a
separate structural rule is justified.

## 17. Acceptance criteria

Implementation is complete only when:

1. recursively following represented layers and active sum alternatives is
   falsey exactly when it reaches `Bool(false)` or `Nil`;
2. represented `Bool(false)` values are falsey without becoming assignable or
   convertible to `Bool`, while struct fields and box pointees are not inspected;
3. zero, signed zero, empty strings, empty aggregates and collections, views,
   boxes, and false or Nil stored inside aggregate fields are truthy;
4. truthiness works only in `if`, `elseif`, and `while` and creates no implicit
   `Bool` conversion, overload, hook, payload binding, or type narrowing;
5. every condition evaluates exactly once per check and observes without
   moving, including always-truthy and move-only values;
6. equality produces `Bool`, evaluates left then right once, observes without
   moving, and `!=` is the exact negation of `==`;
7. equality requires the same exact type except for mixed `Int64`/`Float64` and
   contextual `nil` comparison;
8. every statically known NaN-producing operation is rejected at compile time,
   every value-dependent operation and floating bridge result is checked at
   runtime, and NaN never becomes an observable Snacc value;
9. admitted IEEE floating equality, exact integer and Unicode equality, and
   nominal represented-type equality behave as specified;
10. structs, named unions, inline sums, strings, arrays, lists, and views use the
   complete recursive and short-circuit rules above;
11. comparing a sum with contextual `nil` is a direct tag test and remains valid
    when another member lacks complete equality;
12. maps, sets, boxes, incompatible types, non-`Nil` sum members, standalone
    `Nil`, no-result forms, and references fail closed;
13. checked truthiness facts, equality plans, and NaN validation requirements
    are complete before lowering;
14. any physical change to the runtime invalid-floating-operation path or
     floating bridge validation receives the ABI successor under the shared ABI
     policy and rejects incompatible cached objects and hosts;
15. `LANGUAGE.md`, both grammar copies, implementation comments, diagnostics,
    and tests agree; and
16. formatting, workspace checks, and all conformance tests pass.

## 18. Fallible-result truthiness

Specification 024 makes `T | Error` the language's standard multi-reason
fallible result. It does not special-case truthiness.

Section 2's rule makes a value falsy only when following represented layers and
active sum alternatives reaches exact `Bool(false)` or `Nil`. A value of type
`File | Error` therefore reaches `File` or `Error`, and both are truthy. So:

~~~snacc
let opened: File | Error = File.open("input.txt")

if opened then
    // runs on failure too
end
~~~

does not test success: both active alternatives are truthy. This is deliberate.
The language does not give `Error` a second, contextual truthiness rule.

`T | Nil` has different behavior because active `nil` is falsy. That follows
from its value, not from a special success/failure convention. The standard
no-value fallible shape `Nil | Error` therefore has inverted-looking
truthiness: successful `Nil` is falsey, while failed `Error` is truthy. This is
intentional; a program shall use `is` for explicit handling or
`return_on_error` for propagation rather than use a bare fallible-result
condition as a success test.

## 19. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [Historical Specification 009: Fixed-Width Unsigned Integers and Float32](archive/009-fixed-width-unsigned-and-float32.md)
- [RFC 016: Box Indirection and Recursive Data Structures](archive/016-box-indirection-and-recursive-data.md)
- [RFC 017: UTF-8 Strings, Byte Views, and Unicode Views](017-utf8-strings-and-views.md)
- [Specification 018: Inline Sum Types](archive/018-inline-sum-types.md)
- [Specification 019: Collections and Iteration](019-collections-and-iteration.md)
- [Specification 020: Literal Cleanup and Numeric Radices](020-literal-cleanup-and-numeric-radices.md)
- [Specification 024: Error Handling](024-error-handling.md)
