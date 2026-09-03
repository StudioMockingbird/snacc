use crate::syntax::ast::{
    Arg, BinaryOp, Block, BlockElement, Condition, Expr, ExternFunc, FieldDecl, Func, IfForm,
    MethodDecl, Param, ParamMode, PlacePath, PlaceRootName, Program, Span, Spanned, TypeBody,
    TypeDecl, TypeName, TypeRef, TypeTest, UnionMemberDecl, Value,
};
use crate::syntax::lexer::Token;
use chumsky::{input::ValueInput, prelude::*};
use std::collections::HashMap;

fn builtin_type_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, TypeName, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    select! {
        Token::TyDec64 => TypeName::Dec64,
        Token::TyInt64 => TypeName::Int64,
        Token::TyBool => TypeName::Bool,
        Token::TyNil => TypeName::Nil,
        Token::TyUInt8 => TypeName::UInt8,
        Token::TyUInt16 => TypeName::UInt16,
        Token::TyUInt32 => TypeName::UInt32,
        Token::TyUInt64 => TypeName::UInt64,
        Token::TyFloat32 => TypeName::Float32,
    }
    .labelled("built-in type name")
}

fn name_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<&'src str>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    select! { Token::Ident(ident) => ident }
        .map_with(|name, e| (name, e.span()))
        .labelled("identifier")
}

/// Specification 011 section 5: the permitted declaration sites, named by the
/// diagnostic every other type position produces.
const REFERENCE_OUTSIDE_A_PARAMETER: &str = "'Ref<T>' is only valid as the direct type of a function, method, or Rust \
     bridge parameter; a reference is not storable, so it cannot be a result, a \
     local, a field, a represented type, or a union member";

const NESTED_REFERENCE: &str =
    "'Ref<T>' cannot contain another reference; a reference parameter refers to a value type";

/// `primary-value-type = builtin-value-type | qualified-name | "(", sum-type,
/// ")"` and `sum-type = primary-value-type, { "|", primary-value-type }`
/// (Specification 018 section 3). A single primary with no `|` collapses to
/// that primary directly rather than a one-member [`TypeRef::Sum`], so
/// `TypeRef::Sum` always syntactically holds at least two members; resolution
/// still re-validates this after flattening, since a parenthesized group can
/// supply more than one. `Ref<T>` is never a primary: it is not itself a
/// value-type member (section 3), so it simply cannot appear here, and any
/// attempt fails to parse rather than being silently accepted.
fn sum_type_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<TypeRef<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    recursive(|sum_type| {
        let primary = builtin_type_parser()
            .map(TypeRef::Builtin)
            .or(name_parser()
                .separated_by(just(Token::Ctrl('.')))
                .at_least(1)
                .collect::<Vec<_>>()
                .map(TypeRef::Named))
            .map_with(|ty, e| (ty, e.span()))
            .or(sum_type
                .clone()
                .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')'))))
            .labelled("type")
            .boxed();

        primary
            .clone()
            .then(
                just(Token::Ctrl('|'))
                    .ignore_then(primary)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map_with(|(first, rest), e| {
                if rest.is_empty() {
                    first
                } else {
                    let mut members = vec![first];
                    members.extend(rest);
                    (TypeRef::Sum(members), e.span())
                }
            })
            .boxed()
    })
}

/// `"Ref", "<", sum-type, ">"`, wrapped so the referent's span is preserved
/// and a nested reference is reported as such instead of as a missing type.
/// Specification 018 section 3 widens the referent from a single value type
/// to a full sum type, so `Ref<Byte | Nil>` parses the same way `Ref<Byte>`
/// always has.
fn reference_type_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<TypeRef<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let nested = bracketed_reference(sum_type_parser()).validate(|ty, e, emitter| {
        emitter.emit(Rich::custom(e.span(), NESTED_REFERENCE.to_string()));
        ty
    });
    bracketed_reference(nested.or(sum_type_parser())).boxed()
}

