use crate::syntax::ast::{Span, Spanned};
use chumsky::prelude::*;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Token<'src> {
    Bool(bool),
    Num(f64, bool),
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
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Token::Bool(x) => write!(f, "{x}"),
            Token::Num(n, _) => write!(f, "{n}"),
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
        }
    }
}

pub fn lexer<'src>()
-> impl Parser<'src, &'src str, Vec<Spanned<Token<'src>>>, extra::Err<Rich<'src, char, Span>>> {
    let num = text::int(10)
        .then(just('.').then(text::digits(10)).or_not())
        .to_slice()
        .map(|slice: &str| {
            let value = slice.parse::<f64>().expect("lexer produced a valid number");
            Token::Num(value, slice.contains('.'))
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

    let ctrl = one_of("()[];,:").map(Token::Ctrl);

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
