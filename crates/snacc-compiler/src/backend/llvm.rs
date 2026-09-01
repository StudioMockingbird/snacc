use crate::Optimization;
use crate::semantics::checker::{ArithOp, CmpOp, Program, TExpr, Ty};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};
use inkwell::{FloatPredicate, IntPredicate, OptimizationLevel};
use std::collections::HashMap;

fn llvm_ty(context: &Context, ty: Ty) -> BasicTypeEnum<'_> {
    match ty {
        Ty::Dec64 => context.f64_type().into(),
        Ty::Int64 => context.i64_type().into(),
        Ty::Bool | Ty::Nil => context.i8_type().into(),
    }
}

fn default_value<'ctx>(context: &'ctx Context, ty: Ty) -> BasicValueEnum<'ctx> {
    match ty {
        Ty::Dec64 => context.f64_type().const_zero().into(),
        Ty::Int64 => context.i64_type().const_zero().into(),
        Ty::Bool | Ty::Nil => context.i8_type().const_zero().into(),
    }
}

fn function_type<'ctx>(
    context: &'ctx Context,
    params: &[(String, Ty)],
    ret: Ty,
) -> inkwell::types::FunctionType<'ctx> {
    let mut llvm_params = Vec::new();
    for (_, ty) in params {
        let param: BasicMetadataTypeEnum = llvm_ty(context, *ty).into();
        llvm_params.push(param);
    }
    llvm_ty(context, ret).fn_type(&llvm_params, false)
}

/// Returns the host triple used for native object emission.
pub fn target_triple() -> String {
    TargetMachine::get_default_triple()
        .as_str()
        .to_string_lossy()
        .into_owned()
}

pub fn llvm_version() -> (u32, u32, u32) {
    let mut major = 0;
    let mut minor = 0;
    let mut patch = 0;
    // LLVMGetVersion initializes all three out-parameters and does not retain
    // their addresses.
    unsafe {
        inkwell::llvm_sys::core::LLVMGetVersion(&mut major, &mut minor, &mut patch);
    }
    (major, minor, patch)
}

/// Compiles the checked program directly to a native object for the host.
pub fn compile(
    program: &Program,
    module_name: &str,
    optimization: Optimization,
) -> Result<(Vec<u8>, String), String> {
    Target::initialize_x86(&InitializationConfig::default());

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|error| error.to_string())?;
    let optimization = match optimization {
        Optimization::None => OptimizationLevel::None,
        Optimization::Aggressive => OptimizationLevel::Aggressive,
    };
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            optimization,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| "LLVM could not create a target machine for this host".to_string())?;

    let context = Context::create();
    let module = context.create_module(module_name);
    let builder = context.create_builder();
    module.set_triple(&triple);
    module.set_data_layout(&machine.get_target_data().get_data_layout());

    let print_f64_type = context
        .void_type()
        .fn_type(&[context.f64_type().into()], false);
    let print_f64 = module.add_function("snacc_print_f64", print_f64_type, None);
    let print_i64_type = context
        .void_type()
        .fn_type(&[context.i64_type().into()], false);
    let print_i64 = module.add_function("snacc_print_i64", print_i64_type, None);
    let print_bool_type = context
        .void_type()
        .fn_type(&[context.i8_type().into()], false);
    let print_bool = module.add_function("snacc_print_bool", print_bool_type, None);
    let print_nil = module.add_function(
        "snacc_print_nil",
        context.void_type().fn_type(&[], false),
        None,
    );

    // Every function is declared before any body is lowered, so recursion and
    // forward calls do not depend on source or hash-map iteration order.
    let mut functions = HashMap::new();
    for (name, function) in &program.externs {
        let llvm_function = module.add_function(
            &function.symbol,
            function_type(&context, &function.params, function.ret),
            None,
        );
        functions.insert(name.clone(), llvm_function);
    }
    for (name, function) in &program.funcs {
        let llvm_function = module.add_function(
            &format!("snacc_fn_{name}"),
            function_type(&context, &function.params, function.ret),
            Some(Linkage::Internal),
        );
        functions.insert(name.clone(), llvm_function);
    }

    for (name, function) in &program.funcs {
        let llvm_function = functions[name];
        let entry = context.append_basic_block(llvm_function, "entry");
        builder.position_at_end(entry);

        let mut env = Vec::new();
        let params = llvm_function.get_params();
        for index in 0..function.params.len() {
            env.push((function.params[index].0.clone(), params[index]));
        }

        let result = lower(
            &context,
            &module,
            &builder,
            &functions,
            print_f64,
            print_i64,
            print_bool,
            print_nil,
            &mut env,
            &function.body,
        )?;
        builder
            .build_return(Some(&result))
            .map_err(|error| error.to_string())?;
    }

    // The Rust runtime owns the platform entry point and calls this stable ABI
    // boundary. Snacc has no exit-code semantics yet, so success returns zero.
    let entry_type = context.i32_type().fn_type(&[], false);
    let entry_function = module.add_function("snacc_main", entry_type, None);
    let entry = context.append_basic_block(entry_function, "entry");
    builder.position_at_end(entry);
    if let Some(body) = &program.body {
        let mut env = Vec::new();
        lower(
            &context, &module, &builder, &functions, print_f64, print_i64, print_bool, print_nil,
            &mut env, body,
        )?;
    }
    let zero = context.i32_type().const_zero();
    builder
        .build_return(Some(&zero))
        .map_err(|error| error.to_string())?;

    module.verify().map_err(|error| error.to_string())?;
    let object = machine
        .write_to_memory_buffer(&module, FileType::Object)
        .map_err(|error| error.to_string())?;
    Ok((object.as_slice().to_vec(), target_triple()))
}

