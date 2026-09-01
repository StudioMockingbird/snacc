use std::{fs, process::Command};

fn executable_path(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    dir.path()
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

#[test]
fn cli_compiles_and_runs_one_program() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("cli_success.nrs");
    let output_path = executable_path(&dir, "cli_success");
    fs::write(&source_path, "print(7)").unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_snacc"))
        .arg(&source_path)
        .arg("-o")
        .arg(&output_path)
        .status()
        .expect("failed to invoke snacc");
    assert!(status.success());

    let output = Command::new(&output_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        "7\n"
    );
}

#[test]
fn cli_reports_a_source_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("cli_error.nrs");
    fs::write(&source_path, "fun wrong(): Bool do 1 end").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_snacc"))
        .arg(&source_path)
        .output()
        .expect("failed to invoke snacc");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected 'Bool', found 'Int64'"));
}

#[test]
fn cli_rejects_unknown_options_at_the_process_boundary() {
    let output = Command::new(env!("CARGO_BIN_EXE_snacc"))
        .arg("--unknown")
        .arg("program.nrs")
        .output()
        .expect("failed to invoke snacc");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown option '--unknown'"));
}
