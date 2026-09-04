use crate::Optimization;
use crate::semantics::checker::{
    ArithOp, CmpOp, Place, PlaceRoot, Program, TArg, TBlock, TCondition, TExpr, TMethodCall,
    TParam, TReceiver, TStmt, TSumTypeTest, TTypeTest, TValueIf, Ty,
};
use crate::semantics::types::{BoxId, SumId, TypeDef, TypeId};
use crate::syntax::ast::NumLiteral;
use crate::syntax::ast::ParamMode;
use inkwell::AddressSpace;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetData, TargetMachine,
};
use inkwell::types::{
    BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, IntType, StructType,
};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue,
    StructValue,
};
use inkwell::{FloatPredicate, IntPredicate, OptimizationLevel};
use std::collections::HashMap;

/// Marks a backend failure that is a compiler bug rather than a property of the
/// program. Specification 010 section 19 phase 5 step 7 requires an LLVM
/// verifier failure -- and, by the same reasoning, every "the checker promised
/// this" violation -- to be classified as an internal compiler error rather
/// than an ordinary backend diagnostic.
pub const INTERNAL_ERROR: &str = "internal compiler error: ";

fn internal(message: impl std::fmt::Display) -> String {
    format!("{INTERNAL_ERROR}{message}")
}

/// The receiver's name in a lowered method body. `self` is a reserved word, so
/// no local can collide with it.
const SELF: &str = "self";

/// Specification 009 section 5.2: the value/storage type for each scalar.
fn scalar_ty(context: &Context, ty: Ty) -> BasicTypeEnum<'_> {
    match ty {
        Ty::Dec64 => context.f64_type().into(),
        Ty::Float32 => context.f32_type().into(),
        Ty::Int64 | Ty::UInt64 => context.i64_type().into(),
        Ty::UInt32 => context.i32_type().into(),
        Ty::UInt16 => context.i16_type().into(),
        Ty::Bool | Ty::Nil | Ty::UInt8 => context.i8_type().into(),
        // Every caller routes `Ty::User` and `Ty::Sum` through their layout
        // tables first (see `llvm_ty`).
        Ty::User(_) => unreachable!("a user-defined type resolves through the layout table"),
        Ty::Sum(_) => unreachable!("an inline sum resolves through the sum layout table"),
        // Specification 016 section 4.1/10: `Box<T>` lowers to a non-null
        // target pointer in the private Snacc ABI, exactly like `Ref<T>`'s
        // own pointer representation (`function_type`'s `ParamMode::
        // Reference` arm) -- pointers are opaque in this LLVM version, so
        // the pointee's own type never appears here regardless of `T`.
        Ty::Box(_) => context.ptr_type(AddressSpace::default()).into(),
    }
}

/// The value/storage type for any checked type. A user-defined type is looked
/// up in the layout table built by [`build_layout`]; an inline sum is looked
/// up in that same call's sum layout table, indexed by `SumId` instead of
/// `TypeId` (Specification 018 section 8 reuses named-union lowering, but a
/// sum has no `TypeId` of its own to share that table with).
fn llvm_ty<'ctx>(
    context: &'ctx Context,
    layout: &[BasicTypeEnum<'ctx>],
    sums: &[BasicTypeEnum<'ctx>],
    ty: Ty,
) -> BasicTypeEnum<'ctx> {
    match ty {
        Ty::User(id) => layout[id.index()],
        Ty::Sum(id) => sums[id.index()],
        scalar => scalar_ty(context, scalar),
    }
}

/// Specification 010 section 15.2. A represented type lowers to its immediate
/// representation's LLVM type and gets no named type of its own; a struct and a
/// union member lower to a named LLVM struct in field order; a union lowers to
/// `{i32 tag, member_0, ..., member_n}`, one storage field per member.
/// Specification 018 section 8 reuses that same tag-plus-fields shape for an
/// inline sum, keyed by `SumId` instead of `TypeId` since a sum has no
/// declared name of its own.
///
/// Every named type is predeclared opaque first, then bodies are set through a
/// depth-first walk, so a type is always laid out after everything it contains
/// by value. A struct field or sum member may go the other way too (a sum
/// member may be a user-defined type, and a struct field may be an inline
/// sum), so both walks share one `Layout` and resolve into each other on
/// demand.
fn build_layout<'ctx>(
    context: &'ctx Context,
    defs: &[TypeDef],
    sums: &[Vec<Ty>],
) -> Result<(Vec<BasicTypeEnum<'ctx>>, Vec<BasicTypeEnum<'ctx>>), String> {
    let named: Vec<Option<StructType<'ctx>>> = defs
        .iter()
        .map(|def| match def {
            TypeDef::Represented { .. } => None,
            _ => Some(context.opaque_struct_type(def.name())),
        })
        .collect();
    // A sum has no source name; `sum.<id>` is a debug-only LLVM identifier,
    // never observable from Snacc source (Specification 018 section 8).
    let sum_named: Vec<StructType<'ctx>> = (0..sums.len())
        .map(|index| context.opaque_struct_type(&format!("sum.{index}")))
        .collect();
    let mut state = Layout {
        defs,
        sums,
        named,
        sum_named,
        resolved: vec![None; defs.len()],
        visiting: vec![false; defs.len()],
        sum_resolved: vec![None; sums.len()],
        sum_visiting: vec![false; sums.len()],
    };
    for index in 0..defs.len() {
        state.resolve(context, TypeId(index as u32))?;
    }
    for index in 0..sums.len() {
        state.resolve_sum(context, SumId(index as u32))?;
    }
    let types = state
        .resolved
        .into_iter()
        .map(|ty| ty.expect("every declared type resolves to an LLVM type"))
        .collect();
    let sums = state
        .sum_resolved
        .into_iter()
        .map(|ty| ty.expect("every interned sum resolves to an LLVM type"))
        .collect();
    Ok((types, sums))
}

struct Layout<'ctx, 'a> {
    defs: &'a [TypeDef],
    sums: &'a [Vec<Ty>],
    named: Vec<Option<StructType<'ctx>>>,
    sum_named: Vec<StructType<'ctx>>,
    resolved: Vec<Option<BasicTypeEnum<'ctx>>>,
    visiting: Vec<bool>,
    sum_resolved: Vec<Option<BasicTypeEnum<'ctx>>>,
    sum_visiting: Vec<bool>,
}

