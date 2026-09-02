# Snacc Language

## Grammar

```ebnf
program              = { top-level-declaration | block-element } ;

top-level-declaration
                     = function-declaration
                     | rust-declaration ;

function-declaration = "fun", identifier, parameters, [ ":", type ], "do",
                       block, "end" ;
rust-declaration     = "extern", "rust", string-literal, "fun", identifier,
                       parameters, [ ":", type ] ;
parameters           = "(", [ parameter, { ",", parameter }, [ "," ] ], ")" ;
parameter            = identifier, ":", type ;
type                 = "Int64" | "Dec64" | "Bool" | "Nil" ;

block                = { block-element } ;
block-element        = variable-declaration
                     | assignment
                     | while-statement
                     | break-statement
                     | if-form
                     | expression ;

variable-declaration = "let", [ "mut" ], identifier, ":", type,
                       "=", expression ;
assignment           = identifier, "=", expression ;

while-statement      = "while", expression, "do", block, "end" ;
break-statement      = "break" ;
if-form              = "if", expression, "then", block,
                       { "elseif", expression, "then", block },
                       [ "else", block ], "end" ;

expression           = comparison ;
comparison           = additive, { comparison-operator, additive } ;
comparison-operator  = "==" | "!=" | "<" | "<=" | ">" | ">=" ;
additive             = multiplicative, { ( "+" | "-" ), multiplicative } ;
multiplicative       = postfix, { ( "*" | "/" ), postfix } ;
postfix              = atom, { arguments } ;
arguments            = "(", [ expression, { ",", expression }, [ "," ] ], ")" ;

atom                 = literal
                     | identifier
                     | list-literal
                     | print-expression
                     | "(", expression, ")" ;
print-expression     = "print", "(", expression, ")" ;
list-literal         = "[", [ expression, { ",", expression }, [ "," ] ], "]" ;
literal              = decimal-literal | integer-literal | boolean-literal
                     | nil-literal | string-literal ;
boolean-literal      = "true" | "false" ;
nil-literal          = "nil" | "null" ;

decimal-literal      = digit, { digit }, ".", digit, { digit } ;
integer-literal      = digit, { digit } ;
string-literal       = '"', { string-character }, '"' ;
identifier           = identifier-start, { identifier-continue } ;
identifier-start     = ASCII-letter | "_" ;
identifier-continue  = identifier-start | digit ;
digit                = "0" | "1" | "2" | "3" | "4"
                     | "5" | "6" | "7" | "8" | "9" ;

(* ASCII-letter is A-Z or a-z. string-character is any character except '"'.
   Whitespace, including line breaks, separates tokens but has no other
   grammatical meaning. A comment begins with // and ends immediately before
   the next line feed or at end of input. Keywords are reserved and cannot be
   identifiers. The semicolons above terminate EBNF productions; Snacc has no
   semicolon token. *)
```

String escapes do not exist.

The keywords `fun`, `extern`, `rust`, `let`, `print`, `if`, `then`, `elseif`,
`else`, `while`, `do`, `break`, `end`, `true`, `false`, `nil`, `null`,
`Int64`, `Dec64`, `Bool`, and `Nil` are reserved. `mut` is not a reserved
keyword; it is recognized only in the fixed position immediately after `let`.
Operators of the same precedence associate left to right. Calls bind more
tightly than arithmetic, arithmetic binds more tightly than comparison, and
multiplication and division bind more tightly than addition and subtraction.
There are no unary operators.

## Program structure

A program is a sequence of top-level declarations (functions and Rust bridge
declarations) interleaved with top-level block elements, in any order.
Top-level block elements execute in source order; a top-level expression's
value is discarded. A successful program entry returns process status zero.

Function and bridge names share one namespace and must be unique. Each
external link symbol must also be unique. Parameter names within one
declaration must be unique. Declarations are visible throughout the program,
independent of source order, so forward calls and recursion are valid.

Functions are top-level only. A function can read its parameters and lexical
`let` bindings, but not top-level block values or another function's locals.
Functions are not values: a call target must be a declared function or bridge
name.

## Types and values

`Int64` is a signed 64-bit integer. An integer literal has type `Int64` and its
mathematical value must be representable by that type. Because Snacc has no unary
minus, a negative value is formed by subtraction.

`Dec64` is an IEEE 754 binary64 value. A numeric literal containing a decimal
point has type `Dec64`.

`Bool` contains `true` and `false`. `Nil` contains the single value `nil`; `null`
is an alternate spelling of that value.

String and list forms are reserved syntax. A conforming compiler must diagnose
either form as unsupported before native-code generation.

Every parameter, function result, and local binding has an explicit type. The
only implicit conversion is `Int64` to `Dec64`. It applies to bindings, function
arguments and results, numeric operands, and branches. No conversion to or from
`Bool` or `Nil` exists.

No result is not a type. A function or Rust bridge that omits its result type
produces no result: it cannot be written, stored, passed, returned, or
compared, and it is distinct from `Nil`.

