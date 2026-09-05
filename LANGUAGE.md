# Snacc Language

## Grammar

```ebnf
program              = { top-level-declaration | block-element } ;

top-level-declaration
                     = function-declaration
                     | rust-declaration
                     | type-declaration
                     | method-declaration
                     | static-declaration ;

function-declaration = "fun", identifier, [ generic-parameters ], parameters, [ ":", type ], "do",
                       block, "end" ;
rust-declaration     = "extern", "rust", string-literal, "fun", identifier,
                       parameters, [ ":", type ] ;
method-declaration   = "method", qualified-name, parameters, [ ":", type ],
                       "do", block, "end" ;
static-declaration   = "static", associated-name, parameters, [ ":", type ],
                       "do", block, "end" ;
associated-name      = ( builtin-value-type, ".", identifier )
                     | identifier, ".", identifier
                     | identifier, ".", identifier, ".", identifier ;
parameters           = "(", [ parameter, { ",", parameter }, [ "," ] ], ")" ;
parameter            = identifier, ":", type ;

type-declaration     = "type", identifier, [ generic-parameters ], "is", type-body ;
type-body            = type
                     | struct-body
                     | union-body ;
struct-body          = "struct", { field-declaration }, "end" ;
field-declaration    = identifier, ":", type, [ "," ] ;
union-body           = "union", union-member, { union-member }, "end" ;
union-member         = "|", ( "Nil"
                     | identifier, [ "is", struct-body ] ) ;

qualified-name       = identifier, { ".", identifier } ;
type                 = reference-parameter-type | sum-type ;
reference-parameter-type
                     = "Ref", "<", sum-type, ">" ;
sum-type             = primary-value-type, { "|", primary-value-type } ;
primary-value-type   = builtin-value-type
                     | qualified-name
                     | type-application
                     | parameterized-value-type
                     | "(", sum-type, ")" ;
generic-parameters   = "<", identifier, { ",", identifier }, ">" ;
type-arguments       = "<", type, { ",", type }, ">" ;
type-application     = identifier, type-arguments ;
parameterized-value-type
                     = "Box", "<", sum-type, ">"
                     | "View", "<", sum-type, ">"
                     | "Array", "<", sum-type, ",", integer-literal, ">"
                     | "List", "<", sum-type, ">"
                     | "Map", "<", sum-type, ",", sum-type, ">"
                     | "Set", "<", sum-type, ">" ;
unsigned-type        = "Byte" | "UInt16" | "UInt32" | "UInt64" ;
builtin-value-type   = "Int64" | "Float64" | "Bool" | "Nil"
                     | "String" | "Unicode" | unsigned-type
                     | "Float32" ;

block                = { block-element } ;
block-element        = variable-declaration
                     | assignment
                     | indexed-assignment
                     | while-statement
                     | for-statement
                     | break-statement
                     | defer-statement
                     | defer-on-error-statement
                     | return-statement
                     | if-form
                     | expression ;

variable-declaration = "let", [ "mut" ], identifier, ":", type,
                       "=", expression ;
place                = ( identifier | "self" ), { ".", identifier } ;
assignment           = place, "=", expression ;
indexed-assignment   = expression, "[", expression, "]", "=", expression ;

while-statement      = "while", expression, "do", block, "end" ;
for-statement        = "for", identifier, [ ",", identifier ], "in",
                       expression, "do", block, "end" ;
break-statement      = "break" ;
return-statement     = "return", [ expression ] ;
defer-statement      = "defer", postfix ;
defer-on-error-statement
                     = "defer_on_error", postfix ;
if-form              = "if", condition, "then", block,
                       { "elseif", condition, "then", block },
                       [ "else", block ], "end" ;
condition            = type-test | expression ;
type-test            = place, "is", member-path, [ "(", identifier, ")" ] ;
member-path          = member-segment, { ".", member-segment } ;
member-segment       = identifier | builtin-value-type ;

expression           = logical-or ;
logical-or           = logical-and, { "or", logical-and } ;
logical-and          = comparison, { "and", comparison } ;
comparison           = additive, [ comparison-operator, additive ] ;
comparison-operator  = "==" | "!=" | "<" | "<=" | ">" | ">=" ;
additive             = multiplicative, { ( "+" | "-" ), multiplicative } ;
multiplicative       = unary, { ( "*" | "/" ), unary } ;
unary                = "!", unary | "return_on_error", postfix | postfix ;
postfix              = atom, { generic-call-suffix | arguments | member-suffix | index-suffix } ;
generic-call-suffix  = type-arguments, arguments ;
member-suffix        = ".", identifier, [ arguments ] ;
index-suffix         = "[", expression, "]" ;
arguments            = "(", [ argument, { ",", argument }, [ "," ] ], ")" ;
argument             = [ identifier, ":" ], expression ;

atom                 = literal
                     | identifier
                     | "self"
                     | builtin-value-type
                     | list-literal
                     | map-constructor
                     | set-constructor
                     | box-expression
                     | print-expression
                     | "(", expression, ")" ;
box-expression       = "box", "(", expression, ")" ;
print-expression     = "print", "(", expression, ")" ;
list-literal         = "[", [ expression, { ",", expression }, [ "," ] ], "]" ;
map-constructor      = "Map", "<", sum-type, ",", sum-type, ">", "(", ")" ;
set-constructor      = "Set", "<", sum-type, ">", "(", ")" ;
literal              = float32-literal | float64-literal | integer-literal
                     | boolean-literal | nil-literal | string-literal
                     | unicode-literal ;
boolean-literal      = "true" | "false" ;
nil-literal          = "nil" ;

integer-literal      = integer-magnitude, [ unsigned-suffix ] ;
integer-magnitude    = decimal-digits
                     | "0b", binary-digits
                     | "0o", octal-digits
                     | "0x", hexadecimal-digits ;

float64-literal      = decimal-digits, ".", decimal-digits, [ exponent ]
                     | decimal-digits, exponent ;
float32-literal      = decimal-digits, [ ".", decimal-digits ],
                       [ exponent ], "f32" ;
exponent             = "e", [ "+" | "-" ], decimal-digits ;

unsigned-suffix      = "u8" | "u16" | "u32" | "u64" ;
decimal-digits       = decimal-digit, { [ "_" ], decimal-digit } ;
binary-digits        = binary-digit, { [ "_" ], binary-digit } ;
octal-digits         = octal-digit, { [ "_" ], octal-digit } ;
hexadecimal-digits   = hexadecimal-digit,
                       { [ "_" ], hexadecimal-digit } ;
binary-digit         = "0" | "1" ;
octal-digit          = "0" | "1" | "2" | "3"
                     | "4" | "5" | "6" | "7" ;
decimal-digit        = octal-digit | "8" | "9" ;
hexadecimal-digit    = decimal-digit
                     | "a" | "b" | "c" | "d" | "e" | "f"
                     | "A" | "B" | "C" | "D" | "E" | "F" ;
string-literal       = interpreted-string-literal | raw-string-literal ;
interpreted-string-literal
                     = '"', { string-character | interpolation }, '"' ;
interpolation        = "{{", interpolation-expression, "}}" ;
raw-string-literal  = 'r', { "#" }, '"', { raw-character }, '"', { "#" } ;
unicode-literal      = "'", unicode-character, "'" ;
identifier           = identifier-start, { identifier-continue } ;
identifier-start     = ASCII-letter | "_" ;
identifier-continue  = identifier-start | decimal-digit ;

(* ASCII-letter is A-Z or a-z. string-character is any character except '"'
   or an unescaped line break; interpolation-expression is the balanced token
   sequence accepted by expression. It is lexed once and parsed by the normal
   expression grammar.
   raw-character is any source scalar; the opening and closing raw-string
   delimiters contain the same number of '#' characters. String escapes are
   decoded by the lexer only in interpreted strings; raw strings preserve
   their contents. unicode-character is one scalar
   or a supported escaped scalar. 
   Whitespace, including line breaks, separates tokens but has no other
   grammatical meaning. A comment begins with // and ends immediately before
   the next line feed or at end of input. Keywords are reserved and cannot be
   identifiers. The semicolons above terminate EBNF productions; Snacc has no
   semicolon token. Numeric tokens use maximal munch: a digit-led token
   immediately followed by an ASCII letter, digit, or underscore that does not
   complete one of the literal forms above is one invalid token, not a valid
   number followed by an identifier; thus `1u8`, `1.0f32`, `0x2A`, and `1e6`
   are valid, while `1u9`, `1u8x`, `0X2A`, `1E6`, and `1.0u8` are lexical
   errors. A radix prefix (`0b`/`0o`/`0x`) and the exponent marker `e` are
   lowercase; hexadecimal digits may use either case. `_` may appear only
   between two digits of one `decimal-digits`/`binary-digits`/`octal-digits`/
   `hexadecimal-digits` run, never leading, trailing, doubled, or adjacent to a
   prefix, decimal point, exponent marker or sign, or suffix; it is discarded
   before the literal's value is computed and never changes its type, value,
   or range. A `builtin-value-type`
   is an atom only as a call head, where it removes one represented-type layer
   (`Int64(id)`); a type name alone is never a value. The grammar admits a named
   type wherever a `type` appears, including a Rust bridge signature; that a
   user-defined type may not cross the bridge is a semantic rule, diagnosed by
   the checker. The grammar likewise admits `Nil` wherever a
   `builtin-value-type` appears; that `Nil` is spelled as a value type only as
   a `union-member` or a `type-test` is a semantic rule, also diagnosed by the
   checker. A `reference-parameter-type` is permitted only as the direct
   declared type of a `parameter`; it is rejected in every other `type`
   position, and `Ref<Ref<T>>` is never permitted. The `<` and `>` around a
   referent are type delimiters, not ordered comparisons. A `sum-type` with
   only one `primary-value-type` and no `|` collapses to that primary
   directly rather than a one-member sum. The grammar admits a `sum-type`
   wherever a `type` appears, including a represented type's body and a Rust
   bridge signature; that an inline sum cannot be a represented type's
   immediate representation, cannot cross a Rust bridge, and requires at
   least two distinct members (with `Nil` only alongside a non-`Nil` member)
   are semantic rules, also diagnosed by the checker. A `member-path` in a
   `type-test` may mix `identifier` and `builtin-value-type` segments; that it
   names exactly one direct member of the tested union or inline sum is a
   semantic rule as well. A `return-statement`'s optional expression is bare
   only when the next token closes the current block (`end`, `elseif`, or
   `else`) or ends the program; otherwise the following tokens must begin one
   expression. This makes bare and valued `return` unambiguous without a
   semicolon or significant newline, and a bare `return` can never be
   followed by another element of the same block. *)
```

