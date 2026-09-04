# Specification 012: Variable Declarations and Assignments

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Scope

This specification defines variable declaration, lexical visibility,
assignment, root-variable mutability, member assignment, parameter mutability,
and the permitted use of `Nil`.

It makes the following choices:

- `let` is a declaration statement, not an expression;
- every variable has an explicit type and initializer;
- a local name may be declared only once in a function or method;
- `let mut` permits replacement of a variable and assignment through any field
  path rooted at that variable;
- ordinary parameters are immutable by-value bindings;
- caller-owned storage is mutated only through `Ref<T>`; and
- `Nil` exists only as a member of another type.

This specification does not add inferred declarations, uninitialized
variables, shadowing, field-level `mut`, mutable-parameter syntax, compound
assignment, increment or decrement operators, destructuring declarations,
global variables, static storage, `Out<T>`, or runtime mutability checks.

This specification contains no open design questions. Its implementation order
and phase boundaries are fixed in section 15.

## 2. Normative references and dependencies

[The Snacc language contract](../../../LANGUAGE.md) is normative. This document
defines a change to that contract.

[RFC 008](008-statements-and-functions-without-results.md) establishes
statements and functions without results.

[Specification 010](010-nominal-types-structs-unions-and-methods.md) defines
structs, unions, methods, and type-test bindings. This specification replaces
its rules for declarations, assignment, and receiver mutation.

[Specification 011](011-call-scoped-reference-parameters.md) defines
call-scoped references. This specification defines which places may be passed
to `Ref<T>` parameters. Specification 011 already excludes standalone `Nil`
from its referent and bridge sets.

The coordinated landing order is: the shared RFC 008/Specification 012 block,
declaration, assignment, and scalar-place foundation; Specification 009;
Specification 010 with aggregate-place support; Specification 011; and finally
this specification's standalone-`Nil` removal. The resulting ABI versions are
2, 3, 4, and 5 respectively.

## 3. Terms

A **declaration statement** introduces one initialized variable.

A **variable** associates a name, declared type, storage location, value, and
mutability for part of a block.

**Reassignment** replaces the complete value of an existing variable.

A **place** is storage identified by a variable or by a field path rooted at a
variable or reference parameter.

A **mutable root** is a variable declared with `let mut` or the referent of a
`Ref<T>` parameter.

## 4. Lexical structure and statement boundaries

Snacc has no semicolon token. Whitespace, including line breaks, separates
tokens but has no grammatical meaning beyond that separation.

The parser determines the end of each construct from the grammar. It consumes
the longest expression permitted by the expression grammar, then begins a new
block element when the next token cannot continue that expression. Source may
place successive statements on separate lines or on one line when their token
boundaries remain unambiguous. These programs are equivalent:

~~~snacc
let x: Int64 = 10
print(x)
~~~

~~~snacc
let x: Int64 = 10 print(x)
~~~

`(` is a postfix-call continuation token. Consequently, after a block-element
expression, a following `(` always continues that expression even when a line
break intervenes. For example, `b` followed by `(c)` parses as `b(c)`, not as
two block elements. A parenthesized expression therefore cannot begin a new
block element immediately after another expression. This is an intentional
consequence of newline-insensitive maximal munch; the lexer does not insert a
boundary and the parser shall not warn or guess from layout.

After Specifications 008, 010, and 011, the relevant grammar is:

~~~ebnf
program              = { top-level-declaration | block-element } ;

top-level-declaration
                     = function-declaration
                     | method-declaration
                     | type-declaration
                     | rust-declaration ;

function-declaration = "fun", identifier, parameters, [ ":", type ], "do",
                       block, "end" ;
method-declaration   = "method", qualified-name, parameters, [ ":", type ],
                       "do", block, "end" ;

block                = { block-element } ;
block-element        = variable-declaration
                     | assignment
                     | while-statement
                     | break-statement
                     | if-form
                     | expression ;

variable-declaration = "let", [ "mut" ], identifier, ":", value-type,
                       "=", expression ;
assignment           = place, "=", expression ;
place                = ( identifier | "self" ), { ".", identifier } ;

while-statement      = "while", expression, "do", block, "end" ;
break-statement      = "break" ;
if-form              = "if", condition, "then", block,
                       { "elseif", condition, "then", block },
                       [ "else", block ], "end" ;

