# Snacc Language

## Grammar

```ebnf
program              = { item } ;
item                 = function-declaration
                     | rust-declaration
                     | expression ;

function-declaration = "fun", identifier, parameters, ":", type, "do",
                       expression, "end" ;
rust-declaration     = "extern", "rust", string-literal, "fun", identifier,
                       parameters, ":", type ;
parameters           = "(", [ parameter, { ",", parameter }, [ "," ] ], ")" ;
parameter            = identifier, ":", type ;
type                 = "Int64" | "Dec64" | "Bool" | "Nil" ;

expression           = non-sequence-expression,
                       { ";", [ non-sequence-expression ] } ;
non-sequence-expression
                     = if-expression | while-expression | comparison ;
if-expression        = "if", expression, "then", expression,
                       { "elseif", expression, "then", expression },
                       "else", expression, "end" ;
while-expression     = "while", expression, "do", expression, "end" ;

comparison           = additive, { comparison-operator, additive } ;
comparison-operator  = "==" | "!=" | "<" | "<=" | ">" | ">=" ;
additive             = multiplicative, { ( "+" | "-" ), multiplicative } ;
multiplicative       = postfix, { ( "*" | "/" ), postfix } ;
postfix              = atom, { arguments } ;
arguments            = "(", [ expression, { ",", expression }, [ "," ] ], ")" ;

atom                 = literal
                     | identifier
                     | binding-expression
                     | list-literal
                     | print-expression
                     | "(", expression, ")" ;
binding-expression   = "let", identifier, ":", type, "=", comparison, ";",
                       expression ;
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
   Whitespace is insignificant. A comment begins with // and ends immediately
   before the next line feed or at end of input. Keywords are reserved and
   cannot be identifiers. *)
```

String escapes do not exist.

The keywords `fun`, `extern`, `rust`, `let`, `print`, `if`, `then`, `elseif`,
`else`, `while`, `do`, `end`, `true`, `false`, `nil`, `null`, `Int64`, `Dec64`,
`Bool`, and `Nil` are reserved. Operators of the same precedence associate left
to right. Calls bind more tightly than arithmetic, arithmetic binds more tightly
than comparison, and multiplication and division bind more tightly than addition
and subtraction. There are no unary operators.

## Program structure

A program contains top-level function declarations, Rust bridge declarations,
and executable expressions. Top-level expressions execute in source order; their
final value is discarded. A successful program entry returns process status
zero.

Function and bridge names share one namespace and must be unique. Each external
link symbol must also be unique. Parameter names within one declaration must be
unique. Declarations are visible throughout the program, independent of source
order, so forward calls and recursion are valid.

Functions are top-level only. A function can read its parameters and lexical
`let` bindings, but not top-level expression values or another function's
locals. Functions are not values: a call target must be a declared function or
bridge name.

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

## Expressions

Operands, arguments, and sequence elements evaluate from left to right.

- `let name: T = value; body` evaluates `value` in the outer scope, converts it
  to `T` if permitted, and binds it only within `body`. A nested binding may
  shadow an outer binding. The expression's type and value are those of `body`.
- A semicolon sequence evaluates each element and has the type and value of its
  final non-empty element. A trailing semicolon does not change that result.
- `+`, `-`, `*`, and `/` require numeric operands. Two `Int64` operands produce
  `Int64`; otherwise both operands are widened as needed and the result is
  `Dec64`. `Int64` division truncates toward zero. `Int64` overflow and division
  by zero have unspecified behavior. `Dec64` arithmetic follows IEEE 754.
- `<`, `<=`, `>`, and `>=` require numeric operands and produce `Bool`. `==` and
  `!=` accept numeric operands, two `Bool` operands, or two `Nil` operands and
  produce `Bool`. Mixed numeric operands are compared as `Dec64`.
- `if` and every `elseif` condition must be `Bool`. All result branches must
  have one common type; mixed numeric branches use `Dec64`. The selected branch
  supplies the expression's value.
- `while condition do body end` requires a `Bool` condition. Its type is the
  body's type. It evaluates the condition before each iteration and returns the
  final completed body value. If no iteration completes, it returns the type's
  zero value: `0`, `0.0`, `false`, or `nil`.
- `print(value)` writes the value followed by a line feed and returns the same
  value with the same type.
- A call must supply exactly one argument per parameter. Each argument must be
  assignable to its parameter type. The call's type is the declared result type.

A function body is an expression. Its value must be assignable to the declared
result type and becomes the function result.

## Rust bridge

`extern rust` declares a host function implemented with the platform C ABI. Its
link symbol must begin with `snacc_user_`. The declaration is the complete Snacc
view of that function; a signature mismatch in the host is invalid and need not
be diagnosed by the Snacc compiler.

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
