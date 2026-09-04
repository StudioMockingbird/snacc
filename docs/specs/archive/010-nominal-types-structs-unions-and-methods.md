# Specification 010: Nominal Types, Structs, Unions, and Methods

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Scope

This specification adds nominal represented types, structs, tagged unions,
namespaced union-member types, field access, methods with an implicit `self`,
and type tests that narrow and decompose union values in `if` and `elseif`
conditions.

The design provides algebraic data types without enums, classes, inheritance,
virtual dispatch, general runtime reflection, raw pointers, or a separate
pattern-matching construct.

This specification does not add transparent type aliases, recursive
by-reference types, user-defined conversions, operator overloading, visibility
modifiers, generic types, reference values, object identity, heap allocation,
destructors, or user-defined types at a Rust bridge boundary.

This specification contains no open design questions. Its implementation order
and phase boundaries are fixed in section 19.

## 2. Normative reference and dependency

[The Snacc language contract](../../../LANGUAGE.md) is normative. This document
defines the change to that contract. If the implementation and the updated
contract disagree, the implementation is nonconforming.

[RFC 008](008-statements-and-functions-without-results.md) shall be implemented
before this specification. This specification does not reinstate a writable
unit or standalone `Nil` result.

[Specification 012](012-variable-declarations-assignments-and-member-mutability.md)
shall be implemented with this specification. It owns block syntax, variable
declarations, assignment, root-variable mutability, method receiver mutation,
and the special `Nil` union member. Its rules replace earlier drafts of those
portions of this specification. Its block, place, and union-member work lands
alongside this specification; its final removal of standalone `Nil` follows
Specification 011 and establishes ABI version 5.

[Specification 009](009-fixed-width-unsigned-and-float32.md) shall be
implemented first. Its additional scalar types participate in the rules below
exactly as the existing scalar types do.

## 3. Terms

A **represented type** is a distinct nominal type declared over one existing
type with `type N is T`.

A **struct type** is a nominal product of zero or more named fields.

A **union type** is a nominal tagged sum whose members are nominal types
declared inside the union.

A **member type** is a type declared by one alternative of a union. Its fully
qualified name is `Union.Member`.

A **place** is storage that may be read and, when mutable, assigned. Locals,
parameters, `self`, and fields selected from places are places. Literals,
arithmetic results, calls, constructors, and conditional results are values but
not places.

A **method** is a statically dispatched declaration associated with one exact
nominal receiver type. Its implicit `self` may assign fields only when the call
has a mutable receiver root under Specification 012.

A **type-test chain** is one `if` condition followed by zero or more `elseif`
conditions in which every condition tests the same place with `is`.

## 4. Lexical requirements

`type`, `is`, `struct`, `union`, `method`, `self`, and `mut` become reserved
keywords. They shall not be declared as identifiers. `self` is lowercase and
has no alternate spelling.

`.` continues an identifier path without whitespace significance. A qualified
type or method name contains exactly the components declared by this
specification; arbitrary module paths are not introduced.

## 5. Grammar

After applying RFC 008 and Specification 012, the following type-system rules
replace or extend the corresponding rules in `LANGUAGE.md` and `GRAMMAR.ebnf`:

~~~ebnf
type-declaration     = "type", identifier, "is", type-body ;
type-body            = type
                     | struct-body
                     | union-body ;
struct-body          = "struct", { field-declaration }, "end" ;
field-declaration    = identifier, ":", type, [ "," ] ;
union-body           = "union", union-member, { union-member }, "end" ;
union-member         = "|", ( "Nil"
                     | identifier, [ "is", struct-body ] ) ;

qualified-name       = identifier, { ".", identifier } ;
type                 = builtin-type | qualified-name ;

condition            = type-test | expression ;
type-test            = place, "is", ( qualified-name | "Nil" ),
                       [ "(", identifier, ")" ] ;

postfix              = atom, { arguments | member-suffix } ;
member-suffix        = ".", identifier, [ arguments ] ;
arguments            = "(", [ argument, { ",", argument }, [ "," ] ], ")" ;
argument             = [ identifier, ":" ], expression ;

