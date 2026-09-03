# RFC 017: UTF-8 Strings, Byte Views, and Unicode Views

Status: Proposed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Proposal state

This implementation-ready specification defines immutable owned UTF-8 strings,
byte and Unicode scalar types, zero-copy borrowed views, inline sum results,
concatenation, interpolation, and raw string literals.

`LANGUAGE.md` remains authoritative. Until this specification is accepted,
implemented, and incorporated there, the syntax and semantics below are not
part of Snacc.

This specification contains no open design questions. Section 16 fixes the
implementation order and phase boundaries.

## 2. Scope

This specification adds these source types:

~~~snacc
Byte
Unicode
String
View<Byte>
View<Unicode>
~~~

It also adds:

- interpreted and raw string literals;
- Unicode scalar literals;
- `String.bytes()` and `String.unicode()` views;
- immutable concatenation through `String.concat`;
- interpolation delimited by `{{` and `}}`;
- checked construction of strings from byte views;
- explicit cloning of strings.

It does not add grapheme clusters, normalization, locale-sensitive behavior,
mutable UTF-8 bytes, a general-purpose view type, stack-capacity strings, a
language-visible short-string type, formatting of user-defined values, C
strings, or string-bearing Rust bridge signatures.

## 3. Dependencies and terminology

This specification assumes the ownership and cleanup analysis established by
[RFC 016](016-box-indirection-and-recursive-data.md). `String` is a move-only
owned value, while views are non-owning borrows. It also assumes the inline sum
types established by [Specification 018](018-inline-sum-types.md), which must be
implemented before the fallible operations in this specification.

The spelling `Byte` replaces `UInt8` as the sole source name for an unsigned
8-bit integer. There is no `UInt8` alias or compatibility spelling. This is the
renaming anticipated by the string-related note in the earlier fixed-width
integer proposal. Existing `UInt8` source becomes invalid when this
specification lands. The source break is intentional and receives no
deprecation period because Snacc has not committed to source compatibility for
the pre-string spelling.

The following terms are distinct:

- a **byte** is an unsigned 8-bit integer;
- a **Unicode scalar** is a value in `U+0000..U+D7FF` or
  `U+E000..U+10FFFF`;
- a **grapheme cluster** is a user-perceived character and is not a core type;
- UTF-8 is the required internal encoding of string text;
- a **view** is a non-owning, immutable interpretation of existing storage.

## 4. `Byte`

`Byte` has exactly the values 0 through 255 and the native representation of
an unsigned 8-bit integer. It uses the arithmetic, comparison, conversion,
printing, literal-range, and overflow rules previously assigned to `UInt8`.

~~~snacc
let byte: Byte = 65u8
print(byte)
~~~

The numeric literal suffix remains `u8`: `65u8` has type `Byte`. A suffix names
the literal's unsigned width, not its source type, so renaming the type does not
add a second `byte` or `b` suffix. Unsuffixed integer literals do not become
contextually typed. The wider types and suffixes remain `UInt16`/`u16`,
`UInt32`/`u32`, and `UInt64`/`u64`; only the ubiquitous 8-bit byte receives a
semantic type name.

`Byte` maps to Rust `u8` and C `uint8_t` wherever a bridge specification
permits it. Internal runtime symbol names may continue to contain `u8`; those
names are not source-language aliases.

## 5. `Unicode`

`Unicode` contains exactly one Unicode scalar. Its native representation is an
unsigned 32-bit integer, but it is a distinct type and is not implicitly
interchangeable with `UInt32`.

~~~snacc
let letter: Unicode = 'é'
let cookie: Unicode = '🍪'
~~~

The UTF-16 surrogate range is not a valid `Unicode` value. Every compiler and
runtime operation that constructs a `Unicode` value validates this invariant
before the value enters checked Snacc execution.

A Unicode scalar literal uses single quotes and must decode to exactly one
scalar after escape processing. Empty literals, multiple scalars, malformed
UTF-8, surrogate values, and out-of-range escapes are errors.

