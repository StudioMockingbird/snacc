use crate::Optimization;
use crate::semantics::checker::{ArithOp, CmpOp, Program, TBlock, TExpr, TStmt, TValueIf, Ty};
use crate::syntax::ast::NumLiteral;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, PointerValue,
};
use inkwell::{FloatPredicate, IntPredicate, OptimizationLevel};
use std::collections::HashMap;

fn llvm_ty(context: &Context, ty: Ty) -> BasicTypeEnum<'_> {
    match ty {
        Ty::Dec64 => context.f64_type().into(),
        Ty::Int64 => context.i64_type().into(),
        Ty::Bool | Ty::Nil => context.i8_type().into(),
    }
}

/// A declaration without a result lowers to an LLVM `void` function; no value
/// type stands in for its absent result.
fn function_type<'ctx>(
    context: &'ctx Context,
    params: &[(String, Ty)],
    result: Option<Ty>,
) -> inkwell::types::FunctionType<'ctx> {
    let mut llvm_params = Vec::new();
    for (_, ty) in params {
        let param: BasicMetadataTypeEnum = llvm_ty(context, *ty).into();
        llvm_params.push(param);
    }
    match result {
        Some(ty) => llvm_ty(context, ty).fn_type(&llvm_params, false),
        None => context.void_type().fn_type(&llvm_params, false),
    }
}

/// How a local's current value is reached. Immutable roots stay as SSA values;
/// only a `let mut` root needs addressable storage.
#[derive(Clone, Copy)]
enum Slot<'ctx> {
    Value(BasicValueEnum<'ctx>),
    Mutable(PointerValue<'ctx>, BasicTypeEnum<'ctx>),
}

type Env<'ctx> = Vec<(String, Slot<'ctx>)>;

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
            function_type(&context, &function.params, function.result),
            None,
        );
        functions.insert(name.clone(), llvm_function);
    }
    for (name, function) in &program.funcs {
        let llvm_function = module.add_function(
            &format!("snacc_fn_{name}"),
            function_type(&context, &function.params, function.result),
            Some(Linkage::Internal),
        );
        functions.insert(name.clone(), llvm_function);
    }

    let cg = Codegen {
        context: &context,
        builder: &builder,
        functions: &functions,
        print_f64,
        print_i64,
        print_bool,
        print_nil,
    };

    for (name, function) in &program.funcs {
        let llvm_function = functions[name];
        let entry = context.append_basic_block(llvm_function, "entry");
        builder.position_at_end(entry);

        let mut env: Env = Vec::new();
        for ((name, _), value) in function.params.iter().zip(llvm_function.get_params()) {
            env.push((name.clone(), Slot::Value(value)));
        }

        let mut loops = Vec::new();
        let (value, terminated) = cg.block(&mut env, &mut loops, &function.body)?;
        if !terminated {
            match (function.result, value) {
                (Some(_), Some(value)) => builder.build_return(Some(&value)),
                (None, _) => builder.build_return(None),
                (Some(_), None) => {
                    return Err("checker produced a result function without a result value".into());
                }
            }
            .map_err(|error| error.to_string())?;
        }
    }

    // The Rust runtime owns the platform entry point and calls this stable ABI
    // boundary. Snacc has no exit-code semantics yet, so success returns zero.
    let entry_type = context.i32_type().fn_type(&[], false);
    let entry_function = module.add_function("snacc_main", entry_type, None);
    let entry = context.append_basic_block(entry_function, "entry");
    builder.position_at_end(entry);
    let mut env: Env = Vec::new();
    let mut loops = Vec::new();
    let (_, terminated) = cg.block(&mut env, &mut loops, &program.body)?;
    if !terminated {
        let zero = context.i32_type().const_zero();
        builder
            .build_return(Some(&zero))
            .map_err(|error| error.to_string())?;
    }

    module.verify().map_err(|error| error.to_string())?;
    let object = machine
        .write_to_memory_buffer(&module, FileType::Object)
        .map_err(|error| error.to_string())?;
    Ok((object.as_slice().to_vec(), target_triple()))
}

