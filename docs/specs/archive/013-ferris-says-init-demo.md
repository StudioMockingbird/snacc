# RFC 013: Ferris-Says Init Demonstration

Status: Closed

Document kind: Feature specification (Rust-style RFC)

## Proposal state

This RFC is implementation-ready. It changes only the generated Cargo-hosted
demonstration and its tests; it does not change Snacc language semantics, the
compiler, or the runtime ABI.

This RFC contains no open design questions. The implementation plan below fixes
template compatibility and separates ordinary offline-capable tests from the
explicit network-enabled smoke test.

## Summary

`cargo snacc init` will generate a small Cargo-hosted application that uses the
third-party `ferris-says` crate from its Rust host. The generated Snacc program
continues to run through the normal `snacc_main` path, so the example shows both
ordinary Cargo crate usage and a Snacc program in one minimal application.

The crate is named `ferris-says` in `Cargo.toml` and is imported as
`ferris_says` in Rust. This RFC uses the exact `0.3.2` API:
`ferris_says::say(input, max_width, writer)`.

The documented 0.3.2 signature is
`say<W: Write>(input: &str, max_width: usize, writer: W) -> io::Result<()>`.
The implementation smoke test must compile the generated call against the
exact dependency rather than relying only on this recorded signature.

## Motivation

The current init demo proves that a generated host can link `snacc-runtime` and
call the Snacc entry point, but it does not demonstrate why a Snacc application
is a normal Cargo package. A small, visible Rust dependency makes the package
layout and host boundary concrete without extending the Snacc language or its
bridge ABI.

## Goals

- Add one small, well-known Rust dependency to the generated init package.
- Show the distinction between a Cargo package name (`ferris-says`) and its Rust
  module name (`ferris_says`).
- Keep the generated Snacc source and `snacc_main` host contract intact.
- Keep the example deterministic enough for a smoke test without asserting the
  complete ASCII-art layout.

## Non-goals

- Adding strings, arrays, or any other new Snacc type.
- Calling `ferris-says` from Snacc through `extern rust`; the current bridge ABI
  does not support string parameters.
- Changing the direct single-file compiler workflow.
- Making `ferris-says` a workspace dependency or vendoring its source.
- Turning the init output into a production application template.

## Generated package

`cargo snacc init` will retain its current generated files and make these
changes:

### `Cargo.toml`

Add the dependency alongside `snacc-runtime`:

~~~toml
[dependencies]
snacc-runtime = "0.1"
ferris-says = "=0.3.2"
~~~

The dependency is an ordinary package dependency resolved by Cargo. It is not
part of Snacc's compiler, runtime ABI, or workspace dependency table.

A fresh generated package requires access to crates.io the first time Cargo
resolves and downloads this dependency. It can build offline only after the
exact crate and index data are present in Cargo's local cache. This changes the
Cargo-hosted demonstration only: direct `snacc` compilation remains independent
of Cargo, registries, and network access.

### `src/main.rs`

The generated Rust host will import and call `ferris_says::say` before entering
the compiled Snacc program:

~~~rust
use ferris_says::say;
use std::io::{stdout, BufWriter};

fn main() {
    let stdout = stdout();
    let writer = BufWriter::new(stdout.lock());
    say("Hello from a Snacc application!", 32, writer)
        .expect("ferris-says failed to write the demo");

    snacc_runtime::force_link();
    // SAFETY: cargo-snacc links this host with the object defining this ABI.
    let status = unsafe { snacc_main() };
    std::process::exit(status);
}
~~~

The existing `snacc_main` declaration, assertion include, runtime retention,
and generated test host remain present. The generated `src/main.nrs` remains
`print(0)` so a successful run visibly proves that control reaches the Snacc
program after the Rust crate call.

The exact Ferris art is owned by the pinned crate version. The observable demo
contract is that the output contains the supplied greeting and the Snacc
program's `0` line; tests must not depend on spacing in the art.

## Ownership and boundaries