Supported scalar escapes are:

~~~snacc
'\n'
'\t'
'\\'
'\''
'\u{1F36A}'
~~~

`Unicode` does not mean a UTF-8 byte, UTF-16 code unit, grapheme cluster, or
display cell.

## 6. `String`

### 6.1 Value invariant

`String` is an owned, finite sequence of Unicode scalars stored as canonical
UTF-8. Every `String` contains valid UTF-8. It is length-delimited, is not
implicitly NUL-terminated, and may contain the zero scalar.

The representation is private. The baseline representation may be an owned
byte allocation plus byte length. Capacity, static literal storage, and inline
short storage are implementation details that programs cannot observe.

### 6.2 Immutability

The contents and length of an existing `String` never change. There are no
`push`, `append`, `clear`, mutable-byte-view, or in-place editing operations.

`mut` retains its ordinary binding meaning and permits replacement of the
complete string:

~~~snacc
let mut greeting: String = "Hello"
greeting = greeting.concat("!")
~~~

`concat` creates a new value; it does not mutate `greeting`. The assignment
destroys the old value only after the new value has been constructed.

### 6.3 Ownership

`String` is move-only because it owns storage. Initialization, by-value
passing, returning, aggregate construction, and assignment transfer ownership
under RFC 016's whole-value move rules.

~~~snacc
let first: String = "hello"
let second: String = first
let byte_count: Int64 = first.bytes().count()
~~~

The final statement is a use-after-move error.

`String.clone()` borrows its receiver, allocates independent storage, copies
the UTF-8 bytes, and returns a new `String`. This is the only explicit string
duplication operation in the first version:

~~~snacc
let first: String = "hello"
let second: String = first.clone()
~~~

An implementation may reuse a distinguished empty representation without an
allocation because programs cannot observe storage identity.

### 6.4 Equality and ordering

Two strings are equal exactly when they contain the same Unicode scalar
sequence. Valid UTF-8 has a unique encoding for a scalar sequence, so equality
may compare byte lengths and bytes.

No normalization or case folding occurs:

~~~snacc
let composed: String = "é"
let decomposed: String = "e\u{301}"
print(composed == decomposed)
~~~

The result is `false`. Ordered string comparison is not added by this
specification.

## 7. Interpreted literals

A double-quoted literal produces a `String`:

~~~snacc
let message: String = "Hello, 🍪"
~~~

Supported escapes are `\0`, `\n`, `\r`, `\t`, `\\`, `\"`, `\{`, `\}`, and
`\u{H...}`, where `H...` contains one through six hexadecimal digits and must
denote a Unicode scalar. An unknown or malformed escape is an error.

An unescaped line break is not permitted in an interpreted literal. Source
text must be valid UTF-8, and the compiler normalizes CRLF and CR source line
endings to LF before literal values are formed.

`{{` begins interpolation. Outside interpolation, `}}` is an error so a
mistyped delimiter cannot silently become text. Literal braces may be written
with `\{` and `\}`; for example, `\{\{` produces `{{`.

## 8. Raw literals

A raw string begins with `r`, followed by zero or more `#` characters and a
double quote. It ends at a double quote followed by exactly the same number of
`#` characters:

~~~snacc
let path: String = r"C:\snacc\examples"
let quote: String = r#"She said "hello"."#
let marker: String = r##"The text contains "# inside it."##
~~~

Between its delimiters, a raw literal performs no escape processing and no
interpolation. Backslashes, quotes that do not match the complete closing
delimiter, and `{{` are ordinary contents.

A raw literal may span lines. After source line-ending normalization, every
scalar between its delimiters is preserved, including leading and trailing
newlines and indentation. The delimiter may contain at most 255 `#`
characters. An unterminated raw literal or a longer delimiter is a lexical
error.

There is no raw-interpolated literal in the first version. A program combines
raw text and dynamic values with `concat`.

## 9. Views

### 9.1 Closed built-in family

