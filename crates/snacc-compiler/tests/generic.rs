use snacc_compiler::{check, emit_llvm_ir, parse};

#[test]
fn parses_and_checks_generic_identity() {
    let source = "fun identity<T>(value: T): T do value end print(identity<Int64>(42))";
    parse(source).expect("generic syntax should parse");
    let program = check(source).expect("generic identity should type-check");
    assert!(
        program
            .funcs
            .keys()
            .any(|name| name.starts_with("$snacc$generic$identity$"))
    );
}

#[test]
fn checks_generic_struct_application() {
    let source = "type Pair<A, B> is struct first: A, second: B, end let pair: Pair<Int64, Bool> = Pair<Int64, Bool>(first: 42, second: true) print(pair.first)";
    parse(source).expect("generic type syntax should parse");
    check(source).expect("generic struct should type-check");
}

#[test]
fn monomorphizes_nested_generic_calls() {
    let source = "fun identity<T>(value: T): T do value end fun relay<U>(value: U): U do identity<U>(value) end print(relay<Int64>(7))";
    let program = check(source).expect("nested generic calls should type-check");
    assert!(
        program
            .funcs
            .keys()
            .any(|name| name.starts_with("$snacc$generic$identity$"))
    );
    assert!(
        program
            .funcs
            .keys()
            .any(|name| name.starts_with("$snacc$generic$relay$"))
    );
}

#[test]
fn rejects_generic_calls_without_explicit_arguments() {
    let diagnostics = match check("fun identity<T>(value: T): T do value end print(identity(1))") {
        Ok(_) => panic!("generic type arguments are mandatory"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("not callable"));
}

#[test]
fn lowers_a_specialization_to_llvm() {
    let source = "fun identity<T>(value: T): T do value end print(identity<Int64>(42))";
    let ir = emit_llvm_ir(source).expect("specialized generic should lower");
    assert!(ir.contains("snacc$generic$identity$"));
}

#[test]
fn rejects_duplicate_type_parameters_and_trailing_generic_commas() {
    let diagnostics = match check("fun bad<T, T>(value: T): T do value end") {
        Ok(_) => panic!("duplicate generic parameters must be rejected"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("already exists"));
    assert!(parse("fun bad<T,>(value: T): T do value end").is_err());
    assert!(parse("type Pair<A,> is struct value: A end").is_err());
    assert!(parse("fun id<T>(value: T): T do value end id<Int64,>(1)").is_err());
}

#[test]
fn rejects_operations_on_nested_and_direct_unconstrained_values() {
    for source in [
        "fun bad<T>(value: T): T do if value then value else value end end",
        "fun bad<T>(value: T): T do value.field end",
        "fun bad<T>(value: T): T do value[0] end",
        "fun bad<T>(value: T): String do \"{{value}}\" end",
        "fun bad<T>(value: List<T>): Bool do value == value end",
    ] {
        assert!(
            check(source).is_err(),
            "generic operation unexpectedly accepted: {source}"
        );
    }
}

#[test]
fn rejects_infinite_generic_value_layouts_but_accepts_boxed_recursion() {
    let cyclic =
        "type Node<T> is struct next: Node<T> end let value: Node<Int64> = Node<Int64>(next: nil)";
    let diagnostics = match check(cyclic) {
        Ok(_) => panic!("a direct generic layout cycle must fail"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("infinite value layout"));

    let boxed = "type Node<T> is struct value: T, next: Box<Node<T>> | Nil, end";
    check(boxed).expect("boxing must terminate a generic layout cycle");
}

#[test]
fn specialization_symbols_are_canonical_and_cannot_alias_source_names() {
    let source = "type A is struct end type B is struct end type C is struct end type A_B is struct end type B_C is struct end fun pick<X, Y>(value: X): X do value end let first: A_B = pick<A_B, C>(A_B()) let second: A = pick<A, B_C>(A())";
    let program = check(source).expect("distinct type argument lists should specialize");
    let names = program
        .funcs
        .keys()
        .filter(|name| name.starts_with("$snacc$generic$pick$"))
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);
}

#[test]
fn specialization_errors_include_declaration_use_and_chain_context() {
    let source = "fun wrong<T>(value: Int64): T do value end let result: Bool = wrong<Bool>(1)";
    let diagnostics = match check(source) {
        Ok(_) => panic!("the specialized result mismatch must fail"),
        Err(diagnostics) => diagnostics,
    };
    let rendered = format!("{diagnostics:?}");
    assert!(rendered.contains("while specializing wrong<Bool>"));
    assert!(rendered.contains("declared at"));
    assert!(rendered.contains("requested at"));
    assert!(rendered.contains("instantiation chain"));
}

#[test]
fn expanding_generic_type_recursion_stops_at_the_fixed_depth_limit() {
    let source =
        "type Grow<T> is struct next: Grow<Box<T>> end fun consume(value: Grow<Int64>) do end";
    let diagnostics = match check(source) {
        Ok(_) => panic!("ever-expanding generic recursion must be bounded"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("specialization depth exceeds 128"));
}

#[test]
fn each_generic_struct_specialization_gets_its_own_structural_properties() {
    let moved = "type Holder<T> is struct value: T end fun consume(value: Holder<String>) do end let holder: Holder<String> = Holder<String>(value: \"owned\") consume(holder) print(holder.value)";
    let diagnostics = match check(moved) {
        Ok(_) => panic!("Holder<String> must inherit String's move-only property"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("already moved"));

    check("type Holder<T> is struct value: T end let left: Holder<Int64> = Holder<Int64>(value: 1) let right: Holder<Int64> = Holder<Int64>(value: 1) print(left == right)")
        .expect("Holder<Int64> should derive structural equality");
    assert!(check("type Holder<T> is struct value: T end let left: Holder<Box<Int64>> = Holder<Box<Int64>>(value: box(1)) let right: Holder<Box<Int64>> = Holder<Box<Int64>>(value: box(1)) print(left == right)").is_err());
}

#[test]
fn unused_generic_declarations_still_validate_parameters_fields_and_types() {
    for source in [
        "fun bad<T>(left: T, left: T): T do left end",
        "fun bad<T>(value: Missing): T do value end",
        "fun bad<T>(value: T): T do let copy: Missing = value copy end",
        "type Bad<T> is struct value: Missing end",
        "type Bad<T> is struct value: T, value: T end",
        "type Pair<A, B> is struct first: A, second: B end fun bad<T>(value: Pair<T>): T do value.first end",
    ] {
        assert!(
            check(source).is_err(),
            "unused generic declaration unexpectedly escaped validation: {source}"
        );
    }
}

#[test]
fn rejects_unconstrained_generic_operations() {
    let diagnostics = match check("fun add<T>(left: T, right: T): T do left + right end") {
        Ok(_) => panic!("unconstrained generic arithmetic must be rejected"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("unconstrained generic type parameter"));
}