field-declaration    = identifier, ":", value-type, [ "," ] ;
~~~

The semicolons above are EBNF notation; they are not Snacc tokens.

The single `=` token begins assignment only after the parser recognizes a
place at block-element start. `==` remains an expression operator. Assignment
is not accepted on the right side of another assignment, so chained assignment
does not exist.

A declaration and an assignment are statements. They are accepted as block
elements and are not accepted where an expression is required, such as an
initializer operand, argument, condition, or returned expression.

When a block is required to produce a value, its last block element shall be an
expression assignable to the required type. A declaration or assignment cannot
supply that value. Earlier expression results are discarded. A no-result block
has no required final expression.

~~~snacc
fun square(value: Int64): Int64 do
    let result: Int64 = value * value
    result
end

fun announce(value: Int64) do
    print(value)
end
~~~

## 5. Variable declarations

### 5.1 Form and initialization

A declaration contains `let`, optional `mut`, a name, an explicit value type,
`=`, and an initializer:

~~~snacc
let count: Int64 = 0
let mut total: Int64 = 10
~~~

Both of these forms are invalid:

~~~snacc
let count = 0
let count: Int64
~~~

The initializer is evaluated exactly once. Its value shall be assignable to the
declared type. The name becomes available only after initialization completes
and remains available until the end of the block containing the declaration.

In plain terms, later code in the same block may use the variable; earlier code
and the initializer being evaluated may not.

~~~snacc
let first: Int64 = 10
let second: Int64 = first + 1
~~~

This form is always invalid:

~~~snacc
let count: Int64 = count + 1
~~~

If no earlier `count` exists, the initializer contains an unknown variable. If
an earlier `count` exists, the declaration duplicates that name. The
initializer never refers to the variable being created, and Snacc permits no
same-named earlier variable in the same function.

### 5.2 One declaration per name

Within one function or method, a local name may be declared only once. This
rule includes declarations in nested blocks, sibling branches, loop bodies,
parameters, and type-test bindings. `self` is reserved in methods and cannot be
declared.

The compiler's existing duplicate-parameter check is the completed baseline
for this rule. Implementation shall extend that check to the function-wide
binding set rather than retain a separate parameter-only uniqueness mechanism.

~~~snacc
let x: Int64 = 10
let x: Int64 = 20
~~~

The second declaration is a duplicate-name error. It does not create another
layer and does not replace the first variable.

The following is also invalid:

~~~snacc
let value: Int64 = 1
if ready() then
    let value: Int64 = 2
    print(value)
end
~~~

A declaration inside a nested block is still visible only from that
declaration to the end of that block. The whole-function uniqueness rule merely
prevents another declaration from reusing its name. Different functions may
use the same local names.

Top-level executable variables and type-test bindings shall have unique names
throughout the executable program body and remain unavailable inside functions
and methods. Snacc creates no implicit global state.

Locals, parameters, and type-test bindings share this function-local binding
namespace. Functions and Rust bridges share a callable namespace; types,
fields, constructors, and methods use the resolution rules in Specification
010. Because functions are not values, bare `name` resolves in the local
binding namespace, while call heads resolve as specified by Specification 010.

## 6. Reassignment and root mutability

`let` creates an immutable root. Neither that variable nor a member reached
through it may be assigned:

~~~snacc
let count: Int64 = 1
count = 2
~~~

`let mut` creates a mutable root. Assignment may replace its complete value:

~~~snacc
let mut count: Int64 = 1
count = count + 1
~~~

An assignment evaluates its right side exactly once and replaces the place
only after that evaluation succeeds. It preserves the variable's declared type
and storage identity.

`let` and `=` therefore have separate roles:

~~~snacc
let mut count: Int64 = 1
count = 2
~~~

The first statement creates `count`. The second changes the existing `count`.
A second declaration with the same name is always an error, regardless of
block nesting.

## 7. Struct members

Fields do not declare their own mutability:

~~~snacc
type Point is struct
    x: Dec64,
    y: Dec64,
end
~~~

Construction shall initialize every field exactly once. Field assignment is
controlled by the root variable:

~~~snacc
let point: Point = Point(x: 3.0, y: 4.0)
point = Point(x: 0.0, y: 0.0)  // error
point.x = 5.0                  // error
~~~

