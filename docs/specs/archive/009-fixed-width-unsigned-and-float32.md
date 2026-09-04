# Specification 009: Fixed-Width Unsigned Integers and `Float32`

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Scope

This specification adds the unsigned integer types `UInt8`, `UInt16`, `UInt32`,
and `UInt64`, and the IEEE 754 binary32 type `Float32`. It defines their source
forms, values, operations, conversions, native representation, Rust bridge
mapping, runtime printing, diagnostics, and implementation requirements.

This specification does not add signed integer widths other than `Int64`,
explicit casts, bitwise operators, shifts, hexadecimal or binary literals,
scientific notation, target-sized integers, or implicit conversion among the
new types.

`UInt8` is the canonical source name for now. A future string or buffer
specification may reconsider that spelling with the surrounding byte semantics;
this specification adds no `Byte` alias or compatibility spelling.

This specification contains no open design questions. Its implementation order
and phase boundaries are fixed in section 8.

## 2. Normative reference

[The Snacc language contract](../../../LANGUAGE.md) is normative. This document
defines the change to that contract. If implementation and the updated contract
disagree, the implementation is nonconforming.

[RFC 008](008-statements-and-functions-without-results.md) shall be implemented
first and establishes ABI version 2. [Specification
012](012-variable-declarations-assignments-and-member-mutability.md) supplies
the final declaration and semicolon-free block syntax used in examples here.

## 3. Terms

An **exact type match** means that two values have the same Snacc type.

An **unsigned suffix** is one of `u8`, `u16`, `u32`, or `u64` immediately
following the digits of an integer literal.

The **width** of an unsigned integer type is the number in its name.

## 4. Language requirements

### 4.1 Types and values

The built-in type set shall gain:

| Type | Values |
| --- | --- |
| `UInt8` | integers from 0 through 2^8 - 1 |
| `UInt16` | integers from 0 through 2^16 - 1 |
| `UInt32` | integers from 0 through 2^32 - 1 |
| `UInt64` | integers from 0 through 2^64 - 1 |
| `Float32` | IEEE 754 binary32 values |

All five types are scalar copy types. Copying, binding, passing, and returning a
value duplicates its bits and transfers no resource ownership.

`UInt8`, `UInt16`, `UInt32`, `UInt64`, and `Float32` shall be reserved words and
shall not be identifiers.

### 4.2 Grammar

The following rules replace the corresponding `type` and numeric-literal rules
in `LANGUAGE.md` and `GRAMMAR.ebnf`:

~~~ebnf
unsigned-type        = "UInt8" | "UInt16" | "UInt32" | "UInt64" ;
builtin-value-type   = existing-builtin-value-type
                     | unsigned-type
                     | "Float32" ;

literal              = float32-literal | unsigned-literal | decimal-literal
                     | integer-literal | boolean-literal | nil-literal
                     | string-literal ;

float32-literal      = digit, { digit }, [ ".", digit, { digit } ], "f32" ;
unsigned-literal     = digit, { digit }, unsigned-suffix ;
unsigned-suffix      = "u8" | "u16" | "u32" | "u64" ;
decimal-literal      = digit, { digit }, ".", digit, { digit } ;
integer-literal      = digit, { digit } ;
~~~

Numeric tokens shall use maximal munch. A numeric literal immediately followed
by an ASCII letter, digit, or underscore that is not part of one of the forms
above shall be one invalid token rather than a valid number followed by an
identifier. Thus `1u8` and `1.0f32` are valid, while `1u9`, `1u8x`, `1f64`, and
`1.0u8` are lexical errors.

The suffix is part of the literal token. `u8`, `u16`, `u32`, `u64`, and `f32`
remain ordinary identifiers when they do not immediately follow digits.

### 4.3 Literal types and ranges

An unsigned literal has the type selected by its suffix. Its mathematical value
shall fit that type. An out-of-range literal shall be rejected without
truncation or wrapping.

Examples:

~~~snacc
0u8
255u8
65535u16
4294967295u32
18446744073709551615u64
~~~

`256u8` and `18446744073709551616u64` are invalid.

A literal ending in `f32` has type `Float32`. Both `1f32` and `1.0f32` are
valid. The decimal source value shall be rounded once to the nearest binary32
value using IEEE 754 round-to-nearest, ties-to-even. A literal that rounds to an
infinity shall be rejected as out of range; infinity and NaN have no literal
spellings in this specification.

