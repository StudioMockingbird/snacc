# RFC 007: Rust Bridge Signature Verification

Status: Closed

## Summary

Snacc will prove that every `extern rust` declaration agrees with the ABI
function type of the Rust item it names, before linking. `cargo-snacc` will
derive a generated Rust assertion file from the checked Snacc program's
external declarations and compile it into the application host. Each assertion
coerces the named bridge item to the function-pointer type implied by its Snacc
declaration, so `rustc` rejects any difference in arity or in the Rust type of
any parameter or result.

The Snacc declaration remains the single hand-written source of truth. No Snacc
declaration, Rust wrapper, or binding file is generated from the other side, so
nothing can become stale.

This verifies ABI representation, not Snacc semantics, and it verifies a Rust
item, not an exported linker symbol. Both limits are stated precisely below and
neither is incidental: the residue stays with the final link, which already owns
it.

## Motivation

A bridge is two independent declarations of one symbol:

~~~text
src/main.nrs    extern rust "snacc_user_itoa_len" fun itoa_len(value: Int64): Int64
src/interop.rs  #[unsafe(no_mangle)] pub extern "C" fn snacc_user_itoa_len(value: i64) -> i64
~~~

Nothing compares them. The checker validates the Snacc side in isolation and
requires only the `snacc_user_` prefix. LLVM lowering declares the symbol with
the Snacc-declared types. The final linker proves only that a symbol with that
name exists. Changing the Rust parameter to `f64`, adding a parameter, or
returning `i32` still links, and produces a program that reads a garbage
argument or a garbage result. There is no diagnostic at any phase.

This is undefined behavior reachable from ordinary editing, and it gets worse
with every type added to the bridge ABI. Strings, opaque handles, and result
values each multiply the ways a hand-written pair can disagree while still
linking. Verification must exist before those types do.

`TODO.md` records this as the one open issue too large to fix from a task line.

## Goals

- Detect every arity disagreement between an `extern rust` declaration and the
  Rust item it names.
- Detect every disagreement in the Rust type of a parameter or result, to the
  precision of the ABI mapping in "Type mapping".
- Detect a declared bridge whose Rust item does not exist.
- Report all three before object linking, in every command that compiles the
  host: `check`, `build`, `run`, and `test`.
- Keep the Snacc declaration hand-written and authoritative.
- Introduce no generated artifact that a developer edits, commits, or can leave
  stale.
- Require no Rust parser, procedural macro, or rlib metadata reader in Snacc.
- Add no runtime cost to a verified bridge call.

## Non-goals

- Verifying that the bridge item is actually exported under the declared link
  symbol. `#[unsafe(no_mangle)]` and `#[export_name]` control export
  independently of an item's type, and no type-level assertion can observe
  them. A correctly typed item that is not exported still fails at the final
  link, which keeps ownership of that failure.
- Distinguishing Snacc types that share one ABI representation. `Bool` and
  `Nil` are both `u8`; swapping them in a declaration is undetectable by this
  mechanism. Semantic distinction would require distinct Rust types or separate
  bridge metadata, which is a larger design.
- Extending the bridge ABI. `Int64`, `Dec64`, `Bool`, and `Nil` remain the only
  types that cross the boundary. A later RFC owns strings, handles, aggregates,
  ownership, and error values.
- Generating Snacc declarations from Rust source, or Rust wrappers from Snacc
  declarations.
- Inferring which crate operations an application should expose.
- Reporting a Rust `snacc_user_*` item that no Snacc declaration references.
- Verifying bridges in the direct single-file workflow or the workbench, neither
  of which compiles application Rust source.
- Checking panics, unwinding, or memory safety inside a bridge body.
- Restructuring object emission to consume an already-checked program. The
  Cargo commands already check and then emit; removing that second parse is a
  separate performance change.
- Changing `cargo-snacc` command names, arguments, or successful output.

## Prerequisite

`LANGUAGE.md` is the sole normative language contract but is currently empty
(`TODO.md` housekeeping item). The bridge ABI — the `snacc_user_` symbol rule,
the four permitted bridge types, and the Snacc-to-Rust representation mapping —
belongs there. Populating it with the bridge ABI is a prerequisite of this RFC,
not a side effect. The table in "Type mapping" is the implementation mapping
that must match that contract; where they disagree, `LANGUAGE.md` governs and
the implementation is wrong.

