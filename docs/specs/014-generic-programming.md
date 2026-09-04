# RFC 014: Generic Programming

Status: Proposed

Document kind: Feature specification (Rust-style RFC)

## Proposal state

This document is implementation-ready. It defines the first generic language
contract, the compiler's monomorphization model, and the required diagnostics.
Deferred items are explicitly non-goals for this version and do not leave an
open design question in the selected scope.

`LANGUAGE.md` remains authoritative. Until this RFC is accepted and its
decisions are incorporated there, generic syntax is not part of Snacc.

## Summary

Snacc supports generics as compile-time polymorphism. A generic function or
struct is instantiated with concrete Snacc types, then checked and lowered
into concrete LLVM code. The first version requires explicit type arguments
and uses monomorphization, with no runtime type erasure or dynamic dispatch.

Examples:

~~~snacc
fun identity<T>(value: T): T do
    value
end

print(identity<Int64>(42))

type Pair<A, B> is struct
    first: A,
    second: B,
end

let pair: Pair<Int64, Bool> =
    Pair<Int64, Bool>(first: 42, second: true)
~~~

The syntax and semantics in this document are normative for the first generic
version once copied into `LANGUAGE.md` and `GRAMMAR.ebnf`.

## Motivation

Generics could allow reusable algorithms and data structures without making
each concrete type combination a separate handwritten declaration. They would
also provide a natural basis for future containers such as `Array<T>` and
`Dictionary<K, V>`.

The design must preserve Snacc's existing properties:

- type checking before LLVM lowering;
- concrete, finite value layouts;
- nominal user-defined types;
- value semantics and explicit ownership boundaries;
- no function values, closures, or implicit shared state;
- a fail-closed compiler pipeline.

## Preferred syntax

The leading candidate uses angle brackets:

~~~snacc
fun identity<T>(value: T): T do
    value
end

type Pair<A, B> is struct
    first: A,
    second: B,
end

let value: Pair<Int64, Bool> =
    Pair<Int64, Bool>(first: 1, second: true)
~~~

Other considered spellings (rejected for this RFC) include:

~~~snacc
fun identity[ T ](value: T): T do ... end
fun identity(of T)(value: T): T do ... end
~~~

Angle brackets are already unambiguous and implemented in type position for the
built-in `Ref<T>` form; RFC 016 proposes using the same closed syntax for
`Box<T>`. Generic parameter declarations and generic type applications can
reuse that tokenization and do not reopen the comparison-expression problem.

The preferred spelling is the ordinary form used by the examples,
`identity<Int64>(42)`. Specification 027 resolves the parser ambiguity by
making comparison chaining illegal. The important edge case is:

~~~snacc
f < T > (x)
~~~

The intended generic interpretation is “call function `f` with type argument
`T` and value argument `x`.” The comparison interpretation would compare the
value expressions `f`, `T`, and `(x)`. Both interpretations use the same token
sequence because the lexer gives a function identifier, a variable identifier,
and a user-defined type name the same `identifier` token. With Specification
027's one-comparison-per-expression-level rule, the comparison interpretation
is rejected and `f<T>(x)` is deterministic. For multiple or nested type
arguments, the parser uses the same balanced-delimiter rule: when an
identifier is immediately followed by `<`, it attempts a balanced type-
argument list and commits to a generic call or constructor when the matching
`>` is immediately followed by `(`. The checker then verifies that the
identifier names the required callable or generic struct. A comparison using
the same tokens must parenthesize its operands:

~~~snacc
f<A, B>(x)                         // generic call
Pair<A, Pair<B, C>>(first: x)      // generic constructor
f((a < b), (c > d))                // comparisons, explicitly parenthesized
~~~

A comparison must be parenthesized before it is combined with another comparison:

~~~snacc
(f < T) > (x)
~~~

The syntax decision is now closed by Specification 027: generic calls keep the
ordinary `name<Type>(arguments)` spelling, the balanced-delimiter rule above
resolves multi-argument applications, and comparison chaining is illegal.
The first generic implementation supports nested type applications, so this is
valid:

~~~snacc
Pair<Int64, Pair<Bool, Float64>>
~~~

Generic function calls are limited to top-level function identifiers in this
RFC. Generic struct construction uses the explicit `Type<Args>(fields...)`
form shown above. Generic methods, qualified module calls, static generic
functions, and generic union-member constructors are out of scope because
Snacc has no generic method or module namespace contract yet.
The parser produces a distinct generic type-application/call node rather than
trying to reinterpret an ordinary name node later.

