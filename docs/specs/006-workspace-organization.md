# RFC 006: Rust Workspace Organization

Status: Completed

## Summary

Snacc uses a virtual Cargo workspace whose packages are divided into user-facing
applications under `apps/` and reusable implementation crates under `crates/`.
The former root package is `crates/snacc-compiler`, the direct `snacc`
executable is `apps/snacc`, and the Cargo subcommand is `apps/cargo-snacc`.

RFC 004 promoted native executable construction into `crates/snacc-driver`,
which is shared by the direct CLI and the local web workbench. This preserves a
single native-linking implementation while `crates/snacc-runtime` remains the
sole owner of universal runtime behavior and its stable native ABI.

Compiler phases will remain explicit modules inside `snacc-compiler`. This RFC
does not create a crate for every phase. The package boundary is introduced
only where a consumer needs a different dependency set or a distinct ownership
contract.

## Motivation

The repository root currently has four incompatible responsibilities:

- It is the Cargo workspace root.
- It is the `snacc` compiler library package.
- It builds both the `snacc` and `cargo-snacc` executables.
- Its build script selects and links the LLVM distribution.

This makes package ownership unclear and places the growing Cargo application
in `src/bin/cargo-snacc.rs` beside compiler internals. The Cargo subcommand is
already substantially larger than the direct CLI and compiler facade, but its
tests, dependencies, and release behavior are still attributed to the root
package.

The current workspace manifest also names a missing `myproj` member, so Cargo
cannot load workspace metadata or run workspace-wide checks. A workspace meant
to host a compiler, language server, package tooling, workbench, and runtime
needs stable boundaries before more applications are added.

The target structure must preserve the architectural pipeline defined by
`LANGUAGE.md` and the implementation:

~~~text
source -> tokens -> syntax tree -> typed program -> LLVM IR -> native code
~~~

Repository organization must not add alternate parsing, checking, or lowering
paths.

## Goals

- Make the repository root an unambiguous coordination and documentation root.
- Give each shipped executable its own Cargo package and test boundary.
- Keep reusable compiler and runtime behavior out of application packages.
- Leave a clear compiler boundary that a future LSP specification can refine
  when its first implementation exists.
- Define when direct compilation and the web workbench promote native linking
  into one shared implementation.
- Centralize workspace package metadata and dependency versions where doing so
  prevents drift.
- Preserve executable names, compiler APIs, runtime ABI, fixtures, and release
  contents during migration.
- Keep workspace-wide formatting, checking, and testing commands valid.

## Non-goals

- Designing LSP protocol behavior or editor features.
- Designing a new Snacc package manifest or dependency resolver.
- Replacing Cargo-hosted application behavior.
- Combining all tools into one executable.
- Creating separate crates for the lexer, parser, syntax tree, checker, typed
  program, or LLVM lowering.
- Introducing backend traits, dynamic dispatch, incremental query frameworks,
  or a second compiler pipeline.
- Moving the vendored LLVM distribution or changing the supported LLVM
  version.
- Changing language syntax or semantics. `LANGUAGE.md` remains the sole
  normative language contract.

## Terminology

An **application package** produces a user-facing executable and owns argument
parsing, presentation, process lifecycle, or protocol behavior.

An **implementation crate** exposes reusable Rust APIs with a narrow ownership
contract. It does not depend on an application package.

The **frontend** is the lexing, parsing, and type-checking portion of
`snacc-compiler`. It produces a validated typed program.

The **native backend** lowers a validated typed program through Inkwell and LLVM
to a native object.

The **native driver** combines compiler object emission with runtime selection
and final executable construction. It does not own language semantics.

## Decision

### Repository layout

The workspace will use this layout:

~~~text
snacc/
  Cargo.toml
  Cargo.lock
  LANGUAGE.md
  README.md
  TODO.md
  apps/
    snacc/
      Cargo.toml
      src/main.rs
      tests/
    cargo-snacc/
      Cargo.toml
      src/main.rs
      tests/
    snacc-lsp/                 added when LSP implementation begins
    snacc-workbench/
  crates/
    snacc-compiler/
      Cargo.toml
      build.rs
      src/
        lib.rs
        diagnostics.rs
        syntax/
          mod.rs
          ast.rs
          lexer.rs
          parser.rs
        semantics/
          mod.rs
          checker.rs
        backend/
          mod.rs
          llvm.rs
      tests/
    snacc-driver/
      Cargo.toml
      src/lib.rs
      tests/
    snacc-runtime/
      Cargo.toml
      src/lib.rs
      tests/
  tests/
    cases/
    fixtures/
  examples/
  docs/specs/
  tools/
  vendor/