impl<'ctx> Layout<'ctx, '_> {
    fn resolve(
        &mut self,
        context: &'ctx Context,
        id: TypeId,
    ) -> Result<BasicTypeEnum<'ctx>, String> {
        if let Some(ty) = self.resolved[id.index()] {
            return Ok(ty);
        }
        if std::mem::replace(&mut self.visiting[id.index()], true) {
            // The checker proves every value layout finite before returning a
            // program, so arriving here means that proof was wrong.
            return Err(internal(format!(
                "'{}' contains itself by value",
                self.defs[id.index()].name()
            )));
        }
        let ty: BasicTypeEnum<'ctx> = match &self.defs[id.index()] {
            TypeDef::Represented { target, .. } => self.resolve_ty(context, *target)?,
            TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => {
                let types = self.resolve_all(context, fields.iter().map(|(_, ty)| *ty))?;
                self.named_ty(id)?.set_body(&types, false);
                self.named_ty(id)?.into()
            }
            TypeDef::Union { members, .. } => {
                let members: Vec<Ty> = members.iter().map(|id| Ty::User(*id)).collect();
                let mut types: Vec<BasicTypeEnum<'ctx>> = vec![context.i32_type().into()];
                types.extend(self.resolve_all(context, members)?);
                self.named_ty(id)?.set_body(&types, false);
                self.named_ty(id)?.into()
            }
        };
        self.visiting[id.index()] = false;
        self.resolved[id.index()] = Some(ty);
        Ok(ty)
    }

    /// An inline sum's `{i32 tag, member_0, ..., member_n}` layout, in the
    /// sum's canonical (sorted) member order -- the same order lowering later
    /// reads a member's position from to assign its deterministic tag
    /// (Specification 018 Phase 4 item 1). A `Nil` member is a scalar here
    /// (`Ty::Nil` lowers through `scalar_ty` like `Bool`), not a zero-field
    /// struct the way a named union's `Nil` member is: unlike a union member,
    /// an inline sum member is never itself a `TypeId`.
    fn resolve_sum(
        &mut self,
        context: &'ctx Context,
        id: SumId,
    ) -> Result<BasicTypeEnum<'ctx>, String> {
        if let Some(ty) = self.sum_resolved[id.index()] {
            return Ok(ty);
        }
        if std::mem::replace(&mut self.sum_visiting[id.index()], true) {
            // The checker's layout-cycle check walks through a sum's direct
            // members into any user-defined type among them, so this can only
            // mean that proof was wrong.
            return Err(internal("an inline sum contains itself by value"));
        }
        let members = self.sums[id.index()].clone();
        let mut types: Vec<BasicTypeEnum<'ctx>> = vec![context.i32_type().into()];
        types.extend(self.resolve_all(context, members)?);
        let named = self.sum_named[id.index()];
        named.set_body(&types, false);
        self.sum_visiting[id.index()] = false;
        self.sum_resolved[id.index()] = Some(named.into());
        Ok(named.into())
    }

    fn resolve_all(
        &mut self,
        context: &'ctx Context,
        types: impl IntoIterator<Item = Ty>,
    ) -> Result<Vec<BasicTypeEnum<'ctx>>, String> {
        types
            .into_iter()
            .map(|ty| self.resolve_ty(context, ty))
            .collect()
    }

    fn resolve_ty(
        &mut self,
        context: &'ctx Context,
        ty: Ty,
    ) -> Result<BasicTypeEnum<'ctx>, String> {
        match ty {
            Ty::User(id) => self.resolve(context, id),
            Ty::Sum(id) => self.resolve_sum(context, id),
            scalar => Ok(scalar_ty(context, scalar)),
        }
    }

    fn named_ty(&self, id: TypeId) -> Result<StructType<'ctx>, String> {
        self.named[id.index()]
            .ok_or_else(|| internal("a represented type was given a named LLVM struct"))
    }
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
        | Ty::User(_)
        | Ty::Sum(_)
        | Ty::Box(_) => false,
    }
}

/// Whether an integer type's division and ordering are unsigned. Signedness is
/// read from the checked type, never inferred from an LLVM bit width.
fn is_unsigned(ty: Ty) -> bool {
    match ty {
        Ty::UInt8 | Ty::UInt16 | Ty::UInt32 | Ty::UInt64 => true,
        Ty::Int64
        | Ty::Dec64
        | Ty::Float32
        | Ty::Bool
        | Ty::Nil
        | Ty::User(_)
        | Ty::Sum(_)
        | Ty::Box(_) => false,
    }
}

/// The runtime import that prints one scalar.
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
        // Specification 010 section 14 rejects printing a user-defined type and
        // Specification 012 section 10 leaves no standalone `Nil` value at all,
        // so neither reaches lowering.
        Ty::User(_) => return Err(internal("a user-defined type reached 'print' lowering")),
        Ty::Nil => return Err(internal("a standalone 'Nil' reached 'print' lowering")),
        Ty::Sum(_) => return Err(internal("an inline sum type reached 'print' lowering")),
        // Specification 016 section 8.3 rejects direct printing of a box in
        // the checker, so this never reaches lowering.
        Ty::Box(_) => return Err(internal("a box type reached 'print' lowering")),
    };
    Ok((symbol, vec![scalar_ty(context, ty).into()]))
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
/// type stands in for its absent result. `leading` supplies the hidden receiver
/// pointer of a method (Specification 010 section 15.3) and is empty otherwise.
///
/// Specification 011 section 11: a `Ref<T>` parameter lowers to a pointer to
/// the caller's storage. Pointers are opaque in this LLVM version, so the
/// referent's own type never appears in the signature.
fn function_type<'ctx>(
    context: &'ctx Context,
    layout: &[BasicTypeEnum<'ctx>],
    sums: &[BasicTypeEnum<'ctx>],
    leading: &[BasicMetadataTypeEnum<'ctx>],
    params: &[TParam],
    result: Option<Ty>,
) -> FunctionType<'ctx> {
    let mut llvm_params = leading.to_vec();
    for param in params {
        llvm_params.push(match param.mode {
            ParamMode::Value => llvm_ty(context, layout, sums, param.ty).into(),
            ParamMode::Reference => context.ptr_type(AddressSpace::default()).into(),
        });
    }
    match result {
        Some(ty) => llvm_ty(context, layout, sums, ty).fn_type(&llvm_params, false),
        None => context.void_type().fn_type(&llvm_params, false),
    }
}

/// How a local's current value is reached. Immutable roots stay as SSA values;
/// a `let mut` root and a method's `self` need addressable storage.
#[derive(Clone, Copy)]
enum Slot<'ctx> {
    Value(BasicValueEnum<'ctx>),
    Mutable(PointerValue<'ctx>),
}

type Env<'ctx> = Vec<(String, Slot<'ctx>)>;

/// How one incoming parameter binds. A `Ref<T>` parameter is a mutable root of
/// its referent (Specification 011 section 7), and its incoming LLVM value is
/// already the address of the caller's storage -- so it binds as the very slot
/// shape a `let mut` local uses, with no `alloca` of its own.
fn param_slot<'ctx>(param: &TParam, value: BasicValueEnum<'ctx>) -> Slot<'ctx> {
    match param.mode {
        ParamMode::Value => Slot::Value(value),
        ParamMode::Reference => Slot::Mutable(value.into_pointer_value()),
    }
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
    let triple = TargetMachine::get_default_triple();
    let module = context.create_module(module_name);
    let builder = context.create_builder();
    module.set_triple(&triple);
    // Specification 016 section 8.2: `box(expression)` needs the target's
    // real size and alignment for its pointee to call the runtime allocator
    // correctly, so this target data outlives the one-off value used just
    // above to set the module's data layout string.
    let target_data = machine.get_target_data();
    module.set_data_layout(&target_data.get_data_layout());

    // Named types come first: every function signature below may mention one.
    let (layout, sum_layout) = build_layout(context, &program.types, &program.sums)?;

    // Every function is declared before any body is lowered, so recursion and
    // forward calls do not depend on source or hash-map iteration order.
    let mut functions = HashMap::new();
    for (name, function) in &program.externs {
        let llvm_function = declare(
            context,
            &module,
            &function.symbol,
            function_type(
                context,
                &layout,
                &sum_layout,
                &[],
                &function.params,
                function.result,
            ),
            None,
        );
        functions.insert(name.clone(), llvm_function);
    }
    for (name, function) in &program.funcs {
        let llvm_function = declare(
            context,
            &module,
            &format!("snacc_fn_{name}"),
            function_type(
                context,
                &layout,
                &sum_layout,
                &[],
                &function.params,
                function.result,
            ),
            Some(Linkage::Internal),
        );
        functions.insert(name.clone(), llvm_function);
    }

    // Specification 010 section 15.3: a method is an internal function whose
    // hidden first parameter is a pointer to the receiver's storage. The symbol
    // is derived from the resolved receiver and method IDs and is not public
    // ABI.
    let receiver_param: BasicMetadataTypeEnum = context.ptr_type(AddressSpace::default()).into();
    let mut methods = Vec::with_capacity(program.methods.len());
    for (id, method) in program.methods.iter().enumerate() {
        methods.push(declare(
            context,
            &module,
            &format!("snacc_method_{}_{id}", method.receiver.0),
            function_type(
                context,
                &layout,
                &sum_layout,
                &[receiver_param],
                &method.params,
                method.result,
            ),
            Some(Linkage::Internal),
        ));
    }

    let cg = Codegen {
        context,
        builder: &builder,
        module: &module,
        functions: &functions,
        methods: &methods,
        program,
        layout: &layout,
        sums: &sum_layout,
        target_data: &target_data,
    };

    for (name, function) in &program.funcs {
        let llvm_function = functions[name];
        let entry = context.append_basic_block(llvm_function, "entry");
        builder.position_at_end(entry);

        let mut env: Env = Vec::new();
        for (param, value) in function.params.iter().zip(llvm_function.get_params()) {
            env.push((param.name.clone(), param_slot(param, value)));
        }
        cg.body(
            &mut env,
            &function.body,
            function.result,
            &function.param_drops,
        )?;
    }

    for (id, method) in program.methods.iter().enumerate() {
        let llvm_function = methods[id];
        let entry = context.append_basic_block(llvm_function, "entry");
        builder.position_at_end(entry);

        let receiver = llvm_function
            .get_first_param()
            .ok_or_else(|| internal("a lowered method has no receiver parameter"))?
            .into_pointer_value();
        let mut env: Env = vec![(SELF.to_string(), Slot::Mutable(receiver))];
        for (param, value) in method
            .params
            .iter()
            .zip(llvm_function.get_params().into_iter().skip(1))
        {
            env.push((param.name.clone(), param_slot(param, value)));
        }
        cg.body(&mut env, &method.body, method.result, &method.param_drops)?;
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

    // Specification 010 section 19 phase 5 step 7: a module the backend built
    // that LLVM rejects is a compiler bug, not a property of the program.
    module
        .verify()
        .map_err(|error| internal(format!("LLVM rejected the generated module: {error}")))?;
    Ok(module)
}