atom                 = literal
                     | identifier
                     | "self"
                     | list-literal
                     | print-expression
                     | "(", expression, ")" ;
~~~

`builtin-type` denotes the built-in value-type alternatives specified by
`LANGUAGE.md` when this specification is implemented. Specification 012
supplies the program, block, statement, variable, assignment, field, method,
conditional, and loop rules used with these type-system additions.

The parser shall distinguish named constructor arguments from ordinary
expressions by the `identifier : expression` form inside an argument list.
Named arguments are valid only for struct construction.

## 6. Declarations and names

### 6.1 Type declarations

Type declarations are top-level and visible throughout the program independent
of source order. A type name shall be unique among built-in and user-defined
top-level type names. Type references occupy a namespace separate from local
bindings. Because a bare type constructor and a function call both use
`name(...)`, a top-level type name shall not duplicate a Snacc function or Rust
bridge name.

At an expression call head, a qualified path whose first component resolves to
an in-scope local, parameter, or `self` is a receiver access. Otherwise the
checker resolves the path as a type constructor or top-level callable. This
rule is independent of capitalization and produces no runtime lookup. A bare
`name(...)` never calls a local because Snacc has no function values; it resolves
only a top-level callable or type constructor.

The right side of `type N is T` resolves `T` before `N` is introduced as a
complete type. A represented type, struct field, or union member shall not
contain itself directly or through a cycle. Recursive types require an
indirection facility and are outside this specification.

### 6.2 Union-member namespaces

Every union member declares a new type inside that union's namespace. A member
name shall be unique within its union. It does not enter the enclosing top-level
type namespace.

~~~snacc
type Direction is union
    | East
    | West
end
~~~

This declaration creates the types `Direction`, `Direction.East`, and
`Direction.West`. It does not create top-level `East` or `West` types.

A bare member is exactly shorthand for an empty struct member:

~~~snacc
| East
~~~

has the same meaning as:

~~~snacc
| East is struct end
~~~

Union members in this specification are either bare empty structs or inline
structs. They do not refer to an unrelated existing type and they do not nest a
second union.

### 6.3 Method names

A method declaration's qualified name consists of a receiver type path followed
by exactly one method name. A top-level receiver therefore uses two components;
a union-member receiver uses three:

~~~snacc
method Point.length(): Dec64 do ... end
method Shape.Circle.area(): Int64 do ... end
~~~

A method shall be declared in the same program as its receiver type. A method
name shall be unique for one receiver type; overloads are not permitted. Two
different receiver types may use the same method name. Methods and fields have
separate lookup roles, but `value.name` without `()` always denotes a field and
never a method value.

Methods are visible independent of declaration order.

## 7. Nominal represented types

### 7.1 Identity and representation

~~~snacc
type UserId is Int64
~~~

`UserId` is distinct from `Int64` and from every other type represented by
`Int64`. It has the same set of representable bit patterns and the same native
size and alignment as its immediate representation type, but representation
identity does not establish assignability.

Represented types are not aliases. They do not inherit arithmetic, ordered
comparison, fields, or methods from their representation type.

### 7.2 Wrapping and unwrapping

Calling the represented type with one positional argument explicitly wraps its
immediate representation:

~~~snacc
let id: UserId = UserId(42)
id
~~~

Calling the immediate representation type with one value of the represented
type explicitly unwraps one layer:

~~~snacc
let number: Int64 = Int64(id)
number
~~~

These two type-constructor forms require an exact immediate type match. They do
not introduce general numeric casts, skip represented-type layers, or permit
named arguments.

### 7.3 Equality

Two values of the same represented type support `==` and `!=` exactly when
their immediate representation supports equality. Equality compares the
represented values. A represented value does not compare directly with its
representation or with another nominal type.

No other operator is inherited. Arithmetic over a represented numeric type
requires explicit unwrapping and wrapping or a later operator-definition
facility.

## 8. Struct types and values

### 8.1 Fields

