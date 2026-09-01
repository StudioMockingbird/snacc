# RFC 007 Bridge Signature Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `cargo-snacc` prove every `extern rust` declaration's ABI type against the real `crate::interop` item before linking, by generating one `const _: unsafe extern "C" fn(...) -> ... = crate::interop::<symbol>;` coercion per bridge and compiling it into the host behind a `#[cfg(snacc_bridge_assertions)]` gate.

**Architecture:** `snacc-compiler`'s checker keeps each `extern rust` declaration's source span and rejects link symbols that are not valid Rust identifiers; it already exposes `Program.externs: HashMap<String, TExtern>` and gains a public `Ty` export so `cargo-snacc` can match it exhaustively. `cargo-snacc` renders one deterministic, content-addressed Rust source file from the checked externs, publishes it atomically under `<target_directory>/snacc/bridges/`, and threads `--cfg snacc_bridge_assertions` plus `SNACC_BRIDGE_ASSERTIONS=<path>` through `check`, `build`, `run`, and `test` via one shared helper. The host template gains a two-line `include!` gated by that `cfg`, so plain Cargo/`rust-analyzer` never evaluates the `env!()`.

**Tech Stack:** Rust (edition 2024, workspace at `C:\Users\Rishav\GitHub\snacc`), `sha2` and `tempfile` (already workspace dependencies, no new dependency needed), `snacc-compiler` (chumsky front end, Inkwell/LLVM 22 backend).

**Spec:** [docs/specs/007-bridge-signature-verification.md](../007-bridge-signature-verification.md)

## Global Constraints

