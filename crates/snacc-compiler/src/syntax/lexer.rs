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
    Mut,
    Print,
    If,
    Then,
    Do,
    Type,
    Is,
    Struct,
    Union,
    Method,
    SelfKw,
    /// `Ref`, reserved by Specification 011 section 4 for the reference
    /// parameter type. It is never an ordinary identifier.
    Ref,
    TyDec64,
    TyInt64,
    TyBool,
    TyNil,
    TyUInt8,
    TyUInt16,
    TyUInt32,
    TyUInt64,
    TyFloat32,
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
            Token::Mut => write!(f, "mut"),
            Token::Print => write!(f, "print"),
            Token::If => write!(f, "if"),
            Token::Then => write!(f, "then"),
            Token::Do => write!(f, "do"),
            Token::Type => write!(f, "type"),
            Token::Is => write!(f, "is"),
            Token::Struct => write!(f, "struct"),
            Token::Union => write!(f, "union"),
            Token::Method => write!(f, "method"),
            Token::SelfKw => write!(f, "self"),
            Token::Ref => write!(f, "Ref"),
            Token::TyDec64 => write!(f, "Dec64"),
            Token::TyInt64 => write!(f, "Int64"),
            Token::TyBool => write!(f, "Bool"),
            Token::TyNil => write!(f, "Nil"),
            Token::TyUInt8 => write!(f, "UInt8"),
            Token::TyUInt16 => write!(f, "UInt16"),
            Token::TyUInt32 => write!(f, "UInt32"),
            Token::TyUInt64 => write!(f, "UInt64"),
            Token::TyFloat32 => write!(f, "Float32"),
            Token::Nil => write!(f, "nil"),
            Token::While => write!(f, "while"),
            Token::ElseIf => write!(f, "elseif"),
            Token::Else => write!(f, "else"),
            Token::End => write!(f, "end"),
            Token::Break => write!(f, "break"),
        }
    }
}

/// Every suffix a numeric literal may carry, named when one is rejected.
const NUMERIC_SUFFIXES: &str = "'u8', 'u16', 'u32', 'u64', and 'f32'";

fn out_of_range(digits: &str, suffix: &str, ty: &str) -> String {
    format!("unsigned literal '{digits}{suffix}' is out of range for {ty}")
}

/// Classifies one maximally-munched numeric token, split into its digits (with
/// an optional fractional part) and the alphanumeric run that immediately
/// follows them. Specification 009 section 4.2 makes that whole run part of the
/// literal, so an unsupported suffix is one invalid token rather than a valid
/// number followed by an identifier.
fn classify_number(digits: &str, suffix: &str) -> Result<NumLiteral, String> {
    let fractional = digits.contains('.');
    match suffix {
        "" if fractional => Ok(NumLiteral::Dec(
            digits.parse().expect("lexer produced a valid decimal"),
        )),
        "" => digits
            .parse()
            .map(NumLiteral::Int)
            .map_err(|_| format!("integer literal '{digits}' is out of range for Int64")),
        // Rounded once, straight from the source decimal: parsing to `f64`
        // first and narrowing afterward would round twice.
        "f32" => {
            let value: f32 = digits.parse().expect("lexer produced a valid decimal");
            if value.is_finite() {
                Ok(NumLiteral::F32(value))
            } else {
                Err(format!(
                    "float literal '{digits}f32' is out of range for Float32"
                ))
            }
        }
        "u8" | "u16" | "u32" | "u64" if fractional => Err(format!(
            "numeric literal '{digits}{suffix}' has a fractional part, \
             but '{suffix}' requires an integer literal"
        )),
        "u8" => digits
            .parse()
            .map(NumLiteral::U8)
            .map_err(|_| out_of_range(digits, suffix, "UInt8")),
        "u16" => digits
            .parse()
            .map(NumLiteral::U16)
            .map_err(|_| out_of_range(digits, suffix, "UInt16")),
        "u32" => digits
            .parse()
            .map(NumLiteral::U32)
            .map_err(|_| out_of_range(digits, suffix, "UInt32")),
        "u64" => digits
            .parse()
            .map(NumLiteral::U64)
            .map_err(|_| out_of_range(digits, suffix, "UInt64")),
        _ => Err(format!(
            "numeric literal '{digits}{suffix}' has an unknown suffix; \
             supported suffixes are {NUMERIC_SUFFIXES}"
        )),
    }
}

