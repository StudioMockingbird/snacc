# RFC 003: Cargo-Hosted Snacc Applications

Status: Completed

## Summary

Snacc applications are Cargo packages containing a Snacc entry module and a
small Rust host binary. Application authors use ordinary `Cargo.toml`
dependencies and may select any Rust crate supported by their target. Rust code
adapts selected crate APIs to Snacc through an explicit, typed native ABI.

The installed `cargo-snacc` executable provides commands such as:

~~~text
cargo snacc init
cargo snacc check
cargo snacc build
cargo snacc run
cargo snacc test
cargo snacc doctor
~~~

`cargo-snacc` reads Cargo metadata, invokes the Snacc compiler library to emit a
content-addressed native object, and asks `cargo rustc` to compile the Rust host
and link that object into the final executable. Cargo owns dependency
resolution, feature selection, Rust compilation, incremental compilation, and
the final native link. Snacc owns its forward-only language pipeline through
LLVM object emission.

This RFC does not transpile Snacc to Rust, replace rustc, use `RUSTC_WRAPPER`,
or expose Rust's unstable native ABI.

## Motivation

Snacc already requires Rust to link generated programs, and Rust plus Cargo are
explicit prerequisites for installing Snacc. Making each application a Cargo
package therefore does not introduce an otherwise avoidable toolchain
requirement.

Application authors need to:

- Select their own Rust crates and versions.
- Use normal Cargo features, lockfiles, registries, and workspaces.
- Benefit from Cargo's dependency cache and incremental compilation.
- Keep application logic in native `.nrs` files.
- Compile Snacc through its typed program and Inkwell backend without
  generating Rust source.
- Run one coherent command rather than manually coordinating compilers and a
  linker.

The existing CLI compiles one Snacc file, writes a temporary object, recompiles
`runtime.rs`, and invokes rustc directly. That model cannot resolve an
application-specific Cargo dependency graph and rebuilds Rust runtime code for
each executable. Cargo-hosted applications remove both limitations.

## Decision

Snacc adopts the following application build graph:

~~~text
Snacc source
  -> lexer
  -> syntax tree
  -> typed program
  -> Inkwell
  -> native object
                         \
Cargo.toml                \
  -> Cargo dependencies    -> rustc final link -> executable
Rust host and bridges     /
                         /
snacc-runtime -----------/
~~~

The canonical user command is `cargo snacc`. A package installed with a binary
named `cargo-snacc` is automatically available to Cargo as that subcommand.

`cargo-snacc` must invoke Cargo through the command-line interface and consume
`cargo metadata --format-version 1` and Cargo's versioned JSON build messages.
It must not link Cargo as a Rust library because Cargo's library API is not a
stable integration boundary.

## Goals

- Make Cargo the package manager and final build orchestrator for Snacc
  applications.
- Let every application declare arbitrary target-compatible Rust crates.
- Preserve Snacc's independent syntax, type checker, and LLVM lowering.
- Keep Rust-specific traits, generics, lifetimes, macros, and async machinery
  on the Rust side of an explicit boundary.
- Provide predictable `cargo snacc ...` commands in standalone packages and
  Cargo workspaces.
- Avoid rebuilding unchanged Rust dependencies or unchanged Snacc objects.
- Reject missing, ambiguous, unsupported, and ABI-incompatible configurations
  before the final link whenever possible.
- Preserve Cargo profiles, features, registries, lockfiles, target directories,
  offline mode, and workspace package selection.

## Non-goals

- Importing arbitrary Rust APIs directly into the Snacc type system.
- Calling symbols through Rust's native ABI.
- Transpiling Snacc to Rust.
- Replacing or wrapping rustc.
- Modifying or forking Cargo.
- Making `cargo build` or `cargo run` compile Snacc without the `snacc`
  subcommand in the initial implementation.
- Supporting cross-compilation before LLVM lowering is target-aware.
- Designing the complete future ABI for strings, collections, callbacks, async
  values, and user-defined aggregate types in this RFC.
- Replacing the LLVM 22 toolchain integration described by RFC 002.

## Terminology

- **Snacc entry**: The root `.nrs` source file selected by package metadata.
- **Host binary**: The Rust binary target that owns process startup and calls
  the compiled Snacc entry point.
