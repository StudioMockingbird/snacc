# Specification 027: Boolean and Comparison Operators

Status: Proposed

Document kind: Language semantics (ISO/IEC-style specification)

## 1. Scope and status

This implementation-ready specification adds Boolean conjunction and disjunction, unary Boolean
negation, and the complete comparison-operator grammar. It makes comparison
chaining illegal while retaining ordinary precedence between arithmetic,
comparison, and Boolean operators.

`LANGUAGE.md` remains authoritative until this specification is accepted,
implemented, and incorporated there. This specification follows Specification
021's truthiness and equality semantics; it owns the operator syntax,
precedence, evaluation order, and chaining rule, including the mixed
`Int64`/`Float64` ordering rule in Specification 021 §7.3.

Type tests retain their existing condition-only grammar in this version. They
cannot be nested under `!` or combined directly with `and`/`or`; a future
condition-expression specification may add that composition without changing
the operator meanings defined here.

## 2. Dependencies

This specification depends on:

- [Specification 020](020-literal-cleanup-and-numeric-radices.md) for the
  numeric types and their arithmetic operands; and
- [Specification 021](021-truthiness-and-equality.md) for equality admissibility,
  exact operand types, and the NaN invariant.

## 3. Operators

The first version supports these operators:

| Operator | Kind | Result | Evaluation |
| --- | --- | --- | --- |
| `!` | unary Boolean negation | `Bool` | operand once |
| `and` | binary conjunction | `Bool` | short-circuit |
| `or` | binary disjunction | `Bool` | short-circuit |
| `==` | binary equality | `Bool` | both operands once |
| `!=` | binary inequality | `Bool` | both operands once |
| `<` | binary ordered comparison | `Bool` | both operands once |
| `<=` | binary ordered comparison | `Bool` | both operands once |
| `>` | binary ordered comparison | `Bool` | both operands once |
| `>=` | binary ordered comparison | `Bool` | both operands once |

`and`, `or`, and `!` operate on `Bool` operands only. They do not convert an
`Int64`, `String`, collection, or other truthy value to `Bool`. Truthiness in a
condition remains the separate rule specified by Specification 021:

~~~snacc
let count: Int64 = 1
if count then                 // valid: condition truthiness
    print(count)
end

let flag: Bool = true
let inverse: Bool = !flag     // valid
let bad: Bool = !count        // invalid: no implicit Boolean conversion
~~~

There are no `&&`, `||`, or `not` spellings in this version. The words `and`
and `or` are reserved keywords. Existing bindings with either name must be
renamed when this specification lands; there is no compatibility alias or
contextual interpretation as an identifier.

The distinction between conditions and Boolean algebra is intentional. A
control-flow condition may use Specification 021's general truthiness rule,
but `and`, `or`, and `!` are Boolean operators and therefore require actual
`Bool` operands. Convert or compare a value explicitly before combining it:

~~~snacc
if count then                         // valid: condition truthiness
    print(count)
end

if (count > 0) and ready then          // valid: both operands are Bool
    print(count)
end

if count and ready then                // invalid: count is not Bool
    print(count)
end
~~~

## 4. Evaluation semantics

Operands evaluate left to right.

`left and right` evaluates `left` exactly once. If it is `false`, the result is
`false` and `right` is not evaluated. Otherwise `right` is evaluated exactly
once and must be `Bool`; its value is the result.

`left or right` evaluates `left` exactly once. If it is `true`, the result is
`true` and `right` is not evaluated. Otherwise `right` is evaluated exactly
once and must be `Bool`; its value is the result.

Short-circuiting is observable:

~~~snacc
fun side_effect(): Bool do
    print(1)
    true
end

false and side_effect() // side_effect is not called
true or side_effect()   // side_effect is not called
~~~

`!operand` evaluates `operand` exactly once and returns the opposite Boolean
value. It is right-associative, so `!!flag` means `!(!flag)`.

`==`, `!=`, `<`, `<=`, `>`, and `>=` retain Specification 021's operand type
rules, conversions, NaN handling, and exact logical negation rule for `!=`.

## 5. Precedence and associativity

From tightest to loosest:

1. calls, field access, and method calls;
2. unary `!`;
3. `*` and `/`;
4. `+` and `-`;
5. one comparison operator;
6. `and`;
7. `or`.

Operators at the same precedence associate left to right, except unary `!`,
which associates right to left. Parentheses may override any precedence or
associativity rule.

~~~snacc
!ready or fallback              // (!ready) or fallback
x < limit and enabled           // (x < limit) and enabled
a + b * c                       // a + (b * c)
(a or b) and c                  // explicit override
~~~

