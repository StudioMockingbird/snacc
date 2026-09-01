# RFC 001: Temporary Vendored Clang Bootstrap Pipeline

Status: Completed (superseded bootstrap design)

## Summary

Snacc will first establish a transparent Windows native-build pipeline using
the LLVM 22.1.8 distribution already stored under `vendor/`:

~~~text
source
  -> tokens
  -> syntax tree
  -> typed program
  -> verified textual LLVM IR through Inkwell
  -> vendored clang.exe
  -> Windows COFF object
  -> Rust entry wrapper and rustc
  -> Windows executable
~~~

In this temporary phase, Inkwell remains the only owner of lowering Snacc
semantics into LLVM IR. The vendored Clang driver performs target-specific LLVM
IR compilation into a native object. `rustc` then compiles the existing Rust
runtime wrapper and links the compiler object into an `.exe`.

The final operation is native linking, not a file-format wrapper around the
object. The phrase "wrap the object in an executable" means that the Rust entry
wrapper supplies `main` and runtime functions while rustc drives the platform
linker.

This RFC deliberately exposes each artifact boundary so the downloaded LLVM
distribution, IR lowering, object generation, runtime ABI, native linking, and
execution can be proven independently. RFC 002 replaces the Clang subprocess
with in-process object emission once the LLVM developer and runtime packaging
contracts are established. RFC 003 then makes Cargo the application build
orchestrator.

## Motivation

The current implementation asks Inkwell's target machine to emit an object in
process and then asks rustc to link it. On Windows, in-process target emission
also depends on LLVM target-initialization wrappers, import-library selection,
and DLL discovery. Debugging all of those concerns at once obscures whether a
failure belongs to Snacc lowering, LLVM setup, object emission, or linking.

The vendored LLVM archive already contains a version-matched `clang.exe`.
Temporarily delegating target-specific object generation to that executable
creates a narrow process boundary with inspectable `.ll` and `.obj` artifacts.
It is a bootstrap and diagnosis strategy, not Snacc's final distribution model.

Cargo and rustc's internal LLVM are explicitly out of scope. They do not expose
a supported LLVM C API or a stable command that accepts Snacc's typed program.

## Goals

- Validate the vendored LLVM archive before changing the permanent Inkwell
  linkage and packaging design.
- Keep Snacc-to-LLVM lowering inside Inkwell.
- Produce a real x86-64 Windows COFF object using vendored `clang.exe`.
- Link that object with the Rust runtime into an executable using rustc.
- Make the IR, object, link, and execution boundaries independently testable.
- Remove the immediate need for in-process LLVM target-machine initialization
  during the bootstrap phase.
- Fail closed when Clang, LLVM, rustc, the linker, or an intermediate artifact
  is missing or incompatible.
- Preserve a direct migration path to RFC 002 and RFC 003.

## Non-goals

- Replacing Inkwell with handwritten LLVM IR or a Clang-based language backend.
- Transpiling Snacc to C or Rust.
- Treating the LLVM embedded in rustc as a supported toolchain.
- Defining the final end-user installation or DLL packaging model.
- Making a Clang subprocess the permanent object-emission architecture.
- Implementing Cargo dependency integration; RFC 003 owns that work.
- Supporting cross-compilation or non-Windows hosts in this temporary phase.
- Supporting Windows targets other than x86-64 MSVC.
- Optimizing subprocess count before the pipeline is proven correct.

## Supported configuration

| Property | Required value |
| --- | --- |
| Host operating system | Windows |
| Rust host target | `x86_64-pc-windows-msvc` |
| LLVM distribution | `clang+llvm-22.1.8-x86_64-pc-windows-msvc` |
| Clang | Vendored `bin/clang.exe`, version 22.1.8 |
| LLVM IR producer | Inkwell 0.10.x with LLVM 22.1 support |
| Object format | x86-64 COFF |
| Canonical object suffix | `.obj` |
| Executable suffix | `.exe` |
| Runtime implementation | Rust |
| Final link driver | rustc and the MSVC linker selected by Rust |

Although `.o` is often used as the platform-neutral name for a compiler
object, bootstrap artifacts use `.obj` on Windows so logs and retained files
state the actual platform contract.

## Toolchain contract

The default LLVM prefix is repository-relative:

~~~text
vendor/
  clang+llvm-22.1.8-x86_64-pc-windows-msvc/
    bin/
      clang.exe
      LLVM-C.dll
    include/
      llvm-c/
        Core.h
    lib/
      LLVM-C.lib
~~~

`LLVM_SYS_221_PREFIX` may select an external prefix. An explicit value takes
precedence over the repository default and must never fall back silently to a
different installation.

Before compilation starts, setup validation must:

1. Resolve the selected prefix to an absolute path.
2. Require the files needed by the active Inkwell linkage configuration.
3. Require `<prefix>/bin/clang.exe`.
4. Execute that exact file with `--version` and require LLVM 22.1.8.
5. Ensure the reported target is compatible with x86-64 Windows MSVC, or prove
   support with an explicit target query.
6. Make the selected `LLVM-C.dll` discoverable for the Snacc process without
   changing the machine-wide `PATH`.

The implementation must never invoke an unqualified `clang` from `PATH` after
the vendored prefix has been selected. This prevents Visual Studio, Chocolatey,
or another LLVM installation from changing generated code or hiding an
incomplete archive.

## Compiler boundary

The forward-only compiler phases remain:

~~~text
source -> tokens -> syntax tree -> typed program -> LLVM module
~~~

`llvm_codegen` consumes only a validated typed program. It creates the LLVM
module, lowers every supported typed node, declares the runtime ABI, verifies
the module, and serializes textual LLVM IR. It must not discover Clang, spawn
processes, choose output directories, or link executables.

During this temporary phase, `llvm_codegen` does not create an LLVM target
machine and does not call LLVM target-initialization helpers. Target-specific
data layout and machine-code selection belong to the exact vendored Clang
invocation. The module must declare the supported target triple, and Clang must
reject rather than silently retarget incompatible IR.

A small native-driver layer outside `llvm_codegen` owns:

- Toolchain validation.
- Temporary artifact paths.
- Writing the verified IR.
- Invoking vendored Clang.
- Validating the resulting object.
- Invoking rustc for the final executable.
- Converting process failures into structured backend diagnostics.

This ownership is temporary but prevents process and filesystem concerns from
leaking into parsing, checking, or lowering.

## Artifact pipeline

Each compilation uses one private temporary directory:

~~~text
snacc-<random>/
  program.ll
  program.obj
  runtime.rs
  program.exe
~~~

Only the requested executable is copied or atomically renamed to its final
destination. Temporary artifacts are deleted after success. A verbose debug
option may retain them in an explicitly reported directory; normal failures
must report the artifact directory before deciding whether policy permits its
removal.

Input and output paths must be passed as separate process arguments. Commands
must not be assembled as shell strings. This preserves spaces and prevents
source-controlled filenames from becoming shell syntax.

### Stage 1: emit verified LLVM IR

Inkwell must:

1. Lower the validated typed program into one module.
2. Set the target triple to `x86_64-pc-windows-msvc`.
3. Declare the stable runtime functions and `snacc_main` ABI.
4. Verify the module.
5. Serialize the module as UTF-8 textual LLVM IR to `program.ll`.

Verification failure is a backend or internal compiler error. Invalid IR must
never be passed to Clang.

### Stage 2: compile IR into an object

The native driver invokes the selected absolute Clang path with arguments
equivalent to:

~~~text
<prefix>\bin\clang.exe
  --target=x86_64-pc-windows-msvc
  -x ir
  -c program.ll
  -o program.obj
~~~

The initial implementation uses a fixed optimization setting declared by the
CLI profile. It must not inherit optimization, target, or linker flags from
ambient `CFLAGS`, `CXXFLAGS`, or similarly named environment variables.

Success requires all of the following:

- Clang exits with status zero.
- `program.obj` exists as a regular file inside the private build directory.
- The object is non-empty.
- The object format and machine type are x86-64 COFF, checked with a vendored
  LLVM inspection tool when available or a small format-header check otherwise.

Clang diagnostics are retained verbatim as subordinate diagnostic text, while
Snacc reports the stage, selected executable, exit status, and affected source
unit. A Clang rejection of verified IR is a backend failure, not a Snacc source
diagnostic.

### Stage 3: link the Rust runtime and object

The existing Rust runtime wrapper declares and calls `snacc_main` and owns the
platform `main`. The native driver writes that version-matched wrapper to
`runtime.rs` and invokes rustc with arguments equivalent to:

~~~text
rustc
  --edition=2024
  runtime.rs
  -C link-arg=<absolute path to program.obj>
  -o <absolute path to program.exe>
~~~

rustc compiles the Rust runtime and drives the supported MSVC native linker.
Clang does not replace rustc for this stage because the runtime remains Rust.
The executable is complete only when rustc succeeds and produces a non-empty
regular file at the requested path.

This stage must keep the compiler object and runtime ABI version-matched. The
runtime source is embedded in the Snacc executable or loaded from a declared
installation resource; it is never selected from the current application
directory.

### Stage 4: execute when requested

Compile-only commands stop after publishing the executable. Run commands start
that exact executable, forward the user's program arguments without shell
interpretation, and propagate its exit status. Compiler diagnostics go to
standard error and must not be mixed with program output.

## Diagnostics

The earliest failing stage owns the diagnostic:

| Failure | Owner |
| --- | --- |
| Missing or invalid LLVM prefix | Toolchain validation |
| Missing or wrong Clang version | Toolchain validation |
| Unsupported Snacc syntax or type | Frontend/type checker |
| LLVM module verification failure | `llvm_codegen` |
| Clang rejects verified LLVM IR | Native driver/backend |
| Clang succeeds without a valid object | Native driver/backend |
| rustc is absent | Native driver/toolchain |
| Runtime symbol is unresolved | Final native link |
| Program exits unsuccessfully | Program execution |

Independent toolchain checks should be aggregated by a doctor/setup command.
Compilation itself stops at the first failed forward stage and must not use a
stale artifact from an earlier invocation.

## Test strategy

Fast tests must stop at the earliest relevant phase:

- Lexer and parser tests do not initialize LLVM or spawn tools.
- Checker tests do not initialize LLVM or spawn tools.
- Lowering tests construct and verify LLVM modules without invoking Clang.
- Focused backend tests compile representative IR to `.obj` with vendored
  Clang.
- Link tests combine one known-good object with the Rust runtime.
- End-to-end tests compile, link, run, and compare observable output.

The complete end-to-end corpus must not rebuild the Snacc compiler once per
case. The test harness uses the already-built compiler library or executable,
reuses the validated toolchain description, and runs cases with isolated
artifact directories. Tests may execute independent cases concurrently after
toolchain validation.

At minimum, Windows CI covers:

| Case | Expected result |
| --- | --- |
| Valid vendored Clang 22.1.8 | IR, object, link, and execution pass |
| `clang.exe` missing | Validation fails before lowering |
| Clang 21 or 23 selected | Version mismatch is reported |
| Different global Clang first on `PATH` | Vendored executable is still used |
| Invalid generated IR fixture | Clang stage fails without linking |
| Clang exits zero without an object test double | Artifact validation fails |
| Object has wrong machine type | Artifact validation fails before rustc |
| rustc missing | Link-stage prerequisite is reported |
| Runtime ABI symbol missing | Final link fails with preserved linker output |
| Output path contains spaces | Compilation and execution pass |

## Implementation phases

### Phase 1: validate the archive

- Add repository-relative LLVM prefix selection.
- Validate `clang.exe`, LLVM runtime files, and the exact version.
- Add a setup/doctor command that prints the resolved tools in verbose mode.
- Prove a checked-in minimal LLVM IR fixture can become a COFF object.

### Phase 2: expose the IR boundary

- Split LLVM module construction and serialization from object emission.
- Keep exhaustive typed-program lowering and module verification in Inkwell.
- Remove in-process target-machine initialization from the bootstrap path.
- Add deterministic IR and module-verification tests.

### Phase 3: produce native objects through Clang

- Add the native-driver process boundary.
- Compile verified `.ll` into `.obj` with the absolute vendored Clang path.
- Validate the object header and machine type.
- Add failure-path and paths-with-spaces tests.

### Phase 4: produce and run executables

- Link the object with the version-matched Rust runtime through rustc.
- Publish the executable atomically.
- Run the executable and compare output in the end-to-end corpus.
- Ensure the compiler itself is built only once for the test run.

### Phase 5: retire the bootstrap

- Implement RFC 002's permanent LLVM toolchain integration.
- Restore in-process object emission through Inkwell's target machine.
- Keep the same runtime ABI and object-level integration tests.
- Delete Clang invocation code that has no remaining diagnostic or tooling use.
- Retain only generally useful archive validation and artifact inspection code.

## Acceptance criteria

- The selected vendored `clang.exe` is identified by absolute path and verified
  as LLVM 22.1.8.
- Inkwell produces a verified LLVM module without initializing an in-process
  native target machine.
- Vendored Clang converts the module's textual IR into a valid x86-64 COFF
  object.
- rustc links that object with the Rust runtime into a working `.exe`.
- A program is compiled and executed without selecting Clang from global
  `PATH`.
- Every intermediate failure is attributed to its owning stage and stale
  artifacts are never consumed.
- Frontend tests remain independent of LLVM and subprocesses.
- The end-to-end corpus reuses one compiler build and one validated toolchain.
- The temporary path has an explicit deletion point after RFC 002 provides
  in-process object emission.
- `cargo fmt --check`, `cargo check`, and `cargo test` pass.

## Relationship to later RFCs

- [RFC 002](002-windows-llvm-toolchain.md) owns the permanent LLVM 22 developer
  toolchain, Inkwell target wrappers, direct object emission, DLL resolution,
  and release packaging.
- [RFC 003](003-cargo-hosted-applications.md) owns `cargo snacc`, Cargo package
  discovery, Rust crate integration, caching, and the final Cargo-driven link.

RFC 002 must preserve the observable object and runtime ABI proven here. RFC
003 may reuse the native-driver boundary, but it replaces the temporary direct
CLI link orchestration with Cargo-aware orchestration.
