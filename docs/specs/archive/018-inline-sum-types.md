# Specification 018: Inline Sum Types

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Proposal state

This implemented specification adds inline structural sum types such
as `Byte | Nil`. It complements named algebraic data types declared with
`type Name is union ... end`; it does not replace them.

`LANGUAGE.md` remains authoritative; the implemented contract is incorporated
there.

This specification contains no open design questions. Section 11 fixes the
implementation order and phase boundaries.

## 2. Motivation

A named union is appropriate when alternatives form a domain concept with
named, possibly structured cases:

~~~snacc
type Shape is union
    | Circle is struct
        radius: Float64,
      end
    | Rectangle is struct
        width: Float64,
        height: Float64,
      end
end
~~~

Requiring another declaration for every local combination of existing types is
unnecessary ceremony. A fallible scalar result should be expressible directly:

~~~snacc
fun read_byte(bytes: View<Byte>, index: Int64): Byte | Nil do
    bytes.at(index)
end

let result: Byte | Nil = read_byte(bytes, index)
~~~

Both forms are sum types. The named form introduces new namespaced member
types; the inline form combines existing types without declaring new members.

## 3. Syntax

The `|` operator forms a value type in every value-type position:

~~~ebnf
type                     = reference-parameter-type | sum-type ;
reference-parameter-type = "Ref", "<", sum-type, ">" ;
sum-type                 = primary-value-type,
                           { "|", primary-value-type } ;
primary-value-type       = builtin-value-type
                         | qualified-name
                         | parameterized-value-type
                         | "(", sum-type, ")" ;
~~~

`parameterized-value-type` includes closed built-in forms such as `Box<T>` and
`View<T>` when their owning specifications are implemented. This specification
does not add general user-defined generics.

`|` is recognized only while parsing a type. It does not add a value-level
operator and does not conflict with expression parsing. Whitespace around `|`
has no significance.

For a place whose type is an inline sum, the type-test target expands from a
qualified name to any one non-sum direct member type, including a built-in or
closed parameterized type. This permits `is Byte(byte)`, `is Box<Node>(node)`,
and `is Nil`. A test target cannot itself contain `|` because nested sums are
flattened and are not direct members.

Examples of valid positions include:

~~~snacc
fun find(index: Int64): Byte | Nil do
    nil
end

fun replace(value: Ref<Byte | Nil>) do
    value = nil
end

type CacheEntry is struct
    value: String | Nil,
end

let result: Byte | Unicode | Nil = nil
~~~

`Ref<T>` remains valid only in its existing parameter positions. A reference is
not itself a value-type member, so `Ref<Byte> | Nil` is invalid; the valid form
for referencing sum storage is `Ref<Byte | Nil>`.

## 4. Type identity and normalization

An inline sum is structural. Its identity is the unordered set of its direct
member types:

~~~text
Byte | Nil == Nil | Byte
~~~

Parenthesized and nested sums flatten:

~~~text
(Byte | Unicode) | Nil == Byte | (Unicode | Nil)
~~~

A source sum must contain at least two distinct members. Repeating a member,
including through flattening, is an error rather than silently changing the
written type:

~~~snacc
let invalid: Byte | Byte = 1u8
~~~

`Nil` may occur only in a sum containing at least one non-`Nil` member. It
remains invalid as a standalone variable, parameter, field, result, represented
type, or reference referent.

Each non-`Nil` member must be a fully resolved, storable value type. A named
union may itself be one member of an inline sum; its internal member types do
not flatten into the inline sum:

~~~text
Shape | Nil
~~~

This has the two direct members `Shape` and `Nil`, not `Shape.Circle`,
`Shape.Rectangle`, and `Nil`.

An inline sum is never identical to a named union, even when their possible
runtime values appear equivalent.

An inline sum cannot be the immediate representation in a represented-type
declaration:

~~~snacc
type MaybeByte is Byte | Nil // invalid
~~~

A represented type is opened by calling its named immediate representation
type. An inline sum has no type name that can serve as that call head. Programs
use the inline sum directly when no nominal abstraction is needed, or declare a
named union when the alternatives require a distinct domain type. This
restriction avoids introducing a second represented-type unwrapping operation.

## 5. Values and injection

A value of a direct member type injects implicitly into an expected inline sum:

~~~snacc
let present: Byte | Nil = 42u8
let absent: Byte | Nil = nil
~~~

The selected member is determined as follows:

1. An exact direct member match wins.
2. Otherwise, existing implicit conversions are considered and exactly one
   converted direct member must accept the value.
