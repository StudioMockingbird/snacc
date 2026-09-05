# Specification 019: Collections and Iteration

Status: Closed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Proposal state

This implementation-ready specification adds fixed arrays, growable lists,
general immutable contiguous views, hash maps, hash sets, checked indexing,
collection literals, `for` iteration, and borrowed-view Rust bridge adapters.

`LANGUAGE.md` remains authoritative until this specification is accepted,
implemented, and incorporated there.

This specification contains no open design questions. Section 18 fixes the
implementation order and phase boundaries.

## 2. Dependencies

This specification depends on:

- the current Rust bridge contract in [`LANGUAGE.md`](../../LANGUAGE.md),
  originally established by archived RFC 007, whose user-authored export shape
  section 16.3 deliberately replaces;
- [RFC 016](archive/016-box-indirection-and-recursive-data.md) for move-only ownership,
  deterministic destruction, and allocation cleanup;
- [RFC 017](017-utf8-strings-and-views.md) for the initial immutable
  `View<Byte>` and `View<Unicode>` borrow model;
- [Specification 018](archive/018-inline-sum-types.md) for fallible slice results such
  as `View<T> | Nil`; and
- [Specification 020](archive/020-literal-cleanup-and-numeric-radices.md) for the
  uniform `Float32` and `Float64` floating-point names.

It generalizes RFC 017's two closed view forms into `View<T>`. It does not
depend on RFC 014 or expose user-defined generic declarations. The collection
constructors in this specification are closed compiler-provided type forms,
like `Box<T>`.

## 3. Scope

This specification adds:

~~~snacc
Array<T, N>
List<T>
View<T>
Map<K, V>
Set<T>
~~~

It also adds:

- contextually typed collection literals;
- checked indexing that produces places;
- single-binding sequence and set iteration;
- key-and-value map iteration;
- non-escaping shared collection borrows; and
- generated Rust adapters for `View<T>` parameters.

It does not add tuples, comprehensions, iterators as values, generators,
closures, callbacks, sorting, functional collection operations, concurrent
collections, weak or shared ownership, user-defined iterable protocols, map
literals, mutable views, mutable string access, or bridge ownership transfer.

Stacks use `List<T>.push` and `List<T>.pop`. Trees and linked structures use
ordinary types with `Box<T>`. Queues, deques, priority queues, graphs, and other
specialized structures remain library concerns after user-defined generics are
settled.

## 4. Syntax

The grammar gains these productions and changes:

~~~ebnf
collection-type       = "Array", "<", sum-type, ",", array-length, ">"
                      | "List", "<", sum-type, ">"
                      | "View", "<", sum-type, ">"
                      | "Map", "<", sum-type, ",", sum-type, ">"
                      | "Set", "<", sum-type, ">" ;

array-length          = decimal-digits ;

parameterized-value-type
                      = "Box", "<", sum-type, ">"
                      | collection-type ;

primary-value-type    = builtin-value-type
                      | qualified-name
                      | parameterized-value-type
                      | "(", sum-type, ")" ;

block-element         = variable-declaration
                      | assignment
                      | while-statement
                      | for-statement
                      | break-statement
                      | if-form
                      | expression ;

for-statement         = "for", identifier, [ ",", identifier ],
                        "in", expression, "do", block, "end" ;

postfix               = atom, { arguments | member-suffix | index-suffix } ;
index-suffix          = "[", expression, "]" ;
collection-literal    = "[", [ expression, { ",", expression }, [ "," ] ],
                        "]" ;
collection-constructor
                      = "Map", "<", sum-type, ",", sum-type, ">", "(", ")"
                      | "Set", "<", sum-type, ">", "(", ")" ;
~~~

`for` and `in` become reserved keywords. `Array`, `List`, `View`, `Map`, and
`Set` become reserved type names.

The `array-length` in `Array<T, N>` is a non-negative, unsuffixed decimal
integer literal. `decimal-digits` is the numeric production defined by
Specification 020, so separators between decimal digits are permitted but
radix prefixes and unsigned suffixes are not. The length is a compile-time
array length, not an expression or a general constant argument.

The existing `list-literal` grammar production is renamed
`collection-literal`; the syntax remains unchanged.

`collection-constructor` is added as an `atom` alternative. It constructs only
an empty map or set; arrays and lists use collection literals. `Map` and `Set`
are reserved compiler-provided type names, so these constructor forms are
recognized by the grammar and do not depend on RFC 014's deferred user-generic
expression extensions.

## 5. Element and key types

