# RFC 005: Remove `runtime.rs` in Favor of `snacc-runtime`

Status: Closed

## Summary

Snacc will remove the repository-level `runtime.rs` source template and make
`crates/snacc-runtime/src/lib.rs` the only source implementation of universal
runtime functions. The direct compiler will embed that canonical source into
its own binary at build time, append the generated `snacc_main` host entry, and
retain its current single-`rustc` final-link workflow. Cargo-hosted applications
will continue to use the `snacc-runtime` crate normally.

The change removes the duplicated checked-in runtime implementation while
preserving offline packaged use, the existing generated `snacc_main` ABI, and
the direct `snacc <input.nrs>` workflow.

## Motivation

The current direct compiler workflow embeds the repository-level `runtime.rs`
with `include_str!`, writes a temporary copy for every program, and invokes
`rustc` on that copy together with the generated object. The file contains both
the `snacc_main` host entry call and universal printing functions.

The `snacc-runtime` crate already owns the universal printing functions and
provides `force_link()` to retain those exported symbols in Cargo-hosted final
links. Maintaining both checked-in implementations creates two runtime
contracts that can drift independently.

Generating a temporary Cargo package would not solve that problem more simply.
It would require repository-relative source at runtime, so the separately
packaged `snacc` executable could not find its dependency. Shipping crate source
or a prebuilt rlib would create new packaging and version-matching contracts,
and resolving a fresh temporary Cargo package would add work without defining a
shared target cache. Embedding the canonical crate source when `snacc` itself is
built keeps the packaged compiler self-contained and preserves the existing
small direct-link path.

## Decision

`crates/snacc-runtime/src/lib.rs` is the only checked-in implementation of
universal Rust-side runtime behavior. Snacc will not maintain a second
repository-level runtime source template.

The stable generated-program boundary remains:

```text
Snacc object
  exports snacc_main() -> i32
  calls snacc_print_* symbols
        |
        +--> direct generated Rust host
        |      contains the build-time embedded canonical runtime source
        |      declares and calls snacc_main exactly once
        |
        +--> Cargo-hosted Rust host
               depends on snacc-runtime
               calls snacc_runtime::force_link()
               declares and calls snacc_main exactly once
```

The host declaration of `snacc_main` remains generated. The canonical runtime
source does not define `snacc_main`; the generated Snacc object does.

## Language contract

This RFC changes build and runtime packaging only. It does not change Snacc
syntax, type rules, evaluation order, supported declarations, or generated
language-level behavior. The normative language contract remains
[`LANGUAGE.md`](../../LANGUAGE.md).

## Current implementation

The following current paths are in scope:

- `apps/snacc/src/main.rs` delegates executable construction to
  `snacc-driver`.
- `apps/snacc/tests/conformance.rs` uses the shared native driver for the run
  corpus.
- `crates/snacc-runtime/src/lib.rs` already exports the universal printing
  functions and defines `force_link()`.
- `tests/fixtures/cargo-hosted/src/main.rs` already demonstrates the target
  host contract by calling `snacc_runtime::force_link()` before `snacc_main`.

After this RFC is implemented, no production or test source may reference the
removed root `runtime.rs`. References to the canonical
`crates/snacc-runtime/src/lib.rs` are required by the direct workflow.

## Target design

### Direct single-file workflow

The direct `snacc <input.nrs> [-o <output>]` workflow will retain its current
observable command behavior:

1. Read and validate the Snacc source.
2. Emit the native object through the existing compiler library API.
3. Create a private temporary build directory.
4. Write a generated Rust host consisting of the exact runtime source embedded
   from `crates/snacc-runtime/src/lib.rs` when `snacc` was built, followed by the
   `snacc_main` declaration and host entry point.
5. Invoke `rustc` once with the emitted object as a link argument.
6. Place the resulting executable at the requested output path.
7. Remove the temporary build directory when the operation completes.

