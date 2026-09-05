//! Integration coverage for the runtime ABI in `src/lib.rs`.
//!
//! Every exported `snacc_print_*` symbol writes to process stdout via
//! `println!`, and stable Rust has no supported way to capture that output
//! from within the same test process. Each check here instead compiles a
//! tiny throwaway Rust program with `rustc` -- the same tool the direct
//! compiler and Cargo-hosted workflows already require, see
//! `crates/snacc-driver/src/lib.rs` -- and inspects its stdout via
//! `Command`, the same approach `apps/snacc/tests/conformance.rs` uses for
//! full programs.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};
use tempfile::TempDir;

#[test]
fn runtime_implements_abi_version_twelve() {
    assert_eq!(snacc_runtime::ABI_VERSION, 12);
}

#[test]
fn integer_maps_and_sets_support_mutation_and_iteration() {
    let mut map = snacc_runtime::SnaccMap {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };
    assert_eq!(snacc_runtime::snacc_map_i64_i64_insert(&mut map, 7, 70), 1);
    assert_eq!(snacc_runtime::snacc_map_i64_i64_insert(&mut map, 9, 90), 1);
    assert_eq!(snacc_runtime::snacc_map_i64_i64_insert(&mut map, 7, 71), 0);
    assert_eq!(map.len, 2);
    assert_eq!(snacc_runtime::snacc_map_i64_i64_key_at(&map, 0), 7);
    assert_eq!(snacc_runtime::snacc_map_i64_i64_value_at(&map, 0), 71);
    assert_eq!(snacc_runtime::snacc_map_i64_i64_index(&map, 9), 90);
    snacc_runtime::snacc_map_i64_i64_reserve(&mut map, 8);
    assert!(map.cap >= 8);
    assert_eq!(snacc_runtime::snacc_map_i64_i64_delete(&mut map, 9), 1);
    snacc_runtime::snacc_map_i64_i64_drop(&map);

    let mut set = snacc_runtime::SnaccSet {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };
    assert_eq!(snacc_runtime::snacc_set_i64_insert(&mut set, 3), 1);
    assert_eq!(snacc_runtime::snacc_set_i64_insert(&mut set, 3), 0);
    snacc_runtime::snacc_set_i64_reserve(&mut set, 8);
    assert!(set.cap >= 8);
    assert_eq!(snacc_runtime::snacc_set_i64_at(&set, 0), 3);
    assert_eq!(snacc_runtime::snacc_set_i64_contains(&set, 3), 1);
    assert_eq!(snacc_runtime::snacc_set_i64_delete(&mut set, 3), 1);
    snacc_runtime::snacc_set_i64_drop(&set);
}

#[test]
fn string_collection_iteration_returns_non_owning_descriptors() {
    let bytes = b"Alice";
    let mut map = snacc_runtime::SnaccMap {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };
    let key = snacc_runtime::snacc_string_new(bytes.as_ptr(), bytes.len());
    assert_eq!(
        snacc_runtime::snacc_map_string_i64_insert(&mut map, &key, 10),
        1
    );
    let borrowed = snacc_runtime::snacc_map_string_i64_key_at(&map, 0);
    assert_eq!(borrowed.cap, 0);
    let borrowed_bytes = unsafe { std::slice::from_raw_parts(borrowed.ptr, borrowed.len) };
    assert_eq!(borrowed_bytes, bytes);
    snacc_runtime::snacc_map_string_i64_drop(&map);
}

// ---------------------------------------------------------------------
// snacc_alloc / snacc_dealloc (Specification 016 section 8.2)
// ---------------------------------------------------------------------

#[test]
fn snacc_alloc_returns_aligned_writable_storage_that_dealloc_accepts_back() {
    let ptr = snacc_runtime::snacc_alloc(64, 16);
    assert!(!ptr.is_null(), "snacc_alloc must never return null");
    assert_eq!(
        ptr as usize % 16,
        0,
        "snacc_alloc must honor the requested alignment"
    );
    // Safety: `ptr` was just returned by `snacc_alloc(64, 16)`, so it is
    // valid, writable storage for at least 64 bytes.
    unsafe {
        ptr.cast::<u64>().write(0xDEAD_BEEF_u64);
        assert_eq!(ptr.cast::<u64>().read(), 0xDEAD_BEEF_u64);
    }
    snacc_runtime::snacc_dealloc(ptr, 64, 16);
}

