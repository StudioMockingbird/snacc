use crate::syntax::ast::{NumLiteral, Span, Spanned};
use chumsky::prelude::*;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Token<'src> {
    Bool(bool),
    Num(NumLiteral),
    Str(&'src str),
    Op(&'src str),
    Ctrl(char),
    Ident(&'src str),
    Extern,
    Rust,
    Fun,
    Let,
    Print,
    If,
    Then,
    Do,
    TyDec64,
    TyInt64,
    TyBool,
    TyNil,
    Nil,
    While,
    ElseIf,
    Else,
    End,
    Break,
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Token::Bool(x) => write!(f, "{x}"),
            Token::Num(n) => write!(f, "{n}"),
            Token::Str(s) => write!(f, "{s}"),
            Token::Op(s) => write!(f, "{s}"),
            Token::Ctrl(c) => write!(f, "{c}"),
            Token::Ident(s) => write!(f, "{s}"),
            Token::Extern => write!(f, "extern"),
            Token::Rust => write!(f, "rust"),
            Token::Fun => write!(f, "fun"),
            Token::Let => write!(f, "let"),
            Token::Print => write!(f, "print"),
            Token::If => write!(f, "if"),
            Token::Then => write!(f, "then"),
            Token::Do => write!(f, "do"),
            Token::TyDec64 => write!(f, "Dec64"),
            Token::TyInt64 => write!(f, "Int64"),
            Token::TyBool => write!(f, "Bool"),
            Token::TyNil => write!(f, "Nil"),
            Token::Nil => write!(f, "nil"),
            Token::While => write!(f, "while"),
            Token::ElseIf => write!(f, "elseif"),
            Token::Else => write!(f, "else"),
            Token::End => write!(f, "end"),
            Token::Break => write!(f, "break"),
        }
    }
}

pub fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
    let num = text::int(10)
        .then(just('.').then(text::digits(10)).or_not())
        .to_slice()
        .try_map(|slice: &str, span| {
            if slice.contains('.') {
                let value = slice
                    .parse::<f64>()
                    .expect("lexer produced a valid decimal");
                Ok(Token::Num(NumLiteral::Dec(value)))
            } else {
                match slice.parse::<u64>() {
                    Ok(value) if value <= i64::MAX as u64 => {
                        Ok(Token::Num(NumLiteral::Int(value as i64)))
                    }
                    _ => Err(Rich::custom(
                        span,
                        format!("integer literal '{slice}' is out of range for Int64"),
                    )),
                }
            }
        });

    let str_ = just('"')
        .ignore_then(none_of('"').repeated().to_slice())
        .then_ignore(just('"'))
        .map(Token::Str);

    let op = one_of("+*-/!=<>")
        .repeated()
        .at_least(1)
        .to_slice()
        .map(Token::Op);

    let ctrl = one_of("()[],:").map(Token::Ctrl);

    let ident = text::ascii::ident().map(|ident: &str| match ident {
        "fun" => Token::Fun,
        "extern" => Token::Extern,
        "rust" => Token::Rust,
        "let" => Token::Let,
        "print" => Token::Print,
        "if" => Token::If,
        "then" => Token::Then,
        "Dec64" => Token::TyDec64,
        "Int64" => Token::TyInt64,
        "Bool" => Token::TyBool,
        "Nil" => Token::TyNil,
        "nil" | "null" => Token::Nil,
        "while" => Token::While,
        "do" => Token::Do,
        "elseif" => Token::ElseIf,
        "else" => Token::Else,
        "end" => Token::End,
        "break" => Token::Break,
        "true" => Token::Bool(true),
        "false" => Token::Bool(false),
        _ => Token::Ident(ident),
    });

    let token = num.or(str_).or(op).or(ctrl).or(ident);

    let comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .padded();

    token
        .map_with(|tok, e| (tok, e.span()))
        .padded_by(comment.repeated())
        .padded()
        .recover_with(skip_then_retry_until(any().ignored(), end()))
        .repeated()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> Vec<Token<'_>> {
        let (tokens, errors) = lexer().parse(source).into_output_errors();
        assert!(errors.is_empty(), "lex errors: {errors:?}");
        tokens
            .unwrap()
            .into_iter()
            .map(|(token, _span)| token)
            .collect()
    }

    #[test]
    fn break_lexes_to_its_own_token() {
        assert_eq!(lex("break"), vec![Token::Break]);
    }

    #[test]
    fn break_is_unavailable_as_an_identifier_regardless_of_context() {
        // `break` must never surface as Token::Ident("break"), matching how
        // `while`/`if`/etc. are reserved regardless of surrounding context.
        assert_eq!(
            lex("while break do break end"),
            vec![
                Token::While,
                Token::Break,
                Token::Do,
                Token::Break,
                Token::End,
            ]
        );
    }

    #[test]
    fn semicolon_is_a_lex_error() {
        let (_, errors) = lexer().parse("let x: Int64 = 1;").into_output_errors();
        assert!(
            !errors.is_empty(),
            "expected a lex error for a bare semicolon"
        );
        // The error must clearly name the offending character (not just fail
        // silently) so it's diagnosable as "no semicolon syntax" at this span.
        assert!(
            errors.iter().any(|e| e.to_string().contains("';'")),
            "expected the error to name the semicolon, got: {errors:?}"
        );
    }

    #[test]
    fn semicolon_is_a_lex_error_via_parse_entrypoint() {
        let diagnostics = crate::parse("let x: Int64 = 1;")
            .err()
            .expect("snacc has no semicolon syntax; `crate::parse` should report a diagnostic");
        let diagnostic = diagnostics
            .first()
            .expect("expected at least one diagnostic");
        assert_eq!(diagnostic.phase, crate::DiagnosticPhase::Lex);
    }
}
