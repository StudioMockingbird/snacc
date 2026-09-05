use crate::Optimization;
use crate::semantics::checker::{
    ArithOp, CmpOp, LogicalOp, Place, PlaceRoot, Program, TArg, TBlock, TCleanup, TCondition,
    TExpr, TMethodCall, TParam, TReceiver, TStmt, TSumTypeTest, TTypeTest, TValueIf, Ty,
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
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue, FunctionValue, IntValue,
    PointerValue, StructValue,
};
use inkwell::{FloatPredicate, IntPredicate, OptimizationLevel};
use std::collections::{HashMap, HashSet};

/// Marks a backend failure that is a compiler bug rather than a property of the
/// program. Specification 010 section 19 phase 5 step 7 requires an LLVM
/// verifier failure -- and, by the same reasoning, every "the checker promised
/// this" violation -- to be classified as an internal compiler error rather
/// than an ordinary backend diagnostic.
pub const INTERNAL_ERROR: &str = "internal compiler error: ";

fn internal(message: impl std::fmt::Display) -> String {
    format!("{INTERNAL_ERROR}{message}")
}

fn concat_part_tag(ty: Ty) -> Result<u64, String> {
    match ty {
        Ty::Int64 => Ok(1),
        Ty::Byte => Ok(2),
        Ty::UInt16 => Ok(3),
        Ty::UInt32 => Ok(4),
        Ty::UInt64 => Ok(5),
        Ty::Float32 => Ok(6),
        Ty::Float64 => Ok(7),
        Ty::Bool => Ok(8),
        Ty::Unicode => Ok(9),
        _ => Err(internal("unsupported scalar concatenation part")),
    }
}

fn scalar_map_symbol(prefix: &str, operation: &str) -> &'static str {
    match (prefix, operation) {
        ("u8", "contains") => "snacc_map_u8_i64_contains",
        ("u8", "insert") => "snacc_map_u8_i64_insert",
        ("u8", "delete") => "snacc_map_u8_i64_delete",
        ("u8", "index") => "snacc_map_u8_i64_index",
        ("u8", "take") => "snacc_map_u8_i64_take",
        ("u8", "reserve") => "snacc_map_u8_i64_reserve",
        ("u8", "key_at") => "snacc_map_u8_i64_key_at",
        ("u8", "value_at") => "snacc_map_u8_i64_value_at",
        ("u8", "clear") => "snacc_map_u8_i64_clear",
        ("u8", "drop") => "snacc_map_u8_i64_drop",
        ("u16", "contains") => "snacc_map_u16_i64_contains",
        ("u16", "insert") => "snacc_map_u16_i64_insert",
        ("u16", "delete") => "snacc_map_u16_i64_delete",
        ("u16", "index") => "snacc_map_u16_i64_index",
        ("u16", "take") => "snacc_map_u16_i64_take",
        ("u16", "reserve") => "snacc_map_u16_i64_reserve",
        ("u16", "key_at") => "snacc_map_u16_i64_key_at",
        ("u16", "value_at") => "snacc_map_u16_i64_value_at",
        ("u16", "clear") => "snacc_map_u16_i64_clear",
        ("u16", "drop") => "snacc_map_u16_i64_drop",
        ("u32", "contains") => "snacc_map_u32_i64_contains",
        ("u32", "insert") => "snacc_map_u32_i64_insert",
        ("u32", "delete") => "snacc_map_u32_i64_delete",
        ("u32", "index") => "snacc_map_u32_i64_index",
        ("u32", "take") => "snacc_map_u32_i64_take",
        ("u32", "reserve") => "snacc_map_u32_i64_reserve",
        ("u32", "key_at") => "snacc_map_u32_i64_key_at",
        ("u32", "value_at") => "snacc_map_u32_i64_value_at",
        ("u32", "clear") => "snacc_map_u32_i64_clear",
        ("u32", "drop") => "snacc_map_u32_i64_drop",
        ("u64", "contains") => "snacc_map_u64_i64_contains",
        ("u64", "insert") => "snacc_map_u64_i64_insert",
        ("u64", "delete") => "snacc_map_u64_i64_delete",
        ("u64", "index") => "snacc_map_u64_i64_index",
        ("u64", "take") => "snacc_map_u64_i64_take",
        ("u64", "reserve") => "snacc_map_u64_i64_reserve",
        ("u64", "key_at") => "snacc_map_u64_i64_key_at",
        ("u64", "value_at") => "snacc_map_u64_i64_value_at",
        ("u64", "clear") => "snacc_map_u64_i64_clear",
        ("u64", "drop") => "snacc_map_u64_i64_drop",
        ("i64", "contains") => "snacc_map_i64_i64_contains",
        ("i64", "insert") => "snacc_map_i64_i64_insert",
        ("i64", "delete") => "snacc_map_i64_i64_delete",
        ("i64", "index") => "snacc_map_i64_i64_index",
        ("i64", "take") => "snacc_map_i64_i64_take",
        ("i64", "reserve") => "snacc_map_i64_i64_reserve",
        ("i64", "key_at") => "snacc_map_i64_i64_key_at",
        ("i64", "value_at") => "snacc_map_i64_i64_value_at",
        ("i64", "clear") => "snacc_map_i64_i64_clear",
        ("i64", "drop") => "snacc_map_i64_i64_drop",
        ("bool", "contains") => "snacc_map_bool_i64_contains",
        ("bool", "insert") => "snacc_map_bool_i64_insert",
        ("bool", "delete") => "snacc_map_bool_i64_delete",
        ("bool", "index") => "snacc_map_bool_i64_index",
        ("bool", "take") => "snacc_map_bool_i64_take",
        ("bool", "reserve") => "snacc_map_bool_i64_reserve",
        ("bool", "key_at") => "snacc_map_bool_i64_key_at",
        ("bool", "value_at") => "snacc_map_bool_i64_value_at",
        ("bool", "clear") => "snacc_map_bool_i64_clear",
        ("bool", "drop") => "snacc_map_bool_i64_drop",
        ("unicode", "contains") => "snacc_map_unicode_i64_contains",
        ("unicode", "insert") => "snacc_map_unicode_i64_insert",
        ("unicode", "delete") => "snacc_map_unicode_i64_delete",
        ("unicode", "index") => "snacc_map_unicode_i64_index",
        ("unicode", "take") => "snacc_map_unicode_i64_take",
        ("unicode", "reserve") => "snacc_map_unicode_i64_reserve",
        ("unicode", "key_at") => "snacc_map_unicode_i64_key_at",
        ("unicode", "value_at") => "snacc_map_unicode_i64_value_at",
        ("unicode", "clear") => "snacc_map_unicode_i64_clear",
        ("unicode", "drop") => "snacc_map_unicode_i64_drop",
        _ => unreachable!("unsupported scalar map runtime symbol"),
    }
}

fn scalar_map_raw_symbol(prefix: &str, operation: &str) -> &'static str {
    match (prefix, operation) {
        ("u8", "contains") => "snacc_map_u8_raw_contains",
        ("u8", "insert") => "snacc_map_u8_raw_insert",
        ("u8", "delete") => "snacc_map_u8_raw_delete",
        ("u8", "index") => "snacc_map_u8_raw_index",
        ("u8", "take") => "snacc_map_u8_raw_take",
        ("u8", "reserve") => "snacc_map_u8_raw_reserve",
        ("u8", "key_at") => "snacc_map_u8_raw_key_at",
        ("u8", "value_at") => "snacc_map_u8_raw_value_at",
        ("u8", "clear") => "snacc_map_u8_raw_clear",
        ("u8", "drop") => "snacc_map_u8_raw_drop",
        ("u16", "contains") => "snacc_map_u16_raw_contains",
        ("u16", "insert") => "snacc_map_u16_raw_insert",
        ("u16", "delete") => "snacc_map_u16_raw_delete",
        ("u16", "index") => "snacc_map_u16_raw_index",
        ("u16", "take") => "snacc_map_u16_raw_take",
        ("u16", "reserve") => "snacc_map_u16_raw_reserve",
        ("u16", "key_at") => "snacc_map_u16_raw_key_at",
        ("u16", "value_at") => "snacc_map_u16_raw_value_at",
        ("u16", "clear") => "snacc_map_u16_raw_clear",
        ("u16", "drop") => "snacc_map_u16_raw_drop",
        ("u32", "contains") => "snacc_map_u32_raw_contains",
        ("u32", "insert") => "snacc_map_u32_raw_insert",
        ("u32", "delete") => "snacc_map_u32_raw_delete",
        ("u32", "index") => "snacc_map_u32_raw_index",
        ("u32", "take") => "snacc_map_u32_raw_take",
        ("u32", "reserve") => "snacc_map_u32_raw_reserve",
        ("u32", "key_at") => "snacc_map_u32_raw_key_at",
        ("u32", "value_at") => "snacc_map_u32_raw_value_at",
        ("u32", "clear") => "snacc_map_u32_raw_clear",
        ("u32", "drop") => "snacc_map_u32_raw_drop",
        ("u64", "contains") => "snacc_map_u64_raw_contains",
        ("u64", "insert") => "snacc_map_u64_raw_insert",
        ("u64", "delete") => "snacc_map_u64_raw_delete",
        ("u64", "index") => "snacc_map_u64_raw_index",
        ("u64", "take") => "snacc_map_u64_raw_take",
        ("u64", "reserve") => "snacc_map_u64_raw_reserve",
        ("u64", "key_at") => "snacc_map_u64_raw_key_at",
        ("u64", "value_at") => "snacc_map_u64_raw_value_at",
        ("u64", "clear") => "snacc_map_u64_raw_clear",
        ("u64", "drop") => "snacc_map_u64_raw_drop",
        ("bool", "contains") => "snacc_map_bool_raw_contains",
        ("bool", "insert") => "snacc_map_bool_raw_insert",
        ("bool", "delete") => "snacc_map_bool_raw_delete",
        ("bool", "index") => "snacc_map_bool_raw_index",
        ("bool", "take") => "snacc_map_bool_raw_take",
        ("bool", "reserve") => "snacc_map_bool_raw_reserve",
        ("bool", "key_at") => "snacc_map_bool_raw_key_at",
        ("bool", "value_at") => "snacc_map_bool_raw_value_at",
        ("bool", "clear") => "snacc_map_bool_raw_clear",
        ("bool", "drop") => "snacc_map_bool_raw_drop",
        ("unicode", "contains") => "snacc_map_unicode_raw_contains",
        ("unicode", "insert") => "snacc_map_unicode_raw_insert",
        ("unicode", "delete") => "snacc_map_unicode_raw_delete",
        ("unicode", "index") => "snacc_map_unicode_raw_index",
        ("unicode", "take") => "snacc_map_unicode_raw_take",
        ("unicode", "reserve") => "snacc_map_unicode_raw_reserve",
        ("unicode", "key_at") => "snacc_map_unicode_raw_key_at",
        ("unicode", "value_at") => "snacc_map_unicode_raw_value_at",
        ("unicode", "clear") => "snacc_map_unicode_raw_clear",
        ("unicode", "drop") => "snacc_map_unicode_raw_drop",
        ("i64", "contains") => "snacc_map_i64_raw_contains",
        ("i64", "insert") => "snacc_map_i64_raw_insert",
        ("i64", "delete") => "snacc_map_i64_raw_delete",
        ("i64", "index") => "snacc_map_i64_raw_index",
        ("i64", "take") => "snacc_map_i64_raw_take",
        ("i64", "reserve") => "snacc_map_i64_raw_reserve",
        ("i64", "key_at") => "snacc_map_i64_raw_key_at",
        ("i64", "value_at") => "snacc_map_i64_raw_value_at",
        ("i64", "clear") => "snacc_map_i64_raw_clear",
        ("i64", "drop") => "snacc_map_i64_raw_drop",
        _ => unreachable!("unsupported scalar raw map runtime symbol"),
    }
}

fn scalar_set_symbol(prefix: &str, operation: &str) -> &'static str {
    match (prefix, operation) {
        ("u8", "contains") => "snacc_set_u8_contains",
        ("u8", "insert") => "snacc_set_u8_insert",
        ("u8", "delete") => "snacc_set_u8_delete",
        ("u8", "at") => "snacc_set_u8_at",
        ("u8", "reserve") => "snacc_set_u8_reserve",
        ("u8", "clear") => "snacc_set_u8_clear",
        ("u8", "drop") => "snacc_set_u8_drop",
        ("u16", "contains") => "snacc_set_u16_contains",
        ("u16", "insert") => "snacc_set_u16_insert",
        ("u16", "delete") => "snacc_set_u16_delete",
        ("u16", "at") => "snacc_set_u16_at",
        ("u16", "reserve") => "snacc_set_u16_reserve",
        ("u16", "clear") => "snacc_set_u16_clear",
        ("u16", "drop") => "snacc_set_u16_drop",
        ("u32", "contains") => "snacc_set_u32_contains",
        ("u32", "insert") => "snacc_set_u32_insert",
        ("u32", "delete") => "snacc_set_u32_delete",
        ("u32", "at") => "snacc_set_u32_at",
        ("u32", "reserve") => "snacc_set_u32_reserve",
        ("u32", "clear") => "snacc_set_u32_clear",
        ("u32", "drop") => "snacc_set_u32_drop",
        ("u64", "contains") => "snacc_set_u64_contains",
        ("u64", "insert") => "snacc_set_u64_insert",
        ("u64", "delete") => "snacc_set_u64_delete",
        ("u64", "at") => "snacc_set_u64_at",
        ("u64", "reserve") => "snacc_set_u64_reserve",
        ("u64", "clear") => "snacc_set_u64_clear",
        ("u64", "drop") => "snacc_set_u64_drop",
        ("i64", "contains") => "snacc_set_i64_contains",
        ("i64", "insert") => "snacc_set_i64_insert",
        ("i64", "delete") => "snacc_set_i64_delete",
        ("i64", "at") => "snacc_set_i64_at",
        ("i64", "reserve") => "snacc_set_i64_reserve",
        ("i64", "clear") => "snacc_set_i64_clear",
        ("i64", "drop") => "snacc_set_i64_drop",
        ("bool", "contains") => "snacc_set_bool_contains",
        ("bool", "insert") => "snacc_set_bool_insert",
        ("bool", "delete") => "snacc_set_bool_delete",
        ("bool", "at") => "snacc_set_bool_at",
        ("bool", "reserve") => "snacc_set_bool_reserve",
        ("bool", "clear") => "snacc_set_bool_clear",
        ("bool", "drop") => "snacc_set_bool_drop",
        ("unicode", "contains") => "snacc_set_unicode_contains",
        ("unicode", "insert") => "snacc_set_unicode_insert",
        ("unicode", "delete") => "snacc_set_unicode_delete",
        ("unicode", "at") => "snacc_set_unicode_at",
        ("unicode", "reserve") => "snacc_set_unicode_reserve",
        ("unicode", "clear") => "snacc_set_unicode_clear",
        ("unicode", "drop") => "snacc_set_unicode_drop",
        _ => unreachable!("unsupported scalar set runtime symbol"),
    }
}

/// The receiver's name in a lowered method body. `self` is a reserved word, so
/// no local can collide with it.
const SELF: &str = "self";

/// Specification 009 section 5.2: the value/storage type for each scalar.
fn scalar_ty(context: &Context, ty: Ty) -> BasicTypeEnum<'_> {
    match ty {
        Ty::Float64 => context.f64_type().into(),
        Ty::Float32 => context.f32_type().into(),
        Ty::Int64 | Ty::UInt64 => context.i64_type().into(),
        Ty::UInt32 => context.i32_type().into(),
        Ty::UInt16 => context.i16_type().into(),
        Ty::Bool | Ty::Nil | Ty::Byte => context.i8_type().into(),
        Ty::Unicode => context.i32_type().into(),
        Ty::String => {
            let ptr = context.ptr_type(AddressSpace::default());
            context
                .struct_type(
                    &[
                        ptr.into(),
                        context.i64_type().into(),
                        context.i64_type().into(),
                    ],
                    false,
                )
                .into()
        }
        Ty::ViewByte | Ty::ViewUnicode => {
            let ptr = context.ptr_type(AddressSpace::default());
            context
                .struct_type(&[ptr.into(), context.i64_type().into()], false)
                .into()
        }
        Ty::Array(_) | Ty::List(_) | Ty::Map(_) | Ty::Set(_) | Ty::View(_) => {
            let ptr = context.ptr_type(AddressSpace::default());
            context
                .struct_type(
                    &[
                        ptr.into(),
                        context.i64_type().into(),
                        context.i64_type().into(),
                    ],
                    false,
                )
                .into()
        }
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
        Ty::Float64 | Ty::Float32 => true,
        Ty::Int64
        | Ty::Byte
        | Ty::UInt16
        | Ty::UInt32
        | Ty::UInt64
        | Ty::Bool
        | Ty::Nil
        | Ty::User(_)
        | Ty::Sum(_)
        | Ty::Box(_) => false,
        Ty::String
        | Ty::Unicode
        | Ty::ViewByte
        | Ty::ViewUnicode
        | Ty::Array(_)
        | Ty::List(_)
        | Ty::View(_)
        | Ty::Map(_)
        | Ty::Set(_) => false,
    }
}