Interpreted string and Unicode literals support `\\0`, `\\n`, `\\r`, `\\t`,
`\\\\`, escaped quotes/braces, and `\\u{H...}` Unicode scalar escapes.
Malformed escapes and invalid Unicode scalars are lexical errors. Raw strings
use Rust-style `r"..."`, `r#"..."#`, and matching hash delimiters; they
preserve their contents, may span lines, and do not process escapes.
Strings are owned UTF-8 values and are move-only; `clone` and `concat` produce
new strings.

The keywords `fun`, `extern`, `rust`, `let`, `mut`, `print`, `if`, `then`,
`elseif`, `else`, `while`, `for`, `in`, `do`, `break`, `return`, `end`, `and`, `or`, `true`, `false`,
`nil`, `type`, `is`, `struct`, `union`, `method`, `static`, `self`, `Int64`,
`Float64`, `Bool`, `Nil`, `String`, `Unicode`, `Byte`, `UInt16`, `UInt32`, `UInt64`, `Float32`, and
`Ref` are reserved and cannot be used as identifiers.
Operators of the same precedence associate left to right. Calls, field access,
and method calls bind most tightly, followed by unary `!`, multiplication and
division, addition and subtraction, one comparison, `and`, and `or`.
`return_on_error` is a prefix operation on a postfix call. Comparisons do not
chain; parentheses are required to combine comparison results.

## Program structure

A program is a sequence of top-level declarations (functions, Rust bridge
declarations, type declarations, method declarations, and static associated
function declarations) interleaved with
top-level block elements, in any order. Top-level block elements execute in
source order; a top-level expression's value is discarded. A successful program
entry returns process status zero.

Function and bridge names share one namespace and must be unique. Each
external link symbol must also be unique. Parameter names within one
declaration must be unique. Declarations are visible throughout the program,
independent of source order, so forward calls and recursion are valid.

Type names occupy a namespace separate from local bindings, and a top-level
type name must be unique among built-in and user-defined type names. Because a
bare type constructor and a function call are both written `name(...)`, a
top-level type name must not duplicate a function or bridge name.

Functions, methods, and static associated functions are top-level only. A
static declaration is written `static Type.name(...) do ... end`; its type
provides a namespace but no implicit receiver or `self`. Associated function
names are unique within that type, may be declared on a predeclared concrete
type, a user-defined top-level type, or a named union member, and are called as
`Type.name(...)`. Generic associated functions and associated functions on a
generic type application are not supported.

A function can read its parameters
and lexical `let` bindings, but not top-level block values or another
function's locals. Functions and methods are not values: a call target must be
a declared function, bridge, type, or a method reached through a receiver.

## Types and values

`Int64` is a signed 64-bit integer. An integer literal has type `Int64` unless
an unsigned suffix selects otherwise, and its mathematical value must be
representable by that type. A literal's magnitude may be written in decimal,
or with a lowercase `0b`, `0o`, or `0x` prefix for binary, octal, or
hexadecimal (hexadecimal digits may be either case); the radix is notation
only and never affects the literal's type. Because Snacc has no unary minus, a
negative value is formed by subtraction.

`Float64` is an IEEE 754 binary64 value. A decimal-point or scientific-notation
literal (`1.5`, `1e6`, `1.25e-3`) with no suffix has type `Float64`; `Float32`
also accepts scientific notation, via its existing `f32` suffix. Each literal
converts once, directly from its decimal source text to its target width,
using round-to-nearest, ties-to-even; a value that rounds to infinity is
rejected. A `_` may appear between two digits of one magnitude, fraction, or
exponent to aid readability and is otherwise semantically inert.