A struct contains its declared fields in declaration order. Field names shall
be unique within the struct. A field's type is fixed by its declaration.

Fields are readable throughout the program. This specification adds no
visibility boundary. Field access requires a struct value or place and names a
declared field:

~~~snacc
point.x
circle.radius
~~~

Field access does not invoke code.

### 8.2 Construction

A struct value is constructed by calling its type with named arguments:

~~~snacc
let point: Point = Point(x: 3.0, y: 4.0)
point
~~~

Every declared field shall occur exactly once. Unknown, missing, and duplicate
field names are errors. Argument order does not affect which field receives a
value. Argument expressions evaluate from left to right in written order. Each
argument value shall be assignable to its field type.

Positional construction of a non-empty struct is invalid.

### 8.3 Empty structs

An empty struct is valid and has exactly one value. It is constructed with an
empty argument list:

~~~snacc
type Marker is struct end
let marker: Marker = Marker()
marker
~~~

The parentheses are required. A type name alone is never a value.

### 8.4 Value and equality semantics

Structs are values without object identity. Binding, passing, returning, and
union injection copy the complete value. This specification permits only
recursively copyable fields, so copying transfers no allocation or destructor
obligation.

Two values of the same struct type support `==` and `!=` when every field type
supports equality. Fields compare in declaration order with short-circuiting.
All values of one empty struct type compare equal. Different nominal struct
types never compare directly, even when their fields are identical.

Structs do not support ordered comparison or arithmetic.

## 9. Union types and values

### 9.1 Membership and injection

A union value contains exactly one direct member value and a tag identifying
that member's nominal type.

A value of `Union.Member` is implicitly assignable to its containing `Union`.
This direct member injection is permitted in bindings, arguments, returns, and
conditional branch unification. No conversion from a union to a member is
implicit, and no member is assignable to a different union.

~~~snacc
let direction: Direction = Direction.East()
direction
~~~

Construction always names the member type, not the union:

~~~snacc
Shape.Circle(radius: 10) // Shape.Circle
Shape(value)             // invalid
~~~

### 9.2 Equality

Two values of the same union type support `==` and `!=` when every member type
supports equality. Values with different tags are unequal. Values with the same
tag compare their contained member values. A union does not compare directly
with one of its member types.

Union values do not support ordered comparison or arithmetic.

### 9.3 No enumeration semantics

A bare empty member remains a nominal empty object type. The language does not
assign or expose integer discriminants, permit conversion between a member and
an integer, or introduce an enum declaration category. The runtime tag is an
unobservable implementation detail except through `is`.

## 10. Declarations, assignment, and root mutability

Specification 012 exclusively defines declaration statements, block scope,
duplicate-name rejection, initialized `let`, root-variable `let mut`,
assignment, and their interaction with `Ref<T>`. This specification supplies
the nominal fields and values to which those rules apply.

## 11. Methods and `self`

### 11.1 Receiver

Every method has exactly one implicit receiver named `self`. `self` is a
keyword, not a parameter declaration. It cannot be declared as another name,
or used outside a method body.

`self` denotes the original receiver storage for the call. A method may assign
to `self` as a whole or through its fields; Specification 012 requires a
mutable receiver root at every call that may perform such an assignment. There
is no `method mut` declaration form or source-level mutable-method category.

~~~snacc
method Point.length(): Dec64 do
    sqrt(self.x * self.x + self.y * self.y)
end

method Point.translated(dx: Dec64, dy: Dec64): Point do
    Point(x: self.x + dx, y: self.y + dy)
end
~~~

Here `sqrt` denotes an ordinary separately declared Snacc function or Rust
bridge; this specification does not add it as a built-in.

A method body may write receiver fields without an annotation. The checker
records the transitive receiver-write fact only to validate call sites under
Specification 012.

### 11.2 Calls

`receiver.name(arguments)` resolves statically from the receiver's exact
nominal type. It is not virtual dispatch and performs no runtime member search.
The receiver expression evaluates once before the explicit arguments, which
then evaluate left to right.