3. No match is a type error.
4. More than one converted match is an ambiguity error.

Consequently, an `Int64` value selects the exact `Int64` member of
`Int64 | Float64`, while it may widen into `Float64 | Nil` because no exact
`Int64` member exists.

An inline sum value is assignable to another inline sum only when their
normalized member sets are identical. There is no implicit subset-to-superset
conversion:

~~~snacc
let narrow: Byte | Nil = nil
let wide: Byte | Unicode | Nil = narrow // invalid
~~~

The program can decompose `narrow` and return or assign each bound member under
the wider expected type. This keeps the first version free of union subtyping;
in particular, assignment, argument passing, inference, and `Ref<T>` continue
to use exact sum identity. Adding an alternative to a public sum type therefore
remains an explicit breaking type change.

~~~snacc
fun widen(value: Byte | Nil): Byte | Unicode | Nil do
    if value is Byte(byte) then
        byte
    elseif value is Nil then
        nil
    end
end
~~~

A named union value may inject as one complete direct member of an inline sum,
but its individual member values first inject into the named union only when an
exact expected type establishes that route.

The literal `nil` selects `Nil` only when the expected inline or named union
contains exactly one `Nil` member. It still has no standalone type. `nil == nil`
and `print(nil)` remain invalid without an expected sum. Specification 020
removes the former `null` compatibility spelling.

## 6. Type tests and decomposition

Inline sums use the existing `is` condition. A non-`Nil` test names a direct
member type and binds its value:

~~~snacc
let result: Byte | Nil = read_byte(bytes, index)

if result is Byte(byte) then
    print(byte)
elseif result is Nil then
    let message: String = "missing byte"
    print(message)
end
~~~

The binding has the exact tested member type. It follows the existing branch
scope, unique-name, place-alias, mutability, move, and borrow rules. `Nil` has
no payload and cannot have a binding.

A type test may name only a direct member of the tested sum. Testing a member
inside a named-union member requires a second test after binding that named
union.

A chain is exhaustive when it tests every normalized direct member exactly
once. The existing unreachable-duplicate and unreachable-`else` rules apply.

## 7. Inference and common types

An explicit expected inline sum is required to inject different branch values.
It may come from a variable annotation, parameter, field, function or method
result, or an already checked enclosing expression.

~~~snacc
fun maybe_byte(found: Bool): Byte | Nil do
    if found then
        1u8
    else
        nil
    end
end
~~~

This specification does not synthesize a new inline sum merely because
otherwise unrelated branches have different types. Without an expected sum,
ordinary common-type rules continue to apply. This keeps type formation
explicit and prevents accidental broad sums.

## 8. Representation and value properties

An inline sum has a private tagged representation containing one direct member
value. For the initial implementation, it uses the existing named-union
lowering: an integer tag followed by one correctly typed storage field for each
direct member. Exactly one member field is semantically active. Reusing that
representation keeps one union-lowering strategy and avoids handwritten
maximum-payload layout logic. Named and inline sums may be compacted together
later without changing the language or its bridge ABI.

The compiler chooses a deterministic internal tag for each normalized member.
Source programs cannot observe tags, layout, padding, inactive fields, or
member order.

Value properties are structural:

- the sum is copyable only when every member is copyable;
- it is move-only when any member is move-only;
- it requires destruction when any member requires destruction;
- it is a borrowed type when any member is borrowed;
- its layout provides correctly aligned storage for every non-`Nil` member and
  its tag.

Copying, moving, dropping, or borrowing a sum applies only to its active member.
Borrow source identity propagates through an active borrowed member.

Two values of the same inline sum support `==` and `!=` when every direct member
supports equality. Different active member types compare unequal; equal active
member types use that member's equality. Ordered comparison, arithmetic, field
access, direct printing, and method declarations on an inline sum are not
supported. Programs first decompose the value.

## 9. Named unions and inline sums

The two forms serve different purposes:

| Property | Named union | Inline sum |
| --- | --- | --- |
| Syntax | `type Shape is union ... end` | `Shape | Nil` |
| Identity | Nominal | Structural member set |
| Members | Newly declared namespaced types | Existing types |
| Structured alternatives | Inline member structs | Existing struct types |
| Methods | On union or member types | On member types after decomposition |
| Typical use | Domain ADTs | Optional and local multi-type results |

There is no `Option<T>` type implied by inline sums. `T | Nil` is the direct,
canonical optional-value spelling.

## 10. Rust bridge and ABI

