use snacc_compiler::{emit_llvm_ir, emit_object};

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

/// Specification 009 phase 3: every new scalar lowers through its own LLVM
/// type. `module.verify()` inside `emit_object` rejects a mismatched width,
/// predicate class, or print signature, so this covers every lowering path the
/// run corpus cannot observe from stdout.
#[test]
fn every_new_scalar_lowers_to_llvm() {
    for (name, literal) in [
        ("UInt8", "1u8"),
        ("UInt16", "1u16"),
        ("UInt32", "1u32"),
        ("UInt64", "1u64"),
        ("Float32", "1.5f32"),
    ] {
        for body in [
            format!("print({literal} + {literal})"),
            format!("print({literal} - {literal})"),
            format!("print({literal} * {literal})"),
            format!("print({literal} / {literal})"),
            format!("print({literal} < {literal})"),
            format!("print({literal} <= {literal})"),
            format!("print({literal} > {literal})"),
            format!("print({literal} >= {literal})"),
            format!("print({literal} == {literal})"),
            format!("print({literal} != {literal})"),
            format!("print({literal})"),
            format!("let mut slot: {name} = {literal} slot = {literal} print(slot)"),
            format!("fun identity(value: {name}): {name} do value end print(identity({literal}))"),
            format!(
                "fun pick(flag: Bool): {name} do if flag then {literal} else {literal} end end \
                 print(pick(true))"
            ),
            format!(
                "extern rust \"snacc_user_edge\" fun edge(value: {name}): {name} \
                 print(edge({literal}))"
            ),
        ] {
            emit_object(&body).unwrap_or_else(|error| {
                panic!("LLVM emission failed for {name} case {body:?}: {error:?}")
            });
        }
    }
}

/// Specification 009 section 5.2: sub-word bridge parameters and results carry
/// the same `zeroext` attribute rustc emits for `extern "C"` `u8`/`u16`, on the
/// declaration and on the call site. `UInt32` and wider carry none.
#[test]
fn sub_word_bridge_declarations_and_calls_carry_zeroext() {
    for (name, literal, extended) in [
        ("UInt8", "1u8", true),
        ("UInt16", "1u16", true),
        ("Bool", "true", true),
        ("UInt32", "1u32", false),
        ("UInt64", "1u64", false),
        ("Int64", "1", false),
        ("Float32", "1.5f32", false),
    ] {
        let ir = emit_llvm_ir(&format!(
            "extern rust \"snacc_user_edge\" fun edge(value: {name}): {name}\n\
             print(edge({literal}))"
        ))
        .unwrap_or_else(|error| panic!("LLVM emission failed for {name}: {error:?}"));
        let declaration = ir
            .lines()
            .find(|line| line.contains("@snacc_user_edge") && line.starts_with("declare"))
            .unwrap_or_else(|| panic!("no bridge declaration for {name} in:\n{ir}"));
        let call = ir
            .lines()
            .find(|line| line.contains("call") && line.contains("@snacc_user_edge"))
            .unwrap_or_else(|| panic!("no bridge call for {name} in:\n{ir}"));
        assert_eq!(
            declaration.contains("zeroext"),
            extended,
            "wrong declaration extension for {name}: {declaration}"
        );
        assert_eq!(
            call.contains("zeroext"),
            extended,
            "wrong call-site extension for {name}: {call}"
        );
    }
}
