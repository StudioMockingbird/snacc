use crate::Optimization;
use crate::semantics::checker::{
    ArithOp, CmpOp, Place, PlaceRoot, Program, TBlock, TCondition, TExpr, TStmt, TValueIf, Ty,
};
use crate::syntax::ast::NumLiteral;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, IntType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, PointerValue,
};
use inkwell::{FloatPredicate, IntPredicate, OptimizationLevel};
use std::collections::HashMap;

/// Specification 010's user-defined types have no lowering in this milestone's
/// front-end task. `build_module` rejects any program that declares one, so a
/// `Ty::User` never reaches the helpers below; each still rejects explicitly
/// rather than panicking or silently guessing a representation.
const USER_TYPE_UNSUPPORTED: &str =
    "lowering a user-defined type is not implemented yet (Specification 010 backend task)";

/// Specification 009 section 5.2: the value/storage type for each scalar.
fn llvm_ty(context: &Context, ty: Ty) -> Result<BasicTypeEnum<'_>, String> {
    Ok(match ty {
        Ty::Dec64 => context.f64_type().into(),
        Ty::Float32 => context.f32_type().into(),
        Ty::Int64 | Ty::UInt64 => context.i64_type().into(),
        Ty::UInt32 => context.i32_type().into(),
        Ty::UInt16 => context.i16_type().into(),
        Ty::Bool | Ty::Nil | Ty::UInt8 => context.i8_type().into(),
        Ty::User(_) => return Err(USER_TYPE_UNSUPPORTED.into()),
    })
}

/// Whether a type is lowered as a floating-point value rather than an integer.
fn is_float(ty: Ty) -> bool {
    match ty {
        Ty::Dec64 | Ty::Float32 => true,
        Ty::Int64
        | Ty::UInt8
        | Ty::UInt16
        | Ty::UInt32
        | Ty::UInt64
        | Ty::Bool
        | Ty::Nil
        | Ty::User(_) => false,
    }
}

/// Whether an integer type's division and ordering are unsigned. Signedness is
/// read from the checked type, never inferred from an LLVM bit width.
fn is_unsigned(ty: Ty) -> bool {
    match ty {
        Ty::UInt8 | Ty::UInt16 | Ty::UInt32 | Ty::UInt64 => true,
        Ty::Int64 | Ty::Dec64 | Ty::Float32 | Ty::Bool | Ty::Nil | Ty::User(_) => false,
    }
}

/// The runtime import that prints one scalar. `Nil` carries no value, so it
/// prints through a niladic import.
fn print_import<'ctx>(
    context: &'ctx Context,
    ty: Ty,
) -> Result<(&'static str, Vec<BasicMetadataTypeEnum<'ctx>>), String> {
    let symbol = match ty {
        Ty::Dec64 => "snacc_print_f64",
        Ty::Float32 => "snacc_print_f32",
        Ty::Int64 => "snacc_print_i64",
        Ty::UInt8 => "snacc_print_u8",
        Ty::UInt16 => "snacc_print_u16",
        Ty::UInt32 => "snacc_print_u32",
        Ty::UInt64 => "snacc_print_u64",
        Ty::Bool => "snacc_print_bool",
        Ty::Nil => return Ok(("snacc_print_nil", Vec::new())),
        Ty::User(_) => return Err(USER_TYPE_UNSUPPORTED.into()),
    };
    Ok((symbol, vec![llvm_ty(context, ty)?.into()]))
}

fn is_subword_int<'ctx>(ty: impl TryInto<IntType<'ctx>>) -> bool {
    ty.try_into()
        .is_ok_and(|int: IntType<'ctx>| int.get_bit_width() < 32)
}

/// Rust's `extern "C"` functions carry `zeroext` on sub-word integer parameters
/// and results on every target Snacc emits for (confirmed against rustc's own
/// IR). Specification 009 section 5.2 requires the backend to match that rather
/// than assume an LLVM width alone defines the call ABI, so declarations and
/// their call sites both get the attribute -- this covers `UInt8`, `UInt16`,
/// and the pre-existing `Bool`/`Nil` `u8` mapping alike.
fn zero_extend_subwords<'ctx>(
    context: &'ctx Context,
    signature: FunctionType<'ctx>,
    mut add: impl FnMut(AttributeLoc, Attribute),
) {
    let zeroext = context.create_enum_attribute(Attribute::get_named_enum_kind_id("zeroext"), 0);
    for (index, param) in signature.get_param_types().into_iter().enumerate() {
        if is_subword_int(param) {
            add(AttributeLoc::Param(index as u32), zeroext);
        }
    }
    if signature.get_return_type().is_some_and(is_subword_int) {
        add(AttributeLoc::Return, zeroext);
    }
}

