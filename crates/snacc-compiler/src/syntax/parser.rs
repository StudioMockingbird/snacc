use crate::syntax::ast::{
    BinaryOp, Expr, ExternFunc, Func, Param, Program, Span, Spanned, TypeName, Value,
};
use crate::syntax::lexer::Token;
use chumsky::{input::ValueInput, prelude::*};
use std::collections::HashMap;

pub fn expr_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<Expr<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    recursive(|expr| {
        let inline_expr = recursive(|inline_expr| {
            let val = select! {
                Token::Nil => Expr::Value(Value::Nil),
                Token::Bool(x) => Expr::Value(Value::Bool(x)),
                Token::Num(n) => Expr::Value(Value::Num(n)),
                Token::Str(s) => Expr::Value(Value::Str(s)),
            }
            .labelled("value");

            let ident = select! { Token::Ident(ident) => ident }.labelled("identifier");

            let type_name = select! {
                Token::TyDec64 => TypeName::Dec64,
                Token::TyInt64 => TypeName::Int64,
                Token::TyBool => TypeName::Bool,
                Token::TyNil => TypeName::Nil,
            }
            .labelled("type name")
            .boxed();

            let items = expr
                .clone()
                .separated_by(just(Token::Ctrl(',')))
                .allow_trailing()
                .collect::<Vec<_>>();

            let let_ = just(Token::Let)
                .ignore_then(ident)
                .then_ignore(just(Token::Ctrl(':')))
                .then(type_name.clone())
                .then_ignore(just(Token::Op("=")))
                .then(inline_expr)
                .then_ignore(just(Token::Ctrl(';')))
                .then(expr.clone())
                .map(|(((name, ty), val), body)| {
                    Expr::Let(name, ty, Box::new(val), Box::new(body))
                });

            let list = items
                .clone()
                .map(Expr::List)
                .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')));

            let atom = val
                .or(ident.map(Expr::Local))
                .or(let_)
                .or(list)
                .or(just(Token::Print)
                    .ignore_then(
                        expr.clone()
                            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))),
                    )
                    .map(|expr| Expr::Print(Box::new(expr))))
                .map_with(|expr, e| (expr, e.span()))
                .or(expr
                    .clone()
                    .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))))
                .recover_with(via_parser(nested_delimiters(
                    Token::Ctrl('('),
                    Token::Ctrl(')'),
                    [(Token::Ctrl('['), Token::Ctrl(']'))],
                    |span| (Expr::Error, span),
                )))
                .recover_with(via_parser(nested_delimiters(
                    Token::Ctrl('['),
                    Token::Ctrl(']'),
                    [(Token::Ctrl('('), Token::Ctrl(')'))],
                    |span| (Expr::Error, span),
                )))
                .boxed();

            let call = atom.foldl_with(
                items
                    .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
                    .map_with(|args, e| (args, e.span()))
                    .repeated(),
                |f, args, e| (Expr::Call(Box::new(f), args), e.span()),
            );

            let op = just(Token::Op("*"))
                .to(BinaryOp::Mul)
                .or(just(Token::Op("/")).to(BinaryOp::Div));
            let product = call
                .clone()
                .foldl_with(op.then(call).repeated(), |a, (op, b), e| {
                    (Expr::Binary(Box::new(a), op, Box::new(b)), e.span())
                });

            let op = just(Token::Op("+"))
                .to(BinaryOp::Add)
                .or(just(Token::Op("-")).to(BinaryOp::Sub));
            let sum = product
                .clone()
                .foldl_with(op.then(product).repeated(), |a, (op, b), e| {
                    (Expr::Binary(Box::new(a), op, Box::new(b)), e.span())
                });

            let op = just(Token::Op("=="))
                .to(BinaryOp::Eq)
                .or(just(Token::Op("!=")).to(BinaryOp::NotEq))
                .or(just(Token::Op("<")).to(BinaryOp::Less))
                .or(just(Token::Op("<=")).to(BinaryOp::LessEq))
                .or(just(Token::Op(">")).to(BinaryOp::Greater))
                .or(just(Token::Op(">=")).to(BinaryOp::GreaterEq));
            let compare = sum
                .clone()
                .foldl_with(op.then(sum).repeated(), |a, (op, b), e| {
                    (Expr::Binary(Box::new(a), op, Box::new(b)), e.span())
                });

            compare.labelled("expression").as_context()
        });

        let else_tail = recursive(|tail| {
            let elseif = just(Token::ElseIf)
                .ignore_then(expr.clone())
                .then_ignore(just(Token::Then))
                .then(expr.clone())
                .then(tail)
                .map_with(|((condition, branch), otherwise), e| {
                    (
                        Expr::If(Box::new(condition), Box::new(branch), Box::new(otherwise)),
                        e.span(),
                    )
                });
            elseif.or(just(Token::Else).ignore_then(expr.clone()))
        });

        let if_ = just(Token::If)
            .ignore_then(expr.clone())
            .then_ignore(just(Token::Then))
            .then(expr.clone())
            .then(else_tail)
            .then_ignore(just(Token::End))
            .map_with(|((condition, branch), otherwise), e| {
                (
                    Expr::If(Box::new(condition), Box::new(branch), Box::new(otherwise)),
                    e.span(),
                )
            });

        let while_ = just(Token::While)
            .ignore_then(expr.clone())
            .then_ignore(just(Token::Do))
            .then(expr.clone())
            .then_ignore(just(Token::End))
            .map_with(|(condition, body), e| {
                (Expr::While(Box::new(condition), Box::new(body)), e.span())
            });

        if_.or(while_)
            .or(inline_expr.clone())
            .recover_with(skip_then_retry_until(
                any().ignored(),
                one_of([
                    Token::Ctrl(';'),
                    Token::Ctrl(')'),
                    Token::Ctrl(']'),
                    Token::Extern,
                    Token::Fun,
                    Token::While,
                    Token::Do,
                    Token::Then,
                    Token::ElseIf,
                    Token::Else,
                    Token::End,
                ])
                .ignored(),
            ))
            .foldl_with(
                just(Token::Ctrl(';'))
                    .ignore_then(
                        one_of([
                            Token::Fun,
                            Token::Extern,
                            Token::While,
                            Token::ElseIf,
                            Token::Else,
                            Token::End,
                        ])
                        .not()
                        .ignore_then(expr)
                        .or_not(),
                    )
                    .repeated(),
                |a, b, e| match b {
                    Some(b) => (Expr::Then(Box::new(a), Box::new(b)), e.span()),
                    None => a,
                },
            )
    })
}

