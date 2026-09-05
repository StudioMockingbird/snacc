# Specification 020: Literal Cleanup and Numeric Radices

Status: Proposed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Proposal state

This implementation-ready specification makes the floating-point type names
uniform, adds binary, octal, hexadecimal, and scientific-notation literals,
permits decorative `_` separators in numbers, and removes the redundant
`null` spelling of `nil`.

`LANGUAGE.md` remains authoritative until this specification is accepted,
implemented, and incorporated there.

This specification contains no open design questions. Section 13 fixes the
implementation order and phase boundaries.

## 2. Summary

The resulting literal surface is:

~~~snacc
42                  // Int64
0b101010            // Int64
0o755               // Int64
0x2A                // Int64
255u8               // Byte
0xFFu8              // Byte
65535u16            // UInt16
0xFFFFu16           // UInt16
1_000               // Int64; same value as 1000
0xFF_FFu16           // UInt16; same value as 0xFFFFu16
1.5                 // Float64
1e6                 // Float64
1.25e-3             // Float64
1f32                // Float32
1.5f32              // Float32
1e6f32              // Float32
true                // Bool
nil                 // contextual Nil member
~~~

This specification does not add hexadecimal floating-point literals,
arbitrary-width integers, exact base-10 decimals, imaginary numbers, infinity,
NaN, or a leading sign as part of a numeric token.

## 3. Floating-point type names

The previous `Dec64` and existing `Float32` are both IEEE 754 **binary**
floating-point types. They differ only in width, precision, exponent range,
storage, and ABI representation:

| Snacc | IEEE format | Significant precision | Typical decimal precision | Storage | Rust |
| --- | --- | ---: | ---: | ---: | --- |
| `Float32` | binary32 | 24 bits | 6–9 digits | 4 bytes | `f32` |
| `Float64` | binary64 | 53 bits | 15–17 digits | 8 bytes | `f64` |

`Dec64` is renamed to `Float64`. The old source type name is removed rather
than retained as an alias. `Float32` and its `f32` suffix remain unchanged.

`Float` states that these are binary floating-point approximations. Values such
as `0.1` generally cannot be represented exactly. A future decimal type, if
justified, will mean base-10 arithmetic and use the separate `Decimal` name.

There is no `f64` suffix. A decimal point or exponent without `f32` already has
the single unambiguous type `Float64`.

## 4. Grammar

The numeric and nil literal grammar becomes:

~~~ebnf
literal                = float32-literal
                       | float64-literal
                       | integer-literal
                       | boolean-literal
                       | nil-literal
                       | other-literal ;

nil-literal            = "nil" ;

integer-literal        = integer-magnitude, [ unsigned-suffix ] ;
integer-magnitude      = decimal-digits
                       | "0b", binary-digits
                       | "0o", octal-digits
                       | "0x", hexadecimal-digits ;

float64-literal        = decimal-digits, ".", decimal-digits, [ exponent ]
                       | decimal-digits, exponent ;
float32-literal        = decimal-digits, [ ".", decimal-digits ],
                         [ exponent ], "f32" ;
exponent               = "e", [ "+" | "-" ], decimal-digits ;

unsigned-suffix        = "u8" | "u16" | "u32" | "u64" ;
decimal-digits         = decimal-digit, { [ "_" ], decimal-digit } ;
binary-digits          = binary-digit, { [ "_" ], binary-digit } ;
octal-digits           = octal-digit, { [ "_" ], octal-digit } ;
hexadecimal-digits     = hexadecimal-digit,
                         { [ "_" ], hexadecimal-digit } ;
binary-digit           = "0" | "1" ;
octal-digit            = "0" | "1" | "2" | "3"
                       | "4" | "5" | "6" | "7" ;
decimal-digit          = octal-digit | "8" | "9" ;
hexadecimal-digit      = decimal-digit
                       | "a" | "b" | "c" | "d" | "e" | "f"
                       | "A" | "B" | "C" | "D" | "E" | "F" ;
~~~