fn declare<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    symbol: &str,
    signature: FunctionType<'ctx>,
    linkage: Option<Linkage>,
) -> FunctionValue<'ctx> {
    let function = module.add_function(symbol, signature, linkage);
    zero_extend_subwords(context, signature, |location, attribute| {
        function.add_attribute(location, attribute)
    });
    function
}

/// A declaration without a result lowers to an LLVM `void` function; no value
/// type stands in for its absent result.
fn function_type<'ctx>(
    context: &'ctx Context,
    params: &[(String, Ty)],
    result: Option<Ty>,
) -> Result<inkwell::types::FunctionType<'ctx>, String> {
    let mut llvm_params = Vec::new();
    for (_, ty) in params {
        let param: BasicMetadataTypeEnum = llvm_ty(context, *ty)?.into();
        llvm_params.push(param);
    }
    Ok(match result {
        Some(ty) => llvm_ty(context, ty)?.fn_type(&llvm_params, false),
        None => context.void_type().fn_type(&llvm_params, false),
    })
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

fn host_machine(optimization: Optimization) -> Result<TargetMachine, String> {
    Target::initialize_x86(&InitializationConfig::default());

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|error| error.to_string())?;
    let optimization = match optimization {
        Optimization::None => OptimizationLevel::None,
        Optimization::Aggressive => OptimizationLevel::Aggressive,
    };
    target
        .create_target_machine(
            &triple,
            "generic",
            "",
            optimization,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| "LLVM could not create a target machine for this host".to_string())
}

/// Compiles the checked program directly to a native object for the host.
pub fn compile(
    program: &Program,
    module_name: &str,
    optimization: Optimization,
) -> Result<(Vec<u8>, String), String> {
    let machine = host_machine(optimization)?;
    let context = Context::create();
    let module = build_module(&context, program, module_name, &machine)?;
    let object = machine
        .write_to_memory_buffer(&module, FileType::Object)
        .map_err(|error| error.to_string())?;
    Ok((object.as_slice().to_vec(), target_triple()))
}

/// Renders a checked program as LLVM IR. Calling-convention attributes exist
/// only in the IR -- no object file preserves them -- so this is how they are
/// verified.
pub fn compile_to_ir(program: &Program, module_name: &str) -> Result<String, String> {
    let machine = host_machine(Optimization::None)?;
    let context = Context::create();
    let module = build_module(&context, program, module_name, &machine)?;
    Ok(module.print_to_string().to_string())
}

fn build_module<'ctx>(
    context: &'ctx Context,
    program: &Program,
    module_name: &str,
    machine: &TargetMachine,
) -> Result<Module<'ctx>, String> {
    // Specification 010's lowering lands with the backend task; until then a
    // program that declares a user type is rejected here rather than partially
    // lowered.
    if !program.types.is_empty() || !program.methods.is_empty() {
        return Err(USER_TYPE_UNSUPPORTED.into());
    }

    let triple = TargetMachine::get_default_triple();
    let module = context.create_module(module_name);
    let builder = context.create_builder();
    module.set_triple(&triple);
    module.set_data_layout(&machine.get_target_data().get_data_layout());

    // Every function is declared before any body is lowered, so recursion and
    // forward calls do not depend on source or hash-map iteration order.
    let mut functions = HashMap::new();
    for (name, function) in &program.externs {
        let llvm_function = declare(
            context,
            &module,
            &function.symbol,
            function_type(context, &function.params, function.result)?,
            None,
        );
        functions.insert(name.clone(), llvm_function);
    }
    for (name, function) in &program.funcs {
        let llvm_function = declare(
            context,
            &module,
            &format!("snacc_fn_{name}"),
            function_type(context, &function.params, function.result)?,
            Some(Linkage::Internal),
        );
        functions.insert(name.clone(), llvm_function);
    }

    let cg = Codegen {
        context,
        builder: &builder,
        module: &module,
        functions: &functions,
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
    Ok(module)
}

