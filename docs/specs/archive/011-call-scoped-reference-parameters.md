# Specification 011: Call-Scoped Reference Parameters

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Scope

This specification adds `Ref<T>`, a mutable, call-scoped reference parameter.
`Ref<T>` may appear only as the direct type of a function, method, or Rust
bridge parameter. A caller passes an initialized mutable place without writing
an address-of operator. Inside the callee, reading, assigning, selecting a
field, calling a method, or forwarding the parameter automatically accesses its
referent; the reference itself is not a source value.

This specification does not add raw pointers, reference-valued expressions,
reference locals, reference fields, reference results, nullable references,
uninitialized variables, output-only parameter types, reference arithmetic,
reference identity, or references that outlive a call.

This specification contains no open design questions. Its implementation order
and phase boundaries are fixed in section 15.

## 2. Normative references and dependencies

[The Snacc language contract](../../../LANGUAGE.md) is normative. This document
defines the change to that contract. If the implementation and the updated
contract disagree, the implementation is nonconforming.

[RFC 008](008-statements-and-functions-without-results.md) shall be implemented
first because assignment through a reference produces no result.

[Specification 010](010-nominal-types-structs-unions-and-methods.md) defines
fields, methods, and user-defined value types. [Specification
012](012-variable-declarations-assignments-and-member-mutability.md) defines
declaration statements, assignments, and the root-variable capability that
makes a place mutable. This specification uses both sets of definitions.
An implementation may land the scalar `Ref<T>` subset while implementing that
place and assignment machinery, but the complete acceptance criteria require
all three specifications.

[Specification 009](009-fixed-width-unsigned-and-float32.md) shall be
implemented before `Ref<T>` is admitted at Rust bridge boundaries. This fixes
the bridge type set and makes the ABI version required by section 12
unambiguous.

## 3. Terms

A **reference parameter** is a parameter whose declared type is `Ref<T>`.

The **referent type** is `T` in `Ref<T>`.

The **referent place** is the caller's mutable place supplied for a reference
parameter.

An **automatic dereference** is the language rule that an occurrence of a
reference parameter acts as an occurrence of its referent place rather than as
a readable reference value.

A **borrow interval** begins immediately before control enters the callee and
ends when the callee returns normally. A Rust bridge that violates its return
contract does not extend the interval.

Two places **overlap** when they are identical or one is reached by selecting
fields from the other. Two distinct fields of the same struct do not overlap.

## 4. Syntax

`Ref` becomes a reserved keyword. `<` and `>` delimit its referent type in a
type position; this use is distinct from ordered-comparison expressions.

The following rule extends `type` after applying Specifications 009 and 010:

~~~ebnf
type                 = value-type | reference-parameter-type ;
reference-parameter-type
                     = "Ref", "<", value-type, ">" ;
~~~

`value-type` denotes every non-reference type otherwise permitted at the type
site. `Ref<Ref<T>>` is not syntactically or semantically permitted.

The grammar accepts `reference-parameter-type` only in a parameter declaration.
It shall be rejected in every other type position even if parser recovery
temporarily represents it there.

No address-of or dereference expression is added. Call syntax is unchanged:

~~~snacc
fun add_into(x: Int64, y: Int64, result: Ref<Int64>) do
    result = x + y
end

let x: Int64 = 20
let y: Int64 = 22
let mut z: Int64 = 0
add_into(x, y, z)
print(z)
~~~

This program prints `42`.

## 5. Permitted declaration sites

`Ref<T>` may be the direct declared type of:

- a top-level Snacc function parameter;
- an explicit method parameter other than implicit `self`;
- an `extern rust` function parameter, subject to section 12.

It may not be:

- a function or method result;
- a local binding type;
- a struct field type;
- a represented type's representation;
- a union or union-member type;
- nested inside another type;
- the type of `self` written by an author;
- constructed, returned, or injected into a union.

The restriction is structural, not lexical. A represented type shall not hide
a `Ref<T>`, and no alias mechanism may make a reference storable.

## 6. Calling a reference parameter

### 6.1 Argument requirement