~~~snacc
let mut point: Point = Point(x: 3.0, y: 4.0)
point.x = 5.0                  // valid
point = Point(x: 0.0, y: 0.0) // valid
~~~

Root mutability applies through the complete field path:

~~~snacc
let mut entity: Entity = initial
entity.position.x = 1.0
~~~

No field-level `mut` exists, and mutability is not a property of a struct type.
Two variables of the same struct type may differ because one was declared
`mut` and the other was not.

Type-test bindings are immutable roots. To update the original union, construct
a replacement member and assign it to a mutable union variable or modify it
through `Ref<Union>`.

## 8. Function parameters and `Ref<T>`

An ordinary parameter is an immutable by-value binding. It cannot be
reassigned, and fields rooted at it cannot be assigned:

~~~snacc
fun translate(point: Point, dx: Dec64) do
    point.x = point.x + dx // error
end
~~~

There is no `mut` parameter syntax. A function that must change caller-owned
storage takes `Ref<T>`:

~~~snacc
fun translate(point: Ref<Point>, dx: Dec64, dy: Dec64) do
    point.x = point.x + dx
    point.y = point.y + dy
end

let mut point: Point = Point(x: 3.0, y: 4.0)
translate(point, 1.0, 2.0)
~~~

Within the callee, a `Ref<T>` parameter is a mutable root. Automatic
dereferencing permits the parameter to be read, assigned as a complete `T`, or
used as the root of field assignment. The reference itself cannot be rebound,
stored, returned, or used outside the call as specified by Specification 011.

Passing a local place to `Ref<T>` requires a mutable root:

~~~snacc
fun set(value: Ref<Int64>, replacement: Int64) do
    value = replacement
end

let mut count: Int64 = 0
set(count, 10)
~~~

`set` cannot be called with a variable declared without `mut`. A field place
such as `point.x` may be passed only when its root `point` is mutable.

## 9. Methods and `self`

There is no mutable-method syntax or separate source-level mutable-method
category. `self` is a keyword naming the original receiver place.

~~~snacc
method Point.length(): Dec64 do
    sqrt(self.x * self.x + self.y * self.y)
end

method Point.translate(dx: Dec64, dy: Dec64) do
    self.x = self.x + dx
    self.y = self.y + dy
end
~~~

Here `sqrt` denotes an ordinary separately declared Snacc function or Rust
bridge, not a new built-in.

A method that only reads `self` may be called on any receiver. A call whose
method may assign through `self` requires a mutable receiver root:

~~~snacc
let point: Point = Point(x: 3.0, y: 4.0)
print(point.length())
point.translate(1.0, 2.0) // error: point is immutable

let mut movable: Point = Point(x: 3.0, y: 4.0)
movable.translate(1.0, 2.0)
~~~

The checker shall record whether a method may write through `self`, including
writes performed by methods it calls, solely to validate call sites. This is
an internal effect fact, not a distinct kind of method, source annotation, or
overload dimension. Whole-`self` assignment is permitted and replaces the
caller receiver when the call has a mutable root:

~~~snacc
method Point.reset() do
    self = Point(x: 0.0, y: 0.0)
end
~~~

Explicit method parameters follow section 8. To mutate caller-owned storage
other than the receiver, the author uses `Ref<T>`.

## 10. `Nil` only as part of another type

`Nil` is not a standalone variable, parameter, result, field, represented type,
or reference type. These forms are invalid:

~~~snacc
let value: Nil = nil
fun consume(value: Nil) do end
fun produce(): Nil do nil end
fun update(value: Ref<Nil>) do end
type Empty is Nil
~~~

`Nil` may be one member of a union that also contains at least one non-Nil
member:

~~~snacc
type MaybeUser is union
    | User is struct
        id: UserId,
      end
    | Nil
end
~~~

The literal `nil` or `null` is valid only when one expected union type directly
contains `Nil`:

~~~snacc
let missing: MaybeUser = nil
let present: MaybeUser = MaybeUser.User(id: UserId(10))
~~~

Thus a variable is never initialized with the exact type `Nil`. It may be
initialized with `nil` only when its declared type is a larger union containing
the `Nil` member.