fn bracketed_reference<'tokens, 'src: 'tokens, I, P>(
    referent: P,
) -> impl Parser<'tokens, I, Spanned<TypeRef<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
    P: Parser<'tokens, I, Spanned<TypeRef<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>>
        + Clone,
{
    just(Token::Ref)
        .ignore_then(just(Token::Op("<")))
        .ignore_then(referent)
        .then_ignore(just(Token::Op(">")))
}

/// Every type position except a parameter. `Ref<T>` is still recognised here so
/// it is rejected with the reason rather than with a generic "expected type",
/// and the referent stands in for it so one violation yields one diagnostic.
fn type_ref_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Spanned<TypeRef<'src>>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    reference_type_parser()
        .validate(|ty, e, emitter| {
            emitter.emit(Rich::custom(
                e.span(),
                REFERENCE_OUTSIDE_A_PARAMETER.to_string(),
            ));
            ty
        })
        .or(sum_type_parser())
        .boxed()
}

/// `parameter = identifier, ":", ( value-type | reference-parameter-type )`.
fn param_type_parser<'tokens, 'src: 'tokens, I>() -> impl Parser<
    'tokens,
    I,
    (ParamMode, Spanned<TypeRef<'src>>),
    extra::Err<Rich<'tokens, Token<'src>, Span>>,
> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    reference_type_parser()
        .map(|ty| (ParamMode::Reference, ty))
        .or(sum_type_parser().map(|ty| (ParamMode::Value, ty)))
        .boxed()
}