The only view types introduced here are `View<Byte>` and `View<Unicode>`.
`View` is reserved, takes exactly one argument, and rejects every other type.
This built-in type application does not by itself make user-defined generic
types part of the language.

A view is immutable and non-owning. It contains a source identity and a range
within that source. Its lowered representation may use a pointer and byte
length, but source programs cannot observe either field or construct a view
from an address.

### 9.2 Byte view

`string.bytes()` produces a zero-copy `View<Byte>` over the complete UTF-8
encoding:

~~~snacc
let text: String = "café"
let bytes: View<Byte> = text.bytes()
print(bytes.count())
~~~

`bytes.count()` is the UTF-8 byte count and is O(1). For this example it is 5.

`bytes.at(index)` returns `Byte | Nil`. It is O(1), uses a zero-based `Int64`
index, and returns `nil` for a negative or out-of-range index.

`bytes.slice(start, end)` returns `View<Byte> | Nil`. The bounds are zero-based,
the start is inclusive, the end is exclusive, and success requires
`0 <= start <= end <= count`. A byte slice need not be valid UTF-8.

### 9.3 Unicode view

`string.unicode()` produces a zero-copy `View<Unicode>` that decodes the
string's UTF-8 storage lazily:

~~~snacc
let text: String = "café"
let scalars: View<Unicode> = text.unicode()
print(scalars.count())
~~~

`scalars.count()` returns 4 and is O(n) in the byte length.

`scalars.scalar_at(index)` returns `Unicode | Nil`. It uses a zero-based
`Int64` scalar index, returns `nil` for a negative or out-of-range index, and
is O(n) in the byte position it must scan. `View<Unicode>` deliberately has no
indexing operator or method named `at`, because that spelling would conceal
the variable-width scan.

`scalars.slice(start, end)` returns `View<Unicode> | Nil`. Bounds use Unicode
scalar positions, and locating them is O(n). A successful result refers to the
corresponding UTF-8 subrange and therefore remains valid Unicode text.

General iteration syntax and grapheme traversal are separate language and
library concerns. Their absence does not change the view representation or
scalar-access rules defined here.

### 9.4 View lifetime

Views and values that transitively contain views, including inline sums, are
borrowed types.
They may appear as temporary expressions, local bindings, and function or
method parameters. They may not be boxed, returned from user declarations,
placed in static storage, stored inside a non-borrowed value, or sent to another
thread. A borrowed type cannot cross a Rust bridge in this version.

A struct, named union, or inline sum containing a view is therefore permitted,
but the complete aggregate inherits the view's source identity and restrictions.
This rule lets `View<Byte> | Nil` and `View<Unicode> | Nil` carry successful
views without creating an owning or escaping reference.

The built-in `bytes`, `unicode`, and `slice` operations may return views because
the checker records their source. A user declaration cannot express a view
result.

The compiler infers a view borrow from creation through its last use. During
that interval, the source string cannot be moved, reassigned, or destroyed:

~~~snacc
let mut text: String = "hello"
let bytes: View<Byte> = text.bytes()
print(bytes.count())
text = "goodbye"
~~~

This is valid because the view's last use precedes the assignment.

~~~snacc
let mut text: String = "hello"
let bytes: View<Byte> = text.bytes()
text = "goodbye"
print(bytes.count())
~~~

This is rejected because replacement would invalidate a live view. Branch and
loop analysis must prove the same rule on every reachable path.

Views of views retain the original string as their source. Copying a view
copies only the borrow descriptor and does not copy text or extend the source's
lifetime.

This is a deliberately non-lexical, local borrow model: validity ends at the
last reachable use rather than at the end of the enclosing block. It expands
the call-scoped borrowing used by `Ref<T>`, but remains bounded because views
are immutable, have no user-authored result position, cannot enter stored
values, and always retain one compiler-known source identity. No source-level
lifetime parameter or general borrow graph is introduced.

### 9.5 Expected-view conversion and printing

