use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/cargo-hosted")
}

fn cargo_snacc(target: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-snacc"))
        .args(args)
        .current_dir(fixture())
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("failed to run cargo-snacc fixture")
}

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

fn cargo_external_subcommand(target: &Path, args: &[&str]) -> Output {
    let executable = Path::new(env!("CARGO_BIN_EXE_cargo-snacc"));
    let mut path = vec![
        executable
            .parent()
            .expect("cargo-snacc executable has no parent")
            .to_path_buf(),
    ];
    #[cfg(windows)]
    {
        path.push(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../vendor/clang+llvm-22.1.8-x86_64-pc-windows-msvc/bin"),
        );
    }
    path.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .arg("snacc")
        .args(args)
        .current_dir(fixture())
        .env("CARGO_TARGET_DIR", target)
        .env(
            "PATH",
            std::env::join_paths(path).expect("failed to construct Cargo subcommand PATH"),
        );
    #[cfg(windows)]
    command.env(
        "LLVM_SYS_221_PREFIX",
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/clang+llvm-22.1.8-x86_64-pc-windows-msvc"),
    );
    command
        .output()
        .expect("failed to invoke cargo snacc through Cargo")
}

#[cfg(windows)]
fn cargo_snacc_with_vendored_llvm_at(
    target: &Path,
    current_directory: &Path,
    args: &[&str],
) -> Output {
    let llvm_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/clang+llvm-22.1.8-x86_64-pc-windows-msvc");
    let mut path = vec![llvm_root.join("bin")];
    path.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    Command::new(env!("CARGO_BIN_EXE_cargo-snacc"))
        .args(args)
        .current_dir(current_directory)
        .env("CARGO_TARGET_DIR", target)
        .env("LLVM_SYS_221_PREFIX", &llvm_root)
        .env(
            "PATH",
            std::env::join_paths(path).expect("failed to construct fixture PATH"),
        )
        .output()
        .expect("failed to run cargo-snacc fixture with vendored LLVM")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn find_manifest(directory: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(directory).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_manifest(&path) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|name| name == "manifest.json") {
            return Some(path);
        }
    }
    None
}

#[test]
fn cargo_hosted_bridge_builds_runs_tests_and_validates_cache() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");

    let run = cargo_snacc(target.path(), &["run", "--offline", "--", "one", "two"]);
    assert!(run.status.success(), "run failed:\n{}", combined(&run));
    let stdout = String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n");
    assert!(
        stdout.lines().any(|line| line == "5") && stdout.lines().any(|line| line == "2"),
        "bridge output or program arguments were not preserved:\n{stdout}"
    );

    let reused = cargo_snacc(target.path(), &["build", "--offline", "--verbose"]);
    assert!(
        reused.status.success(),
        "cached build failed:\n{}",
        combined(&reused)
    );
    assert!(
        String::from_utf8_lossy(&reused.stdout).contains("Snacc object reused"),
        "second build did not reuse the object:\n{}",
        combined(&reused)
    );

    let manifest = find_manifest(&target.path().join("snacc"))
        .expect("content-addressed cache manifest was not written");
    fs::write(&manifest, b"not valid JSON").expect("failed to corrupt cache manifest");
    let rebuilt = cargo_snacc(target.path(), &["build", "--offline", "--verbose"]);
    assert!(
        rebuilt.status.success(),
        "cache rebuild failed:\n{}",
        combined(&rebuilt)
    );
    assert!(
        String::from_utf8_lossy(&rebuilt.stdout).contains("Snacc object rebuilt"),
        "invalid manifest was reused:\n{}",
        combined(&rebuilt)
    );

    let tests = cargo_snacc(
        target.path(),
        &[
            "test",
            "--offline",
            "snacc_entry_succeeds",
            "--",
            "--nocapture",
        ],
    );
    assert!(
        tests.status.success(),
        "cargo snacc test failed:\n{}",
        combined(&tests)
    );

    let bridges_dir = target.path().join("snacc").join("bridges");
    assert!(bridges_dir.is_dir(), "bridge assertions were not generated");
    let cleaned = cargo_snacc(target.path(), &["clean", "--offline"]);
    assert!(
        cleaned.status.success(),
        "clean failed:\n{}",
        combined(&cleaned)
    );
    assert!(
        !bridges_dir.exists(),
        "clean should remove generated bridge assertions"
    );
}