`is Nil` tests that member without a binding. `is Nil(name)` is invalid because
the member carries no value. A union shall contain `Nil` at most once and shall
not use `Nil` as its only member.

In equality, assignment, an argument, a return, construction, or a branch,
another expected or already-resolved operand may supply the containing union
type for `nil`. `nil == nil` has no such type and is invalid. `value == nil` is
valid only when `value` has one union type that directly contains `Nil`.

## 11. Checked representation and lowering

The syntax tree shall represent blocks as ordered elements and shall represent
declarations and assignments as statements. A declaration shall not contain
its remaining scope as a nested body.

The checker shall:

- maintain lexical visibility by block;
- maintain one reserved-name set for the entire function or method;
- check an initializer before making its variable visible;
- reject every reuse of a reserved local name;
- record variable-root mutability once;
- resolve every assignment and `Ref<T>` argument to a root and field path;
- require that root to be mutable;
- treat every `Ref<T>` parameter referent as a mutable root; and
- compute the transitive receiver-write fact required by section 9.

Immutable scalar locals may remain SSA values when their address is not needed.
Mutable variables and places passed through `Ref<T>` use addressable storage.
Assignment lowers to a store after complete right-side evaluation.

Root mutability affects checking only. It changes neither struct layout nor
runtime representation. Copies have independent storage.

The `Nil` member receives an ordinary deterministic union tag and no payload.
There is no standalone `Nil` local or function value after checking.

## 12. Rust bridge and ABI

Standalone `Nil` parameters and results are removed from `extern rust`.
Specification 011 already prohibits `Ref<Nil>`. User-defined unions remain
unavailable at the bridge boundary until a stable-layout specification permits
them.

RFC 008 and Specifications 009 and 011 establish ABI versions 2, 3, and 4.
Implementing this specification shall establish ABI version 5 because it
removes standalone `Nil` and the required `snacc_print_nil` runtime import.

Compiler, runtime, Cargo-hosted assertions, generated cache identity, packaging
tests, and version-mismatch tests shall change together. No compatibility path
accepts an ABI version 4 object with a version 5 runtime.

## 13. Required diagnostics

A conforming implementation shall produce structured diagnostics for at least:

| Condition | Required information |
| --- | --- |
| Missing declaration type | Every variable type is explicit |
| Missing initializer | Declaration and initialization are inseparable |
| Duplicate local name | The original and duplicate declarations |
| Assignment through immutable root | The root and its declaration or parameter |
| Assignment type mismatch | The place type and supplied value type |
| Declaration or assignment in expression position | That the construct is a statement |
| `mut` on a parameter or field | That only `let` variable declarations use `mut` |
| Invalid `Ref<T>` argument | That the argument requires a mutable root |
| Receiver-writing call on immutable root | The receiver and method call |
| Standalone `Nil` type | `Nil` is permitted only as a union member |
| `nil` without one expected Nil-containing union | The missing or ambiguous context |
| Duplicate `Nil` union member | The containing union |
| Union containing only `Nil` | That `Nil` requires another member type |
| Binding `is Nil(name)` | `Nil` carries no value |

## 14. Compatibility and migration

This specification is source-breaking:

- `let` becomes a declaration statement;
- semicolons are not Snacc syntax;
- any reuse of a local name in one function or method becomes invalid;
- assignment to a variable or any of its fields requires a `let mut` root;
- field-level `mut` is removed;
- ordinary parameters and their members are immutable;
- caller mutation uses `Ref<T>`; and
- standalone `Nil` becomes invalid.

Existing code shall rename duplicate locals rather than introduce nested name
layers. Existing field mutations shall make the owning root variable mutable.
Existing by-value functions that intend to mutate caller storage shall accept a
`Ref<T>` parameter.

## 15. Detailed implementation plan

The block, declaration, place, and method-call portions shall land with
Specification 010 after RFC 008 and Specification 009. Reference integration
then lands with Specification 011. Standalone-`Nil` removal is the final phase
and establishes ABI version 5.

Primary implementation surfaces are compiler syntax, declaration and place
checking, checked blocks/statements, method-effect validation, LLVM storage and
assignment lowering, compiler bridge metadata, Cargo assertion/cache handling,
runtime imports, and every language, conformance, driver, runtime, and
Cargo-hosted suite.