The grammar delta is:

~~~ebnf
generic-parameters     = "<", identifier, { ",", identifier }, ">" ;
type-arguments          = "<", type, { ",", type }, ">" ;
type-application       = identifier, type-arguments ;
generic-call           = identifier, type-arguments, arguments ;
generic-constructor    = type-application, arguments ;
~~~

`function-declaration` and `type-declaration` accept an optional
`generic-parameters` list after their identifier. `type-application` is an
additional `primary-value-type`; `generic-call` is an additional `postfix`
form for functions, and `generic-constructor` is the corresponding form for a
generic struct constructor. Nested applications recurse through `type`, and
balanced delimiters are required. These productions are the syntax contract;
the implementation must copy them into the main grammar when this RFC is
accepted.

Square brackets conflict with reserved list syntax, and the `of` form is more
verbose and less familiar. Angle brackets are therefore the sole generic
parameter and application spelling in this version.

## Initial semantic direction

### Compile-time instantiation

A generic declaration is a compile-time family, not a runtime value. Each
concrete use creates an instantiation:

~~~text
identity<Int64>
identity<Float64>
Pair<Int64, Bool>
~~~

The compiler emits only the concrete instantiations that are reachable from
the program's entry point. No generic value, runtime type descriptor, boxing,
reflection, or dynamic dispatch is implied.
Every reachable instantiation must have a finite, statically known layout after
substitution. Specializations remain compiler-private unless a later
specification explicitly adds an export rule.

### Nominal identity

Different type arguments produce different nominal types:

~~~text
Pair<Int64, Bool> != Pair<Bool, Int64>
~~~

There is no proposed variance or structural compatibility. Existing implicit
conversions apply to expressions at call and assignment boundaries, not to
generic type identity.

### Type parameter use

An unconstrained type parameter may be used for:

- parameter types;
- result types;
- local bindings;
- fields of generic types;
- construction of other generic types;
- value and reference arguments where the existing parameter rules allow them.

An unconstrained type parameter does not support arithmetic,
comparison, printing, construction from literals, or type tests. The compiler
must reject an operation when it cannot prove that the operation is valid for
every permitted instantiation.

For example, this is rejected because no capability bounds exist in version one:

~~~snacc
fun add<T>(left: T, right: T): T do
    left + right
end
~~~

### Capabilities are deferred

The first generic version has no capability or trait bounds. Consequently an
unconstrained parameter cannot be used with arithmetic, comparison, equality,
printing, literals, or type tests. A later specification may add compiler-
defined capabilities such as `Eq` or `Add`; that future syntax is not part of
this RFC.

~~~snacc
// Future syntax only; rejected by this RFC.
fun add<T: Add>(left: T, right: T): T do
    left + right
end
~~~

### Explicit arguments before inference

The first version requires explicit type arguments:

~~~snacc
identity<Int64>(42)
~~~

Inference is deferred; a later specification could permit:

~~~snacc
identity(42)
~~~

Inference is intentionally deferred because expected-result inference,
implicit numeric conversion, union injection, and branch typing could make
the initial rules ambiguous.

### Generic recursion and specialization limits

The compiler shall permit recursive and mutually recursive calls. It shall
track each instantiation as `(declaration identity, concrete type arguments)`
and enforce a deterministic implementation limit on specialization depth and
total specializations. A recursive edge that exceeds that fixed limit is a
compile-time error with the complete instantiation chain. The limit is an
implementation guard, not a user-visible type-system feature, and must be
stable for the same source program and compiler version.

## Selected scope

The first generic design covers generic functions and generic `struct` types.
Generic represented types (for example `type Id<T> is T`), generic unions,
generic methods, generic static functions, and generic `extern rust`
declarations remain outside this RFC. A generic declaration does not map to one
concrete external ABI signature or necessarily one link symbol, so bridge
specialization is deferred.

Implementation may stage functions and types as separate phases (or separate
execution plans) without changing this language-level scope.

## Implementation direction

This is the approved implementation direction.

The implementation is staged: first monomorphize generic functions, then add
generic struct types using the same substitution machinery. Generic represented
types and other deferred forms require a later specification.

1. Extend the syntax AST with generic parameter declarations, type
   applications, and generic call arguments.