#[test]
fn init_creates_the_complete_host_contract() {
    let directory = tempfile::tempdir().expect("failed to create init directory");
    let package = directory.path().join("hello");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-snacc"))
        .arg("init")
        .arg(&package)
        .output()
        .expect("failed to run cargo snacc init");
    assert!(
        output.status.success(),
        "init failed:\n{}",
        combined(&output)
    );

    let manifest = fs::read_to_string(package.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("schema-version = 1"));
    assert!(manifest.contains("snacc-runtime = \"0.1\""));
    assert!(manifest.contains("check-cfg"));
    assert!(package.join("src/main.nrs").is_file());
    assert!(package.join("src/interop.rs").is_file());
    let host = fs::read_to_string(package.join("src/main.rs")).unwrap();
    assert!(host.contains("fn snacc_entry_succeeds()"));
    assert!(host.contains("#[cfg(snacc_bridge_assertions)]"));
    assert!(host.contains("include!(env!(\"SNACC_BRIDGE_ASSERTIONS\"))"));
}

#[test]
fn cargo_discovers_and_invokes_the_external_subcommand() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_external_subcommand(target.path(), &["check", "--offline"]);
    assert!(
        output.status.success(),
        "cargo snacc check failed:\n{}",
        combined(&output)
    );
}

#[cfg(windows)]
#[test]
fn doctor_validates_the_selected_vendored_runtime_and_linker() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc_with_vendored_llvm_at(target.path(), &fixture(), &["doctor"]);
    assert!(
        output.status.success(),
        "doctor failed:\n{}",
        combined(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("LLVM runtime:"));
    assert!(stdout.contains("doctor: all checks passed"));
}

#[cfg(windows)]
#[test]
fn doctor_skips_package_checks_outside_a_snacc_application() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = cargo_snacc_with_vendored_llvm_at(target.path(), repository, &["doctor"]);
    assert!(
        output.status.success(),
        "doctor failed in compiler repository:\n{}",
        combined(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("package checks skipped"));
    assert!(stdout.contains("doctor: all checks passed"));
}

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
        rendered.contains("E0425"),
        "expected a name-resolution error (E0425), got:\n{rendered}"
    );
}

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
        rendered.contains("E0433"),
        "expected a module-resolution error (E0433), got:\n{rendered}"
    );
}

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
    assert!(
        !combined(&output).contains("unexpected `cfg` condition name"),
        "plain cargo check should not warn about the assertion cfg:\n{}",
        combined(&output)
    );
}

#[test]
fn check_rejects_release_and_profile_flags() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let output = cargo_snacc(target.path(), &["check", "--offline", "--release"]);
    assert!(
        !output.status.success(),
        "check --release should be rejected:\n{}",
        combined(&output)
    );
    assert!(
        combined(&output).contains("check does not support --release or --profile"),
        "expected the --release/--profile rejection message:\n{}",
        combined(&output)
    );
}

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
    // Each rendered line ends in a `// snacc: name (entry:line:column)` trailing
    // comment (see `render_bridge_assertions`), so a non-corrupted, non-truncated
    // publish ends with the comment's closing paren, not the statement's `;`.
    assert!(content.trim_end().ends_with(')'));
}

#[test]
fn concurrent_object_cache_publication_does_not_corrupt_the_cache() {
    let target = tempfile::tempdir().expect("failed to create fixture target directory");
    let target_path = target.path().to_path_buf();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let target_path = target_path.clone();
            std::thread::spawn(move || cargo_snacc(&target_path, &["build", "--offline"]))
        })
        .collect();
    for handle in handles {
        let output = handle.join().expect("cargo-snacc thread panicked");
        assert!(
            output.status.success(),
            "concurrent build failed:\n{}",
            combined(&output)
        );
    }

    let manifest_path = find_manifest(&target_path.join("snacc"))
        .expect("content-addressed cache manifest was not written");
    let cache_dir = manifest_path.parent().unwrap();
    let object_path = cache_dir.join(if cfg!(windows) { "app.obj" } else { "app.o" });
    let object_bytes = fs::read(&object_path).expect("cached object was not published");
    assert!(
        !object_bytes.is_empty(),
        "cached object should not be empty"
    );

    let manifest_bytes = fs::read(&manifest_path).expect("cache manifest was not published");
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect("cache manifest should be valid JSON");
    let mut hash = sha2::Sha256::new();
    hash.update(&object_bytes);
    let expected_sha256 = format!("{:x}", hash.finalize());
    assert_eq!(
        manifest["object_sha256"].as_str(),
        Some(expected_sha256.as_str()),
        "manifest's recorded object hash should match the object bytes actually on disk \
         (a mismatch means one process's manifest published over another's object, or vice versa)"
    );

    let leftover_temp_files: Vec<_> = fs::read_dir(cache_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(
        leftover_temp_files.is_empty(),
        "no unique temp files should remain in the cache directory after publish: {:?}",
        leftover_temp_files
    );
}