An unsuffixed integer literal remains `Int64`. An unsuffixed literal containing
a decimal point remains `Dec64`.

### 4.4 Assignability and conversion

The new types are assignable only by exact type match. This rule applies to
declarations, assignments, function arguments, function results, and
conditional branches.

This specification adds no implicit conversion:

- No unsigned width converts implicitly to another width.
- No unsigned type converts implicitly to or from `Int64`.
- No unsigned type converts implicitly to a floating-point type.
- `Float32` does not convert implicitly to or from `Dec64`.

The existing `Int64` to `Dec64` conversion remains unchanged.

Consequently, literals used with the new types shall carry the matching suffix:

~~~snacc
let byte: UInt8 = 1u8
let ratio: Float32 = 0.5f32
byte
~~~

`let byte: UInt8 = 1` is a type error, not a contextual reinterpretation
of the literal.

### 4.5 Arithmetic

The operators `+`, `-`, `*`, and `/` shall accept two operands of the same new
numeric type. Mixed operands involving any new type shall be rejected.

For `UIntN`, addition, subtraction, and multiplication produce `UIntN` and use
arithmetic modulo 2^N. Division is unsigned integer division and discards the
fractional part. Executing unsigned division by zero is undefined behavior. No
quotient or runtime diagnostic is guaranteed, and lowering may use LLVM `udiv`
without a guard; its poison result may affect surrounding computation. This
specification applies the same explicit undefined-behavior classification to
the existing `Int64` division-by-zero rule when updating `LANGUAGE.md`.

For `Float32`, arithmetic produces `Float32` and follows IEEE 754 binary32
semantics. Each operation rounds its result to binary32; it shall not be
evaluated as `Dec64` and rounded only afterward.

### 4.6 Comparison and equality

The operators `<`, `<=`, `>`, and `>=` shall accept two operands of the same new
numeric type and produce `Bool`. Unsigned operands use unsigned ordering.
`Float32` ordered comparisons follow the existing `Dec64` rule: a comparison
other than `!=` is false when either operand is NaN.

The operators `==` and `!=` shall accept two operands of the same new type and
produce `Bool`. No new type compares directly with a different numeric type.
`Float32` equality follows IEEE 754: NaN is unequal to every value, including
itself.

### 4.7 Printing

`print` shall support every new type and shall return the value it prints. An
unsigned value is written in base ten without a suffix. A `Float32` value uses
the same observable formatting rule as `Dec64`, applied to its binary32 value.
Each output is followed by one line feed.

## 5. Rust bridge and native ABI

### 5.1 ABI version

Implementing this specification shall change the Snacc ABI version from 2 to 3.
Adding bridge-visible scalar representations and required runtime symbols is an
ABI change under `LANGUAGE.md`.

Compiler output, `snacc-runtime`, the direct compiler's embedded runtime, object
cache metadata, and Cargo-hosted ABI assertions shall all report version 3 in
the same change. A version 3 compiler paired with a version 2 runtime shall fail
while building the host.

### 5.2 Representations

The bridge and LLVM mappings shall be:

| Snacc | Rust | LLVM |
| --- | --- | --- |
| `UInt8` | `u8` | `i8` |
| `UInt16` | `u16` | `i16` |
| `UInt32` | `u32` | `i32` |
| `UInt64` | `u64` | `i64` |
| `Float32` | `f32` | `float` |

All are passed and returned by value under the existing C calling convention.
They carry no allocation, borrow, pointer, destructor, or resource ownership.
The LLVM column states the value/storage type. Bridge and runtime declarations
shall also carry any target C ABI integer-extension attributes required for
unsigned sub-word parameters and results. In particular, the backend shall
match Rust/Clang zero-extension behavior for `UInt8` and `UInt16` rather than
assuming that an LLVM integer width alone defines the call ABI.

At ABI version 3, `UInt8`, `Bool`, and standalone `Nil` share the Rust
representation `u8`; the generated Rust type assertion cannot distinguish
them. It still verifies width and ABI representation. Their distinct Snacc
meanings remain the declaration author's contract. Specification 012 later
removes standalone `Nil` in ABI version 5.