2. Represent a type parameter during template checking with a distinct
   semantic type such as `Ty::Param(GenericParamId)`.
3. Collect generic declarations before checking bodies, preserving Snacc's
   source-order-independent declaration visibility.
4. Check generic bodies in an environment that maps each parameter to its
   declared type parameter and validates all operations or bounds.
5. At each concrete use, create an instantiation key consisting of the
   declaration identity and concrete type arguments.
6. Substitute concrete types, check the specialized body, and enqueue any new
   generic uses discovered during checking.
7. Lower only the resulting concrete program to LLVM. Generated symbols would
   need deterministic internal names such as
   `snacc$identity$Int64`.
8. Run layout and by-value cycle checks after substitution, because a generic
   body does not have one final representation until its arguments are known.

The existing `Ty`, type-definition table, checked expression tree, function
index, and LLVM function-generation model must be extended so each specialized
type and callable has a concrete identity before LLVM lowering. Generic
templates remain frontend-only and never reach code generation unresolved.

## Required diagnostics

The implementation diagnoses at least:

- missing, duplicate, or unknown generic arguments;
- generic calls supplied without explicit type arguments;
- attempts to infer type arguments in version one;
- use of a generic represented type, generic union, generic method, qualified
  generic call, generic static function, or generic Rust bridge declaration;
- a type argument that fails ordinary assignability after substitution;
- an unconstrained type parameter used with an operator, equality, printing,
  literal, or type-test operation;
- an invalid `Ref<T>` specialization under the existing referent rules;
- recursive specialization that exceeds the deterministic depth or count
  limit; and
- a specialized-body error, showing the declaration span, concrete use site,
  argument list, and instantiation chain.

These are frontend diagnostics before LLVM lowering. Their wording and source
spans must be deterministic for a given source program and compiler version.

## Acceptance criteria

Implementation is complete only when:

1. Generic functions and generic structs parse with the grammar in this RFC,
   including nested type applications and generic struct constructors.
2. Generic calls require explicit type arguments and comparison chaining remains
   illegal, so `f<T>(x)` has one parse.
3. Every reachable specialization is monomorphized exactly once per canonical
   declaration-and-argument key and has a finite layout before lowering.
4. Ordinary assignability, numeric widening, ownership, borrow checking, and
   by-value cycle checks apply after substitution.
5. Unconstrained parameters cannot use operators, equality, printing, literals,
   or type tests, and all invalid uses produce the diagnostics above.
6. Recursive specialization limits and declaration/use-site diagnostic notes
   are enforced deterministically.
7. Generic declarations and specializations never cross a Rust bridge, and
   generated symbols and debug metadata use canonical concrete argument names.
8. `LANGUAGE.md`, `GRAMMAR.ebnf`, parser, checker, checked tree, lowering, and
   tests agree; formatting, workspace checks, and relevant tests pass.

## Closed decisions and deferred future work

No open design questions remain for the selected first-version scope. The
following rules are closed; the final subsection lists intentionally deferred
future work.

### Syntax and parsing (closed)

- Generic calls use the ordinary `name<Type>(arguments)` spelling resolved by
  Specification 027's no-comparison-chaining grammar.
- Nested generic type applications are supported; the parser records balanced
  type-argument delimiters and produces dedicated application nodes.
- Generic function calls are limited to an unqualified top-level function
  identifier. Generic struct construction uses the explicit
  `Type<Args>(fields...)` form shown above. Qualified calls, generic methods,
  qualified generic type applications, static generic functions, and generic
  union-member constructors are explicitly deferred.

### Type rules (closed)

- Generic arguments are always explicit in version one; no inference is
  performed.
- After substitution, ordinary assignability and conversion rules apply. For
  example, `identity<Float64>(1)` is valid through the existing numeric
  widening rule, while `identity<Int64>(1.0)` is rejected.
- Generic types are invariant and nominal. Different argument tuples are
  different types; no variance or structural compatibility is added.
- Generic struct fields and function parameters/results may use `T` and other
  parameters. Generic represented types are deferred.
- A generic struct's copyability, move-only status, and structural equality are
  computed independently for each specialization from its substituted fields.
  Thus `Pair<Int64, Bool>` is copyable and equality-capable, while
  `Pair<String, Bool>` is move-only but remains equality-capable through
  `String`'s equality rule. This does not permit `==` on an unconstrained `T`
  inside a generic function.
