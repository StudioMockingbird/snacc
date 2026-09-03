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

/// Specification 010 sections 15.2-15.4 and conformance items 20 and 29. These
/// are the lowering facts the run corpus cannot observe from stdout: a union's
/// LLVM shape, its deterministic `i32` source-order tags, the zero-initialized
/// aggregate every injection starts from, and the hidden receiver pointer.
#[test]
fn unions_lower_to_a_tag_and_one_storage_field_per_member() {
    let ir = emit_llvm_ir(
        "type Shade is union\n\
        \x20   | Dim\n\
        \x20   | Mid is struct\n\
        \x20       level: Int64,\n\
        \x20     end\n\
        \x20   | Full is struct\n\
        \x20       level: Int64,\n\
        \x20       hue: Int64,\n\
        \x20     end\n\
         end\n\
         method Shade.Full.total(): Int64 do self.level + self.hue end\n\
         fun wrap(level: Int64): Shade do Shade.Mid(level: level) end\n\
         fun rank(shade: Shade): Int64 do\n\
        \x20   if shade is Shade.Dim then 0\n\
        \x20   elseif shade is Shade.Mid(mid) then mid.level\n\
        \x20   elseif shade is Shade.Full(full) then full.total()\n\
        \x20   end\n\
         end\n\
         print(rank(wrap(3)))\n",
    )
    .unwrap_or_else(|error| panic!("LLVM emission failed: {error:?}"));

    // One `i32` tag plus one storage field per member, in source order.
    assert!(
        ir.contains("%Shade = type { i32, %Shade.Dim, %Shade.Mid, %Shade.Full }"),
        "wrong union layout in:\n{ir}"
    );
    assert!(
        ir.contains("%Shade.Dim = type {}"),
        "an empty member is not an empty struct in:\n{ir}"
    );

    // Every `is` test compares the stored tag against its member's source
    // position, so the three tags are exactly 0, 1, and 2.
    let mut tags: Vec<&str> = ir
        .lines()
        .filter_map(|line| line.trim().strip_prefix("%is"))
        .filter_map(|line| line.rsplit_once("icmp eq i32 %tag"))
        .map(|(_, rest)| rest.rsplit_once(", ").expect("a tag comparison").1)
        .collect();
    tags.sort_unstable();
    assert_eq!(tags, ["0", "1", "2"], "wrong union tags in:\n{ir}");

    // Injection starts from the complete union's zero initializer, so every
    // inactive member slot is deterministic rather than poison.
    let injections: Vec<&str> = ir
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("insertvalue %Shade "))
        .collect();
    assert!(!injections.is_empty(), "no union injection in:\n{ir}");
    for injection in injections {
        assert!(
            injection.contains("%Shade.Dim zeroinitializer")
                && injection.contains("%Shade.Full zeroinitializer"),
            "an injection skipped zero initialization: {injection}"
        );
    }

    // A method is an internal function whose hidden first parameter is the
    // receiver's address; the symbol is derived from the receiver and method
    // IDs and is not public ABI.
    assert!(
        ir.lines().any(
            |line| line.starts_with("define internal i64 @snacc_method_") && line.contains("(ptr ")
        ),
        "no method with a hidden receiver pointer in:\n{ir}"
    );

    // A proven-exhaustive chain has no `else`; its fall-through traps instead
    // of producing a value.
    assert!(
        ir.contains("unreachable"),
        "an exhaustive chain kept a fall-through value path in:\n{ir}"
    );
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
