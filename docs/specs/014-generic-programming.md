# RFC 014: Generic Programming

Status: Proposed

Document kind: Feature specification (Rust-style RFC)

## Proposal state

This document is an exploratory proposal, not an implementation-ready
specification. It records a possible direction for compile-time generics and
the design gaps that must be resolved before the language contract or compiler
is changed.

`LANGUAGE.md` remains authoritative. Until this RFC is accepted and its
decisions are incorporated there, generic syntax is not part of Snacc.

## Summary

Snacc could support generics as compile-time polymorphism. A generic function
or type would be instantiated with concrete Snacc types, then checked and
lowered into concrete LLVM code. The initial direction is explicit type
arguments and monomorphization, with no runtime type erasure or dynamic
dispatch.

Possible examples:

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

The syntax and semantics in this document are deliberately provisional.

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

## Candidate syntax

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

Candidate alternatives include:

~~~snacc
fun identity[ T ](value: T): T do ... end
fun identity(of T)(value: T): T do ... end
~~~

Angle brackets are already unambiguous and implemented in type position for the
built-in `Ref<T>` form; RFC 016 proposes using the same closed syntax for
`Box<T>`. Generic parameter declarations and generic type applications can
reuse that tokenization and do not reopen the comparison-expression problem.

The unresolved ambiguity is a generic argument list in expression position.
Snacc's semicolon-free grammar consumes the longest expression it permits, so
`a < b > (c)` is already a valid comparison expression and cannot also be
silently reinterpreted as a generic call. Candidate solutions include requiring
an unambiguous marker such as `identity::<Int64>(42)` or selecting a different
call syntax. The eventual generic specification must choose one spelling and
must not use semantic name lookup to change how the token sequence parses.

Square brackets conflict with reserved list syntax. The `of` form is more
verbose and less familiar. Angle brackets remain preferred in declarations and
types regardless of the expression-position choice.

## Initial semantic direction

### Compile-time instantiation

A generic declaration is a compile-time family, not a runtime value. Each
concrete use creates an instantiation:

~~~text
identity<Int64>
identity<Dec64>
Pair<Int64, Bool>
~~~

The compiler emits only the concrete instantiations that are reachable from
the program's entry point. No generic value, runtime type descriptor, boxing,
reflection, or dynamic dispatch is implied.

### Nominal identity

Different type arguments produce different nominal types:

~~~text
Pair<Int64, Bool> != Pair<Bool, Int64>
~~~

There is no proposed variance or structural compatibility. Existing implicit
conversions apply to expressions at call and assignment boundaries, not to
generic type identity.

### Type parameter use

An unconstrained type parameter could initially be used for:

- parameter types;
- result types;
- local bindings;
- fields of generic types;
- construction of other generic types;
- value and reference arguments where the existing parameter rules allow them.

An unconstrained type parameter would not automatically support arithmetic,
comparison, printing, construction from literals, or type tests. The compiler
must reject an operation when it cannot prove that the operation is valid for
every permitted instantiation.

For example, this is likely invalid without a bound:

~~~snacc
fun add<T>(left: T, right: T): T do
    left + right
end
~~~

### Possible capability bounds

One possible later extension is a small set of compiler-defined capabilities:

~~~snacc
fun add<T: Add>(left: T, right: T): T do
    left + right
end
~~~

Possible capabilities include `Eq`, `Ordered`, `Add`, `Sub`, `Mul`, and `Div`.
This would not necessarily introduce user-defined traits or dynamic method
dispatch. A full trait system is a separate design and should not be assumed
by this RFC.

### Explicit arguments before inference

The first version should probably require explicit type arguments:

~~~snacc
identity<Int64>(42)
~~~

Inference could later permit:

~~~snacc
identity(42)
~~~

Inference is intentionally deferred because expected-result inference,
implicit numeric conversion, union injection, and branch typing could make
the initial rules ambiguous.

### Generic recursion

The initial generic implementation shall permit recursive and mutually
recursive calls only when every recursive edge preserves the same concrete
type-argument tuple for the called declaration. A recursive edge that changes
that tuple is rejected before specialization because it can create an
unbounded family of monomorphizations.

