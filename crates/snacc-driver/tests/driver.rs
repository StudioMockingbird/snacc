//! Focused tests for `snacc-driver`'s public `build`/`build_to` API.
//!
//! `build()` resolves `rustc` via the process-wide `PATH` environment
//! variable. The missing-rustc test below has to hide `rustc` from `PATH`
//! for the span of one `build()` call, and `std::env::set_var` mutates the
//! whole process, not just the calling thread. Rust's test harness runs the
//! `#[test]` functions in one binary concurrently across threads, so without
//! coordination that mutation could race a different test's `build()` call
//! that expects a real, unmodified `PATH`.
//!
//! Every test in this file that calls `build()` (needs a real `PATH`) or
//! mutates `PATH` (the missing-rustc test) takes `TEST_LOCK` first, so the
//! two kinds of test can never interleave on separate threads.

use snacc_compiler::DiagnosticPhase;
use snacc_driver::{DriverError, build};
use std::{
    env,
    ffi::OsString,
    path::Path,
    process::{Command, Output},
    sync::{Mutex, MutexGuard},
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock_guard() -> MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const VALID_SOURCE: &str = "print(1)\n";

fn run(path: &Path) -> Output {
    Command::new(path)
        .output()
        .expect("failed to run built executable")
}

#[test]
fn compile_error_yields_structured_diagnostics() {
    let _guard = lock_guard();
    let source = "let x: Int64 = true\nprint(x)\n";

    let error = match build(source) {
        Ok(_) => panic!("ill-typed source must not build"),
        Err(error) => error,
    };
    let DriverError::Compile(diagnostics) = error else {
        panic!("expected DriverError::Compile, got {error:?}");
    };

    let diagnostic = diagnostics
        .items
        .iter()
        .find(|item| item.phase == DiagnosticPhase::TypeCheck)
        .unwrap_or_else(|| panic!("expected a TypeCheck diagnostic, got {diagnostics:?}"));
    assert!(
        diagnostic.message.contains("Int64") && diagnostic.message.contains("Bool"),
        "unexpected diagnostic message: {}",
        diagnostic.message
    );
    assert!(
        diagnostic.span.is_some(),
        "diagnostic should carry a source span"
    );
}

#[test]
fn build_creates_a_fresh_directory_each_call() {
    let _guard = lock_guard();

    let first = build(VALID_SOURCE).expect("first build failed");
    let second = build(VALID_SOURCE).expect("second build failed");

    assert_ne!(first.directory(), second.directory());
    assert!(first.directory().is_dir());
    assert!(second.directory().is_dir());
    assert!(first.path().is_file());
    assert!(second.path().is_file());
}

#[test]
fn changed_source_is_not_served_from_a_stale_executable() {
    let _guard = lock_guard();

    let first = build("print(1)\n").expect("build A failed");
    let second = build("print(2)\n").expect("build B failed");

    let first_output = run(first.path());
    let second_output = run(second.path());
    assert!(first_output.status.success());
    assert!(second_output.status.success());

    let first_stdout = String::from_utf8_lossy(&first_output.stdout).replace("\r\n", "\n");
    let second_stdout = String::from_utf8_lossy(&second_output.stdout).replace("\r\n", "\n");
    assert_eq!(first_stdout, "1\n");
    assert_eq!(second_stdout, "2\n");
}

#[test]
fn runtime_print_output_goes_to_stdout_not_stderr() {
    let _guard = lock_guard();

    let executable = build("print(42)\n").expect("build failed");
    let output = run(executable.path());

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stdout, "42\n");
    assert!(stderr.is_empty(), "expected empty stderr, got: {stderr:?}");
}

/// Restores the process `PATH` on drop, including on unwind from a failed
/// assertion, so a failing test never leaves `PATH` corrupted for the rest
/// of the process.
struct RestorePath(Option<OsString>);

impl Drop for RestorePath {
    fn drop(&mut self) {
        // SAFETY: only ever constructed while `TEST_LOCK` is held (see
        // `missing_rustc_yields_a_tool_error`), so no other thread in this
        // process reads or writes `PATH` while this runs.
        unsafe {
            match self.0.take() {
                Some(path) => env::set_var("PATH", path),
                None => env::remove_var("PATH"),
            }
        }
    }
}

#[test]
fn missing_rustc_yields_a_tool_error() {
    let _guard = lock_guard();
    // Dropped after the assertions below (even on panic), restoring the
    // real PATH before `_guard` is released.
    let _restore = RestorePath(env::var_os("PATH"));
    // SAFETY: guarded by `TEST_LOCK` above; no other thread observes PATH
    // while it is empty.
    unsafe { env::set_var("PATH", "") };

    let error = match build(VALID_SOURCE) {
        Ok(_) => panic!("build must fail when rustc is not on PATH"),
        Err(error) => error,
    };
    let DriverError::Tool(message) = error else {
        panic!("expected DriverError::Tool, got {error:?}");
    };
    assert_eq!(
        message,
        "rustc was not found on PATH; install a Rust toolchain to link Snacc programs"
    );
}