`other-literal` represents the boolean, Unicode, string, and other literal
productions owned by their respective specifications. It is explanatory
notation for this grammar delta, not a production copied literally into the
complete grammar.

Radix prefixes and the exponent marker are lowercase. Hexadecimal digits may
use either case. A prefix is required even when the intended radix would be
obvious from its digits.

`_` may appear only between two digits belonging to the same magnitude,
fraction, or exponent component. It has no semantic value and is removed before
parsing or range checking. It cannot be leading, trailing, doubled, or adjacent
to a prefix, decimal point, exponent marker or sign, or type suffix.

## 5. Integer radix literals

Binary, octal, and hexadecimal notation changes only how an integer magnitude
is written:

~~~snacc
let permissions: Int64 = 0o755
let mask: UInt32 = 0xFF00FF00u32
let bits: Byte = 0b10100101u8
let readable: UInt64 = 0xFFFF_FFFF_FFFF_FFFFu64
~~~

An unsuffixed integer literal in any radix has type `Int64`. Its mathematical
magnitude must fit `Int64`; the radix never selects an unsigned type.

The suffix selects the exact unsigned type in every radix:

| Suffix | Type |
| --- | --- |
| `u8` | `Byte` |
| `u16` | `UInt16` |
| `u32` | `UInt32` |
| `u64` | `UInt64` |

The magnitude must fit the selected type. Overflow is a compile-time error;
digits are never truncated or wrapped.

~~~snacc
let byte: Byte = 0xFFu8
let invalid: Byte = 0x100u8 // error: out of range
~~~

Non-decimal radices do not permit a fractional point or exponent. These are
invalid:

~~~snacc
0x1.8
0x1p4
0b1e10
~~~

## 6. Scientific notation

Scientific notation is available only for `Float32` and `Float64`:

~~~snacc
let population: Float64 = 8.1e9
let small: Float64 = 1e-9
let explicit_positive: Float64 = 1e+9
let compact: Float32 = 6.022e23f32
let readable: Float64 = 1_234.567_89e1_0
~~~

The exponent is a signed decimal power of ten applied to the decimal source
value before its single conversion to the target IEEE binary format. The
exponent sign is part of the literal token. It does not add a general unary
`+` or unary `-` operator.

A decimal point or exponent with no suffix produces `Float64`. An `f32` suffix
produces `Float32`, whether or not the significand contains a decimal point or
an exponent:

~~~snacc
1.0       // Float64
1e0       // Float64
1f32      // Float32
1.0f32    // Float32
1e0f32    // Float32
~~~

Each literal is converted directly from its decimal source text to its target
width with round-to-nearest, ties-to-even. `Float32` is never parsed through an
intermediate `Float64`, which would permit double rounding. A finite source
value that rounds to infinity is rejected. Subnormal values and underflow to
signed zero follow the target IEEE format.

## 7. Token boundaries and invalid forms

Numeric tokens use maximal munch. Once a token begins with a decimal digit, the
lexer consumes the complete adjacent numeric-looking sequence before deciding
whether it is valid. An invalid digit, prefix, exponent, or suffix therefore
produces one diagnostic for the complete token rather than smaller valid
tokens.

Examples of invalid tokens include:

~~~text
0b                 missing binary digits
0b102              invalid binary digit
0o89               invalid octal digits
0x                 missing hexadecimal digits
0xGG               invalid hexadecimal digits
0B10               uppercase prefix is not supported
0XFF               uppercase prefix is not supported
1E6                uppercase exponent marker is not supported
1e                 missing exponent digits
1e+                missing exponent digits
1e-f32             missing exponent digits
1.f32              missing digits after decimal point
1u8f32             incompatible suffixes
1e3u32             unsigned suffix on a floating-point literal
1__000             doubled separator
1_                  trailing separator
0x_FF              separator adjacent to prefix
1_.0               separator adjacent to decimal point
1e_3               separator adjacent to exponent marker
1e+_3              separator adjacent to exponent sign
1_f32              separator adjacent to suffix
~~~