`Bool` contains `true` and `false`.

`Nil` is not a standalone type. It is spelled only as a union member or as one
member of an inline sum type (see "Inline sum types"), and it is rejected as a
variable, parameter, function, method or bridge result, field, represented
type, or `Ref<T>` referent on its own. `nil` is the sole spelling of that
member and has no type of its own: it is valid only where exactly one expected
union type or inline sum directly contains `Nil`. `null` is an ordinary
identifier with no built-in meaning.
`print(nil)` and `nil == nil` supply no such type and are rejected.

`Byte`, `UInt16`, `UInt32`, and `UInt64` are unsigned integers of the named
bit width, holding integers from 0 through 2^N - 1. An unsigned literal has
the type selected by its suffix (`u8`, `u16`, `u32`, or `u64`); its
mathematical value must fit that type, or the literal is rejected without
truncation or wrapping (`256u8` and `18446744073709551616u64` are invalid).

`Float32` is an IEEE 754 binary32 value. A literal ending in `f32` has type
`Float32`, with or without a decimal point or exponent; `1f32`, `1.0f32`, and
`1e0f32` are all valid. The decimal source value is rounded once, directly to
the nearest binary32 value, using round-to-nearest, ties-to-even -- never
through an intermediate `Float64`. A literal that would round to infinity is
rejected as out of range; infinity and NaN have no literal spelling.

All five of these types are scalar copy types: copying, binding, passing, and
returning a value duplicates its bits and transfers no resource ownership.

Strings are owned UTF-8 values and are move-only. `bytes()` and `unicode()`
produce immutable views; views support length, checked lookup, and half-open
slicing. Interpreted strings may contain `{{expression}}` interpolation; each
expression is checked and formatted using the same closed set of scalar and
text types accepted by concatenation, in left-to-right order. Raw strings do
not interpolate. A view is a non-owning borrow of a named `String` place: its
source cannot be moved, reassigned, boxed, returned, or stored while the view
is live, and the compiler ends the borrow at the last reachable local use.
`String.clone()`, `String.concat(part)`,
`String.from_unicode(View<Unicode>)`, and checked
`String.from_utf8(View<Byte>)` are built-in operations. `Box<T>` and
`box(value)` are implemented closed built-ins; they are not general
user-defined generic syntax. `concat` accepts exactly `String`,
`View<Unicode>`, `Unicode`, `Byte`, `Int64`, `UInt16`, `UInt32`, `UInt64`,
`Float32`, `Float64`, and `Bool`; a maximal chain and interpolation use one
final String allocation and evaluate every part once from left to right.

Generic functions and generic struct types use explicit angle-bracket type
arguments and compile-time monomorphization. For example,
`fun identity<T>(value: T): T do value end` is called as
`identity<Int64>(42)`, and `Pair<Int64, Bool>` is a concrete nominal struct
type. Type arguments are never inferred. Generic parameters have no implicit
capabilities: they may be passed, returned, stored, and used in other type
applications, but arithmetic, comparison, equality, printing, literals, and
type tests require a concrete type that supports the operation. Generic
methods, unions, represented types, Rust bridges, and qualified generic calls
are not part of the language. Reachable specializations are private concrete
functions/types; the compiler rejects more than 128 active specialization
edges or 4,096 unique specializations in one compilation.

`Array<T, N>`, `List<T>`, `View<T>`, `Map<K, V>`, and `Set<T>` are closed
collection forms. Array and list literals require an expected array or list
type. Arrays and lists support `length()`, `is_empty()`, indexing, and
sequence `for` iteration; lists also expose `capacity()`, `push`, `pop`,
`insert`, `remove`, `clear`, and `reserve` for storable element types, and
`view()` lends an immutable view.
Same-typed arrays, lists, and generic views support element-wise `==` and `!=`
when their element type supports equality.
Empty maps and sets can be constructed with their complete type application.
The current collection runtime supports maps with `Byte`, `UInt16`, `UInt32`,
`UInt64`, `Int64`, `Bool`, `Unicode`, or `String` keys; sets support the same
key/element types. Map values may be any storable, non-borrowed type. Maps
provide `insert`, `contains`, checked indexing, indexed assignment for an
existing key, `delete`, `take`, `clear`, and `reserve`; sets provide `insert`,
`contains`, `delete`, `clear`, and `reserve`. String queries borrow a named
`String` as a byte view, while inserting a `String` transfers ownership into
the collection. Collection runtimes store non-scalar values as opaque bytes;
the compiler retains responsibility for typed loads, moves, and destruction.

Every parameter, function result, method result, static associated-function
result, field, and local binding has
an explicit type. There are exactly two implicit conversions: `Int64` to
`Float64`, and a direct union member to its containing union. Both apply to
bindings, arguments, results, and branches; the numeric one also applies to
operands. No conversion to or from
`Bool` exists. `Byte`, `UInt16`, `UInt32`, `UInt64`, and `Float32`
participate in no implicit conversion at all: not to or from `Int64`, not to
or from `Float64`, not to or from each other, and not to or from
`Bool`. Every declaration, assignment, argument, result, and branch involving
one of these five types requires an exact type match; `let byte: Byte = 1`
is a type error, not a contextual reinterpretation of the literal.

No result is not a type. A function, method, static associated function, or
Rust bridge that omits its
result type produces no result: it cannot be written, stored, passed,
returned, or compared. It is not a value of any kind, and in particular it is
not a union's `Nil` member.

## User-defined types

One declaration family, `type Name is ...`, introduces nominal represented
types, structs, and unions. Every user-defined type has nominal identity and a
finite, compiler-known value layout. A represented type, struct field, or union
member must not contain itself directly or through a cycle. `Box<T>` is the
first explicit indirection facility: it stores one uniquely owned heap
allocation containing a `T`, so recursive layouts may cross a box edge. There
is no general pointer, nullable box, enum, class, inheritance, or separate
`match` construct.

`print` does not accept a represented, struct, member, or union value. A
program prints their scalar fields or explicitly unwraps a represented scalar.
No user-defined type has an implicit zero value.

### Box indirection

`Box<T>` is a closed built-in value type with exactly one storable value type
argument. It owns one non-null, uniquely owned heap allocation containing a
`T`; `Box<Box<T>>` is valid, while `Box<Ref<T>>` and no-result values are not.
The type has pointer-sized, pointer-aligned storage independent of `T` and is
move-only even when `T` is copyable. A struct, union, or inline sum containing
a box is move-only transitively.

`box(expression)` evaluates its operand once, allocates storage sized and
aligned for the checked operand type, stores the value, and produces
`Box<T>`. Allocation failure terminates through the runtime fatal path; a box
is never represented by `nil`. Field access and method calls automatically
dereference box layers needed to resolve the selected member, but do not copy or
consume the box.

Using a box or a value transitively containing one in a consuming context
(initialization, assignment source, by-value argument, return, or aggregate
construction) transfers ownership and makes the source unavailable. A
move-only field or automatic dereference cannot be moved out as a subplace.
An available move-only destination is destroyed before replacement, and all
remaining owners are destroyed exactly once on normal scope exit. Root
mutability controls both box replacement and mutation through its pointee.