- `cargo-snacc` owns the init template because it creates the Cargo package.
- `ferris-says` is used only by the generated host application.
- `snacc-runtime` remains responsible only for Snacc runtime symbols.
- `snacc-compiler` gains no dependency on `ferris-says`.
- No bridge declaration is added to `src/main.nrs` or `src/interop.rs`.

## Implementation plan

1. Extend the generated `Cargo.toml` dependency section with
   `ferris-says = "=0.3.2"`.
2. Rename the current post-RFC-007 `HOST_MAIN_TEMPLATE` value to
   `HOST_MAIN_TEMPLATE_PRE_FERRIS`. Define the new `HOST_MAIN_TEMPLATE` with
   the import, locked stdout writer, and `say` call while preserving the Snacc
   host sequence and generated test.
3. Make reinitialization recognize all existing generated hosts: Cargo's
   initial hello-world template, `HOST_MAIN_TEMPLATE_PRE_RFC_007`,
   `HOST_MAIN_TEMPLATE_PRE_FERRIS`, and the new `HOST_MAIN_TEMPLATE`. Only the
   newest template is written for a fresh initialization. No user-edited host
   is overwritten.
4. Update the `cargo-snacc` init integration test to assert the exact dependency,
   import, greeting, and preserved `snacc_main` call.
5. Add separate reinitialization tests for every recognized historical template
   and for rejection of a modified host.
6. Add an ignored generated-package smoke test named
   `generated_ferris_package_builds_and_runs`. It initializes a temporary
   package, lets Cargo resolve the exact crates.io dependency, runs the package,
   and checks for the greeting and Snacc's `0` output. It must not modify the
   repository workspace. The ordinary workspace suite must make no network
   request; release verification runs this named ignored test in a
   network-enabled job.
7. Update the README or package usage text to state that the generated demo's
   first Cargo build needs crates.io access unless the dependency is cached.
8. Run formatting, workspace checking, the ordinary offline-capable workspace
   tests, and the network-enabled generated-package smoke test.

## Rejected alternatives

### Vendor `ferris-says`

Vendoring would preserve an offline first build but would make a demonstration
dependency part of Snacc's distributed source and maintenance surface. The
example is specifically intended to demonstrate normal Cargo dependency
resolution, so it remains a registry dependency.

### Run the network smoke test in every workspace test

An unconditional test would make routine development and CI depend on network
availability and registry state. The structural init test remains in the
ordinary suite; the real dependency build is an explicit network-enabled
release check.

### Replace the current host without recognizing it

Treating only the pre-RFC-007 and new Ferris templates as generated would reject
untouched packages created by the current post-RFC-007 release. The additional
named compatibility template preserves safe reinitialization without weakening
the protection for user-edited files.

### Call `ferris-says` through a Snacc bridge

The current bridge has no string parameter, and the purpose of this RFC is to
demonstrate that the Rust host can use ordinary Cargo crates. Extending the
language or ABI would obscure that smaller lesson.

## Acceptance criteria

This RFC is implemented when:

1. A fresh `cargo snacc init` package declares `ferris-says = "=0.3.2"` and
   retains its `snacc-runtime` dependency and Snacc metadata.
2. Its generated Rust host imports `ferris_says::say`, calls the documented API,
   and still links and invokes `snacc_main`.
3. Running the generated application prints the Ferris greeting and the
   generated Snacc program's `0` output.
4. The generated host test remains valid and `cargo snacc test` still passes.
5. The compiler, runtime, bridge ABI, workbench, and direct CLI are unchanged.
6. The focused init and generated-package tests pass, along with the required
   formatting and workspace checks.
7. Every untouched historical host template remains eligible for safe
   reinitialization, while a modified host remains protected.
8. The ordinary workspace suite performs no network access, and the separate
   network-enabled smoke test compiles the documented 0.3.2 call.

## References

- [Ferris-Says 0.3.2 API](https://docs.rs/ferris-says/0.3.2/ferris_says/)
- [Historical RFC 005: Remove `runtime.rs` in Favor of `snacc-runtime`](archive/005-remove-runtime-rs.md)
- [Historical RFC 006: Rust Workspace Organization](archive/006-workspace-organization.md)
- [`LANGUAGE.md`](../../LANGUAGE.md)