- Concrete procedural Rust: explicit structs/enums, `match`, ordinary loops; no traits/generics/dynamic dispatch added unless they remove real duplication (AGENTS.md "Implementation").
- Keep `apps/cargo-snacc` a single `src/main.rs` file, matching its current structure — do not split into modules for this change.
- No new workspace dependency: `sha2` and `tempfile` already cover hashing and atomic temp-file publication.
- Do not leak `chumsky::span::SimpleSpan` out of `snacc-compiler`'s public API; convert to `std::ops::Range<usize>` at the crate boundary, mirroring the existing `Diagnostic.span` conversion in `crates/snacc-compiler/src/lib.rs`.
- `LANGUAGE.md` is the sole normative language contract and must be updated in the same change that changes the bridge contract (AGENTS.md "Documentation").
- Before handoff: `cargo fmt`, `cargo check --workspace --all-targets`, and the compiler + `cargo-snacc` test suites must pass (AGENTS.md "Change discipline", RFC 007 acceptance criterion 12).
- This directory (`C:\Users\Rishav\GitHub\snacc`) *is* the Git repository for this work (unlike `docs/specs`'s own note about a different, non-Git directory) — normal commit discipline applies.
- Terminal spec statuses are `Closed`, `Discarded`, `Superseded`, `Rejected` — never `Completed` (AGENTS.md "Specification format"; the archived 001–003 RFCs predate this rule and are not a precedent to copy).

---

### Task 1: Expose checked bridge data from `snacc-compiler`

**Files:**
- Modify: `crates/snacc-compiler/src/semantics/checker.rs:93-97` (`TExtern`), `:169-187` (construction), `:134-149` (prefix validation)
- Modify: `crates/snacc-compiler/src/lib.rs:8-10` (public re-exports)
- Test: `crates/snacc-compiler/src/semantics/checker.rs` (`#[cfg(test)] mod tests`, currently `:464-505`)

**Interfaces:**
- Produces: `pub struct TExtern { pub symbol: String, pub params: Vec<(String, Ty)>, pub ret: Ty, pub span: std::ops::Range<usize> }` (new `span` field); `snacc_compiler::Ty` as a public, exhaustively matchable 4-variant enum (`Int64`, `Dec64`, `Bool`, `Nil`).
- Consumes: nothing new from other tasks (this is the foundation task).

- [ ] **Step 1: Write the failing tests**

Add to `crates/snacc-compiler/src/semantics/checker.rs`'s existing `#[cfg(test)] mod tests` block (after `checks_typed_rust_bridge_calls`):

```rust
    #[test]
    fn checked_externs_carry_their_declaration_span() {
        let source = "extern rust \"snacc_user_double\" fun rust_double(value: Int64): Int64\nprint(rust_double(2))";
        let syntax = crate::parse(source).expect("bridge declaration should parse");
        let program = check(&syntax).expect("bridge call should type check");
        let span = &program.externs["rust_double"].span;
        assert_eq!(span.start, 0);
        assert!(span.end > span.start && span.end <= source.find('\n').unwrap());
    }

    #[test]
    fn rejects_bridge_symbols_that_are_not_rust_identifiers() {
        let source = "extern rust \"snacc_user_bad-name\" fun bad(): Nil\nprint(0)";
        let syntax = crate::parse(source).expect("declaration should parse");
        match check(&syntax) {
            Err(Failure::Source(errors)) => {
                assert!(
                    errors
                        .iter()
                        .any(|error| error.msg.contains("valid Rust identifiers")),
                    "expected a Rust-identifier diagnostic"
                );
            }
            _ => panic!("a non-identifier bridge symbol should fail type checking"),
        }
    }

    #[test]
    fn accepts_bridge_symbols_with_digits_and_underscores() {
        let source = "extern rust \"snacc_user_v2_ok\" fun ok(): Nil\nprint(0)";
        let syntax = crate::parse(source).expect("declaration should parse");
        assert!(check(&syntax).is_ok());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p snacc-compiler checker::tests`
Expected: `checked_externs_carry_their_declaration_span` fails to compile (no `span` field on `TExtern`); `rejects_bridge_symbols_that_are_not_rust_identifiers` fails (currently `check` succeeds for a hyphenated symbol).

- [ ] **Step 3: Add the `span` field and thread it through construction**

In `crates/snacc-compiler/src/semantics/checker.rs`, change the `TExtern` struct (currently lines 93-97):

```rust
pub struct TExtern {
    pub symbol: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub span: std::ops::Range<usize>,
}
```

Then update the construction loop (currently lines 169-187) to populate it:

```rust
    let mut typed_externs = HashMap::new();
    let mut extern_names: Vec<&str> = program.externs.keys().copied().collect();
    extern_names.sort_unstable();
    for name in extern_names {
        let function = &program.externs[name];
        let params = function
            .args
            .iter()
            .map(|param| (param.name.to_string(), param.ty.into()))
            .collect();
        typed_externs.insert(
            name.to_string(),
            TExtern {
                symbol: function.symbol.to_string(),
                params,
                ret: function.ret.into(),
                span: function.span.into_range(),
            },
        );
    }
```

`Span::into_range()` already exists and is used the same way in `crates/snacc-compiler/src/lib.rs:83` (`Some(error.span.into_range())`), so this keeps `chumsky::span::SimpleSpan` out of the public field type.

- [ ] **Step 4: Add the Rust-identifier check**

In the same file, add a small free function near the other free functions (e.g. directly above `fn numeric`):

```rust
fn is_rust_identifier(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
```

Then extend the existing prefix check (currently lines 143-148) with an `else if`:

```rust
        if !function.symbol.starts_with("snacc_user_") {
            ctx.errors.push(Error {
                span: function.span,
                msg: "Rust bridge symbols must start with 'snacc_user_'".into(),
            });
        } else if !is_rust_identifier(function.symbol) {
            ctx.errors.push(Error {
                span: function.span,
                msg: "Rust bridge symbols must be valid Rust identifiers".into(),
            });
        }
```

- [ ] **Step 5: Export `Ty` from the crate root**

In `crates/snacc-compiler/src/lib.rs`, change:

```rust
pub use diagnostics::{Diagnostic, DiagnosticPhase, Diagnostics};
pub use semantics::checker::Program;
pub use syntax::ast::Program as AstProgram;
```

to:

```rust
pub use diagnostics::{Diagnostic, DiagnosticPhase, Diagnostics};
pub use semantics::checker::{Program, Ty};
pub use syntax::ast::Program as AstProgram;
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p snacc-compiler`
Expected: PASS, including the three new tests and every existing `checker`/`parser`/`lexer` test.

- [ ] **Step 7: Commit**

```bash
git add crates/snacc-compiler/src/semantics/checker.rs crates/snacc-compiler/src/lib.rs
git commit -m "feat(compiler): expose bridge span and export Ty for RFC 007"
```

---

### Task 2: Capture the host binary's `src_path` in `cargo-snacc`'s metadata model

**Files:**
- Modify: `apps/cargo-snacc/src/main.rs:50-54` (`Target`), `:56-62` (`Selected`), `:649-667` (host-target selection)
- Modify: `apps/cargo-snacc/src/main.rs:1128-1145` (`selected()` test helper)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `Selected.host_src_path: PathBuf` — the absolute path to the host binary's `src/main.rs`, used by Task 4's host validation.

- [ ] **Step 1: Write the failing test**

Add to `apps/cargo-snacc/src/main.rs`'s `#[cfg(test)] mod tests` block, near `cargo_artifact_selection_requires_package_target_and_kind`:

```rust
    #[test]
    fn selected_test_helper_carries_a_host_src_path() {
        assert_eq!(
            selected().host_src_path,
            PathBuf::from("C:/workspace/src/main.rs")
        );
    }
```

(This test only compiles once `Selected` and the `selected()` helper both carry `host_src_path`, so it doubles as the compile-gate for this task.)

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p cargo-snacc selected_test_helper_carries_a_host_src_path`
Expected: compile error, no field `host_src_path` on `Selected`.

- [ ] **Step 3: Add `src_path` to `Target` and `host_src_path` to `Selected`**

Change the `Target` struct (currently lines 50-54):

```rust
#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}
```

Change the `Selected` struct (currently lines 56-62):

```rust
struct Selected {
    package: Package,
    package_id: String,
    entry: PathBuf,
    host_bin: String,
    host_src_path: PathBuf,
    target_directory: PathBuf,
}
```

- [ ] **Step 4: Thread `host_src_path` through package selection**

Replace the host-target counting block (currently lines 649-658):

```rust
    let host_target = package
        .targets
        .iter()
        .filter(|target| target.name == host_bin && target.kind.iter().any(|kind| kind == "bin"))
        .collect::<Vec<_>>();
    if host_target.len() != 1 {
        return Err(CliError(format!(
            "host binary '{host_bin}' does not resolve to exactly one binary target"
        )));
    }
    let host_src_path = host_target[0].src_path.clone();
```

and update the `Selected` construction immediately after (currently lines 659-666) to include it:

```rust
    let package_id = package.id.clone();
    Ok(Some(Selected {
        package,
        package_id,
        entry,
        host_bin,
        host_src_path,
        target_directory: metadata.target_directory,
    }))
```

- [ ] **Step 5: Update the `selected()` test helper**

In the `#[cfg(test)] mod tests` block, change `selected()` (currently lines 1128-1145) to:

```rust
    fn selected() -> Selected {
        Selected {
            package: Package {
                id: "path+file:///workspace#app@0.1.0".into(),
                name: "app".into(),
                manifest_path: PathBuf::from("C:/workspace/Cargo.toml"),
                targets: vec![Target {
                    name: "app".into(),
                    kind: vec!["bin".into()],
                    src_path: PathBuf::from("C:/workspace/src/main.rs"),
                }],
                metadata: None,
            },
            package_id: "path+file:///workspace#app@0.1.0".into(),
            entry: PathBuf::from("C:/workspace/src/main.nrs"),
            host_bin: "app".into(),
            host_src_path: PathBuf::from("C:/workspace/src/main.rs"),
            target_directory: PathBuf::from("C:/workspace/target"),
        }
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p cargo-snacc`
Expected: PASS (all existing unit tests still compile and pass with the new fields).

- [ ] **Step 7: Commit**

```bash
git add apps/cargo-snacc/src/main.rs
git commit -m "feat(cargo-snacc): retain the host binary's src_path in package selection"
```

---

### Task 3: Render and publish the bridge-assertion file

**Files:**
- Modify: `apps/cargo-snacc/src/main.rs:1-9` (imports), `:696-707` (extract `package_relative_entry`)
- Test: `apps/cargo-snacc/src/main.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `snacc_compiler::Program`, `snacc_compiler::Ty` (Task 1); `Selected.package_id`, `.package.name`, `.target_directory` (existing fields, unchanged by Task 2).
- Produces: `fn render_bridge_assertions(checked: &Program, entry: &Path, source: &str) -> String` (pure, used directly by Task 7's unit tests); `fn write_bridge_assertions(selected: &Selected, checked: &Program, source: &str) -> Result<PathBuf, CliError>` (used by Task 4).

- [ ] **Step 1: Write the failing tests**

Add to `apps/cargo-snacc/src/main.rs`'s `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn render_bridge_assertions_sorts_by_snacc_name_and_maps_types() {
        let source = concat!(
            "extern rust \"snacc_user_zeta\" fun zeta(value: Bool): Nil\n",
            "extern rust \"snacc_user_alpha\" fun alpha(a: Int64, b: Dec64): Bool\n",
            "print(0)\n"
        );
        let checked = check(source).expect("bridge declarations should type check");
        let rendered = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        let alpha_line = rendered
            .lines()
            .find(|line| line.contains("snacc_user_alpha"))
            .expect("alpha assertion line");
        let zeta_line = rendered
            .lines()
            .find(|line| line.contains("snacc_user_zeta"))
            .expect("zeta assertion line");
        assert!(rendered.find(alpha_line).unwrap() < rendered.find(zeta_line).unwrap());
        assert!(alpha_line.contains("fn(i64, f64) -> u8"));
        assert!(alpha_line.contains("crate::interop::snacc_user_alpha"));
        assert!(alpha_line.contains("// snacc: alpha (src/main.nrs:2:1)"));
        assert!(zeta_line.contains("fn(u8) -> u8"));
        assert!(zeta_line.contains("// snacc: zeta (src/main.nrs:1:1)"));
    }

    #[test]
    fn render_bridge_assertions_with_no_externs_is_header_only() {
        let source = "print(0)\n";
        let checked = check(source).expect("source should type check");
        let rendered = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        assert!(rendered.starts_with("// Generated by cargo-snacc"));
        assert!(!rendered.contains("const _:"));
    }

    #[test]
    fn render_bridge_assertions_is_deterministic() {
        let source = "extern rust \"snacc_user_a\" fun a(): Int64\nprint(0)\n";
        let checked = check(source).expect("source should type check");
        let first = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        let second = render_bridge_assertions(&checked, Path::new("src/main.nrs"), source);
        assert_eq!(first, second);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cargo-snacc render_bridge_assertions`
Expected: compile error, `render_bridge_assertions` and `Program`/`Ty` are not in scope yet.

- [ ] **Step 3: Import `Program` and `Ty`**

Change the `snacc_compiler` import (currently line 4):

```rust
use snacc_compiler::{
    CompileOptions, Diagnostics, Optimization, Program, Ty, check, emit_object_with_options,
};
```

- [ ] **Step 4: Extract `package_relative_entry` and add the rendering functions**

Replace the inline relative-entry computation inside `emit_cached` (currently lines 696-707):

```rust
    let relative_entry = selected
        .entry
        .strip_prefix(
            selected
                .package
                .manifest_path
                .parent()
                .unwrap_or(Path::new(".")),
        )
        .unwrap_or(&selected.entry);
    hash.update(relative_entry.to_string_lossy().as_bytes());
    hash.update(source.as_bytes());
```

with:

```rust
    hash.update(package_relative_entry(selected).to_string_lossy().as_bytes());
    hash.update(source.as_bytes());
```

Then add these free functions (a good spot is directly below `emit_cached`, before `backend_build_id`):

```rust
fn package_relative_entry(selected: &Selected) -> &Path {
    selected
        .entry
        .strip_prefix(
            selected
                .package
                .manifest_path
                .parent()
                .unwrap_or(Path::new(".")),
        )
        .unwrap_or(&selected.entry)
}

fn rust_abi_type(ty: Ty) -> &'static str {
    match ty {
        Ty::Int64 => "i64",
        Ty::Dec64 => "f64",
        Ty::Bool | Ty::Nil => "u8",
    }
}

fn render_bridge_assertions(checked: &Program, entry: &Path, source: &str) -> String {
    let mut names: Vec<&String> = checked.externs.keys().collect();
    names.sort();
    let mut rendered = String::from(
        "// Generated by cargo-snacc from checked `extern rust` declarations (RFC 007).\n\
         // Do not edit; this file is regenerated on every build.\n",
    );
    for name in names {
        let extern_decl = &checked.externs[name];
        let params = extern_decl
            .params
            .iter()
            .map(|(_, ty)| rust_abi_type(*ty))
            .collect::<Vec<_>>()
            .join(", ");
        let (line, column) = line_column(source, extern_decl.span.start);
        rendered.push_str(&format!(
            "const _: unsafe extern \"C\" fn({params}) -> {ret} = crate::interop::{symbol}; // snacc: {name} ({entry}:{line}:{column})\n",
            ret = rust_abi_type(extern_decl.ret),
            symbol = extern_decl.symbol,
            entry = entry.display(),
        ));
    }
    rendered
}

fn write_bridge_assertions(
    selected: &Selected,
    checked: &Program,
    source: &str,
) -> Result<PathBuf, CliError> {
    let content = render_bridge_assertions(checked, package_relative_entry(selected), source);
    let mut hash = Sha256::new();
    hash.update(selected.package_id.as_bytes());
    hash.update(content.as_bytes());
    let digest = format!("{:x}", hash.finalize());
    let directory = selected.target_directory.join("snacc").join("bridges");
    fs::create_dir_all(&directory).map_err(io_error)?;
    let path = directory.join(format!("{}-{}.rs", selected.package.name, digest));
    if !path.is_file() {
        let mut temp = tempfile::Builder::new()
            .prefix(&format!("{}-{}-", selected.package.name, digest))
            .suffix(".rs.tmp")
            .tempfile_in(&directory)
            .map_err(io_error)?;
        temp.write_all(content.as_bytes()).map_err(io_error)?;
        if let Err(error) = temp.persist(&path) {
            if !path.is_file() {
                return Err(CliError(format!(
                    "failed to publish bridge assertions: {error}"
                )));
            }
        }
    }
    Ok(path)
}
```

`temp.write_all` uses the `std::io::Write` trait already imported at the top of the file (`use std::io::Write;`, line 7). The `if let Err(error) = temp.persist(&path) { if !path.is_file() { ... } }` guard makes two concurrent `cargo-snacc` invocations that derive the same content-addressed path benign: whichever process loses the rename race simply observes the winner's (byte-identical) file already there instead of erroring.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cargo-snacc render_bridge_assertions`
Expected: PASS.

- [ ] **Step 6: Run the full `cargo-snacc` unit test suite**

Run: `cargo test -p cargo-snacc --lib`
Expected: PASS (confirms the `emit_cached` refactor didn't change its hashing behavior — no test currently pins the object-cache identity hash byte-for-byte, so this is a compile+regression check).

- [ ] **Step 7: Commit**

```bash
git add apps/cargo-snacc/src/main.rs
git commit -m "feat(cargo-snacc): render and publish deterministic bridge assertions"
```

---

### Task 4: Validate the host include and wire assertions into every host-compiling command

**Files:**
- Modify: `apps/cargo-snacc/src/main.rs:203-220` (`check_command`), `:222-282` (`build_command`), `:305-360` (`test_command`)
- Test: `apps/cargo-snacc/src/main.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `write_bridge_assertions`, `render_bridge_assertions` (Task 3); `Selected.host_src_path` (Task 2); `Program` (Task 1).
- Produces: `fn prepare_bridge_assertions(selected: &Selected, checked: &Program, source: &str) -> Result<BridgeAssertions, CliError>` and `fn apply_bridge_assertions(command: &mut Command, assertions: &BridgeAssertions)` — the single shared pair RFC 007 requires every host-compiling command to call, so `--cfg snacc_bridge_assertions` and `SNACC_BRIDGE_ASSERTIONS` are never set anywhere else.

- [ ] **Step 1: Write the failing unit test for host validation**

Add to `apps/cargo-snacc/src/main.rs`'s `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn declares_bridge_assertion_include_requires_the_exact_pair() {
        assert!(declares_bridge_assertion_include(
            "mod interop;\n\n#[cfg(snacc_bridge_assertions)]\ninclude!(env!(\"SNACC_BRIDGE_ASSERTIONS\"));\n"
        ));
        assert!(!declares_bridge_assertion_include("mod interop;\n"));
        assert!(!declares_bridge_assertion_include(
            "#[cfg(snacc_bridge_assertions)]\nfn unrelated() {}\n"
        ));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cargo-snacc declares_bridge_assertion_include_requires_the_exact_pair`
Expected: compile error, `declares_bridge_assertion_include` does not exist.

- [ ] **Step 3: Add the host-validation and assertion-wiring helpers**

Add these functions near `write_bridge_assertions` (from Task 3):

```rust
struct BridgeAssertions {
    path: PathBuf,
}

fn declares_bridge_assertion_include(host_source: &str) -> bool {
    let lines: Vec<&str> = host_source.lines().map(str::trim).collect();
    lines.windows(2).any(|pair| {
        pair[0] == "#[cfg(snacc_bridge_assertions)]"
            && pair[1].starts_with("include!")
            && pair[1].contains("SNACC_BRIDGE_ASSERTIONS")
    })
}

fn validate_host_assertion_include(
    selected: &Selected,
    checked: &Program,
) -> Result<(), CliError> {
    if checked.externs.is_empty() {
        return Ok(());
    }
    let host_source = fs::read_to_string(&selected.host_src_path).map_err(io_error)?;
    if declares_bridge_assertion_include(&host_source) {
        Ok(())
    } else {
        Err(CliError(format!(
            "host '{}' is missing the bridge assertion include; add these two lines after 'mod interop;':\n#[cfg(snacc_bridge_assertions)]\ninclude!(env!(\"SNACC_BRIDGE_ASSERTIONS\"));",
            selected.host_src_path.display()
        )))
    }
}

fn prepare_bridge_assertions(
    selected: &Selected,
    checked: &Program,
    source: &str,
) -> Result<BridgeAssertions, CliError> {
    validate_host_assertion_include(selected, checked)?;
    let path = write_bridge_assertions(selected, checked, source)?;
    Ok(BridgeAssertions { path })
}

fn apply_bridge_assertions(command: &mut Command, assertions: &BridgeAssertions) {
    command.env("SNACC_BRIDGE_ASSERTIONS", &assertions.path);
    command.arg("--cfg").arg("snacc_bridge_assertions");
}
```

`apply_bridge_assertions` is the *only* place that sets the `cfg` or the environment variable; every call site below routes through it.

- [ ] **Step 4: Wire `check_command`**

Change `check_command` (currently lines 203-220) from:

```rust
fn check_command(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_options(args)?;
    reject_extra(&parsed, "check")?;
    let options = parsed.options;
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    check(&source)
        .map_err(|diagnostics| diagnostic_error(&selected.entry, &source, &diagnostics))?;
    let mut command = Command::new(cargo());
    command
        .arg("check")
        .arg("--manifest-path")
        .arg(selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name);
    append_cargo_options(&mut command, &options);
    run_forwarded(command, "cargo check")
}
```

to:

```rust
fn check_command(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_options(args)?;
    reject_extra(&parsed, "check")?;
    let options = parsed.options;
    if options.release || options.profile.is_some() {
        return Err(CliError(
            "check does not support --release or --profile; use build --release to check in release mode".into(),
        ));
    }
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    let checked = check(&source)
        .map_err(|diagnostics| diagnostic_error(&selected.entry, &source, &diagnostics))?;
    let assertions = prepare_bridge_assertions(&selected, &checked, &source)?;
    let mut command = Command::new(cargo());
    command
        .arg("rustc")
        .arg("--profile")
        .arg("check")
        .arg("--manifest-path")
        .arg(selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name)
        .arg("--bin")
        .arg(&selected.host_bin);
    append_metadata_options(&mut command, &options);
    command.arg("--");
    apply_bridge_assertions(&mut command, &assertions);
    run_forwarded(command, "cargo check")
}
```

**Correction discovered during Task 5's verification (not in the original plan text):** plain `cargo check` does not accept a trailing `-- <rustc-args>` section at all — `cargo check -- --cfg foo` fails with `error: unexpected argument '--cfg' found`, verified empirically. Only `cargo rustc` accepts rustc-passthrough args after `--`. `cargo rustc --profile check --bin <name>` reproduces `cargo check`'s fast, non-linking behavior while still accepting the passthrough. `--profile check` cannot combine with `--release` or a user `--profile` (both verified to error), so `check_command` now rejects those up front instead of silently dropping them or hitting Cargo's own conflict error, and uses `append_metadata_options` (the existing helper that forwards `--all-features`/`--no-default-features`/`--features`/`--locked`/`--frozen`/`--offline`, already used for the `cargo metadata` call in `select_package_optional`) instead of `append_cargo_options` (which would re-add the now-forbidden `--release`/`--profile`).

- [ ] **Step 5: Wire `build_command`**

In `build_command` (currently lines 222-282), replace:

```rust
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    let object = emit_cached(&selected, &source, &options)?;

    let mut command = Command::new(cargo());
    command
        .arg("rustc")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name)
        .arg("--bin")
        .arg(&selected.host_bin);
    append_cargo_options(&mut command, &options);
    command
        .arg("--")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()));