An owning collection element or map value must be a fully resolved, finite,
storable, non-borrowed value type. It may be copyable or move-only. `Nil`,
`Ref<T>`, `View<T>`, and any type transitively containing a view are invalid as
owning collection elements or values.

`View<T>` has the same element restriction. The first version therefore has one
compiler-known source allocation per view and does not introduce nested borrow
graphs.

`Ref<View<T>>` is invalid in the first version. A reference to a view descriptor
would neither borrow the described elements nor grant write access to them,
while replacing that descriptor would require source-identity mutation not
otherwise present in the borrow model.

`Array<T, N>` requires that `N` fit the target's address space and that its
complete layout not overflow. `Array<T, 0>` is valid. Zero-sized element types
are valid in arrays and lists; logical length and destruction remain
well-defined even when no element bytes are allocated.

The first map and set implementation accepts these key types:

- `Byte`, `UInt16`, `UInt32`, `UInt64`, `Int64`, `Bool`, and `Unicode`;
- `String`; and
- a nominal represented type whose complete representation chain ends in one
  of the copyable scalar key types above.

`Float32` and `Float64` are excluded from the first version. Specification 021
removes NaN from successful Snacc execution and requires positive and negative
zero to compare equal, so float keys would be technically possible by
canonicalizing both zero encodings before hashing. They remain deferred to keep
the initial key set and hashing contract small; admitting them later requires a
separate specification of infinity, rounding-derived keys, and canonical zero
hashing. This exclusion is unconditional: Specification 021 does not add float
keys when its equality rules land. Structs, boxes, arrays, lists, unions, inline
sums, maps, sets, and represented strings are also excluded in the first
version.

The hash function, table capacity, bucket arrangement, and seed are private.
Equal keys always hash equally within a map or set. Source programs cannot
observe a hash value.

## 6. Arrays

`Array<T, N>` owns exactly `N` elements stored inline in index order. Its
length never changes.

~~~snacc
let coordinates: Array<Int64, 3> = [10, 20, 30]
let empty: Array<Byte, 0> = []
~~~

An array is copyable exactly when `T` is copyable. Otherwise it is move-only.
Moving or destroying an array applies the corresponding operation to each
element in increasing index order. An array has no capacity separate from its
length.

Compiler-provided array operations are:

~~~text
length() -> Int64
is_empty() -> Bool
view() -> View<T>
slice(start: Int64, end: Int64) -> View<T> | Nil
~~~

`is_empty()` returns `true` exactly when the compile-time length `N` is zero.

## 7. Lists

`List<T>` is an owned, growable, contiguous sequence. It preserves element
order and is always move-only because it owns an allocation.

~~~snacc
let mut numbers: List<Int64> = [10, 20, 30]
numbers.push(40)
numbers.insert(1, 15)
let removed: Int64 = numbers.remove(2)
~~~

Compiler-provided list operations are:

~~~text
length() -> Int64
is_empty() -> Bool
capacity() -> Int64
push(value: T)
pop() -> T
insert(index: Int64, value: T)
remove(index: Int64) -> T
clear()
reserve(minimum: Int64)
view() -> View<T>
slice(start: Int64, end: Int64) -> View<T> | Nil
~~~

`push`, `pop`, `insert`, `remove`, `clear`, and `reserve` require a mutable
source root. `insert` accepts indexes from zero through the current length
inclusive. `pop` on an empty list and `remove` at an invalid index cause the
defined bounds failure in section 11.

`remove` shifts later elements toward zero while preserving order. `clear`
destroys every element and sets the length to zero; it need not release
capacity. `reserve(n)` ensures capacity is at least `n`, does nothing when that
is already true, and rejects a negative value through the bounds-failure path.

List growth evaluates and owns the incoming value before reallocating, then
moves every existing element exactly once if storage changes. Failure to
allocate terminates through the runtime allocation-failure path after cleaning
any temporary incoming value; it never exposes a partially changed list.

## 8. Immutable views

`View<T>` is a non-owning contiguous range of zero or more `T` elements. It
contains a source identity, start position, and length. Copying a view copies
only that descriptor and creates another shared borrow of the same range.

Compiler-provided immutable-view operations are:

~~~text
length() -> Int64
is_empty() -> Bool
slice(start: Int64, end: Int64) -> View<T> | Nil
~~~

Indexing a view produces an immutable element place. A view cannot mutate its
elements or the source collection's length or capacity.

RFC 017's `View<Byte>` and `View<Unicode>` keep their string-specific
operations and complexity guarantees. A `View<Unicode>` over a string is a
logical scalar view backed by a UTF-8 byte range; it remains non-indexable as
specified there. Collection indexing applies only when one element has a
constant-time address within the view.