Inline sums are rejected in `extern rust` parameters and results in the first
version, including sums whose members individually have bridge representations.
Their tag and payload layout are compiler-private.

Internal Snacc functions, methods, fields, locals, `Ref<T>` parameters, boxes,
and views may use inline sums wherever the members satisfy the corresponding
storage and lifetime rules.

Adding inline sums changes internal calling conventions and requires advancing
the applicable compiler/runtime ABI when implemented. The numeric successor is
selected from the last implemented ABI specification at landing time.

## 11. Detailed implementation plan

### Phase 1: grammar and syntax tree

1. Add `|` as a type-position token and add parenthesized type grouping.
2. Parse a flat source-spanned list of two or more primary member types without
   changing expression parsing or maximal expression consumption.
3. Permit the new type node in every value-type position and inside the
   referent of `Ref<T>`.
4. Reject a reference as a sum member and preserve all existing position
   restrictions on `Ref<T>` and standalone `Nil`.
5. Add parser tests for whitespace, grouping, nested parameterized forms,
   malformed separators, one-member forms, and expression/type boundaries.

### Phase 2: resolution and canonical identity

1. Resolve every member before constructing a canonical inline-sum identity.
2. Flatten nested sums, detect duplicates, enforce at least two distinct
   members, and enforce the `Nil` and storable-member rules.
3. Reject an inline sum as the immediate representation of a represented-type
   declaration.
4. Intern normalized member sets so order and grouping produce one semantic
   type identity while retaining source order for diagnostics.
5. Keep named unions opaque during normalization.
6. Extend deterministic type rendering, diagnostics, and symbol components for
   inline sums.

### Phase 3: checking and control flow

1. Add exact-member injection followed by unique ordinary-conversion injection.
2. Require identical normalized member sets for sum-to-sum assignment.
3. Propagate expected sums through declarations, arguments, fields, results,
   constructors, and value-required branches.
4. Extend type-test parsing and checking to built-in and closed parameterized
   direct members, then add exact binding types and exhaustive-chain checks
   through the existing conditional machinery.
5. Compute copy, move, destruction, equality, and borrowed properties from all
   direct members.
6. Integrate member places with existing move, mutability, cleanup, and view
   source-identity analysis before lowering.

### Phase 4: lowering and cleanup

1. Assign deterministic internal tags from the canonical resolved member order.
2. Reuse named-union aggregate construction and lower one tag plus one
   correctly typed field per direct member. A `Nil` member requires no payload
   value but retains its deterministic tag.
3. Zero-initialize complete aggregate storage before installing a member so
   inactive fields and padding never carry uninitialized data through
   compiler-generated copies.
4. Lower injection, tests, bindings, equality, copies, moves, and active-member
   cleanup exclusively from checked nodes.
5. Preserve non-null box invariants and borrowed-view source identities when
   those types occur as members.
6. Advance the applicable ABI version and update compatibility diagnostics.

### Phase 5: conformance and documentation

1. Add positive tests for every type position, member category, injection rule,
   grouping equivalence, exhaustive test, and value property above.
2. Add negative tests for duplicates, standalone `Nil`, invalid members,
   ambiguous injection, missing expected types, non-exhaustive value branches,
   represented-type declarations, subset-to-superset assignment, unsupported
   operations, and bridge use.
3. Add execution tests for scalar, named-union, box, string, and borrowed-view
   members as their owning specifications land.
4. Update `LANGUAGE.md`, its leading grammar, and `GRAMMAR.ebnf` in the
   implementation change, keeping both grammar copies identical.
5. Update active dependent specifications to cite this contract and remove any
   workaround result unions.
6. Run formatting, workspace checking, and the complete workspace test suite.

## 12. Required diagnostics

The implementation diagnoses at least:

- fewer than two distinct sum members;
- a repeated member after flattening;
- standalone `Nil` or a sum containing only `Nil`;
- an inline sum used as a represented type's immediate representation;
- a `Ref<T>` or other non-value member;
- an unresolved or non-storable member;
- missing, impossible, or ambiguous member injection;
- assignment between different normalized sum sets;
- a type test naming a nonmember;
- a missing or duplicate member in an exhaustive chain;
- an operation unsupported on the complete sum;
- an inline sum in a Rust bridge signature.

## 13. Rejected alternatives

### Require a named union for every combination

This repeats declarations for local optional and multi-type results and obscures
simple signatures. Named unions remain available when alternatives deserve
domain names or structured payloads.

### Add `Option<T>`

