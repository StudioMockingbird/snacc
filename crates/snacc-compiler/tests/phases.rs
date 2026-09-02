use snacc_compiler::emit_object;

#[test]
fn llvm_emits_objects_in_process() {
    let object =
        emit_object("print(2 + 3)\nwhile false do print(7) end").expect("LLVM emission failed");
    assert!(!object.is_empty(), "LLVM emitted an empty object");
}

/// `emit_object` runs `module.verify()`, so a block that received a second
/// terminator, a dead instruction past one, or a merge block with no terminator
/// fails here instead of reaching an object file. RFC 008 conformance item 8 is
/// proven this way rather than by inspecting IR.
#[test]
fn terminated_blocks_never_receive_a_second_terminator() {
    for source in [
        "while true do break end",
        // Source after `break` is not lowered as reachable code.
        "while true do break print(1) end",
        // Every branch terminates, so nothing reaches the `if` merge block.
        "while true do if true then break else break end print(1) end",
        // A nested `break` leaves the inner loop, not the outer one.
        "fun spin() do while true do while true do break end break end end\nspin()",
        // A value-producing `if` still merges through one phi.
        "fun pick(flag: Bool): Int64 do if flag then 1 else 2 end end\nprint(pick(true))",
        // A no-result function lowers to LLVM `void`.
        "fun announce(value: Int64) do print(value) end\nannounce(1)",
    ] {
        emit_object(source)
            .unwrap_or_else(|error| panic!("LLVM emission failed for {source:?}: {error:?}"));
    }
}