- **Bridge**: Rust code that adapts a crate API to the Snacc ABI.
- **Binding declaration**: A checked Snacc declaration describing a bridge
  function or value.
- **Compiler object**: The native object emitted from a validated typed Snacc
  program.
- **Build identity**: The digest of every input that affects compiler-object
  contents or compatibility.

## Installation

The distribution package exposes at least one executable:

~~~text
cargo-snacc
~~~

The default Windows installation is a prebuilt, versioned package containing:

~~~text
cargo-snacc.exe
LLVM-C.dll
<other required non-system runtime DLLs>
integrity manifest
LLVM and Snacc license notices
~~~

The installer must preserve these files in one directory and make that directory
available on `PATH`. It may use Cargo's user binary directory as that location.
Cargo recognizes any `cargo-snacc` executable on `PATH` as the `cargo snacc`
subcommand; Cargo does not need to have compiled the executable itself.

The package may also install a direct `snacc` executable for compiler
development, object inspection, editor integration, and compatibility with
single-file workflows. `cargo-snacc` is the application-facing entry point and
must not shell out to a separately versioned `snacc` executable. It links the
`snacc-compiler` library so orchestration and compilation use one version.

The prebuilt package uses its adjacent LLVM runtime for Inkwell object emission.
Application programmers do not install an LLVM SDK and must not depend on the
LLVM implementation embedded privately in rustc. `cargo snacc doctor` must
validate Rust, Cargo, the supported native linker, the packaged LLVM runtime,
and runtime DLL discovery. RFC 002 owns the Windows LLVM build-time and runtime
distribution contracts.

Release producers run `tools/build-snacc.ps1 -Release`, which assembles both
executables and the matching LLVM runtime under the project root `bin/`, then
run `tools/package-windows.ps1 -SnaccLicensePath <approved-license-file>`. The
packager refuses to overwrite an existing destination, checks the direct DLL
dependency closure, copies only runtime inputs, writes SHA-256 hashes and sizes
to `integrity.json`, and runs `cargo snacc doctor` from the assembled directory
before publishing it. The license path is mandatory because the release
workflow must not invent or silently omit the project's legal terms.

A source installation remains supported for compiler contributors and advanced
users:

~~~text
cargo install cargo-snacc --locked
~~~

Because this command compiles Snacc, it requires the full LLVM developer
distribution and environment described by RFC 002 before Cargo starts. It is
not the default application-programmer installation path and must fail with
actionable setup guidance when that contract is absent. Neither Cargo nor rustc
provides a supported LLVM C API installation for Inkwell.

## Workspace organization

The implementation should evolve into three concrete Rust crates:

~~~text
crates/
  snacc-compiler/
    lexer, parser, checker, typed program, LLVM lowering
  snacc-runtime/
    universal Rust host support and stable ABI definitions
  cargo-snacc/
    Cargo discovery, command parsing, orchestration, and diagnostics
~~~

`snacc-compiler` must not know about Cargo manifests, workspaces, package
selection, command-line presentation, or process execution. Its public API
accepts source plus explicit compilation options and returns a native object or
structured diagnostics.

`cargo-snacc` owns filesystem discovery, Cargo subprocesses, artifact paths,
profiles, features, targets, and execution.

`snacc-runtime` owns only universal runtime behavior required by the language.
Application-specific crates remain ordinary dependencies of the application's
Rust host.

## Application layout

`cargo snacc init hello` creates:

~~~text
hello/
  Cargo.toml
  src/
    main.nrs
    main.rs
    interop.rs
~~~

The initial manifest is:

~~~toml
[package]
name = "hello"
version = "0.1.0"
edition = "2024"

[dependencies]
snacc-runtime = "0.1"

[package.metadata.snacc]
schema-version = 1
entry = "src/main.nrs"
host-bin = "hello"
~~~

The Rust host contains no application policy:

~~~rust
mod interop;

unsafe extern "C" {
    fn snacc_main() -> i32;
}

fn main() {
    snacc_runtime::force_link();
    let status = unsafe { snacc_main() };
    std::process::exit(status);
}

#[test]
fn snacc_entry_succeeds() {
    snacc_runtime::force_link();
    assert_eq!(unsafe { snacc_main() }, 0);
}
~~~