```

with:

```rust
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    let checked = check(&source)
        .map_err(|diagnostics| diagnostic_error(&selected.entry, &source, &diagnostics))?;
    let assertions = prepare_bridge_assertions(&selected, &checked, &source)?;
    let object = emit_cached(&selected, &source, &options)?;

    let mut command = Command::new(cargo());
    command
        .arg("rustc")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name)
        .arg("--bin")
        .arg(&selected.host_bin);
    append_cargo_options(&mut command, &options);
    command.arg("--");
    apply_bridge_assertions(&mut command, &assertions);
    command
        .arg("-C")
        .arg(format!("link-arg={}", object.display()));
```

`check(&source)` now runs once here and again inside `emit_cached` on a cache miss (`emit_object_with_options` calls it internally). RFC 007 explicitly defers removing that redundancy to a separate performance change ("Restructuring object emission to consume an already-checked program" is a stated non-goal) — `check` is the cheap parse+typecheck pass, not the cached LLVM codegen, so this is acceptable as-is.

`build_command` handles both `build` and `run` (called as `build_command(args, true)`), so this single edit covers both.

- [ ] **Step 6: Wire `test_command`**

In `test_command` (currently lines 305-360), replace:

```rust
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    let object = emit_cached(&selected, &source, &options)?;

    let mut command = Command::new(cargo());
    command
        .arg("rustc")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name)
        .arg("--bin")
        .arg(&selected.host_bin);
    append_cargo_options(&mut command, &options);
    command
        .arg("--")
        .arg("--test")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()));