When a parameter or built-in operation expects `View<Byte>` or
`View<Unicode>`, a `String` place automatically lends the corresponding view
for that call. The expected type makes the interpretation unambiguous:

~~~snacc
fun checksum(bytes: View<Byte>): Int64 do
    bytes.count()
end

let text: String = "hello"
let value: Int64 = checksum(text)
~~~

This conversion never occurs when no exact expected view type is available and
never converts between the two view types.

`print` accepts `Unicode` and `View<Unicode>`. It writes their UTF-8 encoding
followed by a line feed and returns the same scalar or view value. Its view
form has the exact checked behavior
`print(View<Unicode>) -> View<Unicode>`: the returned descriptor has the same
source identity and range, creates no new lifetime, and remains subject to the
original borrow. A `String` may therefore be printed directly through
expected-view conversion:

~~~snacc
let message: String = "Hello, 🍪"
print(message)
~~~

The call does not move or return `message`. Its result has type
`View<Unicode>` and borrows `message` if consumed by a surrounding expression.
When discarded as shown, its borrow ends at that call. `View<Byte>` is not
printable because arbitrary bytes need not be valid UTF-8.

## 10. Construction from views

`String.from_unicode(view)` accepts `View<Unicode>`, allocates a new string,
and copies its UTF-8 range. It always succeeds for a valid view.

`String.from_utf8(view)` accepts `View<Byte>` and returns `String | Nil`. It
validates the entire byte range and returns `nil` without producing a partial
string when the bytes are not valid UTF-8.

~~~snacc
let candidate: String | Nil = String.from_utf8(bytes)

if candidate is String(valid) then
    print(valid)
elseif candidate is Nil then
    let error: String = "invalid UTF-8"
    print(error)
end
~~~

There is no unchecked UTF-8 constructor. A future owning byte buffer may offer
a checked consuming conversion that reuses its allocation.

## 11. Concatenation

### 11.1 Operation

`left.concat(part)` borrows `left` and `part`, creates a new `String` containing
their textual contents in order, and leaves both operands available. It never
modifies the receiver.

The accepted part types are `String`, `View<Unicode>`, `Unicode`, `Byte`,
`Int64`, `UInt16`, `UInt32`, `UInt64`, `Float32`, `Dec64`, and `Bool`. Numeric
and Boolean formatting is exactly the scalar formatting used by `print`.
`Byte` is formatted as its decimal value, not inserted as an unchecked UTF-8
byte. `View<Byte>`, unions, and user-defined types are rejected.

~~~snacc
let name: String = "Ada"
let count: Int64 = 3
let message: String = "Hello, ".concat(name).concat(". Count: ").concat(count)
~~~

This is a closed built-in operation rather than user-defined overload
resolution. A later formatting specification may add one uniform way for
user-defined types to produce text.

### 11.2 Evaluation and allocation

A maximal chain of built-in `concat` calls is one concatenation plan. Parts
evaluate once from left to right. The implementation obtains each formatted
byte length, checks total-length overflow, allocates the result once, and then
copies or formats each part in order.

This rule prevents source-level chaining and interpolation from requiring one
allocation and full-prefix copy per part. The implementation may use bounded
stack scratch space for scalar formatting but may not retain a pointer to a
temporary beyond its checked lifetime.

Length overflow and allocation failure terminate through the runtime fatal
error path. They never wrap, produce invalid UTF-8, or cause undefined
behavior.

## 12. Interpolation

An interpreted string may contain expressions between `{{` and `}}`:

~~~snacc
let name: String = "Ada"
let count: Int64 = 3
let message: String = "Hello, {{name}}. Count: {{count}}"
~~~

Its semantic expansion is:

~~~snacc
let message: String = "Hello, ".concat(name).concat(". Count: ").concat(count)
~~~

The compiler represents the result as the same concatenation plan used for a
written chain. Interpolation therefore preserves the chain's evaluation,
formatting, overflow, allocation, and error semantics without materializing
intermediate strings.