#[allow(clippy::too_many_arguments)]
fn lower<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    functions: &HashMap<String, FunctionValue<'ctx>>,
    print_f64: FunctionValue<'ctx>,
    print_i64: FunctionValue<'ctx>,
    print_bool: FunctionValue<'ctx>,
    print_nil: FunctionValue<'ctx>,
    env: &mut Vec<(String, BasicValueEnum<'ctx>)>,
    expr: &TExpr,
) -> Result<BasicValueEnum<'ctx>, String> {
    match expr {
        TExpr::Num(value, ty) => match ty {
            Ty::Dec64 => Ok(context.f64_type().const_float(*value).into()),
            Ty::Int64 => Ok(context.i64_type().const_int(*value as u64, true).into()),
            _ => Err("numeric literal has a non-numeric type".into()),
        },
        TExpr::Bool(value) => Ok(context.i8_type().const_int(*value as u64, false).into()),
        TExpr::Nil => Ok(context.i8_type().const_zero().into()),
        TExpr::Cast(value, Ty::Dec64) => {
            let value = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, value,
            )?
            .into_int_value();
            Ok(builder
                .build_signed_int_to_float(value, context.f64_type(), "i64_to_f64")
                .map_err(|e| e.to_string())?
                .into())
        }
        TExpr::Cast(_, _) => Err("checker emitted an unsupported cast".into()),
        TExpr::Local(name) => {
            let mut found = None;
            for index in (0..env.len()).rev() {
                if &env[index].0 == name {
                    found = Some(env[index].1);
                    break;
                }
            }
            Ok(found.expect("type checking guarantees every local resolves"))
        }
        TExpr::Let(name, value, body) => {
            let value = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, value,
            )?;
            env.push((name.clone(), value));
            let result = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, body,
            );
            env.pop();
            result
        }
        TExpr::Then(first, second) => {
            lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, first,
            )?;
            lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, second,
            )
        }
        TExpr::Arith(left, op, right, ty) => {
            let left = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, left,
            )?;
            let right = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, right,
            )?;
            match ty {
                Ty::Dec64 => {
                    let left = left.into_float_value();
                    let right = right.into_float_value();
                    let value = match op {
                        ArithOp::Add => builder.build_float_add(left, right, "add"),
                        ArithOp::Sub => builder.build_float_sub(left, right, "sub"),
                        ArithOp::Mul => builder.build_float_mul(left, right, "mul"),
                        ArithOp::Div => builder.build_float_div(left, right, "div"),
                    }
                    .map_err(|e| e.to_string())?;
                    Ok(value.into())
                }
                Ty::Int64 => {
                    let left = left.into_int_value();
                    let right = right.into_int_value();
                    let value = match op {
                        ArithOp::Add => builder.build_int_add(left, right, "add"),
                        ArithOp::Sub => builder.build_int_sub(left, right, "sub"),
                        ArithOp::Mul => builder.build_int_mul(left, right, "mul"),
                        ArithOp::Div => builder.build_int_signed_div(left, right, "div"),
                    }
                    .map_err(|e| e.to_string())?;
                    Ok(value.into())
                }
                _ => Err("checker produced non-numeric arithmetic".into()),
            }
        }
        TExpr::Cmp(left, op, right, operand_ty) => {
            let left = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, left,
            )?;
            let right = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, right,
            )?;
            let comparison = match (*operand_ty, op) {
                (Ty::Dec64, CmpOp::Eq) => builder.build_float_compare(
                    FloatPredicate::OEQ,
                    left.into_float_value(),
                    right.into_float_value(),
                    "equal",
                ),
                (Ty::Dec64, CmpOp::NotEq) => builder.build_float_compare(
                    FloatPredicate::UNE,
                    left.into_float_value(),
                    right.into_float_value(),
                    "not_equal",
                ),
                (Ty::Dec64, CmpOp::Less) => builder.build_float_compare(
                    FloatPredicate::OLT,
                    left.into_float_value(),
                    right.into_float_value(),
                    "less",
                ),
                (Ty::Dec64, CmpOp::LessEq) => builder.build_float_compare(
                    FloatPredicate::OLE,
                    left.into_float_value(),
                    right.into_float_value(),
                    "less_equal",
                ),
                (Ty::Dec64, CmpOp::Greater) => builder.build_float_compare(
                    FloatPredicate::OGT,
                    left.into_float_value(),
                    right.into_float_value(),
                    "greater",
                ),
                (Ty::Dec64, CmpOp::GreaterEq) => builder.build_float_compare(
                    FloatPredicate::OGE,
                    left.into_float_value(),
                    right.into_float_value(),
                    "greater_equal",
                ),
                (Ty::Bool, CmpOp::Eq) => builder.build_int_compare(
                    IntPredicate::EQ,
                    left.into_int_value(),
                    right.into_int_value(),
                    "equal",
                ),
                (Ty::Bool, CmpOp::NotEq) => builder.build_int_compare(
                    IntPredicate::NE,
                    left.into_int_value(),
                    right.into_int_value(),
                    "not_equal",
                ),
                (Ty::Bool, _) => {
                    return Err("checker allowed an ordered boolean comparison".into());
                }
                (Ty::Int64, CmpOp::Eq) => builder.build_int_compare(
                    IntPredicate::EQ,
                    left.into_int_value(),
                    right.into_int_value(),
                    "equal",
                ),
                (Ty::Int64, CmpOp::NotEq) => builder.build_int_compare(
                    IntPredicate::NE,
                    left.into_int_value(),
                    right.into_int_value(),
                    "not_equal",
                ),
                (Ty::Int64, CmpOp::Less) => builder.build_int_compare(
                    IntPredicate::SLT,
                    left.into_int_value(),
                    right.into_int_value(),
                    "less",
                ),
                (Ty::Int64, CmpOp::LessEq) => builder.build_int_compare(
                    IntPredicate::SLE,
                    left.into_int_value(),
                    right.into_int_value(),
                    "less_equal",
                ),
                (Ty::Int64, CmpOp::Greater) => builder.build_int_compare(
                    IntPredicate::SGT,
                    left.into_int_value(),
                    right.into_int_value(),
                    "greater",
                ),
                (Ty::Int64, CmpOp::GreaterEq) => builder.build_int_compare(
                    IntPredicate::SGE,
                    left.into_int_value(),
                    right.into_int_value(),
                    "greater_equal",
                ),
                (Ty::Nil, CmpOp::Eq) => builder.build_int_compare(
                    IntPredicate::EQ,
                    left.into_int_value(),
                    right.into_int_value(),
                    "equal",
                ),
                (Ty::Nil, CmpOp::NotEq) => builder.build_int_compare(
                    IntPredicate::NE,
                    left.into_int_value(),
                    right.into_int_value(),
                    "not_equal",
                ),
                (Ty::Nil, _) => {
                    return Err("checker allowed an ordered non-numeric comparison".into());
                }
            }
            .map_err(|error| error.to_string())?;
            let value = builder
                .build_int_z_extend(comparison, context.i8_type(), "bool")
                .map_err(|error| error.to_string())?;
            Ok(value.into())
        }
        TExpr::Call(name, args) => {
            let mut llvm_args = Vec::new();
            for arg in args {
                let value = lower(
                    context, module, builder, functions, print_f64, print_i64, print_bool,
                    print_nil, env, arg,
                )?;
                let value: BasicMetadataValueEnum = value.into();
                llvm_args.push(value);
            }
            let function = functions[name];
            let call = builder
                .build_call(function, &llvm_args, "call")
                .map_err(|error| error.to_string())?;
            Ok(call
                .try_as_basic_value()
                .expect_basic("checked Snacc functions always return a value"))
        }
        TExpr::If(condition, then_expr, else_expr, result_ty) => {
            let condition = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, condition,
            )?
            .into_int_value();
            let condition = builder
                .build_int_compare(
                    IntPredicate::NE,
                    condition,
                    context.i8_type().const_zero(),
                    "condition",
                )
                .map_err(|error| error.to_string())?;
            let function = builder
                .get_insert_block()
                .and_then(|block| block.get_parent())
                .expect("lowering always occurs inside a function");
            let then_block = context.append_basic_block(function, "then");
            let else_block = context.append_basic_block(function, "else");
            let merge_block = context.append_basic_block(function, "merge");
            builder
                .build_conditional_branch(condition, then_block, else_block)
                .map_err(|error| error.to_string())?;

            builder.position_at_end(then_block);
            let then_value = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, then_expr,
            )?;
            builder
                .build_unconditional_branch(merge_block)
                .map_err(|error| error.to_string())?;
            let then_end = builder
                .get_insert_block()
                .expect("the then branch has an insertion block");

            builder.position_at_end(else_block);
            let else_value = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, else_expr,
            )?;
            builder
                .build_unconditional_branch(merge_block)
                .map_err(|error| error.to_string())?;
            let else_end = builder
                .get_insert_block()
                .expect("the else branch has an insertion block");

            builder.position_at_end(merge_block);
            let phi = builder
                .build_phi(llvm_ty(context, *result_ty), "if_value")
                .map_err(|error| error.to_string())?;
            phi.add_incoming(&[(&then_value, then_end), (&else_value, else_end)]);
            Ok(phi.as_basic_value())
        }
        TExpr::While(condition, body, result_ty) => {
            let function = builder
                .get_insert_block()
                .and_then(|block| block.get_parent())
                .expect("lowering always occurs inside a function");
            let preheader = builder
                .get_insert_block()
                .expect("a while expression has an insertion block");
            let condition_block = context.append_basic_block(function, "while_condition");
            let body_block = context.append_basic_block(function, "while_body");
            let exit_block = context.append_basic_block(function, "while_exit");
            builder
                .build_unconditional_branch(condition_block)
                .map_err(|error| error.to_string())?;

            builder.position_at_end(condition_block);
            let result_phi = builder
                .build_phi(llvm_ty(context, *result_ty), "while_value")
                .map_err(|error| error.to_string())?;
            let initial_value = default_value(context, *result_ty);
            result_phi.add_incoming(&[(&initial_value, preheader)]);
            let condition = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, condition,
            )?
            .into_int_value();
            let condition = builder
                .build_int_compare(
                    IntPredicate::NE,
                    condition,
                    context.i8_type().const_zero(),
                    "while_condition_value",
                )
                .map_err(|error| error.to_string())?;
            builder
                .build_conditional_branch(condition, body_block, exit_block)
                .map_err(|error| error.to_string())?;

            builder.position_at_end(body_block);
            let body_value = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, body,
            )?;
            let body_end = builder
                .get_insert_block()
                .expect("the while body has an insertion block");
            builder
                .build_unconditional_branch(condition_block)
                .map_err(|error| error.to_string())?;
            result_phi.add_incoming(&[(&body_value, body_end)]);

            builder.position_at_end(exit_block);
            Ok(result_phi.as_basic_value())
        }
        TExpr::Print(value, ty) => {
            let value = lower(
                context, module, builder, functions, print_f64, print_i64, print_bool, print_nil,
                env, value,
            )?;
            let function = match ty {
                Ty::Dec64 => print_f64,
                Ty::Int64 => print_i64,
                Ty::Bool => print_bool,
                Ty::Nil => {
                    builder
                        .build_call(print_nil, &[], "")
                        .map_err(|error| error.to_string())?;
                    return Ok(value);
                }
            };
            builder
                .build_call(function, &[value.into()], "")
                .map_err(|error| error.to_string())?;
            Ok(value)
        }
    }
}