```

with:

```rust
    let selected = select_package(options.package.as_deref(), Some(&options))?;
    let source = fs::read_to_string(&selected.entry).map_err(io_error)?;
    let checked = check(&source)
        .map_err(|diagnostics| diagnostic_error(&selected.entry, &source, &diagnostics))?;
    let assertions = prepare_bridge_assertions(&selected, &checked, &source)?;
    let object = emit_cached(&selected, &source, &options)?;

    let mut command = Command::new(cargo());
    command
        .arg("rustc")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&selected.package.manifest_path)
        .arg("--package")
        .arg(&selected.package.name)
        .arg("--bin")
        .arg(&selected.host_bin);
    append_cargo_options(&mut command, &options);
    command.arg("--");
    apply_bridge_assertions(&mut command, &assertions);
    command
        .arg("--test")
        .arg("-C")
        .arg(format!("link-arg={}", object.display()));
```

- [ ] **Step 7: Run unit tests to verify they pass**

Run: `cargo test -p cargo-snacc --lib`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add apps/cargo-snacc/src/main.rs
git commit -m "feat(cargo-snacc): wire bridge assertions into check, build, run, and test"
```

*(Integration-level proof that this actually catches mismatches, missing items, and missing host lines is Task 7 — it needs the updated host template from Task 5 first.)*