### Phase 1: lexer, blocks, and syntax statements

1. Remove `;` from control tokens, parser recovery, sequence parsing, and every
   syntax test. A source semicolon shall produce a focused unsupported-token
   diagnostic.
2. Introduce a spanned `Block` containing ordered block elements. Replace
   expression-form `let` and sequence nodes with declaration and assignment
   statements shared with RFC 008.
3. Parse one block element at a time until the enclosing `end`, `elseif`, or
   `else`. Let the expression parser consume maximally; never inspect newlines
   to determine a boundary. Test that `b` followed by `(c)` on another line is
   the single call `b(c)` and document that a parenthesized expression cannot
   start a distinct element there.
4. Parse assignment only from a place followed by the single `=` token at block
   element start. Keep `==` in comparison parsing and reject chained assignment.
5. Add `mut` only after `let`. Preserve spans for `let`, `mut`, name, type,
   initializer, complete place, each field segment, `=`, and right side.
6. Add parser tests proving equivalent one-line and multiline programs, maximal
   call/postfix consumption, semicolon rejection, and declaration/assignment
   rejection inside expression grammar.

### Phase 2: binding collection and lexical checking

1. Give each function, method, and top-level executable body one reserved local
   name set. Insert parameters first; reserve each local and type-test name when
   its declaration is encountered, regardless of nested or sibling block.
2. Maintain separate block visibility maps keyed by binding ID. Check an
   initializer before adding its new binding to the active map, then leave it
   visible through the end of that block.
3. Reject duplicate names against the region-wide reserved set and report both
   declaration spans. Do not push an older same-named binding or implement
   restoration after a nested block.
4. Keep callable, type, field, and method namespaces separate as stated in
   sections 5.2 and 6 of this and Specification 010.
5. Require an explicit value type and initializer and insert only the permitted
   explicit conversion after initializer checking.

### Phase 3: checked places and root mutability

1. Assign each local and parameter a stable binding ID, declared value type,
   root kind, and mutability. Represent a place as a root plus resolved field-ID
   path.
2. Resolve every read through an explicit place-load checked node. Resolve every
   assignment before checking its right-side conversion.
3. Mark only variables declared by `let mut` and `Ref<T>` referents as mutable
   roots. Variables declared by plain `let`, ordinary parameters, type-test
   bindings, and temporaries are immutable roots.
4. Permit complete or nested-field assignment exactly when the root is mutable;
   never consult the struct definition for mutability.
5. Evaluate the complete right side before emitting the checked store and
   retain the root's declared type and storage identity.
6. Reuse the identical place and root capability for Specification 011
   reference arguments.

### Phase 4: methods and receiver effects

1. Treat assignments, reference passing, and receiver-writing calls rooted at
   `self` as receiver effects while checking method bodies.
2. Use Specification 010's least-fixed-point method effect analysis, then
   validate every receiver-writing call against its root capability.
3. Treat `self` as writable within a method solely to discover and check the
   method body; caller permission is enforced at each resolved call site.
4. Accept whole-`self` and field assignment through `self` and record the effect
   without adding source syntax or an overload distinction.

### Phase 5: lowering declarations and assignments

1. Lower immutable, unreferenced scalar locals as SSA values when convenient.
   Give mutable, aggregate, referenced, and receiver roots stable addressable
   storage.
2. Lower blocks in source order. Discard non-final expression values and use
   only a value-required block's final checked expression as its result.
3. Lower checked field paths to exact GEPs and emit a store only after complete
   right-side lowering.
4. Preserve value-copy semantics for structs and unions; mutation of one copy
   shall not affect another.

### Phase 6: contextual `Nil` and ABI 5

1. Remove standalone `Nil` from value-type resolution while retaining `Nil` as
   a reserved special union-member spelling and `nil`/`null` as contextual
   literals. Reject duplicate Nil members and unions with no non-Nil member.
2. Resolve each nil literal from one expected or already-determined containing
   union. Handle declaration, assignment, argument, return, construction,
   branch common type, and equality contexts; reject absent or conflicting
   context.
3. Give expression checking an optional expected value type for these
   contextual sites. If equality or branch order encounters `nil` before a
   determining operand, retain a local unresolved-nil constraint only until the
   complete expression is checked; no unresolved Nil node may enter the checked
   program.