## Relationship to other RFCs

- [RFC 003](archive/003-cargo-hosted-applications.md) defines the bridge
  workflow, `src/interop.rs`, and the host template. It anticipates this work as
  "generated bridge tooling". It is archived, so it records history rather than
  the active contract.
- [RFC 005](005-remove-runtime-rs.md) defines the generated host and the
  `snacc-runtime` linking contract that the host template extends here.
- [RFC 006](006-workspace-organization.md) assigns Cargo orchestration to
  `apps/cargo-snacc`, which owns everything in this RFC except the compiler
  manifest API.

## Decision

### Bridge item contract

This RFC makes the following normative, replacing the current convention:

1. A bridge function is a `pub` item of the host crate's `interop` module,
   reachable at `crate::interop::<symbol>`.
2. Its Rust item name is exactly the declared link symbol.
3. It carries `#[unsafe(no_mangle)]` and uses the `extern "C"` ABI.
4. It does not carry `#[export_name]`.

Rules 1 and 2 make the assertion addressable. Rules 3 and 4 keep the item name
and the exported symbol identical, so the assertion and the linker are talking
about the same function. The assertion enforces rules 1 and 2 by construction;
rules 3 and 4 are enforced by the linker, as they are today.

`cargo snacc init` already writes `mod interop;` into the host and an empty
`src/interop.rs`, so this codifies the layout it creates.

### Link symbols must be Rust identifiers

The checker currently accepts any string beginning with `snacc_user_`, including
strings that are not valid Rust identifiers. Such a symbol cannot appear in a
generated path, so the checker must additionally reject a link symbol that is
not a valid Rust identifier, with the same structured diagnostic shape as the
existing prefix rule. This narrows a surface no working bridge can use, because
rule 2 above requires the symbol to be a Rust item name.

### Compiler manifest API

`cargo-snacc` needs the checked external declarations as data. `snacc-compiler`
must expose, from the already-public `check` result:

- The Snacc function name and the declared link symbol.
- The declared parameter types and result type, as a public exhaustively
  matchable type.
- The declaration's source span, for the generated comment.

Concretely: `Ty` becomes part of the public API alongside `Program`, and the
checked external record carries the declaration span it currently discards.
Nothing else moves, and no phase gains a dependency on Cargo or `rustc`.

### Mechanism

For each external declaration in the checked program, `cargo-snacc` emits one
constant whose declared type is the ABI signature implied by the Snacc types and
whose value is the bridge item:

~~~rust
const _: unsafe extern "C" fn(i64) -> i64 = crate::interop::snacc_user_itoa_len; // snacc: itoa_len (src/main.nrs:1:1)
~~~

`rustc` accepts this only when the item has exactly that arity and exactly those
parameter and result types. A safe `extern "C"` item coerces to the `unsafe`
target, so both bridge spellings are accepted. A missing item is a
name-resolution error rather than a link error.

The assertion is a compile-time coercion. It emits no code and is never called.

### Type mapping

Each Snacc type determines exactly one Rust type. This mapping is copied from
the bridge ABI contract in `LANGUAGE.md`:

| Snacc | Rust | LLVM |
| --- | --- | --- |
| `Int64` | `i64` | `i64` |
| `Dec64` | `f64` | `double` |
| `Bool` | `u8` | `i8` |
| `Nil` | `u8` | `i8` |

`Bool` and `Nil` are one byte because that is what
`crates/snacc-compiler/src/backend/llvm.rs` lowers them to and what
`snacc-runtime` already declares. `Nil` is a value, not the absence of one: a
bridge returning `Nil` returns `u8`, not `()`.

The mapping is not injective, which is why `Bool` and `Nil` are
interchangeable as far as this verification can see. Making them distinguishable
means giving them distinct Rust types, which changes the ABI and belongs to the
RFC that revises it.

### Where the assertions are compiled

The host template gains two lines:

~~~rust
mod interop;

#[cfg(snacc_bridge_assertions)]
include!(env!("SNACC_BRIDGE_ASSERTIONS"));
~~~

`cargo-snacc` writes the generated file, passes `--cfg snacc_bridge_assertions`,
and sets `SNACC_BRIDGE_ASSERTIONS` to the file's absolute path when it invokes
Cargo. `include!` registers the file with `rustc`'s dependency information, so
Cargo rebuilds the host when the assertions change.

