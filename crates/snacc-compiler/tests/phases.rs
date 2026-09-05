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
        // Specification 026: an early `return` terminates its block exactly
        // like `break` does, and source after it is not lowered either.
        "fun stop() do return end\nstop()",
        "fun stop(): Int64 do return print(1) end\nprint(stop())",
        // Every branch of a value-form `if` returns, so no merge block value
        // is ever read.
        "fun bit(flag: Bool): Int64 do if flag then return 1 else return 0 end end\n\
         print(bit(true))",
        // A returning branch beside a value-producing one still merges
        // through one phi with only the live branch as an incoming edge.
        "fun normalize(value: Int64): Int64 do if value < 0 then return 0 else value end end\n\
         print(normalize(0 - 1))",
        // A `return` inside a `while` body exits the function, not the loop;
        // the loop's own exit block is still reachable on the untaken edge.
        "fun first(): Int64 do while true do return 1 end 0 end\nprint(first())",
        // A `return` inside a method body.
        "type Point is struct x: Int64, end\n\
         method Point.x_or_zero(flag: Bool): Int64 do if flag then return self.x end 0 end\n\
         print(Point(x: 5).x_or_zero(true))",
    ] {
        emit_object(source)
            .unwrap_or_else(|error| panic!("LLVM emission failed for {source:?}: {error:?}"));
    }
}

/// RFC 016 phase 5: a box is lowered as one non-null pointer, allocation uses
/// the pointee size and alignment, and the checked cleanup plan emits the
/// matching deallocation on scope exit.
#[test]
fn boxes_lower_through_the_runtime_allocator_and_cleanup() {
    let ir = emit_llvm_ir(
        "type Node is struct value: Int64, end\n\
         let node: Box<Node> = box(Node(value: 7))\n\
         print(node.value)\n",
    )
    .unwrap_or_else(|error| panic!("LLVM emission failed: {error:?}"));
    assert!(
        ir.contains("declare ptr @snacc_alloc"),
        "missing allocator import:\n{ir}"
    );
    assert!(
        ir.contains("declare void @snacc_dealloc"),
        "missing deallocator import:\n{ir}"
    );
    assert!(
        ir.contains("call ptr @snacc_alloc"),
        "box was not allocated:\n{ir}"
    );
    assert!(
        ir.contains("call void @snacc_dealloc"),
        "box cleanup was not emitted:\n{ir}"
    );
}

#[test]
fn floating_results_are_guarded_before_they_become_snacc_values() {
    let ir = emit_llvm_ir(concat!(
        "fun divide(left: Float64, right: Float64): Float64 do left / right end\n",
        "extern rust \"snacc_user_nan\" fun nan(): Float64\n",
        "print(divide(0.0, 1.0))\n",
        "print(nan())\n",
    ))
    .unwrap_or_else(|error| panic!("LLVM emission failed: {error:?}"));
    assert!(
        ir.contains("fcmp uno"),
        "missing unordered float guard:\n{ir}"
    );
    assert!(
        ir.contains("@snacc_invalid_floating_operation"),
        "missing invalid-floating-operation import:\n{ir}"
    );
}

