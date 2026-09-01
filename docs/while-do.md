# While-do

`while` is a whitespace-agnostic Snacc expression. Newlines do not delimit its
parts.

```text
while <condition> do
    <body>
end
```

The condition is evaluated before every iteration and must have type `Bool`.
The body may contain any Snacc expression, including another `while`, an `if`,
or a semicolon-separated sequence. The body is evaluated only when the
condition is true.

`while` can be used as a statement by placing it in a sequence and ignoring
its value:

```snacc
while ready() do
    print(1)
end
print(2)
```

As an expression, its type is the type of the body. The value is the last body
value produced by the final iteration, matching Snacc's existing `if` branch,
sequence, and function-body value rules. If the condition is false initially,
the loop has no last body value; Snacc returns the type's zero value (`0`,
`0.0`, `false`, or `nil` for `Int64`, `Dec64`, `Bool`, or `Nil`) so every well-
typed loop expression has a
defined value.

The loop does not introduce a function scope. It can read surrounding lexical
values, but Snacc has no assignment or closure capture semantics; functions
continue to use only their explicit parameters and return values.

## Break

`break` exits the innermost enclosing `while` loop immediately:

```snacc
while ready() do
    if done() then
        break
    end
    print(1)
end
```

`break` is a statement and has no value. It may appear directly in a loop body
or inside nested expressions in that body, but it is invalid outside a `while`.
After a break, the rest of the current body is skipped and the loop condition
is not evaluated again. A nested loop handles its own `break`; the nearest loop
always wins.

For a `while` used as an expression, a break returns the last body value
completed before the break. If no body value was completed, the loop returns
the body type's zero value (`0`, `0.0`, `false`, or `nil` for `Int64`, `Dec64`,
`Bool`, or `Nil` respectively).

This syntax is specified for Snacc but is not yet accepted by the compiler;
parser, type-checker, and LLVM lowering support must be added before programs
using `break` can compile.