/// Specification 016 section 8.2's closing paragraph: boxing a zero-sized
/// value is valid, and the runtime may use a fixed non-null sentinel rather
/// than a real allocation for it.
#[test]
fn snacc_alloc_of_a_zero_sized_pointee_is_non_null_and_never_reaches_the_allocator() {
    let first = snacc_runtime::snacc_alloc(0, 8);
    let second = snacc_runtime::snacc_alloc(0, 8);
    assert!(
        !first.is_null(),
        "a zero-sized allocation must still be non-null"
    );
    assert_eq!(
        first as usize % 8,
        0,
        "the sentinel must honor alignment too"
    );
    assert_eq!(
        first, second,
        "every zero-sized allocation of the same alignment shares the fixed sentinel"
    );
    // Must not abort/panic/double-free: the zero-size path never touches the
    // global allocator, so passing it back is always safe.
    snacc_runtime::snacc_dealloc(first, 0, 8);
    snacc_runtime::snacc_dealloc(second, 0, 8);
}

#[test]
fn scalar_list_push_grows_and_clear_resets_length() {
    let mut list = snacc_runtime::SnaccList {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };
    for value in 0..10 {
        snacc_runtime::snacc_list_push_i64(&mut list, value);
    }
    assert_eq!(list.len, 10);
    assert!(list.cap >= 10);
    // Safety: the runtime just initialized `list.len` contiguous i64 values.
    let values = unsafe { std::slice::from_raw_parts(list.ptr.cast::<i64>(), list.len) };
    assert_eq!(values, &(0..10).collect::<Vec<_>>());
    snacc_runtime::snacc_list_clear(&mut list);
    assert_eq!(list.len, 0);
    snacc_runtime::snacc_dealloc(
        list.ptr,
        list.cap * std::mem::size_of::<i64>(),
        std::mem::align_of::<i64>(),
    );
}

#[test]
fn scalar_list_insert_remove_pop_and_reserve_preserve_order() {
    let mut list = snacc_runtime::SnaccList {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };
    snacc_runtime::snacc_list_push_i64(&mut list, 10);
    snacc_runtime::snacc_list_push_i64(&mut list, 30);
    snacc_runtime::snacc_list_insert_i64(&mut list, 1, 20);
    assert_eq!(snacc_runtime::snacc_list_remove_i64(&mut list, 0), 10);
    assert_eq!(snacc_runtime::snacc_list_pop_i64(&mut list), 30);
    assert_eq!(list.len, 1);
    // A reserve request is a lower bound, not an exact capacity request.
    snacc_runtime::snacc_list_reserve(
        &mut list,
        10,
        std::mem::size_of::<i64>(),
        std::mem::align_of::<i64>(),
    );
    assert!(list.cap >= 10);
    // Safety: one initialized i64 remains at the start of the allocation.
    let values = unsafe { std::slice::from_raw_parts(list.ptr.cast::<i64>(), list.len) };
    assert_eq!(values, &[20]);
    snacc_runtime::snacc_dealloc(
        list.ptr,
        list.cap * std::mem::size_of::<i64>(),
        std::mem::align_of::<i64>(),
    );
}