The `cfg` gate exists because a Snacc package must remain compilable by plain
Cargo. An unconditional `env!` is a hard compile error whenever the variable is
absent, which would break `cargo check`, `cargo clippy`, and every editor
running rust-analyzer against a Snacc package — none of which go through
`cargo-snacc`. `cfg` strips the item before expansion, so the `env!` is never
evaluated in those builds.

The gate trades one property for that. With an unconditional `env!`, a command
that forgot to set the variable would fail loudly; with the gate, a command that
forgets to pass the `cfg` silently skips every assertion. Two things contain
that: `cargo-snacc` must set the `cfg` and the variable together from one shared
helper so they cannot diverge, and the test plan requires each of the four
commands to be shown catching a real mismatch.

A program with no external declarations produces an assertion file containing
only a generated-file header comment. Both the `cfg` and the variable are set by
every command that compiles the host, so the host compiles identically whether
or not the program declares bridges.

### Generated file location and publication

Cargo's `target_directory` is shared across a workspace, so it is not a
package-private location. The assertion file path is:

~~~text
<target_directory>/snacc/bridges/<package-name>-<hash>.rs
~~~

where `<hash>` is derived from the package identity and the generated content.
Different packages, different programs, and concurrent invocations therefore
never contend for one path.

The file is written to a unique temporary sibling and published with an atomic
rename, so `rustc` never reads a partially written file. This is the same
publication discipline the object cache requires.

### Command behavior

`check`, `build`, `run`, and `test` each:

1. Check the Snacc source as they do today.
2. Generate the assertion file from the checked external declarations.
3. Validate the host source, as described below.
4. Pass `--cfg snacc_bridge_assertions` and set `SNACC_BRIDGE_ASSERTIONS` for
   their Cargo invocation, through one shared helper.

Every one of these compiles the host through its own `cargo` or `cargo rustc`
invocation, so each must call that helper; a command that skips it compiles a
host with no assertions rather than failing, which is why step 4 has exactly one
implementation and each command has a test proving it catches a mismatch.
`check` additionally gains steps 2 and 3, which it has no equivalent of today.

`clean` removes the generated assertion directory along with the artifacts it
already removes. `doctor` and `init` do not compile the host and are otherwise
unaffected.

Plain `cargo` commands remain valid on a Snacc package and behave exactly as
they do today: the `cfg` is absent, no assertion is compiled, and the host still
fails to link on its own because the Snacc object is missing.

Object emission and caching are unchanged: the assertion file participates in
the host build, never in the Snacc object identity.

### Host validation

`cargo-snacc` locates the host source through the selected binary target's
`src_path` in Cargo metadata, which the metadata model must retain alongside the
target name it already reads.

When the program declares at least one external function, the host source must
contain a line whose trimmed form begins with `include!` and names
`SNACC_BRIDGE_ASSERTIONS`, preceded by a line whose trimmed form is the
`#[cfg(snacc_bridge_assertions)]` attribute. If either is absent, the command
fails with a diagnostic naming the file and the exact lines to add. Without this
check a host missing them would silently skip every assertion, which is the one
failure mode this RFC must not have.

This is a lexical check with a stated ceiling: a developer who comments the line
out, or excludes it with `cfg`, disables the assertions. That is a deliberate
opt-out, in the same category as `#[allow]`, and is not defended against.
Detecting it robustly would require the compiler to emit an undefined reference
that only the generated file satisfies, trading a clear diagnostic for a link
error; that trade is only worth revisiting if opt-out proves to be an accident
people have in practice.

### Diagnostics

Each assertion occupies one line and carries a trailing comment naming the Snacc
function and its declaration site. `rustc` prints the offending line with that
comment, so a mismatch identifies both the expected ABI signature and the Snacc
declaration that produced it, without any span remapping:

~~~text
error[E0308]: mismatched types
 --> target/snacc/bridges/app-3f9c1a.rs:3:45
  |
3 | const _: unsafe extern "C" fn(i64) -> i64 = crate::interop::snacc_user_itoa_len; // snacc: itoa_len (src/main.nrs:1:1)
  |          -------------------------------   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `i64`, found `f64`
~~~