An argument corresponding to `Ref<T>` shall be an initialized mutable place of
exact type `T`:

~~~snacc
let mut total: Int64 = 0
add_into(20, 22, total)
~~~

The call contains no `&`, `ref`, pointer, or constructor syntax. The callee's
parameter declaration is the sole source indication that the argument is
passed by reference.

These arguments are invalid:

~~~snacc
let total: Int64 = 0
add_into(20, 22, total)       // immutable place

add_into(20, 22, 0)           // literal, not a place
add_into(20, 22, make_total()) // temporary, not a place
~~~

A field place rooted at a mutable variable is valid:

~~~snacc
type Totals is struct
    current: Int64,
end

let mut totals: Totals = Totals(current: 0)
add_into(20, 22, totals.current)
~~~

No implicit value conversion applies when establishing a reference. A mutable
`Int64` place cannot satisfy `Ref<Dec64>` even though an `Int64` value can widen
to `Dec64`; the callee must address storage of exactly the referent type.

### 6.2 Initialization

Every referent shall be initialized before the call. This specification does
not permit declarations such as `let result: Int64` without an initializer.
`Ref<T>` is an in/out parameter: the callee may read the incoming value before
writing it.

`Ref<T>` may be used operationally as an output when the caller supplies an
initialized value that the callee replaces:

~~~snacc
fun add_into(x: Int64, y: Int64, result: Ref<Int64>) do
    result = x + y
end

let mut result: Int64 = 0
add_into(20, 22, result)
~~~

The initial `0` remains a valid `Int64`; it is not uninitialized storage. The
callee is allowed to read it even when this particular function does not.

Ordinary function results are the preferred way to produce values:

~~~snacc
fun add(x: Int64, y: Int64): Int64 do
    x + y
end

let result: Int64 = add(20, 22)
result
~~~

The language defines no `Out<T>` type and no uninitialized local declaration.
Such features shall not be added without a demonstrated use that cannot be
served clearly by a result value or an initialized `Ref<T>` parameter.

### 6.3 Evaluation order

Call arguments are processed from left to right. A value argument evaluates to
and preserves its value. A reference argument evaluates its place once and
preserves the identity of that place. The borrow intervals begin only after all
arguments have been processed and immediately before control enters the
callee.

Consequently, reading a place into a by-value argument and also passing it by
reference in the same call is valid; the value argument contains the value read
before the callee starts:

~~~snacc
fun replace(previous: Int64, value: Ref<Int64>) do
    value = previous + 1
end

let mut number: Int64 = 4
replace(number, number)
~~~

After the call, `number` is `5`.

### 6.4 Exclusive access and overlap

Every `Ref<T>` parameter has exclusive access to its referent for the borrow
interval. Two reference arguments in one call shall not overlap:

~~~snacc
fun exchange(left: Ref<Int64>, right: Ref<Int64>) do
    let saved: Int64 = left
    left = right
    right = saved
end

let mut value: Int64 = 1
exchange(value, value) // error: overlapping reference arguments
~~~

A struct and one of its fields overlap. Two different fields do not:

~~~snacc
exchange(point.x, point.y) // valid when point is mutable
use_both(point, point.x)   // invalid when both parameters are Ref
~~~

The checker shall decide overlap from resolved place roots and field paths. It
shall reject a call when it cannot prove the reference arguments disjoint.

For a method call, an addressable receiver place participates in overlap
checking for the complete call. An explicit `Ref<T>` argument shall not overlap
that receiver, whether the method is read-only or receiver-writing, because the
method may access `self` while the reference has exclusive access. A temporary
receiver has independent storage and therefore cannot overlap a caller place.

## 7. Automatic dereference in the callee

### 7.1 Reading and writing

Reading a reference parameter reads its current referent value:

~~~snacc
fun increment(value: Ref<Int64>) do
    value = value + 1
end
~~~

The occurrence on the right of `+` loads the caller's value. The assignment
writes the result back to the caller's place. Assignment does not rebind the
parameter to another place; reference parameters cannot be rebound.

