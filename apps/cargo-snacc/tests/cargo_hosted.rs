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
