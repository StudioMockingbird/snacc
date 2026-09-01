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