struct Codegen<'ctx, 'a> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    functions: &'a HashMap<String, FunctionValue<'ctx>>,
    print_f64: FunctionValue<'ctx>,
    print_i64: FunctionValue<'ctx>,
    print_bool: FunctionValue<'ctx>,
    print_nil: FunctionValue<'ctx>,
}

/// Basic blocks a `break` may branch to, innermost last.
type Loops<'ctx> = Vec<BasicBlock<'ctx>>;

impl<'ctx> Codegen<'ctx, '_> {
    fn current_function(&self) -> FunctionValue<'ctx> {
        self.builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .expect("lowering always occurs inside a function")
    }

    /// Every `alloca` goes in the entry block so a mutable root declared inside
    /// a loop body does not grow the stack per iteration.
    fn entry_alloca(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let function = self.current_function();
        let resume = self
            .builder
            .get_insert_block()
            .expect("lowering always occurs inside a block");
        let entry = function
            .get_first_basic_block()
            .expect("every lowered function has an entry block");
        match entry.get_first_instruction() {
            Some(instruction) => self.builder.position_before(&instruction),
            None => self.builder.position_at_end(entry),
        }
        let slot = self
            .builder
            .build_alloca(ty, name)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(resume);
        Ok(slot)
    }

    /// Lowers a block's statements in order, then its optional result value.
    /// Returns whether control flow left the block terminated, so no caller
    /// ever appends a second terminator or a merge branch to a dead block.
    fn block(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        block: &TBlock,
    ) -> Result<(Option<BasicValueEnum<'ctx>>, bool), String> {
        let scope = env.len();
        let mut terminated = false;
        for statement in &block.statements {
            if terminated {
                // RFC 008: source after a terminator is not lowered as
                // reachable code.
                break;
            }
            terminated = self.stmt(env, loops, statement)?;
        }
        let value = match (&block.result, terminated) {
            (Some(result), false) => Some(self.expr(env, loops, result)?),
            _ => None,
        };
        env.truncate(scope);
        Ok((value, terminated))
    }

    /// Returns whether this statement terminated the current basic block.
    fn stmt(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        statement: &TStmt,
    ) -> Result<bool, String> {
        match statement {
            TStmt::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let value = self.expr(env, loops, value)?;
                if *mutable {
                    let ty = llvm_ty(self.context, *ty);
                    let slot = self.entry_alloca(ty, name)?;
                    self.builder
                        .build_store(slot, value)
                        .map_err(|error| error.to_string())?;
                    env.push((name.clone(), Slot::Mutable(slot, ty)));
                } else {
                    env.push((name.clone(), Slot::Value(value)));
                }
                Ok(false)
            }
            TStmt::Assign { name, value } => {
                let value = self.expr(env, loops, value)?;
                match lookup(env, name) {
                    Some(Slot::Mutable(slot, _)) => {
                        self.builder
                            .build_store(slot, value)
                            .map_err(|error| error.to_string())?;
                        Ok(false)
                    }
                    _ => Err("checker allowed an assignment to an immutable local".into()),
                }
            }
            TStmt::While { condition, body } => {
                let function = self.current_function();
                let condition_block = self.context.append_basic_block(function, "while_condition");
                let body_block = self.context.append_basic_block(function, "while_body");
                let exit_block = self.context.append_basic_block(function, "while_exit");
                self.builder
                    .build_unconditional_branch(condition_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(condition_block);
                let test = self.condition(env, loops, condition)?;
                self.builder
                    .build_conditional_branch(test, body_block, exit_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(body_block);
                loops.push(exit_block);
                let (_, terminated) = self.block(env, loops, body)?;
                loops.pop();
                if !terminated {
                    self.builder
                        .build_unconditional_branch(condition_block)
                        .map_err(|error| error.to_string())?;
                }

                // The exit block is always reachable: the condition branches to
                // it when false.
                self.builder.position_at_end(exit_block);
                Ok(false)
            }
            TStmt::Break => {
                let exit = *loops
                    .last()
                    .ok_or("checker allowed a 'break' outside every loop")?;
                self.builder
                    .build_unconditional_branch(exit)
                    .map_err(|error| error.to_string())?;
                Ok(true)
            }
            TStmt::If(form) => {
                let function = self.current_function();
                let merge = self.context.append_basic_block(function, "if_merge");
                let mut reaches_merge = false;
                for (condition, body) in &form.arms {
                    let test = self.condition(env, loops, condition)?;
                    let then_block = self.context.append_basic_block(function, "if_then");
                    let next_block = self.context.append_basic_block(function, "if_next");
                    self.builder
                        .build_conditional_branch(test, then_block, next_block)
                        .map_err(|error| error.to_string())?;
                    self.builder.position_at_end(then_block);
                    let (_, terminated) = self.block(env, loops, body)?;
                    if !terminated {
                        self.builder
                            .build_unconditional_branch(merge)
                            .map_err(|error| error.to_string())?;
                        reaches_merge = true;
                    }
                    self.builder.position_at_end(next_block);
                }
                // The builder now sits in the block reached when no arm
                // matched: the `else` body, or a direct path to the merge.
                match &form.else_branch {
                    Some(body) => {
                        let (_, terminated) = self.block(env, loops, body)?;
                        if !terminated {
                            self.builder
                                .build_unconditional_branch(merge)
                                .map_err(|error| error.to_string())?;
                            reaches_merge = true;
                        }
                    }
                    None => {
                        self.builder
                            .build_unconditional_branch(merge)
                            .map_err(|error| error.to_string())?;
                        reaches_merge = true;
                    }
                }
                self.builder.position_at_end(merge);
                if reaches_merge {
                    Ok(false)
                } else {
                    // Every branch terminated, so nothing reaches the merge. It
                    // still needs a terminator to verify.
                    self.builder
                        .build_unreachable()
                        .map_err(|error| error.to_string())?;
                    Ok(true)
                }
            }
            TStmt::Call(name, args) => {
                self.call(env, loops, name, args)?;
                Ok(false)
            }
            TStmt::Expr(expression) => {
                self.expr(env, loops, expression)?;
                Ok(false)
            }
        }
    }

    fn condition(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        condition: &TExpr,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let value = self.expr(env, loops, condition)?.into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                value,
                self.context.i8_type().const_zero(),
                "condition",
            )
            .map_err(|error| error.to_string())
    }

    fn call(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        name: &str,
        args: &[TExpr],
    ) -> Result<inkwell::values::CallSiteValue<'ctx>, String> {
        let mut llvm_args = Vec::new();
        for arg in args {
            let value: BasicMetadataValueEnum = self.expr(env, loops, arg)?.into();
            llvm_args.push(value);
        }
        self.builder
            .build_call(self.functions[name], &llvm_args, "call")
            .map_err(|error| error.to_string())
    }

    fn value_if(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        form: &TValueIf,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "if_merge");
        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        for (condition, body) in &form.arms {
            let test = self.condition(env, loops, condition)?;
            let then_block = self.context.append_basic_block(function, "if_then");
            let next_block = self.context.append_basic_block(function, "if_next");
            self.builder
                .build_conditional_branch(test, then_block, next_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(then_block);
            self.branch_value(env, loops, body, merge, &mut incoming)?;
            self.builder.position_at_end(next_block);
        }
        self.branch_value(env, loops, &form.else_branch, merge, &mut incoming)?;

        self.builder.position_at_end(merge);
        if incoming.is_empty() {
            return Err("a value-producing 'if' had no branch that produces a value".into());
        }
        let phi = self
            .builder
            .build_phi(llvm_ty(self.context, form.ty), "if_value")
            .map_err(|error| error.to_string())?;
        let incoming: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
            .iter()
            .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
            .collect();
        phi.add_incoming(&incoming);
        Ok(phi.as_basic_value())
    }

    /// Lowers one branch of a value-producing `if`, recording its incoming phi
    /// edge only when the branch actually reaches the merge block.
    fn branch_value(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        body: &TBlock,
        merge: BasicBlock<'ctx>,
        incoming: &mut Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)>,
    ) -> Result<(), String> {
        let (value, terminated) = self.block(env, loops, body)?;
        if terminated {
            return Ok(());
        }
        let value = value.ok_or("a value-producing 'if' branch produced no value")?;
        let end = self
            .builder
            .get_insert_block()
            .expect("a lowered branch has an insertion block");
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        incoming.push((value, end));
        Ok(())
    }

    fn expr(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        expr: &TExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match expr {
            TExpr::Num(literal) => match literal {
                NumLiteral::Dec(value) => Ok(self.context.f64_type().const_float(*value).into()),
                NumLiteral::Int(value) => Ok(self
                    .context
                    .i64_type()
                    .const_int(*value as u64, true)
                    .into()),
            },
            TExpr::Bool(value) => Ok(self
                .context
                .i8_type()
                .const_int(*value as u64, false)
                .into()),
            TExpr::Nil => Ok(self.context.i8_type().const_zero().into()),
            TExpr::Cast(value, Ty::Dec64) => {
                let value = self.expr(env, loops, value)?.into_int_value();
                Ok(self
                    .builder
                    .build_signed_int_to_float(value, self.context.f64_type(), "i64_to_f64")
                    .map_err(|e| e.to_string())?
                    .into())
            }
            TExpr::Cast(_, _) => Err("checker emitted an unsupported cast".into()),
            TExpr::Local(name) => match lookup(env, name) {
                Some(Slot::Value(value)) => Ok(value),
                Some(Slot::Mutable(slot, ty)) => self
                    .builder
                    .build_load(ty, slot, name)
                    .map_err(|error| error.to_string()),
                None => Err("type checking guarantees every local resolves".into()),
            },
            TExpr::Arith(left, op, right, ty) => {
                let left = self.expr(env, loops, left)?;
                let right = self.expr(env, loops, right)?;
                match ty {
                    Ty::Dec64 => {
                        let left = left.into_float_value();
                        let right = right.into_float_value();
                        let value = match op {
                            ArithOp::Add => self.builder.build_float_add(left, right, "add"),
                            ArithOp::Sub => self.builder.build_float_sub(left, right, "sub"),
                            ArithOp::Mul => self.builder.build_float_mul(left, right, "mul"),
                            ArithOp::Div => self.builder.build_float_div(left, right, "div"),
                        }
                        .map_err(|e| e.to_string())?;
                        Ok(value.into())
                    }
                    Ty::Int64 => {
                        let left = left.into_int_value();
                        let right = right.into_int_value();
                        let value = match op {
                            ArithOp::Add => self.builder.build_int_add(left, right, "add"),
                            ArithOp::Sub => self.builder.build_int_sub(left, right, "sub"),
                            ArithOp::Mul => self.builder.build_int_mul(left, right, "mul"),
                            ArithOp::Div => self.builder.build_int_signed_div(left, right, "div"),
                        }
                        .map_err(|e| e.to_string())?;
                        Ok(value.into())
                    }
                    _ => Err("checker produced non-numeric arithmetic".into()),
                }
            }
            TExpr::Cmp(left, op, right, operand_ty) => {
                let left = self.expr(env, loops, left)?;
                let right = self.expr(env, loops, right)?;
                let builder = self.builder;
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
                    .build_int_z_extend(comparison, self.context.i8_type(), "bool")
                    .map_err(|error| error.to_string())?;
                Ok(value.into())
            }
            TExpr::Call(name, args) => {
                let call = self.call(env, loops, name, args)?;
                Ok(call
                    .try_as_basic_value()
                    .expect_basic("a checked call expression always returns a value"))
            }
            TExpr::If(form) => self.value_if(env, loops, form),
            TExpr::Print(value, ty) => {
                let value = self.expr(env, loops, value)?;
                let function = match ty {
                    Ty::Dec64 => self.print_f64,
                    Ty::Int64 => self.print_i64,
                    Ty::Bool => self.print_bool,
                    Ty::Nil => {
                        self.builder
                            .build_call(self.print_nil, &[], "")
                            .map_err(|error| error.to_string())?;
                        return Ok(value);
                    }
                };
                self.builder
                    .build_call(function, &[value.into()], "")
                    .map_err(|error| error.to_string())?;
                Ok(value)
            }
        }
    }
}

fn lookup<'ctx>(env: &Env<'ctx>, name: &str) -> Option<Slot<'ctx>> {
    env.iter()
        .rev()
        .find(|(bound, _)| bound == name)
        .map(|(_, slot)| *slot)
}
