use snacc_compiler::{check, emit_llvm_ir};

#[test]
fn lowers_each_maximal_concat_chain_to_one_runtime_plan() {
    let ir = emit_llvm_ir(
        "let name: String = \"Ada\" let message: String = \"Hello \".concat(name).concat(3).concat(true)",
    )
    .expect("concatenation should lower");
    assert_eq!(
        ir.matches("call void @snacc_string_concat_parts_out")
            .count(),
        1
    );
    assert!(!ir.contains("call void @snacc_string_concat_out"));
}

#[test]
fn view_carrying_aggregates_preserve_all_source_borrows() {
    let prefix = "type Views is struct first: View<Byte>, second: View<Unicode>, end ";
    let invalid = format!(
        "{prefix}let mut first: String = \"a\" let mut second: String = \"b\" let views: Views = Views(first: first.bytes(), second: second.unicode()) first = \"x\" print(views.first.length())"
    );
    let diagnostics = match check(&invalid) {
        Ok(_) => panic!("an aggregate must keep every source String borrowed"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("still borrows"));

    let valid = format!(
        "{prefix}let mut first: String = \"a\" let second: String = \"b\" let views: Views = Views(first: first.bytes(), second: second.unicode()) print(views.first.length()) first = \"x\""
    );
    check(&valid).expect("all aggregate borrows should end after the aggregate's last use");
}

#[test]
fn lowers_interpolation_through_the_same_single_allocation_plan() {
    let ir = emit_llvm_ir("let message: String = \"value={{7}}, flag={{true}}\"")
        .expect("interpolation should lower");
    assert_eq!(
        ir.matches("call void @snacc_string_concat_parts_out")
            .count(),
        1
    );
}