## 9. Mutable borrowed access is deferred

This specification deliberately adds only immutable `View<T>`. Basic
collection construction, reading, indexed mutation through a mutable owner,
iteration, and immutable Rust-slice interoperability do not require a mutable
view type.

Whole owned collections remain mutable through ordinary mutable roots and
`Ref<T>` parameters:

~~~snacc
fun append(values: Ref<List<Int64>>, value: Int64) do
    values.push(value)
end

let mut values: List<Int64> = [1, 2]
append(values, 3)
values[0] = 4
~~~

Adding `mut` to a parameter would not provide mutable borrowed access under the
current language rules. `mut` controls reassignment of a binding's own root; it
does not change the access rights of the value described by a shared,
copyable `View<T>`. Parameter grammar therefore remains unchanged, and this is
invalid:

~~~snacc
fun zero(mut values: View<Int64>) do // invalid
end
~~~

A later specification may introduce an exclusive mutable-view capability if
programs need to mutate a borrowed subrange or pass one to Rust as
`&mut [T]`. That decision must define exclusivity, overlap, reborrowing, and
invalidation together. This specification reserves no type name or syntax for
it.

## 10. Collection literals

A collection literal has no standalone type. An expected `Array<T, N>` or
`List<T>` supplies its collection and element type:

~~~snacc
let fixed: Array<Int64, 3> = [1, 2, 3]
let dynamic: List<Int64> = [1, 2, 3]
let names: List<String> = []
~~~

For an array, the number of literal elements must equal `N`. An empty literal
is therefore valid only for `Array<T, 0>` or any `List<T>`.

Elements evaluate exactly once from left to right and inject or convert under
the expected element type. Successfully evaluated move-only elements transfer
into the collection. If a later element fails at runtime, already constructed
elements and remaining temporaries are destroyed before propagating the
failure.

Without one exact expected array or list type, a collection literal is a type
error. The compiler does not infer array versus list or synthesize an element
sum:

~~~snacc
print([1, 2, 3]) // invalid: no expected collection type
~~~

Map and set literals are not part of this specification. Empty maps and sets
are constructed with their complete type names:

~~~snacc
let scores: Map<String, Int64> = Map<String, Int64>()
let seen: Set<Int64> = Set<Int64>()
~~~

## 11. Index places and bounds

Indexing is a postfix operation and evaluates the collection expression and
index expression exactly once, in that order.

For arrays, lists, and constant-time-addressable views, the index must be
`Int64`. Valid indexes are zero through `length() - 1`. A valid index produces
an element place rather than eagerly producing an owned element:

~~~snacc
let mut points: List<Point> = [
    Point(x: 1.0, y: 2.0),
    Point(x: 3.0, y: 4.0),
]

print(points[0].x)
points[1].x = 10.0
points[0].translate(2.0, 3.0)
~~~

Reading a copyable element copies it. Moving a move-only element out through
an index is rejected because it would leave uninitialized collection storage.
Field access and receiver calls may borrow the indexed place without moving
it. `remove`, `pop`, and `Map.take` are the ownership-transferring operations.

Assignment through an array or list index requires a mutable owning root.
Assignment through `View<T>` is always rejected.

A negative or too-large sequence index, a missing map key, `pop` on an empty
list, or another checked-operation precondition failure terminates execution
with a defined runtime collection-bounds diagnostic and a nonzero status. It
never reads invalid memory, silently wraps, or creates undefined behavior.

`slice(start, end)` uses a half-open range. It returns `nil` unless
`0 <= start <= end <= length`. An empty range is valid.

## 12. Maps and sets

`Map<K, V>` owns unique keys and their associated values. `Set<T>` owns unique
values and uses the same key restrictions. Both are move-only.

Compiler-provided map operations are:

~~~text
length() -> Int64
is_empty() -> Bool
contains(key: Query<K>) -> Bool
insert(key: K, value: V) -> Bool
delete(key: Query<K>) -> Bool
take(key: Query<K>) -> V
clear()
reserve(minimum: Int64)
~~~

Compiler-provided set operations are:

~~~text
length() -> Int64
is_empty() -> Bool
contains(value: Query<T>) -> Bool
insert(value: T) -> Bool
delete(value: Query<T>) -> Bool
clear()
reserve(minimum: Int64)
~~~