An interpolation expression must have one of the accepted `concat` part types.
It may contain any ordinary expression syntax. The terminating `}}` is
recognized only at the interpolation's outer delimiter level; delimiters
inside nested string literals belong to those literals. An empty interpolation,
an unmatched opening delimiter, or an unexpected closing delimiter is an
error.

Interpolation expressions evaluate exactly once, from left to right, among the
literal segments. Raw strings never interpolate.

## 13. Stack and short strings

### 13.1 No stack-string type

This specification adds no `StackString<N>` or other fixed-capacity text type.
Such a type would require a capacity parameter, overflow behavior, and a second
set of string operations. It is not needed for the semantic string model.

A future library specification may add a fixed-capacity `TextBuffer<N>` for
embedded, real-time, or allocation-free construction. It must expose capacity
failure rather than silently allocate on the heap. It is a construction buffer,
not another spelling for `String`.

String descriptors, temporary formatting buffers, and compile-time-known
literal data may reside on the stack or in read-only static storage as ordinary
implementation choices.

### 13.2 No short-string type

There is no language-visible `ShortString`. The private `String`
representation may store sufficiently short UTF-8 contents inline. Such a
short-string optimization must preserve all ownership, move, view-lifetime,
equality, and failure semantics in this specification.

The first implementation should use the smallest correct uniform
representation. Inline storage is an optimization to consider only after
measurement; programs cannot depend on its threshold or presence.

## 14. Rejected alternatives

### Restrict views to call arguments

Matching `Ref<T>` exactly would avoid local lifetime analysis but would make a
view impossible to name, slice, inspect more than once, or use across ordinary
statements. Zero-copy byte and Unicode views would cease to be useful as
sequence values. The bounded local model provides that utility without allowing
stored or returned borrows.

### Keep every view alive to the end of its lexical block

Purely lexical borrows are simpler but reject safe reassignment after a view's
last use and make long functions sensitive to unrelated block structure.
Last-use analysis is local, deterministic, and already required across branches
and loops for move-only values.

### Make views retain shared string ownership

Reference-counted views could escape freely but would add reference-count
updates, atomicity decisions for threads, shared ownership, and delayed
deallocation. A view remains a zero-cost borrow instead.

### Copy view contents

Copying would avoid lifetime analysis but would make the type an owned string or
byte buffer rather than a view. Explicit `String.clone` and construction from a
view already express allocation.

### Preserve `UInt8` as an alias

Two source names for the same primitive conflict with the language's single
obvious spelling. `Byte` is the canonical type; the representation-oriented
`u8` remains only the numeric literal suffix and native ABI terminology.

### Add `Option<T>` or operation-specific result wrappers

Inline sums already express optional values directly as `T | Nil`. A generic
option type would duplicate that spelling, while nominal wrappers such as
`ByteAtResult` would add declarations and payload access for a simple existing
value. Domain-specific named unions remain appropriate when failures carry
distinct meanings or data.

## 15. Inline sum results

Fallible string and view operations use the inline sum types defined by
Specification 018. No `Option<T>` or operation-specific wrapper type is added:

| Operation | Result |
| --- | --- |
| `View<Byte>.at` | `Byte | Nil` |
| `View<Byte>.slice` | `View<Byte> | Nil` |
| `View<Unicode>.scalar_at` | `Unicode | Nil` |
| `View<Unicode>.slice` | `View<Unicode> | Nil` |
| `String.from_utf8` | `String | Nil` |

A direct value injects as its exact sum member, while `nil` selects `Nil`.
Callers decompose the result with `is`:

~~~snacc
let result: Byte | Nil = bytes.at(index)

if result is Byte(byte) then
    print(byte)
elseif result is Nil then
    print(0u8)
end
~~~

`View<Byte> | Nil` and `View<Unicode> | Nil` are borrowed types because one
member is a view. Their values retain the original string's source identity and
all restrictions in section 9.4. `String | Nil` is move-only because its
`String` member is move-only.

## 16. Detailed implementation plan

### Phase 1: tokens, literals, and syntax