Whitespace is not permitted inside a literal. A suffix immediately follows the
magnitude or exponent.

Valid `_` separators are discarded before conversion. Consequently, each pair
below produces exactly the same type, mathematical value, checked
representation, and native constant:

~~~snacc
let a: Int64 = 1000
let b: Int64 = 1_000
let c: UInt32 = 0xFFFF0000u32
let d: UInt32 = 0xFFFF_0000u32
let e: Float64 = 12.345e10
let f: Float64 = 1_2.3_4_5e1_0
~~~

As before, a whole-number sign is an operator outside the token. Snacc has no
unary arithmetic operators, so negative values are formed through subtraction;
Specification 027 separately defines unary Boolean `!`:

~~~snacc
let negative: Float64 = 0.0 - 1e3
let negative_integer: Int64 = 0 - 42
~~~

## 8. Removing `null`

`nil` is the sole contextual spelling of a union's `Nil` member:

~~~snacc
let result: Byte | Nil = nil
~~~

`null` is removed from the keyword and literal sets. It becomes an ordinary
identifier under the normal identifier and name-resolution rules; it has no
built-in meaning:

~~~snacc
let null: Int64 = 10 // valid ordinary identifier
~~~

Existing source that used `null` as the old Nil spelling must be migrated to
`nil` wherever it meant the union member. Existing source that intentionally
used `null` as a binding becomes valid without renaming. There is no
compatibility alias or deprecation period; the lexer and parser determine the
meaning from the new token contract. Archived specifications remain historical
records and may continue to show the old spelling.

This source-language change does not alter uses of the word “null” in JSON,
HTTP, Rust, C, implementation comments about non-null pointers, or other
non-Snacc formats.

## 9. Conversions and operations

This specification changes no implicit conversion rule. In particular:

- an unsuffixed integer literal has `Int64`, regardless of radix;
- `Int64` may widen implicitly to `Float64` under the renamed existing rule;
- `Float32` does not convert implicitly to or from `Float64`;
- unsigned values do not convert implicitly to or from signed or floating
  values; and
- literal notation never changes arithmetic overflow, division, comparison,
  NaN, or printing semantics.

Printing a numeric value uses the existing canonical base-ten formatting. The
radix used to write a literal is not retained at runtime.

## 10. Rust bridge and ABI

`Float32` remains Rust `f32`, C `float`, and LLVM `float`. `Float64` has the
same representation previously assigned to `Dec64`: Rust `f64`, C `double`,
and LLVM `double`. Internal runtime symbols may retain `f32` and `f64` in their
private names.

Literal radix and scientific notation do not affect ABI representation.
Removing `null` changes no value representation because it was an alternate
source spelling for the existing `Nil` member.

Renaming a source-visible type does not by itself change the physical bridge
ABI and therefore does not require an ABI-version bump. The compiler still
rejects stale source through ordinary name checking, while cache keys and
generated bridge assertions retain their existing compatibility checks. An ABI
bump is required only when an implementation change alters a physical bridge
signature, runtime symbol, or runtime representation.

## 11. Required diagnostics

The implementation diagnoses at least:

- a missing digit after a radix prefix;
- a digit invalid for the selected radix;
- an unsupported uppercase prefix or exponent marker;
- a missing exponent digit;
- an exponent on a non-decimal integer;
- a fractional point in a non-decimal literal;
- a missing digit before or after a decimal point;
- an unknown, repeated, removed, or incompatible suffix;
- an integer magnitude outside its exact selected type;
- a decimal value that rounds to infinity at its selected width;
- a misplaced, leading, trailing, or doubled numeric separator;
- `Dec64` where a type is required, unless a user declaration independently
  defines that name;
- a contextless or otherwise invalid use of `nil` under the existing union and
  inline-sum rules.

`null` receives no special literal diagnostic because it is an ordinary
identifier. An unresolved use receives the ordinary unknown-name diagnostic.

## 12. Compatibility