The compiler embeds the canonical source with `include_str!`; the manifest-
relative path is updated when RFC 006 moves the direct CLI package. The
generated host suffix is equivalent to:

```rust
unsafe extern "C" {
    fn snacc_main() -> i32;
}

fn main() {
    force_link();
    // SAFETY: the linked Snacc object defines snacc_main with this ABI.
    let status = unsafe { snacc_main() };
    std::process::exit(status);
}
```

The host source may be written to a temporary `host.rs`, but the name
`runtime.rs` must not be used. Materializing the compiler-embedded canonical
source in that temporary host is not a second checked-in implementation. No
repository path is resolved when the packaged compiler runs.

The direct workflow requires `rustc` and the platform linker exactly as it does
today. A missing Rust compiler or linker produces an actionable diagnostic. It
does not require Cargo, a registry, network access, a runtime source directory,
or a prebuilt runtime library beside `snacc.exe`.

The initial direct workflow intentionally retains per-invocation `rustc`
compilation of the small generated host. It has no Cargo target directory or
runtime artifact cache. Caching final-link inputs is outside this RFC and must
be justified by measured direct-build latency.

### Cargo-hosted workflow

The Cargo-hosted workflow remains based on the application host contract:

- Generated hosts depend on `snacc-runtime` through Cargo.
- Hosts call `snacc_runtime::force_link()` before the `snacc_main` call.
- Application-specific Rust dependencies remain in the application package.
- `cargo-snacc` continues to own Cargo metadata discovery, object paths,
  profile selection, and final-link orchestration.

No generated application source may copy the implementations from
`crates/snacc-runtime`.

### Runtime ABI

The following functions remain exported from `snacc-runtime` with their current
C ABI and behavior:

| Symbol | ABI | Behavior |
| --- | --- | --- |
| `snacc_print_f64` | `extern "C" fn(f64)` | Writes a floating-point value and newline. |
| `snacc_print_i64` | `extern "C" fn(i64)` | Writes a signed integer and newline. |
| `snacc_print_bool` | `extern "C" fn(u8)` | Writes `true` for nonzero and `false` for zero, followed by a newline. |
| `snacc_print_nil` | `extern "C" fn()` | Writes `nil` and newline. |

The generated object continues to call those symbols by name. Any ABI change
is outside this RFC and requires a separate language or runtime specification.

## Implementation plan

### 1. Centralize direct host linking

- Add a concrete helper for writing the temporary generated host and linking
  one emitted object with `rustc`.
- Keep filesystem paths, Rust compiler invocation, output handling, and subprocess
  diagnostics in the native-driver/application layer rather than in the
  compiler frontend or LLVM lowering module.
- Embed `crates/snacc-runtime/src/lib.rs` in the direct compiler at build time.
- Use explicit paths for the temporary host source, object, and final
  executable.
- Write the embedded canonical runtime source before the generated host suffix.
- Pass the object path as a linker argument and copy or move `rustc`'s produced
  executable to the requested output path only after a successful link.
- Preserve the existing failure distinction for invalid Snacc input, missing
  Rust tooling, failed linking, and failed executable output.

### 2. Remove the direct runtime template

- Delete the old runtime-template constant from the direct CLI.
- Remove the temporary `runtime.rs` path and the code that writes the old
  embedded template.
- Replace the embedded duplicate with a constant containing the canonical
  `snacc-runtime` crate source.
- Retain the direct single-`rustc` link helper and generate `host.rs` from that
  canonical source plus the host suffix.
- Delete the repository-level `runtime.rs` after all references are removed.

### 3. Update conformance coverage

- Remove the old runtime-template constant and temporary `runtime.rs` from
  `apps/snacc/tests/conformance.rs`.
- Compile the run corpus through the same embedded canonical runtime source and
  generated-host contract used by the direct workflow.
- Keep the existing output assertions and normalize Windows newlines as they
  do today.