/// `place = ( identifier | "self" ), { ".", identifier }` (Specification 012
/// section 4).
fn place_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, PlacePath<'src>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    select! {
        Token::Ident(name) => PlaceRootName::Name(name),
        Token::SelfKw => PlaceRootName::SelfRef,
    }
    .map_with(|root, e| (root, e.span()))
    .then(
        just(Token::Ctrl('.'))
            .ignore_then(name_parser())
            .repeated()
            .collect::<Vec<_>>(),
    )
    .map_with(|((root, root_span), fields), e| PlacePath {
        root,
        root_span,
        fields,
        span: e.span(),
    })
    .labelled("place")
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

        let name = name_parser();

        let items = expr
            .clone()
            .separated_by(just(Token::Ctrl(',')))
            .allow_trailing()
            .collect::<Vec<_>>();

        let list = items
            .clone()
            .map(Expr::List)
            .delimited_by(just(Token::Ctrl('[')), just(Token::Ctrl(']')));

        // `argument = [ identifier, ":" ], expression`. A named argument is
        // recorded without deciding whether the call head can accept one.
        let argument = name
            .clone()
            .then_ignore(just(Token::Ctrl(':')))
            .or_not()
            .then(expr.clone())
            .map(|(name, value)| Arg { name, value });
        let arguments = argument
            .separated_by(just(Token::Ctrl(',')))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
            .map_with(|args, e| (args, e.span()));

        let atom = val
            .or(select! { Token::SelfKw => Expr::SelfRef })
            // A built-in type name is a call head only: `Int64(id)` unwraps one
            // represented layer. Checking rejects it in any other position.
            .or(builtin_type_parser().map(Expr::BuiltinType))
            .or(name.clone().map(|(name, _)| Expr::Local(name)))
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

        // `postfix = atom, { arguments | member-suffix }`. Nothing here decides
        // whether a name is a type, field, constructor, or method.
        enum Suffix<'src> {
            Call(Spanned<Vec<Arg<'src>>>),
            Member(Spanned<&'src str>),
        }
        let suffix = arguments
            .map(Suffix::Call)
            .or(just(Token::Ctrl('.')).ignore_then(name).map(Suffix::Member));
        let postfix = atom.foldl_with(suffix.repeated(), |base, suffix, e| {
            let expr = match suffix {
                Suffix::Call(args) => Expr::Call(Box::new(base), args),
                Suffix::Member(name) => Expr::Member(Box::new(base), name),
            };
            (expr, e.span())
        });

        let op = just(Token::Op("*"))
            .to(BinaryOp::Mul)
            .or(just(Token::Op("/")).to(BinaryOp::Div));
        let product = postfix
            .clone()
            .foldl_with(op.then(postfix).repeated(), |a, (op, b), e| {
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
    .boxed()
}

/// `condition = type-test | expression`. A type test is valid only as a
/// complete `if`/`elseif` condition (Specification 010 section 12.2), which
/// this shape enforces structurally.
fn condition_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Condition<'src>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    // Every built-in type name is a reserved word rather than an identifier,
    // so a member path accepts each one as its own segment spelling.
    // Specification 018 section 3 extends the tested member to any direct
    // sum member, including a built-in scalar (`is UInt8(byte)`), not only
    // `Nil` as before.
    let segment = select! {
        Token::Ident(name) => name,
        Token::TyNil => "Nil",
        Token::TyDec64 => "Dec64",
        Token::TyInt64 => "Int64",
        Token::TyBool => "Bool",
        Token::TyUInt8 => "UInt8",
        Token::TyUInt16 => "UInt16",
        Token::TyUInt32 => "UInt32",
        Token::TyUInt64 => "UInt64",
        Token::TyFloat32 => "Float32",
    }
    .map_with(|name, e| (name, e.span()));

    let type_test = place_parser()
        .then_ignore(just(Token::Is))
        .then(
            segment
                .separated_by(just(Token::Ctrl('.')))
                .at_least(1)
                .collect::<Vec<_>>()
                .map_with(|member, e| (member, e.span())),
        )
        .then(
            name_parser()
                .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
                .or_not(),
        )
        .map_with(|((place, (member, member_span)), binding), e| {
            Condition::TypeTest(TypeTest {
                place,
                member,
                member_span,
                binding,
                span: e.span(),
            })
        })
        .labelled("type test");

    type_test.or(expr_parser().map(Condition::Expr)).boxed()
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

    let name = name_parser();
    let expr = expr_parser();

    let let_ = just(Token::Let)
        .ignore_then(just(Token::Mut).or_not().map(|m| m.is_some()))
        .then(name)
        .then_ignore(just(Token::Ctrl(':')))
        .then(type_ref_parser())
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

    // The single `=` token only begins an assignment when a place sits at
    // block-element start; `==` is a distinct token, so no lookahead is needed
    // beyond chumsky's ordinary backtracking into the expression alternative.
    let assign = place_parser()
        .then_ignore(just(Token::Op("=")))
        .then(expr.clone())
        .map(|(place, value)| BlockElement::Assign { place, value })
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

    let arm = condition_parser()
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
    Type(TypeDecl<'src>),
    Method(MethodDecl<'src>),
    Element(Spanned<BlockElement<'src>>),
}

pub fn program_parser<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Program<'src>, extra::Err<Rich<'tokens, Token<'src>, Span>>> + Clone
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = Span>,
{
    let ident = select! { Token::Ident(ident) => ident };
    let name = name_parser();
    let type_ref = type_ref_parser();

    let param = ident
        .map_with(|name, e| (name, e.span()))
        .then_ignore(just(Token::Ctrl(':')))
        .then(param_type_parser())
        .map(|((name, name_span), (mode, ty))| Param {
            name,
            mode,
            ty,
            span: name_span,
        });

    let args = param
        .separated_by(just(Token::Ctrl(',')))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Ctrl('(')), just(Token::Ctrl(')')))
        .labelled("function args")
        .boxed();

    // `: type` is optional: omitting it declares no result.
    let result = just(Token::Ctrl(':'))
        .ignore_then(type_ref.clone())
        .or_not()
        .boxed();

    let field = name
        .clone()
        .then_ignore(just(Token::Ctrl(':')))
        .then(type_ref.clone())
        .then_ignore(just(Token::Ctrl(',')).or_not())
        .map(|((name, name_span), ty)| FieldDecl {
            name,
            name_span,
            ty,
        })
        .labelled("field declaration");

    let struct_body = just(Token::Struct)
        .ignore_then(field.repeated().collect::<Vec<_>>())
        .then_ignore(just(Token::End))
        .boxed();

    // A bare alternative is exactly an empty inline struct member.
    let union_member = just(Token::Ctrl('|'))
        .ignore_then(
            select! {
                Token::TyNil => ("Nil", true),
                Token::Ident(name) => (name, false),
            }
            .map_with(|member, e| (member, e.span())),
        )
        .then(
            just(Token::Is)
                .ignore_then(struct_body.clone())
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .map(|(((name, nil), name_span), fields)| UnionMemberDecl {
            name,
            name_span,
            nil,
            fields,
        })
        .labelled("union member");

    let union_body = just(Token::Union)
        .ignore_then(union_member.repeated().at_least(1).collect::<Vec<_>>())
        .then_ignore(just(Token::End))
        .boxed();

    let type_decl = just(Token::Type)
        .ignore_then(name.clone().labelled("type name"))
        .then_ignore(just(Token::Is))
        .then(
            struct_body
                .map(TypeBody::Struct)
                .or(union_body.map(TypeBody::Union))
                .or(type_ref.clone().map(TypeBody::Represented)),
        )
        .map_with(|((name, name_span), body), e| {
            Item::Type(TypeDecl {
                name,
                name_span,
                body,
                span: e.span(),
            })
        })
        .labelled("type declaration");

    let method_decl = just(Token::Method)
        .ignore_then(
            name.clone()
                .separated_by(just(Token::Ctrl('.')))
                .at_least(1)
                .collect::<Vec<_>>()
                .labelled("method name"),
        )
        .then(args.clone())
        .then(result.clone())
        .then_ignore(just(Token::Do))
        .then(block_parser().then_ignore(just(Token::End)))
        .map_with(|(((path, args), ret), body), e| {
            Item::Method(MethodDecl {
                path,
                args,
                ret,
                span: e.span(),
                body,
            })
        })
        .labelled("method declaration");

    let func = just(Token::Fun)
        .ignore_then(name.clone().labelled("function name"))
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
        .then(name.labelled("function name"))
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

    // None of `fun`, `extern`, `type`, or `method` can begin a block element, so
    // declarations and executable elements interleave freely at the top level.
    let item = extern_func
        .or(func)
        .or(type_decl)
        .or(method_decl)
        .or(block_element_parser().map(Item::Element));

    item.repeated()
        .collect::<Vec<_>>()
        .map_with(|items, e| (items, e.span()))
        .validate(|(items, span), _, emitter| {
            let mut funcs = HashMap::new();
            let mut externs = HashMap::new();
            let mut link_names = HashMap::new();
            let mut types = Vec::new();
            let mut methods = Vec::new();
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
                    Item::Type(declaration) => types.push(declaration),
                    Item::Method(declaration) => methods.push(declaration),
                    Item::Element(element) => elements.push(element),
                }
            }
            Program {
                funcs,
                externs,
                types,
                methods,
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

    fn builtin(ty: &Spanned<TypeRef<'_>>) -> TypeName {
        match &ty.0 {
            TypeRef::Builtin(name) => *name,
            other => panic!("expected a built-in type, got {other:?}"),
        }
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
        assert_eq!(
            builtin(program.funcs["double"].ret.as_ref().unwrap()),
            TypeName::Int64
        );
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
        assert_eq!(
            builtin(program.externs["rust_double"].ret.as_ref().unwrap()),
            TypeName::Int64
        );
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

    /// Specification 009 conformance 1: every new type name is accepted in a
    /// binding, a parameter, a function result, and a bridge declaration.
    #[test]
    fn parses_every_new_type_name_in_every_type_position() {
        for (name, expected) in [
            ("UInt8", TypeName::UInt8),
            ("UInt16", TypeName::UInt16),
            ("UInt32", TypeName::UInt32),
            ("UInt64", TypeName::UInt64),
            ("Float32", TypeName::Float32),
        ] {
            let source = format!(
                "extern rust \"snacc_user_edge\" fun edge(value: {name}): {name}\n\
                 fun identity(value: {name}): {name} do value end"
            );
            let program = parse(&source);
            assert_eq!(builtin(&program.funcs["identity"].args[0].ty), expected);
            assert_eq!(
                builtin(program.funcs["identity"].ret.as_ref().unwrap()),
                expected
            );
            assert_eq!(builtin(&program.externs["edge"].args[0].ty), expected);
            assert_eq!(
                builtin(program.externs["edge"].ret.as_ref().unwrap()),
                expected
            );
        }
        let program = parse("let byte: UInt8 = 1u8 let ratio: Float32 = 0.5f32");
        let BlockElement::Let { ty, .. } = &program.body.elements[0].0 else {
            panic!("expected a declaration");
        };
        assert_eq!(builtin(ty), TypeName::UInt8);
    }

    #[test]
    fn rejects_semicolons() {
        assert_rejects("while false do print(1) end; print(2)");
        assert_rejects("let x: Int64 = 1; print(x)");
    }

    // Specification 010 section 5: type, struct, union, method, and type-test
    // syntax.

    #[test]
    fn parses_a_represented_type_declaration() {
        let program = parse("type UserId is Int64");
        assert_eq!(program.types.len(), 1);
        assert_eq!(program.types[0].name, "UserId");
        assert!(matches!(program.types[0].body, TypeBody::Represented(_)));
    }

    #[test]
    fn parses_a_struct_with_and_without_a_trailing_comma() {
        for source in [
            "type Point is struct x: Dec64, y: Dec64, end",
            "type Point is struct x: Dec64, y: Dec64 end",
            "type Point is struct\n    x: Dec64\n    y: Dec64\nend",
        ] {
            let program = parse(source);
            let TypeBody::Struct(fields) = &program.types[0].body else {
                panic!("expected a struct body for {source}");
            };
            assert_eq!(fields.len(), 2, "{source}");
            assert_eq!(fields[0].name, "x");
            assert_eq!(fields[1].name, "y");
        }
    }

    #[test]
    fn parses_an_empty_struct() {
        let program = parse("type Marker is struct end");
        let TypeBody::Struct(fields) = &program.types[0].body else {
            panic!("expected a struct body");
        };
        assert!(fields.is_empty());
    }

    #[test]
    fn parses_bare_inline_and_nil_union_members() {
        let program = parse(
            "type Shape is union\n\
             | Circle is struct radius: Int64, end\n\
             | Point\n\
             | Nil\n\
             end",
        );
        let TypeBody::Union(members) = &program.types[0].body else {
            panic!("expected a union body");
        };
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].fields.len(), 1);
        assert!(members[1].fields.is_empty() && !members[1].nil);
        assert!(members[2].nil && members[2].name == "Nil");
    }

    #[test]
    fn parses_top_level_and_member_method_receivers() {
        let program = parse(
            "method Point.length(): Dec64 do 1.0 end\n\
             method Shape.Circle.area(): Int64 do 1 end",
        );
        assert_eq!(program.methods.len(), 2);
        let (receiver, name) = program.methods[0].split().expect("two components");
        assert_eq!(receiver.len(), 1);
        assert_eq!(name.0, "length");
        let (receiver, name) = program.methods[1].split().expect("three components");
        assert_eq!(
            receiver.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec!["Shape", "Circle"]
        );
        assert_eq!(name.0, "area");
    }

    #[test]
    fn parses_named_constructor_arguments_and_nested_postfix_chains() {
        let program = parse("print(Point(x: 3.0, y: 4.0).x)\nprint(a.b.c(1).d)");
        assert_eq!(program.body.elements.len(), 2);
    }

    #[test]
    fn parses_field_assignment_through_a_field_path() {
        let program = parse("entity.position.x = 1.0");
        let BlockElement::Assign { place, .. } = &program.body.elements[0].0 else {
            panic!("expected an assignment");
        };
        assert_eq!(place.fields.len(), 2);
        assert!(matches!(place.root, PlaceRootName::Name("entity")));
    }

    #[test]
    fn parses_whole_self_assignment_inside_a_method() {
        let program = parse("method Point.reset() do self = Point(x: 0.0, y: 0.0) end");
        let BlockElement::Assign { place, .. } = &program.methods[0].body.elements[0].0 else {
            panic!("expected an assignment");
        };
        assert!(matches!(place.root, PlaceRootName::SelfRef));
        assert!(place.fields.is_empty());
    }

    #[test]
    fn parses_type_tests_with_and_without_a_binding() {
        let program = parse(
            "if shape is Shape.Circle(circle) then print(1) elseif shape is Nil then print(2) end",
        );
        let BlockElement::If(form) = &program.body.elements[0].0 else {
            panic!("expected an if form");
        };
        let Condition::TypeTest(first) = &form.arms[0].0 else {
            panic!("expected a type test");
        };
        assert_eq!(
            first.member.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec!["Shape", "Circle"]
        );
        assert_eq!(first.binding.map(|(name, _)| name), Some("circle"));
        let Condition::TypeTest(second) = &form.arms[1].0 else {
            panic!("expected a type test");
        };
        assert_eq!(second.member[0].0, "Nil");
        assert!(second.binding.is_none());
    }

    #[test]
    fn a_comparison_still_parses_as_an_ordinary_condition() {
        let program = parse("if x > 0 then print(1) end");
        let BlockElement::If(form) = &program.body.elements[0].0 else {
            panic!("expected an if form");
        };
        assert!(matches!(form.arms[0].0, Condition::Expr(_)));
    }

    #[test]
    fn parses_a_qualified_type_in_every_type_position() {
        assert_parses(
            "type Shape is union | Circle is struct radius: Int64, end end\n\
             type Holder is struct shape: Shape.Circle, end\n\
             fun take(value: Shape.Circle): Shape.Circle do value end\n\
             let held: Shape.Circle = Shape.Circle(radius: 1)",
        );
    }

    #[test]
    fn parses_a_builtin_type_name_as_an_unwrapping_call_head() {
        let program = parse("type UserId is Int64\nlet id: UserId = UserId(1)\nprint(Int64(id))");
        assert_eq!(program.body.elements.len(), 2);
    }

    #[test]
    fn rejects_malformed_declaration_delimiters() {
        for source in [
            "type Point is struct x: Dec64,",
            "type Shape is union | end",
            "type Shape is union end",
            "method Point.length(): Dec64 do 1.0",
            "type is Int64",
            "method () do end",
        ] {
            assert_rejects(source);
        }
    }

    #[test]
    fn mut_is_reserved_outside_a_declaration() {
        assert_rejects("fun f(mut value: Int64): Int64 do value end");
        assert_rejects("let mut: Int64 = 1");
    }

    // Specification 011 sections 4-5: `Ref<T>` parses only as a direct
    // parameter type.

    fn parse_errors(source: &str) -> Vec<String> {
        let (tokens, lex_errors) = lexer::lexer().parse(source).into_output_errors();
        assert!(lex_errors.is_empty(), "lex errors: {lex_errors:?}");
        let tokens = tokens.unwrap();
        let (_, errors) = program_parser()
            .parse(
                tokens
                    .as_slice()
                    .map((source.len()..source.len()).into(), |(token, span)| {
                        (token, span)
                    }),
            )
            .into_output_errors();
        errors.iter().map(ToString::to_string).collect()
    }

    fn assert_parse_error_contains(source: &str, needle: &str) {
        let errors = parse_errors(source);
        assert!(
            errors.iter().any(|error| error.contains(needle)),
            "expected a parse error containing {needle:?} for {source}, got: {errors:?}"
        );
    }

    #[test]
    fn ref_is_a_reserved_word_and_not_an_identifier() {
        assert_rejects("let Ref: Int64 = 1");
        assert_rejects("fun Ref(value: Int64): Int64 do value end");
    }

    #[test]
    fn parses_a_reference_parameter_in_every_permitted_declaration() {
        for source in [
            "fun add_into(x: Int64, y: Int64, result: Ref<Int64>) do result = x + y end",
            "type Point is struct x: Dec64, end\n\
             method Point.give(other: Ref<Dec64>) do other = self.x end",
            "extern rust \"snacc_user_bump\" fun bump(value: Ref<Int64>)",
        ] {
            assert_parses(source);
        }
    }

    #[test]
    fn a_reference_parameter_records_its_mode_and_referent_type() {
        let program = parse("fun f(a: Int64, b: Ref<Int64>) do b = a end");
        let args = &program.funcs["f"].args;
        assert_eq!(args[0].mode, ParamMode::Value);
        assert_eq!(args[1].mode, ParamMode::Reference);
        assert_eq!(builtin(&args[1].ty), TypeName::Int64);
    }

    #[test]
    fn a_user_defined_referent_keeps_its_written_path() {
        let program = parse(
            "type Shape is union | Circle is struct radius: Int64, end | Nil end\n\
             fun grow(shape: Ref<Shape.Circle>) do shape.radius = 1 end",
        );
        let TypeRef::Named(segments) = &program.funcs["grow"].args[0].ty.0 else {
            panic!("expected a qualified referent path");
        };
        assert_eq!(
            segments.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec!["Shape", "Circle"]
        );
    }

    #[test]
    fn rejects_a_reference_in_every_other_type_position() {
        for source in [
            "fun f(value: Int64): Ref<Int64> do value end",
            "method Point.f(): Ref<Int64> do 1 end",
            "extern rust \"snacc_user_f\" fun f(value: Int64): Ref<Int64>",
            "let saved: Ref<Int64> = 1",
            "type Holder is struct value: Ref<Int64>, end",
            "type Alias is Ref<Int64>",
            "type Shape is union | Circle is struct radius: Ref<Int64>, end | Nil end",
        ] {
            assert_parse_error_contains(source, "only valid as the direct type of a function");
        }
    }

    #[test]
    fn rejects_a_nested_reference() {
        assert_parse_error_contains(
            "fun f(value: Ref<Ref<Int64>>) do print(1) end",
            "cannot contain another reference",
        );
    }

    #[test]
    fn rejects_an_authored_self_annotation() {
        // `self` is a keyword, so it can never be written as a parameter name;
        // no author-written `self` type exists to carry `Ref<T>`.
        assert_rejects("method Point.f(self: Ref<Point>) do print(1) end");
    }

    #[test]
    fn ordered_comparisons_still_parse_as_expressions() {
        // Specification 011 section 14: `<` and `>` outside a type position are
        // unchanged, including the two-character forms.
        let program = parse("print(a < b) print(a > b) print(a <= b) print(a >= b)");
        assert_eq!(program.body.elements.len(), 4);
    }

    // Specification 018 sections 3 and 6: `|` forms an inline sum type in
    // every value-type position, and a type test may name any direct member.

    fn sum_member_names(ty: &Spanned<TypeRef<'_>>) -> Vec<String> {
        match &ty.0 {
            TypeRef::Sum(members) => members.iter().map(|(m, _)| m.to_string()).collect(),
            other => panic!("expected an inline sum type, got {other:?}"),
        }
    }

    fn let_ty<'src>(program: &Program<'src>, index: usize) -> Spanned<TypeRef<'src>> {
        let BlockElement::Let { ty, .. } = &program.body.elements[index].0 else {
            panic!("expected a variable declaration");
        };
        (ty.0.clone(), ty.1)
    }

    #[test]
    fn parses_an_inline_sum_type_in_every_value_type_position() {
        for source in [
            "type Point is struct x: Dec64, end\nfun read(): UInt8 | Nil do nil end",
            "type Point is struct x: Dec64, end\nfun take(value: UInt8 | Nil) do print(1) end",
            "type Point is struct x: Dec64, end\nlet result: UInt8 | Nil = nil",
            "type Point is struct x: Dec64, end\ntype Holder is struct value: UInt8 | Nil, end",
            "type Point is struct x: Dec64, end\n\
             extern rust \"snacc_user_maybe\" fun maybe(): UInt8 | Nil",
            "type Point is struct x: Dec64, end\n\
             method Point.length(): UInt8 | Nil do nil end",
        ] {
            assert_parses(source);
        }
    }

    #[test]
    fn a_sum_type_records_every_member_in_source_order() {
        let program = parse("let result: UInt8 | Bool | Nil = nil");
        let ty = let_ty(&program, 0);
        assert_eq!(sum_member_names(&ty), vec!["UInt8", "Bool", "Nil"]);
    }

    #[test]
    fn a_single_member_never_wraps_in_a_sum_node() {
        // A sum-type production with no `|` collapses to its one primary, and
        // a fully parenthesized single member is exactly ordinary grouping,
        // not a one-member sum.
        let program = parse("let value: (UInt8) = 1u8");
        let ty = let_ty(&program, 0);
        assert_eq!(builtin(&ty), TypeName::UInt8);
    }

    #[test]
    fn parses_parenthesized_grouping_with_a_nested_sum_member() {
        // Flattening `(A | B) | C` into one three-member set is a resolution
        // concern; the parser only needs to record the grouped sum as one
        // member of the outer sum, in source order.
        let program = parse("let value: (UInt8 | Bool) | Nil = nil");
        let ty = let_ty(&program, 0);
        let TypeRef::Sum(members) = &ty.0 else {
            panic!("expected an inline sum type");
        };
        assert_eq!(members.len(), 2);
        assert_eq!(sum_member_names(&members[0]), vec!["UInt8", "Bool"]);
        assert_eq!(members[1].0.to_string(), "Nil");
    }

    #[test]
    fn whitespace_around_the_sum_operator_has_no_significance() {
        let tight = parse("let value: UInt8|Bool|Nil = nil");
        let spaced = parse("let value: UInt8 | Bool | Nil = nil");
        assert_eq!(
            sum_member_names(&let_ty(&tight, 0)),
            sum_member_names(&let_ty(&spaced, 0))
        );
    }

    #[test]
    fn rejects_malformed_sum_separators() {
        for source in [
            "let value: | UInt8 = 1u8",
            "let value: UInt8 | = 1u8",
            "let value: UInt8 || Bool = nil",
            "let value: () = 1u8",
        ] {
            assert_rejects(source);
        }
    }

    #[test]
    fn rejects_a_reference_as_a_sum_member() {
        for source in [
            "let value: Ref<UInt8> | Nil = nil",
            "fun f(value: UInt8 | Ref<Nil>) do print(1) end",
        ] {
            assert_rejects(source);
        }
    }

    #[test]
    fn a_reference_referent_may_be_a_sum_type() {
        assert_parses("fun replace(value: Ref<UInt8 | Nil>) do value = nil end");
    }

    #[test]
    fn the_sum_operator_never_parses_as_an_expression_operator() {
        // `|` is recognized only while parsing a type; it adds no value-level
        // operator and does not conflict with expression parsing.
        assert_rejects("print(1 | 2)");
        assert_rejects("let x: Int64 = 1 | 2");
    }

    #[test]
    fn parses_a_type_test_naming_a_builtin_direct_member() {
        let program = parse(
            "fun show(value: UInt8 | Nil) do \
             if value is UInt8(byte) then print(1) elseif value is Nil then print(2) end end",
        );
        let BlockElement::If(form) = &program.funcs["show"].body.elements[0].0 else {
            panic!("expected an if form");
        };
        let Condition::TypeTest(first) = &form.arms[0].0 else {
            panic!("expected a type test");
        };
        assert_eq!(first.member[0].0, "UInt8");
        assert_eq!(first.binding.map(|(name, _)| name), Some("byte"));
        let Condition::TypeTest(second) = &form.arms[1].0 else {
            panic!("expected a type test");
        };
        assert_eq!(second.member[0].0, "Nil");
        assert!(second.binding.is_none());
    }
}