/// Whether an integer type's division and ordering are unsigned. Signedness is
/// read from the checked type, never inferred from an LLVM bit width.
fn is_unsigned(ty: Ty) -> bool {
    match ty {
        Ty::Byte | Ty::UInt16 | Ty::UInt32 | Ty::UInt64 => true,
        Ty::Int64
        | Ty::Float64
        | Ty::Float32
        | Ty::Bool
        | Ty::Nil
        | Ty::User(_)
        | Ty::Sum(_)
        | Ty::Box(_) => false,
        Ty::String | Ty::ViewByte | Ty::ViewUnicode => false,
        Ty::Unicode => true,
        Ty::Array(_) | Ty::List(_) | Ty::View(_) | Ty::Map(_) | Ty::Set(_) => false,
    }
}

fn is_scalar_collection_element(ty: Ty) -> bool {
    matches!(
        ty,
        Ty::Int64
            | Ty::Byte
            | Ty::UInt16
            | Ty::UInt32
            | Ty::UInt64
            | Ty::Float32
            | Ty::Float64
            | Ty::Bool
            | Ty::Unicode
    )
}

/// The runtime import that prints one scalar.
fn print_import<'ctx>(
    context: &'ctx Context,
    ty: Ty,
) -> Result<(&'static str, Vec<BasicMetadataTypeEnum<'ctx>>), String> {
    let symbol = match ty {
        Ty::Float64 => "snacc_print_f64",
        Ty::Float32 => "snacc_print_f32",
        Ty::Int64 => "snacc_print_i64",
        Ty::Byte => "snacc_print_u8",
        Ty::UInt16 => "snacc_print_u16",
        Ty::UInt32 => "snacc_print_u32",
        Ty::UInt64 => "snacc_print_u64",
        Ty::Bool => "snacc_print_bool",
        Ty::String => {
            return Ok((
                "snacc_print_string_ptr",
                vec![context.ptr_type(AddressSpace::default()).into()],
            ));
        }
        Ty::Unicode => "snacc_print_unicode",
        Ty::ViewUnicode => "snacc_print_unicode_view",
        // Specification 010 section 14 rejects printing a user-defined type and
        // Specification 012 section 10 leaves no standalone `Nil` value at all,
        // so neither reaches lowering.
        Ty::User(_) => return Err(internal("a user-defined type reached 'print' lowering")),
        Ty::Nil => return Err(internal("a standalone 'Nil' reached 'print' lowering")),
        Ty::Sum(_) => return Err(internal("an inline sum type reached 'print' lowering")),
        // Specification 016 section 8.3 rejects direct printing of a box in
        // the checker, so this never reaches lowering.
        Ty::Box(_) => return Err(internal("a box type reached 'print' lowering")),
        Ty::ViewByte => return Err(internal("a byte view reached 'print' lowering")),
        Ty::Array(_) | Ty::List(_) | Ty::View(_) | Ty::Map(_) | Ty::Set(_) => {
            return Err(internal("a collection reached 'print' lowering"));
        }
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
/// their call sites both get the attribute -- this covers `Byte`, `UInt16`,
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
    target_data: &TargetData,
    layout: &[BasicTypeEnum<'ctx>],
    sums: &[BasicTypeEnum<'ctx>],
    leading: &[BasicMetadataTypeEnum<'ctx>],
    params: &[TParam],
    result: Option<Ty>,
    bridge: bool,
) -> FunctionType<'ctx> {
    let mut llvm_params = leading.to_vec();
    for param in params {
        match param.mode {
            ParamMode::Value if bridge && is_bridge_view_ty(param.ty) => {
                let pointer = context.ptr_type(AddressSpace::default());
                llvm_params.push(pointer.into());
                llvm_params.push(context.ptr_sized_int_type(target_data, None).into());
            }
            ParamMode::Value => llvm_params.push(llvm_ty(context, layout, sums, param.ty).into()),
            ParamMode::Reference => {
                llvm_params.push(context.ptr_type(AddressSpace::default()).into())
            }
        }
    }
    match result {
        Some(ty) => llvm_ty(context, layout, sums, ty).fn_type(&llvm_params, false),
        None => context.void_type().fn_type(&llvm_params, false),
    }
}

fn is_bridge_view_ty(ty: Ty) -> bool {
    matches!(ty, Ty::ViewByte | Ty::ViewUnicode | Ty::View(_))
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
                &target_data,
                &layout,
                &sum_layout,
                &[],
                &function.params,
                function.result,
                true,
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
                &target_data,
                &layout,
                &sum_layout,
                &[],
                &function.params,
                function.result,
                false,
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
                &target_data,
                &layout,
                &sum_layout,
                &[receiver_param],
                &method.params,
                method.result,
                false,
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

    /// Declares the single fatal runtime entry used for every invalid
    /// floating-point result. Keeping this as one import gives the NaN
    /// invariant a stable ABI surface instead of one symbol per operation.
    fn invalid_floating_operation_import(&self) -> FunctionValue<'ctx> {
        let symbol = "snacc_invalid_floating_operation";
        self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(&[], false),
                None,
            )
        })
    }

    /// Branches around one unordered floating-point check. LLVM's UNO
    /// predicate is true exactly when either operand is NaN; comparing a
    /// value with itself therefore detects NaN without changing the value.
    fn validate_float(&self, value: FloatValue<'ctx>, label: &str) -> Result<(), String> {
        let is_nan = self
            .builder
            .build_float_compare(FloatPredicate::UNO, value, value, label)
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let invalid = self.context.append_basic_block(function, "invalid_float");
        let valid = self.context.append_basic_block(function, "valid_float");
        self.builder
            .build_conditional_branch(is_nan, invalid, valid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.invoke(self.invalid_floating_operation_import(), &[])?;
        self.builder
            .build_unreachable()
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(valid);
        Ok(())
    }

    /// Calls the runtime deallocator for `ptr`, sized and aligned for
    /// `pointee` (Specification 016 section 8.1: a box releases its
    /// allocation on destruction).
    fn call_dealloc(&self, ptr: PointerValue<'ctx>, pointee: Ty) -> Result<(), String> {
        let (size, align) = self.size_align(pointee);
        self.call_raw_dealloc(
            ptr,
            self.usize_ty().const_int(size, false),
            self.usize_ty().const_int(align, false),
        )
    }

    fn call_raw_dealloc(
        &self,
        ptr: PointerValue<'ctx>,
        size: IntValue<'ctx>,
        align: IntValue<'ctx>,
    ) -> Result<(), String> {
        let dealloc_fn = self.dealloc_import();
        self.invoke(dealloc_fn, &[ptr.into(), size.into(), align.into()])?;
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
        // Specification 025: only a normal fall-through executes the block's
        // normal cleanup plan. Early exits carry their own plan on the checked
        // statement, so no second cleanup is appended after a terminator.
        if !terminated {
            self.cleanup(env, loops, &block.cleanup, None)?;
        }
        env.truncate(scope);
        Ok((value, terminated))
    }

    /// Emits one checked cleanup plan in its stored execution order. `error`
    /// is absent for a statically successful exit and present for a return
    /// whose active result tag selects success or error. Deferred calls that
    /// consume a root disarm that root's later destruction in the applicable
    /// branch.
    fn cleanup(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        cleanup: &[TCleanup],
        error: Option<IntValue<'ctx>>,
    ) -> Result<(), String> {
        let mut always_consumed = HashSet::new();
        let mut error_consumed = HashSet::new();
        for entry in cleanup {
            match entry {
                TCleanup::Drop(place) if always_consumed.contains(&place.root) => {}
                TCleanup::Drop(place) if error_consumed.contains(&place.root) => {
                    let Some(error) = error else {
                        return Err(internal(
                            "an error-only deferred move reached a successful cleanup plan",
                        ));
                    };
                    let function = self.current_function();
                    let skip = self.context.append_basic_block(function, "skip_drop");
                    let run = self.context.append_basic_block(function, "run_drop");
                    let merge = self.context.append_basic_block(function, "drop_merge");
                    self.builder
                        .build_conditional_branch(error, skip, run)
                        .map_err(|err| err.to_string())?;
                    self.builder.position_at_end(run);
                    self.drop_places(env, std::slice::from_ref(place))?;
                    self.builder
                        .build_unconditional_branch(merge)
                        .map_err(|err| err.to_string())?;
                    self.builder.position_at_end(skip);
                    self.builder
                        .build_unconditional_branch(merge)
                        .map_err(|err| err.to_string())?;
                    self.builder.position_at_end(merge);
                }
                TCleanup::Drop(place) => {
                    self.drop_places(env, std::slice::from_ref(place))?;
                }
                TCleanup::Deferred(deferred) if !deferred.on_error => {
                    self.stmt(env, loops, &deferred.call)?;
                    always_consumed.extend(deferred.consumes.iter().cloned());
                }
                TCleanup::Deferred(deferred) => {
                    let Some(error) = error else {
                        continue;
                    };
                    let function = self.current_function();
                    let run = self.context.append_basic_block(function, "defer_error");
                    let skip = self.context.append_basic_block(function, "defer_skip");
                    let merge = self.context.append_basic_block(function, "defer_merge");
                    self.builder
                        .build_conditional_branch(error, run, skip)
                        .map_err(|err| err.to_string())?;
                    self.builder.position_at_end(run);
                    self.stmt(env, loops, &deferred.call)?;
                    self.builder
                        .build_unconditional_branch(merge)
                        .map_err(|err| err.to_string())?;
                    self.builder.position_at_end(skip);
                    self.builder
                        .build_unconditional_branch(merge)
                        .map_err(|err| err.to_string())?;
                    self.builder.position_at_end(merge);
                    error_consumed.extend(deferred.consumes.iter().cloned());
                }
            }
        }
        Ok(())
    }

    fn result_error_condition(
        &self,
        value: BasicValueEnum<'ctx>,
        result: Option<Ty>,
    ) -> Result<Option<IntValue<'ctx>>, String> {
        let Some(Ty::Sum(sum)) = result else {
            return Ok(None);
        };
        let Some(error_index) = self
            .program
            .types
            .iter()
            .position(|def| def.name() == "Error")
        else {
            return Err(internal("the predeclared Error type is missing"));
        };
        let error_ty = Ty::User(TypeId(error_index as u32));
        let Some(error_tag) = self.program.sums[sum.index()]
            .iter()
            .position(|member| *member == error_ty)
        else {
            return Ok(None);
        };
        let tag = self
            .builder
            .build_extract_value(as_struct(value)?, 0, "return_tag")
            .map_err(|err| err.to_string())?
            .into_int_value();
        let is_error = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i32_type().const_int(error_tag as u64, false),
                "return_is_error",
            )
            .map_err(|err| err.to_string())?;
        Ok(Some(is_error))
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
            TStmt::SequenceIndexAssign {
                receiver,
                index,
                value,
                elem,
            } => {
                // The value is completely evaluated before the destination
                // collection is touched, matching ordinary assignment and
                // preventing a replacement from invalidating its source.
                let index = self.expr(env, loops, index)?.into_int_value();
                let value = self.expr(env, loops, value)?;
                let Some((collection_ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal(
                        "sequence indexed assignment reached a place with no storage",
                    ));
                };
                let descriptor = self
                    .builder
                    .build_load(
                        self.collection_type(),
                        collection_ptr,
                        "sequence_descriptor",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let data = self
                    .builder
                    .build_extract_value(descriptor, 0, "sequence_data")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let len = self
                    .builder
                    .build_extract_value(descriptor, 1, "sequence_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let nonnegative = self
                    .builder
                    .build_int_compare(
                        IntPredicate::SGE,
                        index,
                        self.context.i64_type().const_zero(),
                        "assignment_index_nonnegative",
                    )
                    .map_err(|error| error.to_string())?;
                let in_range = self
                    .builder
                    .build_int_compare(IntPredicate::ULT, index, len, "assignment_index_in_range")
                    .map_err(|error| error.to_string())?;
                let valid = self
                    .builder
                    .build_and(nonnegative, in_range, "assignment_index_valid")
                    .map_err(|error| error.to_string())?;
                let function = self.current_function();
                let valid_block = self
                    .context
                    .append_basic_block(function, "index_assign_valid");
                let invalid_block = self
                    .context
                    .append_basic_block(function, "index_assign_invalid");
                self.builder
                    .build_conditional_branch(valid, valid_block, invalid_block)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(invalid_block);
                self.invoke(self.collection_bounds_fail_import(), &[])?;
                self.builder
                    .build_unreachable()
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(valid_block);
                // Safety: the checker restricts this statement to an owning
                // array/list and the preceding branch proves the index range.
                let element_ptr = unsafe {
                    self.builder
                        .build_gep(self.ty(*elem), data, &[index], "assigned_element")
                }
                .map_err(|error| error.to_string())?;
                if is_move_only(self.program, *elem) {
                    let old = self
                        .builder
                        .build_load(self.ty(*elem), element_ptr, "replaced_element")
                        .map_err(|error| error.to_string())?;
                    self.drop_value(*elem, old)?;
                }
                self.builder
                    .build_store(element_ptr, value)
                    .map_err(|error| error.to_string())?;
                Ok(false)
            }
            TStmt::MethodCall(call) => {
                self.method_call(env, loops, call)?;
                Ok(false)
            }
            TStmt::ListPush {
                receiver,
                value,
                elem,
            } => {
                // Evaluate the argument before taking the receiver address,
                // matching assignment's left-to-right effect ordering.
                let value = self.expr(env, loops, value)?;
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("List.push reached a place with no storage"));
                };
                if is_scalar_collection_element(*elem) {
                    let function = self.list_push_import(*elem)?;
                    self.invoke(function, &[ptr.into(), value.into()])?;
                } else {
                    let slot = self.entry_alloca(self.ty(*elem), "list_push_value")?;
                    self.builder
                        .build_store(slot, value)
                        .map_err(|error| error.to_string())?;
                    let (size, align) = self.size_align(*elem);
                    self.invoke(
                        self.list_raw_import("push"),
                        &[
                            ptr.into(),
                            slot.into(),
                            self.usize_ty().const_int(size, false).into(),
                            self.usize_ty().const_int(align, false).into(),
                        ],
                    )?;
                }
                Ok(false)
            }
            TStmt::ListClear { receiver, elem } => {
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("List.clear reached a place with no storage"));
                };
                if is_move_only(self.program, *elem) {
                    let descriptor = self
                        .builder
                        .build_load(self.collection_type(), ptr, "list_descriptor")
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                    let data = self
                        .builder
                        .build_extract_value(descriptor, 0, "list_data")
                        .map_err(|error| error.to_string())?
                        .into_pointer_value();
                    let len = self
                        .builder
                        .build_extract_value(descriptor, 1, "list_length")
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                    self.drop_sequence_elements(data, len, *elem)?;
                }
                if is_scalar_collection_element(*elem) {
                    self.invoke(self.list_clear_import(), &[ptr.into()])?;
                } else {
                    self.invoke(self.list_raw_import("clear"), &[ptr.into()])?;
                }
                Ok(false)
            }
            TStmt::ListInsert {
                receiver,
                index,
                value,
                elem,
            } => {
                let index = self.expr(env, loops, index)?;
                let value = self.expr(env, loops, value)?;
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("List.insert reached a place with no storage"));
                };
                if is_scalar_collection_element(*elem) {
                    self.invoke(
                        self.list_insert_import(*elem)?,
                        &[ptr.into(), index.into(), value.into()],
                    )?;
                } else {
                    let slot = self.entry_alloca(self.ty(*elem), "list_insert_value")?;
                    self.builder
                        .build_store(slot, value)
                        .map_err(|error| error.to_string())?;
                    let (size, align) = self.size_align(*elem);
                    self.invoke(
                        self.list_raw_import("insert"),
                        &[
                            ptr.into(),
                            index.into(),
                            slot.into(),
                            self.usize_ty().const_int(size, false).into(),
                            self.usize_ty().const_int(align, false).into(),
                        ],
                    )?;
                }
                Ok(false)
            }
            TStmt::ListReserve {
                receiver,
                minimum,
                elem,
            } => {
                let minimum = self.expr(env, loops, minimum)?;
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("List.reserve reached a place with no storage"));
                };
                let (size, align) = self.size_align(*elem);
                self.invoke(
                    self.list_reserve_import(),
                    &[
                        ptr.into(),
                        minimum.into(),
                        self.usize_ty().const_int(size, false).into(),
                        self.usize_ty().const_int(align, false).into(),
                    ],
                )?;
                Ok(false)
            }
            TStmt::MapClear {
                receiver,
                key_ty,
                value_ty,
            } => {
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Map.clear reached a place with no storage"));
                };
                let descriptor = self
                    .builder
                    .build_load(self.collection_type(), ptr, "map_clear_descriptor")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                if self.map_uses_raw_value(*value_ty) {
                    self.drop_map_values(*key_ty, *value_ty, descriptor)?;
                }
                if self.map_uses_raw_value(*value_ty) {
                    self.invoke(self.map_raw_clear_import(*key_ty)?, &[ptr.into()])?;
                } else {
                    self.invoke(self.map_clear_import(*key_ty, *value_ty)?, &[ptr.into()])?;
                }
                Ok(false)
            }
            TStmt::MapReserve {
                receiver,
                minimum,
                key_ty,
                value_ty,
            } => {
                let minimum = self.expr(env, loops, minimum)?;
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Map.reserve reached a place with no storage"));
                };
                if self.map_uses_raw_value(*value_ty) {
                    self.invoke(
                        self.map_raw_reserve_import(*key_ty)?,
                        &[ptr.into(), minimum.into()],
                    )?;
                } else {
                    self.invoke(
                        self.map_reserve_import(*key_ty, *value_ty)?,
                        &[ptr.into(), minimum.into()],
                    )?;
                }
                Ok(false)
            }
            TStmt::SetClear { receiver, elem } => {
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Set.clear reached a place with no storage"));
                };
                self.invoke(self.set_clear_import(*elem)?, &[ptr.into()])?;
                Ok(false)
            }
            TStmt::SetReserve {
                receiver,
                minimum,
                elem,
            } => {
                let minimum = self.expr(env, loops, minimum)?;
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Set.reserve reached a place with no storage"));
                };
                self.invoke(
                    self.set_reserve_import(*elem)?,
                    &[ptr.into(), minimum.into()],
                )?;
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
            TStmt::For {
                value_name,
                value_ty,
                key_name,
                key_ty,
                iterable,
                collection_ty,
                body,
            } => {
                let collection = self.expr(env, loops, iterable)?.into_struct_value();
                let is_map = matches!(collection_ty, Ty::Map(_));
                let is_set = matches!(collection_ty, Ty::Set(_));
                let collection_slot = if is_map || is_set {
                    let slot = self.entry_alloca(self.collection_type().into(), "for_map")?;
                    self.builder
                        .build_store(slot, collection)
                        .map_err(|error| error.to_string())?;
                    Some(slot)
                } else {
                    None
                };
                let ptr = if !is_map && !is_set {
                    Some(
                        self.builder
                            .build_extract_value(collection, 0, "for_ptr")
                            .map_err(|error| error.to_string())?
                            .into_pointer_value(),
                    )
                } else {
                    None
                };
                let len = if *collection_ty == Ty::ViewUnicode {
                    let view = self.descriptor_ptr(
                        collection.into(),
                        self.view_type().into(),
                        "for_view",
                    )?;
                    self.invoke(self.view_length_import(true), &[view.into()])?
                        .try_as_basic_value()
                        .expect_basic("Unicode view length returns an integer")
                        .into_int_value()
                } else {
                    self.builder
                        .build_extract_value(collection, 1, "for_len")
                        .map_err(|error| error.to_string())?
                        .into_int_value()
                };
                let function = self.current_function();
                let condition_block = self.context.append_basic_block(function, "for_condition");
                let body_block = self.context.append_basic_block(function, "for_body");
                let exit_block = self.context.append_basic_block(function, "for_exit");
                let entry = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| internal("for loop has no insertion block"))?;
                self.builder
                    .build_unconditional_branch(condition_block)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(condition_block);
                let index = self
                    .builder
                    .build_phi(self.context.i64_type(), "for_index")
                    .map_err(|error| error.to_string())?;
                let zero = self.context.i64_type().const_zero();
                index.add_incoming(&[(&zero, entry)]);
                let current = index.as_basic_value().into_int_value();
                let more = self
                    .builder
                    .build_int_compare(IntPredicate::ULT, current, len, "for_more")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(more, body_block, exit_block)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(body_block);
                let item_slot = if is_map {
                    let key_ty = key_ty.ok_or_else(|| internal("map loop has no key type"))?;
                    if self.map_uses_raw_value(*value_ty) {
                        let out = self.entry_alloca(self.ty(*value_ty), "map_loop_value")?;
                        self.invoke(
                            self.map_raw_read_import("value_at", key_ty, false)?,
                            &[
                                collection_slot
                                    .ok_or_else(|| internal("map loop has no descriptor slot"))?
                                    .into(),
                                current.into(),
                                out.into(),
                                self.usize_ty()
                                    .const_int(self.size_align(*value_ty).0, false)
                                    .into(),
                            ],
                        )?;
                        Slot::Mutable(out)
                    } else {
                        let item = self
                            .invoke(
                                self.map_iteration_import("value_at", key_ty, *value_ty)?,
                                &[
                                    collection_slot
                                        .ok_or_else(|| internal("map loop has no descriptor slot"))?
                                        .into(),
                                    current.into(),
                                ],
                            )?
                            .try_as_basic_value()
                            .expect_basic("map iteration value lookup returns the map value");
                        Slot::Value(item)
                    }
                } else if is_set {
                    if *value_ty == Ty::String {
                        let out = self.entry_alloca(self.string_type().into(), "set_loop_value")?;
                        self.invoke(
                            self.set_iteration_import(*value_ty)?,
                            &[
                                out.into(),
                                collection_slot
                                    .ok_or_else(|| internal("set loop has no descriptor slot"))?
                                    .into(),
                                current.into(),
                            ],
                        )?;
                        Slot::Value(
                            self.builder
                                .build_load(self.string_type(), out, "set_loop_value")
                                .map_err(|error| error.to_string())?,
                        )
                    } else {
                        let item = self
                            .invoke(
                                self.set_iteration_import(*value_ty)?,
                                &[
                                    collection_slot
                                        .ok_or_else(|| internal("set loop has no descriptor slot"))?
                                        .into(),
                                    current.into(),
                                ],
                            )?
                            .try_as_basic_value()
                            .expect_basic("set iteration lookup returns the set element");
                        Slot::Value(item)
                    }
                } else if *collection_ty == Ty::ViewUnicode {
                    let scalar = self
                        .invoke(
                            self.view_at_import(true),
                            &[
                                self.descriptor_ptr(
                                    collection.into(),
                                    self.view_type().into(),
                                    "for_view_at",
                                )?
                                .into(),
                                current.into(),
                            ],
                        )?
                        .try_as_basic_value()
                        .expect_basic("Unicode view lookup returns an integer")
                        .into_int_value();
                    let item = self
                        .builder
                        .build_int_truncate(scalar, self.context.i32_type(), "for_unicode_item")
                        .map_err(|error| error.to_string())?;
                    Slot::Value(item.into())
                } else {
                    let item_ptr = unsafe {
                        self.builder.build_gep(
                            self.ty(*value_ty),
                            ptr.ok_or_else(|| internal("sequence loop has no storage"))?,
                            &[current],
                            "for_item_ptr",
                        )
                    }
                    .map_err(|error| error.to_string())?;
                    if is_move_only(self.program, *value_ty) {
                        Slot::Mutable(item_ptr)
                    } else {
                        Slot::Value(
                            self.builder
                                .build_load(self.ty(*value_ty), item_ptr, "for_item")
                                .map_err(|error| error.to_string())?,
                        )
                    }
                };
                let scope = env.len();
                if is_map {
                    let key_ty = key_ty.ok_or_else(|| internal("map loop has no key type"))?;
                    let key = if self.map_uses_raw_value(*value_ty) && key_ty == Ty::String {
                        let out = self.entry_alloca(self.string_type().into(), "map_loop_key")?;
                        self.invoke(
                            self.map_raw_read_import("key_at", key_ty, false)?,
                            &[
                                out.into(),
                                collection_slot
                                    .ok_or_else(|| internal("raw map loop has no descriptor slot"))?
                                    .into(),
                                current.into(),
                            ],
                        )?;
                        self.builder
                            .build_load(self.string_type(), out, "map_loop_key")
                            .map_err(|error| error.to_string())?
                    } else if self.map_uses_raw_value(*value_ty) {
                        self.invoke(
                            self.map_raw_read_import("key_at", key_ty, false)?,
                            &[
                                collection_slot
                                    .ok_or_else(|| internal("raw map loop has no descriptor slot"))?
                                    .into(),
                                current.into(),
                            ],
                        )?
                        .try_as_basic_value()
                        .expect_basic("raw map iteration key lookup returns the map key")
                    } else if key_ty == Ty::String {
                        let out = self.entry_alloca(self.string_type().into(), "map_loop_key")?;
                        self.invoke(
                            self.map_iteration_import("key_at", key_ty, *value_ty)?,
                            &[
                                out.into(),
                                collection_slot
                                    .ok_or_else(|| internal("map loop has no descriptor slot"))?
                                    .into(),
                                current.into(),
                            ],
                        )?;
                        self.builder
                            .build_load(self.string_type(), out, "map_loop_key")
                            .map_err(|error| error.to_string())?
                    } else {
                        self.invoke(
                            self.map_iteration_import("key_at", key_ty, *value_ty)?,
                            &[
                                collection_slot
                                    .ok_or_else(|| internal("map loop has no descriptor slot"))?
                                    .into(),
                                current.into(),
                            ],
                        )?
                        .try_as_basic_value()
                        .expect_basic("map iteration key lookup returns the map key")
                    };
                    env.push((
                        key_name
                            .clone()
                            .ok_or_else(|| internal("map loop has no key binding"))?,
                        Slot::Value(key),
                    ));
                }
                env.push((value_name.clone(), item_slot));
                let (_, terminated) = self.block(env, loops, body)?;
                env.truncate(scope);
                if !terminated {
                    let next = self
                        .builder
                        .build_int_add(
                            current,
                            self.context.i64_type().const_int(1, false),
                            "for_next",
                        )
                        .map_err(|error| error.to_string())?;
                    self.builder
                        .build_unconditional_branch(condition_block)
                        .map_err(|error| error.to_string())?;
                    index.add_incoming(&[(&next, self.builder.get_insert_block().unwrap())]);
                }
                self.builder.position_at_end(exit_block);
                Ok(false)
            }
            TStmt::Break { cleanup } => {
                self.cleanup(env, loops, cleanup, None)?;
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
            // Specification 026 section 10: the result -- if any -- is
            // evaluated and materialized before the checked cleanup plan
            // runs, so a moved-out local is read for its value here before
            // (not being) destroyed below; `drops` already excludes any root
            // the checker transferred to the caller. Shares no lowering path
            // with `body`'s own implicit-fallthrough exit beyond `drop_places`
            // itself, but produces the exact same instructions that path
            // would for the same result and cleanup facts.
            TStmt::Return {
                value,
                result,
                cleanup,
            } => {
                let value = match value {
                    Some(expression) => Some(self.expr(env, loops, expression)?),
                    None => None,
                };
                let error = match (value, *result) {
                    (Some(value), Some(result)) => {
                        self.result_error_condition(value, Some(result))?
                    }
                    _ => None,
                };
                self.cleanup(env, loops, cleanup, error)?;
                match &value {
                    Some(value) => self.builder.build_return(Some(value)),
                    None => self.builder.build_return(None),
                }
                .map_err(|error| error.to_string())?;
                Ok(true)
            }
            TStmt::ReturnOnError {
                value,
                sum,
                result,
                cleanup,
            } => {
                self.lower_return_on_error(env, loops, value, *sum, Ty::Nil, *result, cleanup)?;
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

    /// Lowers Specification 021 truthiness without changing the value's
    /// representation. Only `Bool(false)` and an active `Nil` are falsey;
    /// numeric zeroes, aggregates, boxes, strings, and views are truthy.
    fn truthiness_value(
        &self,
        ty: Ty,
        value: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        match ty {
            Ty::Bool => {
                let compared = self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        value.into_int_value(),
                        self.context.i8_type().const_zero(),
                        "truthy_bool",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_int_z_extend(compared, self.context.i8_type(), "truthy_bool_byte")
                    .map_err(|error| error.to_string())
            }
            Ty::Nil => Ok(self.context.i8_type().const_zero()),
            Ty::User(id) => match self.def(id) {
                TypeDef::Represented { target, .. } => self.truthiness_value(*target, value),
                TypeDef::Union { members, .. } => {
                    let aggregate = as_struct(value)?;
                    let tag = self
                        .builder
                        .build_extract_value(aggregate, 0, "truthy_tag")
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                    let mut result = self.context.i8_type().const_zero();
                    for (index, member) in members.iter().enumerate().rev() {
                        let member_truth = match self.def(*member) {
                            TypeDef::UnionMember { nil: true, .. } => {
                                self.context.i8_type().const_zero()
                            }
                            _ => self.context.i8_type().const_int(1, false),
                        };
                        let selected = self
                            .builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                tag,
                                self.context.i32_type().const_int(index as u64, false),
                                "truthy_member_tag",
                            )
                            .map_err(|error| error.to_string())?;
                        result = self
                            .builder
                            .build_select(selected, member_truth, result, "truthy_union")
                            .map_err(|error| error.to_string())?
                            .into_int_value();
                    }
                    Ok(result)
                }
                TypeDef::UnionMember { nil, .. } => {
                    Ok(self.context.i8_type().const_int((!nil) as u64, false))
                }
                TypeDef::Struct { .. } => Ok(self.context.i8_type().const_int(1, false)),
            },
            Ty::Sum(id) => {
                let aggregate = as_struct(value)?;
                let tag = self
                    .builder
                    .build_extract_value(aggregate, 0, "truthy_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let mut result = self.context.i8_type().const_zero();
                for (index, member) in self.program.sums[id.index()].iter().enumerate().rev() {
                    let member_value = self
                        .builder
                        .build_extract_value(aggregate, (index + 1) as u32, "truthy_member")
                        .map_err(|error| error.to_string())?;
                    let member_truth = self.truthiness_value(*member, member_value)?;
                    let selected = self
                        .builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            tag,
                            self.context.i32_type().const_int(index as u64, false),
                            "truthy_member_tag",
                        )
                        .map_err(|error| error.to_string())?;
                    result = self
                        .builder
                        .build_select(selected, member_truth, result, "truthy_sum")
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                }
                Ok(result)
            }
            _ => Ok(self.context.i8_type().const_int(1, false)),
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
        let extern_decl = self.program.externs.get(name);
        for (index, arg) in args.iter().enumerate() {
            let bridge_view = extern_decl
                .and_then(|declaration| declaration.params.get(index))
                .is_some_and(|param| param.mode == ParamMode::Value && is_bridge_view_ty(param.ty));
            if bridge_view {
                let TArg::Value(value) = arg else {
                    return Err(internal(
                        "a view bridge parameter reached lowering by reference",
                    ));
                };
                let descriptor = self.expr(env, loops, value)?.into_struct_value();
                llvm_args.push(
                    self.builder
                        .build_extract_value(descriptor, 0, "bridge_view_ptr")
                        .map_err(|error| error.to_string())?
                        .into(),
                );
                llvm_args.push(
                    self.builder
                        .build_extract_value(descriptor, 1, "bridge_view_len")
                        .map_err(|error| error.to_string())?
                        .into(),
                );
            } else {
                llvm_args.push(self.argument(env, loops, arg)?);
            }
        }
        let call = self.invoke(self.functions[name], &llvm_args)?;
        if let Some(extern_decl) = extern_decl {
            for (param, arg) in extern_decl.params.iter().zip(args) {
                if matches!(param.ty, Ty::Float32 | Ty::Float64)
                    && matches!(param.mode, ParamMode::Reference)
                {
                    let TArg::Reference(place) = arg else {
                        return Err(internal(
                            "a reference bridge parameter was lowered as a value",
                        ));
                    };
                    let Some((ptr, _)) = self.place_ptr(env, place)? else {
                        return Err(internal("a bridge reference has no storage"));
                    };
                    let value = self
                        .builder
                        .build_load(self.ty(param.ty), ptr, "bridge_float_ref")
                        .map_err(|error| error.to_string())?
                        .into_float_value();
                    self.validate_float(value, "bridge_float_ref_nan")?;
                }
            }
        }
        Ok(call)
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

    fn descriptor_ptr(
        &self,
        value: BasicValueEnum<'ctx>,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let slot = self.entry_alloca(ty, name)?;
        self.builder
            .build_store(slot, value)
            .map_err(|error| error.to_string())?;
        Ok(slot)
    }

    fn string_out_call(
        &self,
        function: FunctionValue<'ctx>,
        mut args: Vec<BasicMetadataValueEnum<'ctx>>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let out = self.entry_alloca(self.string_type().into(), name)?;
        args.insert(0, out.into());
        self.invoke(function, &args)?;
        self.builder
            .build_load(self.string_type(), out, name)
            .map_err(|error| error.to_string())
    }

    fn view_out_call(
        &self,
        function: FunctionValue<'ctx>,
        mut args: Vec<BasicMetadataValueEnum<'ctx>>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let out = self.entry_alloca(self.view_type().into(), name)?;
        args.insert(0, out.into());
        self.invoke(function, &args)?;
        self.builder
            .build_load(self.view_type(), out, name)
            .map_err(|error| error.to_string())
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

    fn string_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(
            &[ptr.into(), self.usize_ty().into(), self.usize_ty().into()],
            false,
        )
    }

    fn unsigned_to_i64(&self, value: IntValue<'ctx>, name: &str) -> Result<IntValue<'ctx>, String> {
        let width = value.get_type().get_bit_width();
        match width.cmp(&64) {
            std::cmp::Ordering::Less => self
                .builder
                .build_int_z_extend(value, self.context.i64_type(), name)
                .map_err(|error| error.to_string()),
            std::cmp::Ordering::Equal => Ok(value),
            std::cmp::Ordering::Greater => self
                .builder
                .build_int_truncate(value, self.context.i64_type(), name)
                .map_err(|error| error.to_string()),
        }
    }

    fn concat_scalar_bits(
        &self,
        value: BasicValueEnum<'ctx>,
        ty: Ty,
    ) -> Result<IntValue<'ctx>, String> {
        match ty {
            Ty::Float32 => {
                let bits = self
                    .builder
                    .build_bit_cast(value, self.context.i32_type(), "concat_f32_bits")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                self.unsigned_to_i64(bits, "concat_f32_bits64")
            }
            Ty::Float64 => Ok(self
                .builder
                .build_bit_cast(value, self.context.i64_type(), "concat_f64_bits")
                .map_err(|error| error.to_string())?
                .into_int_value()),
            Ty::Int64 | Ty::UInt64 => Ok(value.into_int_value()),
            Ty::Byte | Ty::UInt16 | Ty::UInt32 | Ty::Bool | Ty::Unicode => {
                self.unsigned_to_i64(value.into_int_value(), "concat_scalar_bits")
            }
            _ => Err(internal("unsupported scalar concatenation value")),
        }
    }

    fn view_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context
            .struct_type(&[ptr.into(), self.usize_ty().into()], false)
    }

    fn collection_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(
            &[ptr.into(), self.usize_ty().into(), self.usize_ty().into()],
            false,
        )
    }

    fn string_view_import(&self, unicode: bool) -> FunctionValue<'ctx> {
        let symbol = if unicode {
            "snacc_string_unicode_out"
        } else {
            "snacc_string_bytes_out"
        };
        self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.ptr_type(AddressSpace::default()).into(),
                    ],
                    false,
                ),
                None,
            )
        })
    }

    fn string_from_view_import(&self, utf8: bool) -> FunctionValue<'ctx> {
        let symbol = if utf8 {
            "snacc_string_from_utf8_out"
        } else {
            "snacc_string_from_view_out"
        };
        self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.ptr_type(AddressSpace::default()).into(),
                    ],
                    false,
                ),
                None,
            )
        })
    }

    fn view_length_import(&self, unicode: bool) -> FunctionValue<'ctx> {
        let symbol = if unicode {
            "snacc_view_unicode_length"
        } else {
            "snacc_view_byte_length"
        };
        self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i64_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                ),
                None,
            )
        })
    }

    fn view_equal_import(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("snacc_view_equal")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_view_equal_ptr",
                    self.context.i8_type().fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        ],
                        false,
                    ),
                    None,
                )
            })
    }

    fn view_at_import(&self, unicode: bool) -> FunctionValue<'ctx> {
        let symbol = if unicode {
            "snacc_view_unicode_at_ptr"
        } else {
            "snacc_view_byte_at_ptr"
        };
        self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i64_type().fn_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                    ],
                    false,
                ),
                None,
            )
        })
    }

    fn view_slice_import(&self, unicode: bool) -> FunctionValue<'ctx> {
        let symbol = if unicode {
            "snacc_view_unicode_slice_out"
        } else {
            "snacc_view_byte_slice_out"
        };
        self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                        self.context.i64_type().into(),
                    ],
                    false,
                ),
                None,
            )
        })
    }

    fn collection_bounds_fail_import(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("snacc_collection_bounds_fail")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_collection_bounds_fail",
                    self.context.void_type().fn_type(&[], false),
                    None,
                )
            })
    }

    fn map_symbol(&self, operation: &str, key_ty: Ty) -> Result<&'static str, String> {
        let symbol = match key_ty {
            Ty::String | Ty::ViewByte => match operation {
                "contains" => "snacc_map_string_i64_contains",
                "insert" => "snacc_map_string_i64_insert",
                "delete" => "snacc_map_string_i64_delete",
                "index" => "snacc_map_string_i64_index",
                "take" => "snacc_map_string_i64_take",
                "reserve" => "snacc_map_string_i64_reserve",
                "key_at" => "snacc_map_string_i64_key_at",
                "value_at" => "snacc_map_string_i64_value_at",
                "clear" => "snacc_map_string_i64_clear",
                "drop" => "snacc_map_string_i64_drop",
                _ => return Err(internal("unsupported map operation reached lowering")),
            },
            Ty::Byte => scalar_map_symbol("u8", operation),
            Ty::UInt16 => scalar_map_symbol("u16", operation),
            Ty::UInt32 => scalar_map_symbol("u32", operation),
            Ty::UInt64 => scalar_map_symbol("u64", operation),
            Ty::Int64 => scalar_map_symbol("i64", operation),
            Ty::Bool => scalar_map_symbol("bool", operation),
            Ty::Unicode => scalar_map_symbol("unicode", operation),
            _ => return Err(internal("unsupported map key type reached lowering")),
        };
        Ok(symbol)
    }

    fn map_raw_symbol(&self, operation: &str, key_ty: Ty) -> Result<&'static str, String> {
        if matches!(key_ty, Ty::String | Ty::ViewByte) {
            return Ok(match operation {
                "contains" => "snacc_map_string_raw_contains",
                "insert" => "snacc_map_string_raw_insert",
                "delete" => "snacc_map_string_raw_delete",
                "index" => "snacc_map_string_raw_index",
                "take" => "snacc_map_string_raw_take",
                "reserve" => "snacc_map_string_raw_reserve",
                "key_at" => "snacc_map_string_raw_key_at",
                "value_at" => "snacc_map_string_raw_value_at",
                "clear" => "snacc_map_string_raw_clear",
                "drop" => "snacc_map_string_raw_drop",
                _ => return Err(internal("unsupported raw map operation reached lowering")),
            });
        }
        let prefix = match key_ty {
            Ty::Byte => "u8",
            Ty::UInt16 => "u16",
            Ty::UInt32 => "u32",
            Ty::UInt64 => "u64",
            Ty::Int64 => "i64",
            Ty::Bool => "bool",
            Ty::Unicode => "unicode",
            _ => return Err(internal("unsupported raw map key type reached lowering")),
        };
        Ok(scalar_map_raw_symbol(prefix, operation))
    }

    fn map_uses_raw_value(&self, value_ty: Ty) -> bool {
        value_ty != Ty::Int64
    }

    fn map_contains_import(
        &self,
        key_ty: Ty,
        _value_ty: Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("contains", key_ty)?;
        let query = if matches!(key_ty, Ty::ViewByte) {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(key_ty).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(), query],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_insert_import(&self, key_ty: Ty, value_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("insert", key_ty)?;
        let key = if key_ty == Ty::String {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(key_ty).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        key,
                        self.ty(value_ty).into(),
                    ],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_delete_import(&self, key_ty: Ty, _value_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("delete", key_ty)?;
        let query = if matches!(key_ty, Ty::ViewByte) {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(key_ty).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(), query],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_index_import(&self, key_ty: Ty, value_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("index", key_ty)?;
        let query = if matches!(key_ty, Ty::ViewByte) {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(key_ty).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.ty(value_ty).fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(), query],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_take_import(&self, key_ty: Ty, value_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("take", key_ty)?;
        let query = if matches!(key_ty, Ty::ViewByte) {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(key_ty).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.ty(value_ty).fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(), query],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_iteration_import(
        &self,
        operation: &str,
        key_ty: Ty,
        value_ty: Ty,
    ) -> Result<FunctionValue<'ctx>, String> {
        let string_key = operation == "key_at" && key_ty == Ty::String;
        let symbol = if string_key {
            "snacc_map_string_i64_key_at_out"
        } else {
            self.map_symbol(operation, key_ty)?
        };
        let result = if operation == "key_at" {
            key_ty
        } else {
            value_ty
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                if string_key {
                    self.context.void_type().fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.i64_type().into(),
                        ],
                        false,
                    )
                } else {
                    self.ty(result).fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.i64_type().into(),
                        ],
                        false,
                    )
                },
                None,
            )
        }))
    }

    fn map_clear_import(&self, key_ty: Ty, value_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("clear", key_ty)?;
        let _ = value_ty;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_reserve_import(&self, key_ty: Ty, value_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("reserve", key_ty)?;
        let _ = value_ty;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                    ],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_drop_import(&self, key_ty: Ty, value_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_symbol("drop", key_ty)?;
        let _ = value_ty;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_raw_insert_import(&self, key_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_raw_symbol("insert", key_ty)?;
        let key: BasicMetadataTypeEnum<'ctx> = if key_ty == Ty::String {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(key_ty).into()
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[
                        ptr.into(),
                        key,
                        ptr.into(),
                        self.usize_ty().into(),
                        ptr.into(),
                    ],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_raw_delete_import(&self, key_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_raw_symbol("delete", key_ty)?;
        let key: BasicMetadataTypeEnum<'ctx> = if key_ty == Ty::ViewByte {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(key_ty).into()
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[ptr.into(), key, ptr.into(), self.usize_ty().into()],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_raw_contains_import(&self, key_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_raw_symbol("contains", key_ty)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let key: BasicMetadataTypeEnum<'ctx> = if key_ty == Ty::ViewByte {
            ptr.into()
        } else {
            self.ty(key_ty).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(&[ptr.into(), key], false),
                None,
            )
        }))
    }

    fn map_raw_read_import(
        &self,
        operation: &str,
        key_ty: Ty,
        result_is_index: bool,
    ) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_raw_symbol(operation, key_ty)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let signature = if operation == "key_at" {
            if key_ty == Ty::String {
                self.context.void_type().fn_type(
                    &[ptr.into(), ptr.into(), self.context.i64_type().into()],
                    false,
                )
            } else {
                self.ty(key_ty)
                    .fn_type(&[ptr.into(), self.context.i64_type().into()], false)
            }
        } else if result_is_index {
            let key: BasicMetadataTypeEnum<'ctx> = if key_ty == Ty::ViewByte {
                ptr.into()
            } else {
                self.ty(key_ty).into()
            };
            let params: Vec<BasicMetadataTypeEnum<'ctx>> =
                vec![ptr.into(), key, ptr.into(), self.usize_ty().into()];
            self.context.void_type().fn_type(&params, false)
        } else if operation == "value_at" {
            let params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
                ptr.into(),
                self.usize_ty().into(),
            ];
            self.context.void_type().fn_type(&params, false)
        } else {
            let key: BasicMetadataTypeEnum<'ctx> = if key_ty == Ty::ViewByte {
                ptr.into()
            } else {
                self.ty(key_ty).into()
            };
            let params: Vec<BasicMetadataTypeEnum<'ctx>> =
                vec![ptr.into(), key, ptr.into(), self.usize_ty().into()];
            self.context.void_type().fn_type(&params, false)
        };
        Ok(self
            .module
            .get_function(symbol)
            .unwrap_or_else(|| declare(self.context, self.module, symbol, signature, None)))
    }

    fn map_raw_clear_import(&self, key_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_raw_symbol("clear", key_ty)?;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                ),
                None,
            )
        }))
    }

    fn map_raw_reserve_import(&self, key_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_raw_symbol("reserve", key_ty)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context
                    .void_type()
                    .fn_type(&[ptr.into(), self.context.i64_type().into()], false),
                None,
            )
        }))
    }

    fn map_raw_drop_import(&self, key_ty: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.map_raw_symbol("drop", key_ty)?;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                ),
                None,
            )
        }))
    }

    fn set_symbol(&self, operation: &str, elem: Ty) -> Result<&'static str, String> {
        let string_elem = elem == Ty::String || elem == Ty::ViewByte;
        let scalar_elem = matches!(
            elem,
            Ty::Byte | Ty::UInt16 | Ty::UInt32 | Ty::UInt64 | Ty::Int64 | Ty::Bool | Ty::Unicode
        );
        if !string_elem && !scalar_elem {
            return Err(internal("unsupported set element type reached lowering"));
        }
        if scalar_elem && elem != Ty::Int64 {
            let prefix = match elem {
                Ty::Byte => "u8",
                Ty::UInt16 => "u16",
                Ty::UInt32 => "u32",
                Ty::UInt64 => "u64",
                Ty::Bool => "bool",
                Ty::Unicode => "unicode",
                _ => unreachable!("set scalar element was validated above"),
            };
            return Ok(scalar_set_symbol(prefix, operation));
        }
        Ok(match (string_elem, operation) {
            (true, "contains") => "snacc_set_string_contains",
            (true, "insert") => "snacc_set_string_insert",
            (true, "delete") => "snacc_set_string_delete",
            (true, "at") => "snacc_set_string_at",
            (true, "reserve") => "snacc_set_string_reserve",
            (true, "clear") => "snacc_set_string_clear",
            (true, "drop") => "snacc_set_string_drop",
            (false, "contains") => "snacc_set_i64_contains",
            (false, "at") => "snacc_set_i64_at",
            (false, "insert") => "snacc_set_i64_insert",
            (false, "delete") => "snacc_set_i64_delete",
            (false, "reserve") => "snacc_set_i64_reserve",
            (false, "clear") => "snacc_set_i64_clear",
            (false, "drop") => "snacc_set_i64_drop",
            _ => return Err(internal("unsupported set operation reached lowering")),
        })
    }

    fn set_contains_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.set_symbol("contains", elem)?;
        let value = if elem == Ty::String || elem == Ty::ViewByte {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(elem).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(), value],
                    false,
                ),
                None,
            )
        }))
    }

    fn set_insert_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.set_symbol("insert", elem)?;
        let value = if elem == Ty::String {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(elem).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(), value],
                    false,
                ),
                None,
            )
        }))
    }

    fn set_delete_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.set_symbol("delete", elem)?;
        let value = if elem == Ty::String || elem == Ty::ViewByte {
            self.context.ptr_type(AddressSpace::default()).into()
        } else {
            self.ty(elem).into()
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.i8_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into(), value],
                    false,
                ),
                None,
            )
        }))
    }

    fn set_iteration_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = if elem == Ty::String {
            "snacc_set_string_at_out"
        } else {
            self.set_symbol("at", elem)?
        };
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                if elem == Ty::String {
                    self.context.void_type().fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.i64_type().into(),
                        ],
                        false,
                    )
                } else {
                    self.ty(elem).fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.i64_type().into(),
                        ],
                        false,
                    )
                },
                None,
            )
        }))
    }

    fn set_clear_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.set_symbol("clear", elem)?;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                ),
                None,
            )
        }))
    }

    fn set_reserve_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.set_symbol("reserve", elem)?;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                    ],
                    false,
                ),
                None,
            )
        }))
    }

    fn set_drop_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.set_symbol("drop", elem)?;
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[self.context.ptr_type(AddressSpace::default()).into()],
                    false,
                ),
                None,
            )
        }))
    }

    fn list_push_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = match elem {
            Ty::Int64 => "snacc_list_push_i64",
            Ty::Byte => "snacc_list_push_u8",
            Ty::UInt16 => "snacc_list_push_u16",
            Ty::UInt32 => "snacc_list_push_u32",
            Ty::UInt64 => "snacc_list_push_u64",
            Ty::Float32 => "snacc_list_push_f32",
            Ty::Float64 => "snacc_list_push_f64",
            Ty::Bool => "snacc_list_push_bool",
            Ty::Unicode => "snacc_list_push_unicode",
            _ => return Err(internal("unsupported List.push element type")),
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context
                    .void_type()
                    .fn_type(&[ptr.into(), self.ty(elem).into()], false),
                None,
            )
        }))
    }

    fn list_clear_import(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.module
            .get_function("snacc_list_clear")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_list_clear",
                    self.context.void_type().fn_type(&[ptr.into()], false),
                    None,
                )
            })
    }

    fn list_scalar_symbol(&self, operation: &str, elem: Ty) -> Result<&'static str, String> {
        let symbol = match (operation, elem) {
            ("pop", Ty::Int64) => "snacc_list_pop_i64",
            ("pop", Ty::Byte) => "snacc_list_pop_u8",
            ("pop", Ty::UInt16) => "snacc_list_pop_u16",
            ("pop", Ty::UInt32) => "snacc_list_pop_u32",
            ("pop", Ty::UInt64) => "snacc_list_pop_u64",
            ("pop", Ty::Float32) => "snacc_list_pop_f32",
            ("pop", Ty::Float64) => "snacc_list_pop_f64",
            ("pop", Ty::Bool) => "snacc_list_pop_bool",
            ("pop", Ty::Unicode) => "snacc_list_pop_unicode",
            ("insert", Ty::Int64) => "snacc_list_insert_i64",
            ("insert", Ty::Byte) => "snacc_list_insert_u8",
            ("insert", Ty::UInt16) => "snacc_list_insert_u16",
            ("insert", Ty::UInt32) => "snacc_list_insert_u32",
            ("insert", Ty::UInt64) => "snacc_list_insert_u64",
            ("insert", Ty::Float32) => "snacc_list_insert_f32",
            ("insert", Ty::Float64) => "snacc_list_insert_f64",
            ("insert", Ty::Bool) => "snacc_list_insert_bool",
            ("insert", Ty::Unicode) => "snacc_list_insert_unicode",
            ("remove", Ty::Int64) => "snacc_list_remove_i64",
            ("remove", Ty::Byte) => "snacc_list_remove_u8",
            ("remove", Ty::UInt16) => "snacc_list_remove_u16",
            ("remove", Ty::UInt32) => "snacc_list_remove_u32",
            ("remove", Ty::UInt64) => "snacc_list_remove_u64",
            ("remove", Ty::Float32) => "snacc_list_remove_f32",
            ("remove", Ty::Float64) => "snacc_list_remove_f64",
            ("remove", Ty::Bool) => "snacc_list_remove_bool",
            ("remove", Ty::Unicode) => "snacc_list_remove_unicode",
            _ => return Err(internal("unsupported scalar list operation")),
        };
        Ok(symbol)
    }

    fn list_pop_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.list_scalar_symbol("pop", elem)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.ty(elem).fn_type(&[ptr.into()], false),
                None,
            )
        }))
    }

    fn list_insert_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.list_scalar_symbol("insert", elem)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.context.void_type().fn_type(
                    &[
                        ptr.into(),
                        self.context.i64_type().into(),
                        self.ty(elem).into(),
                    ],
                    false,
                ),
                None,
            )
        }))
    }

    fn list_remove_import(&self, elem: Ty) -> Result<FunctionValue<'ctx>, String> {
        let symbol = self.list_scalar_symbol("remove", elem)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        Ok(self.module.get_function(symbol).unwrap_or_else(|| {
            declare(
                self.context,
                self.module,
                symbol,
                self.ty(elem)
                    .fn_type(&[ptr.into(), self.context.i64_type().into()], false),
                None,
            )
        }))
    }

    fn list_reserve_import(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.module
            .get_function("snacc_list_reserve")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_list_reserve",
                    self.context.void_type().fn_type(
                        &[
                            ptr.into(),
                            self.context.i64_type().into(),
                            self.usize_ty().into(),
                            self.usize_ty().into(),
                        ],
                        false,
                    ),
                    None,
                )
            })
    }

    fn list_raw_import(&self, operation: &str) -> FunctionValue<'ctx> {
        let symbol = match operation {
            "push" => "snacc_list_push_raw",
            "pop" => "snacc_list_pop_raw",
            "insert" => "snacc_list_insert_raw",
            "remove" => "snacc_list_remove_raw",
            "clear" => "snacc_list_clear_raw",
            _ => unreachable!("unsupported raw list operation"),
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        let signature = match operation {
            "push" => self.context.void_type().fn_type(
                &[
                    ptr.into(),
                    ptr.into(),
                    self.usize_ty().into(),
                    self.usize_ty().into(),
                ],
                false,
            ),
            "pop" => self
                .context
                .void_type()
                .fn_type(&[ptr.into(), ptr.into(), self.usize_ty().into()], false),
            "remove" => self.context.void_type().fn_type(
                &[
                    ptr.into(),
                    self.context.i64_type().into(),
                    ptr.into(),
                    self.usize_ty().into(),
                ],
                false,
            ),
            "insert" => self.context.void_type().fn_type(
                &[
                    ptr.into(),
                    self.context.i64_type().into(),
                    ptr.into(),
                    self.usize_ty().into(),
                    self.usize_ty().into(),
                ],
                false,
            ),
            "clear" => self.context.void_type().fn_type(&[ptr.into()], false),
            _ => unreachable!(),
        };
        self.module
            .get_function(symbol)
            .unwrap_or_else(|| declare(self.context, self.module, symbol, signature, None))
    }

    fn string_equal_import(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("snacc_string_equal_ptr")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_string_equal_ptr",
                    self.context.i8_type().fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        ],
                        false,
                    ),
                    None,
                )
            })
    }

    fn string_new_import(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.module
            .get_function("snacc_string_new_out")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_string_new_out",
                    self.context
                        .void_type()
                        .fn_type(&[ptr.into(), ptr.into(), self.usize_ty().into()], false),
                    None,
                )
            })
    }

    fn string_concat_parts_import(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.module
            .get_function("snacc_string_concat_parts_out")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_string_concat_parts_out",
                    self.context
                        .void_type()
                        .fn_type(&[ptr.into(), ptr.into(), self.usize_ty().into()], false),
                    None,
                )
            })
    }

    fn string_clone_import(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("snacc_string_clone_out")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_string_clone_out",
                    self.context.void_type().fn_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.ptr_type(AddressSpace::default()).into(),
                        ],
                        false,
                    ),
                    None,
                )
            })
    }

    fn string_drop_import(&self) -> FunctionValue<'ctx> {
        self.module
            .get_function("snacc_string_drop_ptr")
            .unwrap_or_else(|| {
                declare(
                    self.context,
                    self.module,
                    "snacc_string_drop_ptr",
                    self.context.void_type().fn_type(
                        &[self.context.ptr_type(AddressSpace::default()).into()],
                        false,
                    ),
                    None,
                )
            })
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
            Ty::Array(id) | Ty::List(id) | Ty::View(id) => {
                let elem = match &self.program.collections[id.index()] {
                    crate::semantics::types::CollectionDef::Array { elem, .. }
                    | crate::semantics::types::CollectionDef::List { elem }
                    | crate::semantics::types::CollectionDef::View { elem } => *elem,
                    _ => return Err(internal("collection equality has non-sequence metadata")),
                };
                self.equal_collection(elem, left, right)
            }
            Ty::String | Ty::ViewByte | Ty::ViewUnicode => {
                let descriptor_ty = if ty == Ty::String {
                    self.string_type().into()
                } else {
                    self.view_type().into()
                };
                let left = self.descriptor_ptr(left, descriptor_ty, "equal_left")?;
                let right = self.descriptor_ptr(right, descriptor_ty, "equal_right")?;
                let equal = if ty == Ty::String {
                    self.invoke(self.string_equal_import(), &[left.into(), right.into()])?
                        .try_as_basic_value()
                        .expect_basic("string equality returns a byte")
                        .into_int_value()
                } else {
                    self.invoke(self.view_equal_import(), &[left.into(), right.into()])?
                        .try_as_basic_value()
                        .expect_basic("view equality returns a byte")
                        .into_int_value()
                };
                self.builder
                    .build_int_compare(
                        IntPredicate::NE,
                        equal,
                        self.context.i8_type().const_zero(),
                        "equal",
                    )
                    .map_err(|error| error.to_string())
            }
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

    /// Compares two same-typed sequence descriptors by length and then by
    /// increasing element index. The descriptor's third field is intentionally
    /// ignored: capacity is storage state, not sequence content.
    fn equal_collection(
        &self,
        elem: Ty,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let left = as_struct(left)?;
        let right = as_struct(right)?;
        let left_ptr = self
            .builder
            .build_extract_value(left, 0, "left_collection_ptr")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let right_ptr = self
            .builder
            .build_extract_value(right, 0, "right_collection_ptr")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let left_len = self
            .builder
            .build_extract_value(left, 1, "left_collection_len")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let right_len = self
            .builder
            .build_extract_value(right, 1, "right_collection_len")
            .map_err(|error| error.to_string())?
            .into_int_value();

        let function = self.current_function();
        let length_match = self
            .context
            .append_basic_block(function, "collection_eq_length_match");
        let loop_block = self
            .context
            .append_basic_block(function, "collection_eq_loop");
        let element_block = self
            .context
            .append_basic_block(function, "collection_eq_element");
        let all_equal = self
            .context
            .append_basic_block(function, "collection_eq_all_equal");
        let done = self
            .context
            .append_basic_block(function, "collection_eq_done");
        let entry = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("collection equality has no insertion block"))?;
        let lengths_equal = self
            .builder
            .build_int_compare(IntPredicate::EQ, left_len, right_len, "lengths_equal")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(lengths_equal, length_match, done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(length_match);
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(loop_block);
        let index = self
            .builder
            .build_phi(self.context.i64_type(), "collection_eq_index")
            .map_err(|error| error.to_string())?;
        let zero = self.context.i64_type().const_zero();
        index.add_incoming(&[(&zero, length_match)]);
        let index_value = index.as_basic_value().into_int_value();
        let more = self
            .builder
            .build_int_compare(IntPredicate::ULT, index_value, left_len, "more_elements")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(more, element_block, all_equal)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(element_block);
        let left_element_ptr = unsafe {
            self.builder
                .build_gep(self.ty(elem), left_ptr, &[index_value], "left_element_ptr")
        }
        .map_err(|error| error.to_string())?;
        let right_element_ptr = unsafe {
            self.builder.build_gep(
                self.ty(elem),
                right_ptr,
                &[index_value],
                "right_element_ptr",
            )
        }
        .map_err(|error| error.to_string())?;
        let left_element = self
            .builder
            .build_load(self.ty(elem), left_element_ptr, "left_element")
            .map_err(|error| error.to_string())?;
        let right_element = self
            .builder
            .build_load(self.ty(elem), right_element_ptr, "right_element")
            .map_err(|error| error.to_string())?;
        let equal = self.equal(elem, left_element, right_element)?;
        let mismatch = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("collection element equality has no block"))?;
        let next = self
            .context
            .append_basic_block(function, "collection_eq_next");
        self.builder
            .build_conditional_branch(equal, next, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(next);
        let next_index = self
            .builder
            .build_int_add(
                index_value,
                self.context.i64_type().const_int(1, false),
                "next_index",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next_index, next)]);

        self.builder.position_at_end(all_equal);
        let all_equal_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("collection equality completion has no block"))?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(done);
        let false_value = self.context.bool_type().const_zero();
        let true_value = self.context.bool_type().const_all_ones();
        self.phi_bool(&[
            (false_value, entry),
            (false_value, mismatch),
            (true_value, all_equal_end),
        ])
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

    /// Lowers the shared control-flow operation used by expression- and
    /// statement-form `return_on_error`. The source and enclosing result sums
    /// may differ; the error payload is therefore re-tagged into the declared
    /// result before the checked cleanup plan runs.
    fn lower_return_on_error(
        &self,
        env: &mut Env<'ctx>,
        loops: &mut Loops<'ctx>,
        value_expr: &TExpr,
        sum: SumId,
        success: Ty,
        result: Ty,
        cleanup: &[TCleanup],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.expr(env, loops, value_expr)?;
        let aggregate = as_struct(value)?;
        let tag = self
            .builder
            .build_extract_value(aggregate, 0, "error_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let error_ty = self
            .program
            .types
            .iter()
            .position(|def| def.name() == "Error")
            .map(|index| Ty::User(TypeId(index as u32)))
            .ok_or_else(|| internal("the predeclared Error type is missing"))?;
        let error_tag = self.sum_member_tag(sum, error_ty)?;
        let function = self.current_function();
        let error_block = self.context.append_basic_block(function, "return_error");
        let success_block = self.context.append_basic_block(function, "return_success");
        let merge = self.context.append_basic_block(function, "return_ok");
        let is_error = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context
                    .i32_type()
                    .const_int(u64::from(error_tag), false),
                "is_error",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(is_error, error_block, success_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(error_block);
        let error_value = if result == Ty::Sum(sum) {
            value
        } else {
            let Ty::Sum(result_sum) = result else {
                return Err(internal("return_on_error result is not an inline sum"));
            };
            let payload = self
                .builder
                .build_extract_value(aggregate, error_tag + 1, "error_payload")
                .map_err(|error| error.to_string())?;
            let zeroed = self.struct_ty(result)?.const_zero();
            let result_tag = self.sum_member_tag(result_sum, error_ty)?;
            let tagged = self
                .builder
                .build_insert_value(
                    zeroed,
                    self.context
                        .i32_type()
                        .const_int(u64::from(result_tag), false),
                    0,
                    "error_result_tag",
                )
                .map_err(|error| error.to_string())?
                .into_struct_value();
            self.builder
                .build_insert_value(tagged, payload, result_tag + 1, "error_result_payload")
                .map_err(|error| error.to_string())?
                .into_struct_value()
                .into()
        };
        self.cleanup(
            env,
            loops,
            cleanup,
            Some(self.context.bool_type().const_int(1, false)),
        )?;
        self.builder
            .build_return(Some(&error_value))
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(success_block);
        let success_value = match success {
            Ty::Nil => self.context.i8_type().const_zero().into(),
            Ty::Sum(reduced_sum) => {
                let reduced_members = self.program.sums[reduced_sum.index()].clone();
                let done = self.context.append_basic_block(function, "return_ok_sum");
                let unknown = self
                    .context
                    .append_basic_block(function, "return_ok_unknown");
                let cases: Vec<_> = reduced_members
                    .iter()
                    .map(|member| {
                        (
                            self.context.i32_type().const_int(
                                u64::from(
                                    self.sum_member_tag(sum, *member)
                                        .expect("checker selected a sum member"),
                                ),
                                false,
                            ),
                            self.context
                                .append_basic_block(function, "return_ok_member"),
                        )
                    })
                    .collect();
                self.builder
                    .build_switch(tag, unknown, &cases)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(unknown);
                self.exhausted()?;
                let zeroed = self.struct_ty(Ty::Sum(reduced_sum))?.const_zero();
                let mut incoming = Vec::with_capacity(cases.len());
                for (member, (_, member_block)) in reduced_members.iter().zip(cases) {
                    self.builder.position_at_end(member_block);
                    let source_tag = self.sum_member_tag(sum, *member)?;
                    let payload = if *member == Ty::Nil {
                        self.context.i8_type().const_zero().into()
                    } else {
                        self.builder
                            .build_extract_value(aggregate, source_tag + 1, "success")
                            .map_err(|error| error.to_string())?
                    };
                    let reduced_tag = self.sum_member_tag(reduced_sum, *member)?;
                    let tagged = self
                        .builder
                        .build_insert_value(
                            zeroed,
                            self.context
                                .i32_type()
                                .const_int(u64::from(reduced_tag), false),
                            0,
                            "success_tag",
                        )
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                    let injected = self
                        .builder
                        .build_insert_value(tagged, payload, reduced_tag + 1, "success_payload")
                        .map_err(|error| error.to_string())?;
                    let current = self
                        .builder
                        .get_insert_block()
                        .ok_or_else(|| internal("return_on_error sum arm has no block"))?;
                    self.builder
                        .build_unconditional_branch(done)
                        .map_err(|error| error.to_string())?;
                    incoming.push((injected.into_struct_value(), current));
                }
                self.builder.position_at_end(done);
                let phi = self
                    .builder
                    .build_phi(self.struct_ty(Ty::Sum(reduced_sum))?, "success_sum")
                    .map_err(|error| error.to_string())?;
                let incoming: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                    .iter()
                    .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
                    .collect();
                phi.add_incoming(&incoming);
                phi.as_basic_value()
            }
            _ => self
                .builder
                .build_extract_value(
                    aggregate,
                    self.sum_member_tag(sum, success)? + 1,
                    "success_value",
                )
                .map_err(|error| error.to_string())?,
        };
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        let success_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("return_on_error success block has no end"))?;
        self.builder.position_at_end(merge);
        let phi = self
            .builder
            .build_phi(self.ty(success), "success_value")
            .map_err(|error| error.to_string())?;
        phi.add_incoming(&[(&success_value, success_end)]);
        Ok(phi.as_basic_value())
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
                NumLiteral::F64(value) => self.context.f64_type().const_float(*value).into(),
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
            TExpr::Unicode(value) => Ok(self
                .context
                .i32_type()
                .const_int(u64::from(*value), false)
                .into()),
            TExpr::StringLiteral(value) => {
                let global = self
                    .builder
                    .build_global_string_ptr(value, "string_literal")
                    .map_err(|error| error.to_string())?;
                let out = self.entry_alloca(self.string_type().into(), "string_literal_value")?;
                self.invoke(
                    self.string_new_import(),
                    &[
                        out.into(),
                        global.as_pointer_value().into(),
                        self.usize_ty().const_int(value.len() as u64, false).into(),
                    ],
                )?;
                Ok(self
                    .builder
                    .build_load(self.string_type(), out, "string_literal_value")
                    .map_err(|error| error.to_string())?)
            }
            TExpr::StringClone(value) => {
                let value = self.expr(env, loops, value)?;
                let value =
                    self.descriptor_ptr(value, self.string_type().into(), "string_clone")?;
                self.string_out_call(
                    self.string_clone_import(),
                    vec![value.into()],
                    "string_clone_value",
                )
            }
            TExpr::StringConcat(parts) => {
                if parts.is_empty() {
                    let empty = self
                        .builder
                        .build_global_string_ptr("", "empty_interpolation")
                        .map_err(|error| error.to_string())?;
                    return self.string_out_call(
                        self.string_new_import(),
                        vec![
                            empty.as_pointer_value().into(),
                            self.usize_ty().const_zero().into(),
                        ],
                        "empty_interpolation_value",
                    );
                }
                let i64_type = self.context.i64_type();
                let erased_type = self
                    .context
                    .struct_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false);
                let array_type = erased_type.array_type(parts.len() as u32);
                let storage = self.entry_alloca(array_type.into(), "string_concat_parts")?;
                let zero = self.context.i32_type().const_zero();
                let mut owned_temporaries = Vec::new();

                for (index, part) in parts.iter().enumerate() {
                    let (tag, first, second) = if let TExpr::StringLiteral(text) = &part.value {
                        let global = self
                            .builder
                            .build_global_string_ptr(text, "string_concat_literal")
                            .map_err(|error| error.to_string())?;
                        let pointer = self
                            .builder
                            .build_ptr_to_int(
                                global.as_pointer_value(),
                                i64_type,
                                "string_concat_literal_ptr",
                            )
                            .map_err(|error| error.to_string())?;
                        (0, pointer, i64_type.const_int(text.len() as u64, false))
                    } else {
                        let value = self.expr(env, loops, &part.value)?;
                        match part.ty {
                            Ty::String => {
                                if !matches!(&part.value, TExpr::Place(_, _)) {
                                    owned_temporaries.push(value);
                                }
                                let descriptor = value.into_struct_value();
                                let pointer = self
                                    .builder
                                    .build_extract_value(descriptor, 0, "string_concat_text_ptr")
                                    .map_err(|error| error.to_string())?
                                    .into_pointer_value();
                                let length = self
                                    .builder
                                    .build_extract_value(descriptor, 1, "string_concat_text_len")
                                    .map_err(|error| error.to_string())?
                                    .into_int_value();
                                let pointer = self
                                    .builder
                                    .build_ptr_to_int(pointer, i64_type, "string_concat_text_bits")
                                    .map_err(|error| error.to_string())?;
                                let length =
                                    self.unsigned_to_i64(length, "string_concat_text_len64")?;
                                (0, pointer, length)
                            }
                            Ty::ViewUnicode => {
                                let descriptor = value.into_struct_value();
                                let pointer = self
                                    .builder
                                    .build_extract_value(descriptor, 0, "string_concat_view_ptr")
                                    .map_err(|error| error.to_string())?
                                    .into_pointer_value();
                                let length = self
                                    .builder
                                    .build_extract_value(descriptor, 1, "string_concat_view_len")
                                    .map_err(|error| error.to_string())?
                                    .into_int_value();
                                let pointer = self
                                    .builder
                                    .build_ptr_to_int(pointer, i64_type, "string_concat_view_bits")
                                    .map_err(|error| error.to_string())?;
                                let length =
                                    self.unsigned_to_i64(length, "string_concat_view_len64")?;
                                (0, pointer, length)
                            }
                            ty => (
                                concat_part_tag(ty)?,
                                self.concat_scalar_bits(value, ty)?,
                                i64_type.const_zero(),
                            ),
                        }
                    };
                    let mut erased = erased_type.const_zero();
                    erased = self
                        .builder
                        .build_insert_value(
                            erased,
                            i64_type.const_int(tag, false),
                            0,
                            "string_concat_tag",
                        )
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                    erased = self
                        .builder
                        .build_insert_value(erased, first, 1, "string_concat_first")
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                    erased = self
                        .builder
                        .build_insert_value(erased, second, 2, "string_concat_second")
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                    let slot = unsafe {
                        self.builder.build_in_bounds_gep(
                            array_type,
                            storage,
                            &[zero, self.context.i32_type().const_int(index as u64, false)],
                            "string_concat_part",
                        )
                    }
                    .map_err(|error| error.to_string())?;
                    self.builder
                        .build_store(slot, erased)
                        .map_err(|error| error.to_string())?;
                }

                let result = self.string_out_call(
                    self.string_concat_parts_import(),
                    vec![
                        storage.into(),
                        self.usize_ty().const_int(parts.len() as u64, false).into(),
                    ],
                    "string_concat_value",
                )?;
                for temporary in owned_temporaries {
                    self.drop_value(Ty::String, temporary)?;
                }
                Ok(result)
            }
            TExpr::StringFromUnicode(value) => {
                let value = self.expr(env, loops, value)?;
                let value =
                    self.descriptor_ptr(value, self.view_type().into(), "string_from_view")?;
                self.string_out_call(
                    self.string_from_view_import(false),
                    vec![value.into()],
                    "string_from_view_value",
                )
            }
            TExpr::StringFromUtf8(value, sum) => {
                let value = self.expr(env, loops, value)?;
                let value =
                    self.descriptor_ptr(value, self.view_type().into(), "string_from_utf8")?;
                let string = self
                    .string_out_call(
                        self.string_from_view_import(true),
                        vec![value.into()],
                        "string_from_utf8_value",
                    )?
                    .into_struct_value();
                let valid = self
                    .builder
                    .build_is_not_null(
                        self.builder
                            .build_extract_value(string, 0, "utf8_ptr")
                            .map_err(|error| error.to_string())?
                            .into_pointer_value(),
                        "utf8_valid",
                    )
                    .map_err(|error| error.to_string())?;
                let nil_tag = self.sum_member_tag(*sum, Ty::Nil)?;
                let string_tag = self.sum_member_tag(*sum, Ty::String)?;
                let zeroed = self.struct_ty(Ty::Sum(*sum))?.const_zero();
                let nil = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context.i32_type().const_int(u64::from(nil_tag), false),
                        0,
                        "utf8_nil",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let tagged = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context
                            .i32_type()
                            .const_int(u64::from(string_tag), false),
                        0,
                        "utf8_string",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let valid_value = self
                    .builder
                    .build_insert_value(tagged, string, string_tag + 1, "utf8_payload")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                Ok(self
                    .builder
                    .build_select(valid, valid_value, nil, "utf8_result")
                    .map_err(|error| error.to_string())?
                    .into())
            }
            TExpr::ViewFromString(value, ty) => {
                let value = self.expr(env, loops, value)?;
                let value =
                    self.descriptor_ptr(value, self.string_type().into(), "string_view_input")?;
                Ok(self.view_out_call(
                    self.string_view_import(*ty == Ty::ViewUnicode),
                    vec![value.into()],
                    "string_view_value",
                )?)
            }
            TExpr::ViewLength(value, ty) => {
                let value = self.expr(env, loops, value)?;
                let value =
                    self.descriptor_ptr(value, self.view_type().into(), "view_length_input")?;
                let call = self.invoke(
                    self.view_length_import(*ty == Ty::ViewUnicode),
                    &[value.into()],
                )?;
                Ok(call
                    .try_as_basic_value()
                    .expect_basic("view length returns Int64"))
            }
            TExpr::ViewAt(value, index, view_ty, sum) => {
                let value = self.expr(env, loops, value)?;
                let index = self.expr(env, loops, index)?;
                let value = self.descriptor_ptr(value, self.view_type().into(), "view_at_input")?;
                let raw = self
                    .invoke(
                        self.view_at_import(*view_ty == Ty::ViewUnicode),
                        &[value.into(), index.into()],
                    )?
                    .try_as_basic_value()
                    .expect_basic("view lookup returns a signed sentinel")
                    .into_int_value();
                let nil = self.context.i64_type().const_int(u64::MAX, true);
                let is_nil = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, raw, nil, "view_at_nil")
                    .map_err(|error| error.to_string())?;
                let members = self.program.sums[sum.index()].clone();
                let success = if *view_ty == Ty::ViewUnicode {
                    Ty::Unicode
                } else {
                    Ty::Byte
                };
                let success_tag = self.sum_member_tag(*sum, success)?;
                let nil_tag = self.sum_member_tag(*sum, Ty::Nil)?;
                let zeroed = self.struct_ty(Ty::Sum(*sum))?.const_zero();
                let nil_value = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context.i32_type().const_int(u64::from(nil_tag), false),
                        0,
                        "nil_tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let payload: BasicValueEnum<'ctx> = if *view_ty == Ty::ViewUnicode {
                    self.builder
                        .build_int_truncate(raw, self.context.i32_type(), "unicode_at")
                        .map_err(|error| error.to_string())?
                        .into()
                } else {
                    self.builder
                        .build_int_truncate(raw, self.context.i8_type(), "byte_at")
                        .map_err(|error| error.to_string())?
                        .into()
                };
                let success_value = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context
                            .i32_type()
                            .const_int(u64::from(success_tag), false),
                        0,
                        "value_tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let success_value = self
                    .builder
                    .build_insert_value(success_value, payload, success_tag + 1, "value_payload")
                    .map_err(|error| error.to_string())?;
                let _ = members;
                Ok(self
                    .builder
                    .build_select(
                        is_nil,
                        nil_value,
                        success_value.into_struct_value(),
                        "view_at",
                    )
                    .map_err(|error| error.to_string())?)
            }
            TExpr::MapContains {
                receiver,
                key,
                key_ty,
                value_ty,
            } => {
                let receiver = self.expr(env, loops, receiver)?;
                let key = self.expr(env, loops, key)?;
                let receiver = self.descriptor_ptr(
                    receiver,
                    self.collection_type().into(),
                    "map_contains_map",
                )?;
                let key = if *key_ty == Ty::ViewByte {
                    self.descriptor_ptr(key, self.view_type().into(), "map_contains_key")?
                        .into()
                } else {
                    key.into()
                };
                if self.map_uses_raw_value(*value_ty) {
                    return Ok(self
                        .invoke(
                            self.map_raw_contains_import(*key_ty)?,
                            &[receiver.into(), key],
                        )?
                        .try_as_basic_value()
                        .expect_basic("raw Map.contains returns Bool"));
                }
                Ok(self
                    .invoke(
                        self.map_contains_import(*key_ty, Ty::Int64)?,
                        &[receiver.into(), key],
                    )?
                    .try_as_basic_value()
                    .expect_basic("Map.contains returns Bool"))
            }
            TExpr::MapInsert {
                receiver,
                key,
                value,
                key_ty,
                value_ty,
                require_existing,
            } => {
                let key = self.expr(env, loops, key)?;
                let value = self.expr(env, loops, value)?;
                let key = if *key_ty == Ty::String {
                    self.descriptor_ptr(key, self.string_type().into(), "map_insert_key")?
                        .into()
                } else {
                    key.into()
                };
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Map.insert reached a place with no storage"));
                };
                if self.map_uses_raw_value(*value_ty) {
                    let value_slot = self.entry_alloca(self.ty(*value_ty), "map_value")?;
                    let old_slot = self.entry_alloca(self.ty(*value_ty), "map_old_value")?;
                    self.builder
                        .build_store(value_slot, value)
                        .map_err(|error| error.to_string())?;
                    let size = self.size_align(*value_ty).0;
                    let inserted = self
                        .invoke(
                            self.map_raw_insert_import(*key_ty)?,
                            &[
                                ptr.into(),
                                key,
                                value_slot.into(),
                                self.usize_ty().const_int(size, false).into(),
                                old_slot.into(),
                            ],
                        )?
                        .try_as_basic_value()
                        .expect_basic("raw Map.insert returns Bool")
                        .into_int_value();
                    if *require_existing {
                        let fresh = self
                            .builder
                            .build_int_compare(
                                IntPredicate::NE,
                                inserted,
                                self.context.i8_type().const_zero(),
                                "map_assignment_missing",
                            )
                            .map_err(|error| error.to_string())?;
                        let fail = self
                            .context
                            .append_basic_block(self.current_function(), "map_assignment_fail");
                        let continue_block = self
                            .context
                            .append_basic_block(self.current_function(), "map_assignment_done");
                        self.builder
                            .build_conditional_branch(fresh, fail, continue_block)
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(fail);
                        self.invoke(self.collection_bounds_fail_import(), &[])?;
                        self.builder
                            .build_unreachable()
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(continue_block);
                    }
                    if is_move_only(self.program, *value_ty) {
                        let replaced = self
                            .builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                inserted,
                                self.context.i8_type().const_zero(),
                                "map_replaced",
                            )
                            .map_err(|error| error.to_string())?;
                        let drop_block = self
                            .context
                            .append_basic_block(self.current_function(), "map_drop_replaced");
                        let continue_block = self
                            .context
                            .append_basic_block(self.current_function(), "map_insert_done");
                        self.builder
                            .build_conditional_branch(replaced, drop_block, continue_block)
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(drop_block);
                        let old = self
                            .builder
                            .build_load(self.ty(*value_ty), old_slot, "map_replaced_value")
                            .map_err(|error| error.to_string())?;
                        self.drop_value(*value_ty, old)?;
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(continue_block);
                    }
                    return Ok(inserted.into());
                }
                let inserted = self
                    .invoke(
                        self.map_insert_import(*key_ty, *value_ty)?,
                        &[ptr.into(), key, value.into()],
                    )?
                    .try_as_basic_value()
                    .expect_basic("Map.insert returns Bool")
                    .into_int_value();
                if *require_existing {
                    let fresh = self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            inserted,
                            self.context.i8_type().const_zero(),
                            "map_assignment_missing",
                        )
                        .map_err(|error| error.to_string())?;
                    let fail = self
                        .context
                        .append_basic_block(self.current_function(), "map_assignment_fail");
                    let continue_block = self
                        .context
                        .append_basic_block(self.current_function(), "map_assignment_done");
                    self.builder
                        .build_conditional_branch(fresh, fail, continue_block)
                        .map_err(|error| error.to_string())?;
                    self.builder.position_at_end(fail);
                    self.invoke(self.collection_bounds_fail_import(), &[])?;
                    self.builder
                        .build_unreachable()
                        .map_err(|error| error.to_string())?;
                    self.builder.position_at_end(continue_block);
                }
                Ok(inserted.into())
            }
            TExpr::MapDelete {
                receiver,
                key,
                key_ty,
                value_ty,
            } => {
                let key = self.expr(env, loops, key)?;
                let key = if *key_ty == Ty::ViewByte {
                    self.descriptor_ptr(key, self.view_type().into(), "map_delete_key")?
                        .into()
                } else {
                    key.into()
                };
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Map.delete reached a place with no storage"));
                };
                if self.map_uses_raw_value(*value_ty) {
                    let old_slot = self.entry_alloca(self.ty(*value_ty), "map_deleted_value")?;
                    let existed = self
                        .invoke(
                            self.map_raw_delete_import(*key_ty)?,
                            &[
                                ptr.into(),
                                key,
                                old_slot.into(),
                                self.usize_ty()
                                    .const_int(self.size_align(*value_ty).0, false)
                                    .into(),
                            ],
                        )?
                        .try_as_basic_value()
                        .expect_basic("raw Map.delete returns Bool")
                        .into_int_value();
                    if is_move_only(self.program, *value_ty) {
                        let found = self
                            .builder
                            .build_int_compare(
                                IntPredicate::NE,
                                existed,
                                self.context.i8_type().const_zero(),
                                "map_delete_found",
                            )
                            .map_err(|error| error.to_string())?;
                        let drop_block = self
                            .context
                            .append_basic_block(self.current_function(), "map_drop_deleted");
                        let continue_block = self
                            .context
                            .append_basic_block(self.current_function(), "map_delete_done");
                        self.builder
                            .build_conditional_branch(found, drop_block, continue_block)
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(drop_block);
                        let old = self
                            .builder
                            .build_load(self.ty(*value_ty), old_slot, "map_deleted_value")
                            .map_err(|error| error.to_string())?;
                        self.drop_value(*value_ty, old)?;
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(continue_block);
                    }
                    return Ok(existed.into());
                }
                Ok(self
                    .invoke(
                        self.map_delete_import(*key_ty, Ty::Int64)?,
                        &[ptr.into(), key],
                    )?
                    .try_as_basic_value()
                    .expect_basic("Map.delete returns Bool"))
            }
            TExpr::MapIndex {
                receiver,
                key,
                key_ty,
                value_ty,
            } => {
                let receiver = self.expr(env, loops, receiver)?;
                let key = self.expr(env, loops, key)?;
                let receiver =
                    self.descriptor_ptr(receiver, self.collection_type().into(), "map_index_map")?;
                let key = if *key_ty == Ty::ViewByte {
                    self.descriptor_ptr(key, self.view_type().into(), "map_index_key")?
                        .into()
                } else {
                    key.into()
                };
                if self.map_uses_raw_value(*value_ty) {
                    let out = self.entry_alloca(self.ty(*value_ty), "map_index_value")?;
                    self.invoke(
                        self.map_raw_read_import("index", *key_ty, true)?,
                        &[
                            receiver.into(),
                            key,
                            out.into(),
                            self.usize_ty()
                                .const_int(self.size_align(*value_ty).0, false)
                                .into(),
                        ],
                    )?;
                    return Ok(self
                        .builder
                        .build_load(self.ty(*value_ty), out, "map_index_value")
                        .map_err(|error| error.to_string())?);
                }
                Ok(self
                    .invoke(
                        self.map_index_import(*key_ty, *value_ty)?,
                        &[receiver.into(), key],
                    )?
                    .try_as_basic_value()
                    .expect_basic("Map indexing returns its value"))
            }
            TExpr::MapTake {
                receiver,
                key,
                key_ty,
                value_ty,
            } => {
                let key = self.expr(env, loops, key)?;
                let key = if *key_ty == Ty::ViewByte {
                    self.descriptor_ptr(key, self.view_type().into(), "map_take_key")?
                        .into()
                } else {
                    key.into()
                };
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Map.take reached a place with no storage"));
                };
                if self.map_uses_raw_value(*value_ty) {
                    let out = self.entry_alloca(self.ty(*value_ty), "map_take_value")?;
                    self.invoke(
                        self.map_raw_read_import("take", *key_ty, false)?,
                        &[
                            ptr.into(),
                            key,
                            out.into(),
                            self.usize_ty()
                                .const_int(self.size_align(*value_ty).0, false)
                                .into(),
                        ],
                    )?;
                    return Ok(self
                        .builder
                        .build_load(self.ty(*value_ty), out, "map_take_value")
                        .map_err(|error| error.to_string())?);
                }
                Ok(self
                    .invoke(
                        self.map_take_import(*key_ty, *value_ty)?,
                        &[ptr.into(), key],
                    )?
                    .try_as_basic_value()
                    .expect_basic("Map.take returns its value"))
            }
            TExpr::SetContains {
                receiver,
                value,
                elem,
            } => {
                let receiver = self.expr(env, loops, receiver)?;
                let value = self.expr(env, loops, value)?;
                let receiver = self.descriptor_ptr(
                    receiver,
                    self.collection_type().into(),
                    "set_contains_set",
                )?;
                let value = if *elem == Ty::String || *elem == Ty::ViewByte {
                    self.descriptor_ptr(value, self.view_type().into(), "set_contains_value")?
                        .into()
                } else {
                    value.into()
                };
                Ok(self
                    .invoke(self.set_contains_import(*elem)?, &[receiver.into(), value])?
                    .try_as_basic_value()
                    .expect_basic("Set.contains returns Bool"))
            }
            TExpr::SetInsert {
                receiver,
                value,
                elem,
            } => {
                let value = self.expr(env, loops, value)?;
                let value = if *elem == Ty::String {
                    self.descriptor_ptr(value, self.string_type().into(), "set_insert_value")?
                        .into()
                } else {
                    value.into()
                };
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Set.insert reached a place with no storage"));
                };
                Ok(self
                    .invoke(self.set_insert_import(*elem)?, &[ptr.into(), value])?
                    .try_as_basic_value()
                    .expect_basic("Set.insert returns Bool"))
            }
            TExpr::SetDelete {
                receiver,
                value,
                elem,
            } => {
                let value = self.expr(env, loops, value)?;
                let value = if *elem == Ty::String || *elem == Ty::ViewByte {
                    self.descriptor_ptr(value, self.view_type().into(), "set_delete_value")?
                        .into()
                } else {
                    value.into()
                };
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("Set.delete reached a place with no storage"));
                };
                Ok(self
                    .invoke(self.set_delete_import(*elem)?, &[ptr.into(), value])?
                    .try_as_basic_value()
                    .expect_basic("Set.delete returns Bool"))
            }
            TExpr::CollectionLiteral { ty, items } => {
                let collection_id = match *ty {
                    Ty::Array(id) | Ty::List(id) => id,
                    _ => return Err(internal("only arrays and lists have collection literals")),
                };
                let elem = match &self.program.collections[collection_id.index()] {
                    crate::semantics::types::CollectionDef::Array { elem, .. }
                    | crate::semantics::types::CollectionDef::List { elem } => *elem,
                    _ => return Err(internal("collection literal metadata is not a sequence")),
                };
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.expr(env, loops, item)?);
                }
                let ptr = if values.is_empty() {
                    self.context.ptr_type(AddressSpace::default()).const_null()
                } else {
                    let (element_size, element_align) = self.size_align(elem);
                    let count = self.usize_ty().const_int(values.len() as u64, false);
                    let size = self
                        .builder
                        .build_int_mul(
                            count,
                            self.usize_ty().const_int(element_size, false),
                            "collection_size",
                        )
                        .map_err(|error| error.to_string())?;
                    let ptr = self
                        .invoke(
                            self.alloc_import(),
                            &[
                                size.into(),
                                self.usize_ty().const_int(element_align, false).into(),
                            ],
                        )?
                        .try_as_basic_value()
                        .expect_basic("collection allocation returns a pointer")
                        .into_pointer_value();
                    for (index, value) in values.iter().enumerate() {
                        // Safety: `ptr` is the allocation returned above and
                        // every index is within the exact number of elements.
                        let slot = unsafe {
                            self.builder.build_gep(
                                self.ty(elem),
                                ptr,
                                &[self.context.i64_type().const_int(index as u64, false)],
                                "collection_element",
                            )
                        }
                        .map_err(|error| error.to_string())?;
                        self.builder
                            .build_store(slot, *value)
                            .map_err(|error| error.to_string())?;
                    }
                    ptr
                };
                let len = self
                    .context
                    .i64_type()
                    .const_int(values.len() as u64, false);
                let mut descriptor = self.ty(*ty).into_struct_type().const_zero();
                descriptor = self
                    .builder
                    .build_insert_value(descriptor, ptr, 0, "collection_ptr")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                descriptor = self
                    .builder
                    .build_insert_value(descriptor, len, 1, "collection_len")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                descriptor = self
                    .builder
                    .build_insert_value(descriptor, len, 2, "collection_cap")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                Ok(descriptor.into())
            }
            TExpr::CollectionNew(ty) => {
                let mut descriptor = self.ty(*ty).into_struct_type().const_zero();
                descriptor = self
                    .builder
                    .build_insert_value(
                        descriptor,
                        self.context.ptr_type(AddressSpace::default()).const_null(),
                        0,
                        "collection_ptr",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                Ok(descriptor.into())
            }
            TExpr::CollectionLength(value) => {
                let value = self.expr(env, loops, value)?;
                Ok(self
                    .builder
                    .build_extract_value(as_struct(value)?, 1, "collection_len")
                    .map_err(|error| error.to_string())?)
            }
            TExpr::CollectionIsEmpty(value) => {
                let value = self.expr(env, loops, value)?;
                let length = self
                    .builder
                    .build_extract_value(as_struct(value)?, 1, "collection_len")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let empty = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        length,
                        self.context.i64_type().const_zero(),
                        "collection_empty",
                    )
                    .map_err(|error| error.to_string())?;
                Ok(self
                    .builder
                    .build_int_z_extend(empty, self.context.i8_type(), "collection_empty_byte")
                    .map_err(|error| error.to_string())?
                    .into())
            }
            TExpr::ViewSlice {
                value,
                start,
                end,
                view_ty,
                sum,
            } => {
                let value = self.expr(env, loops, value)?;
                let start = self.expr(env, loops, start)?;
                let end = self.expr(env, loops, end)?;
                let value =
                    self.descriptor_ptr(value, self.view_type().into(), "view_slice_input")?;
                let slice = self
                    .view_out_call(
                        self.view_slice_import(*view_ty == Ty::ViewUnicode),
                        vec![value.into(), start.into(), end.into()],
                        "view_slice_value",
                    )?
                    .into_struct_value();
                let valid = self
                    .builder
                    .build_is_not_null(
                        self.builder
                            .build_extract_value(slice, 0, "slice_ptr")
                            .map_err(|error| error.to_string())?
                            .into_pointer_value(),
                        "slice_valid",
                    )
                    .map_err(|error| error.to_string())?;
                let nil_tag = self.sum_member_tag(*sum, Ty::Nil)?;
                let view_tag = self.sum_member_tag(*sum, *view_ty)?;
                let zeroed = self.struct_ty(Ty::Sum(*sum))?.const_zero();
                let nil = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context.i32_type().const_int(u64::from(nil_tag), false),
                        0,
                        "slice_nil",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let tagged = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context
                            .i32_type()
                            .const_int(u64::from(view_tag), false),
                        0,
                        "slice_view",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let view = self
                    .builder
                    .build_insert_value(tagged, slice, view_tag + 1, "slice_payload")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                Ok(self
                    .builder
                    .build_select(valid, view, nil, "slice_result")
                    .map_err(|error| error.to_string())?
                    .into())
            }
            TExpr::CollectionCapacity(value) => {
                let value = self.expr(env, loops, value)?;
                Ok(self
                    .builder
                    .build_extract_value(as_struct(value)?, 2, "collection_cap")
                    .map_err(|error| error.to_string())?)
            }
            TExpr::CollectionView(value, _) => self.expr(env, loops, value),
            TExpr::CollectionSlice {
                value,
                start,
                end,
                view_ty,
                sum,
                elem,
            } => {
                let value = self.expr(env, loops, value)?.into_struct_value();
                let start = self.expr(env, loops, start)?.into_int_value();
                let end = self.expr(env, loops, end)?.into_int_value();
                let ptr = self
                    .builder
                    .build_extract_value(value, 0, "view_ptr")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(value, 1, "view_len")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let zero = self.context.i64_type().const_zero();
                let start_nonnegative = self
                    .builder
                    .build_int_compare(IntPredicate::SGE, start, zero, "slice_start_nonnegative")
                    .map_err(|error| error.to_string())?;
                let end_nonnegative = self
                    .builder
                    .build_int_compare(IntPredicate::SGE, end, zero, "slice_end_nonnegative")
                    .map_err(|error| error.to_string())?;
                let ordered = self
                    .builder
                    .build_int_compare(IntPredicate::ULE, start, end, "slice_ordered")
                    .map_err(|error| error.to_string())?;
                let end_in_range = self
                    .builder
                    .build_int_compare(IntPredicate::ULE, end, length, "slice_end_in_range")
                    .map_err(|error| error.to_string())?;
                let valid = self
                    .builder
                    .build_and(start_nonnegative, end_nonnegative, "slice_nonnegative")
                    .map_err(|error| error.to_string())?;
                let valid = self
                    .builder
                    .build_and(valid, ordered, "slice_ordered_valid")
                    .map_err(|error| error.to_string())?;
                let valid = self
                    .builder
                    .build_and(valid, end_in_range, "slice_range_valid")
                    .map_err(|error| error.to_string())?;
                let function = self.current_function();
                let valid_block = self.context.append_basic_block(function, "slice_valid");
                let invalid_block = self.context.append_basic_block(function, "slice_invalid");
                let join_block = self.context.append_basic_block(function, "slice_join");
                self.builder
                    .build_conditional_branch(valid, valid_block, invalid_block)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(invalid_block);
                let nil_tag = self.sum_member_tag(*sum, Ty::Nil)?;
                let zeroed = self.struct_ty(Ty::Sum(*sum))?.const_zero();
                let nil = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context.i32_type().const_int(u64::from(nil_tag), false),
                        0,
                        "slice_nil_tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                self.builder
                    .build_unconditional_branch(join_block)
                    .map_err(|error| error.to_string())?;
                let invalid_end = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| internal("slice invalid block has no insertion block"))?;

                self.builder.position_at_end(valid_block);
                // Safety: the checker establishes the half-open range and the
                // source descriptor owns aligned storage for `elem` values.
                let sliced_ptr = unsafe {
                    self.builder
                        .build_gep(self.ty(*elem), ptr, &[start], "slice_ptr")
                }
                .map_err(|error| error.to_string())?;
                let sliced_length = self
                    .builder
                    .build_int_sub(end, start, "slice_len")
                    .map_err(|error| error.to_string())?;
                let view_zero = self.ty(*view_ty).into_struct_type().const_zero();
                let view = self
                    .builder
                    .build_insert_value(view_zero, sliced_ptr, 0, "slice_view_ptr")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let view = self
                    .builder
                    .build_insert_value(view, sliced_length, 1, "slice_view_len")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let view_tag = self.sum_member_tag(*sum, *view_ty)?;
                let tagged = self
                    .builder
                    .build_insert_value(
                        zeroed,
                        self.context
                            .i32_type()
                            .const_int(u64::from(view_tag), false),
                        0,
                        "slice_view_tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                let tagged = self
                    .builder
                    .build_insert_value(tagged, view, view_tag + 1, "slice_view_payload")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                self.builder
                    .build_unconditional_branch(join_block)
                    .map_err(|error| error.to_string())?;
                let valid_end = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| internal("slice valid block has no insertion block"))?;

                self.builder.position_at_end(join_block);
                let phi = self
                    .builder
                    .build_phi(self.struct_ty(Ty::Sum(*sum))?, "slice_result")
                    .map_err(|error| error.to_string())?;
                phi.add_incoming(&[(&nil, invalid_end), (&tagged, valid_end)]);
                Ok(phi.as_basic_value())
            }
            TExpr::CollectionIndex {
                collection,
                index,
                collection_ty,
                elem,
            } => {
                let collection = self.expr(env, loops, collection)?.into_struct_value();
                let index = self.expr(env, loops, index)?.into_int_value();
                let ptr = self
                    .builder
                    .build_extract_value(collection, 0, "collection_ptr")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let len = self
                    .builder
                    .build_extract_value(collection, 1, "collection_len")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let nonnegative = self
                    .builder
                    .build_int_compare(
                        IntPredicate::SGE,
                        index,
                        self.context.i64_type().const_zero(),
                        "index_nonnegative",
                    )
                    .map_err(|error| error.to_string())?;
                let in_range = self
                    .builder
                    .build_int_compare(IntPredicate::ULT, index, len, "index_in_range")
                    .map_err(|error| error.to_string())?;
                let valid = self
                    .builder
                    .build_and(nonnegative, in_range, "index_valid")
                    .map_err(|error| error.to_string())?;
                let function = self.current_function();
                let valid_block = self.context.append_basic_block(function, "index_valid");
                let invalid_block = self.context.append_basic_block(function, "index_invalid");
                self.builder
                    .build_conditional_branch(valid, valid_block, invalid_block)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(invalid_block);
                self.invoke(self.collection_bounds_fail_import(), &[])?;
                self.builder
                    .build_unreachable()
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(valid_block);
                let _ = collection_ty;
                // Safety: the checker enforces the range predicate above and
                // the descriptor points at storage for this element type.
                let element_ptr = unsafe {
                    self.builder
                        .build_gep(self.ty(*elem), ptr, &[index], "indexed_element")
                }
                .map_err(|error| error.to_string())?;
                self.builder
                    .build_load(self.ty(*elem), element_ptr, "indexed_value")
                    .map_err(|error| error.to_string())
            }
            TExpr::ListPop { receiver, elem } => {
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("List.pop reached a place with no storage"));
                };
                if is_scalar_collection_element(*elem) {
                    Ok(self
                        .invoke(self.list_pop_import(*elem)?, &[ptr.into()])?
                        .try_as_basic_value()
                        .expect_basic("List.pop returns an element"))
                } else {
                    let slot = self.entry_alloca(self.ty(*elem), "list_pop_value")?;
                    let (size, _) = self.size_align(*elem);
                    self.invoke(
                        self.list_raw_import("pop"),
                        &[
                            ptr.into(),
                            slot.into(),
                            self.usize_ty().const_int(size, false).into(),
                        ],
                    )?;
                    self.builder
                        .build_load(self.ty(*elem), slot, "list_pop_value")
                        .map_err(|error| error.to_string())
                }
            }
            TExpr::ListRemove {
                receiver,
                index,
                elem,
            } => {
                let index = self.expr(env, loops, index)?;
                let Some((ptr, _)) = self.place_ptr(env, receiver)? else {
                    return Err(internal("List.remove reached a place with no storage"));
                };
                if is_scalar_collection_element(*elem) {
                    Ok(self
                        .invoke(self.list_remove_import(*elem)?, &[ptr.into(), index.into()])?
                        .try_as_basic_value()
                        .expect_basic("List.remove returns an element"))
                } else {
                    let slot = self.entry_alloca(self.ty(*elem), "list_remove_value")?;
                    let (size, _) = self.size_align(*elem);
                    self.invoke(
                        self.list_raw_import("remove"),
                        &[
                            ptr.into(),
                            index.into(),
                            slot.into(),
                            self.usize_ty().const_int(size, false).into(),
                        ],
                    )?;
                    self.builder
                        .build_load(self.ty(*elem), slot, "list_remove_value")
                        .map_err(|error| error.to_string())
                }
            }
            TExpr::Cast(value, Ty::Float64) => {
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
            TExpr::LiftSum { value, from, to } => {
                let value = self.expr(env, loops, value)?;
                let source = as_struct(value)?;
                let source_tag = self
                    .builder
                    .build_extract_value(source, 0, "source_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let source_members = self.program.sums[from.index()].clone();
                let function = self.current_function();
                let done = self.context.append_basic_block(function, "lift_done");
                let unknown = self.context.append_basic_block(function, "lift_unknown");
                let cases: Vec<_> = source_members
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        (
                            self.context.i32_type().const_int(index as u64, false),
                            self.context.append_basic_block(function, "lift_member"),
                        )
                    })
                    .collect();
                self.builder
                    .build_switch(source_tag, unknown, &cases)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(unknown);
                self.exhausted()?;
                let target_ty = self.struct_ty(Ty::Sum(*to))?;
                let zeroed = target_ty.const_zero();
                let mut incoming = Vec::with_capacity(source_members.len());
                for (member, (_, block)) in source_members.iter().zip(cases) {
                    self.builder.position_at_end(block);
                    let target_tag = self.sum_member_tag(*to, *member)?;
                    let tagged = self
                        .builder
                        .build_insert_value(
                            zeroed,
                            self.context
                                .i32_type()
                                .const_int(u64::from(target_tag), false),
                            0,
                            "lift_tag",
                        )
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                    let payload = if *member == Ty::Nil {
                        self.context.i8_type().const_zero().into()
                    } else {
                        let source_tag = self.sum_member_tag(*from, *member)?;
                        self.builder
                            .build_extract_value(source, source_tag + 1, "lift_payload")
                            .map_err(|error| error.to_string())?
                    };
                    let lifted = self
                        .builder
                        .build_insert_value(tagged, payload, target_tag + 1, "lift_payload")
                        .map_err(|error| error.to_string())?
                        .into_struct_value();
                    let current = self
                        .builder
                        .get_insert_block()
                        .ok_or_else(|| internal("sum lift arm has no block"))?;
                    self.builder
                        .build_unconditional_branch(done)
                        .map_err(|error| error.to_string())?;
                    incoming.push((lifted, current));
                }
                self.builder.position_at_end(done);
                let phi = self
                    .builder
                    .build_phi(target_ty, "lifted_sum")
                    .map_err(|error| error.to_string())?;
                let incoming: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                    .iter()
                    .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
                    .collect();
                phi.add_incoming(&incoming);
                Ok(phi.as_basic_value())
            }
            TExpr::Not(value) => {
                let value = self.expr(env, loops, value)?.into_int_value();
                let result = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        value,
                        self.context.i8_type().const_zero(),
                        "not",
                    )
                    .map_err(|error| error.to_string())?;
                Ok(self
                    .builder
                    .build_int_z_extend(result, self.context.i8_type(), "bool")
                    .map_err(|error| error.to_string())?
                    .into())
            }
            TExpr::Logical(left, op, right) => {
                let left = self.expr(env, loops, left)?.into_int_value();
                let function = self.current_function();
                let right_block = self.context.append_basic_block(function, "logical_right");
                let short_block = self.context.append_basic_block(function, "logical_short");
                let merge = self.context.append_basic_block(function, "logical_merge");
                let left_true = self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        left,
                        self.context.i8_type().const_zero(),
                        "logical_test",
                    )
                    .map_err(|error| error.to_string())?;
                let (when_true, when_false, short_value) = match op {
                    LogicalOp::And => (
                        right_block,
                        short_block,
                        self.context.i8_type().const_zero(),
                    ),
                    LogicalOp::Or => (
                        short_block,
                        right_block,
                        self.context.i8_type().const_int(1, false),
                    ),
                };
                self.builder
                    .build_conditional_branch(left_true, when_true, when_false)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(short_block);
                self.builder
                    .build_unconditional_branch(merge)
                    .map_err(|error| error.to_string())?;
                let short_end = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| internal("logical short-circuit block has no end"))?;

                self.builder.position_at_end(right_block);
                let right = self.expr(env, loops, right)?.into_int_value();
                self.builder
                    .build_unconditional_branch(merge)
                    .map_err(|error| error.to_string())?;
                let right_end = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| internal("logical right-hand block has no end"))?;

                self.builder.position_at_end(merge);
                let phi = self
                    .builder
                    .build_phi(self.context.i8_type(), "logical_value")
                    .map_err(|error| error.to_string())?;
                phi.add_incoming(&[(&short_value, short_end), (&right, right_end)]);
                Ok(phi.as_basic_value())
            }
            TExpr::Truthiness(value, ty) => {
                let value = self.expr(env, loops, value)?;
                Ok(self.truthiness_value(*ty, value)?.into())
            }
            TExpr::ReturnOnError {
                value,
                sum,
                success,
                result,
                cleanup,
            } => self.lower_return_on_error(env, loops, value, *sum, *success, *result, cleanup),
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
                    self.validate_float(value, "arithmetic_nan")?;
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
                let comparison = if matches!(operand_ty, Ty::String) {
                    let left =
                        self.descriptor_ptr(left, self.string_type().into(), "equal_left")?;
                    let right =
                        self.descriptor_ptr(right, self.string_type().into(), "equal_right")?;
                    let call =
                        self.invoke(self.string_equal_import(), &[left.into(), right.into()])?;
                    let equal = call
                        .try_as_basic_value()
                        .expect_basic("string equality returns a byte")
                        .into_int_value();
                    match op {
                        CmpOp::Eq => equal,
                        CmpOp::NotEq => builder
                            .build_xor(equal, self.context.i8_type().const_int(1, false), "ne")
                            .map_err(|error| error.to_string())?,
                        _ => return Err(internal("checker allowed ordered string comparison")),
                    }
                } else if matches!(operand_ty, Ty::ViewByte | Ty::ViewUnicode) {
                    let left = self.descriptor_ptr(left, self.view_type().into(), "equal_left")?;
                    let right =
                        self.descriptor_ptr(right, self.view_type().into(), "equal_right")?;
                    let equal = self
                        .invoke(self.view_equal_import(), &[left.into(), right.into()])?
                        .try_as_basic_value()
                        .expect_basic("view equality returns a byte")
                        .into_int_value();
                    match op {
                        CmpOp::Eq => equal,
                        CmpOp::NotEq => builder
                            .build_xor(equal, self.context.i8_type().const_int(1, false), "ne")
                            .map_err(|error| error.to_string())?,
                        _ => return Err(internal("checker allowed ordered view comparison")),
                    }
                } else if matches!(operand_ty, Ty::Array(_) | Ty::List(_) | Ty::View(_)) {
                    let equal = self.equal(operand_ty, left, right)?;
                    match op {
                        CmpOp::Eq => equal,
                        CmpOp::NotEq => builder
                            .build_not(equal, "ne")
                            .map_err(|error| error.to_string())?,
                        _ => return Err(internal("checker allowed ordered collection comparison")),
                    }
                } else if matches!(operand_ty, Ty::User(_) | Ty::Sum(_)) {
                    // Recursive, type-directed equality; `!=` is its negation.
                    let equal = self.equal(operand_ty, left, right)?;
                    match op {
                        CmpOp::Eq => equal,
                        _ => builder
                            .build_not(equal, "ne")
                            .map_err(|error| error.to_string())?,
                    }
                } else if is_float(operand_ty) {
                    // `Float32` reuses the `Float64` rule: every predicate but
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
                let value = call
                    .try_as_basic_value()
                    .expect_basic("a checked call expression always returns a value");
                let result_ty = self
                    .program
                    .externs
                    .get(name)
                    .and_then(|function| function.result)
                    .or_else(|| {
                        self.program
                            .funcs
                            .get(name)
                            .and_then(|function| function.result)
                    });
                if matches!(result_ty, Some(Ty::Float32 | Ty::Float64)) {
                    self.validate_float(value.into_float_value(), "bridge_float_result_nan")?;
                }
                Ok(value)
            }
            TExpr::MethodCall(call) => {
                let method_id = call.method;
                let call_value = self.method_call(env, loops, call)?;
                let value = call_value
                    .try_as_basic_value()
                    .expect_basic("a checked method call expression always returns a value");
                if matches!(
                    self.program.methods[method_id.index()].result,
                    Some(Ty::Float32 | Ty::Float64)
                ) {
                    self.validate_float(value.into_float_value(), "method_float_result_nan")?;
                }
                Ok(value)
            }
            TExpr::If(form) => self.value_if(env, loops, form),
            TExpr::Print(value, ty) => {
                let value = self.expr(env, loops, value)?;
                let function = self.print_import(*ty)?;
                if *ty == Ty::String {
                    let value =
                        self.descriptor_ptr(value, self.string_type().into(), "print_string")?;
                    self.invoke(function, &[value.into()])?;
                } else {
                    self.invoke(function, &[value.into()])?;
                }
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

    fn drop_sequence_elements(
        &self,
        ptr: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        elem: Ty,
    ) -> Result<(), String> {
        let function = self.current_function();
        let loop_block = self
            .context
            .append_basic_block(function, "list_clear_drop_loop");
        let done = self
            .context
            .append_basic_block(function, "list_clear_drop_done");
        let entry = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("list clear has no insertion block"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(loop_block);
        let index = self
            .builder
            .build_phi(self.context.i64_type(), "list_clear_drop_index")
            .map_err(|error| error.to_string())?;
        let zero = self.context.i64_type().const_zero();
        index.add_incoming(&[(&zero, entry)]);
        let current = index.as_basic_value().into_int_value();
        let more = self
            .builder
            .build_int_compare(IntPredicate::ULT, current, len, "list_clear_drop_more")
            .map_err(|error| error.to_string())?;
        let body = self
            .context
            .append_basic_block(function, "list_clear_drop_item");
        self.builder
            .build_conditional_branch(more, body, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(body);
        // Safety: `current < len` and the descriptor points at initialized
        // storage for one value of the checked element type.
        let item_ptr = unsafe {
            self.builder
                .build_gep(self.ty(elem), ptr, &[current], "list_clear_drop_ptr")
        }
        .map_err(|error| error.to_string())?;
        let item = self
            .builder
            .build_load(self.ty(elem), item_ptr, "list_clear_drop_value")
            .map_err(|error| error.to_string())?;
        self.drop_value(elem, item)?;
        let next = self
            .builder
            .build_int_add(
                current,
                self.context.i64_type().const_int(1, false),
                "list_clear_drop_next",
            )
            .map_err(|error| error.to_string())?;
        let body_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("list clear drop body has no block"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next, body_end)]);
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Drops every opaque value still held by a raw map. The runtime stores
    /// only bytes, so typed destruction must happen before the runtime erases
    /// the entries or releases the map allocation.
    fn drop_map_values(
        &self,
        key_ty: Ty,
        value_ty: Ty,
        descriptor: StructValue<'ctx>,
    ) -> Result<(), String> {
        if !is_move_only(self.program, value_ty) {
            return Ok(());
        }
        let len = self
            .builder
            .build_extract_value(descriptor, 1, "map_drop_len")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let loop_block = self.context.append_basic_block(function, "map_drop_loop");
        let done = self.context.append_basic_block(function, "map_drop_done");
        let body = self.context.append_basic_block(function, "map_drop_item");
        let entry = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("map drop has no insertion block"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(loop_block);
        let index = self
            .builder
            .build_phi(self.context.i64_type(), "map_drop_index")
            .map_err(|error| error.to_string())?;
        let zero = self.context.i64_type().const_zero();
        index.add_incoming(&[(&zero, entry)]);
        let current = index.as_basic_value().into_int_value();
        let more = self
            .builder
            .build_int_compare(IntPredicate::ULT, current, len, "map_drop_more")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(more, body, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(body);
        let out = self.entry_alloca(self.ty(value_ty), "map_drop_value")?;
        let map_slot = self.entry_alloca(self.collection_type().into(), "map_drop_map")?;
        self.builder
            .build_store(map_slot, descriptor)
            .map_err(|error| error.to_string())?;
        self.invoke(
            self.map_raw_read_import("value_at", key_ty, false)?,
            &[
                map_slot.into(),
                current.into(),
                out.into(),
                self.usize_ty()
                    .const_int(self.size_align(value_ty).0, false)
                    .into(),
            ],
        )?;
        let item = self
            .builder
            .build_load(self.ty(value_ty), out, "map_drop_item_value")
            .map_err(|error| error.to_string())?;
        self.drop_value(value_ty, item)?;
        let next = self
            .builder
            .build_int_add(
                current,
                self.context.i64_type().const_int(1, false),
                "map_drop_next",
            )
            .map_err(|error| error.to_string())?;
        let body_end = self
            .builder
            .get_insert_block()
            .ok_or_else(|| internal("map drop body has no block"))?;
        self.builder
            .build_unconditional_branch(loop_block)
            .map_err(|error| error.to_string())?;
        index.add_incoming(&[(&next, body_end)]);
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Recursively destroys one owned value of `ty` (Specification 016
    /// section 8.1). Only ever called where the checked cleanup plan already
    /// decided `ty` is move-only -- a copyable value is never dropped -- so
    /// every arm recurses only into fields/members/pointees that are
    /// themselves move-only, skipping a copyable one entirely rather than
    /// re-deriving that fact from scratch at each level.
    fn drop_value(&self, ty: Ty, value: BasicValueEnum<'ctx>) -> Result<(), String> {
        match ty {
            Ty::String => {
                let slot = self.entry_alloca(self.string_type().into(), "string_drop_value")?;
                self.builder
                    .build_store(slot, value)
                    .map_err(|error| error.to_string())?;
                self.invoke(self.string_drop_import(), &[slot.into()])?;
                Ok(())
            }
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
            Ty::Map(id) => {
                let descriptor = as_struct(value)?;
                let (key_ty, value_ty) = match &self.program.collections[id.index()] {
                    crate::semantics::types::CollectionDef::Map { key, value } => (*key, *value),
                    _ => return Err(internal("map drop has non-map metadata")),
                };
                if self.map_uses_raw_value(value_ty) {
                    self.drop_map_values(key_ty, value_ty, descriptor)?;
                    let map_slot =
                        self.entry_alloca(self.collection_type().into(), "map_drop_map")?;
                    self.builder
                        .build_store(map_slot, descriptor)
                        .map_err(|error| error.to_string())?;
                    self.invoke(self.map_raw_drop_import(key_ty)?, &[map_slot.into()])?;
                } else {
                    let slot = self.entry_alloca(self.collection_type().into(), "map_drop_map")?;
                    self.builder
                        .build_store(slot, descriptor)
                        .map_err(|error| error.to_string())?;
                    self.invoke(self.map_drop_import(key_ty, value_ty)?, &[slot.into()])?;
                }
                Ok(())
            }
            Ty::Set(id) => {
                let descriptor = as_struct(value)?;
                let elem = match &self.program.collections[id.index()] {
                    crate::semantics::types::CollectionDef::Set { elem } => *elem,
                    _ => return Err(internal("set drop has non-set metadata")),
                };
                let slot = self.entry_alloca(self.collection_type().into(), "set_drop_set")?;
                self.builder
                    .build_store(slot, descriptor)
                    .map_err(|error| error.to_string())?;
                self.invoke(self.set_drop_import(elem)?, &[slot.into()])?;
                Ok(())
            }
            Ty::Array(id) | Ty::List(id) => {
                let descriptor = as_struct(value)?;
                let ptr = self
                    .builder
                    .build_extract_value(descriptor, 0, "collection_ptr")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let len = self
                    .builder
                    .build_extract_value(descriptor, 1, "collection_len")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let cap = self
                    .builder
                    .build_extract_value(descriptor, 2, "collection_cap")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let elem = match &self.program.collections[id.index()] {
                    crate::semantics::types::CollectionDef::Array { elem, .. }
                    | crate::semantics::types::CollectionDef::List { elem } => *elem,
                    _ => return Err(internal("sequence drop has non-sequence metadata")),
                };
                let function = self.current_function();
                let loop_block = self
                    .context
                    .append_basic_block(function, "collection_drop_loop");
                let done = self
                    .context
                    .append_basic_block(function, "collection_drop_done");
                let entry_block = self
                    .builder
                    .get_insert_block()
                    .ok_or_else(|| internal("collection drop has no insertion block"))?;
                self.builder
                    .build_unconditional_branch(loop_block)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(loop_block);
                let index = self
                    .builder
                    .build_phi(self.context.i64_type(), "collection_drop_index")
                    .map_err(|error| error.to_string())?;
                let zero = self.context.i64_type().const_zero();
                index.add_incoming(&[(&zero, entry_block)]);
                let current = index.as_basic_value().into_int_value();
                let more = self
                    .builder
                    .build_int_compare(IntPredicate::ULT, current, len, "collection_drop_more")
                    .map_err(|error| error.to_string())?;
                let body = self
                    .context
                    .append_basic_block(function, "collection_drop_item");
                self.builder
                    .build_conditional_branch(more, body, done)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(body);
                if is_move_only(self.program, elem) {
                    let item_ptr = unsafe {
                        self.builder.build_gep(
                            self.ty(elem),
                            ptr,
                            &[current],
                            "collection_drop_item_ptr",
                        )
                    }
                    .map_err(|error| error.to_string())?;
                    let item = self
                        .builder
                        .build_load(self.ty(elem), item_ptr, "collection_drop_item")
                        .map_err(|error| error.to_string())?;
                    self.drop_value(elem, item)?;
                }
                let next = self
                    .builder
                    .build_int_add(
                        current,
                        self.context.i64_type().const_int(1, false),
                        "collection_drop_next",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_unconditional_branch(loop_block)
                    .map_err(|error| error.to_string())?;
                index.add_incoming(&[(&next, self.builder.get_insert_block().unwrap())]);
                self.builder.position_at_end(done);
                let elem_size = self.size_align(elem).0;
                let size = self
                    .builder
                    .build_int_mul(
                        cap,
                        self.usize_ty().const_int(elem_size, false),
                        "collection_drop_size",
                    )
                    .map_err(|error| error.to_string())?;
                let align = self.usize_ty().const_int(self.size_align(elem).1, false);
                let nonnull = self
                    .builder
                    .build_is_not_null(ptr, "collection_drop_allocated")
                    .map_err(|error| error.to_string())?;
                let free = self
                    .context
                    .append_basic_block(function, "collection_drop_free");
                let after_free = self
                    .context
                    .append_basic_block(function, "collection_drop_after_free");
                self.builder
                    .build_conditional_branch(nonnull, free, after_free)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(free);
                self.call_raw_dealloc(ptr, size, align)?;
                self.builder
                    .build_unconditional_branch(after_free)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(after_free);
                Ok(())
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
        Ty::Box(_) | Ty::String | Ty::List(_) | Ty::Map(_) | Ty::Set(_) => true,
        Ty::Array(id) => match &program.collections[id.index()] {
            crate::semantics::types::CollectionDef::Array { elem, .. } => {
                is_move_only(program, *elem)
            }
            _ => false,
        },
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