`Query<K>` is specification notation, not a source type or a user-extensible
capability. The checker selects the query form from the statically resolved map
or set key type: for a copyable scalar key it requires an exact `K` expression;
for `String` it requires `View<Byte>`, using RFC 017's expected-view conversion
so a string place or temporary string supplies that view without moving the
string. No other conversion, inference, or user-defined operation participates
in query checking. This permits efficient string lookup without adding a
general-purpose reference value:

~~~snacc
let mut scores: Map<String, Int64> = Map<String, Int64>()
let name: String = "Alice"

scores.insert(name.clone(), 10)
if scores.contains(name) then
    print(scores[name])
end
~~~

`Query` cannot be written in Snacc source, named in a declaration, stored, or
used to define a user operation. It is only a compact description of the
receiver-directed signatures of these compiler-provided map and set methods.
User-defined generic lookup APIs cannot express the same scalar-or-borrowed-
string query relationship until the language has an appropriate generic
abstraction; this is an intentional first-version limitation.

Map indexing uses the corresponding query type, produces the stored value
place, and fails through section 11 when the key is absent. Assignment through
that place requires a mutable map root and replaces the old value after the
new value is completely evaluated.

`insert` returns `true` when it adds a new key and `false` when it replaces the
value of an existing key. The replaced value is destroyed. `delete` returns
whether a value existed and destroys it. `take` removes and returns the value,
or causes a defined missing-key failure. These signatures deliberately avoid
`V | Nil`: when `V` already contains `Nil`, structural sum flattening cannot
distinguish an absent entry from a stored `nil` value.

This version intentionally has no total, non-consuming map lookup that returns
`V | Nil`; callers use `contains` followed by indexing when they need a
non-trapping read. A future collection specification may add a dedicated
option-like lookup once it defines an absence representation that remains
distinct when `V` already contains `Nil`.

Map iteration order is unspecified and may differ after any structural
mutation or between executions. Set iteration order has the same rule. Programs
that require a stable order store keys in a list or sort them using a future
library facility.

## 13. `for` iteration

Sequence and set iteration uses one value binding:

~~~snacc
for value in values do
    print(value)
end
~~~

Map iteration uses a key and value binding:

~~~snacc
for key, value in bag do
    print(key)
    print(value)
end
~~~

The checked iterable type determines the required arity:

| Iterable | Bindings | Order |
| --- | --- | --- |
| `Array<T, N>` | `value: T` | increasing element order |
| `List<T>` | `value: T` | increasing element order |
| `View<T>` | `value: T` | increasing element order |
| `Set<T>` | `value: T` | unspecified |
| `Map<K, V>` | `key: K`, `value: V` | unspecified |

This specification deliberately extends `break` to `for` bodies. Before it
lands, `break` is valid only inside `while`; after it lands, the same statement
also exits the innermost `for`. The iterable expression is evaluated exactly
once before the first iteration, even when it produces an empty collection or
view and the body executes zero times.

A `for` loop is a statement and its body is a no-result block. It cannot supply
the final value of a value-required function, method, or conditional branch.

Each element, key, and value binding is an immutable borrowed place valid only
for that iteration. It can be read, have fields selected, or receive read-only
method calls. It cannot be reassigned, moved from, returned, boxed, captured,
passed as `Ref<T>`, or used after that iteration. A copyable value may be copied
from it explicitly through ordinary value use. Mutation through the binding is
also rejected: it would require an exclusive mutable-view/reborrow capability,
which section 9 deliberately defers. Programs mutate through a mutable owning
collection root and an index place instead.

Loop bindings follow the language's function-wide unique-name rule. Their
lexical scope is the loop body. `break` exits the nearest loop. This
specification does not add `continue`.

This deliberately supersedes the current `LANGUAGE.md` rule, established with
statement-only loops, that permits `break` only inside `while`. After this
specification lands, `break` is valid inside either `while` or `for` and always
targets the nearest lexically enclosing loop of either kind.

The iterable expression evaluates exactly once before iteration. An owning
temporary remains alive through the loop and is destroyed afterwards. The
collection is borrowed for the loop's duration, so it cannot be moved,
destroyed, or structurally mutated in the body.

An empty collection executes no body iteration. This specification adds no
implicit or discarded index binding. A program that needs an index uses an
explicit `while` loop; an enumeration library facility may be considered after
iterator values exist.

## 14. Equality and conversions

Arrays of the same complete type and lists of the same element type support
`==` and `!=` when `T` supports equality. Equality compares lengths and then
elements in increasing index order. Arrays with different lengths or arrays
and lists do not compare.

Views of the same type compare their element sequences, not their source
identity, when `T` supports equality.

Maps and sets do not support whole-collection equality in the first version.
No collection supports ordered comparison or arithmetic.