`Box<T>` and types containing one cannot cross an `extern rust` bridge. Boxes
do not support direct equality or printing, shared ownership, cloning, raw
pointers, or implicit nullable behavior. Use `Box<T> | Nil` when absence is
needed.

### Represented types

~~~snacc
type UserId is Int64
let id: UserId = UserId(42)
print(Int64(id))
~~~

`UserId` is distinct from `Int64` and from every other type represented by
`Int64`. It has the same representable values, size, and alignment as its
immediate representation, but representation identity does not make the two
assignable. Represented types are not aliases: they do not inherit arithmetic,
ordered comparison, fields, or methods.

Wrapping and unwrapping are explicit and remove exactly one layer.
`Name(value)` requires a value of the immediate representation type;
`Immediate(value)` recovers it. Two values of one represented type support `==`
and `!=` when the represented type does; the comparison is the representation's.

### Structs

~~~snacc
type Point is struct
    x: Float64,
    y: Float64,
end

type Marker is struct end
~~~

A struct contains its declared fields in declaration order, and field names are
unique within the struct. A field's type is fixed by its declaration. Fields are
readable everywhere; there is no visibility boundary, and field access invokes
no code.

A struct value is constructed by calling its type with named arguments. Every
declared field must occur exactly once; missing, duplicate, and unknown field
names are errors, and positional construction of a non-empty struct is invalid.
Argument order does not select fields, but argument expressions still evaluate
from left to right in written order.

~~~snacc
let point: Point = Point(y: 4.0, x: 3.0)
print(point.x)
~~~

An empty struct is valid, has exactly one value, and is constructed with an
empty argument list (`Marker()`); the parentheses are required, because a type
name alone is never a value.

Structs are values without object identity: binding, passing, returning, and
union injection copy the complete value. Two values of the same struct type
support `==` and `!=` when every field type does; fields compare in declaration
order with short-circuiting, and all values of one empty struct type are equal.
Different nominal struct types never compare, even with identical fields.
Structs support no ordered comparison and no arithmetic.

Fields declare no mutability of their own. Whether `point.x = 5.0` is legal is
decided entirely by the root variable, exactly as “Declarations and assignment”
describes.

### Unions

~~~snacc
type Shape is union
    | Circle is struct
        radius: Int64,
      end
    | Rectangle is struct
        length: Int64,
        width: Int64,
      end
end

type Direction is union
    | East
    | West
end
~~~

Every union member declares a new type inside that union's namespace, so the
first declaration creates `Shape`, `Shape.Circle`, and `Shape.Rectangle`, and
never a top-level `Circle`. A member name is unique within its union. A bare
member is exactly shorthand for an empty struct member: `| East` means
`| East is struct end`. A member is either a bare or an inline struct; it does
not name an unrelated existing type and does not nest a second union. A union
may also declare the member `| Nil`, which carries no value and is the type of
the contextual literal `nil` in a position expecting that union. `Nil` is the
only place a union member is spelled with a reserved type name. A union
declares `Nil` at most once and never as its only member, and `is Nil` tests
that member without a binding: `is Nil(name)` is rejected because the member
carries no value.

A union value contains exactly one direct member value and a tag identifying
that member's type. Construction always names the member type, never the union:
`Shape.Circle(radius: 10)` is valid and `Shape(value)` is not. A value of
`Shape.Circle` is implicitly assignable to `Shape` in bindings, arguments,
results, and branch unification. There is no implicit conversion from a union
to a member, and no member is assignable to a different union.

Two values of the same union type support `==` and `!=` when every member type
does. Values with different tags are unequal; values with the same tag compare
their contained member values. A union never compares directly with one of its
member types. Unions support no ordered comparison and no arithmetic.

A bare member remains a nominal empty type. The language assigns no integer
discriminant, permits no conversion between a member and an integer, and adds
no enum declaration category. The runtime tag is unobservable except through a
type test.

### Methods and `self`

~~~snacc
method Point.length_squared(): Float64 do
    self.x * self.x + self.y * self.y
end

method Point.translate(dx: Float64, dy: Float64) do
    self.x = self.x + dx
    self.y = self.y + dy
end

method Shape.Circle.area(): Int64 do
    self.radius * self.radius
end
~~~

A method's qualified name is a receiver type path followed by exactly one
method name, so a top-level receiver uses two components and a union-member
receiver uses three. A method must be declared in the same program as its
receiver type, is visible independent of declaration order, and its name is
unique for that receiver; there are no overloads. Two receiver types may reuse
one method name. Methods and fields have separate roles, and `value.name`
without `()` always denotes a field, never a method value.

Every method has exactly one implicit receiver named `self`. `self` is a
keyword: it cannot be renamed, declared as a local, or used outside a method
body, and there is no `method mut` form or source-level mutable-method
category. `self` denotes the original receiver storage for the call, so a
method may assign to `self` as a whole or through its fields:

~~~snacc
method Point.reset() do
    self = Point(x: 0.0, y: 0.0)
end
~~~

`receiver.name(arguments)` resolves statically from the receiver's exact
nominal type. It is never virtual and performs no runtime member search. The
receiver expression evaluates once, before the explicit arguments, which then
evaluate left to right. At a call head, a qualified path whose first component
resolves to an in-scope local, parameter, or `self` is a receiver access;
otherwise the path resolves as a type constructor or top-level callable. This
rule is independent of capitalization. A bare `name(...)` never calls a local,
because Snacc has no function values.

A method that only reads `self` may be called on a variable or a temporary. A
method that may assign through `self` — including through methods it calls —
requires a mutable receiver root and therefore cannot be called on a temporary
or on a plain `let` binding:

~~~snacc
let point: Point = Point(x: 3.0, y: 4.0)
print(point.length_squared())
point.translate(1.0, 2.0) // error: point is an immutable root

let mut movable: Point = Point(x: 3.0, y: 4.0)
movable.translate(1.0, 2.0)
~~~

Whether a method may write through `self` is an internal fact used only to
validate call sites. It is not a distinct kind of method, a source annotation,
or an overload dimension. A method with `: T` returns a value assignable to
`T`; a method with no result type produces no result and its call is valid only
in a statement position. Methods are internal declarations: they are never
exported and `extern rust` has no method form. Methods cannot be nested and
cannot capture lexical state.

Methods on a union receive the union value. A member method is callable only on
that member type, ordinarily through a type-test binding.

### Type tests

`is` has one type-relationship meaning in declarations and in conditions. As a
condition, `place is Union.Member` tests whether the union place currently
contains that direct member type and produces `Bool`:

~~~snacc
if direction is Direction.East then
    east()
else
    west()
end
~~~

The left side must be a place with the containing union type, and the right
side must name a direct member of that union. Testing an unrelated type is an
error rather than a constant `false`, and testing a value against its own
static type is an error rather than a constant `true`. There is no general
reflection or structural type test.

`place is Union.Member(name)` additionally binds the contained member value to
`name` when the test succeeds:

~~~snacc
if shape is Shape.Circle(circle) then
    circle.radius * circle.radius
elseif shape is Shape.Rectangle(rectangle) then
    rectangle.length * rectangle.width