- `Ref<T>` is permitted when the substituted referent satisfies the existing
  reference rules; a generic parameter does not bypass those rules.
- An unconstrained `T` cannot be used with operators, equality, printing,
  literals, or type tests. The checker rejects such use in the template rather
  than relying on each instantiation to make it valid.

### Capabilities and traits (closed for version one)

- `Eq`, `Add`, and similar capability names are not part of the first generic
  syntax or type system.
- User-defined capabilities, automatic capability derivation, and generic
  formatting are deferred. There is no implicit `Print` capability.

### Runtime and ABI (closed for version one)

- Every reachable specialization has a finite, statically known layout after
  substitution.
- Generic declarations cannot be Rust bridge declarations. Specializations
  are compiler-private and receive deterministic symbols derived from the
  declaration identity and canonical concrete type arguments.
- Heap allocation and runtime-handle interactions are deferred with the
  corresponding future type specifications.

### Recursion and diagnostics (closed)

- Recursive instantiation is bounded by a deterministic compiler limit on
  specialization depth and count. Exceeding it is a compile-time diagnostic,
  not a runtime failure.
- A specialized-body error reports the generic declaration as the primary
  source location and the concrete instantiation use site plus argument list as
  a secondary note. The diagnostic includes the instantiation chain when
  recursion or nested specialization is involved.

### Tooling and compatibility (closed for version one)

- Diagnostics print the declaration name, parameter names, concrete arguments,
  and source locations deterministically. Workbench output follows the same
  text but is not a public compatibility contract.
- Standard-library generic container conventions are deferred to the library
  specifications that introduce those containers.
- Generic declarations do not make `snacc-workbench` part of the language
  contract.
- Debug information records the source declaration and canonical concrete
  argument list for each emitted specialization.

### Deferred future work

Future specifications may add type inference, capability/trait bounds,
user-defined capabilities, generic represented types, generic unions or
methods, generic external bridges, heap/handle-specific rules, and generic
formatting. None is required to implement this RFC.

## Non-goals for this proposal

- Runtime reflection or type inspection for arbitrary `T`.
- Automatic conversion between different generic instantiations.
- Higher-kinded types such as `F<T>` where `F` itself is a type parameter.
- Function values, closures, or generic closures.
- Dynamic dispatch or a runtime trait object model.
- Making Rust's `Vec<T>`, `HashMap<K, V>`, or other Rust containers directly
  usable as Snacc values.

## Implementation readiness

The ordinary angle-bracket spelling, explicit type arguments,
monomorphization, generic-function and generic-struct scope, recursion limit,
operator restrictions, and diagnostic locations are all closed. The compiler
may proceed using the phases above; any deferred capability or type form must
be specified separately before implementation.

## Findings from Specifications 022 and 023

Two later specifications deferred a decision to this one. Their constraints
are recorded here as implementation inputs, not as new proposals.

**Closed generic forms are accumulating.** `Box<T>` (RFC 016), `View<T>`,
`Array<T, N>`, `List<T>`, `Map<K, V>`, `Set<T>` (Specification 019), and the
proposed `Task<T>` and `Chan<T>` (Specification 022 sections 5.3 and 5.4) are
all compiler-provided type forms that look generic and are not. Each one is
individually justified. Collectively they are the strongest evidence that user
programs will want the same expressive power, and they are also the best
available test corpus: the generic implementation should be able to express
most of that list without special cases.

**The error model no longer waits on this decision.** Specification 024 fixes
one predeclared `Error` struct, `T | Error`, and `return_on_error` as the sole
multi-reason recoverable-error convention. General generics shall not add a
parallel `Result<T, E>` convention. Signatures do not enumerate possible
string categories; each fallible API documents the categories it produces.

This does not block generic programming. Generic code must still be able to
accept, return, and preserve ordinary inline sums containing `Error`, so those
programs belong in the implementation corpus and the design is
judged against them rather than against abstract examples.

## References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 016: Box Indirection and Recursive Data Structures](archive/016-box-indirection-and-recursive-data.md)
- [Specification 020: Literal Cleanup and Numeric Radices](020-literal-cleanup-and-numeric-radices.md)
- [Specification 024: Error Handling](024-error-handling.md)
- [Specification 027: Boolean and Comparison Operators](027-boolean-and-comparison-operators.md)