- Retain the Cargo-hosted fixture tests, including the `force_link()` and
  `snacc_main` host behavior.

### 4. Verify the runtime crate contract

- Keep `crates/snacc-runtime/Cargo.toml` and `crates/snacc-runtime/src/lib.rs`
  as checked-in workspace source.
- Verify the direct compiler's embedded source equals that file at build time
  by construction rather than through a copied constant or generated mirror.
- Keep `force_link()` small and explicit; it is the host's required retention
  hook for the exported runtime symbols.
- Add focused tests only where they verify the ABI or host-linking contract.
  Do not add a second runtime implementation to make tests easier to run.

### 5. Update documentation and generated templates

- Update active documentation that describes direct `runtime.rs` compilation.
- Update any `cargo snacc init` or host-generation templates to depend on
  `snacc-runtime` and call `force_link()`.
- Update RFC 006's moved direct-CLI source path without changing which canonical
  runtime file is embedded.
- When `tools/package-windows.ps1` includes the direct compiler, run a packaged
  single-file compile-and-execute smoke test from the temporary package with no
  repository-relative runtime path available.
- Leave archived specifications immutable. Later behavior is recorded by this
  RFC and by any subsequent active specification.
- Add a repository-wide reference check that fails if production code or active
  templates reintroduce `runtime.rs`.

## Testing plan

The implementation is complete only when all of the following pass:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- Frontend and type-check tests that do not require native linking
- LLVM lowering tests
- The run/conformance corpus using `snacc-runtime`
- Cargo-hosted application build, run, and test flows
- Direct single-file compilation and execution on the supported Windows
  target
- A packaged direct-compiler smoke test run from outside the repository, with
  no runtime source tree or network access available
- A negative test for missing Rust tooling, with an actionable diagnostic
- A repository search confirming that no production or active-template code
  embeds, writes, or compiles `runtime.rs`

The implementation must not require a separately installed LLVM SDK beyond the
vendored LLVM contract owned by the existing toolchain specifications. The
runtime-crate migration must not change which LLVM version is selected or how
LLVM objects are emitted.

## Compatibility and migration

This is source-compatible for Cargo-hosted Snacc applications because their
host contract already depends on `snacc-runtime`. The direct single-file
workflow remains available, but its final-link implementation changes from a
temporary hand-written runtime source to a generated host containing the
build-time embedded canonical `snacc-runtime` source.

The following are intentionally removed:

- The repository-level `runtime.rs` file.
- The duplicated printing functions that were compiled from that file.

The following remain stable:

- The `snacc_main` symbol and return ABI.
- The exported `snacc_print_*` symbol names and signatures.
- The direct command shape and output-path behavior.
- The Cargo-hosted application dependency and host pattern.

## Non-goals

- Changing Snacc language syntax or semantics.
- Changing the LLVM version, target, or Inkwell integration.
- Moving compiler frontend or LLVM lowering code into `snacc-runtime`.
- Adding function values, closures, nested functions, or implicit shared state.
- Designing new runtime types or a general foreign-function interface.
- Removing the direct single-file workflow.
- Making `snacc-runtime` an application-specific dependency container.

## Acceptance criteria

This RFC is implemented when:

1. `runtime.rs` no longer exists in the repository.
2. No production or active-template source references `runtime.rs`.
3. Direct single-file builds link successfully through one `rustc` invocation
   using the canonical runtime source embedded in `snacc.exe` at build time.
4. Cargo-hosted builds continue to compile, run, and test successfully.
5. The generated Snacc object resolves all universal runtime symbols from
   `snacc-runtime`.
6. The runtime ABI table above remains covered by focused or integration tests.
7. The packaged direct compiler works outside a repository checkout without
   runtime source files, Cargo, registry access, or network access.
8. Missing `rustc`, the platform linker, or object-link inputs fail closed with
   structured, actionable diagnostics.
9. The required formatting, checking, and test commands pass.