end
~~~

The binding has the exact member type, is an immutable root, and is scoped to
that branch alone: it does not exist in later conditions, later branches, or
after the `if`, and its name must be unused everywhere else in the containing
function or method. Binding an empty member is permitted but unnecessary.

Both forms are valid only as the complete condition of an `if` or `elseif`.
Each written condition reads its place once. A test's place may be a local, a
parameter, `self`, or a sequence of direct field accesses rooted at one of
them; a call result or other computed expression is not a place and must be
bound to a name first.

A value-form `if` may omit `else` only when every condition is an `is` test,
every test names the same union-typed place, and every direct member type of
that union appears exactly once. Such a chain is exhaustive, and the ordinary
common-result-type rule applies across its branches. If any member is absent,
any condition is not a qualifying type test, or the conditions inspect
different places, a value-form chain requires `else`. Testing one member twice
is an unreachable-branch error, and supplying `else` after covering every
member is an unreachable-branch error in statement and value context alike.
Adding a member to a union therefore makes every formerly exhaustive chain that
lacks `else` fail to check until it handles the new member.

### Native representation

The native layout of every user-defined type is private to one compiler build.
Source programs cannot observe size, alignment, field offsets, union tags, or
padding, and no layout guarantee crosses a Rust bridge.

## Inline sum types

`|` combines two or more existing types into a structural sum wherever a
`type` is written, including a function or method result, a parameter, a
local, a field, and the referent of `Ref<T>`:

~~~snacc
fun find(index: Int64): Byte | Nil do
    nil
end

fun replace(value: Ref<Byte | Nil>) do
    value = nil
end

type CacheEntry is struct
    value: Byte | Nil,
end
~~~

An inline sum's identity is the unordered set of its direct member types, so
written order and parenthesized grouping are not significant:
`Byte | Nil`, `Nil | Byte`, and `(Byte) | Nil` name one type, and
`(Byte | Bool) | Nil` and `Byte | (Bool | Nil)` name another. A source sum
must name at least two distinct members; repeating a member, including after
flattening a parenthesized group, is an error rather than a silent
deduplication. `Nil` is permitted as a member only alongside at least one
non-`Nil` member, exactly like a named union. A named union may itself be one
member of an inline sum (`Shape | Nil`); its own members do not flatten into
the inline sum. `Ref<T>` is not a value-type member: `Ref<Byte> | Nil` is
invalid, while `Ref<Byte | Nil>` -- a reference to sum-typed storage -- is
valid. An inline sum cannot be a represented type's immediate representation,
because a represented type is opened by calling its representation's type
name, and an inline sum has no such callable name.

A value of a direct member type injects implicitly into an expected inline
sum: an exact member match is used when one exists, and otherwise the value
converts through the one existing `Int64`-to-`Float64` conversion when exactly
one member accepts it. `nil` selects an expected sum's `Nil` member the same
way it selects a union's. An inline sum value is assignable to another inline
sum only when their member sets are identical; there is no implicit
subset-to-superset conversion, so widening a sum requires decomposing it and
re-injecting each bound member under the wider type.

`is` decomposes an inline sum exactly like a union, except the tested member
may be any one direct member -- a built-in scalar, a named type, or a named
union -- not only a qualified union-member name:

~~~snacc
if result is Byte(byte) then
    print(byte)
elseif result is Nil then
    print(0u8)
end
~~~

The binding has the exact tested member's type. A test names only a direct
member: testing a type nested inside a named-union member requires a second
test after binding that union. An `if`/`elseif` chain over an inline sum is
exhaustive, and may omit `else`, when it tests the same place and covers every
direct member exactly once, under the same rules as a named union's chain.

Forming a new inline sum from otherwise unrelated branch values still requires
an explicit expected sum type -- from a declaration, parameter, field, result,
or enclosing expression -- exactly like a union; Snacc never synthesizes a
sum type merely because two branches disagree.

For its first version, an inline sum reuses a named union's tagged
representation: a private, deterministically tagged storage field for each
direct member, with exactly one member active at a time. Copying, moving, and
destroying a sum act on its active member alone. Two values of one inline sum
support `==` and `!=` when every direct member does: different active members
compare unequal, and equal active members compare using that member's
equality. An inline sum supports no ordered comparison, arithmetic, field
access, direct printing, or method declaration; a program decomposes the value
with `is` first. Source programs cannot observe a sum's tag, member order, or
layout, exactly as for a named union.

An inline sum is never identical to a named union, even when every possible
runtime value looks the same, and there is no `Option<T>` type: `T | Nil` is
the direct spelling for an optional value. Represented, struct, union, and
inline sum types may not appear in an `extern rust` parameter or result,
including an inline sum whose members individually have bridge
representations; their layouts are compiler-private.

## Statements and blocks

A block is an ordered sequence of block elements: variable declarations,
assignments, `while` statements, `break`, `return`, `if` forms, and
expressions. Block elements execute in source order. There is no statement
separator; a new block element begins wherever the previous construct's
grammar cannot continue. Snacc has no semicolon token, and none of the
examples below use one.

A block that is required to produce a value (a value-returning function or
method body, or a branch of a value-form `if`) must have every reachable path
either end its last reachable block element in an expression assignable to
the required type, or execute `return` first. A declaration, an assignment,
`while`, `break`, or a no-result call cannot supply that value; earlier
expression results in the block are discarded regardless. A no-result block
(the top-level executable body, a loop body, a no-result function or bridge's
call site, or a branch of a statement-form `if`) has no required final value.

A block element following a `return` that executes on every path reaching it
-- directly, or through an `if` whose every reachable branch returns -- is
unreachable and an error; a `return` confined to one non-exhaustive branch
does not make later source unreachable.

~~~snacc
fun square(value: Int64): Int64 do
    let result: Int64 = value * value
    result
end

fun announce(value: Int64) do
    print(value)
end
~~~

### Declarations and assignment

`let name: T = value` declares a new binding, converting `value` to `T` if
permitted. The name becomes visible after the declaration and remains visible
until the end of its enclosing block; a nested block may declare a name that
shadows an outer one, but within one function or method every declared name
(parameters and locals, including names declared in nested blocks and sibling
branches) must be unique.

`let` alone creates an immutable root: neither the declared name nor a later
use of it may appear on the left of `=`. `let mut name: T = value` creates a
mutable root, which may be reassigned:

~~~snacc
let count: Int64 = 1
let mut total: Int64 = 10
total = total + count
~~~

Assigning to a name that was not declared `mut` is an error, as is assigning
a value not assignable to the declared type. `place = expression` is
accepted only as a block element, never where an expression is required (an
initializer, an argument, a condition, or a returned value).

An indexed assignment `collection[index] = expression` is also a block element.
For an array or list, the collection must be an owning place rooted at a
mutable binding, the index must be `Int64`, and the checked index must be in
range. For a map, the key uses the map's ordinary key or String query rules,
the map must be mutable, and the key must already exist. A missing map key is a
defined collection-bounds failure; indexed assignment never inserts a new key.
Views are immutable and cannot be indexed-assignment targets.

