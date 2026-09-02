mod backend;
mod diagnostics;
mod semantics;
mod syntax;

use chumsky::prelude::*;

pub use diagnostics::{Diagnostic, DiagnosticPhase, Diagnostics};
pub use semantics::checker::{Program, Ty};
pub use syntax::ast::Program as AstProgram;

/// Contract version for generated objects, runtime imports, and Rust bridges.
pub const ABI_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optimization {
    None,
    Aggressive,
}

impl Optimization {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Aggressive => "aggressive",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    pub optimization: Optimization,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            optimization: Optimization::Aggressive,
        }
    }
}

#[derive(Debug)]
pub struct EmittedObject {
    pub bytes: Vec<u8>,
    pub target_triple: String,
    pub optimization: Optimization,
    pub abi_version: u32,
}

pub fn parse<'src>(source: &'src str) -> Result<AstProgram<'src>, Diagnostics> {
    let (tokens, lex_errors) = syntax::lexer::lexer().parse(source).into_output_errors();
    if !lex_errors.is_empty() {
        return Err(Diagnostics::from_rich(DiagnosticPhase::Lex, lex_errors));
    }
    let Some(tokens) = tokens else {
        return Err(Diagnostics::internal("lexer produced no token stream"));
    };

    let (program, parse_errors) = syntax::parser::program_parser()
        .parse(
            tokens
                .as_slice()
                .map((source.len()..source.len()).into(), |(token, span)| {
                    (token, span)
                }),
        )
        .into_output_errors();
    if !parse_errors.is_empty() {
        return Err(Diagnostics::from_rich(DiagnosticPhase::Parse, parse_errors));
    }
    program.ok_or_else(|| Diagnostics::internal("parser produced no syntax tree"))
}

pub fn check(source: &str) -> Result<Program, Diagnostics> {
    let syntax = parse(source)?;
    semantics::checker::check(&syntax).map_err(|failure| match failure {
        semantics::checker::Failure::Source(errors) => Diagnostics {
            items: errors
                .into_iter()
                .map(|error| Diagnostic {
                    phase: DiagnosticPhase::TypeCheck,
                    message: error.msg,
                    span: Some(error.span.into_range()),
                })
                .collect(),
        },
        semantics::checker::Failure::Unknown(detail) => Diagnostics::internal(detail),
    })
}

pub fn emit_object(source: &str) -> Result<Vec<u8>, Diagnostics> {
    emit_object_with_options(source, CompileOptions::default()).map(|object| object.bytes)
}

/// Renders a program as LLVM IR without emitting an object. Nothing in the
/// build pipeline uses this; it exists so calling-convention attributes, which
/// no object file preserves, can be inspected.
pub fn emit_llvm_ir(source: &str) -> Result<String, Diagnostics> {
    let program = check(source)?;
    backend::llvm::compile_to_ir(&program, "snacc").map_err(Diagnostics::backend)
}

pub fn target_triple() -> String {
    backend::llvm::target_triple()
}

pub fn llvm_version() -> (u32, u32, u32) {
    backend::llvm::llvm_version()
}

pub fn emit_object_with_options(
    source: &str,
    options: CompileOptions,
) -> Result<EmittedObject, Diagnostics> {
    let program = check(source)?;
    let result = backend::llvm::compile(&program, "snacc", options.optimization);
    result
        .map(|(bytes, target_triple)| EmittedObject {
            bytes,
            target_triple,
            optimization: options.optimization,
            abi_version: ABI_VERSION,
        })
        .map_err(Diagnostics::backend)
}
