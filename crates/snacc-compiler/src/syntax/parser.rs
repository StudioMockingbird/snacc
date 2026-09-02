use crate::syntax::ast::{
    BinaryOp, Block, BlockElement, Expr, ExternFunc, Func, IfForm, Param, Program, Span, Spanned,
    TypeName, Value,
};
use crate::syntax::lexer::Token;
use chumsky::{input::ValueInput, prelude::*};
use std::collections::HashMap;

fn type_name_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, TypeName, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    select! {
        Token::TyDec64 => TypeName::Dec64,
        Token::TyInt64 => TypeName::Int64,
        Token::TyBool => TypeName::Bool,
        Token::TyNil => TypeName::Nil,
    }
    .labelled("type name")
}

/// Value-producing expressions only. `let`, assignment, `while`, `break`, and
/// `if` are block elements, so none of them can appear in an operand,
/// argument, initializer, or condition position.
pub fn expr_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<Expr<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    recursive(|expr| {
        let val = select! {
            Token::Nil => Expr::Value(Value::Nil),
            Token::Bool(x) => Expr::Value(Value::Bool(x)),
            Token::Num(n) => Expr::Value(Value::Num(n)),
            Token::Str(s) => Expr::Value(Value::Str(s)),
        }
        .labelled("value");

        let ident = select! { Token::Ident(ident) => ident }.labelled("identifier");

        let items = expr
            .clone()
            .separated_by(just(Token::Ctrl(',')))
            .allow_trailing()
            .collect::<Vec<_>>();

        let list = items
            .clone()
            .map(Expr::List)
            .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')));

        let atom = val
            .or(ident.map(Expr::Local))
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
    })
}

/// One block element. Nested blocks (loop bodies, `if` branches) are parsed by
/// recursing through this same parser, so `block_element_parser` owns the whole
/// statement grammar.
pub fn block_element_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<BlockElement<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>>
+ Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let mut element = Recursive::declare();

    // A block runs until the enclosing terminator (`end`, `elseif`, `else`, or
    // end of input); none of those can begin a block element, so `repeated`
    // stops on its own without an explicit guard.
    let block = element
        .clone()
        .repeated()
        .collect::<Vec<_>>()
        .map_with(|elements, e| Block {
            elements,
            span: e.span(),
        });

    let ident = select! { Token::Ident(ident) => ident }.labelled("identifier");
    let name = ident.map_with(|name, e| (name, e.span()));
    let expr = expr_parser();

    // `mut` is recognized positionally after `let` rather than as its own
    // token, so the lexer keeps a single reserved-word table.
    let let_ = just(Token::Let)
        .ignore_then(just(Token::Ident("mut")).or_not().map(|m| m.is_some()))
        .then(name)
        .then_ignore(just(Token::Ctrl(':')))
        .then(type_name_parser())
        .then_ignore(just(Token::Op("=")))
        .then(expr.clone())
        .map(
            |(((mutable, (name, name_span)), ty), value)| BlockElement::Let {
                mutable,
                name,
                name_span,
                ty,
                value,
            },
        )
        .labelled("variable declaration");

    // The single `=` token only begins an assignment when a bare name sits at
    // block-element start; `==` is a distinct token, so no lookahead is needed
    // beyond chumsky's ordinary backtracking into the expression alternative.
    let assign = name
        .then_ignore(just(Token::Op("=")))
        .then(expr.clone())
        .map(|((name, name_span), value)| BlockElement::Assign {
            name,
            name_span,
            value,
        })
        .labelled("assignment");

    let while_ = just(Token::While)
        .ignore_then(expr.clone())
        .then_ignore(just(Token::Do))
        .then(block.clone())
        .then_ignore(just(Token::End))
        .map_with(|(condition, body), e| BlockElement::While {
            condition,
            body,
            span: e.span(),
        })
        .labelled("while statement");

    let break_ = just(Token::Break).map_with(|_, e| BlockElement::Break(e.span()));

    let arm = expr
        .clone()
        .then_ignore(just(Token::Then))
        .then(block.clone());
    let if_ = just(Token::If)
        .ignore_then(arm.clone())
        .then(
            just(Token::ElseIf)
                .ignore_then(arm)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then(just(Token::Else).ignore_then(block).or_not())
        .then_ignore(just(Token::End))
        .map_with(|((first, mut arms), else_branch), e| {
            arms.insert(0, first);
            BlockElement::If(IfForm {
                arms,
                else_branch,
                span: e.span(),
            })
        })
        .labelled("if form");

    element.define(
        let_.or(while_)
            .or(break_)
            .or(if_)
            .or(assign)
            .or(expr.map(BlockElement::Expr))
            .map_with(|element, e| (element, e.span()))
            .boxed(),
    );

    element
}

