use snacc_compiler::{DiagnosticPhase, check, parse};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn cases(kind: &str, outcome: &str) -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cases")
        .join(kind)
        .join(outcome);
    let mut paths = fs::read_dir(directory)
        .expect("language corpus directory is missing")
        .map(|entry| entry.expect("invalid language corpus entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "nrs"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn expected_message(path: &Path) -> Option<String> {
    let path = path.with_extension("stderr");
    path.exists()
        .then(|| fs::read_to_string(path).expect("failed to read .stderr expectation"))
}

fn assert_expected_diagnostic(path: &Path, phases: &[DiagnosticPhase], source: &str) {
    let diagnostics = parse(source).err().expect("case unexpectedly parsed");
    assert!(
        !diagnostics.items.is_empty(),
        "case has no diagnostics: {path:?}"
    );
    assert!(
        diagnostics
            .items
            .iter()
            .all(|diagnostic| phases.contains(&diagnostic.phase))
    );
    if let Some(expected) = expected_message(path) {
        let messages = diagnostics
            .items
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            messages.contains(expected.trim()),
            "unexpected diagnostics for {path:?}: {messages}"
        );
    }
}

#[test]
fn parse_corpus_matches_expected_outcomes() {
    for path in cases("parse", "pass") {
        let source = fs::read_to_string(&path).expect("failed to read parse case");
        parse(&source)
            .unwrap_or_else(|diagnostics| panic!("parse case failed {path:?}: {diagnostics:?}"));
    }
    for path in cases("parse", "fail") {
        let source = fs::read_to_string(&path).expect("failed to read parse case");
        assert_expected_diagnostic(
            &path,
            &[DiagnosticPhase::Lex, DiagnosticPhase::Parse],
            &source,
        );
    }
}

#[test]
fn typecheck_corpus_matches_expected_outcomes() {
    for path in cases("typecheck", "pass") {
        let source = fs::read_to_string(&path).expect("failed to read typecheck case");
        check(&source).unwrap_or_else(|diagnostics| {
            panic!("typecheck case failed {path:?}: {diagnostics:?}")
        });
    }
    for path in cases("typecheck", "fail") {
        let source = fs::read_to_string(&path).expect("failed to read typecheck case");
        let diagnostics = check(&source)
            .err()
            .expect("typecheck case unexpectedly passed");
        assert!(
            !diagnostics.items.is_empty(),
            "case has no diagnostics: {path:?}"
        );
        assert!(
            diagnostics
                .items
                .iter()
                .all(|diagnostic| diagnostic.phase == DiagnosticPhase::TypeCheck)
        );
        if let Some(expected) = expected_message(&path) {
            let messages = diagnostics
                .items
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                messages.contains(expected.trim()),
                "unexpected diagnostics for {path:?}: {messages}"
            );
        }
    }
}