4. Lower the Nil member as its deterministic tag with no payload. Remove all
   standalone Nil checked and LLVM values.
5. Remove the standalone `Nil` bridge mapping, `snacc_print_nil`, force-link
   retention, and standalone print lowering. Preserve Specification 011's
   existing rejection of `Ref<Nil>`; there is no such mapping to remove.
6. Advance compiler and runtime ABI constants from 4 to 5, update cache
   identity and generated assertions, and reject ABI 4↔5 combinations.

### Phase 7: contract, migration, and verification

1. Update formal EBNF first in `LANGUAGE.md`, then copy it identically to
   `GRAMMAR.ebnf`.
2. Add only the normative declaration, visibility, uniqueness, assignment,
   root mutability, parameter, method-call, and Nil rules to `LANGUAGE.md`.
3. Migrate every semicolon, expression-form binding, duplicate local, loop
   value, and standalone Nil use in examples, corpus, fixtures, and workbench
   snippets. Update expected diagnostics and output.
4. Add focused parser, checker, LLVM, direct-driver, Cargo-hosted bridge, ABI
   mismatch, cache, and packaging tests.
5. Run formatting, workspace checking, and the complete workspace test suite.

## 16. Conformance tests

A conforming implementation shall test at least:

1. Declarations require explicit types and initializers.
2. An initializer executes exactly once before its variable becomes visible.
3. A variable is visible from its declaration to the end of its block only.
4. Duplicate names are rejected in the same block, nested blocks, sibling
   branches, loop bodies, parameters, and type-test bindings.
5. Different functions may reuse local names.
6. Immutable-root reassignment fails and mutable-root reassignment succeeds.
7. Field and nested-field assignment follow root mutability only.
8. Field-level and parameter-level `mut` syntax are rejected.
9. Declarations and assignments are accepted as statements and rejected in
   expression positions.
10. Programs require no semicolons and line breaks do not terminate statements.
11. Ordinary parameters and their fields cannot be assigned.
12. `Ref<T>` referents may be replaced or field-mutated automatically.
13. `Ref<T>` arguments require mutable roots and retain Specification 011's
    overlap and escape restrictions.
14. Read-only methods accept immutable receivers; receiver-writing calls require
    mutable roots, including whole-`self` replacement and transitive method
    calls.
15. Struct copies mutate independently.
16. Standalone `Nil` is rejected in every type and bridge position.
17. Nil-containing unions accept contextual `nil` and `null`; context-free,
    ambiguous, duplicate, Nil-only, and binding forms are rejected.
18. ABI version 5 removes standalone Nil and `snacc_print_nil`, rejects version
    4 combinations, and invalidates version 4 caches.
19. Formatting, workspace checking, and all workspace tests pass.

## 17. Acceptance criteria

1. `let` is only an initialized declaration statement.
2. A name is declared at most once per function or method; shadowing does not
   exist.
3. Snacc source contains no semicolon syntax and does not assign meaning to line
   breaks.
4. `let mut` controls both complete-variable and member assignment for every
   place rooted at that variable.
5. Ordinary parameters are immutable; caller-owned storage is changed through
   `Ref<T>`.
6. Methods have no source-level mutable-method category; calls that write
   through `self` require mutable receiver roots.
7. `Nil` exists only as a member of another union type.
8. Declarations, assignments, places, root mutability, and receiver-write facts
   are explicit before lowering.
9. `LANGUAGE.md`, `GRAMMAR.ebnf`, compiler phases, and implemented behavior are
   synchronized.

## 18. Non-normative rationale

`let` marks the only operation that creates a local name. `=` changes storage
that already exists. Rejecting duplicate names removes shadowing and makes each
name identify one variable throughout a function or method.

One `mut` on the owning variable grants one easy-to-read capability: code may
replace that value or any member within it. The type definition describes data
shape, while each variable declaration controls whether its storage may change.

Ordinary parameters are values owned by the callee and remain immutable.
`Ref<T>` makes mutation of caller-owned storage explicit at the function
boundary without exposing references as general language values.

Required initialization eliminates uninitialized reads and definite-assignment
analysis. Restricting `Nil` to unions makes absence part of an explicit larger
type rather than an independently usable value type.
