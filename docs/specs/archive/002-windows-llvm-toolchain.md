# RFC 002: Windows LLVM 22 Toolchain Integration

Status: Completed

This RFC supersedes RFC 001's temporary use of vendored `clang.exe` for object
generation. The verified typed-program-to-LLVM lowering remains unchanged, but
the final architecture emits native objects in process through Inkwell.

## Summary

Snacc uses LLVM through Inkwell as its only native-code backend. Windows builds
must use the versioned LLVM 22 developer distribution stored under `vendor/`
instead of depending on an LLVM installation in `C:\Program Files`.

The integration retains dynamic linkage to the distribution's monolithic
`LLVM-C.dll`. Inkwell continues to use `llvm22-1-no-llvm-linking` so Snacc can
select that import library explicitly, while `llvm-sys` uses the vendored
`llvm-config.exe` and LLVM headers to compile its standard target initialization
wrappers. Once verified, Snacc's handwritten `llvm_windows` module is removed.

This design makes LLVM versioned and reproducible without making Snacc
responsible for building LLVM from source.

The full developer distribution is a compiler-build dependency, not an
application-programmer prerequisite. Windows release packages contain the
prebuilt Snacc executable, the matching LLVM runtime DLL, and the required
license notices. That runtime remains available while `cargo-snacc` is running,
so the installed compiler can use Inkwell to emit native objects without
requiring `llvm-config`, LLVM headers, an import library, or a machine-wide LLVM
installation on the application programmer's computer.

## Goals

- Use LLVM 22.1.8 consistently for Windows development, tests, and releases.
- Preserve Inkwell as the sole owner of Snacc-to-LLVM lowering.
- Remove handwritten replacements for `llvm-sys` target wrappers.
- Make Windows builds independent of a machine-wide LLVM installation.
- Fail early when the toolchain is absent, incomplete, or incompatible.
- Load the same `LLVM-C.dll` whose import library was selected at link time.
- Permit an explicit external LLVM 22 prefix without source changes.
- Separate the LLVM files required to build Snacc from those required to run a
  prebuilt Snacc compiler.
- Let a prebuilt Windows package emit native objects on a machine that has Rust,
  Cargo, and the supported native linker but no separately installed LLVM SDK.

## Non-goals

- Building LLVM from source as part of a Snacc build.
- Statically linking all LLVM component libraries.
- Supporting multiple LLVM major versions in one Snacc revision.
- Adding another native-code backend.
- Supporting Windows targets other than x86-64 MSVC in this change.
- Making Cargo's or rustc's private LLVM installation a supported Inkwell SDK.
- Making a source-based `cargo install` independent of the LLVM developer
  distribution.

## Supported configuration

| Property | Required value |
| --- | --- |
| Operating system | Windows |
| Rust target | `x86_64-pc-windows-msvc` |
| LLVM release | 22.1.8 |
| Distribution | `clang+llvm-22.1.8-x86_64-pc-windows-msvc` |
| Inkwell | 0.10.x with `llvm22-1-no-llvm-linking` and `target-x86` |
| LLVM linkage | `LLVM-C.lib` import library and `LLVM-C.dll` |

An LLVM 22 patch upgrade must update the artifact metadata and pass the complete
validation suite. A major upgrade requires an explicit Inkwell feature and API
migration.

## Toolchain layout

The default extracted distribution root is:

~~~text
vendor/
  clang+llvm-22.1.8-x86_64-pc-windows-msvc/
    bin/
      llvm-config.exe
      LLVM-C.dll
    include/
      llvm-c/
        Target.h
    lib/
      LLVM-C.lib
      LLVMCore.lib
      ...
~~~

The root directory alone is not proof of a valid installation. These files form
the minimum installation contract:

- `bin/llvm-config.exe`
- `bin/LLVM-C.dll`
- `include/llvm-c/Target.h`
- `lib/LLVM-C.lib`

The default root is repository-relative. `LLVM_SYS_221_PREFIX` may override it.
An explicit override takes precedence and must satisfy the same contract.

This layout is the **compiler-build contract** used by contributors, CI, and
source installations. It is not the end-user release layout. A prebuilt release
does not need `llvm-config.exe`, the headers, or `LLVM-C.lib`, because those
inputs were consumed when `cargo-snacc.exe` was linked.