Normal value rules apply after an automatic read. A `Ref<Int64>` parameter may
be passed to an `Int64` parameter, compared with an `Int64`, printed as an
`Int64`, and widened to `Dec64` wherever an ordinary loaded `Int64` could be.
The reference itself is never printed or compared.

### 7.2 Fields and methods

Field access proceeds through the referent automatically:

~~~snacc
fun move_right(point: Ref<Point>, amount: Dec64) do
    point.x = point.x + amount
end
~~~

`point.x` denotes the caller's field place. If a method is invoked on a
reference parameter, method lookup uses `T`, not `Ref<T>`:

~~~snacc
print(point.length())
~~~

A receiver-writing method is valid on a reference parameter because its
referent is a mutable root for the call.

This specification does not change the implicit receiver rules in
Specification 010. Only an explicitly declared method parameter can use
`Ref<T>` under this specification.

### 7.3 Forwarding and reborrowing

A reference parameter may be supplied to another `Ref<T>` parameter without
special syntax:

~~~snacc
fun twice(value: Ref<Int64>) do
    increment(value)
    increment(value)
end
~~~

Each nested call creates a shorter exclusive reborrow. The outer function
cannot access the referent while the nested call executes and regains access
when it returns. Because calls are synchronous and references are not storable,
this interval is syntactically bounded by the call.

Supplying a reference parameter to a by-value `T` parameter copies its current
referent value instead of forwarding the reference.

## 8. Lifetime and escape prevention

A reference parameter exists only for one invocation. The language provides no
operation that obtains, stores, returns, compares, converts, or exposes its
address.

The following are invalid by the declaration-site rules:

~~~snacc
fun return_ref(value: Ref<Int64>): Ref<Int64> do value end

type Holder is struct
    value: Ref<Int64>,
end

let saved: Ref<Int64> = value
~~~

A closure cannot capture a reference because Snacc has no nested functions,
closures, or function values. Top-level state cannot receive it because
functions cannot access top-level locals and reference types are not storable.

These restrictions make every Snacc reference lifetime exactly call-scoped;
the language needs no lifetime parameters or general borrow graph.

## 9. Assignability and type identity

`Ref<T>` is invariant in `T`. It is satisfied only by a mutable place whose
exact type is `T`, or by a `Ref<T>` parameter being reborrowed.

`Ref<T>` is not a value type and does not participate in ordinary assignability,
branch common-type selection, equality, union injection, represented-type
wrapping, or zero-value rules. Automatic reads produce `T`, after which the
ordinary rules for `T` apply.

Nominal identity is preserved. Given:

~~~snacc
type UserId is Int64
~~~

`Ref<UserId>` requires mutable `UserId` storage. `Ref<Int64>` cannot refer to it
until the program explicitly unwraps into separate mutable `Int64` storage.

## 10. Control flow and mutation visibility

A write through `Ref<T>` is observable by the caller immediately and remains
observable after return. Normal left-to-right expression and sequence order
determines which value is read after a call.

~~~snacc
let mut value: Int64 = 1
increment(value)
print(value)
~~~

This prints `2`.

If a function returns through multiple control-flow paths, every completed
write performed before the selected return remains visible. `Ref<T>` does not
provide transaction, rollback, or definite-write semantics. A function may
return without changing its referent.

## 11. Internal lowering

A checked reference parameter records its referent type and reference
capability explicitly; downstream phases shall not infer it from source syntax.

An internal Snacc `Ref<T>` parameter lowers to a non-null pointer to the LLVM
storage type of `T`. A call passes the address of the checked referent place.
Automatic reads lower to loads and assignments lower to stores.

The pointer is an implementation detail. It shall not appear as a Snacc typed
value, and it shall not be materialized for a value argument. Lowering may add
target-correct non-null, alignment, dereferenceability, and no-alias attributes
only where they follow from the checked reference contract.

The compiler shall not heap-allocate solely to satisfy a reference argument. A
mutable local or parameter that may be referenced shall have addressable
storage for the required interval. Escape analysis is unnecessary because the
checker prohibits every escape.