The following source changes are intentional:

| Previous source | New source |
| --- | --- |
| `Dec64` | `Float64` |
| `null` | `nil` |

Existing Snacc source must be migrated by renaming `Dec64` to `Float64` and
replacing the `null` literal with contextual `nil` uses. The compiler provides
no compatibility alias or automatic source rewrite; stale `Dec64` and
contextually invalid `null` uses receive the diagnostics in section 11.

No compatibility spellings remain. Internal implementation names and Rust/C
type terminology may continue to use `f32`, `float`, `f64`, and `double` where
those are the native platform names.

### 12.1 Combined migration for Specifications 017–020

Specifications 017–020 intentionally form one pre-1.0 breaking migration. A
codebase adopting the complete set applies these source and bridge changes
together:

| Previous contract | Replacement | Owning specification |
| --- | --- | --- |
| `UInt8` | `Byte` | 017 |
| `Dec64` | `Float64` | 020 |
| `null` | `nil` | 020 |
| user-authored `#[unsafe(no_mangle)] pub unsafe extern "C" fn` bridge item | ordinary safe `pub fn` behind a generated adapter | 019 |

The migration order for source is: rename `UInt8`, rename `Dec64`, replace the
`null` literal with `nil`, then convert each Rust bridge implementation to the
ordinary signature generated or required by `cargo-snacc`. Specification 019's
generated adapter becomes the sole owner of the exported C symbol and unsafe
ABI conversion.

These changes deliberately have no aliases or mixed old/new bridge mode. The
compiler diagnoses stale source names and bridge shapes. Applying the source
renames as one migration prevents intermediate code from depending on a
combination that no single completed language version supports; only the
physical bridge-adapter change owned by Specification 019 requires a new ABI
version.

## 13. Detailed implementation plan

### Phase 1: lexer and syntax

1. Replace the `Dec64` keyword and syntax type with `Float64`; remove `null`
   from the keyword table and nil-literal parser. Preserve `Float32` and `f32`.
2. Add lowercase radix-prefix recognition, radix-specific digit validation,
   decimal exponents, optional exponent signs, and numeric separators to the
   single maximal-munch numeric lexer path.
3. Validate every separator position, then discard separators before numeric
   conversion without changing the token's complete source span.
4. Store numeric token kind, exact mathematical magnitude or decimal source,
   radix, target width, and complete source span without reparsing downstream.
5. Add lexer and parser tests for every valid boundary and required diagnostic
   in sections 5–7 and 11.

### Phase 2: syntax and semantic types

1. Rename every exhaustive syntax and checked-type variant from `Dec64` to
   `Float64`; do not add aliases or parallel variants.
2. Resolve all integer radices to one exact mathematical magnitude before range
   checking against `Int64`, `Byte`, `UInt16`, `UInt32`, or `UInt64`.
3. Convert decimal source directly to the selected IEEE width exactly once and
   preserve the target bits in the checked literal.
4. Keep exponent signs lexical and reject whole-value leading signs under the
   existing no-unary-operator rule.
5. Update deterministic type and literal rendering in diagnostics.

### Phase 3: checking and lowering

1. Reuse the existing integer and floating checked nodes after token
   validation; lowering must not interpret source text or radix.
2. Lower every integer from its checked magnitude and every decimal from its
   checked target bits.
3. Rename source-facing binary64 operation, diagnostic, method-resolution, and
   printing cases to `Float64` while retaining native `f64` backend APIs.
4. Prove with execution tests that equal magnitudes in decimal, binary, octal,
   and hexadecimal forms produce identical values and operations.
5. Test direct `Float32` rounding, exponent extremes, subnormals, signed zero,
   overflow rejection, and the absence of double rounding.

### Phase 4: bridge, runtime, and compatibility

1. Preserve the existing `Float32` mapping and map `Float64` exhaustively to
   Rust `f64`, C `double`, LLVM `double`, and the existing binary64 runtime
   operations.
2. Update generated bridge assertions, templates, fixtures, examples, and
   diagnostics to the new source name and suffix.