`T | Nil` already expresses optionality through the general sum-type mechanism.
A dedicated optional container would introduce a second spelling and require
generic or special-case machinery.

### Treat member order as type identity

`Byte | Nil` and `Nil | Byte` describe the same alternatives. Making order
semantic would create incompatible types without changing their values.

### Implicitly synthesize sums from unrelated branches

Automatic formation can silently broaden types after a branch edit. Requiring
an expected sum keeps public and stored types explicit.

### Permit represented types over inline sums

The existing represented-type conversion syntax calls the immediate
representation type to unwrap a value. An inline sum has no callable type name.
Adding special unwrapping syntax would expand represented types solely for a
case already covered by direct inline sums and named unions.

### Implicitly widen a sum to a superset

This would add structural union subtyping to assignment, arguments, inference,
and reference compatibility. Decomposition followed by contextual reinjection
is explicit and complete, so the first version requires identical sum types.

### Expose runtime tags or layout

Observable tags would turn a source type set into an ABI promise and conflict
with nominal ADT abstraction. Decomposition remains type-directed through `is`.

## 14. Acceptance criteria

Implementation is complete only when:

1. `Byte | Nil` is valid in function results and local annotations;
2. normalized member order and grouping do not affect type identity;
3. duplicates and fewer than two members are rejected;
4. direct values and contextual `nil` inject into expected sums;
5. inline sums cannot directly represent nominal represented types;
6. sum-to-sum assignment requires identical normalized member sets;
7. type tests bind exact direct members and support exhaustive `if` chains;
8. named unions remain nominal and do not flatten into inline sums;
9. copy, move, destruction, equality, and borrow behavior follow all members;
10. inline sums and named unions share the initial tag-plus-member-fields
    lowering strategy;
11. inline sums cannot cross the first-version Rust bridge;
12. parsing and checking finish before lowering and every unsupported case fails
   closed with a structured diagnostic;
13. `LANGUAGE.md`, both grammar copies, parser, checker, lowering, and tests
    agree;
14. formatting, workspace checks, and all conformance tests pass.

## 15. Findings from Specifications 022 and 023

Specification 023 makes inline sums the language's error-reporting mechanism,
which exercises this specification harder than any example in it. Two findings
follow. Neither reopens a rule above and neither blocks implementation, so
section 1's readiness claim stands: the first is a forward-compatibility
consequence that a later specification may act on, and the second belongs to
the boundary between this specification and its consumers rather than to this
design.

**15.1 Exhaustiveness makes a published sum's member list a compatibility
surface.** Section 9's rule -- an `is` chain over an inline sum may omit `else`
only when it covers every direct member exactly once -- is what makes
Specification 023's error handling safe: a program that ignores an I/O failure
does not compile. It has a matching cost. Specification 023 section 15.1
records that adding a ninth member to its predeclared `Error` union would break
every exhaustive chain in every existing program, which is why that member list
is frozen on first release.

The same applies to any inline sum a library publishes in a signature. This
specification should say plainly that a sum's member set is part of its
compatibility contract, and that widening one is a breaking change for every
exhaustive consumer -- a documentation addition, not a rule change.

Whether a non-exhaustive opt-in is worth adding -- a way to write a chain that
tolerates future members -- is a separate question. It is deliberately left
open: the current design can be implemented as written, and an opt-in is a
strictly additive later feature. Adding one now would weaken the exhaustiveness
guarantee before anything has demonstrated it is too strict.

**15.2 The fallible-result ABI shape is unspecified. (gap, shared with Specification 023)** Section 10 fixes
an inline sum's *internal* representation as a private tag plus per-member
storage, and Specification 023 relies on values of type `T | Error` crossing
between generated Snacc code and `snacc-runtime`. Because the layout is
compiler-private, such a value cannot cross as a value, and Specification 023
section 12.2 defers the shape to "a tag and a payload out-parameter pair"
without fixing it.

Someone must fix it: which parameter carries the tag, how an owned `String` or
`File` payload transfers ownership across the boundary, and how the degenerate
`Nil | Error` case -- where one member carries nothing -- is encoded. It is a
private ABI rather than a language rule, but it is a shared one, and both
specifications currently point at each other for it.

## 16. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 016: Box Indirection and Recursive Data Structures](016-box-indirection-and-recursive-data.md)
- [RFC 017: UTF-8 Strings, Byte Views, and Unicode Views](017-utf8-strings-and-views.md)
- [Specification 020: Literal Cleanup and Numeric Radices](020-literal-cleanup-and-numeric-radices.md)