fn run(command: &mut Command, what: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to run {what}: {error}"));
    assert!(
        output.status.success(),
        "{what} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

// ---------------------------------------------------------------------
// snacc_print_* value formatting
// ---------------------------------------------------------------------

/// The exact runtime source under test, embedded the same way
/// `crates/snacc-driver` embeds it for the direct workflow.
const RUNTIME_SOURCE: &str = include_str!("../src/lib.rs");

/// Appended to `RUNTIME_SOURCE` so one compiled probe binary can exercise
/// any of the eight print symbols by argv, e.g. `probe f64 1.5` or `probe u8 7`.
/// Calling a `pub extern "C" fn` directly by name (not through an FFI
/// declaration) is ordinary, safe Rust -- no `unsafe` required.
const PROBE_MAIN: &str = r#"

fn main() {
    let mut args = std::env::args().skip(1);
    let selector = args.next().expect("missing selector argument");
    match selector.as_str() {
        "f64" => snacc_print_f64(args.next().expect("missing value").parse().expect("invalid f64")),
        "i64" => snacc_print_i64(args.next().expect("missing value").parse().expect("invalid i64")),
        "bool" => snacc_print_bool(args.next().expect("missing value").parse().expect("invalid u8")),
        "u8" => snacc_print_u8(args.next().expect("missing value").parse().expect("invalid u8")),
        "u16" => snacc_print_u16(args.next().expect("missing value").parse().expect("invalid u16")),
        "u32" => snacc_print_u32(args.next().expect("missing value").parse().expect("invalid u32")),
        "u64" => snacc_print_u64(args.next().expect("missing value").parse().expect("invalid u64")),
        "f32" => snacc_print_f32(args.next().expect("missing value").parse().expect("invalid f32")),
        other => panic!("unknown selector: {other}"),
    }
}
"#;

/// Compiles the shared print-symbol probe once and reuses it for every
/// `#[test]` below (tests in one binary run concurrently by default).
fn probe_path() -> &'static Path {
    static PROBE: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
    let (_dir, path) = PROBE.get_or_init(|| {
        let dir = tempfile::Builder::new()
            .prefix("snacc-runtime-probe-")
            .tempdir()
            .expect("failed to create temp dir for the print-symbol probe");
        let source_path = dir.path().join("probe.rs");
        fs::write(&source_path, format!("{RUNTIME_SOURCE}{PROBE_MAIN}"))
            .expect("failed to write probe source");
        let exe_path = dir.path().join(format!("probe{}", env::consts::EXE_SUFFIX));
        run(
            Command::new("rustc")
                .arg("--edition=2024")
                .arg(&source_path)
                .arg("-o")
                .arg(&exe_path),
            "compiling the print-symbol probe",
        );
        (dir, exe_path)
    });
    path
}

fn probe_stdout(args: &[&str]) -> String {
    let output = run(
        Command::new(probe_path()).args(args),
        &format!("running the print-symbol probe with {args:?}"),
    );
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

#[test]
fn snacc_print_f64_uses_default_f64_display() {
    assert_eq!(probe_stdout(&["f64", "1.5"]), "1.5\n");
    assert_eq!(probe_stdout(&["f64", "0.0"]), "0\n");
    assert_eq!(probe_stdout(&["f64", "-3.25"]), "-3.25\n");
}

#[test]
fn snacc_print_i64_uses_default_i64_display() {
    assert_eq!(probe_stdout(&["i64", "0"]), "0\n");
    assert_eq!(probe_stdout(&["i64", "42"]), "42\n");
    assert_eq!(
        probe_stdout(&["i64", &i64::MIN.to_string()]),
        "-9223372036854775808\n"
    );
}

#[test]
fn snacc_print_bool_treats_any_nonzero_byte_as_true() {
    assert_eq!(probe_stdout(&["bool", "0"]), "false\n");
    assert_eq!(probe_stdout(&["bool", "1"]), "true\n");
    // Documents today's behavior (`value != 0`): any nonzero byte is
    // truthy, not just 1. Not asserting this should stay this way forever,
    // just confirming what the current code actually does.
    assert_eq!(probe_stdout(&["bool", "2"]), "true\n");
}

#[test]
fn snacc_print_u8_uses_default_u8_display() {
    assert_eq!(probe_stdout(&["u8", "0"]), "0\n");
    assert_eq!(probe_stdout(&["u8", "255"]), "255\n");
}

#[test]
fn snacc_print_u16_uses_default_u16_display() {
    assert_eq!(probe_stdout(&["u16", "0"]), "0\n");
    assert_eq!(probe_stdout(&["u16", "65535"]), "65535\n");
}

#[test]
fn snacc_print_u32_uses_default_u32_display() {
    assert_eq!(probe_stdout(&["u32", "0"]), "0\n");
    assert_eq!(probe_stdout(&["u32", "4294967295"]), "4294967295\n");
}

#[test]
fn snacc_print_u64_uses_default_u64_display() {
    assert_eq!(probe_stdout(&["u64", "0"]), "0\n");
    assert_eq!(
        probe_stdout(&["u64", &u64::MAX.to_string()]),
        "18446744073709551615\n"
    );
}

#[test]
fn snacc_print_f32_uses_default_f32_display() {
    assert_eq!(probe_stdout(&["f32", "1.5"]), "1.5\n");
    assert_eq!(probe_stdout(&["f32", "0.0"]), "0\n");
    assert_eq!(probe_stdout(&["f32", "-3.25"]), "-3.25\n");
}

// ---------------------------------------------------------------------
// force_link retention contract
// ---------------------------------------------------------------------

/// Stands in for the object file the LLVM backend emits: undefined
/// references to all eight print symbols plus the allocator pair, called
/// from one exported entry point that a host can invoke without knowing
/// anything else about it.
const FAKE_OBJECT_SOURCE: &str = r#"
unsafe extern "C" {
    fn snacc_print_f64(value: f64);
    fn snacc_print_i64(value: i64);
    fn snacc_print_bool(value: u8);
    fn snacc_print_u8(value: u8);
    fn snacc_print_u16(value: u16);
    fn snacc_print_u32(value: u32);
    fn snacc_print_u64(value: u64);
    fn snacc_print_f32(value: f32);
    fn snacc_alloc(size: usize, align: usize) -> *mut u8;
    fn snacc_dealloc(ptr: *mut u8, size: usize, align: usize);
}

#[unsafe(no_mangle)]
pub extern "C" fn probe_entry() {
    unsafe {
        snacc_print_f64(1.5);
        snacc_print_i64(-42);
        snacc_print_bool(1);
        snacc_print_u8(255);
        snacc_print_u16(65535);
        snacc_print_u32(4294967295);
        snacc_print_u64(18446744073709551615);
        snacc_print_f32(2.5);
        let ptr = snacc_alloc(8, 8);
        (ptr as *mut i64).write(99);
        snacc_print_i64((ptr as *mut i64).read());
        snacc_dealloc(ptr, 8, 8);
    }
}
"#;

/// A host that depends on `snacc-runtime` as an ordinary, separately
/// compiled crate and calls nothing from it but `force_link()` -- exactly
/// the shape of the generated hosts in `crates/snacc-driver/src/lib.rs` and
/// `apps/cargo-snacc`.
const FORCE_LINK_HOST_SOURCE: &str = r#"
unsafe extern "C" {
    fn probe_entry();
}

fn main() {
    snacc_runtime::force_link();
    unsafe { probe_entry(); }
}
"#;

/// Proves the actual property `force_link` exists to guarantee: a host that
/// touches `snacc-runtime` only by calling `force_link()` still lets an
/// externally linked native object resolve, call, and correctly observe all
/// eight `snacc_print_*` symbols plus the RFC 016 allocator pair across a
/// real link -- not just that `force_link()` runs without panicking.
///
/// This builds `snacc-runtime` as a standalone `.rlib` (the same separately
/// compiled shape Cargo-hosted apps depend on, not source-embedded), links
/// it with the fake object above via `--extern` and `-C link-arg=...` (the
/// same mechanism `crates/snacc-driver::build` and `apps/cargo-snacc` use),
/// and asserts both that the link succeeds and that the resulting binary
/// prints exactly what the print symbols, plus one allocate/write/read/
/// deallocate round trip, should produce. A successful link plus correct
/// output is strictly more informative than finding the symbol names in a
/// `dumpbin`/`nm` symbol table: it also proves the symbols are correctly
/// defined, exported, and callable across the crate boundary, not merely
/// present as leftover names.
///
/// Caveat found while building this test: on the current toolchain
/// (rustc 1.98, x86_64-pc-windows-msvc, linking via MSVC `link.exe`), the
/// same link *also* succeeds if `force_link()` is not called at all --
/// MSVC's linker resolves the fake object's undefined symbols against the
/// rlib regardless of link-line order. So this test demonstrates that
/// calling `force_link()` is sufficient for retention; it does not (and, on
/// this toolchain, cannot) demonstrate that `force_link()` is necessary.
/// The historical risk it guards against -- a single-pass linker extracting
/// archive members strictly in link-line order, before a later object's
/// undefined references are known -- is a property of some linkers (e.g.
/// classic GNU `ld`), not of MSVC's.
#[test]
fn force_link_retains_all_print_and_allocator_symbols_through_a_real_link() {
    let dir = tempfile::Builder::new()
        .prefix("snacc-runtime-force-link-")
        .tempdir()
        .expect("failed to create temp dir for the force_link retention test");

    // Compile snacc-runtime as a standalone rlib: an ordinary, separately
    // compiled crate, not source-embedded.
    let runtime_source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let rlib_path = dir.path().join("libsnacc_runtime.rlib");
    run(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg("--crate-type=lib")
            .arg(&runtime_source_path)
            .arg("-o")
            .arg(&rlib_path),
        "compiling snacc-runtime as a standalone rlib",
    );

    let fake_object_source_path = dir.path().join("fake_object.rs");
    fs::write(&fake_object_source_path, FAKE_OBJECT_SOURCE)
        .expect("failed to write fake object source");
    let fake_object_path = dir.path().join("fake_object.o");
    run(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg("--crate-type=lib")
            .arg("--emit=obj")
            .arg(&fake_object_source_path)
            .arg("-o")
            .arg(&fake_object_path),
        "compiling the fake externally linked object",
    );

    let host_source_path = dir.path().join("host.rs");
    fs::write(&host_source_path, FORCE_LINK_HOST_SOURCE).expect("failed to write host source");
    let executable_path = dir.path().join(format!("host{}", env::consts::EXE_SUFFIX));
    run(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg(&host_source_path)
            .arg("--extern")
            .arg(format!("snacc_runtime={}", rlib_path.display()))
            .arg("-C")
            .arg(format!("link-arg={}", fake_object_path.display()))
            .arg("-o")
            .arg(&executable_path),
        "linking the force_link-only host against the fake object",
    );

    let output = run(
        &mut Command::new(&executable_path),
        "running the force_link-only host",
    );
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert_eq!(
        stdout,
        "1.5\n-42\ntrue\n255\n65535\n4294967295\n18446744073709551615\n2.5\n99\n"
    );
}