1. Replace the source token and type spelling `UInt8` with `Byte` throughout
   the lexer, parser, syntax tree, diagnostics, examples, and conformance data.
2. Preserve `u8` as the numeric suffix that produces `Byte`; do not add a
   second suffix or accept unsuffixed literals contextually.
3. Add distinct tokens or structured token contents for Unicode scalar,
   interpreted string, raw string, literal segment, and interpolation boundary.
4. Decode escapes and validate Unicode scalars once in the lexer while
   preserving exact source spans for malformed contents.
5. Implement raw delimiters with 0 through 255 matching `#` characters,
   multiline contents, and no escape or interpolation processing.
6. Parse interpolation expressions with balanced nested syntax and produce one
   source-spanned interpolation expression rather than reparsing literal text
   in a later phase.
7. Add syntax tests for every delimiter, escape, malformed scalar, malformed
   UTF-8 input, multiline raw literal, and interpolation nesting case.

### Phase 2: semantic types and operations

1. Add concrete semantic types for `Byte`, `Unicode`, `String`,
   `View<Byte>`, and `View<Unicode>` after Specification 018's inline sum type
   is available.
2. Preserve `Byte`'s existing unsigned 8-bit operations while removing
   `UInt8` from the source namespace.
3. Make `String` move-only and destruction-requiring using RFC 016's structural
   ownership properties.
4. Add checked built-in operations for `clone`, `bytes`, `unicode`, `count`,
   `at`, `scalar_at`, `slice`, `from_unicode`, `from_utf8`, and `concat`.
5. Assign the exact inline sum from section 15 to every fallible built-in and
   inject its direct success member or contextual `nil` through ordinary sum
   checking.
6. Add expected-type conversion from `String` places to the exact requested
   view and extend `print` to `Unicode` and `View<Unicode>`.
7. Give every view-producing checked node an explicit source identity and byte
   range; do not reconstruct provenance during lowering.
8. Type interpolation parts against the closed `concat` part set and preserve
   left-to-right evaluation in the checked representation.

### Phase 3: view borrow analysis

1. Extend canonical places with immutable view-borrow identities.
2. Compute each view's live interval from creation through last use across
   sequential code, branches, and loop fixed points.
3. Reject moves, assignments, destruction, and escaping storage that can
   invalidate a live view.
4. Propagate the original source through byte and Unicode slicing.
5. Propagate borrowed-type status and source identity through structs and
   unions containing views. Reject user-authored borrowed results, owning
   storage, boxes, static storage, thread transfer, and bridge positions.
6. Permit view and borrowed-result copies without duplicating ownership.
7. Test conditional lifetimes, loop-carried views, nested slices, multiple
   immutable views, moves after last use, and all escape paths.

### Phase 4: runtime representation

1. Add an opaque owned-string representation containing valid UTF-8 and byte
   length, initially using one uniform allocation strategy.
2. Reuse the allocator and cleanup framework established by RFC 016.
3. Implement exact once-only destruction, cloning, byte equality, UTF-8
   validation, view creation, scalar decoding, and range discovery.
4. Implement `String.from_utf8` as complete validation followed by allocation;
   failure returns contextual `nil` in `String | Nil` without partial
   ownership.
5. Keep all unsafe byte-to-scalar operations inside minimal runtime boundaries
   with documented validity and bounds contracts.
6. Test empty strings, embedded zero, ASCII, multibyte scalars, maximum scalar,
   invalid UTF-8, very large lengths, and allocation failure behavior.

### Phase 5: concatenation and lowering

1. Canonicalize maximal `concat` chains and interpolation into one checked
   concatenation plan.
2. Evaluate parts once from left to right and retain their values or borrows
   until copying completes.
3. Compute formatted lengths with checked arithmetic, allocate once, and write
   exactly the computed UTF-8 byte count.
4. Reuse the established scalar `print` formatting algorithms so `concat` and
   interpolation cannot disagree with printed values.
5. Lower views as checked pointer-and-byte-length values without exposing that
   representation to source code.