pub fn block_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Block<'src>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    block_element_parser()
        .repeated()
        .collect::<Vec<_>>()
        .map_with(|elements, e| Block {
            elements,
            span: e.span(),
        })
}

enum Item<'src> {
    Func(Spanned<&'src str>, Func<'src>),
    Extern(Spanned<&'src str>, ExternFunc<'src>),
    Element(Spanned<BlockElement<'src>>),
}

pub fn program_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Program<'src>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let ident = select! { Token::Ident(ident) => ident };
    let type_name = type_name_parser();

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

    // `: type` is optional: omitting it declares no result.
    let result = just(Token::Ctrl(':'))
        .ignore_then(type_name.clone())
        .or_not();

    let func = just(Token::Fun)
        .ignore_then(
            ident
                .map_with(|name, e| (name, e.span()))
                .labelled("function name"),
        )
        .then(args.clone())
        .then(result.clone())
        .then_ignore(just(Token::Do))
        .then(block_parser().then_ignore(just(Token::End)))
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
        .then(result)
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

    // Neither `fun` nor `extern` can begin a block element, so declarations and
    // executable elements interleave freely at the top level.
    let item = extern_func
        .or(func)
        .or(block_element_parser().map(Item::Element));

    item.repeated()
        .collect::<Vec<_>>()
        .map_with(|items, e| (items, e.span()))
        .validate(|(items, span), _, emitter| {
            let mut funcs = HashMap::new();
            let mut externs = HashMap::new();
            let mut link_names = HashMap::new();
            let mut elements = Vec::new();
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
                    Item::Element(element) => elements.push(element),
                }
            }
            Program {
                funcs,
                externs,
                body: Block { elements, span },
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::lexer;

    fn parse(source: &str) -> Program<'_> {
        let (tokens, lex_errors) = lexer::lexer().parse(source).into_output_errors();
        assert!(lex_errors.is_empty(), "lex errors: {lex_errors:?}");
        let tokens = tokens.unwrap();
        let (program, parse_errors) = program_parser()
            .parse(
                tokens
                    .as_slice()
                    .map((source.len()..source.len()).into(), |(token, span)| {
                        (token, span)
                    }),
            )
            .into_output_errors();
        assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
        program.expect("a program without parse errors has a syntax tree")
    }

    fn assert_parses(source: &str) {
        parse(source);
    }

    fn assert_rejects(source: &str) {
        let (tokens, lex_errors) = lexer::lexer().parse(source).into_output_errors();
        if !lex_errors.is_empty() {
            return;
        }
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
        assert!(
            !parse_errors.is_empty(),
            "expected a parse error for: {source}"
        );
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
    fn parses_while_as_a_statement() {
        let program = parse("while false do print(1) end print(2)");
        assert_eq!(program.body.elements.len(), 2);
        assert!(matches!(
            program.body.elements[0].0,
            BlockElement::While { .. }
        ));
        assert!(matches!(program.body.elements[1].0, BlockElement::Expr(_)));
    }

    #[test]
    fn rejects_while_in_an_expression_position() {
        // `while` is a statement: it can never supply an argument, operand,
        // initializer, or condition.
        assert_rejects("print(while false do 7 end)");
        assert_rejects("let x: Int64 = while false do 7 end");
        assert_rejects("1 + while false do 7 end");
    }

    #[test]
    fn rejects_if_in_an_expression_position() {
        // `if` is a block element, never nested inside an expression.
        assert_rejects("print(if true then 1 else 2 end)");
        assert_rejects("let x: Int64 = if true then 1 else 2 end");
    }

    #[test]
    fn parses_break_inside_a_while_body() {
        let program = parse("while true do break end");
        let BlockElement::While { body, .. } = &program.body.elements[0].0 else {
            panic!("expected a while statement");
        };
        assert_eq!(body.elements.len(), 1);
        assert!(matches!(body.elements[0].0, BlockElement::Break(_)));
    }

    #[test]
    fn parses_a_function_without_a_result_type() {
        let program = parse("fun announce(value: Int64) do print(value) end");
        assert!(program.funcs["announce"].ret.is_none());
    }

    #[test]
    fn parses_a_function_with_a_result_type() {
        let program = parse("fun double(value: Int64): Int64 do value * 2 end");
        assert_eq!(program.funcs["double"].ret, Some(TypeName::Int64));
    }

    #[test]
    fn parses_a_bridge_without_a_result_type() {
        let program = parse("extern rust \"snacc_user_log\" fun log(value: Int64)");
        assert!(program.externs["log"].ret.is_none());
    }

    #[test]
    fn parses_typed_rust_bridge_declaration() {
        let program = parse(
            "extern rust \"snacc_user_double\" fun rust_double(value: Int64): Int64\nprint(rust_double(2))",
        );
        assert_eq!(program.externs["rust_double"].ret, Some(TypeName::Int64));
    }

    #[test]
    fn parses_an_if_without_an_else() {
        let program = parse("if true then print(1) end");
        assert_eq!(program.body.elements.len(), 1);
        let BlockElement::If(form) = &program.body.elements[0].0 else {
            panic!("expected an if form");
        };
        assert_eq!(form.arms.len(), 1);
        assert!(form.else_branch.is_none());
    }

    #[test]
    fn parses_an_elseif_chain_as_one_block_element() {
        let program = parse("if x > 0 then print(1) elseif x == 0 then print(2) else print(3) end");
        let BlockElement::If(form) = &program.body.elements[0].0 else {
            panic!("expected an if form");
        };
        assert_eq!(form.arms.len(), 2);
        assert!(form.else_branch.is_some());
    }

    #[test]
    fn parses_declarations_and_assignments() {
        let program = parse("let mut total: Int64 = 1 total = total + 1 print(total)");
        assert_eq!(program.body.elements.len(), 3);
        assert!(matches!(
            program.body.elements[0].0,
            BlockElement::Let { mutable: true, .. }
        ));
        assert!(matches!(
            program.body.elements[1].0,
            BlockElement::Assign { .. }
        ));
    }

    #[test]
    fn a_leading_name_only_begins_an_assignment_on_a_single_equals() {
        // `=` and `==` are distinct tokens, so `x == 1` backtracks out of the
        // assignment alternative into an ordinary comparison expression.
        let program = parse("x == 1");
        assert!(matches!(program.body.elements[0].0, BlockElement::Expr(_)));
        let program = parse("x = 1");
        assert!(matches!(
            program.body.elements[0].0,
            BlockElement::Assign { .. }
        ));
    }

    #[test]
    fn one_line_and_multi_line_block_elements_parse_identically() {
        // Specification 012 section 4: whitespace only separates tokens.
        let one_line = parse("let x: Int64 = 10 print(x)");
        let multi_line = parse("let x: Int64 = 10\nprint(x)");
        assert_eq!(one_line.body.elements.len(), 2);
        assert_eq!(
            format!("{:?}", one_line.body.elements[0].0),
            format!("{:?}", multi_line.body.elements[0].0)
        );
    }

    #[test]
    fn rejects_semicolons() {
        assert_rejects("while false do print(1) end; print(2)");
        assert_rejects("let x: Int64 = 1; print(x)");
    }
}