There is no implicit conversion between arrays and lists. An array or list
converts to `View<T>` only for an exact expected `View<T>` call parameter,
borrowing the entire collection for that call.

## 15. Ownership, invalidation, and failure

Lists, maps, and sets uniquely own all their allocations and elements. Moving
one transfers the complete ownership; destroying one destroys every live
element exactly once and then releases its allocations. Collections cannot
contain themselves by value; recursive ownership goes through `Box<T>`.

An immutable view prevents movement, destruction, element replacement, and
structural mutation of its source until the view's last use. List operations
that may change length or capacity are structural. Every map or set insertion,
deletion, take, clear, or reserve operation is structural.

All public lengths and indexes are `Int64` and never exceed `Int64::MAX`.
Allocation-size arithmetic is checked before allocation. Bounds failure,
capacity overflow, and allocation failure are defined runtime failures rather
than undefined behavior. Cleanup already established for live owned values
runs before termination where the runtime can do so safely; no recovery value
is synthesized.

## 16. Rust bridge

### 16.1 Why owned layouts do not cross

`Array<T, N>`, `List<T>`, `Map<K, V>`, and `Set<T>` are rejected in Rust bridge
parameters and results. Their layouts, allocation strategies, capacity rules,
hash tables, and destruction logic are private to Snacc. Rust's `Vec`,
`HashMap`, and `HashSet` layouts are likewise private Rust-library details.

Treating a Snacc list as a Rust `Vec<T>` would give both languages apparent
ownership of the same allocation while they may disagree about field order,
capacity, allocator, growth, and destructor. Passing only a borrowed element
range exposes the useful data without exposing or transferring the owner.

### 16.2 Permitted view elements

This specification supersedes RFC 017's first-version blanket prohibition for
the generalized `View<T>` form: `View<T>` is permitted only as a bridge
parameter, never as a result. `T` must
have one scalar bridge representation:

| Snacc element | Rust element |
| --- | --- |
| `Byte` | `u8` |
| `UInt16` | `u16` |
| `UInt32` | `u32` |
| `UInt64` | `u64` |
| `Int64` | `i64` |
| `Bool` | `u8` |
| `Unicode` | `u32` |
| `Float32` | `f32` |
| `Float64` | `f64` |

`View<T>` maps to `&[R]` in the user-authored Rust function, where `R` is the
table's Rust element type. Strings, represented types, aggregates, boxes, sums,
and collections are not bridge view elements in the first version.

### 16.3 Physical ABI and generated adapter

A Rust slice is a Rust reference with compiler-enforced validity requirements;
it is not a stable C-ABI aggregate. Snacc therefore never fabricates an
`&[R]` value in LLVM and never relies on Rust's internal fat-pointer layout.

At the physical C ABI, each view parameter expands in place to two parameters:

~~~text
View<T> -> (*const R data, usize length)
~~~

Snacc passes a valid aligned pointer to `length` consecutive elements. The
range remains alive and is not mutated during the call. For zero length, the
generated adapter uses an aligned dangling pointer without dereferencing it,
satisfying Rust's empty-slice requirements independently of the physical
pointer value.

`cargo-snacc` generates the exported `extern "C"` adapter. The adapter receives
pointer and length, constructs a temporary Rust slice with
`core::slice::from_raw_parts`, and calls the ordinary user-authored Rust
function. The unsafe conversion exists only in generated code whose
preconditions are established by checked Snacc lowering.

For example:

~~~snacc
extern rust "snacc_user_checksum" fun checksum(
    bytes: View<Byte>
): UInt64
~~~

is implemented by ordinary Rust source shaped as:

~~~rust
pub fn snacc_user_checksum(bytes: &[u8]) -> u64 {
    checksum_crate::checksum(bytes)
}
~~~

The generated code conceptually supplies:

~~~rust
const _: fn(&[u8]) -> u64 = crate::interop::snacc_user_checksum;

#[unsafe(export_name = "snacc_user_checksum")]
unsafe extern "C" fn __snacc_bridge_checksum(
    data: *const u8,
    length: usize,
) -> u64 {
    let bytes = if length == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(data, length) }
    };
    crate::interop::snacc_user_checksum(bytes)
}
~~~

The exact generated identifier is collision-free and private. The exported
symbol remains the declaration's `snacc_user_...` link symbol.

To keep one bridge mechanism, this specification changes every `extern rust`
implementation—not only view-bearing ones—to an ordinary `pub fn` verified and
called through a generated `extern "C"` adapter. Existing scalar and `Ref<T>`
physical representations remain unchanged. Host functions remove
`#[unsafe(no_mangle)]` and `extern "C"`; the generated adapter owns both.

