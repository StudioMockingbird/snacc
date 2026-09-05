# RFC 028: Descriptor ABI Boundary

**Status:** Proposed

## Summary

Snacc currently represents several runtime values as multi-word descriptors,
including `String`, `View<T>`, `List<T>`, `Map<K, V>`, and `Set<T>`. The LLVM
backend calls Rust `extern "C"` runtime functions for these values. On the
Windows target, returning or passing a descriptor by value does not always use
the direct aggregate convention emitted by the backend: sufficiently large
aggregates may use hidden return storage or indirect argument passing.

This RFC isolates that boundary contract from collection and string semantics.
The current implementation uses this provisional form for the supported
descriptor-bearing runtime calls. The RFC remains open for the target matrix,
adapter-generation strategy, and long-term public symbol policy.

## Problem

Declaring a Rust function that returns a three-word descriptor as an LLVM
function returning the descriptor directly can shift the visible arguments at
the C ABI boundary. The same class of mismatch can corrupt descriptor inputs
when the backend passes them by value. The resulting failure is target-specific
and can appear as an invalid pointer, length, or capacity rather than as a
compile-time diagnostic.

The problem affects more than one feature and should not be solved by adding
feature-specific workarounds to collection lowering.

## Provisional rule

The backend and runtime bridge shall use these forms for every descriptor
crossing the native boundary:

```text
descriptor result:  void operation(descriptor* out, ...)
descriptor input:   ... operation(const descriptor* value, ...)
descriptor mutation: ... operation(descriptor* value, ...)
```

Scalar results and scalar parameters retain their existing ABI mappings. A
pointer-output adapter may call an internal Rust implementation that returns a
descriptor by value, provided the adapter itself is the symbol linked by LLVM.

## Scope

The follow-up implementation must inventory and normalize all runtime symbols
for:

- `String` construction, cloning, concatenation, equality, printing, views,
  UTF-8 conversion, and cleanup;
- `View<Byte>` and `View<Unicode>` length, equality, indexing, and slicing;
- collection descriptor reads, iteration, equality, cleanup, and String-key
  queries; and
- generated bridge signatures and ABI conformance tests for each descriptor
  family.

The implementation must document the target ABI assumption beside each
adapter family and verify the generated LLVM declarations against the Rust
function signatures.

## Open design questions

1. Should the public runtime symbol set expose only pointer-safe adapters, or
   should by-value Rust helpers remain exported for runtime unit tests?
2. Should descriptor adapters be handwritten, generated from a single ABI
   table, or represented as a compiler-side intrinsic family?
3. Which targets besides Windows require indirect aggregate handling, and what
   is the minimum supported target matrix for the first closure?
4. Should the compiler add an explicit ABI-version bump whenever a descriptor
   calling convention changes?

## Acceptance criteria

1. No LLVM declaration for a descriptor-bearing runtime call relies on an
   unverified by-value aggregate convention.
2. String, view, list, map, and set hosted tests pass on the supported Windows
   target, including construction, read, mutation, iteration, equality, and
   cleanup.
3. A deliberate ABI mismatch test fails during compiler verification or link
   preparation with a structured diagnostic rather than executing corrupted
   descriptor data.
4. `LANGUAGE.md` documents only the stable language behavior; ABI mechanics
   remain implementation documentation and tests.

## Non-goal

This RFC does not redesign `String`, `View<T>`, collections, ownership, or
their source syntax. It only defines the native boundary required to lower the
already specified behavior safely.