Mixed precedence levels do not require parentheses. Parentheses are required
only when the desired grouping differs from the precedence above or when a
comparison is being composed with another comparison.

## 6. Comparison chaining is illegal

An expression contains at most one comparison operator at its current
parenthesis level. The grammar does not accept a repeated comparison operator:

~~~snacc
a < b > c       // syntax error
a == b == c     // syntax error
a < b <= c      // syntax error
~~~

To compare several relationships, write separate comparisons joined by `and`
or `or`:

~~~snacc
x < a and x > b             // valid
left == right or fallback   // valid
~~~

Parentheses create a new expression level, so explicit composition remains
possible:

~~~snacc
(a < b) == flag
(a < b) and (b < c)
~~~

The first expression compares the Boolean result of `a < b` with `flag`; the
second expresses a range test without a comparison chain.

This rule also removes the generic-call ambiguity described by Specification
014. `f<T>(x)` has the generic-call shape, while an unparenthesized comparison
with two comparison operators is no longer a valid competing parse.

## 7. Grammar after adoption

The formal grammar becomes:

~~~ebnf
expression           = logical-or ;
logical-or           = logical-and, { "or", logical-and } ;
logical-and          = comparison, { "and", comparison } ;
comparison           = additive, [ comparison-operator, additive ] ;
comparison-operator  = "==" | "!=" | "<" | "<=" | ">" | ">=" ;
additive             = multiplicative, { ( "+" | "-" ), multiplicative } ;
multiplicative       = unary, { ( "*" | "/" ), unary } ;
unary                = "!", unary | postfix ;
~~~

The existing `postfix`, atom, and parenthesized-expression productions remain
unchanged. The same text must be copied to `LANGUAGE.md` and `GRAMMAR.ebnf`
when implementation lands. `and` and `or` become reserved keywords at that
time.

## 8. Diagnostics

The checker or parser must report:

- a syntax error for a repeated comparison operator at one parenthesis level;
- a type error when `!`, `and`, or `or` receives a non-`Bool` operand;
- the operand types and operator when a comparison violates Specification 021's
  exact-type rules; and
- no diagnostic for an unevaluated short-circuit right operand, because it is
  intentionally not executed.

Diagnostics must identify the operator span and, for a chained comparison, the
second comparison operator that cannot follow the completed comparison.

## 9. Detailed implementation plan

### Phase 1: tokens and grammar

1. Reserve the keywords `and` and `or` and add the `!` token.
2. Update the parser grammar in `GRAMMAR.ebnf`, `LANGUAGE.md`, and the parser's
   expression module so comparison accepts at most one operator.
3. Preserve source spans for every operator and parenthesized expression.

### Phase 2: syntax tree and checker

1. Add explicit syntax nodes for unary negation, conjunction, and disjunction.
2. Reuse Specification 021's checked comparison plans for all six comparison
   operators.
3. Enforce `Bool` operands for `!`, `and`, and `or`; do not apply truthiness
   conversion.
4. Reject a second comparison operator before lowering and produce the chained
   comparison diagnostic.

### Phase 3: lowering

1. Lower `!` to a Boolean inversion after evaluating its operand once.
2. Lower `and` and `or` to explicit control-flow blocks so the right operand is
   not evaluated when short-circuited.
3. Lower comparisons from Specification 021's checked plans and preserve its
   NaN failure behavior.
4. Ensure every branch produces a `Bool` value and joins only after the selected
   operand has been evaluated.

### Phase 4: conformance and documentation

1. Add parser cases for every legal operator and every rejected comparison
   chain.
2. Add checker cases for non-Boolean logical operands and all existing equality
   and ordered-comparison type combinations.
3. Add runtime cases proving left-to-right evaluation and short-circuit side
   effects.
4. Add precedence cases covering `!`, arithmetic, comparisons, `and`, and `or`
   with and without parentheses.
5. Synchronize `LANGUAGE.md`, `GRAMMAR.ebnf`, diagnostics, and examples.

## 10. Completion criteria

Implementation is complete only when:

1. all nine listed operators parse with the precedence and associativity in
   section 5;
2. comparison chaining is rejected syntactically;
3. logical operators require `Bool` and short-circuit exactly as specified;
4. `!` evaluates once and returns the Boolean complement;
5. existing Specification 021 equality, comparison, and NaN semantics remain
   unchanged;
6. generic calls such as `f<T>(x)` no longer conflict with a legal comparison
   chain;
7. type tests retain their existing condition-only grammar; composition under
   `!`, `and`, or `or` remains outside this specification;
8. `LANGUAGE.md`, `GRAMMAR.ebnf`, parser, checker, lowering, diagnostics, and
   tests agree; and
9. formatting, checking, and the relevant test suites pass.
