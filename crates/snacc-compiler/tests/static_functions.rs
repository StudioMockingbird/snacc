use snacc_compiler::{check, emit_llvm_ir, parse};

#[test]
fn associated_functions_have_a_type_namespace_and_no_receiver() {
    let source = "type Point is struct x: Int64 end static Point.origin(): Point do Point(x: 1) end static Int64.answer(): Int64 do 42 end let point: Point = Point.origin() print(Int64.answer())";
    let program = check(source).expect("associated functions should type-check");
    assert!(program.funcs.contains_key("Point.origin"));
    assert!(program.funcs.contains_key("Int64.answer"));
    emit_llvm_ir(source).expect("associated functions should lower");
}

#[test]
fn associated_functions_support_forward_calls_and_recursion() {
    check(
        "type Counter is struct end static Counter.first(): Int64 do Counter.second() end static Counter.second(): Int64 do 2 end print(Counter.first())",
    )
    .expect("associated declarations should be visible independent of source order");
}

#[test]
fn associated_function_names_are_unique_per_type() {
    let diagnostics = match check(
        "type Point is struct end static Point.make(): Point do Point() end static Point.make(): Point do Point() end",
    ) {
        Ok(_) => panic!("duplicate associated functions must be rejected"),
        Err(diagnostics) => diagnostics,
    };
    assert!(format!("{diagnostics:?}").contains("already exists"));
}

#[test]
fn associated_functions_do_not_introduce_self_or_generic_forms() {
    assert!(check("type Point is struct end static Point.bad(): Point do self end").is_err());
    assert!(
        parse("type Point<T> is struct value: T end static Point<Int64>.bad(): Int64 do 1 end")
            .is_err()
    );
    assert!(
        parse("type Point is struct end static Point.make<T>(): Point do Point() end").is_err()
    );
}

#[test]
fn built_in_string_constructors_remain_reserved() {
    let diagnostics =
        match check("static String.from_utf8(value: View<Byte>): String do \"bad\" end") {
            Ok(_) => panic!("the built-in String constructor must not be redeclared"),
            Err(diagnostics) => diagnostics,
        };
    assert!(format!("{diagnostics:?}").contains("built in"));
}