struct Codegen<'ctx, 'a> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    module: &'a Module<'ctx>,
    functions: &'a HashMap<String, FunctionValue<'ctx>>,
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
                    let ty = llvm_ty(self.context, *ty)?;
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
            TStmt::Assign { place, value } => {
                let value = self.expr(env, loops, value)?;
                let name = scalar_local(place)?;
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
            TStmt::MethodCall(_) => Err(USER_TYPE_UNSUPPORTED.into()),
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
                    let test = self.arm_condition(env, loops, condition)?;
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

    /// An `if`/`elseif` arm's condition. A proven type test lowers to a tag
    /// comparison in the backend task; nothing here guesses one.
    fn arm_condition(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        condition: &TCondition,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        match condition {
            TCondition::Expr(expression) => self.condition(env, loops, expression),
            TCondition::Test(_) => Err(USER_TYPE_UNSUPPORTED.into()),
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
        self.invoke(self.functions[name], &llvm_args)
    }

    /// Builds a call and repeats the callee's ABI extension attributes on the
    /// call site, the way a C compiler does, so a bridge's sub-word arguments
    /// and result agree on both sides of the boundary.
    fn invoke(
        &self,
        callee: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Result<inkwell::values::CallSiteValue<'ctx>, String> {
        let call = self
            .builder
            .build_call(callee, args, "call")
            .map_err(|error| error.to_string())?;
        zero_extend_subwords(self.context, callee.get_type(), |location, attribute| {
            call.add_attribute(location, attribute)
        });
        Ok(call)
    }

    /// Declares the runtime print import for `ty` on first use.
    fn print_import(&self, ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let (symbol, params) = print_import(self.context, ty)?;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(&params, false),
                None,
            )
        }))
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
            let test = self.arm_condition(env, loops, condition)?;
            let then_block = self.context.append_basic_block(function, "if_then");
            let next_block = self.context.append_basic_block(function, "if_next");
            self.builder
                .build_conditional_branch(test, then_block, next_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(then_block);
            self.branch_value(env, loops, body, merge, &mut incoming)?;
            self.builder.position_at_end(next_block);
        }
        // An exhaustive type-test chain has no `else`; that lowering lands with
        // the backend task.
        let else_branch = form
            .else_branch
            .as_ref()
            .ok_or_else(|| USER_TYPE_UNSUPPORTED.to_string())?;
        self.branch_value(env, loops, else_branch, merge, &mut incoming)?;

        self.builder.position_at_end(merge);
        if incoming.is_empty() {
            return Err("a value-producing 'if' had no branch that produces a value".into());
        }
        let phi = self
            .builder
            .build_phi(llvm_ty(self.context, form.ty)?, "if_value")
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
            // Each literal arrived at its exact value in the lexer, so nothing
            // here re-parses or re-rounds it. `f32 as f64` is exact, so the
            // already-rounded binary32 value reaches `const_float` unchanged.
            TExpr::Num(literal) => Ok(match literal {
                NumLiteral::Dec(value) => self.context.f64_type().const_float(*value).into(),
                NumLiteral::F32(value) => self
                    .context
                    .f32_type()
                    .const_float(f64::from(*value))
                    .into(),
                NumLiteral::Int(value) => self
                    .context
                    .i64_type()
                    .const_int(*value as u64, true)
                    .into(),
                NumLiteral::U8(value) => self
                    .context
                    .i8_type()
                    .const_int(u64::from(*value), false)
                    .into(),
                NumLiteral::U16(value) => self
                    .context
                    .i16_type()
                    .const_int(u64::from(*value), false)
                    .into(),
                NumLiteral::U32(value) => self
                    .context
                    .i32_type()
                    .const_int(u64::from(*value), false)
                    .into(),
                NumLiteral::U64(value) => self.context.i64_type().const_int(*value, false).into(),
            }),
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
            TExpr::Place(place) => {
                let name = scalar_local(place)?;
                match lookup(env, name) {
                    Some(Slot::Value(value)) => Ok(value),
                    Some(Slot::Mutable(slot, ty)) => self
                        .builder
                        .build_load(ty, slot, name)
                        .map_err(|error| error.to_string()),
                    None => Err("type checking guarantees every local resolves".into()),
                }
            }
            TExpr::FieldRead { .. }
            | TExpr::Construct { .. }
            | TExpr::Represent { .. }
            | TExpr::Inject { .. }
            | TExpr::MethodCall(_) => Err(USER_TYPE_UNSUPPORTED.into()),
            TExpr::Arith(left, op, right, ty) => {
                let ty = *ty;
                let left = self.expr(env, loops, left)?;
                let right = self.expr(env, loops, right)?;
                let builder = self.builder;
                if is_float(ty) {
                    // `Float32` operands are already `float`, so every rounding
                    // happens at binary32 and never through `double`.
                    let (left, right) = (left.into_float_value(), right.into_float_value());
                    let value = match op {
                        ArithOp::Add => builder.build_float_add(left, right, "add"),
                        ArithOp::Sub => builder.build_float_sub(left, right, "sub"),
                        ArithOp::Mul => builder.build_float_mul(left, right, "mul"),
                        ArithOp::Div => builder.build_float_div(left, right, "div"),
                    }
                    .map_err(|e| e.to_string())?;
                    return Ok(value.into());
                }
                if matches!(ty, Ty::Bool | Ty::Nil) {
                    return Err("checker produced non-numeric arithmetic".into());
                }
                // Plain `add`/`sub`/`mul` on an N-bit integer already wrap
                // modulo 2^N; no no-wrap flag may be added or the modular
                // result Specification 009 section 4.5 requires becomes poison.
                // Unsigned division is `udiv`, whose division by zero is
                // undefined behavior by that same section -- deliberately
                // unguarded.
                let (left, right) = (left.into_int_value(), right.into_int_value());
                let value = match op {
                    ArithOp::Add => builder.build_int_add(left, right, "add"),
                    ArithOp::Sub => builder.build_int_sub(left, right, "sub"),
                    ArithOp::Mul => builder.build_int_mul(left, right, "mul"),
                    ArithOp::Div if is_unsigned(ty) => {
                        builder.build_int_unsigned_div(left, right, "div")
                    }
                    ArithOp::Div => builder.build_int_signed_div(left, right, "div"),
                }
                .map_err(|e| e.to_string())?;
                Ok(value.into())
            }
            TExpr::Cmp(left, op, right, operand_ty) => {
                let operand_ty = *operand_ty;
                let left = self.expr(env, loops, left)?;
                let right = self.expr(env, loops, right)?;
                let builder = self.builder;
                let ordered = !matches!(op, CmpOp::Eq | CmpOp::NotEq);
                if ordered && matches!(operand_ty, Ty::Bool | Ty::Nil) {
                    return Err("checker allowed an ordered non-numeric comparison".into());
                }
                let comparison = if is_float(operand_ty) {
                    // `Float32` reuses the `Dec64` rule: every predicate but
                    // `!=` is ordered, so a NaN operand makes it false.
                    let predicate = match op {
                        CmpOp::Eq => FloatPredicate::OEQ,
                        CmpOp::NotEq => FloatPredicate::UNE,
                        CmpOp::Less => FloatPredicate::OLT,
                        CmpOp::LessEq => FloatPredicate::OLE,
                        CmpOp::Greater => FloatPredicate::OGT,
                        CmpOp::GreaterEq => FloatPredicate::OGE,
                    };
                    builder.build_float_compare(
                        predicate,
                        left.into_float_value(),
                        right.into_float_value(),
                        "compare",
                    )
                } else {
                    let unsigned = is_unsigned(operand_ty);
                    let predicate = match op {
                        CmpOp::Eq => IntPredicate::EQ,
                        CmpOp::NotEq => IntPredicate::NE,
                        CmpOp::Less if unsigned => IntPredicate::ULT,
                        CmpOp::Less => IntPredicate::SLT,
                        CmpOp::LessEq if unsigned => IntPredicate::ULE,
                        CmpOp::LessEq => IntPredicate::SLE,
                        CmpOp::Greater if unsigned => IntPredicate::UGT,
                        CmpOp::Greater => IntPredicate::SGT,
                        CmpOp::GreaterEq if unsigned => IntPredicate::UGE,
                        CmpOp::GreaterEq => IntPredicate::SGE,
                    };
                    builder.build_int_compare(
                        predicate,
                        left.into_int_value(),
                        right.into_int_value(),
                        "compare",
                    )
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
                let function = self.print_import(*ty)?;
                let args = match ty {
                    Ty::Nil => Vec::new(),
                    _ => vec![value.into()],
                };
                self.invoke(function, &args)?;
                Ok(value)
            }
        }
    }
}

/// The local name of a place that selects no field. Field paths and `self`
/// roots lower with the backend task.
fn scalar_local(place: &Place) -> Result<&str, String> {
    match (&place.root, place.path.is_empty()) {
        (PlaceRoot::Local(name), true) => Ok(name),
        _ => Err(USER_TYPE_UNSUPPORTED.into()),
    }
}

fn lookup<'ctx>(env: &Env<'ctx>, name: &str) -> Option<Slot<'ctx>> {
    env.iter()
        .rev()
        .find(|(bound, _)| bound == name)
        .map(|(_, slot)| *slot)
}