This supersedes the current bridge-item contract in `LANGUAGE.md`, originally
introduced by archived RFC 007. The complete source migration is:

| Current user-authored Rust item | After this specification |
| --- | --- |
| `pub unsafe extern "C" fn` | ordinary safe `pub fn` |
| user-authored `#[unsafe(no_mangle)]` | no user-authored export attribute |
| direct exported implementation | private generated `unsafe extern "C"` adapter |
| `const _: unsafe extern "C" fn(...)` assertion | ordinary `const _: fn(...)` assertion |

The current prohibition on user-authored `export_name` remains. The generated
adapter alone uses `#[unsafe(export_name = ...)]`, owns the exported symbol, and
contains the required unsafe ABI conversion. Old direct-export bridge items are
rejected rather than supported through a compatibility path.

The host function must not retain a view's pointer directly or indirectly,
store it in external state, start work that outlives the call, mutate through
another alias, or unwind across the adapter. Violating these requirements is
invalid host code.

Adding adapters and expanding view parameters changes the compiler/runtime
bridge ABI. Implementation assigns the explicit ABI successor under the shared
ABI policy and rejects older cached objects and hosts; source-only collection
changes do not bump the ABI.

## 17. Required diagnostics

The implementation diagnoses at least:

- an unknown, borrowed, non-storable, or infinitely sized element type;
- an unsupported map or set key type;
- a map or set query argument that does not match the key-directed `Query<K>`
  form, including a string query that cannot supply `View<Byte>` without moving
  the string;
- a negative, suffixed, excessive, or non-literal array length;
- an array literal whose element count differs from its length;
- a collection literal without exactly one expected array or list type;
- an element that cannot inject or convert to the expected element type;
- moving a move-only element from an index or iteration binding;
- indexing a non-indexable value or with a non-`Int64` index;
- writing through an immutable view or immutable owning root;
- moving, resizing, or destroying a borrowed collection;
- the wrong number of `for` bindings for the iterable type;
- a reused loop-binding name or a non-iterable `for` expression;
- mutation or escape through an iteration binding;
- a collection or unsupported view element in a Rust bridge;
- a view in a Rust bridge result;
- `Ref<View<T>>`; and
- a host bridge item whose ordinary Rust signature differs from the generated
  assertion.

## 18. Detailed implementation plan

### Phase 1: syntax and resolved types

1. Reserve the new keywords and type names and extend type parsing with the five
   closed collection forms.
2. Parse and source-span the array length separately from type arguments.
3. Rename the existing syntax node for list literals to collection literals
   without changing its surface syntax.
4. Add index suffixes and `for` statements with one or two bindings.
5. Add resolved canonical collection types, rejecting every invalid element,
   key, value, and length before checking bodies.
6. Keep every new syntax and type consumer exhaustive and add parser and
   resolution tests for malformed delimiters, arities, and boundaries.

### Phase 2: array and list checking

1. Propagate exact expected array and list types into collection literals and
   check their elements left to right.
2. Add compiler-known method resolution for arrays and lists without exposing
   generic declarations or ordinary overloads.
3. Check index expressions into typed element places and distinguish copy,
   borrow, assignment, and prohibited move uses.
4. Apply root mutability to indexed owning places and establish all operation
   preconditions and result types.
5. Add move and cleanup facts for partially constructed literals, removals,
   reallocation, and zero-sized elements before lowering.

### Phase 3: views and borrow checking

1. Generalize RFC 017's view identity from two closed element types to
   canonical `View<T>` and retain the specialized Unicode-string behavior.
2. Add explicit immutable full-range and slicing constructors.
3. Track source identity plus half-open ranges and reject source mutation,
   movement, or destruction while a view remains live.
4. Extend non-lexical last-use analysis through branches, loops, inline sums,
   moves, and derived immutable views.
5. Add checked view nodes carrying source, range, and exact element type so
   lowering performs no borrow decisions.

### Phase 4: iteration

1. Resolve the iterable once and select its statically known iteration form.
2. Allocate function-unique binding IDs and exact element or key/value types.
3. Represent iteration bindings as non-moving borrowed places and apply the
   collection-wide borrow across the loop body.
4. Lower sequence loops with a private increasing index and maps/sets through
   private bucket traversal; neither internal position is source-visible.
5. Integrate `break`, cleanup, zero iterations, and mutation diagnostics with
   existing loop control flow, and replace the checker rule that recognizes
   only an enclosing `while` with one that recognizes the nearest enclosing
   loop of either kind.