enum Item<'src> {
    Func(Spanned<&'src str>, Func<'src>),
    Extern(Spanned<&'src str>, ExternFunc<'src>),
    Expr(Spanned<Expr<'src>>),
}

pub fn program_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Program<'src>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let ident = select! { Token::Ident(ident) => ident };

    let type_name = select! {
        Token::TyDec64 => TypeName::Dec64,
        Token::TyInt64 => TypeName::Int64,
        Token::TyBool => TypeName::Bool,
        Token::TyNil => TypeName::Nil,
    }
    .labelled("type name")
    .boxed();

    let param = ident
        .map_with(|name, e| (name, e.span()))
        .then_ignore(just(Token::Ctrl(':')))
        .then(type_name.clone())
        .map(
            |((name, name_span), ty): ((&'src str, Span), TypeName)| Param {
                name,
                ty,
                span: name_span,
            },
        );

    let args = param
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect()
        .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
        .labelled("function args");

    let func = just(Token::Fun)
        .ignore_then(
            ident
                .map_with(|name, e| (name, e.span()))
                .labelled("function name"),
        )
        .then(args.clone())
        .then_ignore(just(Token::Ctrl(':')))
        .then(type_name.clone())
        .then_ignore(just(Token::Do))
        .then(expr_parser().then_ignore(just(Token::End)))
        .map_with(|(((name, args), ret), body), e| {
            Item::Func(
                name,
                Func {
                    args,
                    ret,
                    span: e.span(),
                    body,
                },
            )
        })
        .labelled("function");

    let extern_func = just(Token::Extern)
        .ignore_then(just(Token::Rust))
        .ignore_then(select! { Token::Str(symbol) => symbol }.labelled("link symbol"))
        .then_ignore(just(Token::Fun))
        .then(
            ident
                .map_with(|name, e| (name, e.span()))
                .labelled("function name"),
        )
        .then(args)
        .then_ignore(just(Token::Ctrl(':')))
        .then(type_name)
        .map_with(|(((symbol, name), args), ret), e| {
            Item::Extern(
                name,
                ExternFunc {
                    symbol,
                    args,
                    ret,
                    span: e.span(),
                },
            )
        })
        .labelled("external Rust function");

    let item = extern_func.or(func).or(expr_parser().map(Item::Expr));

    item.repeated()
        .collect::<Vec<_>>()
        .validate(|items, _, emitter| {
            let mut funcs = HashMap::new();
            let mut externs = HashMap::new();
            let mut link_names = HashMap::new();
            let mut body: Option<Spanned<Expr<'src>>> = None;
            for item in items {
                match item {
                    Item::Func((name, name_span), function) => {
                        if funcs.contains_key(name) || externs.contains_key(name) {
                            emitter.emit(Rich::custom(
                                name_span,
                                format!("Function '{name}' already exists"),
                            ));
                        } else {
                            funcs.insert(name, function);
                        }
                    }
                    Item::Extern((name, name_span), function) => {
                        if funcs.contains_key(name) || externs.contains_key(name) {
                            emitter.emit(Rich::custom(
                                name_span,
                                format!("Function '{name}' already exists"),
                            ));
                            continue;
                        }
                        if let Some(previous) = link_names.insert(function.symbol, name_span) {
                            emitter.emit(Rich::custom(
                                function.span,
                                format!(
                                    "External link symbol '{}' is already declared at {}..{}",
                                    function.symbol, previous.start, previous.end
                                ),
                            ));
                        }
                        externs.insert(name, function);
                    }
                    Item::Expr(expression) => {
                        body = Some(match body {
                            None => expression,
                            Some(previous) => {
                                let span = (previous.1.start..expression.1.end).into();
                                (Expr::Then(Box::new(previous), Box::new(expression)), span)
                            }
                        });
                    }
                }
            }
            Program {
                funcs,
                externs,
                body,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::lexer;

    fn assert_parses(source: &str) {
        let (tokens, lex_errors) = lexer::lexer().parse(source).into_output_errors();
        assert!(lex_errors.is_empty(), "lex errors: {lex_errors:?}");
        let tokens = tokens.unwrap();
        let (_, parse_errors) = program_parser()
            .parse(
                tokens
                    .as_slice()
                    .map((source.len()..source.len()).into(), |(token, span)| {
                        (token, span)
                    }),
            )
            .into_output_errors();
        assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    }

    #[test]
    fn parses_lua_style_blocks_and_conditions() {
        assert_parses(
            "fun isfoo(a: Int64): Bool do\n    a == 0\nend\n\nif x > 0 then\n    \"yes\"\nelseif x == 0 then \"maybe\"\nelse\n    \"no\"\nend",
        );
    }

    #[test]
    fn newlines_are_not_significant() {
        assert_parses(
            "fun isfoo(a: Int64): Bool do a == 0 end if x > 0 then \"yes\" elseif x == 0 then \"maybe\" else \"no\" end",
        );
    }

    #[test]
    fn parses_while_do_as_an_expression() {
        assert_parses("print(while false do 7 end)");
        assert_parses("while false do print(1) end; print(2)");
    }

    #[test]
    fn parses_typed_rust_bridge_declaration() {
        assert_parses(
            "extern rust \"snacc_user_double\" fun rust_double(value: Int64): Int64\nprint(rust_double(2))",
        );
    }
}