A read-only method may be called on a value or place. A method that may write
through `self` requires a mutable-rooted receiver place and therefore cannot be
called on a temporary.

Method calls do not expose methods as values. `point.length` without `()` is an
error unless `length` is a field. Methods cannot be nested and cannot capture
lexical state.

Methods on a union receive the union value. A member method is callable only on
that member type, ordinarily through a type-test binding:

~~~snacc
if shape is Shape.Circle(circle) then
    circle.area()
else
    0
end
~~~

### 11.3 Results

A method with `: T` returns a value assignable to `T`. A method with no result
type produces no result under RFC 008. Its call is valid only in a statement
position.

Methods are internal Snacc declarations. This specification does not add
external Rust methods or method syntax to `extern rust`.

## 12. Type tests, narrowing, and decomposition

### 12.1 Meaning of `is`

`is` has one type-relationship meaning in both declarations and conditions.
In a condition, `place is Union.Member` tests whether the union place currently
contains that direct member type and produces `Bool`.

~~~snacc
if direction is Direction.East then
    east()
else
    west()
end
~~~

The left side shall have the containing union type. The right side shall name a
direct member of that union. Testing unrelated types is an error rather than a
constant false result. Testing a value against its own static type is an error
rather than a constant true result. General reflection and structural type
tests do not exist.

### 12.2 Binding form

`place is Union.Member(name)` additionally binds the contained member value to
`name` when the test succeeds:

~~~snacc
if shape is Shape.Circle(circle) then
    circle.radius * circle.radius
elseif shape is Shape.Rectangle(rectangle) then
    rectangle.length * rectangle.width
end
~~~

The binding has the exact member type, is immutable, and is scoped only to that
branch. It does not exist in later conditions, later branches, or after the
`if`. Under Specification 012, its name shall be unused everywhere else in the
containing function or method, including sibling branches.

Both forms are valid only as the complete condition of an `if` or `elseif`.
The no-binding form supplies the `Bool` condition but does not introduce a
general type-reflection expression elsewhere in the language.

Binding an empty member is permitted but unnecessary.

### 12.3 Evaluation

Each written type-test condition reads its left place once. An exhaustive chain
shall test the same syntactic place in every condition. That place may be a
local, parameter, `self`, or a sequence of direct field accesses rooted at one
of them. Calls and other computed expressions are not places and cannot be the
subject of `is`; the author shall bind such a result first.

### 12.4 Exhaustiveness and omitted `else`

RFC 008 permits a statement-form `if` to omit `else` because no branch value is
required. A value-form `if` may omit `else` only when all of the following hold:

1. Every condition is an `is` test.
2. Every test has the same union-typed place.
3. Every direct member type of that union appears exactly once.

Such a chain is exhaustive and may produce a value from its branches. The
ordinary common-result-type rule applies across all branches. Every branch of
a value-form chain shall end in a value-producing expression.

If any member is absent, any condition is not a qualifying type test, or the
conditions inspect different places, a value-form chain requires `else`.
Testing one member twice is an unreachable-branch error. Supplying `else` after
covering every member is also an unreachable-branch error in either statement
or value context.

This rejection is intentional. Snacc does not admit a defensive `else` after a
statically exhaustive chain: every written branch must be reachable, and adding
a union member must force the previously exhaustive chain to handle that member
explicitly.

Adding a member to a union therefore makes every formerly exhaustive chain
that lacks `else` fail to check until it handles the new member.

## 13. Assignability and common types

Existing exact-type and numeric-conversion rules remain in force. This
specification adds only direct union-member injection as an implicit
conversion.

Branches have common union type `U` when every branch is `U`, a direct member of
`U`, or a contextually valid `nil`, and `U` is determined by an expected type,
one union-typed branch, or the shared containing union of all member-typed
branches. Each member type names exactly one containing union, so this rule
requires no search over declared unions. If these sources imply different
unions or do not determine one union, the branches have no union common type.

No structural assignability exists. Structs with identical fields, represented
types with identical representations, and empty types with identical layouts
remain distinct.

## 14. Printing and zero values