The default `interop.rs` is empty except for documentation pointing to the
binding workflow. The user adds dependencies with normal Cargo commands:

~~~text
cargo add regex
cargo add serde_json
~~~

`cargo-snacc` must not maintain a second dependency manifest or lockfile.

## Manifest contract

The initial `package.metadata.snacc` schema contains:

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `schema-version` | integer | Yes | Manifest schema; currently `1` |
| `entry` | string | Yes | Package-relative Snacc entry file |
| `host-bin` | string | Yes | Cargo binary target receiving the object |

Unknown keys, missing keys, and unsupported schema versions are errors. This
keeps configuration fail-closed and prevents an older tool from silently
misreading a newer manifest.

Paths are resolved relative to the directory containing the selected
`Cargo.toml`. The resolved entry must remain within that package directory
unless a later workspace-source RFC explicitly permits shared source roots.

The initial implementation supports exactly one Snacc entry and one host binary
per Cargo package. Multiple Snacc applications require separate workspace
packages. This keeps target selection and final-artifact discovery
unambiguous.

## Package and target selection

`cargo-snacc` must follow Cargo's workspace model:

1. Locate the applicable manifest.
2. Run `cargo metadata --format-version 1`.
3. Select the package containing the current directory unless `--package` is
   provided.
4. Read that package's Snacc metadata.
5. Resolve `host-bin` against its declared Cargo binary targets.

Failure cases include:

- No selected package.
- Multiple possible packages without an explicit selection.
- Missing `package.metadata.snacc`.
- Missing or non-file entry.
- Missing host binary.
- A host name that resolves to multiple targets.

These failures belong to orchestration and must be reported before Snacc
compilation.

## Command interface

### `cargo snacc init`

~~~text
cargo snacc init [PATH] [--name NAME]
~~~

The command:

1. Invokes the current Cargo executable to initialize a binary package.
2. Adds the Snacc runtime dependency and metadata.
3. Creates `src/main.nrs`, the Rust host, and `src/interop.rs`.
4. Refuses to overwrite existing non-template files.

The Cargo executable is taken from the `CARGO` environment variable when Cargo
provides it, otherwise from `cargo` on `PATH`.

### `cargo snacc check`

~~~text
cargo snacc check [CARGO SELECTION AND FEATURE OPTIONS]
~~~

The command performs:

1. Snacc lexing, parsing, and type checking without LLVM lowering.
2. `cargo check` for the selected Rust package and feature set.

Snacc source failures stop before Cargo work. A valid Snacc program does not
make an invalid Rust bridge acceptable, so Rust checking remains required.

### `cargo snacc build`

~~~text
cargo snacc build [--release | --profile NAME]
                  [--package PACKAGE]
                  [--features FEATURES]
                  [--all-features]
                  [--no-default-features]
                  [--locked | --frozen | --offline]
~~~

The command emits or reuses the compiler object and invokes `cargo rustc` for
the selected host binary. It prints the final executable path on success.

`--release` is an alias for the Cargo `release` profile. `--release` and
`--profile` are mutually exclusive.

### `cargo snacc run`

~~~text
cargo snacc run [BUILD OPTIONS] [-- PROGRAM_ARGUMENTS...]
~~~

The command performs `build`, locates the executable from Cargo's JSON
`compiler-artifact` message, and runs it. Everything after `--` is passed
verbatim to the application. The command returns the application's exit code
when launching succeeds.

### `cargo snacc test`

~~~text
cargo snacc test [BUILD OPTIONS] [FILTER] [-- TEST_ARGUMENTS...]
~~~

The first implementation supports:

- Snacc compiler and language conformance tests in the Snacc repository.
- Rust unit tests for application bridge code through Cargo.
- End-to-end application tests that build the host with a Snacc test object.

The generated host contains a Rust unit test that calls `snacc_main` and checks
its success status. `cargo snacc test` emits or reuses the Snacc object, asks
`cargo rustc --bin <host> -- --test` to build that host as a test harness with
the object linked only into that invocation, then executes the exact artifact
reported by Cargo. Application bridge unit tests in `interop.rs` run in the
same harness. An optional filter and arguments after `--` are forwarded to the
Rust test harness. Global `RUSTFLAGS` are never used.

Native Snacc test declarations remain future work; compiler and language
conformance suites continue to run in the Snacc repository.