6. Emit checked ownership cleanup for strings and temporaries on every normal
   scope exit.

### Phase 6: runtime ABI, tools, and documentation

1. Add the runtime imports required for length-delimited UTF-8 printing,
   allocation support not already supplied by RFC 016, and fatal string
   failures.
2. Advance the compiler/runtime ABI from RFC 016's implemented ABI successor
   to the next version and reject incompatible runtimes and cached artifacts.
3. Keep `Byte`'s native representation and bridge mapping equal to the former
   `UInt8` mapping while updating generated source assertions and diagnostics.
4. Reject `String` and both view types in Rust bridge signatures in the first
   version.
5. Update examples, the conformance runner, formatter-facing syntax data, and
   syntax highlighting where present. Do not expand the temporary workbench
   beyond what is required to keep it functional.
6. Update `LANGUAGE.md`, its leading formal grammar, and `GRAMMAR.ebnf` in the
   implementation change, keeping both grammar copies identical to the parser.
7. Run formatting, workspace checking, and the complete workspace test suite.

## 17. Required diagnostics

The implementation diagnoses at least:

- every use of the removed `UInt8` source name;
- malformed or unterminated interpreted and raw literals;
- invalid, empty, or multiple-scalar Unicode literals;
- invalid escapes and Unicode escape values;
- empty, unclosed, or unexpectedly closed interpolation;
- interpolation and concatenation values outside the accepted part set;
- constructing `Unicode` from an invalid scalar;
- every use-after-move of a string;
- invalidation or escape of a live view;
- use of an unsupported `View<T>` specialization;
- use of string indexing or Unicode-view constant-time indexing syntax;
- a string or view in a Rust bridge signature;
- any internal length overflow as a controlled fatal runtime error rather than
  undefined behavior.

## 18. Acceptance criteria

Implementation is complete only when:

1. `Byte` is the sole unsigned 8-bit source type and `UInt8` is rejected;
2. `Unicode` can contain every and only Unicode scalar value;
3. every `String` remains valid UTF-8 and immutable for its complete lifetime;
4. strings move, clone explicitly, compare by exact scalar sequence, and drop
   exactly once;
5. interpreted, interpolated, and raw literals follow their exact delimiter,
   escape, line-ending, and evaluation rules;
6. byte and Unicode views are zero-copy and expose their documented complexity;
7. no accepted program can retain a view after its source moves, is replaced,
   or is destroyed;
8. expected-view conversion is type-directed, zero-copy, and preserves the
   source borrow;
9. arbitrary bytes become strings only after complete UTF-8 validation;
10. every fallible view and UTF-8 operation returns its specified inline sum;
11. view-carrying inline sums preserve the original borrow and cannot escape;
12. concatenation and interpolation evaluate each part once from left to right
   and allocate the final string once;
13. embedded zero bytes do not truncate string operations or output;
14. no core stack-string or short-string source type is introduced;
15. strings and views cannot cross the first-version Rust bridge;
16. all invalid dynamic conditions terminate or return the specified union
    result and never cause undefined behavior;
17. `LANGUAGE.md`, both grammar copies, parser, checker, lowering, runtime, and
    diagnostics agree;
18. all lexer, parser, checker, ownership, runtime, conformance, and workspace
    tests pass.

## 19. Deferred work

- grapheme-cluster segmentation and iteration;
- Unicode normalization, case folding, collation, and locale services;
- a general iteration protocol;
- formatting of user-defined values;
- mutable byte buffers and checked allocation-reusing UTF-8 conversion;
- fixed-capacity `TextBuffer<N>`;
- measured short-string optimization;
- C-string construction and validation;
- Rust bridge adapters for `&str`, `String`, and `&[u8]`;
- returning or storing views through explicit lifetime relationships.

## 20. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [RFC 016: Box Indirection and Recursive Data Structures](016-box-indirection-and-recursive-data.md)
- [Specification 018: Inline Sum Types](018-inline-sum-types.md)