This is a deliberate ceiling: the error points at a generated file rather than
at the `.nrs` declaration. Mapping the failing line back to the Snacc span means
parsing `--message-format=json` diagnostics and re-rendering, which
`cargo-snacc` should add only if the generated-file location proves confusing in
practice. The line-comment form costs nothing and carries the same information.

Failure ownership extends RFC 003's table:

| Failure | Owner |
| --- | --- |
| Bridge item's ABI type disagrees with its declaration | Generated assertion, through `rustc` |
| Declared bridge has no `crate::interop` item | Generated assertion, through `rustc` |
| Host crate has no `interop` module at all | Generated assertion, through `rustc` |
| Bridge item exists but is not exported under its symbol | Final native link |
| Link symbol is not a valid Rust identifier | Compiler diagnostic |
| Host is missing the assertion include | `cargo-snacc` |

Only the first two rows move a failure earlier. The third stays with the linker
by design.

## Implementation plan

### 1. Expose the checked bridge data

- Add the declaration span to the checked external record.
- Re-export `Ty` so `cargo-snacc` can match it exhaustively.
- Reject link symbols that are not valid Rust identifiers, with the same
  diagnostic shape as the existing prefix rule.

### 2. Emit the assertion file

- Add a `cargo-snacc` function turning checked external declarations into
  assertion source using the type mapping.
- Begin the file with a generated-file header comment naming this RFC, so a
  developer who opens it from a diagnostic knows what wrote it and that editing
  it has no effect. A program with no externals produces the header alone.
- Sort declarations by Snacc function name so the file is deterministic.
- Name the file `<package-name>-<hash>.rs`, hashing the package id and the
  generated content with the `sha2` dependency the object cache already uses.
- Publish through a unique temporary sibling and an atomic rename.
- Record each declaration's `.nrs` line and column in the trailing comment.

### 3. Wire it into every host-compiling command

- Add one shared helper that generates the file, passes
  `--cfg snacc_bridge_assertions`, and sets `SNACC_BRIDGE_ASSERTIONS`. Call it
  from `check`, `build`, `run`, and `test`; nothing else sets either.
- Retain `src_path` for the selected binary target in the metadata model.
- Validate the host `cfg` and include lines when the program declares externals.
- Extend `clean` to remove the generated assertion directory.
- Keep object caching and cache identity unchanged.

### 4. Update the host template

- Add the `cfg` and `include!` lines to the template `cargo snacc init` writes.
- Accept both the old and new host template where `init` refuses to overwrite a
  non-template host.
- Diagnose an existing host that lacks the line, naming the exact line to add,
  rather than editing a user-owned file.
- Update `tests/fixtures/cargo-hosted/src/main.rs` to the new template.

### 5. Populate the bridge ABI contract

- Write the `snacc_user_` symbol rule, the bridge item contract, the four
  permitted types, and the representation mapping into `LANGUAGE.md`.
- Confirm the implementation mapping matches it.

## Testing

- A correct bridge builds, links, runs, and prints its expected output.
- Changing one Rust parameter type fails before linking with a type error.
- Changing the Rust result type fails before linking.
- Adding or removing a Rust parameter fails before linking.
- Declaring an `extern rust` bridge with no `crate::interop` item fails with a
  name-resolution error, not a link error.
- Each of `check`, `build`, `run`, and `test` performs the verification, with
  and without bridge declarations present.
- A program with no external declarations builds with a header-only assertion
  file.
- A host missing either the `cfg` line or the include line reports an actionable
  diagnostic naming the file and the lines.
- Plain `cargo check` and `cargo clippy` on a Snacc package succeed exactly as
  they do today, with no environment-variable error, proving the `cfg` gate
  keeps rust-analyzer working.
- A host crate declaring bridges but having no `interop` module fails with a
  module-resolution error.
- A link symbol that is not a valid Rust identifier is rejected by the checker.
- The generated file is byte-identical across repeated runs for one program, and
  two packages in one workspace generate to distinct paths without contention.
- Concurrent generation for the same package does not expose a partially written
  file to `rustc`.
- Each of the four bridge types round-trips through a bridge parameter and a
  bridge result.
- A correctly typed bridge item without `#[unsafe(no_mangle)]` still fails at
  link, confirming the documented division of ownership.

## Compatibility and migration

Snacc syntax, type rules, the bridge ABI, symbol naming, and program behavior
are unchanged for every bridge that follows the contract above. A correct bridge
that builds today builds after this RFC.