struct Codegen<'ctx, 'a> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    module: &'a Module<'ctx>,
    functions: &'a HashMap<String, FunctionValue<'ctx>>,
    /// Lowered methods, indexed by `MethodId`.
    methods: &'a [FunctionValue<'ctx>],
    /// The checked program, read for type definitions and the receiver-write
    /// effect, both indexed by their resolved ID.
    program: &'a Program,
    /// LLVM types for those definitions, indexed by `TypeId`.
    layout: &'a [BasicTypeEnum<'ctx>],
    /// LLVM types for every interned inline sum, indexed by `SumId`
    /// (Specification 018 section 8).
    sums: &'a [BasicTypeEnum<'ctx>],
    /// The target's real size and alignment facts (Specification 016 section
    /// 8.2): `box(expression)` and every drop of a boxed value need a
    /// pointee's actual byte size and alignment to call the runtime
    /// allocator/deallocator correctly, which an LLVM type alone does not
    /// carry without consulting the target.
    target_data: &'a TargetData,
}

/// Basic blocks a `break` may branch to, innermost last.
type Loops<'ctx> = Vec<BasicBlock<'ctx>>;

/// An `if`/`elseif` arm's pending binding, deferred from [`Codegen::arm_condition`]
/// to [`Codegen::bind`] so it loads only on the successful edge. Kept as two
/// variants rather than folded into one shape because a union member's tag is
/// already resolved (`TTypeTest::tag`) while a sum member's tag is computed
/// from canonical order on demand (`Codegen::sum_member_tag`).
enum ArmBinding<'t> {
    Union(&'t TTypeTest),
    Sum(&'t TSumTypeTest),
}

impl<'ctx> Codegen<'ctx, '_> {
    fn ty(&self, ty: Ty) -> BasicTypeEnum<'ctx> {
        llvm_ty(self.context, self.layout, self.sums, ty)
    }

    /// The named LLVM struct behind a type that has fields, or a union.
    fn struct_ty(&self, ty: Ty) -> Result<StructType<'ctx>, String> {
        match self.ty(ty) {
            BasicTypeEnum::StructType(structure) => Ok(structure),
            _ => Err(internal("a field path reached a type without fields")),
        }
    }

    fn def(&self, id: TypeId) -> &'_ TypeDef {
        &self.program.types[id.index()]
    }

    /// The declared type of one field of a struct or union-member type.
    fn field_ty(&self, ty: Ty, index: usize) -> Result<Ty, String> {
        let Ty::User(id) = ty else {
            return Err(internal("a field path reached a built-in type"));
        };
        self.def(id)
            .fields()
            .and_then(|fields| fields.get(index))
            .map(|(_, ty)| *ty)
            .ok_or_else(|| internal("a field path selected a field that does not exist"))
    }

    /// A union member's deterministic source-order tag.
    fn member_tag(&self, member: TypeId) -> Result<u32, String> {
        match self.def(member) {
            TypeDef::UnionMember { tag, .. } => Ok(*tag),
            _ => Err(internal(
                "injection named a type that is not a union member",
            )),
        }
    }

    /// An inline sum member's deterministic tag (Specification 018 Phase 4
    /// item 1): its position in the sum's canonical (sorted) member list, the
    /// same list `SumTable::intern` built and the checker's `InjectSum` and
    /// `SumTest` nodes both refer back to by `SumId` alone -- neither records
    /// a tag itself, since assigning one is lowering's job.
    fn sum_member_tag(&self, sum: SumId, member: Ty) -> Result<u32, String> {
        self.program.sums[sum.index()]
            .iter()
            .position(|candidate| *candidate == member)
            .map(|index| index as u32)
            .ok_or_else(|| {
                internal(
                    "a sum type test or injection named a type that is not a member of the sum",
                )
            })
    }

    /// A box's pointee type (Specification 016 section 4.1), mirroring
    /// `Types::box_pointee` against the lowering-only snapshot `Program`
    /// carries instead.
    fn box_pointee(&self, id: BoxId) -> Ty {
        self.program.boxes[id.index()]
    }

    /// The target's `usize`-equivalent integer type (Specification 016
    /// section 8.2): the runtime allocator's `size`/`align` parameters are
    /// Rust `usize`, whose width is always the target's pointer width.
    fn usize_ty(&self) -> IntType<'ctx> {
        self.context.ptr_sized_int_type(self.target_data, None)
    }

    /// A type's real target size and ABI alignment (Specification 016
    /// section 8.2), for a runtime allocate/deallocate call's `size`/`align`
    /// arguments.
    fn size_align(&self, ty: Ty) -> (u64, u64) {
        let llvm_ty = self.ty(ty);
        (
            self.target_data.get_abi_size(&llvm_ty),
            u64::from(self.target_data.get_abi_alignment(&llvm_ty)),
        )
    }

    /// Declares the runtime allocator import on first use, mirroring
    /// `Codegen::print_import`.
    fn alloc_import(&self) -> FunctionValue<'ctx> {
        let symbol = "snacc_alloc";
        self.module.get_function(symbol).unwrap_or_else(|| {
            let usize_ty = self.usize_ty();
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            declare(
                self.context,
                self.module,
                symbol,
                ptr_ty.fn_type(&[usize_ty.into(), usize_ty.into()], false),
                None,
            )
        })
    }

    /// Declares the runtime deallocator import on first use, mirroring
    /// `Codegen::print_import`.
    fn dealloc_import(&self) -> FunctionValue<'ctx> {
        let symbol = "snacc_dealloc";
        self.module.get_function(symbol).unwrap_or_else(|| {
            let usize_ty = self.usize_ty();
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            declare(
                self.context,
                self.module,
                symbol,
                self.context
                    .void_type()
                    .fn_type(&[ptr_ty.into(), usize_ty.into(), usize_ty.into()], false),
                None,
            )
        })
    }

    /// Calls the runtime deallocator for `ptr`, sized and aligned for
    /// `pointee` (Specification 016 section 8.1: a box releases its
    /// allocation on destruction).
    fn call_dealloc(&self, ptr: PointerValue<'ctx>, pointee: Ty) -> Result<(), String> {
        let (size, align) = self.size_align(pointee);
        let usize_ty = self.usize_ty();
        let dealloc_fn = self.dealloc_import();
        self.invoke(
            dealloc_fn,
            &[
                ptr.into(),
                usize_ty.const_int(size, false).into(),
                usize_ty.const_int(align, false).into(),
            ],
        )?;
        Ok(())
    }

    /// Follows zero or more `Box<T>` layers from an address `ptr` whose
    /// current content is a value of `ty`, loading each box's stored pointer
    /// value in turn, until `ty` is no longer `Ty::Box` (Specification 016
    /// section 4.3). Mirrors `deref_box` in `checker.rs` at the value level:
    /// that function decides *how many* layers automatic access crosses; this
    /// one performs each crossing as a genuine load, since a box field or
    /// local's own storage holds a pointer that must be read to reach its
    /// pointee's real (heap) address.
    fn deref_box_ptr(
        &self,
        mut ptr: PointerValue<'ctx>,
        mut ty: Ty,
    ) -> Result<(PointerValue<'ctx>, Ty), String> {
        while let Ty::Box(id) = ty {
            ptr = self
                .builder
                .build_load(self.context.ptr_type(AddressSpace::default()), ptr, "deref")
                .map_err(|error| error.to_string())?
                .into_pointer_value();
            ty = self.box_pointee(id);
        }
        Ok((ptr, ty))
    }

    fn current_function(&self) -> FunctionValue<'ctx> {
        self.builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .expect("lowering always occurs inside a function")
    }

    /// Lowers one function or method body and its return. `param_drops` is
    /// the checked cleanup plan's by-value-parameter obligations
    /// (Specification 016 section 8.1), run after the body's own locals
    /// (already handled inside `self.block` via `block.drops`) and after the
    /// result value itself has been fully computed, so a parameter returned
    /// by value is never destroyed out from under its own return.
    fn body(
        &self,
        env: &mut Env<'ctx>,
        block: &TBlock,
        result: Option<Ty>,
        param_drops: &[Place],
    ) -> Result<(), String> {
        let mut loops = Vec::new();
        let (value, terminated) = self.block(env, &mut loops, block)?;
        if terminated {
            return Ok(());
        }
        self.drop_places(env, param_drops)?;
        match (result, value) {
            (Some(_), Some(value)) => self.builder.build_return(Some(&value)),
            (None, _) => self.builder.build_return(None),
            (Some(_), None) => {
                return Err(internal("a declaration with a result produced no value"));
            }
        }
        .map_err(|error| error.to_string())?;
        Ok(())
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

    /// Gives a value compiler-owned addressable storage, so a read-only method
    /// call on a temporary still receives a receiver pointer
    /// (Specification 010 section 15.3).
    fn materialize(
        &self,
        value: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let slot = self.entry_alloca(value.get_type(), name)?;
        self.builder
            .build_store(slot, value)
            .map_err(|error| error.to_string())?;
        Ok(slot)
    }

    /// The address of a place, or `None` when its root is an SSA value with no
    /// addressable storage of its own. Field selectors lower to GEPs
    /// (Specification 010 section 15.3), and a `Box<T>` step along the way
    /// lowers to a load through the box's own stored pointer, crossing as
    /// many layers as `place.path` requires before each field selector --
    /// exactly mirroring `walk_fields`/`deref_box`'s automatic dereference in
    /// `checker.rs` (Specification 016 section 4.3).
    ///
    /// A caller occasionally wants a place dereferenced *beyond* what
    /// `place.path` alone implies -- a union/sum type-test subject or a
    /// `Box<T>`-to-`Ref<T>` lending argument, both of which the checker
    /// resolves by leaving `path` untouched but overwriting `place.ty` itself
    /// to the already-dereferenced type (see `check_arm_condition` and
    /// `check_reference_arg`). The trailing check below catches up to that by
    /// dereferencing further whenever the path walk's own natural result
    /// still disagrees with `place.ty`.
    fn place_ptr(
        &self,
        env: &Env<'ctx>,
        place: &Place,
    ) -> Result<Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>, String> {
        let Some(slot) = lookup(env, root_name(&place.root)) else {
            return Err(internal("a checked place root is not in scope"));
        };
        let mut ty = place.root_ty;
        let mut ptr = match slot {
            Slot::Mutable(ptr) => ptr,
            Slot::Value(value) => {
                if place.path.is_empty() && place.ty == ty {
                    // Nothing beyond the root's own SSA value is wanted; it
                    // has no address of its own to hand back.
                    return Ok(None);
                }
                let Ty::Box(id) = ty else {
                    // A non-box SSA root (an immutable struct local, say)
                    // still has no address; `place_value`'s `extract_value`
                    // fallback reads through it instead.
                    return Ok(None);
                };
                // The box's own SSA value already *is* the address of real
                // (heap) storage for its pointee (Specification 016 section
                // 4.3), unlike a `Slot::Mutable` root's address, which holds
                // a box pointer that still needs loading -- so this peels
                // exactly the outer layer "for free" before the loop below
                // (which only ever performs genuine loads) continues through
                // any further nested layers.
                ty = self.box_pointee(id);
                value.into_pointer_value()
            }
        };
        for &index in &place.path {
            (ptr, ty) = self.deref_box_ptr(ptr, ty)?;
            ptr = self
                .builder
                .build_struct_gep(self.struct_ty(ty)?, ptr, index as u32, "field")
                .map_err(|error| error.to_string())?;
            ty = self.field_ty(ty, index)?;
        }
        if ty != place.ty {
            (ptr, ty) = self.deref_box_ptr(ptr, ty)?;
            debug_assert_eq!(
                ty, place.ty,
                "a place's automatic dereference did not land on its own checked type"
            );
        }
        Ok(Some((ptr, self.ty(place.ty))))
    }

    /// Reads a place's current value.
    fn place_value(&self, env: &Env<'ctx>, place: &Place) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some((ptr, ty)) = self.place_ptr(env, place)? {
            return self
                .builder
                .build_load(ty, ptr, root_name(&place.root))
                .map_err(|error| error.to_string());
        }
        let Some(Slot::Value(mut value)) = lookup(env, root_name(&place.root)) else {
            return Err(internal("a checked place root is not in scope"));
        };
        for &index in &place.path {
            value = self
                .builder
                .build_extract_value(as_struct(value)?, index as u32, "field")
                .map_err(|error| error.to_string())?;
        }
        Ok(value)
    }

    /// Reads one LLVM field of a union place: index 0 is the tag, index
    /// `tag + 1` is that member's storage. A member field is only ever read
    /// here on a control-flow edge where its tag already matched.
    fn union_field(
        &self,
        env: &Env<'ctx>,
        place: &Place,
        index: u32,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match self.place_ptr(env, place)? {
            Some((ptr, _)) => {
                let field = self
                    .builder
                    .build_struct_gep(self.struct_ty(place.ty)?, ptr, index, name)
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_load(ty, field, name)
                    .map_err(|error| error.to_string())
            }
            None => {
                let value = self.place_value(env, place)?;
                self.builder
                    .build_extract_value(as_struct(value)?, index, name)
                    .map_err(|error| error.to_string())
            }
        }
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
        // Specification 016 section 8.1: cleanup runs on every supported
        // *normal* edge leaving an owning scope -- a block that already
        // terminated (an unreachable fall-through, a `break`) leaves through
        // neither, so no drop instruction is appended after its terminator.
        if !terminated {
            self.drop_places(env, &block.drops)?;
        }
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
                    let ty = self.ty(*ty);
                    let slot = self.entry_alloca(ty, name)?;
                    self.builder
                        .build_store(slot, value)
                        .map_err(|error| error.to_string())?;
                    env.push((name.clone(), Slot::Mutable(slot)));
                } else {
                    env.push((name.clone(), Slot::Value(value)));
                }
                Ok(false)
            }
            TStmt::Assign {
                place,
                value,
                drop_before,
            } => {
                // Specification 016 section 6.3: the right operand evaluates
                // completely before the destination is touched at all.
                let value = self.expr(env, loops, value)?;
                let Some((ptr, ty)) = self.place_ptr(env, place)? else {
                    return Err(internal("an assignment reached a place with no storage"));
                };
                if *drop_before {
                    let old = self
                        .builder
                        .build_load(ty, ptr, "old")
                        .map_err(|error| error.to_string())?;
                    self.drop_value(place.ty, old)?;
                }
                self.builder
                    .build_store(ptr, value)
                    .map_err(|error| error.to_string())?;
                Ok(false)
            }
            TStmt::MethodCall(call) => {
                self.method_call(env, loops, call)?;
                Ok(false)
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
                    let (test, bound) = self.arm_condition(env, loops, condition)?;
                    let then_block = self.context.append_basic_block(function, "if_then");
                    let next_block = self.context.append_basic_block(function, "if_next");
                    self.builder
                        .build_conditional_branch(test, then_block, next_block)
                        .map_err(|error| error.to_string())?;
                    self.builder.position_at_end(then_block);
                    let scope = env.len();
                    self.bind(env, bound)?;
                    let (_, terminated) = self.block(env, loops, body)?;
                    env.truncate(scope);
                    if !terminated {
                        self.builder
                            .build_unconditional_branch(merge)
                            .map_err(|error| error.to_string())?;
                        reaches_merge = true;
                    }
                    self.builder.position_at_end(next_block);
                }
                // The builder now sits in the block reached when no arm
                // matched: the `else` body, a direct path to the merge, or --
                // for a proven-exhaustive type-test chain -- nothing at all.
                match (&form.else_branch, form.exhaustive) {
                    (Some(body), _) => {
                        let (_, terminated) = self.block(env, loops, body)?;
                        if !terminated {
                            self.builder
                                .build_unconditional_branch(merge)
                                .map_err(|error| error.to_string())?;
                            reaches_merge = true;
                        }
                    }
                    (None, true) => {
                        self.exhausted()?;
                    }
                    (None, false) => {
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

    /// The fall-through edge of a proven-exhaustive type-test chain. Every
    /// direct member of the union was tested, so no tag can arrive here; it is
    /// reachable only if the checker's coverage proof was wrong, which `unreachable`
    /// states rather than silently producing a value.
    fn exhausted(&self) -> Result<(), String> {
        self.builder
            .build_unreachable()
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// An `if`/`elseif` arm's condition. A type test compares the tested
    /// place's stored tag against the tag the checker resolved for a named
    /// union (Specification 010 section 15.4) or lowering computes on the fly
    /// from canonical member order for an inline sum (Specification 018
    /// Phase 4 item 1). The returned binding, when present, must be loaded on
    /// the successful edge only.
    fn arm_condition<'t>(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        condition: &'t TCondition,
    ) -> Result<(IntValue<'ctx>, Option<ArmBinding<'t>>), String> {
        match condition {
            TCondition::Expr(expression) => Ok((self.condition(env, loops, expression)?, None)),
            TCondition::Test(test) => {
                let tag =
                    self.union_field(env, &test.place, 0, self.context.i32_type().into(), "tag")?;
                let expected = self
                    .context
                    .i32_type()
                    .const_int(u64::from(test.tag), false);
                let compared = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, tag.into_int_value(), expected, "is")
                    .map_err(|error| error.to_string())?;
                Ok((
                    compared,
                    test.binding.is_some().then_some(ArmBinding::Union(test)),
                ))
            }
            TCondition::SumTest(test) => {
                let member_tag = self.sum_member_tag(test.sum, test.member)?;
                let tag =
                    self.union_field(env, &test.place, 0, self.context.i32_type().into(), "tag")?;
                let expected = self
                    .context
                    .i32_type()
                    .const_int(u64::from(member_tag), false);
                let compared = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, tag.into_int_value(), expected, "is")
                    .map_err(|error| error.to_string())?;
                Ok((
                    compared,
                    test.binding.is_some().then_some(ArmBinding::Sum(test)),
                ))
            }
        }
    }

    /// Loads a successful type test's binding. Called only after the builder is
    /// positioned in the arm's `then` block, so an inactive member is never
    /// read on the failing edge.
    fn bind(&self, env: &mut Env<'ctx>, bound: Option<ArmBinding<'_>>) -> Result<(), String> {
        match bound {
            None => Ok(()),
            Some(ArmBinding::Union(test)) => {
                let Some((name, ty)) = &test.binding else {
                    return Ok(());
                };
                let value = self.union_field(env, &test.place, test.tag + 1, self.ty(*ty), name)?;
                env.push((name.clone(), Slot::Value(value)));
                Ok(())
            }
            Some(ArmBinding::Sum(test)) => {
                let Some((name, ty)) = &test.binding else {
                    return Ok(());
                };
                let member_tag = self.sum_member_tag(test.sum, test.member)?;
                let value =
                    self.union_field(env, &test.place, member_tag + 1, self.ty(*ty), name)?;
                env.push((name.clone(), Slot::Value(value)));
                Ok(())
            }
        }
    }

    fn condition(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        condition: &TExpr,
    ) -> Result<IntValue<'ctx>, String> {
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
        args: &[TArg],
    ) -> Result<inkwell::values::CallSiteValue<'ctx>, String> {
        let mut llvm_args = Vec::new();
        for arg in args {
            llvm_args.push(self.argument(env, loops, arg)?);
        }
        self.invoke(self.functions[name], &llvm_args)
    }

    /// Specification 011 section 11: a reference argument passes the address of
    /// its checked place, never a copy of its value. The checker already
    /// required a mutable root, so `place_ptr` always finds storage here.
    fn argument(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        arg: &TArg,
    ) -> Result<BasicMetadataValueEnum<'ctx>, String> {
        match arg {
            TArg::Value(value) => Ok(self.expr(env, loops, value)?.into()),
            TArg::Reference(place) => match self.place_ptr(env, place)? {
                Some((ptr, _)) => Ok(ptr.into()),
                None => Err(internal(
                    "a reference argument reached a place with no storage",
                )),
            },
        }
    }

    /// Resolves a method call receiver to the address methods actually
    /// expect: the pointee's storage after peeling every `Box<T>` layer the
    /// receiver's static type has (Specification 016 section 4.3). Unlike
    /// `place_ptr`'s own automatic dereference, a method-call receiver's
    /// `place.ty` is deliberately left un-dereferenced by the checker (see
    /// `check_method_call`'s comment on `TReceiver::Place`), so this always
    /// derefs further on top of whatever `place_ptr`/`place_value` returned,
    /// rather than relying on them to have already done it.
    ///
    /// `borrowed` is true exactly when the returned address is real
    /// caller-owned storage rather than a compiler-owned temporary the call
    /// result discards (Specification 010 section 15.3) -- a box's pointee is
    /// always real heap storage, so a `Box<T>` receiver is `borrowed` under
    /// the same rule as any other addressable place.
    fn receiver_ptr(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        receiver: &TReceiver,
    ) -> Result<(PointerValue<'ctx>, bool), String> {
        match receiver {
            TReceiver::Place(place) => match self.place_ptr(env, place)? {
                Some((ptr, _)) => {
                    let (ptr, _) = self.deref_box_ptr(ptr, place.ty)?;
                    Ok((ptr, true))
                }
                None => {
                    let value = self.place_value(env, place)?;
                    self.receiver_from_value(place.ty, value)
                }
            },
            TReceiver::Value(value, ty) => {
                let value = self.expr(env, loops, value)?;
                self.receiver_from_value(*ty, value)
            }
        }
    }

    /// A receiver read as a bare value with no place of its own. A `Box<T>`
    /// value already *is* the address of real storage, so it is peeled
    /// directly with no compiler-owned temporary of its own; anything else
    /// gets compiler-owned storage the call result discards (Specification
    /// 010 section 15.3).
    fn receiver_from_value(
        &self,
        ty: Ty,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(PointerValue<'ctx>, bool), String> {
        match ty {
            Ty::Box(id) => {
                let pointee = self.box_pointee(id);
                let (ptr, _) = self.deref_box_ptr(value.into_pointer_value(), pointee)?;
                Ok((ptr, false))
            }
            _ => Ok((self.materialize(value, SELF)?, false)),
        }
    }

    fn method_call(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        call: &TMethodCall,
    ) -> Result<inkwell::values::CallSiteValue<'ctx>, String> {
        let (receiver, borrowed) = self.receiver_ptr(env, loops, &call.receiver)?;
        // Compiler-owned storage is discarded when the call returns, so a
        // method that may assign through `self` must have reached the caller's
        // own storage. The checker enforces a mutable receiver root for exactly
        // that case; this catches the write being silently dropped if it ever
        // did not.
        if !borrowed && self.program.methods[call.method.index()].writes_receiver {
            return Err(internal(
                "a receiver-writing method call reached a receiver with no caller storage",
            ));
        }
        let mut args: Vec<BasicMetadataValueEnum> = vec![receiver.into()];
        for arg in &call.args {
            args.push(self.argument(env, loops, arg)?);
        }
        let callee = *self
            .methods
            .get(call.method.index())
            .ok_or_else(|| internal("a method call named a method that was not lowered"))?;
        self.invoke(callee, &args)
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
            let (test, bound) = self.arm_condition(env, loops, condition)?;
            let then_block = self.context.append_basic_block(function, "if_then");
            let next_block = self.context.append_basic_block(function, "if_next");
            self.builder
                .build_conditional_branch(test, then_block, next_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(then_block);
            let scope = env.len();
            self.bind(env, bound)?;
            self.branch_value(env, loops, body, merge, &mut incoming)?;
            env.truncate(scope);
            self.builder.position_at_end(next_block);
        }
        match (&form.else_branch, form.exhaustive) {
            (Some(body), _) => self.branch_value(env, loops, body, merge, &mut incoming)?,
            (None, true) => self.exhausted()?,
            (None, false) => {
                return Err(internal(
                    "a value-producing 'if' has neither an 'else' nor a proven-exhaustive chain",
                ));
            }
        }

        self.builder.position_at_end(merge);
        if incoming.is_empty() {
            return Err(internal(
                "a value-producing 'if' had no branch that produces a value",
            ));
        }
        let phi = self
            .builder
            .build_phi(self.ty(form.ty), "if_value")
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

    /// Type-directed structural equality producing an `i1`
    /// (Specification 010 sections 7.3, 8.4, and 9.2). Specification 018
    /// section 8 extends this structurally to an inline sum, reusing
    /// [`Self::equal_union`]'s tag-then-active-member strategy through
    /// [`Self::equal_sum`].
    fn equal(
        &self,
        ty: Ty,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        match ty {
            Ty::User(id) => match self.def(id) {
                // A represented type is its target at runtime, so it delegates.
                TypeDef::Represented { target, .. } => self.equal(*target, left, right),
                TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => {
                    self.equal_fields(fields, left, right)
                }
                TypeDef::Union { members, .. } => self.equal_union(members, left, right),
            },
            Ty::Sum(id) => self.equal_sum(&self.program.sums[id.index()], left, right),
            _ if is_float(ty) => self
                .builder
                .build_float_compare(
                    FloatPredicate::OEQ,
                    left.into_float_value(),
                    right.into_float_value(),
                    "eq",
                )
                .map_err(|error| error.to_string()),
            _ => self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    left.into_int_value(),
                    right.into_int_value(),
                    "eq",
                )
                .map_err(|error| error.to_string()),
        }
    }

    /// Fields compare in declaration order with short-circuiting. All values of
    /// an empty struct type are equal.
    fn equal_fields(
        &self,
        fields: &[(String, Ty)],
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let extract = |value: BasicValueEnum<'ctx>, index: usize| {
            self.builder
                .build_extract_value(as_struct(value)?, index as u32, "field")
                .map_err(|error| error.to_string())
        };
        match fields.len() {
            0 => return Ok(self.context.bool_type().const_all_ones()),
            1 => return self.equal(fields[0].1, extract(left, 0)?, extract(right, 0)?),
            _ => {}
        }
        let function = self.current_function();
        let done = self.context.append_basic_block(function, "eq_done");
        let unequal = self.context.bool_type().const_zero();
        let mut incoming: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let last = fields.len() - 1;
        for (index, (_, field_ty)) in fields.iter().enumerate() {
            let equal = self.equal(*field_ty, extract(left, index)?, extract(right, index)?)?;
            let current = self
                .builder
                .get_insert_block()
                .expect("a comparison has an insertion block");
            if index == last {
                self.builder
                    .build_unconditional_branch(done)
                    .map_err(|error| error.to_string())?;
                incoming.push((equal, current));
            } else {
                let next = self.context.append_basic_block(function, "eq_next");
                self.builder
                    .build_conditional_branch(equal, next, done)
                    .map_err(|error| error.to_string())?;
                // The short-circuit edge reaches `done` only when this field
                // differed, so it carries `false`.
                incoming.push((unequal, current));
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(done);
        self.phi_bool(&incoming)
    }

    /// Union equality compares tags first and only then the active member; no
    /// inactive member field is ever read (Specification 010 section 15.4).
    fn equal_union(
        &self,
        members: &[TypeId],
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let function = self.current_function();
        let done = self.context.append_basic_block(function, "union_eq_done");
        let matched = self
            .context
            .append_basic_block(function, "union_eq_matched");
        let left_tag = self
            .builder
            .build_extract_value(as_struct(left)?, 0, "tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let right_tag = self
            .builder
            .build_extract_value(as_struct(right)?, 0, "tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let same = self
            .builder
            .build_int_compare(IntPredicate::EQ, left_tag, right_tag, "tag_eq")
            .map_err(|error| error.to_string())?;
        let entry = self
            .builder
            .get_insert_block()
            .expect("a comparison has an insertion block");
        self.builder
            .build_conditional_branch(same, matched, done)
            .map_err(|error| error.to_string())?;
        // Different tags are unequal without touching either payload.
        let mut incoming = vec![(self.context.bool_type().const_zero(), entry)];

        self.builder.position_at_end(matched);
        let unknown = self
            .context
            .append_basic_block(function, "union_eq_unknown");
        let mut cases = Vec::new();
        for member in members {
            let tag = self.member_tag(*member)?;
            let block = self.context.append_basic_block(function, "union_eq_member");
            cases.push((
                self.context.i32_type().const_int(u64::from(tag), false),
                block,
            ));
        }
        self.builder
            .build_switch(left_tag, unknown, &cases)
            .map_err(|error| error.to_string())?;
        // A stored tag outside the union's members means construction wrote one
        // that does not exist.
        self.builder.position_at_end(unknown);
        self.exhausted()?;

        for (member, (_, block)) in members.iter().zip(cases) {
            let field = self.member_tag(*member)? + 1;
            self.builder.position_at_end(block);
            let left = self
                .builder
                .build_extract_value(as_struct(left)?, field, "member")
                .map_err(|error| error.to_string())?;
            let right = self
                .builder
                .build_extract_value(as_struct(right)?, field, "member")
                .map_err(|error| error.to_string())?;
            let equal = self.equal(Ty::User(*member), left, right)?;
            let current = self
                .builder
                .get_insert_block()
                .expect("a comparison has an insertion block");
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            incoming.push((equal, current));
        }

        self.builder.position_at_end(done);
        self.phi_bool(&incoming)
    }

    /// An inline sum's equality: identical strategy to [`Self::equal_union`]
    /// (Specification 018 section 8 reuses named-union equality unchanged),
    /// except a member's deterministic tag is its position in `members`
    /// rather than a `TypeId`'s own tag, since a sum member is not always one.
    fn equal_sum(
        &self,
        members: &[Ty],
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let function = self.current_function();
        let done = self.context.append_basic_block(function, "sum_eq_done");
        let matched = self.context.append_basic_block(function, "sum_eq_matched");
        let left_tag = self
            .builder
            .build_extract_value(as_struct(left)?, 0, "tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let right_tag = self
            .builder
            .build_extract_value(as_struct(right)?, 0, "tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let same = self
            .builder
            .build_int_compare(IntPredicate::EQ, left_tag, right_tag, "tag_eq")
            .map_err(|error| error.to_string())?;
        let entry = self
            .builder
            .get_insert_block()
            .expect("a comparison has an insertion block");
        self.builder
            .build_conditional_branch(same, matched, done)
            .map_err(|error| error.to_string())?;
        // Different tags are unequal without touching either payload.
        let mut incoming = vec![(self.context.bool_type().const_zero(), entry)];

        self.builder.position_at_end(matched);
        let unknown = self.context.append_basic_block(function, "sum_eq_unknown");
        let cases: Vec<_> = (0..members.len())
            .map(|tag| {
                (
                    self.context.i32_type().const_int(tag as u64, false),
                    self.context.append_basic_block(function, "sum_eq_member"),
                )
            })
            .collect();
        self.builder
            .build_switch(left_tag, unknown, &cases)
            .map_err(|error| error.to_string())?;
        // A stored tag outside the sum's members means construction wrote one
        // that does not exist.
        self.builder.position_at_end(unknown);
        self.exhausted()?;

        for (tag, (member, (_, block))) in members.iter().zip(cases).enumerate() {
            let field = tag as u32 + 1;
            self.builder.position_at_end(block);
            let left = self
                .builder
                .build_extract_value(as_struct(left)?, field, "member")
                .map_err(|error| error.to_string())?;
            let right = self
                .builder
                .build_extract_value(as_struct(right)?, field, "member")
                .map_err(|error| error.to_string())?;
            let equal = self.equal(*member, left, right)?;
            let current = self
                .builder
                .get_insert_block()
                .expect("a comparison has an insertion block");
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            incoming.push((equal, current));
        }

        self.builder.position_at_end(done);
        self.phi_bool(&incoming)
    }

    fn phi_bool(
        &self,
        incoming: &[(IntValue<'ctx>, BasicBlock<'ctx>)],
    ) -> Result<IntValue<'ctx>, String> {
        let phi = self
            .builder
            .build_phi(self.context.bool_type(), "eq")
            .map_err(|error| error.to_string())?;
        let edges: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
            .iter()
            .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
            .collect();
        phi.add_incoming(&edges);
        Ok(phi.as_basic_value().into_int_value())
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
            TExpr::Cast(_, _) => Err(internal("checker emitted an unsupported cast")),
            // RFC 016 Task B's `UseMode` records whether this read is a
            // consuming context for the checker's own move-availability
            // analysis; lowering reads the place identically either way.
            TExpr::Place(place, _) => self.place_value(env, place),
            TExpr::FieldRead {
                base,
                base_ty,
                index,
                ty,
            } => {
                let base_value = self.expr(env, loops, base)?;
                match base_ty {
                    // Specification 016 section 4.3: a fresh box value (not a
                    // place) reached this field access, so lowering derefs it
                    // the same number of layers `checker.rs`'s `deref_box`
                    // did to resolve the field, then reads through the
                    // resulting real address instead of extracting from an
                    // (absent) aggregate SSA value.
                    Ty::Box(id) => {
                        let pointee = self.box_pointee(*id);
                        let (ptr, struct_ty) =
                            self.deref_box_ptr(base_value.into_pointer_value(), pointee)?;
                        let field = self
                            .builder
                            .build_struct_gep(
                                self.struct_ty(struct_ty)?,
                                ptr,
                                *index as u32,
                                "field",
                            )
                            .map_err(|error| error.to_string())?;
                        self.builder
                            .build_load(self.ty(*ty), field, "field")
                            .map_err(|error| error.to_string())
                    }
                    _ => self
                        .builder
                        .build_extract_value(as_struct(base_value)?, *index as u32, "field")
                        .map_err(|error| error.to_string()),
                }
            }
            // Specification 010 section 8.2: arguments evaluate left to right in
            // written order, then land in their declared field slots.
            TExpr::Construct { type_id, fields } => {
                let mut values = Vec::with_capacity(fields.len());
                for (index, value) in fields {
                    values.push((*index as u32, self.expr(env, loops, value)?));
                }
                let mut aggregate = self.struct_ty(Ty::User(*type_id))?.const_zero();
                for (index, value) in values {
                    aggregate = self
                        .builder
                        .build_insert_value(aggregate, value, index, "field")
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                }
                Ok(aggregate.into())
            }
            // Adding or removing a represented layer is identity at runtime.
            TExpr::Represent { value, .. } => self.expr(env, loops, value),
            // Specification 010 section 15.2: construction begins from the
            // complete union's zero initializer, then writes the tag and the
            // active member. Inactive storage is deterministic, never poison.
            TExpr::Inject {
                member,
                into_union,
                value,
            } => {
                let value = self.expr(env, loops, value)?;
                let tag = self.member_tag(*member)?;
                let zeroed = self.struct_ty(Ty::User(*into_union))?.const_zero();
                let tagged = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context.i32_type().const_int(u64::from(tag), false),
                        0,
                        "tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let injected = self
                    .builder
                    .build_insert_value(tagged, value, tag + 1, "member")
                    .map_err(|error| error.to_string())?;
                Ok(injected.into_struct_value().into())
            }
            // Specification 018 section 8 and Phase 4 items 2-3 reuse
            // `TExpr::Inject`'s strategy exactly: start from the complete
            // sum's zero initializer, then write the tag and the active
            // member, so an inactive field is always deterministic. Only the
            // tag source differs -- a sum member's tag is its position in the
            // canonical member list, computed on demand, since `InjectSum`
            // never records one itself.
            TExpr::InjectSum { sum, member, value } => {
                let value = self.expr(env, loops, value)?;
                let tag = self.sum_member_tag(*sum, *member)?;
                let zeroed = self.struct_ty(Ty::Sum(*sum))?.const_zero();
                let tagged = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context.i32_type().const_int(u64::from(tag), false),
                        0,
                        "tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let injected = self
                    .builder
                    .build_insert_value(tagged, value, tag + 1, "member")
                    .map_err(|error| error.to_string())?;
                Ok(injected.into_struct_value().into())
            }
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
                if matches!(ty, Ty::Bool | Ty::Nil | Ty::User(_) | Ty::Sum(_)) {
                    return Err(internal("checker produced non-numeric arithmetic"));
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
                if ordered && matches!(operand_ty, Ty::Bool | Ty::Nil | Ty::User(_) | Ty::Sum(_)) {
                    return Err(internal(
                        "checker allowed an ordered non-numeric comparison",
                    ));
                }
                let comparison = if matches!(operand_ty, Ty::User(_) | Ty::Sum(_)) {
                    // Recursive, type-directed equality; `!=` is its negation.
                    let equal = self.equal(operand_ty, left, right)?;
                    match op {
                        CmpOp::Eq => equal,
                        _ => builder
                            .build_not(equal, "ne")
                            .map_err(|error| error.to_string())?,
                    }
                } else if is_float(operand_ty) {
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
                    builder
                        .build_float_compare(
                            predicate,
                            left.into_float_value(),
                            right.into_float_value(),
                            "compare",
                        )
                        .map_err(|error| error.to_string())?
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
                    builder
                        .build_int_compare(
                            predicate,
                            left.into_int_value(),
                            right.into_int_value(),
                            "compare",
                        )
                        .map_err(|error| error.to_string())?
                };
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
            TExpr::MethodCall(call) => {
                let call = self.method_call(env, loops, call)?;
                Ok(call
                    .try_as_basic_value()
                    .expect_basic("a checked method call expression always returns a value"))
            }
            TExpr::If(form) => self.value_if(env, loops, form),
            TExpr::Print(value, ty) => {
                let value = self.expr(env, loops, value)?;
                let function = self.print_import(*ty)?;
                self.invoke(function, &[value.into()])?;
                Ok(value)
            }
            // Specification 016 section 4.2 and 8.2: the operand evaluates
            // exactly once, the runtime allocator is sized and aligned for
            // the *pointee* type (not the box pointer itself), and the
            // evaluated value is stored into the fresh allocation before its
            // address is produced as the box's own value. The cleanup
            // obligation this allocation creates is not registered here --
            // it is whatever the checked cleanup plan already attached to
            // wherever this `box(...)` result ends up bound (a block's
            // `drops`, a function's `param_drops`, or an assignment's
            // `drop_before`), exactly like any other move-only value.
            TExpr::Box(operand, ty) => {
                let Ty::Box(id) = *ty else {
                    return Err(internal("a 'box(...)' node did not have a box result type"));
                };
                let pointee = self.box_pointee(id);
                let value = self.expr(env, loops, operand)?;
                let (size, align) = self.size_align(pointee);
                let usize_ty = self.usize_ty();
                let alloc_fn = self.alloc_import();
                let call = self.invoke(
                    alloc_fn,
                    &[
                        usize_ty.const_int(size, false).into(),
                        usize_ty.const_int(align, false).into(),
                    ],
                )?;
                let ptr = call
                    .try_as_basic_value()
                    .expect_basic("'snacc_alloc' always returns a pointer")
                    .into_pointer_value();
                self.builder
                    .build_store(ptr, value)
                    .map_err(|error| error.to_string())?;
                Ok(ptr.into())
            }
        }
    }

    /// Recursively destroys one owned value of `ty` (Specification 016
    /// section 8.1). Only ever called where the checked cleanup plan already
    /// decided `ty` is move-only -- a copyable value is never dropped -- so
    /// every arm recurses only into fields/members/pointees that are
    /// themselves move-only, skipping a copyable one entirely rather than
    /// re-deriving that fact from scratch at each level.
    fn drop_value(&self, ty: Ty, value: BasicValueEnum<'ctx>) -> Result<(), String> {
        match ty {
            Ty::Box(id) => {
                let pointee = self.box_pointee(id);
                let ptr = value.into_pointer_value();
                if is_move_only(self.program, pointee) {
                    let pointee_ty = self.ty(pointee);
                    let loaded = self
                        .builder
                        .build_load(pointee_ty, ptr, "boxed")
                        .map_err(|error| error.to_string())?;
                    self.drop_value(pointee, loaded)?;
                }
                self.call_dealloc(ptr, pointee)
            }
            Ty::User(id) => match self.def(id) {
                TypeDef::Represented { target, .. } => self.drop_value(*target, value),
                TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => {
                    // Specification 016 section 8.1: a struct drops its
                    // move-only fields in source declaration order.
                    for (index, (_, field_ty)) in fields.iter().enumerate() {
                        if is_move_only(self.program, *field_ty) {
                            let field = self
                                .builder
                                .build_extract_value(as_struct(value)?, index as u32, "field")
                                .map_err(|error| error.to_string())?;
                            self.drop_value(*field_ty, field)?;
                        }
                    }
                    Ok(())
                }
                TypeDef::Union { members, .. } => self.drop_union(members, value),
            },
            Ty::Sum(id) => {
                let members = self.program.sums[id.index()].clone();
                self.drop_sum(&members, value)
            }
            // A caller only ever reaches this function for a move-only type
            // (Specification 016 section 5.3 composes structurally, so a
            // scalar is never move-only itself); kept as a safe no-op rather
            // than an internal-error panic since a struct/union/sum's field
            // loop above deliberately does not pre-filter its recursive call
            // by scalar-ness before checking `is_move_only`.
            _ => Ok(()),
        }
    }

    /// A union's drop (Specification 016 section 8.1: "a union drops only its
    /// active payload"): the active member is not statically known here, so
    /// this dispatches on the union's own runtime tag, mirroring
    /// `Codegen::equal_union`'s tag-then-member-block shape exactly, except
    /// each block runs a drop (a side effect) rather than producing a phi
    /// value.
    fn drop_union(&self, members: &[TypeId], value: BasicValueEnum<'ctx>) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(as_struct(value)?, 0, "tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let done = self.context.append_basic_block(function, "drop_done");
        let unknown = self.context.append_basic_block(function, "drop_unknown");
        let mut cases = Vec::with_capacity(members.len());
        for member in members {
            let tag_value = self.member_tag(*member)?;
            let block = self.context.append_basic_block(function, "drop_member");
            cases.push((
                self.context
                    .i32_type()
                    .const_int(u64::from(tag_value), false),
                block,
            ));
        }
        self.builder
            .build_switch(tag, unknown, &cases)
            .map_err(|error| error.to_string())?;
        // A stored tag outside the union's members means construction wrote
        // one that does not exist.
        self.builder.position_at_end(unknown);
        self.exhausted()?;

        for (member, (_, block)) in members.iter().zip(&cases) {
            self.builder.position_at_end(*block);
            let member_ty = Ty::User(*member);
            if is_move_only(self.program, member_ty) {
                let field_index = self.member_tag(*member)? + 1;
                let payload = self
                    .builder
                    .build_extract_value(as_struct(value)?, field_index, "member")
                    .map_err(|error| error.to_string())?;
                self.drop_value(member_ty, payload)?;
            }
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(done);
        Ok(())
    }

    /// An inline sum's drop: identical strategy to [`Self::drop_union`],
    /// except a member's deterministic tag is its position in `members`
    /// rather than a `TypeId`'s own tag (Specification 018 section 8's
    /// tag-plus-fields shape, reused unchanged).
    fn drop_sum(&self, members: &[Ty], value: BasicValueEnum<'ctx>) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(as_struct(value)?, 0, "tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let done = self.context.append_basic_block(function, "drop_done");
        let unknown = self.context.append_basic_block(function, "drop_unknown");
        let cases: Vec<_> = (0..members.len())
            .map(|tag| {
                (
                    self.context.i32_type().const_int(tag as u64, false),
                    self.context.append_basic_block(function, "drop_member"),
                )
            })
            .collect();
        self.builder
            .build_switch(tag, unknown, &cases)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(unknown);
        self.exhausted()?;

        for (index, (member, (_, block))) in members.iter().zip(cases).enumerate() {
            self.builder.position_at_end(block);
            if is_move_only(self.program, *member) {
                let payload = self
                    .builder
                    .build_extract_value(as_struct(value)?, index as u32 + 1, "member")
                    .map_err(|error| error.to_string())?;
                self.drop_value(*member, payload)?;
            }
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Runs the checked cleanup plan's drops (Specification 016 section 8.1),
    /// in the order the checker already put them in (reverse declaration
    /// order): loads each place's current value, then destroys it.
    fn drop_places(&self, env: &Env<'ctx>, drops: &[Place]) -> Result<(), String> {
        for place in drops {
            let value = self.place_value(env, place)?;
            self.drop_value(place.ty, value)?;
        }
        Ok(())
    }
}

/// Mirrors `Types::is_move_only` (Specification 016 section 5.3) using only
/// the lowering-time `Program` snapshot, since the backend does not carry the
/// checker's own `Types` table. Recursion only follows by-value fields/
/// members, which the checker's layout-cycle check already proved acyclic,
/// and a `Box<T>` edge short-circuits to `true` without recursing into its
/// pointee, so this needs no memoization -- the same reasoning
/// `Types::is_move_only` itself relies on.
fn is_move_only(program: &Program, ty: Ty) -> bool {
    match ty {
        Ty::Box(_) => true,
        Ty::User(id) => match &program.types[id.index()] {
            TypeDef::Represented { target, .. } => is_move_only(program, *target),
            TypeDef::Struct { fields, .. } | TypeDef::UnionMember { fields, .. } => {
                fields.iter().any(|(_, ty)| is_move_only(program, *ty))
            }
            TypeDef::Union { members, .. } => members
                .iter()
                .any(|member| is_move_only(program, Ty::User(*member))),
        },
        Ty::Sum(id) => program.sums[id.index()]
            .iter()
            .any(|member| is_move_only(program, *member)),
        _ => false,
    }
}

/// The environment key for a place root. `self` is a reserved word, so the
/// receiver never collides with a declared local.
fn root_name(root: &PlaceRoot) -> &str {
    match root {
        PlaceRoot::Local(name) => name,
        PlaceRoot::SelfRef => SELF,
    }
}

fn as_struct(value: BasicValueEnum<'_>) -> Result<StructValue<'_>, String> {
    match value {
        BasicValueEnum::StructValue(structure) => Ok(structure),
        _ => Err(internal("a field read reached a value without fields")),
    }
}

fn lookup<'ctx>(env: &Env<'ctx>, name: &str) -> Option<Slot<'ctx>> {
    env.iter()
        .rev()
        .find(|(bound, _)| bound == name)
        .map(|(_, slot)| *slot)
}