## Artifact acquisition and integrity

LLVM is a toolchain artifact, not Snacc source. Its metadata must record:

- Exact LLVM release.
- Official archive URL.
- Archive SHA-256 digest.
- Expected extracted directory name.

An acquisition script should download only when the validated distribution is
absent, verify SHA-256 before extraction, and extract into a temporary sibling
before renaming it into place. A partial download or extraction must never
appear valid.

CI should cache the archive or extracted directory with a key containing the
LLVM version, host target, archive digest, and acquisition-script revision.

The extracted distribution is approximately 3.84 GiB. If Snacc is put in
version control, the distribution should not be committed directly. Commit its
metadata and acquisition script instead. Ignore rules require care because the
current `bin/` rule also matches the distribution's `bin` directory.

## Cargo configuration

Repository-local Cargo configuration supplies the default prefix before
dependency build scripts execute:

~~~toml
[env]
LLVM_SYS_221_PREFIX = {
    value = "vendor/clang+llvm-22.1.8-x86_64-pc-windows-msvc",
    relative = true
}
~~~

An existing process environment value must remain able to override this
default. This supports CI caches and toolchains stored outside the source tree.

Setting the prefix in Snacc's `build.rs` is too late because Cargo runs
dependency build scripts, including `llvm-sys`, first.

The Windows dependency remains:

~~~toml
[target.'cfg(windows)'.dependencies]
inkwell = {
    version = "0.10.0",
    default-features = false,
    features = ["llvm22-1-no-llvm-linking", "target-x86"]
}
~~~

`no-llvm-linking` delegates native library selection to Snacc. It does not stop
`llvm-sys` from compiling its target wrappers when `llvm-config` and the headers
are available.

Static component linking is deferred. This distribution reports system library
`xml2s.lib` but does not contain it. Switching to `llvm22-1-prefer-static` would
add an unresolved native dependency and enlarge Snacc's native link surface.

## Build-script responsibilities

On Windows, `build.rs` must:

1. Read `LLVM_SYS_221_PREFIX`.
2. Validate every file in the minimum installation contract.
3. Run `bin/llvm-config.exe --version` and require LLVM 22.1.8.
4. Add `<prefix>/lib` as a native link-search directory.
5. Link the `LLVM-C` dynamic import library.
6. Emit rerun directives for the prefix and selected import library.

It must not:

- Search `C:\Program Files` after an explicit prefix fails.
- Silently select another LLVM version from `PATH`.
- Invoke CMake or build LLVM.
- Compile Snacc-specific target initialization wrappers.
- Perform LLVM IR lowering.

Failure must report the resolved prefix, failed requirement, expected release,
and remediation command or setup-script name, then stop the build.

Non-Windows linkage remains outside this specification.

## Wrapper ownership

LLVM declares `LLVM_InitializeAll*` and `LLVM_InitializeNative*` as C header
helpers rather than functions exported by `LLVM-C.dll`. `llvm-sys` ships
`wrappers/target.c` to provide callable symbols.

With a working `llvm-config.exe`, `llvm-sys` can locate the headers and compile
that wrapper. Snacc must then remove:

- `src/llvm_windows.rs`
- The conditional `mod llvm_windows;` declaration

No Rust replacement should be introduced. X86 initialization remains an
Inkwell operation in LLVM lowering; wrapper generation remains an `llvm-sys`
implementation detail. Removal is allowed only after a clean build proves that
`llvm-sys` supplies the symbols.

## Runtime DLL resolution

Windows resolves `LLVM-C.dll` when a linked Snacc executable starts. Link
success does not prove the correct DLL can be loaded.

Development commands must prepend `<prefix>/bin` to `PATH` for the command:

~~~powershell
$llvmRoot = Resolve-Path .\vendor\clang+llvm-22.1.8-x86_64-pc-windows-msvc
$env:LLVM_SYS_221_PREFIX = $llvmRoot
$env:Path = "$llvmRoot\bin;$env:Path"
~~~