`print` does not support represented, struct, member, or union types. Programs
may print their scalar fields or explicitly unwrap a represented scalar. This
limitation is intentional and requires no follow-on work for conformance; any
future derived or user-defined formatting is a separate language feature that
shall receive its own numbered specification before implementation.

This specification defines no implicit zero value for a user-defined type. RFC
008 removes the language construct that currently requires a zero value before
this specification is implemented.

## 15. Native representation and lowering

### 15.1 Representation privacy

The native layout of every user-defined type is private to one compiler build.
Source programs cannot observe size, alignment, field offsets, union tags, or
padding. No layout guarantee crosses a Rust bridge under this specification.

### 15.2 LLVM types

A represented type lowers to its immediate representation's LLVM type but
retains distinct identity in syntax and checked representations.

A struct lowers to an LLVM struct whose fields occur in declaration order. An
empty struct lowers to an empty LLVM struct value.

For the initial implementation, a union lowers to an LLVM struct containing an
integer tag and one storage field for every member type. Exactly one member
field is semantically active. This intentionally uses more storage than a
maximum-sized payload representation; it gives every member target-correct
size and alignment without handwritten byte-layout logic. The private layout
may be compacted later without changing the language or ABI.

Construction shall begin from an LLVM zero initializer for the complete union,
then write the selected tag and active member. Inactive storage is never read,
but deterministic initialization prevents poison or undefined inactive fields
from being copied through aggregate values.

Tags are assigned deterministically in source order beginning at zero. Tags are
stored as LLVM `i32` values and are not exposed to source or Rust bridges. A
union with more than 2^32 direct members is rejected before lowering.

### 15.3 Places and mutation

Lowering distinguishes places from values. Mutable locals use addressable
storage. Field places use LLVM field addresses. Reading a place loads its
current value; assignment stores a checked value.

A method lowers to an internal function with a hidden first pointer parameter
for `self`. A receiver-writing call passes an existing mutable-rooted receiver
place. A read-only call may instead use compiler-owned temporary storage. This
pointer is a backend implementation detail and does not create a Snacc pointer
or reference value. The receiver evaluates exactly once.

### 15.4 Union operations

Union injection writes the member's deterministic tag and member storage. An
`is` test compares the stored tag. Its binding reads the matching member field
only along the successful control-flow edge. The checker guarantees that
lowering never reads an inactive member.

Union equality compares tags first and dispatches to equality for the active
member. It never reads inactive member storage.

## 16. Rust bridge and ABI

Represented, struct, member, and union types are not permitted in an `extern
rust` parameter or result under this specification. Their source-level
representation does not imply a stable C ABI representation.

Methods are not exported. Internal Snacc functions may accept and return all
types introduced here.

The user-defined types in this specification do not themselves change the
permitted Rust bridge type set. Specification 012 changes the ABI separately by
removing standalone `Nil`. A later specification for strings, buffers, or
user-defined bridge values shall define stable layouts, ownership, validity,
and versioning before permitting them across the boundary.

## 17. Diagnostics

A conforming implementation shall produce structured source diagnostics for at
least:

| Condition | Required information |
| --- | --- |
| Duplicate or unknown type | The conflicting or unresolved qualified name |
| Type/callable name conflict | The two declarations sharing a call head |
| Recursive value layout | The cycle of types that makes layout infinite |
| Duplicate field or union member | The containing type and duplicate name |
| Too many union members | The containing union and 32-bit tag limit |
| Missing, unknown, or duplicate constructor field | The constructed type and field |
| Positional non-empty struct construction | That named fields are required |
| Represented-type conversion mismatch | The required immediate type |
| Field access on a non-struct | The receiver type and field name |
| Unknown field or method | The receiver type and requested name |
| Method called as a value | That methods require a receiver call |
| `self` outside a method | That `self` is method-only |
| Invalid `is` relationship | The union and tested type |
| Type-test binding outside an `if` condition | The binding form restriction |
| Duplicate type-test member | The unreachable member branch |
| Value-form type-test chain missing a path | Every unhandled qualified member name |
| Exhaustive chain followed by `else` | That the `else` branch is unreachable |
| User-defined Rust bridge type | That only the ABI's permitted types may cross |