pub fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
    // Maximal munch: the digits and every alphanumeric character touching them
    // are consumed as one candidate before anything is classified.
    let digits = text::int(10)
        .then(just('.').then(text::digits(10)).or_not())
        .to_slice();
    let suffix = any()
        .filter(|c: &char| c.is_ascii_alphanumeric() || *c == '_')
        .repeated()
        .to_slice();
    // A rejected literal is reported through `validate` rather than `try_map`
    // so the diagnostic keeps the whole token's span: chumsky merges a
    // `try_map` failure into the surrounding alternative's error and narrows it
    // to that alternative's position. `crate::parse` stops at any lex error, so
    // the stand-in token below never reaches the parser.
    let num = digits
        .then(suffix)
        .validate(|(digits, suffix): (&str, &str), extra, emitter| {
            match classify_number(digits, suffix) {
                Ok(literal) => Token::Num(literal),
                Err(message) => {
                    emitter.emit(Rich::custom(extra.span(), message));
                    Token::Num(NumLiteral::Int(0))
                }
            }
        });

    let str_ = just('"')
        .ignore_then(none_of('"').repeated().to_slice())
        .then_ignore(just('"'))
        .map(Token::Str);

    // Snacc's only multi-character operators are the four two-character
    // comparisons, so they are spelled out instead of munched greedily.
    // Specification 011 section 4 puts `>` in a type position, where a greedy
    // run would swallow the two closing brackets of `Ref<Ref<T>>` into one
    // token and hide the nested reference from the parser.
    let op = just("==")
        .or(just("!="))
        .or(just("<="))
        .or(just(">="))
        .or(one_of("+*-/!=<>").to_slice())
        .map(Token::Op);

    // `.` selects a member and continues a qualified path; `|` introduces a
    // union alternative. Numeric literals are munched before this alternative
    // runs, so `1.5` is still one token.
    let ctrl = one_of("()[],:.|").map(Token::Ctrl);

    let ident = text::ascii::ident().map(|ident: &str| match ident {
        "fun" => Token::Fun,
        "extern" => Token::Extern,
        "rust" => Token::Rust,
        "let" => Token::Let,
        "mut" => Token::Mut,
        "type" => Token::Type,
        "is" => Token::Is,
        "struct" => Token::Struct,
        "union" => Token::Union,
        "method" => Token::Method,
        "self" => Token::SelfKw,
        "Ref" => Token::Ref,
        "print" => Token::Print,
        "if" => Token::If,
        "then" => Token::Then,
        "Dec64" => Token::TyDec64,
        "Int64" => Token::TyInt64,
        "Bool" => Token::TyBool,
        "Nil" => Token::TyNil,
        "UInt8" => Token::TyUInt8,
        "UInt16" => Token::TyUInt16,
        "UInt32" => Token::TyUInt32,
        "UInt64" => Token::TyUInt64,
        "Float32" => Token::TyFloat32,
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

    /// Lexes a source that is expected to fail, returning one message and span
    /// per error.
    fn lex_failing(source: &str) -> Vec<(String, Span)> {
        let (_, errors) = lexer().parse(source).into_output_errors();
        errors
            .iter()
            .map(|error| (error.to_string(), *error.span()))
            .collect()
    }

    fn num(source: &str) -> NumLiteral {
        match lex(source).as_slice() {
            [Token::Num(literal)] => *literal,
            other => panic!("expected one numeric token for {source}, got: {other:?}"),
        }
    }

    // Specification 009 section 4.2-4.3: literal forms, ranges, and munching.

    #[test]
    fn lexes_zero_and_the_maximum_for_every_unsigned_width() {
        assert_eq!(num("0u8"), NumLiteral::U8(0));
        assert_eq!(num("255u8"), NumLiteral::U8(u8::MAX));
        assert_eq!(num("0u16"), NumLiteral::U16(0));
        assert_eq!(num("65535u16"), NumLiteral::U16(u16::MAX));
        assert_eq!(num("0u32"), NumLiteral::U32(0));
        assert_eq!(num("4294967295u32"), NumLiteral::U32(u32::MAX));
        assert_eq!(num("0u64"), NumLiteral::U64(0));
        assert_eq!(num("18446744073709551615u64"), NumLiteral::U64(u64::MAX));
    }

    #[test]
    fn rejects_one_above_the_maximum_for_every_unsigned_width() {
        for (source, ty) in [
            ("256u8", "UInt8"),
            ("65536u16", "UInt16"),
            ("4294967296u32", "UInt32"),
            ("18446744073709551616u64", "UInt64"),
        ] {
            let errors = lex_failing(source);
            assert!(
                errors
                    .iter()
                    .any(|(message, _)| message.contains(source) && message.contains(ty)),
                "expected an out-of-range error naming {source} and {ty}, got: {errors:?}"
            );
        }
    }

    #[test]
    fn lexes_float32_with_and_without_a_fractional_part() {
        assert_eq!(num("0f32"), NumLiteral::F32(0.0));
        assert_eq!(num("1f32"), NumLiteral::F32(1.0));
        assert_eq!(num("1.5f32"), NumLiteral::F32(1.5));
    }

    #[test]
    fn a_float32_literal_is_rounded_to_the_nearest_binary32_value() {
        assert_eq!(
            num("0.1f32"),
            NumLiteral::F32("0.1".parse::<f32>().expect("0.1 parses as f32")),
            "0.1 has no exact binary32 value and rounds to nearest"
        );
        assert_eq!(
            num("16777217f32"),
            NumLiteral::F32(16_777_216.0),
            "2^24 + 1 has no binary32 value and rounds to 2^24"
        );
    }

    #[test]
    fn rejects_a_float32_literal_that_rounds_to_infinity() {
        let source = "999999999999999999999999999999999999999999f32";
        let errors = lex_failing(source);
        assert!(
            errors
                .iter()
                .any(|(message, _)| message.contains("Float32") && message.contains("out of range")),
            "expected a Float32 range error, got: {errors:?}"
        );
    }

    #[test]
    fn a_malformed_suffix_is_one_invalid_token_not_a_number_and_an_identifier() {
        // Specification 009 section 4.2: maximal munch. Each of these is one
        // bad token, so each yields exactly one error whose span covers the
        // whole text -- not a valid number plus a separate identifier.
        for source in ["1u9", "1u8x", "1f64", "1.0u8", "1u8_", "12.5f64"] {
            let errors = lex_failing(source);
            assert_eq!(
                errors.len(),
                1,
                "expected exactly one error for {source}, got: {errors:?}"
            );
            let (message, span) = &errors[0];
            assert!(
                message.contains(source),
                "error for {source} should name the complete token, got: {message}"
            );
            assert_eq!(
                (span.start, span.end),
                (0, source.len()),
                "the error for {source} should span the whole token"
            );
        }
    }

    #[test]
    fn suffix_spellings_stay_ordinary_identifiers_away_from_digits() {
        assert_eq!(lex("u8"), vec![Token::Ident("u8")]);
        assert_eq!(lex("f32"), vec![Token::Ident("f32")]);
        assert_eq!(
            lex("1 u8"),
            vec![Token::Num(NumLiteral::Int(1)), Token::Ident("u8")]
        );
    }

    #[test]
    fn the_new_type_names_are_reserved_words() {
        assert_eq!(
            lex("UInt8 UInt16 UInt32 UInt64 Float32"),
            vec![
                Token::TyUInt8,
                Token::TyUInt16,
                Token::TyUInt32,
                Token::TyUInt64,
                Token::TyFloat32,
            ]
        );
    }

    #[test]
    fn unsuffixed_literals_keep_their_existing_types() {
        assert_eq!(num("7"), NumLiteral::Int(7));
        assert_eq!(num("7.5"), NumLiteral::Dec(7.5));
        assert_eq!(
            num("9223372036854775807"),
            NumLiteral::Int(i64::MAX),
            "Int64 literals still arrive exactly, never through f64"
        );
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

    /// Specification 010 section 4: every new keyword is reserved, and `.`/`|`
    /// lex as their own control characters.
    #[test]
    fn the_nominal_type_keywords_are_reserved_words() {
        assert_eq!(
            lex("type is struct union method self mut"),
            vec![
                Token::Type,
                Token::Is,
                Token::Struct,
                Token::Union,
                Token::Method,
                Token::SelfKw,
                Token::Mut,
            ]
        );
    }

    /// Specification 011 section 4: `Ref` is reserved, and the type brackets are
    /// the ordinary comparison operator tokens -- never munched into `>>`.
    #[test]
    fn ref_is_reserved_and_type_brackets_lex_one_at_a_time() {
        assert_eq!(
            lex("Ref<Ref<Int64>>"),
            vec![
                Token::Ref,
                Token::Op("<"),
                Token::Ref,
                Token::Op("<"),
                Token::TyInt64,
                Token::Op(">"),
                Token::Op(">"),
            ]
        );
    }

    #[test]
    fn two_character_comparisons_still_lex_as_one_token() {
        assert_eq!(
            lex("a == b != c <= d >= e"),
            vec![
                Token::Ident("a"),
                Token::Op("=="),
                Token::Ident("b"),
                Token::Op("!="),
                Token::Ident("c"),
                Token::Op("<="),
                Token::Ident("d"),
                Token::Op(">="),
                Token::Ident("e"),
            ]
        );
    }

    #[test]
    fn member_selection_and_union_bars_lex_as_control_characters() {
        assert_eq!(
            lex("a.b | c"),
            vec![
                Token::Ident("a"),
                Token::Ctrl('.'),
                Token::Ident("b"),
                Token::Ctrl('|'),
                Token::Ident("c"),
            ]
        );
    }

    #[test]
    fn a_decimal_literal_still_munches_its_own_point() {
        assert_eq!(lex("1.5"), vec![Token::Num(NumLiteral::Dec(1.5))]);
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