### `cargo snacc clean`

~~~text
cargo snacc clean [--package PACKAGE] [--all]
~~~

Without `--all`, the command removes only content-addressed Snacc artifacts for
the selected package. With `--all` it delegates to `cargo clean` after removing
Snacc-owned state.

### `cargo snacc doctor`

The command performs read-only validation of:

- The Cargo executable and version.
- rustc and the selected host triple.
- The packaged LLVM 22 runtime used by `snacc-compiler`.
- Runtime discovery and version compatibility of `LLVM-C.dll` on Windows.
- The selected native linker.
- Cargo metadata parsing for the current package, when present.
- Required Snacc metadata and source files.

It reports every independent failure in one invocation and returns failure if
any required capability is absent. A prebuilt installation must not diagnose
missing `llvm-config`, headers, or import libraries because those are
source-build inputs rather than runtime capabilities.

## Compilation algorithm

`cargo snacc build` follows this sequence:

1. Discover the manifest and selected package.
2. Resolve Cargo features, profile, target, and host binary arguments.
3. Read the Snacc entry and its transitive Snacc imports.
4. Run the forward-only Snacc frontend.
5. Stop and render structured diagnostics on any source failure.
6. Compute the build identity.
7. Reuse the existing object when its validated cache entry exists.
8. Otherwise lower the typed program through Inkwell and atomically write the
   object.
9. Invoke `cargo rustc` for exactly the selected host binary.
10. Pass the compiler object only to the final rustc invocation.
11. Parse Cargo JSON messages and identify the executable.
12. Return the artifact or execute it for `run`.

Snacc compilation never consumes Rust source or Cargo dependency output.
Cargo/rustc never consumes Snacc syntax or typed nodes. Their only compilation
boundary is the native object and declared ABI.

## Invoking Cargo

The conceptual Cargo command is:

~~~text
cargo rustc --message-format=json-render-diagnostics \
    --package <package> \
    --bin <host-bin> \
    <profile, feature, target, and network options> \
    -- \
    -C link-arg=<absolute compiler object path>
~~~

`cargo rustc` documents that arguments after `--` are passed to the final
compiler invocation and not to dependency compilations. This is the required
behavior.

`cargo-snacc` must:

- Construct process arguments directly without shell interpolation.
- Preserve relevant Cargo and rustup environment variables.
- Forward Cargo stdout/stderr in a way that preserves terminal color choices.
- Parse JSON messages without scraping human-formatted output.
- Select exactly one host target when supplying final compiler arguments.
- Avoid `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `RUSTC`, and
  `RUSTC_WRAPPER` manipulation.

If platform link ordering makes direct `link-arg` objects unreliable, the
fallback is a small static archive placed in the same content-addressed
directory and linked through target-specific rustc arguments. Such a fallback
must be verified on every supported target and must not change the language ABI.

## Build identity and cache

The compiler object path is content-addressed:

~~~text
target/
  snacc/
    <target>/
      <profile>/
        <package-id>/
          <build-identity>/
            app.obj
            manifest.json
~~~

On platforms using `.o` rather than `.obj`, the native suffix is selected from
the target contract.

The build identity includes:

- Contents and package-relative paths of every participating Snacc source.
- Snacc compiler version and backend build identifier.
- Snacc language edition when introduced.
- Snacc ABI version.
- LLVM target triple, CPU policy, and enabled target features.
- Optimization, debug, overflow, and panic policies affecting generated code.
- Relevant profile values.
- Binding declaration contents.

The identity must not include absolute workspace paths, timestamps, temporary
paths, terminal settings, or unrelated Cargo dependencies.

An object is reusable only when `manifest.json` exists, is valid, and matches
the expected identity contract. Unknown cache schema versions fail closed and
cause regeneration.

Objects are written to a temporary sibling and atomically renamed after
successful emission. Failed compilation must not leave a cache entry that can
be mistaken for valid output.

Using a content-addressed object path also makes Cargo's final rustc arguments
change whenever Snacc output-relevant inputs change. Cargo therefore cannot
reuse a host executable linked against an older compiler object merely because
the pathname remained constant.

## Profiles and optimization

Cargo profile selection drives both sides of the executable:

| Cargo profile property | Snacc behavior |
| --- | --- |
| `opt-level` | Mapped to an explicit LLVM optimization policy |
| `debug` | Controls Snacc debug metadata when supported |
| `overflow-checks` | Controls checked arithmetic when the language defines it |
| `panic` | Does not change Snacc failure semantics unless explicitly specified |
| `lto` | Rust-side only until cross-language LTO has a separate verified design |

Unsupported profile values must not be guessed or silently ignored when they
would change observable Snacc semantics. Optimization-only values may use a
documented conservative mapping.

The initial implementation may support only Cargo `dev` and `release` profiles.
Other profiles must fail with a diagnostic until their mapping is implemented.

## Target handling

The target used by Snacc and rustc must be identical.

The initial implementation supports only the host target because current LLVM
lowering creates a host target machine. Supplying `--target` must fail before
code generation until `snacc-compiler` accepts an explicit target specification
and emits a matching object.

Cross-compilation support requires:

- Target-aware LLVM initialization and target machine creation.
- Target data layout and calling convention selection.
- Rust standard-library availability for the target.
- A compatible linker.
- ABI conformance tests for the target.

The compiler must never emit a host object and pass it to a different Cargo
target.

## Host and runtime ABI

The Rust host owns `main`. The compiler object exports:

~~~text
snacc_main: extern "C" fn() -> i32
~~~

The host calls `snacc_main` exactly once in the default executable template and
uses its return value as the process exit status.

`snacc-runtime` provides universal imports required by generated code. The
initial set includes numeric and boolean printing. As the language grows, every
runtime symbol must have:

- A stable symbol name.
- A documented C-compatible signature.
- Defined ownership and lifetime rules.
- A versioned semantic contract.
- Conformance tests on each supported target.

The `snacc_` symbol prefix is reserved for compiler and runtime use.
Application bridge exports use `snacc_user_` followed by a deterministic
module-and-function name or an explicitly declared link name.

Rust panics and unwinds must not cross into Snacc. Bridge implementations must
return a declared error value or terminate through a documented runtime
failure. Snacc failures must likewise not unwind into Rust.

## Rust crate interoperability

Application authors may declare any Rust dependency accepted by Cargo for the
selected target:

~~~toml
[dependencies]
regex = "1"
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["rt-multi-thread"] }
~~~

This does not make arbitrary Rust items directly callable from Snacc. Rust
crates expose Rust types, traits, generics, lifetimes, async values, macros, and
an unstable native ABI. A Rust bridge must adapt the selected API to the Snacc
ABI.

The initial bridge workflow is explicit:

1. Add the dependency to `Cargo.toml`.
2. Implement a C-compatible exported wrapper in `src/interop.rs`.
3. Add the matching typed binding declaration to Snacc.
4. Let the Snacc checker validate calls against that declaration.
5. Let rustc validate the wrapper against the selected crate version.
6. Let the final linker prove that every referenced bridge symbol exists.

Implemented primitive Rust bridge:

~~~rust
#[unsafe(no_mangle)]
pub extern "C" fn snacc_user_itoa_len(value: i64) -> i64 {
    let mut buffer = itoa::Buffer::new();
    buffer.format(value).len() as i64
}
~~~

Matching checked Snacc declaration:

~~~text
extern rust "snacc_user_itoa_len" fun itoa_len(value: Int64): Int64
~~~

The initial bridge ABI supports `Int64`, `Dec64`, `Bool`, and `Nil`. The parser
creates an external-function node, the checker validates its `snacc_user_`
symbol and concrete signature, and LLVM lowering declares only that checked
symbol. Strings, aggregates, ownership, and error values require a later ABI
RFC before they can cross the boundary.

Snacc must not:

- Read rlib metadata as its crate interface.
- Guess mangled Rust symbol names.
- Pass Rust `String`, `Vec`, trait objects, references, or enums directly.
- Permit opaque untyped foreign calls.
- Defer Snacc argument or result checking to rustc.

## Generated bridge tooling

Manual bridge pairs are acceptable for the first end-to-end prototype but do
not scale. A later `cargo snacc bridge` command should use one declarative
interface as the source of truth and generate:

- Snacc binding declarations.
- Rust extern wrappers.
- ABI-safe record and tagged-union layouts.
- Conversion and destruction functions.
- Compile-time size and alignment assertions.
- Tests for round trips and ownership.

The generator must not infer arbitrary Rust APIs. Platform- or application
authors deliberately choose concrete crate operations and generic
instantiations to expose.

## Diagnostics

Errors are classified by owner:

| Failure | Owner |
| --- | --- |
| Invalid Snacc syntax or types | `snacc-compiler` |
| Invalid Snacc package metadata | `cargo-snacc` |
| Invalid Rust bridge or crate use | rustc through Cargo |
| Missing bridge symbol | Final native link |
| ABI declaration/layout mismatch detected before link | Bridge generator |
| Unsupported target/profile | `cargo-snacc` or `snacc-compiler` |
| Unclassifiable compiler state | Snacc internal compiler error |

`cargo-snacc` must preserve structured Snacc source spans and Cargo's rendered
Rust diagnostics. It must not rewrite a rustc error as a Snacc source error.

A missing bridge symbol should be augmented, when possible, with the
corresponding Snacc binding declaration and a message that the Rust bridge did
not provide its declared link name.

Exit behavior:

- Command-line or metadata failure: nonzero.
- Snacc diagnostic: nonzero.
- Cargo/rustc/linker failure: Cargo's failure is propagated as nonzero.
- Failed program launch: nonzero.
- Successful `run` launch: application exit status.

## Incremental behavior

Changing only:

- A Rust dependency or bridge recompiles through Cargo without regenerating the
  Snacc object unless binding declarations changed.
- Snacc source regenerates the content-addressed object and relinks the host
  without rebuilding unchanged dependencies.
- Build profile or target generates a separate object namespace.
- Program arguments does not trigger any compilation.

`cargo-snacc` should print concise reasons when `--verbose` is enabled:

~~~text
Snacc object reused: build identity unchanged
Snacc object rebuilt: src/main.nrs changed
Rust host rebuilt by Cargo
~~~

The cache is an optimization only. Deleting `target/snacc` must never change
program semantics.

## Security and process execution

Cargo dependencies, build scripts, and procedural macros execute with the
permissions Cargo normally grants them. Snacc does not add a sandbox.

`cargo-snacc` must:

- Pass arguments without a shell.
- Canonicalize and validate selected package-owned input paths.
- Never execute paths obtained from untrusted Snacc source.
- Treat Cargo JSON contents and crate diagnostics as data, not commands.
- Avoid printing environment secrets in verbose output.
- Respect Cargo's `--locked`, `--frozen`, `--offline`, and registry behavior.

## Testing strategy

### Compiler tests

Phase tests call `snacc-compiler` directly and never invoke Cargo. Object
generation tests stop before linking unless runtime semantics are under test.

### Orchestrator tests

Fixture workspaces verify:

- Package and workspace discovery.
- Metadata validation.
- Command argument forwarding.
- Profile and feature selection.
- Content-addressed cache reuse and invalidation.
- Cargo JSON artifact selection.
- Program argument and exit-code forwarding.
- Clean failures for ambiguous targets.

Subprocess tests use a fake Cargo executable where native compilation is not
the behavior under test.

### Integration tests

At least one real fixture must:

1. Depend on a small crates.io Rust crate.
2. Export one Rust bridge function.
3. Call it from checked Snacc code.
4. Compile the Snacc object.
5. Build the Rust host through Cargo.
6. Run the executable and compare output.

Windows tests must use the LLVM toolchain selected by RFC 002 rather than a
globally installed fallback.

### Performance tests

CI should record:

- Clean compiler installation build time.
- Clean application build time.
- No-change `cargo snacc build` time.
- Snacc-only edit rebuild time.
- Rust-only bridge edit rebuild time.
- Dependency-only edit rebuild time.

No-change builds must avoid LLVM lowering. Snacc-only edits must not rebuild
unchanged Cargo dependencies.

## Migration plan

### Phase 1: compiler library boundary

- Preserve `parse`, `check`, and `emit_object` as library operations.
- Add explicit compilation options and object metadata.
- Keep the current direct CLI for existing tests.

### Phase 2: Cargo host prototype

- Create one fixture Cargo binary with the minimal Rust host.
- Compile Snacc to a stable object path.
- Prove final linking through `cargo rustc` on Windows.
- Replace the stable object path with the content-addressed cache.

### Phase 3: `cargo-snacc` commands

- Add `init`, `check`, `build`, `run`, and `doctor`.
- Use Cargo metadata and JSON build messages.
- Support package, profile, feature, lockfile, and offline options.

### Phase 4: runtime extraction

- Move universal runtime functions from the temporary `runtime.rs` compilation
  path into `snacc-runtime`.
- Make the generated host depend on the matching runtime crate version.
- Remove per-program rustc compilation of `runtime.rs`.

### Phase 5: typed Rust bridges

- Add checked external-function declarations.
- Specify the primitive ABI.
- Implement one end-to-end crate bridge.
- Add missing-symbol and ABI conformance diagnostics.

### Phase 6: tests and cross-compilation

- Add application test orchestration.
- Make LLVM lowering target-aware.
- Add target ABI suites before enabling `--target`.

### Phase 7: direct CLI disposition

- Retain direct compiler commands needed by editors and compiler developers.
- Deprecate direct executable linking once `cargo snacc build` reaches parity.
- Keep an explicit object-emission command as a stable low-level tool.

## Alternatives considered

### Plain `build.rs`

A checked-in build script can invoke Snacc and emit Cargo link directives. It
would make `cargo build` and `cargo run` work directly, but every package would
compile or locate the Snacc compiler during its build script, and compiler
version selection would be split between the installed CLI and manifest build
dependencies.

This RFC instead gives `cargo-snacc` explicit orchestration ownership and uses
`cargo rustc` to pass the object only to the selected final target.

### `RUSTC` or `RUSTC_WRAPPER`

Cargo expects replacements and wrappers to behave like rustc for Rust crates,
metadata, dependency information, and output artifacts. Snacc is not rustc and
must not impersonate it.

### Procedural macro DSL

A function-like procedural macro consumes and produces Rust token streams.
Using it as the language boundary would embed Snacc in `.rs` files and
ultimately generate Rust syntax. That is a different product and violates the
no-transpilation decision.

### Universal runtime containing ecosystem crates

A monolithic runtime would prevent applications from selecting crate versions
and features independently. `snacc-runtime` remains intentionally small while
each host declares its own application dependencies.

### Prebuilt Roc-style platforms

Prebuilt platforms remove the Rust requirement from application authors but
move crate selection to platform authors. This conflicts with the requirement
that each application programmer choose arbitrary Cargo dependencies.

## Acceptance criteria

The RFC is implemented when:

- The prebuilt Windows package installs a functioning `cargo snacc` subcommand
  and emits a native object without a separately installed LLVM SDK.
- `cargo install cargo-snacc --locked` installs a functioning Cargo subcommand
  when RFC 002's documented source-build environment is already configured.
- `cargo snacc init hello` creates a buildable Cargo-hosted Snacc application.
- `cargo snacc check` validates both Snacc and Rust bridge code.
- `cargo snacc build` emits a content-addressed Snacc object and links it only
  into the selected host binary.
- `cargo snacc run -- arg` runs the executable with the exact supplied argument
  and propagates its exit status.
- Adding a normal Cargo dependency requires no Snacc package-manager changes.
- A Rust bridge can call that dependency and be invoked from checked Snacc code.
- No generated Rust source represents Snacc application logic.
- No `RUSTC`, `RUSTC_WRAPPER`, or global `RUSTFLAGS` override is required.
- A no-change build reuses both Cargo and Snacc artifacts.
- A Snacc-only change does not rebuild unchanged Rust dependencies.
- Unsupported targets and profiles fail before incompatible objects are linked.
- `cargo fmt`, `cargo check`, and the relevant `cargo test` suites pass for the
  Snacc workspace.

## References

- Cargo external subcommands:
  <https://doc.rust-lang.org/cargo/reference/external-tools.html>
- `cargo rustc`:
  <https://doc.rust-lang.org/cargo/commands/cargo-rustc.html>
- Cargo build scripts and native linkage:
  <https://doc.rust-lang.org/cargo/reference/build-scripts.html>
- Cargo installation:
  <https://doc.rust-lang.org/cargo/commands/cargo-install.html>
- Rust external ABI:
  <https://doc.rust-lang.org/reference/items/external-blocks.html>
- RFC 002:
  [Windows LLVM 22 Toolchain Integration](002-windows-llvm-toolchain.md)