Three categories begin failing:

- A bridge whose Rust type disagrees with its declaration now fails to compile.
  That is the purpose of the change, and any program it breaks was already
  undefined behavior.
- A bridge item outside `crate::interop`, or whose item name differs from its
  link symbol, now fails to compile. This was previously legal and working.
  Migration is moving or renaming the item, and the diagnostic names it.
- A link symbol that is not a valid Rust identifier is now a compiler
  diagnostic. No working bridge can have one, because the item name must match.

Existing packages need two lines added to their host `src/main.rs`. The
diagnostic names them. `cargo snacc init` writes them for new packages.

## Rejected alternatives

### Verify the exported symbol rather than the item

There is no type-level construct that observes `#[unsafe(no_mangle)]` or
`#[export_name]`, so no assertion can prove an item is exported under a given
name. Declaring the symbol in an `extern` block and comparing does not help:
`rustc` does not check an extern declaration against a definition in the same
crate, which is precisely today's hole. The linker already owns this and keeps
it.

### Generate the Snacc declaration from Rust source

Requires a Rust parser in `cargo-snacc` and makes the `.nrs` file depend on a
generated include, so Snacc source stops standing alone. The generated
declarations also become a committed artifact that can go stale.

### Generate the Rust wrapper from the Snacc declaration

Correct by construction, but it puts `cargo-snacc` in the business of writing
the code that calls a user's chosen crate operation. RFC 003 deliberately keeps
that choice with the application author. A generator is worth revisiting when
richer types make wrappers mechanical; for four scalars it is not.

### Encode the signature in the symbol name

Hashing the signature into the exported symbol makes a mismatch a link failure
with no tooling at all, and it verifies the exported symbol rather than the
item, closing the gap this RFC leaves open. It produces an unreadable diagnostic
naming a hash instead of two types, and forces every bridge through a macro to
compute the name. The coercion gives a real type error for the same cost; the
residual gap stays with the linker, which reports a missing symbol by name.

### Verify at runtime

A bridge that already read the wrong argument cannot report it. Any check after
the call is too late, and one before it pays for every call.

### Extend the final linker check

The linker sees names, not types. It cannot detect the failure this RFC exists
to detect.

## Acceptance criteria

This RFC is implemented when:

1. `check`, `build`, `run`, and `test` each compile a generated assertion for
   every external declaration in the checked program.
2. An arity disagreement, or a disagreement in the Rust type of any parameter or
   result, fails before linking.
3. A declared bridge with no `crate::interop` item fails before linking.
4. The generated file is deterministic, package- and content-addressed,
   published atomically, and never edited or committed.
5. The implementation mapping matches the bridge ABI contract in `LANGUAGE.md`,
   which states it normatively.
6. The checker rejects a link symbol that is not a valid Rust identifier.
7. A host missing the assertion `cfg` or include fails with a diagnostic naming
   the file and the lines to add.
8. Plain Cargo commands on a Snacc package behave as they did before this RFC,
   with no environment-variable error.
9. Correct existing bridges that satisfy the bridge item contract build, run,
   and test with unchanged output.
10. `cargo snacc init` writes a host containing the assertion cfg and include.
11. `snacc-compiler` gains no dependency on Cargo, `rustc`, or host crate
    layout, and exposes the bridge data as a public exhaustive API.
12. `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and
    the compiler, Cargo subcommand, and fixture test suites pass.

## Future considerations

- Remapping assertion failures onto `.nrs` declaration spans.
- Reporting a `snacc_user_*` item that no declaration references.
- A bridge module path other than `crate::interop`, if a package needs one.
- Distinguishing `Bool` from `Nil` at the ABI, which requires distinct Rust
  representations.
- Lowering an already-checked program without re-parsing, removing the second
  check the Cargo commands perform today.
- Richer bridge types — strings, opaque handles, `Result` — which this
  verification is a precondition for, not a substitute for.

## References

- [RFC 003: Cargo-hosted applications](archive/003-cargo-hosted-applications.md)
- [RFC 006: Rust workspace organization](006-workspace-organization.md)
- [Rust type coercions](https://doc.rust-lang.org/reference/type-coercions.html)
- [Rust ABI attributes](https://doc.rust-lang.org/reference/abi.html)