`tools/build-snacc.ps1` establishes this environment, builds both Snacc
executables through Cargo, and assembles the successful result in the project
root `bin/` directory. `bin/` contains `cargo-snacc.exe`, `snacc.exe`, the
matching `LLVM-C.dll`, the LLVM license, and `build-info.json`. LLVM headers,
`llvm-config.exe`, and import/static libraries remain build inputs under
`vendor/` and are not copied into the runnable bundle.

Raw `cargo build` remains available for Rust development artifacts under
`target/`, but it is not a distributable Snacc build. All runnable local and
release builds use `tools/build-snacc.ps1` so executable and LLVM runtime
updates cannot silently diverge.

Windows release packages must place `LLVM-C.dll` beside every executable that
directly links it, including `cargo-snacc.exe` and `snacc.exe` when both are
shipped. Depending on a global DLL is forbidden because Windows could load a
different LLVM release. Packages must include upstream LLVM licensing notices.

The package producer must inspect the runtime dependency closure of
`LLVM-C.dll`. Any non-system DLL required by that closure must also be placed
beside the executable and covered by the package's integrity manifest. The
release package must not contain the multi-gigabyte developer distribution,
`llvm-config.exe`, LLVM headers, or static/import libraries.

The executable directory must remain intact after installation. Installing
only `cargo-snacc.exe`, or copying it away from its packaged DLLs, is an invalid
installation. Cargo discovers `cargo-snacc` through `PATH`; the executable does
not need to have been compiled by `cargo install`.

`build.rs` must not copy the DLL into guessed Cargo output directories. Cargo
does not provide a stable final-executable directory contract to build scripts.
DLL placement belongs to development command wrappers and release packaging.

## Compiler architecture boundary

The toolchain integration remains below the compiler pipeline:

~~~text
source
  -> tokens
  -> syntax tree
  -> typed program
  -> llvm_codegen through Inkwell
  -> LLVM C API from LLVM-C.dll
  -> native object
~~~

Discovery, linkage, wrapper compilation, and DLL deployment must not affect the
typed program or lowering. `llvm_codegen` may initialize X86 and request object
emission through Inkwell, but it must not discover paths, load arbitrary DLLs,
or compensate for incomplete LLVM installations.

## Developer workflow

1. Acquire and validate the distribution.
2. Run `tools/build-snacc.ps1` for a development build or
   `tools/build-snacc.ps1 -Release` for a release build.
3. Put the project root `bin/` directory on `PATH` when testing the assembled
   Cargo subcommand.

Required verification:

~~~powershell
cargo fmt --check
cargo check
cargo test
~~~

The workflow must also compile and execute a Snacc source program through LLVM.
That proves DLL loading, target initialization, object emission, native linking,
and execution rather than only Rust compilation.

## Distribution and installation workflow

Windows has two deliberately different installation paths.

### Prebuilt installation

This is the default path for application programmers:

1. Download a versioned Snacc package produced by the release workflow.
2. Verify the package digest or installer signature.
3. Install the complete package directory into a user-writable tools location.
4. Add that directory to `PATH`, or place the complete package contents in an
   existing user-writable `PATH` directory such as Cargo's binary directory.
5. Run `cargo snacc doctor` and an object-emission smoke test.

The package must contain `cargo-snacc.exe`, `LLVM-C.dll`, every required
non-system runtime DLL, an integrity manifest, and applicable license notices.
The application programmer needs Rust, Cargo, and the supported MSVC linker but
does not need the LLVM developer distribution.

### Source installation

A source installation, including `cargo install cargo-snacc --locked`, compiles
the Snacc executable and therefore uses the compiler-build contract. Before
invoking Cargo, the user must acquire the validated LLVM developer distribution,
set `LLVM_SYS_221_PREFIX`, and make its `bin` directory available for build and
test commands. Plain `cargo install` without that preparation is not a supported
self-contained installation path.

Snacc's Cargo build scripts must not download LLVM. Network acquisition,
integrity verification, extraction, and environment setup belong to a separate
bootstrap or release-installation command so dependency builds never select an
implicit or partially downloaded toolchain.

## CI workflow

A Windows CI job must:

1. Restore the versioned cache or acquire the verified archive.
2. Set `LLVM_SYS_221_PREFIX` to its absolute path.
3. Prepend its `bin` directory to the job-local `PATH`.
4. Record `llvm-config --version` in job metadata.
5. Run formatting, `cargo check`, and the complete tests.
6. Run the LLVM end-to-end execution corpus.

At least one job must run without `C:\Program Files\LLVM\bin` on `PATH`. This
proves the selected artifact is used and prevents an installed LLVM from hiding
packaging mistakes.

CI must not build LLVM from source during normal validation. A future source
build belongs in a separate toolchain-production workflow whose versioned output
is cached and consumed as a fixed artifact.

## Validation matrix

| Case | Expected result |
| --- | --- |
| Valid vendored LLVM 22.1.8 | Build and tests pass |
| Valid external LLVM 22.1.8 prefix | Override is used and tests pass |
| Missing prefix directory | Build reports the resolved missing path |
| Missing `llvm-config.exe` | Build stops before native linking |
| Missing `LLVM-C.lib` | Build stops before Rust links Snacc |
| Missing `LLVM-C.dll` | Setup validation fails before tests |
| LLVM 21 or LLVM 23 prefix | Build reports a version mismatch |
| Global LLVM present but artifact DLL absent | Clean-environment CI fails |
| Artifact `bin` first on `PATH` | LLVM end-to-end program executes |
| Complete prebuilt package, no LLVM SDK installed | Object emission, Cargo link, and execution pass |
| Prebuilt package missing `LLVM-C.dll` | `cargo snacc doctor` reports an invalid installation |
| Prebuilt package with mismatched `LLVM-C.dll` | Version validation fails before compilation |

The execution test must emit an object, link a program, run it, and compare
observable output.

## Migration plan

### Phase 1: establish the toolchain contract

- Add artifact metadata and integrity verification.
- Add the repository-local default prefix.
- Add a command wrapper that sets both required environment values.
- Extend `build.rs` validation to cover version, headers, library, and DLL.

The handwritten shim remains during this phase.

### Phase 2: transfer wrapper ownership

- Confirm `llvm-sys` finds the vendored `llvm-config.exe`.
- Remove the `llvm_windows` declaration and implementation.
- Perform a clean Windows build so cached symbols cannot mask failure.
- Run the full tests and LLVM end-to-end corpus.

### Phase 3: make CI and releases independent

- Add verified acquisition and caching to Windows CI.
- Remove global LLVM directories from the validation job's `PATH`.
- Package `LLVM-C.dll` beside release executables.
- Inspect and package the non-system runtime dependency closure of
  `LLVM-C.dll`.
- Publish a versioned integrity manifest and LLVM license notices.
- Test the package on Windows without LLVM installed by emitting, linking, and
  executing a Snacc program.

## Acceptance criteria

- A new Windows environment can acquire one declared LLVM artifact and build
  Snacc without installing LLVM globally.
- `llvm-sys` supplies its target initialization wrappers.
- Snacc contains no `llvm_windows` module or duplicate wrapper symbols.
- Snacc links against the selected `LLVM-C.lib`.
- Tests and packages load the matching `LLVM-C.dll`.
- A prebuilt package contains no LLVM headers, `llvm-config.exe`, or LLVM
  libraries that are needed only while building Snacc.
- On a clean Windows machine with Rust, Cargo, and the supported MSVC linker,
  the prebuilt package emits a native object and completes a Cargo-hosted Snacc
  application build without a separate LLVM installation.
- A source-based `cargo install` fails early with actionable LLVM setup guidance
  when the compiler-build contract is not satisfied.
- Missing files and version mismatches fail before compiler code executes.
- `cargo fmt --check`, `cargo check`, and `cargo test` pass.
- The LLVM end-to-end corpus compiles and executes on Windows.

## Future considerations

A custom LLVM build may be reconsidered for LLVM patches, additional targets,
assertions, smaller distributions, or reproducibility unavailable upstream. It
must be produced outside normal Cargo builds, pinned to a release commit,
limited to required targets, tested independently, and published as a versioned
artifact.

Static component linking may be reconsidered after every transitive native
dependency is available and its packaging size, build time, licensing, and
update cost are measured. Neither change is required to remove the current
Windows wrapper shim.