An assignment target is a place: a root (a local, a parameter, or `self`)
followed by zero or more field selectors. Mutability belongs to the root
alone and reaches through the complete field path. A `let mut` local and
`self` inside its own method are mutable roots; a plain `let` local, an
ordinary parameter, and a type-test binding are not, and no field selector
changes that either way. There is no field-level `mut`, and mutability is
never a property of a type.

### `while`

`while condition do body end` is a statement, not an expression. Its
condition uses truthiness, evaluated before each iteration, and its body is a
no-result block:

~~~snacc
while ready() do
    step()
end
~~~

`while` cannot appear as an initializer, operand, argument, condition,
returned expression, or any other expression position. Unlike earlier
versions of this contract, a `while` that never executes its body produces no
value at all — there is no type-specific zero-value fallback. A corpus site
that used to rely on that fallback now writes the loop followed by an
explicit trailing value:

~~~snacc
fun zero_after_loop(value: Int64): Int64 do
    while false do
        print(value)
    end
    0
end
~~~

### `break`

`break` is a statement valid only within the body of a `while` or `for`. It
immediately exits the innermost enclosing loop and takes no operand:

~~~snacc
while ready() do
    if done() then
        break
    end
    step()
end
~~~

A `break` outside any loop body is an error. Because a loop condition is an
expression and `break` is a statement, `break` can never occur in a
condition.

### `return`

`return` is a statement, valid only lexically inside a function, method, or
static associated-function
body -- including a nested `while` or `if` belonging to that same callable --
and it exits the nearest enclosing callable, not merely the nearest block or
loop; `return` at top level or in an `extern rust` declaration's absent body
is an error. `break` continues to exit only the nearest loop; neither
statement accepts the other's role or operand form.

The enclosing callable's declared result -- not the kind of the immediately
enclosing block -- selects the permitted form: a callable that declares `: T`
accepts only `return expression` with a value assignable to `T`, using the
same assignability rules as an implicit final expression; a callable that
omits a result accepts only bare `return`.

~~~snacc
fun absolute(value: Int64): Int64 do
    if value < 0 then
        return 0 - value
    end
    value
end

fun print_if_positive(value: Int64) do
    if value <= 0 then
        return
    end
    print(value)
end
~~~

`return` is not an expression: it has no type and cannot be stored, passed,
compared, or nested inside another expression. `return`'s optional value
evaluates exactly once and is materialized as the callable's result before
any enclosing scope is exited; a move-only result transfers ownership and its
cleanup obligation to the caller, and every other locally owned value still
available at that point is destroyed, in reverse declaration order, from the
innermost exited scope outward. A write already made through `Ref<T>` remains
visible to the caller; `return` does not roll it back.

`Error` is a predeclared move-only struct with `category`, `header`, and
`message` fields of type `String`. A fallible result is an inline sum such as
`T | Error`. `return_on_error call` requires an enclosing fallible callable; it
returns the active `Error` or yields the non-Error success member(s),
preserving a reduced inline sum when more than one success member is present.
When the operand is exactly `Nil | Error`, it may be used as a statement: an
active `Nil` is consumed and execution continues, while an active `Error` is
propagated unchanged. This form produces no value and cannot discard a
non-`Nil` success member. Every form consumes the complete owning operand, and
propagation returns through the enclosing callable's ordinary checked cleanup
path.

`defer call` schedules a no-result direct call for scope exit without evaluating
its receiver or arguments at the scheduling point. On exit, the call evaluates
against current values. `defer_on_error call` is valid only in a callable whose
result directly contains exact `Error`, and runs only when an active `Error`
crosses that scope. Both forms follow one reverse-registration cleanup order
with owned-value destruction; `defer` runs on successful and error exits, while
`defer_on_error` runs only on error exits. Neither form runs for runtime traps.

### `if`

`if` is always a block element, never nested inside an expression. The parser
produces one `if` form; whether it is a statement or a value-producing
construct is decided by where it appears, not by its syntax.

An `if` used as an ordinary block element is a statement. Its branches are
no-result blocks, and `else` is optional:

~~~snacc
if ready() then
    step()
end
~~~

An `if` used as the final element of a value-required block is value-form: it
must produce a value on every reachable path, so it requires an `else` branch
unless an exhaustive type-test chain covers every path (see “Type tests”).
Every reachable branch must end in a value-producing expression with a common
assignable type; a declaration, assignment, `while`, `break`, or no-result
call cannot supply that value. A branch that ends in `return` needs no such
value and does not participate in this requirement. When every reachable
branch returns, the `if` itself terminates that callable path and supplies no
value at all, even where a value-form `if` would otherwise be required.

Each ordinary condition of an `if` or `elseif` uses truthiness. `Bool(false)`
and an active `Nil` are falsey; every other value is truthy. Type tests remain
complete Boolean conditions.

## Expressions

Operands, arguments, and block elements evaluate from left to right. A
discarded expression result still evaluates completely, including its side
effects.

- `+`, `-`, `*`, and `/` on `Int64` and `Float64` operands require numeric
  operands. Two `Int64` operands produce `Int64`; otherwise both operands are
  widened as needed and the result is `Float64`. `Int64` division truncates
  toward zero. `Int64` overflow and division by zero have undefined behavior;
  no arithmetic-failure diagnostic or quotient is guaranteed. `Float64` arithmetic
  follows IEEE 754 for admitted results; any operation that would produce NaN
  is rejected statically when known or terminates with
  `InvalidFloatingOperation` at runtime.
- For `Byte`, `UInt16`, `UInt32`, and `UInt64`, `+`, `-`, and `*` require two
  operands of the same `UIntN` type and produce that same type, using
  arithmetic modulo 2^N. `/` is unsigned integer division, discarding the
  fractional part. Executing unsigned division by zero is undefined behavior,
  exactly like `Int64` division by zero.
- `Float32` arithmetic requires two `Float32` operands, produces `Float32`,
  and follows IEEE 754 binary32 semantics for admitted results; each operation
  rounds its result to binary32 and is never evaluated at `Float64` precision.
  An operation that would produce NaN follows the same static rejection or
  `InvalidFloatingOperation` runtime failure as `Float64`.
- `Byte`, `UInt16`, `UInt32`, `UInt64`, and `Float32` operands are accepted
  only in exact-type pairs for arithmetic and comparison: mixing any two of
  these five types, or mixing one of them with `Int64` or `Float64`, is a type
  error.
- `<`, `<=`, `>`, and `>=` require numeric operands and produce `Bool`. `==` and
  `!=` accept numeric operands, two `Bool` operands, or two
  operands of one user-defined type that supports equality, and produce `Bool`.
  One operand may be `nil` when the other is a union that directly contains
  `Nil`; `nil == nil` has no such type and is rejected.
  Mixed `Int64`/`Float64` operands are compared as `Float64`.
  `Byte`, `UInt16`, `UInt32`, `UInt64`, and `Float32` operands must match
  exactly for ordered comparison and equality alike; unsigned comparisons use
  unsigned ordering, and `Float32` follows the same invalid-floating-operation
  rule as `Float64`; NaN never becomes an observable Snacc value.
- `print(value)` writes the value followed by a line feed and returns the same
  value with the same type. An unsigned value is written in base ten without a
  suffix; a `Float32` value uses the same observable formatting rule as
  `Float64`, applied to its binary32 value.
