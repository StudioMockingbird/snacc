use chumsky::prelude::Rich;
use std::{fmt::Display, ops::Range};

use crate::syntax::ast;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticPhase {
    Lex,
    Parse,
    TypeCheck,
    Backend,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub phase: DiagnosticPhase,
    pub message: String,
    pub span: Option<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn first(&self) -> Option<&Diagnostic> {
        self.items.first()
    }

    pub(crate) fn from_rich<'a, T: Display + Clone>(
        phase: DiagnosticPhase,
        errors: Vec<Rich<'a, T, ast::Span>>,
    ) -> Self {
        let mut items = Vec::new();
        for error in errors {
            items.push(Diagnostic {
                phase,
                message: error.to_string(),
                span: Some(error.span().into_range()),
            });
        }
        Self { items }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            items: vec![Diagnostic {
                phase: DiagnosticPhase::Internal,
                message: message.into(),
                span: None,
            }],
        }
    }

    pub(crate) fn backend(message: impl Into<String>) -> Self {
        Self {
            items: vec![Diagnostic {
                phase: DiagnosticPhase::Backend,
                message: message.into(),
                span: None,
            }],
        }
    }
}