## Statements and blocks

A block is an ordered sequence of block elements: variable declarations,
assignments, `while` statements, `break`, `if` forms, and expressions. Block
elements execute in source order. There is no statement separator; a new
block element begins wherever the previous construct's grammar cannot
continue. Snacc has no semicolon token, and none of the examples below use
one.

A block that is required to produce a value (a value-returning function or
method body, or a branch of a value-form `if`) must end its last reachable
block element in an expression assignable to the required type. A
declaration, an assignment, `while`, `break`, or a no-result call cannot
supply that value; earlier expression results in the block are discarded
regardless. A no-result block (the top-level executable body, a loop body, a
no-result function or bridge's call site, or a branch of a statement-form
`if`) has no required final value.

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
initializer, an argument, a condition, or a returned value). At this
milestone an assignment or declaration target is always a bare identifier.

### `while`

`while condition do body end` is a statement, not an expression. It requires
a `Bool` condition, evaluated before each iteration, and its body is a
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

`break` is a statement valid only within the body of a `while`. It
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
must produce a value on every reachable path, so it requires an `else`
branch. (At this milestone there is no union type to check exhaustively
instead — every value-form `if` needs `else`.) Every reachable branch must end
in a value-producing expression with a common assignable type; a
declaration, assignment, `while`, `break`, or no-result call cannot supply
that value.

## Expressions

Operands, arguments, and block elements evaluate from left to right. A
discarded expression result still evaluates completely, including its side
effects.

- `+`, `-`, `*`, and `/` require numeric operands. Two `Int64` operands produce
  `Int64`; otherwise both operands are widened as needed and the result is
  `Dec64`. `Int64` division truncates toward zero. `Int64` overflow and division
  by zero have unspecified behavior. `Dec64` arithmetic follows IEEE 754.
- `<`, `<=`, `>`, and `>=` require numeric operands and produce `Bool`. `==` and
  `!=` accept numeric operands, two `Bool` operands, or two `Nil` operands and
  produce `Bool`. Mixed numeric operands are compared as `Dec64`.
- `print(value)` writes the value followed by a line feed and returns the same
  value with the same type.
- A call must supply exactly one argument per parameter. Each argument must be
  assignable to its parameter type. A call to a declaration with a result is
  itself an expression, whose type is the declared result type. A call to a
  declaration without a result is not an expression: it is valid only as a
  block element whose value is not consumed (for example, as an argument, an
  initializer, or an operand, it is an error).

A function body is a block. If the function declares a result type, the body
is value-required and its value becomes the function result; otherwise the
body is a no-result block.

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

The ABI representation is `i64` for `Int64`, IEEE binary64 for `Dec64`, and
`u8` for `Bool` and `Nil`. A host must encode `false` as zero, `true` as
one, and `nil` as zero. Rust bridges must not unwind across the ABI boundary.

A bridge function is a `pub` item of the host crate's `interop` module, reachable
at `crate::interop::<symbol>`. Its Rust item name is exactly the declared link
symbol. It carries `#[unsafe(no_mangle)]` and uses the `extern "C"` ABI, and it
does not carry `#[export_name]`. `cargo-snacc` verifies the item's Rust type
against the Snacc declaration's implied ABI signature before linking; it does not
verify that the item is exported under its symbol, which remains the final
linker's responsibility.

## ABI version and ownership

The current Snacc ABI version is 2. The version covers the `snacc_main` entry,
the required `snacc_print_*` runtime imports, the permitted Rust bridge types
(including the no-result bridge signature added in ABI version 2), their
representations and valid values, the C calling convention, and the ownership
rules for values crossing those boundaries.

ABI version 2 exports `snacc_main` as `extern "C" fn() -> i32` and imports:

| Symbol | Rust signature |
| --- | --- |
| `snacc_print_f64` | `extern "C" fn(f64)` |
| `snacc_print_i64` | `extern "C" fn(i64)` |
| `snacc_print_bool` | `extern "C" fn(u8)` |
| `snacc_print_nil` | `extern "C" fn()` |

Every value permitted across an ABI version 2 Rust bridge is a scalar passed
or returned by value, or no value at all (a no-result bridge's C ABI result is
`void`). Crossing the boundary copies the value. No allocation, pointer,
reference, borrow, destructor obligation, or resource ownership crosses with
it. A Rust bridge may retain its scalar copy; Rust-owned state otherwise
remains behind the bridge. Pointers, buffers, aggregates, and handles are not
ABI version 2 values.

An ABI version must change when a permitted type, representation, valid-value
rule, ownership rule, calling convention, required symbol, or required symbol
signature is added, removed, or changed. A language or implementation change
that leaves every boundary contract unchanged does not change the ABI version.

The compiler and runtime must declare the same ABI version. `cargo-snacc`
compares them while compiling the Rust host, including programs with no user
bridges. The direct compiler embeds and checks the runtime source built with it.
An ABI version mismatch is a build failure; it is never deferred to execution.