- `!`, `and`, and `or` require `Bool` operands. `and` and `or` short-circuit;
  they do not perform truthiness conversion.
- A call must supply exactly one argument per parameter. Each argument must be
  assignable to its parameter type. A call to a declaration with a result is
  itself an expression, whose type is the declared result type. A call to a
  declaration without a result is not an expression: it is valid only as a
  block element whose value is not consumed (for example, as an argument, an
  initializer, or an operand, it is an error). The same rule applies to a
  method call without a result.
- `value.field` reads one declared field of a struct or union-member value or
  place, and invokes no code. `receiver.name(arguments)` is a method call.

A function, method, or static associated-function body is a block. If it
declares a result type, the body is
value-required and its value becomes the result; otherwise the body is a
no-result block.

## Recursion

A function, method, or static associated function may call itself directly, or
two or more callables may
call one another (mutual recursion), including a call to a declaration that
appears later in the source. Recursion introduces no syntax and no distinct
call form: declarations are visible throughout the program independent of
source order, so a recursive or forward call resolves through the ordinary
declaration table and is checked with the same argument, result, no-result,
and diagnostic rules as any other call.

~~~snacc
fun factorial(n: Int64): Int64 do
    if n == 0 then
        1
    else
        n * factorial(n - 1)
    end
end

fun even(n: Int64): Bool do
    if n == 0 then true else odd(n - 1) end
end

fun odd(n: Int64): Bool do
    if n == 0 then false else even(n - 1) end
end
~~~

A method may recursively call itself or another statically resolved method.
Ordinary receiver-mutability rules still apply at every call site: the "may
assign through `self`" fact is computed as a fixed point over the whole
call graph, so it remains correct through a self-recursive or mutually
recursive method cycle. If any method reachable from a call — through any
number of intermediate calls, including back through the caller itself —
assigns through `self`, every method in that cycle is treated as assigning
through `self`, and every call site that reaches it still requires a mutable
receiver root.

Snacc does not require a syntactic base case and does not prove termination.
A function, method, or static associated function that recurses without bound
is well-typed:

~~~snacc
fun loop() do
    loop()
end
~~~

Unbounded recursion exhausts the native call stack, or fails some other
platform-dependent way, at runtime; the compiler diagnoses neither
nontermination nor probable stack overflow. Snacc makes no tail-call
guarantee: an implementation may eliminate a tail call when doing so preserves
observable behavior, but a program cannot depend on that optimization, and a
recursive call is not required to run in constant stack space.

## Reference parameters

`Ref<T>` declares a call-scoped mutable reference to caller-owned storage of
type `T`. It may be the direct declared type of a top-level function
parameter, an explicit method or static associated-function parameter, or an
`extern rust` parameter, and
nowhere else:

~~~snacc
fun add_into(x: Int64, y: Int64, result: Ref<Int64>) do
    result = x + y
end

let x: Int64 = 20
let y: Int64 = 22
let mut z: Int64 = 0
add_into(x, y, z)
print(z) // 42
~~~

`Ref<T>` is not a value type and never becomes one. It may not be a function or
method result, a local binding type, a struct field type, a represented type's
representation, a union or union-member type, the author-written type of
`self`, or nested inside another type, including another reference: `Ref<Ref<T>>`
does not exist. There is no address-of or dereference operator, no reference
literal, and no way to construct, store, return, or compare a reference. A
reference therefore cannot outlive the call that created it, and no escape
analysis is required.

An argument for a `Ref<T>` parameter is written like any other argument, and
must be an initialized mutable place of exact type `T`: a `let mut` local, a
reference parameter, `self`, or a field path rooted at one of those. A literal,
a call result, an arithmetic or conditional result, and a plain `let` binding
are all rejected. The referent type is matched exactly: neither the `Int64` to
`Float64` widening nor a represented type's equivalence to its representation
applies, so an `Int64` place is not a valid argument for `Ref<Float64>`, and a
`UserId` place is not one for `Ref<Int64>`.

Inside the callee, a reference parameter is dereferenced automatically. Its
name denotes the referent, so it reads, participates in arithmetic and
comparison, prints, converts, and is passed by value exactly as a `T` would be,
and assigning to it replaces the caller's complete value:

~~~snacc
fun bump(counter: Ref<Int64>, by: Int64) do
    counter = counter + by
end
~~~

Field selection, field assignment, and method calls through a reference
parameter reach the caller's storage the same way, and a reference parameter is
a mutable root, so a receiver-writing method may be called on one. Passing a
reference parameter to another `Ref<T>` parameter reborrows it for the nested
call and needs no extra syntax; passing it to a value parameter copies its
current value at that moment.

Each `Ref<T>` parameter has exclusive access to its referent for the duration
of the call. Two reference arguments in one call must not overlap, and two
places overlap when they are identical or one is reached by selecting fields
from the other. Two distinct fields of the same struct do not overlap:

~~~snacc
exchange(value, value)     // error: overlapping reference arguments
exchange(point.x, point.y) // valid when point is a mutable root
use_both(point, point.x)   // error when both parameters are Ref
~~~

For a method call, an addressable receiver place participates in the same
check, whether the method writes through `self` or only reads it. A temporary
receiver has independent storage and cannot overlap a caller place.

Arguments are processed left to right. A value argument evaluates to and keeps
its value; a reference argument evaluates its place once and keeps that place's
identity. Every borrow begins only after all arguments are processed, so
reading a place by value and also passing it by reference in the same call is
valid, and the value argument holds the value read before the call:

~~~snacc
fun replace(previous: Int64, value: Ref<Int64>) do
    value = previous + 1
end

let mut number: Int64 = 4
replace(number, number)
print(number) // 5
~~~

A write through a reference is visible to the caller immediately and stays
visible after the call returns. `Ref<T>` provides no transaction, rollback, or
definite-write semantics: a function may return without changing its referent,
and every write completed before the selected return remains visible.

Changing a parameter between `T` and `Ref<T>` is a breaking signature change
for every caller and, for a bridge, for the host.

## Rust bridge

`extern rust` declares a host function implemented with the platform C ABI. Its
link symbol must begin with `snacc_user_`. The declaration is the complete Snacc
view of that function; a signature mismatch in the host is invalid and need not
be diagnosed by the Snacc compiler.

A bridge declaration may omit its result type, exactly like a Snacc function:

~~~snacc
extern rust "snacc_user_log" fun log(value: Int64)
~~~

Its Rust assertion result is `()`, and its C ABI result is `void`. Its call
has the same no-result restriction as an internal no-result function.

Represented, struct, member, and union types may not appear in an `extern rust`
parameter or result, and neither may an inline sum type, even one whose
members individually have bridge representations. Their source-level
representation implies no stable C ABI representation, and the checker
rejects them while collecting declarations, before any body is checked.
Methods are never exported and `extern rust` has no method form. Internal
Snacc functions and methods may accept and return every user-defined type and
inline sum.