## 12. Rust bridges and native ABI

### 12.1 Permitted referent types

An `extern rust` parameter may use `Ref<T>` only when `T` is already a permitted
by-value scalar in the active Snacc ABI. User-defined represented, struct,
member, and union types remain prohibited because Specification 010 does not
give them stable bridge layouts. Standalone `Nil` is excluded because
Specification 012 removes it from source and the following ABI version.

### 12.2 Rust mapping

For each reference-permitted scalar mapping `T` to Rust type `R`, `Ref<T>` maps
to `&mut R`. For example:

| Snacc | Rust bridge parameter |
| --- | --- |
| `Ref<Int64>` | `&mut i64` |
| `Ref<Dec64>` | `&mut f64` |
| `Ref<Bool>` | `&mut u8` |

After Specification 009, its additional scalar types map by the same rule.

Rust's function-pointer ABI compatibility contract guarantees that `*mut R`
and `&mut R` are ABI-compatible, while the type-layout contract also gives
pointers and references the same layout. Rust's mutable-reference contract
supplies the exclusive-access invariant required here. The generated bridge
signature uses the reference type rather than exposing a raw pointer to
application code.

The generated bridge assertion shall include the reference in the asserted
function-pointer type. A value parameter and a reference parameter are not
interchangeable even when their referent uses the same scalar representation.

### 12.3 Host contract

The compiler passes a non-null, correctly aligned, initialized, exclusively
borrowed `T` value. The Rust bridge may read and write it during the call. It
shall not retain its address or create any reference, pointer, callback, thread,
or external state that can access it after return.

Before returning, the bridge shall leave a valid Snacc `T` representation. In
particular, `Ref<Bool>` shall contain zero or one. Violating this contract is
invalid host code and need not be diagnosed by the Snacc compiler.

The bridge shall not unwind across the ABI boundary, as for existing bridge
functions.

### 12.4 ABI version

RFC 008 changes ABI version 1 to 2, and Specification 009 changes version 2 to
3. Implementing Rust bridge `Ref<T>` support shall change ABI version 3 to 4
because it adds pointer-based
bridge parameter representations and ownership rules. Compiler output,
`snacc-runtime`, direct builds, Cargo-hosted assertions, and cache identity shall
change together.

Internal-only `Ref<T>` support may be developed before the ABI change, but the
specification is not complete until the version 4 bridge contract and tests are
implemented.

No runtime print symbol is added because printing a reference parameter
automatically prints its current `T` value through the existing symbol.

## 13. Diagnostics

A conforming implementation shall produce structured source diagnostics for at
least:

| Condition | Required information |
| --- | --- |
| `Ref<T>` outside a direct parameter | The permitted parameter-only sites |
| Nested `Ref` | That references cannot contain references |
| Reference result, field, local, represented type, or union use | That references are not storable |
| Immutable argument | The `Ref<T>` parameter and immutable root binding |
| Non-place argument | The argument and requirement for mutable storage |
| Referent type mismatch | The exact expected and found types |
| Overlapping reference arguments | Both argument places and parameter names |
| Assignment to a reference parameter with wrong type | The referent type and supplied value type |
| User-defined Rust bridge referent | The permitted ABI scalar referent set |

Declaration-site violations belong to type resolution. Argument kind, exact
type, mutability, and overlap errors belong to call checking. Invalid host
representations and retained host references are host contract violations.

## 14. Compatibility

`Ref` becomes reserved, so a program using that spelling as an identifier must
rename it. Existing `<` and `>` comparisons are unchanged outside type syntax.

Existing by-value parameters remain by value. No call changes behavior unless
its declaration is explicitly changed from `T` to `Ref<T>`.

Changing a parameter between `T` and `Ref<T>` is a source and ABI breaking
signature change. Every caller and Rust bridge assertion shall be rebuilt.

## 15. Detailed implementation plan

The internal reference machinery shall land after Specifications 010 and 012
establish explicit places and root mutability. Rust bridge exposure lands only
after Specification 009 establishes ABI version 3.