### 5.3 Runtime imports

ABI version 3 shall add these required runtime symbols:

| Symbol | Rust signature |
| --- | --- |
| `snacc_print_u8` | `extern "C" fn(u8)` |
| `snacc_print_u16` | `extern "C" fn(u16)` |
| `snacc_print_u32` | `extern "C" fn(u32)` |
| `snacc_print_u64` | `extern "C" fn(u64)` |
| `snacc_print_f32` | `extern "C" fn(f32)` |

`snacc-runtime::force_link` shall retain all five additions alongside the ABI
version 2 print symbols.

## 6. Diagnostics

A conforming implementation shall produce structured source diagnostics for:

| Condition | Required information |
| --- | --- |
| Unknown numeric suffix | The complete invalid numeric token and supported suffixes |
| Unsigned literal out of range | The literal and its declared unsigned type |
| `Float32` literal out of range | The literal and `Float32` |
| Assignment or argument type mismatch | Existing expected and found types |
| Mixed-type arithmetic or comparison | Both operand types and the requirement for an exact match |

Literal range errors belong to lexing because the lexer owns conversion from
source text to exact literal values. Operand and assignability errors belong to
type checking.

## 7. Compatibility

This change is source-compatible except where `UInt8`, `UInt16`, `UInt32`,
`UInt64`, or `Float32` is currently used as an identifier. Those spellings
become reserved.

Adjacent forms such as `1u8` that currently fail parsing become valid numeric
literals. Invalid suffixes continue to fail, but shall receive a lexical range
or suffix diagnostic instead of incidental parser output.

The native ABI changes from version 2 to version 3. Cargo-hosted applications
shall rebuild against ABI version 3 of `snacc-runtime`; content-addressed Snacc
objects compiled under version 2 shall not be reused.

## 8. Detailed implementation plan

Primary implementation surfaces are the compiler lexer/AST/parser, semantic
checker, LLVM backend, checked bridge declarations, Cargo assertion renderer,
runtime print functions, driver embedding, and the parse, typecheck,
conformance, runtime ABI, driver, and Cargo-hosted test suites.

### Phase 1: tokens and exact literal storage

The current compiler already preserves integer literals as exact `i64` values
instead of routing them through `f64`. This phase extends that completed
baseline; it does not add a second literal representation.

1. Add the five type names to the syntax type representation, keyword tokens,
   token display, reserved-word mapping, parser type sites, and recovery sets.
2. Replace the two-case numeric literal with explicit `Int64`, each unsigned
   width, `Float32`, and `Dec64` cases. Store unsigned values without narrowing;
   `UInt64` shall retain a full `u64`.
3. Lex a complete digit-led alphanumeric candidate, including an optional
   fractional part, before classifying it. Reject unsupported suffixes as one
   token and preserve the full source span.
4. Parse unsigned magnitudes with integer parsing and check the selected width
   before creating the token. Parse `Float32` directly to `f32`, reject a
   non-finite literal result, and never round via an intermediate `f64` source
   parse.
5. Add lexer tests at zero, maximum, one above maximum, malformed suffixes,
   identifier adjacency, fractional `f32`, rounding boundaries, and overflow.

### Phase 2: type checking and typed operations

1. Add all five checked scalar types and update every exhaustive match in
   assignability, common-type selection, equality, printing, signatures, and
   diagnostics.
2. Keep the conversion table explicit: only the pre-existing `Int64` to
   `Dec64` conversion remains. Do not route new types through a generic numeric
   promotion function.
3. Check new-type arithmetic and comparisons only when operand types are
   identical. Preserve the exact operand/result type in checked nodes.
4. Record integer signedness and float width explicitly in the typed operation;
   the backend shall not infer semantics from an LLVM bit width.
5. Add checker matrix tests for every allowed same-type operation and every
   prohibited mixed-type category.

### Phase 3: LLVM lowering

1. Add one LLVM type mapping per new scalar and predeclare the five print
   imports with exact signatures and target-required C ABI attributes.
   Apply the same target ABI audit to the existing `Bool`/`u8` bridge mapping.
2. Lower unsigned division with `udiv`, ordered comparison with unsigned
   predicates, and wrapping arithmetic without no-wrap flags.