~~~

Directories for unimplemented packages must not be added as empty placeholders.
They appear above to reserve package placement, not to require scaffolding
before usable code exists.

The root `Cargo.toml` is a virtual manifest. It has no `[package]`, build
script, library, or binary targets. It lists only packages that exist in the
repository.

### Workspace manifest

The root manifest will declare resolver version 3 and explicit members:

~~~toml
[workspace]
resolver = "3"
members = [
    "apps/snacc",
    "apps/cargo-snacc",
    "crates/snacc-compiler",
    "crates/snacc-runtime",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
~~~

The resolver declaration is explicit because a virtual manifest has no root
package edition from which Cargo can infer a resolver.

The workspace does not narrow `default-members`; commands issued at the root
cover every member unless the caller explicitly selects packages. This keeps a
plain root `cargo test` from silently omitting compiler or runtime tests. RFC
The workspace includes `crates/snacc-driver` and `apps/snacc-workbench`, which
RFC 004 adds as the second native-driver consumer.

Workspace dependencies centralize versions shared by two or more packages.
Each package still declares only the dependencies and features it uses. A
dependency used by one package remains in that package until sharing it removes
real duplication.

Path dependencies use workspace-relative paths. Published packages must replace
or augment those paths with versions according to the release process before
publication is enabled.

### Dependency direction

Dependencies move from applications toward implementation crates:

Initially:

~~~text
apps/snacc ---------------------> snacc-compiler
  native-link module -----------> snacc-runtime contract

apps/cargo-snacc ---------------> snacc-compiler
~~~

When RFC 004 introduces a second consumer:

~~~text
apps/snacc ----------------------> snacc-driver
apps/snacc-workbench ------------> snacc-driver
                                      |
                                      v
                                 snacc-compiler
                                      |
                                      v
                                 snacc-runtime contract
~~~

`snacc-runtime` does not depend on the compiler or driver. Application packages
must not depend on other application packages. Shared behavior moves into an
implementation crate only after at least two applications require the same
contract or a specification assigns that crate explicit ownership.

Cargo metadata, command-line models, HTTP models, LSP models, rendered
diagnostics, and subprocess output must not enter compiler syntax or typed
representations.

### `snacc-compiler`

`crates/snacc-compiler` owns the complete validated compiler pipeline. Its
frontend modules own tokens, syntax nodes, parsing, type checking, and the typed
program. Its backend module owns LLVM lowering and native object emission.

The existing public operations remain the initial compatibility surface:

- `parse` accepts a source string and returns a syntax tree or structured
  diagnostics.
- `check` accepts a source string and returns a typed program or structured
  diagnostics.
- `emit_object_with_options` accepts source and explicit options and returns an
  emitted object with metadata or structured diagnostics.
- `emit_object` remains the convenience operation using default compilation
  options.

Parsing and checking must not perform filesystem access, start processes, or
initialize LLVM. This RFC does not feature-gate or extract the LLVM backend,
because every current compiler consumer emits native code.

The Inkwell dependency and the compiler build script belong to
`snacc-compiler`. The build script continues to validate the configured LLVM 22
distribution and link the required native library. Moving it must not change
LLVM selection, version validation, target support, or packaged DLL behavior.

The first LSP implementation must decide, using measured build and deployment
needs, whether to add a `native` feature or extract `snacc-frontend`. That
decision belongs to the LSP specification and is not made for a prospective
consumer here.

### `snacc-driver`

`crates/snacc-driver` owns construction of a runnable native executable from
Snacc source and explicit build options. RFC 004 made the workbench its second
consumer, and RFC 005 defines its generated-host and runtime-linking behavior.

It owns:

- Temporary build-directory lifecycle.
- Emitted object placement.
- Generated Rust host construction.
- Selection of the matching `snacc-runtime` package.
- Cargo or linker invocation for the direct workflow.
- Final executable placement.
- Structured distinction between compiler, host-build, linker, and filesystem
  failures.

It does not own:

- Lexing, parsing, checking, or LLVM lowering.
- CLI argument parsing or rendered terminal diagnostics.
- Cargo workspace package discovery for user projects.
- Running an arbitrary completed program.
- HTTP or LSP request handling.

When extracted, the direct CLI and workbench call the same concrete driver API.
The move must not introduce a backend trait or duplicate compiler phases. RFC
005 determines the generated host and `snacc-runtime` linking contract.

### `snacc-runtime`

`crates/snacc-runtime` remains the sole implementation of universal Rust-side
runtime behavior. It owns stable exported runtime symbols, ABI declarations,
and focused ABI tests. It does not contain compiler, driver, Cargo discovery,
or application-specific dependency logic.

RFC 005 governs removal of the root `runtime.rs` file. This workspace migration
must not create another runtime template or copy runtime implementations into
an application package.

### `apps/snacc`

`apps/snacc` produces the `snacc` executable. It owns:

- Direct CLI argument parsing.
- Reading a requested source file.
- Terminal diagnostic rendering.
- Calling `snacc-driver` for direct native executable construction.
- Process exit codes and user-facing messages.

It contains no compiler phases. Native executable construction is owned by
`snacc-driver`, with an application-independent input and result contract shared
by both applications. The package may depend on `ariadne`;
compiler libraries must not depend on terminal rendering solely for this
application.

### `apps/cargo-snacc`

`apps/cargo-snacc` produces the `cargo-snacc` executable and remains the owner
of Cargo-hosted project orchestration. It owns Cargo discovery, metadata,
package selection, command behavior, object caching, Cargo subprocesses,
profile handling, artifact selection, and its `doctor` checks.

The workspace move does not require a cosmetic module split. The existing
source moves intact first. Later changes may extract cohesive modules for CLI
parsing, metadata, build orchestration, caching, or doctor checks when doing so
supports an implementation change or removes demonstrated coupling. Such a
split must preserve command output and exit behavior and is not an acceptance
condition of this RFC.

`cargo-snacc` calls `snacc-compiler` directly for object generation. It does
not use `snacc-driver`, because Cargo-hosted projects have a distinct final-host
and artifact-selection contract already owned by `cargo-snacc`.

### Future applications

`apps/snacc-lsp` will own the language-server protocol, document lifecycle,
request scheduling, editor-facing diagnostics, and source-position conversion.
Its specification must select a frontend dependency boundary before
implementation. This RFC neither makes the existing backend optional nor
extracts a frontend crate in advance.

`apps/snacc-workbench` owns the HTTP server, browser assets, snippet catalog,
execution lifecycle, and response models described by RFC 004. It calls
`snacc-driver` rather than invoke the `snacc` executable or duplicate final
linking.

A future standalone package manager belongs under `apps/`. Package resolution,
manifest, registry, or lockfile behavior requires a separate specification.
This RFC does not infer those semantics from `cargo-snacc` and does not create a
generic package-management library in advance.

## Source organization inside the compiler

The current source files move without semantic redesign:

| Current path | Target path |
| --- | --- |
| `src/ast.rs` | `crates/snacc-compiler/src/syntax/ast.rs` |
| `src/lexer.rs` | `crates/snacc-compiler/src/syntax/lexer.rs` |
| `src/parser.rs` | `crates/snacc-compiler/src/syntax/parser.rs` |
| `src/checker.rs` | `crates/snacc-compiler/src/semantics/checker.rs` |
| `src/llvm_codegen.rs` | `crates/snacc-compiler/src/backend/llvm.rs` |
| `src/lib.rs` | `crates/snacc-compiler/src/lib.rs` |
| `src/main.rs` | `apps/snacc/src/main.rs` |
| `src/bin/cargo-snacc.rs` | `apps/cargo-snacc/src/main.rs` |
| `build.rs` | `crates/snacc-compiler/build.rs` |

Module visibility must keep the forward pipeline explicit. Syntax types may be
consumed by semantic checking. The typed program may be consumed by the LLVM
backend. Backend types must not appear in syntax or semantic modules.

Moving files must not leave compatibility modules or duplicate source files at
their old locations.

## Test organization

Tests live with the narrowest package whose public contract they exercise:

| Test responsibility | Location |
| --- | --- |
| Lexer, parser, checker, diagnostic, and object emission | `crates/snacc-compiler/tests/` or focused unit modules |
| Direct `snacc` CLI behavior | `apps/snacc/tests/` |
| Cargo subcommand behavior | `apps/cargo-snacc/tests/` |
| Direct executable construction before RFC 004 | `apps/snacc/tests/` |
| Shared native driver after RFC 004 | `crates/snacc-driver/tests/` |
| Runtime symbols and ABI | `crates/snacc-runtime/tests/` |
| Shared parse/type-check cases and diagnostics | root `tests/cases/` |
| Executable examples and expected stdout | root `examples/` |
| Cross-package Cargo projects | root `tests/fixtures/` |

Shared cases, examples, and fixtures are data, not additional root packages.
Package tests locate them from the workspace root through an explicit test
helper or compile-time path. Production code must not depend on the repository
test tree. RFC 004 may embed selected public examples in the workbench binary.

Parser, checker, and object-emission tests remain distinguishable suites even
though this RFC does not give them different Cargo feature configurations.

## Build, toolchain, and release behavior

The vendored LLVM directory remains at its current repository location. The
compiler build script resolves the workspace root from
`CARGO_MANIFEST_DIR` rather than assuming the compiler manifest is itself at
the root. `.cargo/config.toml` continues to provide a workspace-relative LLVM
prefix.

Scripts under `tools/` must select Cargo packages explicitly. They must not
extract the release version from the first `version` entry in the virtual root
manifest. They read workspace package metadata or the designated shipped
package version instead.

Release artifacts retain their current executable names:

- `snacc` or `snacc.exe`
- `cargo-snacc` or `cargo-snacc.exe`

The Windows package continues to place the required LLVM runtime beside the
executables according to the existing toolchain contract. The source move must
not change DLL lookup, package contents, or `cargo-snacc doctor` validation.

## Documentation ownership

- `LANGUAGE.md` remains the sole normative syntax and semantics contract.
- `TODO.md` continues to track only open bugs and small tasks.
- Repository architecture and migrations live in numbered specifications.
- Package-level README files may explain build and API usage but must not define
  language behavior.
- Archived specifications remain immutable.

References to old source paths in active specifications must be updated during
the migration which makes those references false. Historical paths in archived
specifications remain unchanged.

## Sequencing

RFC 005 removes the duplicate root runtime implementation before this workspace
migration moves package boundaries. This RFC then completes Phases 1 through 5
while RFC 004 creates `apps/snacc-workbench` and promotes the direct CLI's
native-link module to `crates/snacc-driver` because the second consumer is now
real. No LSP-only compiler configuration is created.

## Implementation plan

### Phase 1: repair and declare the workspace

1. Remove the nonexistent `myproj` member from the root manifest.
2. Convert the root manifest to a virtual workspace using resolver version 3.
3. Add workspace package metadata and only the shared dependencies used by the
   initial package set.
4. Create manifests for `snacc-compiler`, `snacc`, `cargo-snacc`, and
   `snacc-workbench` with their existing package and binary versions.
5. Keep `snacc-runtime` as an explicit member.
6. Confirm `cargo metadata --no-deps` resolves every member before moving
   source.
7. Move the `myproj` workspace failure out of TODO.md's open issues after the
   repaired manifest is verified.

### Phase 2: move the compiler package

1. Move the existing library source and root build script to
   `crates/snacc-compiler` according to the mapping above.
2. Introduce `syntax`, `semantics`, and `backend` modules without changing
   representations or phase behavior.
3. Move structured diagnostic definitions to `diagnostics.rs` while retaining
   their public names and fields.
4. Keep `parse`, `check`, and object-emission APIs available with their current
   behavior.
5. Verify emitted object metadata remains unchanged.

### Phase 3: move application packages

1. Move the direct CLI to `apps/snacc` and depend on `snacc-compiler` and
   `snacc-driver`.
2. Move native executable construction into `crates/snacc-driver`, with no CLI
   presentation types in its input or result.
3. Move `cargo-snacc` intact to `apps/cargo-snacc`; do not gate the workspace
   migration on an internal module split.
4. Move each integration test to its owning application package.
5. Update compile-time executable references and manifest-directory-relative
   fixture paths.
6. Delete the old root `src/` tree after all references are moved.

### Phase 4: update repository tooling

1. Update PowerShell build and packaging scripts to build explicit package and
   binary targets.
2. Update version discovery for the virtual manifest.
3. Preserve packaged executable and LLVM runtime locations.
4. Update README build commands and active specifications that refer to moved
   paths.
5. Regenerate `Cargo.lock` from the resolved workspace.

### Phase 5: verify boundaries

1. Run compiler, CLI, Cargo subcommand, runtime, conformance, and fixture tests.
2. Run Windows packaging verification and `cargo-snacc doctor` where supported.
3. Search production sources for old root paths and duplicate compiler or
   runtime implementations.
4. Archive this RFC with `Status: Closed` only after every acceptance criterion
   is verified against the resulting workspace.

## Compatibility and migration

This is a source-tree and package-boundary change. It does not intentionally
change Snacc syntax, type checking, generated ABI, command names, command
arguments, diagnostic text, or program behavior.

The Rust library package changes from `snacc` to `snacc-compiler`. Internal
workspace consumers update immediately. Because no stable public crates.io API
is currently specified, this RFC does not add a compatibility re-export crate.
If an external release contract requiring the old Rust crate name is discovered
before implementation, migration must stop and that compatibility requirement
must be specified rather than guessed.

The direct executable remains named `snacc`; the Cargo external subcommand
remains named `cargo-snacc`. Cargo-hosted project manifests continue to depend
on `snacc-runtime`, not on application packages.

The move was performed in compiling phases. Native executable construction now
lives in `crates/snacc-driver`, and no temporary duplicate remains in
`apps/snacc`.

## Rejected alternatives

### Keep the root compiler package

Using a root package plus workspace members leaves the root with a privileged
shape unlike every future application and continues to mix coordination files
with compiler build behavior. A virtual root makes package ownership uniform.

### Put every package under `crates/`

This is a valid Cargo convention but obscures the important distinction between
shipped applications and reusable implementation. Separate `apps/` and
`crates/` directories make intended dependency direction visible in the tree.

### Create one crate per compiler phase

The current phases share one pipeline and are not independently versioned or
reused. Separate crates would force additional public APIs and manifests
without removing real duplication. Modules provide the required exhaustiveness
and forward flow with less machinery.

### Create a frontend-only compiler configuration immediately

The only frontend-only consumer is prospective. Adding feature gates or a
`snacc-frontend` crate now would create conditional APIs or another package plus
a second CI configuration without serving present code. The first LSP RFC must
choose the smallest boundary supported by its actual build and deployment
requirements.

### Make the direct CLI the shared native driver

Invoking a CLI would force other applications to serialize source through files
or process I/O and parse rendered diagnostics. RFC 004 explicitly requires an
in-process compiler and driver API.

### Move Cargo orchestration into `snacc-driver`

The native driver builds one direct executable. `cargo-snacc` owns user Cargo
workspace discovery, target selection, caching, and artifact handling. Combining
them would make the workbench and direct CLI depend on unrelated Cargo project
semantics.

## Acceptance criteria

This RFC is implemented when:

1. The root manifest is virtual and lists no nonexistent workspace member.
2. `cargo metadata --no-deps` resolves all workspace packages.
3. `apps/snacc` produces the `snacc` executable with its existing interface.
4. `apps/cargo-snacc` produces the `cargo-snacc` executable with its existing
   commands and external-subcommand behavior.
5. `crates/snacc-compiler` owns the only lexer, parser, checker, typed program,
   and LLVM lowering implementation.
6. Parsing, checking, and native object emission retain their existing public
   behavior and the configured LLVM 22 contract.
7. Direct and workbench native executable construction have one concrete
   implementation in `crates/snacc-driver`.
8. `crates/snacc-runtime` remains the only universal runtime implementation.
9. Tests reside with their owning package while shared cases and fixtures
    remain reusable repository data.
10. Build and packaging scripts use explicit package targets and still produce
    the expected Windows package contents.
11. No production source depends upward from an implementation crate into an
    application package.
12. No obsolete root compiler source, compatibility path, or duplicate native
    executable construction remains.
13. `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and
    the relevant compiler, application, runtime, and conformance
    test suites pass.

## References

- [Language contract](../../LANGUAGE.md)
- [RFC 004: Local Web Workbench](004-web-workbench.md)
- [RFC 005: Remove `runtime.rs` in Favor of `snacc-runtime`](005-remove-runtime-rs.md)
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html)