Name and declaration errors belong to declaration collection. Construction,
field, call, mutability, assignability, type-test, and exhaustiveness errors
belong to type checking. Infinite layout shall be rejected after name
resolution and before expression checking.

## 18. Compatibility

This is source-breaking for programs that use any new keyword as an identifier.
Those programs shall rename the identifier.

The optional-`else` grammar accepts statement-form `if` without `else`. A
value-form `if` still requires `else` unless an exhaustive union type-test chain
covers every path.

`.` gains field, method, and qualified-type meaning. No currently valid source
uses those forms. Specification 012 owns the source-breaking declaration and
assignment migration.

The checked `Ty` re-export from `snacc-compiler` is a Rust API consumed by
`apps/cargo-snacc`. Extending that type with user-defined identities is a
source-breaking API change for exhaustive downstream matches; all workspace
consumers shall migrate in the same change. User-defined types remain rejected
at Rust bridge declarations.

## 19. Detailed implementation plan

Specification 010 and the syntax/place portions of Specification 012 shall land
as one compiler migration after RFC 008 and Specification 009. Reference
parameters and final standalone-`Nil` removal then follow Specifications 011
and 012. No compatibility AST or duplicate type representation shall remain.

Primary implementation surfaces are the compiler syntax AST, lexer, and parser;
semantic type/declaration/checking modules; LLVM lowering; the public checked
type API in `crates/snacc-compiler/src/lib.rs`; bridge type rendering in
`apps/cargo-snacc/src/main.rs`; compiler bridge rejection; and the parse,
typecheck, phase, conformance, driver, and Cargo-hosted suites. This
specification adds no workbench-only feature.

### Phase 1: syntax and AST

1. Add tokens, display forms, recovery boundaries, and reserved-word handling
   for `type`, `is`, `struct`, `union`, `method`, `self`, `.`, and `|`; `mut`
   is added by Specification 012 only for local declarations.
2. Replace built-in-only syntax type names with a spanned qualified type path.
   Retain built-in keywords as path leaves only where the grammar permits.
3. Add explicit nodes for represented, struct, and union declarations; inline
   member structs; fields; methods; named constructor arguments; member access;
   method calls; and type tests.
4. Preserve declaration order, written argument order, every component span,
   and the optional `else`. Keep top-level declarations in source-order vectors
   rather than unordered maps. Parse `self` as its own node.
5. Parse construction, call, and member postfix syntax without deciding whether
   a name denotes a type, field, or method; resolution owns that decision.
6. Add parser recovery and round-trip shape tests for empty structs, qualified
   member methods, nested postfix chains, and every malformed delimiter.

### Phase 2: declaration collection and type resolution

1. Collect built-in and top-level type names in deterministic source order,
   reject duplicates, and allocate stable `TypeId` values before resolving
   bodies.
2. Allocate each union member a separate `TypeId` keyed by `(UnionId, name)`;
   never insert it into the top-level namespace.
3. Resolve represented targets, field types, function signatures, method
   receivers, and method signatures into IDs. Reject user types at bridge sites
   during this phase.
4. Build a by-value layout dependency graph and run a three-state depth-first
   traversal. Report the complete first cycle with spans before expression
   checking.
5. Reject callable/type call-head conflicts, then collect field and method
   lookup tables after type resolution, rejecting duplicate fields and
   receiver-local duplicate methods.

### Phase 3: checked values, places, and bodies

1. Extend RFC 008's `Ty` from built-in cases to built-in cases plus
   `User(TypeId)` identity, and retain `snacc_compiler::Ty` as its public name.
   Do not retain a parallel scalar-only checked type. Store all resolved
   definitions in the checked program.
2. Resolve every local and `self` access to a binding ID and every field path to
   field IDs. Emit explicit place loads rather than preserving ambiguous syntax.