The ABI representation is `i64` for `Int64`, IEEE binary64 for `Float64`, `u8`
for `Bool`, `u8`/`u16`/`u32`/`u64` for
`Byte`/`UInt16`/`UInt32`/`UInt64`, and IEEE binary32 (`f32`) for `Float32`. A
host must encode `false` as zero and `true` as one. `Byte`
and `UInt16` sub-word parameters and results carry the zero-extension
attribute the target C ABI requires, matching Rust's and Clang's `zeroext`
behavior; `Bool` carries the same attribute for consistency. Rust bridges
must not unwind across the ABI boundary.

A bridge parameter may use `Ref<T>` only when `T` is one of those by-value
scalars; every user-defined type is excluded, and `Nil` is not a type that can
be written there at all. `Ref<T>` maps to `&mut R`, where `R` is the referent's
own ABI representation:

| Snacc | Rust bridge parameter |
| --- | --- |
| `Ref<Int64>` | `&mut i64` |
| `Ref<Float64>` | `&mut f64` |
| `Ref<Bool>` | `&mut u8` |
| `Ref<Byte>` / `Ref<UInt16>` / `Ref<UInt32>` / `Ref<UInt64>` | `&mut u8` / `&mut u16` / `&mut u32` / `&mut u64` |
| `Ref<Float32>` | `&mut f32` |

The generated assertion spells the reference out, so a value parameter and a
reference parameter are never interchangeable even when their representations
match. The compiler passes a non-null, correctly aligned, initialized,
exclusively borrowed `T`. The bridge may read and write it for the duration of
the call. It must not retain its address, and must not create any reference,
pointer, callback, thread, or external state that can reach it after the call
returns. Before returning, it must leave a valid Snacc `T` representation; in
particular a `&mut u8` standing for `Ref<Bool>` must contain zero or one.
Violating this contract is invalid host code and need not be diagnosed by the
Snacc compiler.

A bridge function is an ordinary safe `pub fn` in the host crate's `interop`
module, reachable at `crate::interop::<symbol>`. Its Rust item name is exactly
the declared link symbol. `Ref<T>` appears as `&mut R`; an immutable scalar
`View<T>` appears as `&[R]`. `cargo-snacc` emits the private `extern "C"`
adapter that exports the declared symbol, expands each view to `(*const R,
usize)`, constructs the temporary slice, and calls the safe host function.
Host code does not carry `no_mangle`, `export_name`, or an `extern "C"` ABI;
the generated assertion verifies its ordinary Rust signature before linking.

## ABI version and ownership

The current Snacc ABI version is 12. The version covers the `snacc_main` entry,
the required `snacc_print_*` runtime imports, the permitted Rust bridge types
(including the no-result bridge signature added in ABI version 2, the
fixed-width unsigned and `Float32` types added in ABI version 3, and the
`Ref<T>` bridge parameters added in ABI version 4, and the removal of
standalone `Nil` and of the `snacc_print_nil` import in ABI version 5), their
representations and
valid values, the C calling convention, and the ownership rules for values
crossing those boundaries.

ABI version 12 exports `snacc_main` as `extern "C" fn() -> i32` and imports:

| Symbol | Rust signature |
| --- | --- |
| `snacc_print_f64` | `extern "C" fn(f64)` |
| `snacc_print_i64` | `extern "C" fn(i64)` |
| `snacc_print_bool` | `extern "C" fn(u8)` |
| `snacc_print_u8` | `extern "C" fn(u8)` |
| `snacc_print_u16` | `extern "C" fn(u16)` |
| `snacc_print_u32` | `extern "C" fn(u32)` |
| `snacc_print_u64` | `extern "C" fn(u64)` |
| `snacc_print_f32` | `extern "C" fn(f32)` |
| `snacc_invalid_floating_operation` | `extern "C" fn() -> !` |

The compiler also emits private runtime imports for owned strings, lists,
boxes, views, maps, and sets. Their descriptor layouts, operation symbols,
and cleanup signatures are compiler/runtime implementation details covered by
ABI version 12; they are not Rust bridge values and are not callable from
Snacc source.

Every value permitted across an ABI version 12 Rust bridge is a scalar passed
or returned by value, a `Ref<T>` parameter borrowing one such scalar for the
duration of the call, an immutable scalar `View<T>` parameter, or no value at
all (a no-result bridge's C ABI result is `void`). A view is presented to safe
Rust as `&[R]` but crosses physically as `(*const R, usize)`; it cannot be a
result or a `Ref<T>` referent. Crossing the boundary by value copies the value;
no allocation, destructor obligation, or resource ownership crosses with it,
and a Rust bridge may retain its scalar copy. A `Ref<T>` parameter borrows caller
storage rather than transferring it: the bridge may read and write the referent
during the call and must not retain access to it afterwards. No reference is
returned, and no bridge result is a reference. Buffers, aggregates, handles,
boxes, and owned strings and collection descriptors are not ABI version 12
bridge values. A bridge-returned or
bridge-mutated floating value must not be NaN; the generated boundary check
terminates with `InvalidFloatingOperation` before Snacc can observe it.

At ABI version 12, `Byte` and `Bool` share the Rust representation `u8`; the
generated Rust type assertion verifies width and ABI representation but cannot
distinguish these two Snacc types from one another. Their distinct Snacc
meanings remain the declaration author's contract. Standalone `Nil` was a
version 4 bridge type and is not a version 5 one; `snacc_print_nil` was a
version 4 required import and is not a version 5 one. No version 4 object,
runtime, or cached artifact is accepted by a version 5 build.

User-defined types add no ABI representation and do not change the ABI version:
none of them may cross a Rust bridge, so every boundary contract above is
unchanged by them. Inline sum types are the same: none of them may cross a
Rust bridge either, not even one whose members individually have bridge
representations, so no permitted type, representation, required symbol, or
calling-convention rule above changes at version 12. The version advanced from
5 to 6 when inline sum types were added, from 6 to 7 when Box allocation,
ownership, cleanup, and allocator imports were added, from 7 to 8 when
owned strings and immutable views were added, from 8 to 9 when the
invalid-floating-operation runtime entry and NaN boundary checks were added,
and from 9 to 10 when collection descriptors, iteration, and generated view
bridge adapters were added. It advanced from 10 to 11 when descriptor-bearing
runtime calls were normalized to pointer-safe inputs and caller-provided output
storage for the supported native targets. It advanced from 11 to 12 when
maximal concatenation and interpolation plans gained one erased-part runtime
call that performs the final String allocation exactly once.
These compiler-internal changes
are ABI-relevant and force every object and cache entry built by an older
compiler to be rebuilt. ABI version 12 requires `snacc_alloc(size, align)` and
`snacc_dealloc(ptr, size, align)`; allocation failure does not return. No
version 11 object, runtime, or cached artifact is accepted by an ABI version 12
build.

An ABI version must change when a permitted type, representation, valid-value
rule, ownership rule, calling convention, required symbol, or required symbol
signature is added, removed, or changed. A language or implementation change
that leaves every boundary contract unchanged does not change the ABI version.

The compiler and runtime must declare the same ABI version. `cargo-snacc`
compares them while compiling the Rust host, including programs with no user
bridges. The direct compiler embeds and checks the runtime source built with it.
An ABI version mismatch is a build failure; it is never deferred to execution.
