use snacc_compiler::emit_object;

#[test]
fn llvm_emits_objects_in_process() {
    let object = emit_object("print(2 + 3); while false do 7 end").expect("LLVM emission failed");
    assert!(!object.is_empty(), "LLVM emitted an empty object");
}