3. Check constructors in written evaluation order, then store their values in
   declaration-field order. Insert explicit represented wrapping/unwrapping and
   direct union injection nodes only where this specification allows them.
4. Resolve call heads by section 6.1, then resolve methods by exact receiver
   type. Evaluate the receiver once and store the selected callable or method
   ID and checked explicit arguments.
5. Check type-test subjects as places, produce explicit tag-test and member-read
   nodes, enforce Specification 012's function-wide binding-name uniqueness,
   and store proven exhaustive chains.
6. Apply the deterministic common-type algorithm in section 13, including
   contextual `nil` from Specification 012. Lowering shall perform no name,
   conversion, or exhaustiveness analysis.
7. Update `apps/cargo-snacc` and every other exhaustive consumer of the public
   checked type. Its Rust ABI renderer shall handle every built-in bridge type
   and treat a user-defined type reaching it as an internal error because
   declaration collection rejects that source earlier.

### Phase 4: receiver-write effect and final validation

1. While checking methods, record a direct receiver write when an assignment or
   `Ref<T>` argument is rooted at `self`. Record an effect edge when a call is
   made on a receiver place rooted at `self`.
2. Solve the method call graph to the least fixed point: a method writes its
   receiver if it writes directly or reaches a receiver-writing method through
   a self-rooted receiver edge. Strongly connected components may be used, but
   the result shall be deterministic and independent of declaration order.
3. Validate every receiver-writing call against Specification 012's mutable
   root. Calls rooted at a `Ref<T>` parameter are mutable; temporaries,
   immutable locals, ordinary parameters, and type-test bindings are not.
4. Do not mark a method receiver-writing merely because it mutates an unrelated
   local or explicit `Ref<T>` parameter.
5. Validate equality support recursively and memoize by resolved type ID to
   avoid repeated traversal.
6. Contextually classify each `if` under RFC 008. Permit omitted `else` freely
   for statement form and only for proven exhaustive union chains in value form.

### Phase 5: LLVM types and lowering

1. Predeclare named LLVM types for every resolved user type, then set bodies in
   layout-dependency order before declaring functions.
2. Lower represented types to their immediate representation, structs in field
   order, and unions to `{i32 tag, member_0, ... member_n}` with deterministic
   source-order tags.
3. Zero-initialize a union aggregate before writing its tag and active member.
   Never read or compare inactive member fields.
4. Materialize addressable storage only for mutable or referenced roots and
   hidden receiver places. Lower checked field paths to GEPs and checked loads
   and assignments to exact typed operations.
5. Lower methods to internal functions with a hidden receiver pointer; no
   source-visible pointer type or method effect enters the ABI. Give every
   method deterministic internal linkage and a collision-free symbol derived
   from its resolved receiver ID and method ID; the spelling is not public ABI.
6. Lower constructors and injection without allocation. Lower exhaustive tests
   from stored tags/member IDs and emit recursive type-directed equality with
   tag-first union dispatch.
7. Verify each generated module before object emission and classify verifier
   failure as an internal compiler error.

### Phase 6: contract, corpus, and verification

1. Update formal EBNF first in `LANGUAGE.md`, then copy it identically to
   `GRAMMAR.ebnf`.
2. Add only the normative identity, construction, equality, method, `self`,
   union, `is`, exhaustiveness, and bridge rules to `LANGUAGE.md`.
3. Add parse and checker rejection cases for every diagnostic in section 17,
   execution cases for every representation, and integration cases with
   Specifications 008, 009, 011, and 012.
4. Search and update every exhaustive syntax/type match across parser, checker,
   bridge metadata, backend, tests, and diagnostic rendering.
5. Run formatting, workspace checking, LLVM verification, and the complete
   workspace test suite.

## 20. Conformance tests

A conforming implementation shall test at least:

1. Represented types are nominal and require explicit one-layer wrapping and
   unwrapping.
2. Same-representation nominal types are not assignable or comparable.
3. Struct construction accepts reordered named fields and a trailing comma.
4. Missing, duplicate, unknown, and positional struct fields are rejected.
5. Empty top-level and union-member structs construct with `()` and compare
   equal only to their own type.