---

### Task 5: Update the host template

**Files:**
- Modify: `apps/cargo-snacc/src/main.rs:116-201` (`init`), `:1107-1118` (`ensure_cargo_main_template`)
- Modify: `tests/fixtures/cargo-hosted/src/main.rs`
- Test: `apps/cargo-snacc/tests/cargo_hosted.rs:160-182` (`init_creates_the_complete_host_contract`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `const HOST_MAIN_TEMPLATE: &str` — the single source of truth `init` writes and `ensure_cargo_main_template` validates against, so the two can never drift.

- [ ] **Step 1: Write the failing test**

In `apps/cargo-snacc/tests/cargo_hosted.rs`, extend `init_creates_the_complete_host_contract` (currently lines 160-182) by adding two assertions after the existing `assert!(host.contains("fn snacc_entry_succeeds()"));`:

```rust
    assert!(host.contains("#[cfg(snacc_bridge_assertions)]"));
    assert!(host.contains("include!(env!(\"SNACC_BRIDGE_ASSERTIONS\"))"));
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cargo-snacc --test cargo_hosted init_creates_the_complete_host_contract`
Expected: FAIL, the freshly `init`'d host does not yet contain those lines.

- [ ] **Step 3: Add the template constants**

In `apps/cargo-snacc/src/main.rs`, add near the top of the file (after the `use` statements, before `struct CliError`):

```rust
const HOST_MAIN_TEMPLATE: &str = "mod interop;\n\n#[cfg(snacc_bridge_assertions)]\ninclude!(env!(\"SNACC_BRIDGE_ASSERTIONS\"));\n\nunsafe extern \"C\" {\n    fn snacc_main() -> i32;\n}\n\nfn main() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this host with the object defining this ABI.\n    let status = unsafe { snacc_main() };\n    std::process::exit(status);\n}\n\n#[test]\nfn snacc_entry_succeeds() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this harness with the object defining this ABI.\n    assert_eq!(unsafe { snacc_main() }, 0);\n}\n";

const HOST_MAIN_TEMPLATE_PRE_RFC_007: &str = "mod interop;\n\nunsafe extern \"C\" {\n    fn snacc_main() -> i32;\n}\n\nfn main() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this host with the object defining this ABI.\n    let status = unsafe { snacc_main() };\n    std::process::exit(status);\n}\n\n#[test]\nfn snacc_entry_succeeds() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this harness with the object defining this ABI.\n    assert_eq!(unsafe { snacc_main() }, 0);\n}\n";
```

- [ ] **Step 4: Use the constant in `init`**

In `init` (currently lines 189-193), replace:

```rust
    fs::write(
        &main_rs,
        "mod interop;\n\nunsafe extern \"C\" {\n    fn snacc_main() -> i32;\n}\n\nfn main() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this host with the object defining this ABI.\n    let status = unsafe { snacc_main() };\n    std::process::exit(status);\n}\n\n#[test]\nfn snacc_entry_succeeds() {\n    snacc_runtime::force_link();\n    // SAFETY: cargo-snacc links this harness with the object defining this ABI.\n    assert_eq!(unsafe { snacc_main() }, 0);\n}\n",
    )
    .map_err(io_error)?;
```

with:

```rust
    fs::write(&main_rs, HOST_MAIN_TEMPLATE).map_err(io_error)?;
```

- [ ] **Step 5: Accept both templates in `ensure_cargo_main_template`**

Replace `ensure_cargo_main_template` (currently lines 1107-1118):

```rust
fn ensure_cargo_main_template(path: &Path) -> Result<(), CliError> {
    if path.is_file() {
        let existing = fs::read_to_string(path).map_err(io_error)?;
        let trimmed = existing.trim();
        let known = [
            "fn main() {\n    println!(\"Hello, world!\");\n}",
            HOST_MAIN_TEMPLATE_PRE_RFC_007.trim(),
            HOST_MAIN_TEMPLATE.trim(),
        ];
        if !known.contains(&trimmed) {
            return Err(CliError(format!(
                "refusing to overwrite non-template Rust host '{}'",
                path.display()
            )));
        }
    }
    Ok(())
}
```

- [ ] **Step 6: Update the fixture host to the new template**

Replace the full contents of `tests/fixtures/cargo-hosted/src/main.rs` with:

```rust
mod interop;

#[cfg(snacc_bridge_assertions)]
include!(env!("SNACC_BRIDGE_ASSERTIONS"));

unsafe extern "C" {
    fn snacc_main() -> i32;
}

fn main() {
    snacc_runtime::force_link();
    // SAFETY: cargo-snacc links this host with the object defining this ABI.
    let status = unsafe { snacc_main() };
    std::process::exit(status);
}

#[test]
fn snacc_entry_succeeds() {
    snacc_runtime::force_link();
    // SAFETY: cargo-snacc links this harness with the object defining this ABI.
    assert_eq!(unsafe { snacc_main() }, 0);
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p cargo-snacc --test cargo_hosted init_creates_the_complete_host_contract`
Expected: PASS.

Run: `cargo build -p cargo-snacc` (sanity: the crate still compiles with the new constants in scope).

- [ ] **Step 8: Commit**

```bash
git add apps/cargo-snacc/src/main.rs tests/fixtures/cargo-hosted/src/main.rs
git commit -m "feat(cargo-snacc): add the bridge assertion include to the host template"
```

---

### Task 6: Populate the bridge item contract in `LANGUAGE.md`

**Files:**
- Modify: `LANGUAGE.md` (the "Rust bridge" section, currently lines 142-152)

**Interfaces:** none — documentation only.

- [ ] **Step 1: Add the bridge item contract**

In `LANGUAGE.md`, replace the final paragraph of the "Rust bridge" section (currently line 151, `Rust bridges must not unwind across the ABI boundary.`) so the section ends with:

```markdown
Rust bridges must not unwind across the ABI boundary.

A bridge function is a `pub` item of the host crate's `interop` module, reachable
at `crate::interop::<symbol>`. Its Rust item name is exactly the declared link
symbol. It carries `#[unsafe(no_mangle)]` and uses the `extern "C"` ABI, and it
does not carry `#[export_name]`. `cargo-snacc` verifies the item's Rust type
against the Snacc declaration's implied ABI signature before linking; it does not
verify that the item is exported under its symbol, which remains the final
linker's responsibility.
```

- [ ] **Step 2: Verify GRAMMAR.ebnf still matches**

This is prose about the host-side contract, not Snacc syntax — the EBNF in `LANGUAGE.md` and `GRAMMAR.ebnf` is unaffected. Confirm by diffing: `git diff LANGUAGE.md` should show only the "Rust bridge" section changing, with no line inside the ` ```ebnf ` fence touched.

- [ ] **Step 3: Commit**

```bash
git add LANGUAGE.md
git commit -m "docs: record the bridge item contract in LANGUAGE.md"
```

---

### Task 7: End-to-end verification tests

**Files:**
- Modify: `apps/cargo-snacc/tests/cargo_hosted.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-5 (a fully wired `cargo-snacc` binary).

Mismatch tests need a *copy* of the fixture package whose `interop.rs` or `main.rs` can be safely mutated without corrupting the shared fixture used by other tests (which may run concurrently under `cargo test`'s default parallelism) or leaving dirty state in the repo.

This task covers every RFC 007 Testing-section bullet except "plain `cargo check` and `cargo clippy` ... succeed exactly as they do today": Step 10 below covers `cargo check` only. Adding a `cargo clippy` invocation would make this suite depend on the `clippy` rustup component being installed wherever tests run, for a code path (the `cfg` gate) already proven by the `cargo check` case — skip it; add it later only if a real environment is confirmed to always have `clippy` available.

- [ ] **Step 1: Add fixture-copy and directory-parameterized run helpers**

In `apps/cargo-snacc/tests/cargo_hosted.rs`, add after `fn cargo_snacc(...)` (currently lines 9-16):

```rust
fn cargo_snacc_at(target: &Path, current_directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-snacc"))
        .args(args)
        .current_dir(current_directory)
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("failed to run cargo-snacc against a fixture copy")
}

fn copy_fixture_to(destination: &Path) {
    fs::create_dir_all(destination.join("src")).expect("failed to create fixture copy");
    let runtime_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/snacc-runtime");
    let manifest = fs::read_to_string(fixture().join("Cargo.toml")).unwrap();
    let manifest = manifest.replace(
        "snacc-runtime = { path = \"../../../crates/snacc-runtime\" }",
        &format!(
            "snacc-runtime = {{ path = {:?} }}",
            runtime_path.to_string_lossy()
        ),
    );
    fs::write(destination.join("Cargo.toml"), manifest).unwrap();
    for name in ["src/main.nrs", "src/main.rs", "src/interop.rs"] {
        fs::copy(fixture().join(name), destination.join(name))
            .unwrap_or_else(|error| panic!("failed to copy fixture file '{name}': {error}"));
    }
}
```

The copy rewrites `snacc-runtime`'s path dependency to an absolute path (derived from `CARGO_MANIFEST_DIR`) so the copied package works regardless of how deep the temp directory is nested — the original fixture's `../../../crates/snacc-runtime` is only correct at its own fixed location.

- [ ] **Step 2: Write the mismatch-across-every-command test**

Add:

```rust
#[test]
fn bridge_type_mismatch_is_caught_before_linking_by_every_command() {
    for command in ["check", "build", "run", "test"] {
        let workspace = tempfile::tempdir().expect("failed to create mismatch workspace");
        let package = workspace.path().join("package");
        copy_fixture_to(&package);
        let interop = package.join("src/interop.rs");
        let original = fs::read_to_string(&interop).unwrap();
        let mismatched = original.replace(
            "pub extern \"C\" fn snacc_user_itoa_len(value: i64) -> i64",
            "pub extern \"C\" fn snacc_user_itoa_len(value: f64) -> i64",
        );
        assert_ne!(
            original, mismatched,
            "fixture interop.rs no longer contains the expected itoa_len signature"
        );
        fs::write(&interop, mismatched).unwrap();

        let target = tempfile::tempdir().expect("failed to create fixture target directory");
        let output = cargo_snacc_at(target.path(), &package, &[command, "--offline"]);
        assert!(
            !output.status.success(),
            "'{command}' should reject a mismatched bridge signature:\n{}",
            combined(&output)
        );
        assert!(
            combined(&output).contains("mismatched types"),
            "'{command}' did not report a type mismatch:\n{}",
            combined(&output)
        );
    }
}
```

- [ ] **Step 3: Write the missing-item test**

Add:

```rust
#[test]
fn bridge_missing_interop_item_fails_with_a_name_resolution_error() {
    let workspace = tempfile::tempdir().expect("failed to create missing-item workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    let interop = package.join("src/interop.rs");
    let original = fs::read_to_string(&interop).unwrap();
    let renamed = original.replace("snacc_user_itoa_len", "snacc_user_itoa_len_renamed");
    assert_ne!(original, renamed);
    fs::write(&interop, renamed).unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["build", "--offline"]);
    assert!(
        !output.status.success(),
        "build should fail when the bridge item is missing:\n{}",
        combined(&output)
    );
    let rendered = combined(&output);
    assert!(
        rendered.contains("cannot find") || rendered.contains("unresolved"),
        "expected a name-resolution error, got:\n{rendered}"
    );
}
```

- [ ] **Step 4: Write the missing-host-include test**

Add:

```rust
#[test]
fn host_missing_assertion_include_reports_a_diagnostic() {
    let workspace = tempfile::tempdir().expect("failed to create host-missing-include workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    let host = package.join("src/main.rs");
    let original = fs::read_to_string(&host).unwrap();
    let without_include = original.replace(
        "#[cfg(snacc_bridge_assertions)]\ninclude!(env!(\"SNACC_BRIDGE_ASSERTIONS\"));\n\n",
        "",
    );
    assert_ne!(original, without_include);
    fs::write(&host, without_include).unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["build", "--offline"]);
    assert!(
        !output.status.success(),
        "build should refuse a host missing the assertion include:\n{}",
        combined(&output)
    );
    let rendered = combined(&output);
    assert!(
        rendered.contains("bridge assertion include"),
        "unexpected diagnostic:\n{rendered}"
    );
    assert!(
        rendered.contains("main.rs"),
        "diagnostic should name the host file:\n{rendered}"
    );
}
```

- [ ] **Step 5: Write the no-`no_mangle`-still-fails-at-link test**

Add:

```rust
#[test]
fn bridge_item_without_no_mangle_fails_at_link_not_at_the_assertion() {
    let workspace = tempfile::tempdir().expect("failed to create no-mangle workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    let interop = package.join("src/interop.rs");
    let original = fs::read_to_string(&interop).unwrap();
    let without_no_mangle = original.replacen(
        "#[unsafe(no_mangle)]\npub extern \"C\" fn snacc_user_itoa_len",
        "pub extern \"C\" fn snacc_user_itoa_len",
        1,
    );
    assert_ne!(original, without_no_mangle);
    fs::write(&interop, without_no_mangle).unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["build", "--offline"]);
    assert!(
        !output.status.success(),
        "build should still fail without #[unsafe(no_mangle)], but at link time:\n{}",
        combined(&output)
    );
    assert!(
        !combined(&output).contains("mismatched types"),
        "removing #[unsafe(no_mangle)] should not trip the type assertion:\n{}",
        combined(&output)
    );
}
```

- [ ] **Step 6: Write the result-type and arity mismatch tests**

The "every command" loop in Step 2 already proves the mechanism fires from `check`, `build`, `run`, and `test` alike for a parameter-type mismatch. A result-type mismatch and an arity mismatch are different *kinds* of disagreement (RFC 007's Testing section lists all three separately), but they exercise the same one-command-deep code path, so one command (`build`) is enough for each — repeating the four-command loop for every mutation kind would just multiply process-spawn cost without adding coverage. Add:

```rust
#[test]
fn bridge_result_type_mismatch_fails_before_linking() {
    let workspace = tempfile::tempdir().expect("failed to create workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    let interop = package.join("src/interop.rs");
    let original = fs::read_to_string(&interop).unwrap();
    let mismatched = original.replace(
        "pub extern \"C\" fn snacc_user_itoa_len(value: i64) -> i64",
        "pub extern \"C\" fn snacc_user_itoa_len(value: i64) -> f64",
    );
    assert_ne!(original, mismatched);
    fs::write(&interop, mismatched).unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["build", "--offline"]);
    assert!(
        !output.status.success(),
        "build should reject a mismatched bridge result type:\n{}",
        combined(&output)
    );
    assert!(
        combined(&output).contains("mismatched types"),
        "build did not report a result-type mismatch:\n{}",
        combined(&output)
    );
}

#[test]
fn bridge_arity_mismatch_fails_before_linking() {
    let workspace = tempfile::tempdir().expect("failed to create workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    let interop = package.join("src/interop.rs");
    let original = fs::read_to_string(&interop).unwrap();
    let mismatched = original.replace(
        "pub extern \"C\" fn snacc_user_itoa_len(value: i64) -> i64",
        "pub extern \"C\" fn snacc_user_itoa_len(value: i64, extra: i64) -> i64",
    );
    assert_ne!(original, mismatched);
    fs::write(&interop, mismatched).unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["build", "--offline"]);
    assert!(
        !output.status.success(),
        "build should reject an arity mismatch:\n{}",
        combined(&output)
    );
    assert!(
        combined(&output).contains("mismatched types"),
        "build did not report an arity mismatch:\n{}",
        combined(&output)
    );
}
```

- [ ] **Step 7: Write the no-interop-module test**

Distinct from the missing-*item* test in Step 3: here the `interop` module itself does not exist, so `crate::interop` fails to resolve at all. Add:

```rust
#[test]
fn host_without_an_interop_module_fails_with_a_module_resolution_error() {
    let workspace = tempfile::tempdir().expect("failed to create workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    let host = package.join("src/main.rs");
    let original = fs::read_to_string(&host).unwrap();
    let without_module = original.replace("mod interop;\n\n", "");
    assert_ne!(original, without_module);
    fs::write(&host, without_module).unwrap();
    fs::remove_file(package.join("src/interop.rs")).unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["build", "--offline"]);
    assert!(
        !output.status.success(),
        "build should fail when the host has no interop module:\n{}",
        combined(&output)
    );
    let rendered = combined(&output);
    assert!(
        rendered.contains("cannot find") || rendered.contains("unresolved"),
        "expected a module-resolution error, got:\n{rendered}"
    );
}
```

`without_module` keeps the `#[cfg(snacc_bridge_assertions)]`/`include!` lines intact (only the `mod interop;` line is stripped), so `validate_host_assertion_include` still passes and the failure genuinely comes from the generated assertion's `crate::interop::...` path not resolving, not from the earlier host-include check.

- [ ] **Step 8: Write the no-bridge-declarations test**

Add:

```rust
#[test]
fn program_without_bridge_declarations_builds_with_a_header_only_assertion_file() {
    let workspace = tempfile::tempdir().expect("failed to create workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    fs::write(package.join("src/main.nrs"), "print(0)\n").unwrap();
    fs::write(package.join("src/interop.rs"), "// no bridges\n").unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["build", "--offline"]);
    assert!(
        output.status.success(),
        "build should succeed for a program with no bridge declarations:\n{}",
        combined(&output)
    );
    let bridges_dir = target.path().join("snacc").join("bridges");
    let entries: Vec<_> = fs::read_dir(&bridges_dir)
        .expect("bridges directory was not created")
        .filter_map(|entry| entry.ok())
        .collect();
    assert_eq!(entries.len(), 1);
    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(content.starts_with("// Generated by cargo-snacc"));
    assert!(!content.contains("const _:"));
}
```

- [ ] **Step 9: Write the four-ABI-types round-trip test**

Add:

```rust
#[test]
fn each_bridge_type_round_trips_through_a_parameter_and_a_result() {
    let workspace = tempfile::tempdir().expect("failed to create workspace");
    let package = workspace.path().join("package");
    copy_fixture_to(&package);
    fs::write(
        package.join("src/main.nrs"),
        concat!(
            "extern rust \"snacc_user_echo_int\" fun echo_int(value: Int64): Int64\n",
            "extern rust \"snacc_user_echo_dec\" fun echo_dec(value: Dec64): Dec64\n",
            "extern rust \"snacc_user_echo_bool\" fun echo_bool(value: Bool): Bool\n",
            "extern rust \"snacc_user_echo_nil\" fun echo_nil(value: Nil): Nil\n",
            "print(echo_int(1));\n",
            "print(echo_dec(1.5));\n",
            "print(echo_bool(true));\n",
            "print(echo_nil(nil))\n",
        ),
    )
    .unwrap();
    fs::write(
        package.join("src/interop.rs"),
        concat!(
            "#[unsafe(no_mangle)]\n",
            "pub extern \"C\" fn snacc_user_echo_int(value: i64) -> i64 { value }\n\n",
            "#[unsafe(no_mangle)]\n",
            "pub extern \"C\" fn snacc_user_echo_dec(value: f64) -> f64 { value }\n\n",
            "#[unsafe(no_mangle)]\n",
            "pub extern \"C\" fn snacc_user_echo_bool(value: u8) -> u8 { value }\n\n",
            "#[unsafe(no_mangle)]\n",
            "pub extern \"C\" fn snacc_user_echo_nil(value: u8) -> u8 { value }\n",
        ),
    )
    .unwrap();

    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_at(target.path(), &package, &["run", "--offline"]);
    assert!(
        output.status.success(),
        "a bridge using every ABI type should build and run:\n{}",
        combined(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert!(stdout.lines().any(|line| line == "1"));
    assert!(stdout.lines().any(|line| line == "1.5"));
    assert!(stdout.lines().any(|line| line == "true"));
    assert!(stdout.lines().any(|line| line == "nil"));
}
```

`snacc-runtime`'s `snacc_print_bool`/`snacc_print_nil`/`snacc_print_f64` (`crates/snacc-runtime/src/lib.rs:4-21`) confirm the expected text: `println!("{}", value != 0)` for `Bool`, `println!("nil")` for `Nil`, and plain `{value}` Display for `Dec64`, so `1.5` round-trips without extra formatting.

- [ ] **Step 10: Write the plain-Cargo-unaffected test**

Add:

```rust
#[test]
fn plain_cargo_check_succeeds_without_cargo_snacc() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("check")
        .arg("--offline")
        .current_dir(fixture())
        .env("CARGO_TARGET_DIR", target.path())
        .output()
        .expect("failed to run plain cargo check");
    assert!(
        output.status.success(),
        "plain cargo check should succeed without cargo-snacc's cfg/env:\n{}",
        combined(&output)
    );
}
```

- [ ] **Step 11: Write the concurrent-generation test**

Add:

```rust
#[test]
fn concurrent_bridge_assertion_generation_does_not_corrupt_the_file() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let target_path = target.path().to_path_buf();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let target_path = target_path.clone();
            std::thread::spawn(move || cargo_snacc(&target_path, &["check", "--offline"]))
        })
        .collect();
    for handle in handles {
        let output = handle.join().expect("cargo-snacc thread panicked");
        assert!(
            output.status.success(),
            "concurrent check failed:\n{}",
            combined(&output)
        );
    }
    let bridges_dir = target_path.join("snacc").join("bridges");
    let entries: Vec<_> = fs::read_dir(&bridges_dir)
        .expect("bridges directory was not created")
        .filter_map(|entry| entry.ok())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one deterministic assertion file"
    );
    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(content.contains("crate::interop::snacc_user_itoa_len"));
    assert!(content.trim_end().ends_with(';'));
}
```

- [ ] **Step 12: Extend the primary end-to-end test to cover `clean`**

In `cargo_hosted_bridge_builds_runs_tests_and_validates_cache` (currently lines 104-158), after the final `tests` assertion block (after the closing brace that ends with `);` around line 157), add:

```rust

    let bridges_dir = target.path().join("snacc").join("bridges");
    assert!(bridges_dir.is_dir(), "bridge assertions were not generated");
    let cleaned = cargo_snacc(target.path(), &["clean", "--offline"]);
    assert!(cleaned.status.success(), "clean failed:\n{}", combined(&cleaned));
    assert!(
        !bridges_dir.exists(),
        "clean should remove generated bridge assertions"
    );
```

- [ ] **Step 13: Run the full integration suite**

Run: `cargo test -p cargo-snacc --test cargo_hosted`
Expected: PASS, all new and existing tests green. This suite now spawns considerably more `cargo`/`rustc`/LLVM subprocesses than before (roughly a dozen extra real builds) — expect it to take noticeably longer than pre-RFC-007; that is inherent to proving each failure mode compiles for real rather than a cost worth optimizing away.

- [ ] **Step 14: Commit**

```bash
git add apps/cargo-snacc/tests/cargo_hosted.rs
git commit -m "test(cargo-snacc): verify bridge signature mismatches fail before linking"
```

---

### Task 8: Workspace-wide verification and RFC close-out

**Files:**
- Modify: `TODO.md` (remove item 2)
- Modify: `docs/specs/007-bridge-signature-verification.md` → move to `docs/specs/archive/007-bridge-signature-verification.md`

**Interfaces:** none — final verification and housekeeping.

- [ ] **Step 1: Run the full workspace check**

Run: `cargo fmt --all -- --check`
Expected: no diff. If it reports one, run `cargo fmt --all` and re-stage.

Run: `cargo check --workspace --all-targets`
Expected: PASS, no warnings introduced.

- [ ] **Step 2: Run the full test suite**

Run: `cargo test --workspace`
Expected: PASS. This exercises `snacc-compiler`, `snacc-driver`, `snacc-runtime`, `cargo-snacc`, `snacc`, and `snacc-workbench` together — confirm nothing outside the touched crates regressed.

- [ ] **Step 3: Re-check every RFC 007 acceptance criterion**

Walk the 12 items in [docs/specs/007-bridge-signature-verification.md](../007-bridge-signature-verification.md)'s "Acceptance criteria" section one by one against the now-implemented code and tests from Tasks 1-7. Note any gap in the commit message for this task if one turns up (there should not be one if Tasks 1-7 were followed as written).

- [ ] **Step 4: Remove the TODO.md entry and close the spec**

In `TODO.md`, delete item 2 ("Make Rust bridge signatures verifiable rather than symbol-checked.") The numbering is one continuous sequence across all three `##` subsections (`Known issues` 1-5, `Missing verification` 6-11, `Housekeeping` 12-13), not per-section, so removing item 2 means renumbering every item from the old 3 through the old 13 down by one: old 3→2, 4→3, 5→4, 6→5, 7→6, 8→7, 9→8, 10→9, 11→10, 12→11, 13→12. Check no other document cross-references a TODO item by number before renumbering (a search for `TODO.md` mentions elsewhere in the repo turns up none as of this plan's writing).

In `docs/specs/007-bridge-signature-verification.md`, change line 3 from:

```
Status: Proposed
```

to:

```
Status: Closed
```

Then move the file:

```bash
git mv docs/specs/007-bridge-signature-verification.md docs/specs/archive/007-bridge-signature-verification.md
```

Also move this plan alongside it, since AGENTS.md treats an `-plan.md` as belonging with its specification once the specification is archived:

```bash
git mv docs/specs/007-bridge-signature-verification-plan.md docs/specs/archive/007-bridge-signature-verification-plan.md
```

- [ ] **Step 5: Commit**

```bash
git add TODO.md docs/specs/archive/007-bridge-signature-verification.md docs/specs/archive/007-bridge-signature-verification-plan.md
git commit -m "chore: close RFC 007 now bridge signatures are verified before linking"
```