/// Specification 009 phase 3: every new scalar lowers through its own LLVM
/// type. `module.verify()` inside `emit_object` rejects a mismatched width,
/// predicate class, or print signature, so this covers every lowering path the
/// run corpus cannot observe from stdout.
#[test]
fn every_new_scalar_lowers_to_llvm() {
    for (name, literal) in [
        ("Byte", "1u8"),
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

/// Specification 018 section 8 and Phase 4 items 1-3: an inline sum reuses
/// the named-union tag-plus-fields shape, its member tags are its canonical
/// (sorted) member order rather than declaration order, and every injection
/// starts from the complete sum's zero initializer -- the same lowering-only
/// facts the run corpus cannot observe from stdout.
#[test]
fn inline_sums_lower_to_a_tag_and_one_zero_initialized_storage_field_per_member() {
    let ir = emit_llvm_ir(
        "fun pick(flag: Bool, byte: Byte): Byte | Nil do\n\
        \x20   if flag then byte else nil end\n\
         end\n\
         fun rank(value: Byte | Nil): Byte do\n\
        \x20   if value is Byte(byte) then byte elseif value is Nil then 0u8 end\n\
         end\n\
         print(rank(pick(true, 7u8)))\n",
    )
    .unwrap_or_else(|error| panic!("LLVM emission failed: {error:?}"));

    // One `i32` tag plus one storage field per member. `Nil` lowers as a
    // scalar field here (like `Bool`), not as the zero-field struct a named
    // union's `Nil` member would be, because an inline sum member is never
    // itself a `TypeId`.
    assert!(
        ir.contains("%sum.0 = type { i32, i8, i8 }"),
        "wrong sum layout in:\n{ir}"
    );

    // Canonical member order sorts `Nil` before `Byte` (Specification 018
    // section 4), so `is Nil` and `is Byte(...)` compare the stored tag
    // against 0 and 1 respectively -- not their written source order.
    let mut tags: Vec<&str> = ir
        .lines()
        .filter_map(|line| line.trim().strip_prefix("%is"))
        .filter_map(|line| line.rsplit_once("icmp eq i32 %tag"))
        .map(|(_, rest)| rest.rsplit_once(", ").expect("a tag comparison").1)
        .collect();
    tags.sort_unstable();
    assert_eq!(tags, ["0", "1"], "wrong sum tags in:\n{ir}");

    // Injection starts from the complete sum's zero initializer, so every
    // slot still inactive at the point the active member is written is a
    // deterministic zero, never poison or undef (Phase 4 item 3). The tag
    // write folds into a compile-time constant (both its operands are
    // constants), so the runtime `insertvalue` that installs the
    // non-constant active member starts from that already-tagged,
    // still-all-zero-elsewhere constant rather than from `undef`.
    let injections: Vec<&str> = ir
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("insertvalue %sum.0 "))
        .collect();
    assert!(!injections.is_empty(), "no sum injection in:\n{ir}");
    for injection in injections {
        assert!(
            injection.contains("{ i32 1, i8 0, i8 0 }"),
            "an injection did not start from an all-zero-but-tag base: {injection}"
        );
    }
    assert!(
        !ir.contains("undef") && !ir.contains("poison"),
        "an inactive sum field was left uninitialized in:\n{ir}"
    );
}

/// Specification 009 section 5.2: sub-word bridge parameters and results carry
/// the same `zeroext` attribute rustc emits for `extern "C"` `u8`/`u16`, on the
/// declaration and on the call site. `UInt32` and wider carry none.
#[test]
fn sub_word_bridge_declarations_and_calls_carry_zeroext() {
    for (name, literal, extended) in [
        ("Byte", "1u8", true),
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

/// Specification 011 section 11: the lowering facts the run corpus cannot
/// observe from stdout. A `Ref<T>` parameter is a `ptr` in the signature and
/// binds directly to the incoming value -- unlike a `let mut` local, it gets no
/// `alloca` of its own -- and every call site (internal, forwarded, and bridge)
/// passes the caller's address with no intervening copy.
#[test]
fn a_reference_parameter_lowers_to_a_pointer_with_no_alloca_of_its_own() {
    let ir = emit_llvm_ir(
        "extern rust \"snacc_user_scale\" fun scale(value: Ref<Int64>, by: Int64)\n\
         fun add_into(x: Int64, y: Int64, result: Ref<Int64>) do result = x + y end\n\
         fun forward(target: Ref<Int64>) do add_into(1, 2, target) end\n\
         let mut z: Int64 = 0\n\
         add_into(20, 22, z)\n\
         forward(z)\n\
         scale(z, 3)\n\
         print(z)\n",
    )
    .unwrap_or_else(|error| panic!("LLVM emission failed: {error:?}"));

    // The referent's own type never appears: pointers are opaque, and a value
    // parameter beside a reference one keeps its by-value type.
    assert!(
        ir.contains("define internal void @snacc_fn_add_into(i64 %0, i64 %1, ptr %2)"),
        "wrong reference parameter signature in:\n{ir}"
    );
    assert!(
        ir.contains("declare void @snacc_user_scale(ptr, i64)"),
        "wrong bridge reference parameter signature in:\n{ir}"
    );

    // The whole point of Design Decision 2: the incoming value already is the
    // address, so the callee allocates nothing and stores straight through it.
    let body = ir
        .split("define internal void @snacc_fn_add_into")
        .nth(1)
        .and_then(|rest| rest.split("\n}").next())
        .expect("add_into was not lowered");
    assert!(
        !body.contains("alloca"),
        "a reference parameter must not get an alloca of its own:\n{body}"
    );
    assert!(
        body.contains("store i64 %add, ptr %2"),
        "an assignment through a reference must store through the incoming pointer:\n{body}"
    );

    // A `let mut` local, by contrast, does allocate -- and that same allocation
    // is what every reference argument passes, unloaded and uncopied, including
    // the reborrow that forwards its own incoming pointer straight on.
    assert!(
        ir.contains("%z = alloca i64"),
        "missing local storage in:\n{ir}"
    );
    for expected in [
        "call void @snacc_fn_add_into(i64 20, i64 22, ptr %z)",
        "call void @snacc_fn_forward(ptr %z)",
        "call void @snacc_user_scale(ptr %z, i64 3)",
        "call void @snacc_fn_add_into(i64 1, i64 2, ptr %0)",
    ] {
        assert!(
            ir.contains(expected),
            "a reference argument did not pass an address directly ({expected}):\n{ir}"
        );
    }
}