### Phase 5: maps and sets

1. Add compiler-private descriptors for ownership, length, capacity, buckets,
   and any iteration state; expose none of their layout to source code.
2. Implement checked allocation arithmetic and open-addressed lookup with
   tombstone handling and bounded load-factor growth.
3. Generate concrete hash, equality, move, and destruction operations for each
   admitted key and value instantiation; source-level function values are not
   introduced.
4. Implement string queries over borrowed UTF-8 bytes and scalar queries by
   value.
5. Make insertion commit only after all fallible allocation work succeeds and
   make take/delete/clear destroy or transfer each value exactly once.
6. Add collision-heavy, growth, deletion, replacement, zero-sized-value, and
   deterministic-cleanup tests.

### Phase 6: LLVM and runtime support

1. Lower arrays to target-correct inline LLVM arrays and reject layout
   overflow before type construction.
2. Lower lists to private pointer/length/capacity descriptors and add the
   smallest runtime allocation surface needed for checked growth and release.
3. Handle zero-sized elements without null dereferences or unbounded physical
   allocation.
4. Lower index places from checked element metadata and route every failed
   precondition to one structured runtime failure path.
5. Verify generated LLVM modules and treat impossible type, ownership, or
   alignment states as internal compiler errors.

### Phase 7: bridge adapters

1. Change generated bridge assertions to ordinary Rust function signatures and
   generate an exported C adapter for every Rust bridge declaration.
2. Expand each view parameter to pointer and target-sized length in the Snacc
   physical signature while retaining one source-level parameter.
3. Generate the only required unsafe slice construction with explicit empty,
   non-null, alignment, lifetime, immutability, and unwind contracts.
4. Update `cargo snacc init`, bridge-source validation, bridge fixtures, and
   diagnostics to the ordinary-`pub fn` host convention.
5. If the physical bridge changes, assign the explicit ABI successor under the
   shared ABI policy, invalidate old caches, and test mixed scalar, `Ref<T>`,
   and immutable-view signatures on every supported
   target.

### Phase 8: contract and conformance

1. Update the formal EBNF first in `LANGUAGE.md`, then copy it identically to
   `GRAMMAR.ebnf`.
2. Add the terse normative semantics from this specification to `LANGUAGE.md`
   without duplicating implementation documentation, including replacement of
   the current direct-export Rust item shape and `while`-only `break` rule.
3. Update RFC 017's implemented contract to identify the generalized view
   owner while preserving its UTF-8-specific rules.
4. Add positive and negative parser, checker, ownership, borrow, lowering,
   execution, bridge, and diagnostics tests for every rule above.
5. Add conformance programs covering arrays, move-only lists, immutable views,
   sequence and map iteration, string-keyed maps, sets, bounds failure, and
   cleanup.
6. Run formatting, workspace checking, and the complete workspace test suite.

## 19. Rejected alternatives

### Use `Ref<View<T>>`

`Ref<T>` grants access to the storage containing `T`. A `Ref<View<T>>` could
replace the view descriptor but would not grant write access to the elements it
describes. Giving it that second meaning would violate the existing reference
contract.

### Make `List<T>` a linked list

Most language and Cargo APIs need contiguous storage. Recursive linked lists
are already expressible through `Box<T> | Nil`; making them the default would
add allocation and pointer chasing to ordinary sequence use.

### Return `T | Nil` from every generic lookup

When `T` already contains `Nil`, structural sum normalization collapses the
absence alternative into a valid stored alternative. Checked ownership-
transferring operations plus explicit predicates work for every `T` without an
`Option<T>` type or a special generic result wrapper.

### Pass owned collections directly to Rust

Snacc and Rust collection layouts and allocators are private and independent.
Sharing their ownership representation would make normal growth and
destruction unsound and would freeze both implementations. Borrowed slices
expose the elements needed by Cargo crates without transferring ownership.

### Treat a Rust slice as a C-ABI struct

Rust does not promise a C layout for references to slices. The generated
pointer-and-length adapter preserves a stable physical boundary and constructs
the Rust reference only inside Rust.

### Require an index binding in every `for`

Most loops need only the element, or the key and value. Requiring an index adds
an unused binding to the common case and gives maps and sets a traversal ordinal
with no stable collection meaning. Indexed algorithms use `while`; a later
iterator library may provide explicit enumeration when requested.

## 20. Acceptance criteria

Implementation is complete only when:

1. fixed arrays and growable contiguous lists obey their stated ownership,
   layout, ordering, and cleanup rules;