3. Remove source uses of `null` while leaving non-Snacc JSON and implementation
   terminology untouched.
4. Do not advance the ABI version for these source-only renames and literal
   changes; record the migration in compatibility history. A version advance
   belongs to the physical bridge-adapter change in Specification 019.
5. Add negative compatibility tests proving that the built-in `Dec64` name and
   literal uses of unresolved `null` are no longer accepted.

### Phase 5: contract and conformance

1. Update the formal EBNF first in `LANGUAGE.md`, then copy it identically to
   `GRAMMAR.ebnf`.
2. Replace the terse normative type, literal, arithmetic, comparison, printing,
   nil, and bridge text in `LANGUAGE.md` without duplicating implementation
   documentation.
3. Update every active dependent specification and plan to use `Float32`,
   `Float64`, `f32`, and `nil`; do not edit archived specifications.
4. Add conformance programs covering each radix and target type, scientific
   notation at both widths, nil-containing named and inline sums, and mixed
   arithmetic diagnostics.
5. Run formatting, workspace checking, and the complete workspace test suite.

## 14. Rejected alternatives

### Keep `Dec64` beside `Float32`

The types differ only by IEEE binary width. Using unrelated source names makes
that relationship appear semantic when it is not. `Float32` and `Float64`
present one numeric family and accurately signal approximate binary arithmetic.

### Add `f64`

Decimal-point and exponent literals already select `Float64` unambiguously.
Adding a suffix would create a second spelling without enabling another value.

### Make non-decimal prefixes select unsigned types

Radix describes notation, not signedness or width. Keeping unsuffixed literals
as `Int64` in every radix preserves the existing explicit-width rule.

### Add hexadecimal floating-point notation

It is valuable mainly for exact bit-oriented floating constants and adds a
second significand and exponent grammar. Decimal scientific notation covers
ordinary programs; exact bit construction can be considered with future bit
conversion facilities.

### Preserve `null` as an alias

Two spellings for the same absence value conflict with the language's
one-obvious-way goal. `nil` is already the established spelling.

## 15. Acceptance criteria

Implementation is complete only when:

1. `Float32` and `Float64` are the only built-in floating-point type names;
2. `Dec64` has no built-in meaning or compatibility path, while the existing
   `f32` suffix continues to produce `Float32`;
3. binary, octal, hexadecimal, and decimal integer literals select types only
   through the existing unsigned suffixes;
4. every integer magnitude is range-checked exactly without truncation;
5. decimal scientific notation selects `Float64` or suffixed `Float32` and
   rounds directly once to that width;
6. non-decimal fractions and exponents are rejected as complete invalid tokens;
7. `_` is accepted only between digits, is semantically discarded, and does
   not change a literal's type, value, range, or generated constant;
8. `nil` is the sole contextual `Nil` spelling and `null` is an ordinary
   identifier;
9. notation does not change runtime arithmetic, equality, ordering, printing,
   ownership, or layout;
10. `Float32` preserves the existing binary32 behavior and `Float64` preserves
    the binary64 Rust, C, LLVM, and runtime behavior formerly named `Dec64`;
11. the ABI version advances only if a physical bridge or runtime ABI changes,
    and incompatible cached artifacts fail closed when it does;
12. diagnostics retain the complete invalid token and precise source span;
13. parsing and checking finish before lowering and lowering receives exact
    typed values rather than source strings;
14. `LANGUAGE.md`, both grammar copies, active specifications, implementation
    comments, diagnostics, examples, and tests agree; and
15. formatting, workspace checks, and all conformance tests pass.

## 16. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [Specification 017: UTF-8 Strings, Byte Views, and Unicode Views](017-utf8-strings-and-views.md)
- [Specification 018: Inline Sum Types](archive/018-inline-sum-types.md)
- [Specification 019: Collections and Iteration](019-collections-and-iteration.md)
- [Specification 021: Truthiness and Equality](021-truthiness-and-equality.md)