3. Lower stored `f32` constants to LLVM `float`; converting the already-rounded
   `f32` bits through an exactly representable host `f64` API is permitted, but
   recomputing the source decimal as binary64 is not.
4. Lower Float32 arithmetic and comparisons with floating instructions and
   ordered predicates matching section 4.6.
5. Add execution cases whose results distinguish unsigned from signed
   comparison and binary32 from binary64 evaluation. Include bridge parameters
   and results at `UInt8` and `UInt16` maxima so missing zero-extension cannot
   pass unnoticed.

### Phase 4: runtime, bridges, and ABI 3

1. Add the five runtime print functions and retain their addresses in
   `force_link`; verify symbol retention in a linked executable.
2. Extend checked bridge declaration types and `render_bridge_assertions` in
   `apps/cargo-snacc/src/main.rs` with the exact Rust mappings from section 5.2.
3. Advance compiler and runtime ABI constants from 2 to 3. Do not accept ABI 2
   objects or runtimes through a compatibility path.
4. Ensure ABI version, checked bridge types, and runtime source all contribute
   to Cargo-hosted object-cache identity.
5. Add real bridge parameter/result round trips for all five types, an ABI 2↔3
   mismatch test, and cache invalidation coverage.

### Phase 5: contract, corpus, and verification

1. Update formal EBNF first in `LANGUAGE.md`, then copy it identically to
   `GRAMMAR.ebnf`.
2. Update reserved words, literal ranges, conversions, arithmetic, comparison,
   printing, bridge mappings, ABI version, and runtime-symbol tables in
   `LANGUAGE.md`.
3. Add focused parse, typecheck, run, bridge, runtime, mismatch, and cache
   corpus cases. Keep all Snacc samples semicolon-free under Specification 012.
4. Run formatting, workspace checking, and the complete workspace test suite.

## 9. Conformance tests

A conforming implementation shall test at least:

1. Every type name in bindings, parameters, function results, and Rust bridge
   declarations.
2. `0` and the maximum value for every unsigned literal width.
3. One-above-maximum rejection for every unsigned literal width.
4. `0f32`, `1f32`, fractional `Float32`, rounding, and out-of-range rejection.
5. Malformed and unknown suffixes, including a suffix followed by another
   identifier character.
6. Exact-type assignment and rejection of every category of implicit
   conversion prohibited by 4.4.
7. Arithmetic and ordered comparisons for every unsigned width, including a
   case that distinguishes unsigned from signed comparison.
8. Modular unsigned addition, subtraction, and multiplication at each width.
9. Direct binary32 arithmetic that would differ if evaluated as binary64 and
   rounded only at the end.
10. Equality and NaN behavior for `Float32`.
11. Printing every new type through a compiled executable.
12. Rust bridge round trips through both parameters and results for every new
    type.
13. Generated Rust assertion signatures for all new mappings.
14. Runtime symbol retention for all new print functions.
15. Rejection of compiler/runtime ABI version 2 and 3 mismatches.
16. ABI version 2 cache objects are not reused under ABI version 3.
17. `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and
all workspace tests.

## 10. Acceptance criteria

1. Every new literal retains its exact integer magnitude or once-rounded
   binary32 value through lexing, checking, and lowering.
2. New numeric types use exact-match assignability and operations; no implicit
   promotion beyond the existing `Int64` to `Dec64` rule exists.
3. Unsigned lowering uses unsigned predicates and division and modular
   arithmetic at the declared width.
4. `Float32` operations execute at binary32 precision.
5. Printing and Rust bridges support every new type with the exact mappings in
   this specification.
6. Compiler, runtime, assertions, caches, and tests establish ABI version 3.
7. `LANGUAGE.md`, `GRAMMAR.ebnf`, parser, checker, backend, runtime, and
   implemented behavior agree.

## 11. Non-normative rationale

Explicit suffixes make the complete `UInt64` range expressible without
contextual literal typing. Exact-match operations avoid a promotion lattice
whose rules would become a permanent constraint before Snacc has explicit
casts. Fixed-width bridge mappings cover common Cargo APIs while leaving
target-sized integers and richer conversion policy to later specifications.
Keeping `UInt8` aligns the type with the other fixed-width unsigned scalars;
string and buffer design can reconsider a byte-oriented name once its semantics
are known.