2. collection literals require and honor one exact expected array or list type;
3. indexing produces checked places with no move-out hole or undefined bounds
   behavior;
4. `View<T>` is shared and immutable, and this specification adds no mutable
   borrowed-view capability;
5. non-lexical view analysis prevents every locally detectable invalidation,
   mutation, move, and escape above;
6. every sequence and set loop binds one value, and every map loop binds one
   key and one value with the specified ordering and borrow behavior;
7. `break` is valid in `while` and `for` and exits the nearest enclosing loop;
8. maps and sets accept exactly the first-version key set and preserve unique
   key/value ownership through collisions, growth, replacement, and deletion;
9. generic absence is not confused with a stored `Nil` alternative;
10. all allocation arithmetic, bounds, capacity, and missing-key failures are
   defined and never become undefined behavior;
11. owned collection layouts never cross the Rust bridge;
12. permitted view parameters reach ordinary Rust functions as valid `&[T]`
    through generated pointer-and-length adapters;
13. all user-authored Rust bridge functions use ordinary safe `pub fn` items,
    generated adapters alone own exports and unsafe ABI conversion, and old
    direct-export items are rejected;
14. view results and unsupported bridge element types fail closed;
15. the ABI version and cache compatibility advance with the bridge change;
16. parsing, resolution, checking, ownership, borrowing, and cleanup finish
    before lowering and lowering consumes only explicit checked facts;
17. `LANGUAGE.md`, both grammar copies, implementation comments, diagnostics,
    and tests agree; and
18. formatting, workspace checks, and all conformance tests pass.

## 21. Findings from Specifications 022 and 023

Specifications 022 and 023 are the first consumers of these collections outside
this document. Four findings follow. None of them reopens a rule above: this
specification's design survives contact with its first two consumers, and its
section 1 readiness claim stands.

Sections 6 and 7 already name `view() -> View<T>` on both `Array<T, N>` and
`List<T>`, which is what Specification 023 section 11 relies on when it writes
`request.view()`. No addition is needed.

**21.1 The runtime must know the private list growth ABI.** Specification 023's
`read` appends bytes into a caller's `List<Byte>` through `Ref<List<Byte>>`, so
`snacc-runtime` calls the same allocation and growth entry points this
specification's lowering uses. Section 15 keeps allocation strategy and
capacity rules private to one compiler build, which stays true -- the runtime
is inside that build -- but the coupling should be stated, because it means the
runtime versions together with the collection implementation.

**21.2 Deferred mutable views shape every I/O read signature.** Section 9
defers exclusive mutable borrowed access with sound reasoning. Specification
023 pays the bill: its `read` must *append* into an owned list rather than fill
a caller-provided range, because there is no way to lend a writable subrange.
That is workable and even has an advantage -- a loop reuses one buffer's
capacity -- but it forecloses reading into a stack `Array<Byte, N>` without an
owning list, which is the ordinary systems-programming shape and the one a
"better C" is expected to support. This is the strongest concrete argument yet
for the mutable-view capability section 9 describes, and it now has a named
caller.

**21.3 Parallel fan-out needs index-place disjointness.** Specification 022
section 7.4 defers writing into distinct elements of one array from concurrent
tasks -- `spawn work(index, results[index])` -- because nothing proves two
index places disjoint. Section 11's index places are where that proof would
live. Until it exists, every parallel fan-out needs one named local per task,
which Specification 022 calls the largest expressiveness gap in its first
version.

**21.4 Collections of move-only elements are load-bearing.** A `List<File>`
destroys every element exactly once and therefore closes every handle, which
section 15 already promises for elements generally. Specification 023 makes
that promise carry real resources rather than memory. Worth an explicit
mention: it is the mechanism by which a program can hold many open files
safely, and it is the first case where element destruction order is observable
outside the program's own memory.

## 22. References

- [`LANGUAGE.md`](../../LANGUAGE.md)
- [Historical RFC 007: Bridge Signature Verification](archive/007-bridge-signature-verification.md)
- [RFC 016: Box Indirection and Recursive Data Structures](archive/016-box-indirection-and-recursive-data.md)
- [RFC 017: UTF-8 Strings, Byte Views, and Unicode Views](017-utf8-strings-and-views.md)
- [Specification 018: Inline Sum Types](archive/018-inline-sum-types.md)
- [Specification 020: Literal Cleanup and Numeric Radices](archive/020-literal-cleanup-and-numeric-radices.md)
- [Specification 021: Truthiness and Equality](archive/021-truthiness-and-equality.md)