This is a generic-instantiation rule, not a change to ordinary function or
method recursion. A later generic RFC may relax it only with a finite,
deterministic specialization rule.

## Scope candidates

The following scopes are possible and remain undecided:

1. Generic functions only.
2. Generic functions and represented or struct types.
3. Generic unions and methods as well.
4. Generic Rust bridge declarations.

The likely incremental path is functions first, then generic nominal types,
then unions and methods. Generic `extern rust` declarations should probably be
excluded: a generic declaration does not map to one concrete external ABI
signature or necessarily one link symbol.

## Implementation direction

This is a possible implementation outline, not an approved task breakdown.

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
index, and LLVM function-generation model would all need a design pass before
implementation begins. In particular, the current checked program assumes
that every type and callable has already been resolved to a concrete identity.

## Known design gaps and open questions

This RFC intentionally leaves the following unresolved.

### Syntax and parsing

- What unambiguous syntax introduces explicit generic arguments in expression
  position while preserving maximal expression consumption?
- How are generic arguments parsed for qualified names and member calls?
- Is `Maybe<Int64>.Some(...)` acceptable syntax for a generic union member?
- Are nested applications such as `Pair<Int64, Pair<Bool, Dec64>>` supported?
- Does the grammar need a distinct type-application node rather than extending
  qualified names?

### Type rules

- Are generic arguments always explicit in the first version?
- If inference is added, does it infer from arguments, result context, or both?
- How do `Int64` to `Dec64` conversion and union injection interact with
  inference?
- Are generic types invariant in all positions?
- Are generic represented types permitted, for example `type Id<T> is T`?
- Are generic references permitted as `Ref<T>` parameters?
- Can an unconstrained `T` be used in equality when the operation is checked
  separately for every instantiation, or must bounds be present in the source?

### Capabilities and traits

- Are `Eq`, `Add`, and similar names built-in capabilities, ordinary traits, or
  not part of the first generic design?
- Can users define capabilities later?
- Are capabilities implemented by compiler-known operations, methods, or both?
- Can a user-defined struct satisfy `Eq` automatically when all of its fields
  satisfy it?
- Is there any need for a `Print` or formatting capability, given that current
  `print` deliberately accepts only scalar values?

### Runtime and ABI

- Are all generic instantiations required to have finite, statically known
  layouts?
- How should generic types interact with future heap allocation and runtime
  handles?
- Can a generic declaration cross the Rust bridge after explicit
  specialization, or must users write concrete bridge declarations?
- What symbol naming and collision rules are required for specializations?
- Should specialized functions be exported, or remain compiler-private?

### Recursion and compilation limits

- Is there a deterministic maximum number of specializations per declaration or
  compilation?
- How are diagnostics reported when an error appears in a specialized body:
  at the declaration, the use site, or both?

### Tooling and compatibility

- How are generic declarations displayed in diagnostics and workbench output?
- Does the language need a standard-library convention for generic containers?
- Can generic declarations be added without making `snacc-workbench` part of
  the public feature contract?
- How are generated specialization names represented in debug information?

## Non-goals for this proposal

- Runtime reflection or type inspection for arbitrary `T`.
- Automatic conversion between different generic instantiations.
- Higher-kinded types such as `F<T>` where `F` itself is a type parameter.
- Function values, closures, or generic closures.
- Dynamic dispatch or a runtime trait object model.
- Making Rust's `Vec<T>`, `HashMap<K, V>`, or other Rust containers directly
  usable as Snacc values.

## Suggested next decision

Before implementation, resolve the smallest useful first slice:

- angle-bracket syntax;
- explicit type arguments;
- generic functions;
- generic structs;
- monomorphization;
- no generic Rust bridge declarations;
- no inference or user-defined traits.

That decision should then be converted into a narrower implementation-ready
RFC with updated grammar, exact typing rules, diagnostics, and conformance
examples in `LANGUAGE.md`.

## References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 016: Box Indirection and Recursive Data Structures](016-box-indirection-and-recursive-data.md)
