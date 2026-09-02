use snacc_driver::build;
use std::{fs, path::Path, process::Command};

fn run_llvm(case_names: &str, source: &str, expected: &str) {
    let executable = build(source)
        .unwrap_or_else(|error| panic!("LLVM build failed for run corpus [{case_names}]: {error}"));
    let output = Command::new(executable.path())
        .output()
        .expect("failed to run conformance executable");
    assert!(
        output.status.success(),
        "LLVM execution failed for run corpus [{case_names}]"
    );
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert_eq!(
        stdout, expected,
        "unexpected output for LLVM run corpus [{case_names}]"
    );
}

#[test]
fn run_pass_corpus_with_llvm() {
    let cases_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases/run/pass");
    let mut cases = fs::read_dir(&cases_dir)
        .expect("run corpus directory is missing")
        .map(|entry| entry.expect("invalid corpus directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "nrs"))
        .collect::<Vec<_>>();
    cases.sort();
    assert!(!cases.is_empty(), "run corpus has no cases");

    let mut source = String::new();
    let mut expected = String::new();
    let mut case_names = Vec::new();
    for case_path in cases {
        source.push_str(&fs::read_to_string(&case_path).expect("failed to read run case"));
        source.push('\n');
        let expected_path = case_path.with_extension("stdout");
        expected
            .push_str(&fs::read_to_string(&expected_path).expect("run case has no .stdout file"));
        case_names.push(case_path.display().to_string());
    }
    let case_names = case_names.join(", ");
    run_llvm(&case_names, &source, &expected);
}

/// The workbench embeds `examples/*.nrs` files individually (see
/// `apps/snacc-workbench/build.rs` and `snippets.json`), each paired with a
/// `.stdout` sidecar that `build.rs` only checks for existence. Run each
/// example on its own, the same way the workbench runs a snippet, so a stale
/// or wrong `.stdout` file fails a test instead of shipping unverified.
#[test]
fn run_examples_individually_with_llvm() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut cases = fs::read_dir(&examples_dir)
        .expect("examples directory is missing")
        .map(|entry| entry.expect("invalid examples directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "nrs"))
        .filter(|path| path.with_extension("stdout").is_file())
        .collect::<Vec<_>>();
    cases.sort();
    assert!(
        !cases.is_empty(),
        "no examples/*.nrs files have a matching .stdout sidecar"
    );

    for case_path in cases {
        let case_name = case_path.display().to_string();
        let source = fs::read_to_string(&case_path).expect("failed to read example");
        let expected = fs::read_to_string(case_path.with_extension("stdout"))
            .expect("example has no .stdout file");
        run_llvm(&case_name, &source, &expected);
    }
}