Primary implementation surfaces are compiler parameter syntax, resolved and
checked signatures, place checking, LLVM function/call lowering, compiler
checked bridge declarations exported by `snacc-compiler`, Cargo assertion
generation, generated application guidance, and the checker, LLVM, driver, and
Cargo-hosted suites.

### Phase 1: syntax and resolved signatures

1. Add `Ref` keyword handling and parse angle brackets only in the direct
   parameter-type parser; expression comparison parsing remains unchanged.
2. Represent parameter syntax as a passing mode plus a spanned value-type path.
   Do not add `Ref` to the syntax or checked value-type enum.
3. Preserve a complete reference-type span and reject `Ref` at results, locals,
   fields, represented bodies, union members, nested references, and `self`.
4. Resolve the referent through ordinary built-in and qualified nominal type
   resolution, then store `ParameterMode::Value` or
   `ParameterMode::Reference` beside the resolved value type.
5. Update signature collection, duplicate-parameter checking, diagnostics, and
   every exhaustive parameter consumer before checking bodies.

### Phase 2: place and call checking

1. Bind a reference parameter as a place root of type `T` with reference
   capability. Reads create ordinary `T` loads; complete and field assignments
   target the referent.
2. Check arguments left to right. Materialize each value argument as a checked
   value and each reference argument as one resolved root plus field-ID path,
   evaluating neither more than once.
3. Require exact referent type and a mutable root for local-place arguments.
   Accept a reference parameter as a reborrow without requiring a local `mut`.
4. Compare all reference-argument paths pairwise. Reject equal paths and prefix
   relationships; accept paths that first differ at sibling fields. For method
   calls, also reject every reference path that overlaps the receiver place.
5. Represent checked value, place-address, and reborrow arguments as distinct
   variants so lowering cannot accidentally copy a reference argument.
6. Apply receiver lookup to referent type `T`. Integrate Specification 010's
   receiver-write validation by treating the referent as a mutable root.

### Phase 3: internal LLVM lowering

1. Lower a reference parameter to a pointer to the exact LLVM storage type of
   `T`; keep value parameters unchanged.
2. Give any local whose address is passed stable entry-block storage and lower
   its checked reads and writes consistently through that storage.
3. Lower local reference arguments to resolved place addresses and forwarded
   parameters to their existing pointer. Never allocate a conversion temporary.
4. Preserve the checked argument order when constructing LLVM call operands.
5. Add non-null, alignment, dereferenceability, and no-alias attributes only
   when supported by the active Inkwell/LLVM API and justified by this contract;
   correctness shall not depend on the attributes.
6. Add LLVM execution tests for complete replacement, nested-field mutation,
   forwarding, and simultaneous disjoint-field references.

### Phase 4: Cargo host and ABI 4

1. Permit bridge references only when the resolved referent is in the active
   scalar ABI set; reject every user-defined referent before producing the
   checked bridge declaration.
2. Extend checked bridge signatures and `render_bridge_assertions` in
   `apps/cargo-snacc/src/main.rs` with the exact `&mut R` mappings in section 12.
3. Advance compiler and runtime ABI constants from 3 to 4, update cache
   identity, and reject ABI 3↔4 combinations even when no user bridge exists.
4. Update generated Cargo application guidance with initialized, exclusive,
   no-retain, no-unwind, and valid-representation-on-return requirements.
5. Add real host round trips for every permitted scalar, signature mismatch
   cases, ABI mismatch, and cache invalidation tests.

### Phase 5: contract and final verification

1. Update formal EBNF first in `LANGUAGE.md`, then copy it identically to
   `GRAMMAR.ebnf`.
2. Add only the normative parameter-only, automatic-dereference, exclusivity,
   escape, reborrow, and bridge rules to `LANGUAGE.md`.
3. Search every parser, checker, typed-program, metadata, assertion-renderer,
   backend, and diagnostic match for parameter-mode exhaustiveness.
4. Run formatting, workspace checking, and the complete workspace test suite.

## 16. Conformance tests

A conforming implementation shall test at least:

1. A scalar reference parameter reads and changes its caller's mutable local.
2. A function with value and reference parameters implements the `add_into`
   example and produces `42`.
3. Immutable locals, literals, calls, arithmetic results, and conditional
   results are rejected as reference arguments.
4. A local declaration without an initializer remains invalid; initializing a
   mutable local and using it operationally as an output through `Ref<T>` works.
5. Struct fields rooted at mutable variables are accepted as reference
   arguments.
6. Exact referent typing rejects implicit numeric widening and nominal
   representation equivalence.
7. Automatic reads participate in arithmetic, comparison, printing, value
   calls, and existing value conversions as `T`.
8. Assignment through a reference updates complete scalar, represented, struct,
   and union values.
9. Field reads and writes through `Ref<Struct>` affect the caller's struct.
10. Forwarding a reference parameter creates a working nested reborrow.
11. Passing a reference parameter to a value parameter copies its current value.
12. Two identical reference arguments and parent/child field overlap are
    rejected.
13. Two statically distinct struct fields may be passed by reference together.
14. A method receiver and an overlapping explicit reference argument are
    rejected for both read-only and receiver-writing methods; a temporary
    receiver and an unrelated reference are accepted.
15. Value argument evaluation observes the pre-call value when the same place
    is also passed by reference.
16. `Ref<T>` is rejected as a result, local, field, represented type, union
    member, nested reference, and `self` annotation.
17. A reference cannot be constructed, returned, compared as a reference, or
    stored through any syntax.
18. Every permitted scalar `Ref<T>` round-trips through a real Rust bridge.
19. Generated Rust assertions distinguish `T` from `Ref<T>` and map the latter
    to `&mut R`.
20. Rust bridge writes of valid `Bool` representations are observed correctly
    by Snacc.
21. Compiler/runtime ABI version 3 and 4 mismatches fail before execution.
22. Cached ABI version 3 objects are not reused under ABI version 4.
23. No additional runtime print symbol is required for a referenced scalar.
24. `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and
    all workspace tests pass.

## 17. Acceptance criteria

1. `Ref<T>` appears only as a direct parameter type and never becomes a source
   value.
2. Callers pass initialized mutable places with ordinary argument syntax.
3. Callees read, assign, select fields, call methods, and forward references
   through automatic dereference.
4. Every reference is non-null, exact-typed, exclusive for its call, and unable
   to escape.
5. The checked program makes parameter mode, referent place, and reborrow facts
   explicit for lowering.
6. Internal lowering uses pointers without exposing pointer operations to
   Snacc.
7. Rust bridges support references only to stable ABI scalars, enforce the
   call-scoped host contract, and use ABI version 4 after Specification 009.
8. `LANGUAGE.md`, `GRAMMAR.ebnf`, compiler phases, bridge assertions, runtime
   version, and implemented behavior agree.
9. Every conformance test in section 16 passes.

## 18. Non-normative rationale

`Ref<T>` supplies the essential operation behind pass-by-reference programming:
a callee can update caller-owned storage. Restricting it to parameters removes
the operations that require general lifetime syntax—construction, storage,
return, nullability, and arbitrary aliasing—while retaining forwarding and
composition across synchronous calls.

Automatic dereference makes passing mode a property of the function contract
rather than an operation repeated throughout the body. Exact referent typing
prevents a temporary conversion buffer from masquerading as caller storage.
Initialized in/out semantics avoid uninitialized reads and definite-assignment
analysis. Return values and initialized references cover both ordinary output
and caller-owned update use cases without a second parameter category.

Exclusive call-scoped access is stronger than a C pointer but substantially
smaller than a general borrow checker. Every borrow begins at a call, ends at
its return, and can reach only a statically known mutable place.

## 19. External references

- [Rust Reference: type layout](https://doc.rust-lang.org/reference/type-layout.html#pointers-and-references-layout)
- [Rust Reference: mutable references](https://doc.rust-lang.org/reference/types/pointer.html#mutable-references-mut)
- [Rust function-pointer ABI compatibility](https://doc.rust-lang.org/core/primitive.fn.html#abi-compatibility)