6. Nested member names resolve only through their union namespace.
7. Struct, represented, and union equality follows section 7 through 9.
8. A union accepts each direct member and rejects unrelated or other-union
   members.
9. A union value retains and reads every member shape correctly.
10. Methods access `self` and return values; read-only methods work on variables
    and temporaries.
11. Field-writing and whole-`self`-replacing method calls require mutable roots;
    no method-level mutation marker exists.
12. `self` is rejected outside methods and cannot be declared as a local name.
13. Method lookup is exact, namespaced by receiver type, non-overloaded, and
    never dynamic.
14. Method receivers, constructor arguments, and call arguments evaluate once
    from left to right.
15. Specification 012's root-variable assignment rules apply to every struct
    field and receiver.
16. No-result method calls are rejected in value positions.
17. `is` without a binding supplies the correct `Bool` condition for every
    member tag.
18. `is` binding exposes the exact member type only in its successful branch.
19. Unrelated, always-true, and non-place type tests are rejected.
20. Exhaustive `if`/`elseif` works without `else` for empty and data-carrying
    members.
21. Missing and duplicate members, different tested places, and an unreachable
    `else` are rejected with the required diagnostics.
22. Adding a union member makes an old no-`else` chain non-exhaustive.
23. Direct and indirect recursive value layouts are rejected before lowering.
24. Every user-defined type is rejected in Rust bridge parameters and results.
25. Programs combining user-defined types with RFC 008 no-result functions and
    with Specification 009 scalar fields compile and run.
26. Type/callable call-head conflicts are rejected, and qualified constructor
    versus local-receiver resolution follows section 6.1.
27. Receiver-write effects propagate through direct, recursive, and mutually
    recursive method calls without marking unrelated local or `Ref<T>` writes.
28. Common union types are determined from expected types, union branches, and
    member branches; ambiguous or conflicting unions are rejected.
29. Union tags are deterministic `i32` values, excessive member counts are
    rejected, and injection initializes inactive aggregate storage without
    reading it.
30. `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and
    all workspace tests pass.

## 21. Acceptance criteria

1. The syntax and semantics in sections 4 through 14 are implemented without
   an enum, class, inheritance, pointer, or separate match construct.
2. Every user-defined type has nominal identity and finite, compiler-known
   value layout.
3. Union alternatives are namespaced types, including bare alternatives as
   empty struct types.
4. `self` is a keyword denoting the call-scoped receiver storage.
5. Method dispatch is static, there is no mutable-method category, and receiver
   field assignment follows Specification 012.
6. `if`/`elseif` plus `is` is the only union decomposition mechanism and is
   exhaustively checked when `else` is absent.
7. The checked program makes all type, place, member, method, and exhaustiveness
   facts explicit for lowering.
8. No new user-defined type crosses the Rust bridge, so this specification
   itself adds no ABI representation; surrounding specifications own their
   scalar, reference, and standalone-`Nil` version changes.
9. `LANGUAGE.md`, `GRAMMAR.ebnf`, the parser, checker, lowering, and implemented
   behavior agree.
10. Every conformance test in section 20 passes.

## 22. Non-normative rationale

One `type N is ...` declaration family makes nominal scalar wrappers, products,
and sums visibly related without treating them as aliases. A union member is a
real namespaced type, so an empty member supplies the closed-choice use case
often assigned to enums while preserving the same ADT model used by members
with data.

Qualified methods preserve static resolution and the forward-only compiler
pipeline. An implicit keyword receiver keeps call syntax concise. Receiver
mutation remains ordinary assignment governed by the receiver root; the
checker-computed effect is internal and adds no source-level method category.

Using `if`/`elseif` for tag tests avoids a second branching construct. Requiring
exhaustiveness when `else` is absent retains the principal safety property of
sum-type decomposition.

The initial union representation favors correctness and target-native alignment
over compactness. Because user-defined layouts do not cross the ABI, later
storage compaction is an implementation change rather than a language change.
