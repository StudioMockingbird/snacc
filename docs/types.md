# Types

Snacc is statically typed. Every function parameter, function return value, and
local binding has an explicit type annotation; omitted annotations are syntax
errors rather than requests for inference.

The built-in types are:

- `Int64`: signed 64-bit integer. Integer literals such as `5` have this type.
- `Dec64`: IEEE-754 double precision number. Literals with a decimal point such
  as `5.0` have this type.
- `Bool`: `true` or `false`.
- `Nil`: the single value `nil`.

Integer values may be promoted to `Dec64` where a `Dec64` value is required. No
other implicit conversions exist. Arithmetic and ordered comparisons operate
on `Int64` or `Dec64`; equality also works for matching `Bool` and `Nil` values.

Functions use Lua-style blocks and declare both parameter and result types:

```snacc
fun square(x: Int64): Int64 do
    x * x
end

let x: Int64 = square(5);
print(x);
```

Function bodies, `if` expressions, `while` expressions, and semicolon-
separated sequences return their final expression. A function's final
expression must be assignable to its declared result type.

Cargo-hosted applications expose selected Rust crate operations through typed
C-compatible bridges. The Snacc declaration names both the Snacc function and
the stable exported Rust symbol:

```snacc
extern rust "snacc_user_double" fun rust_double(value: Int64): Int64

print(rust_double(21))
```

Bridge parameters and results initially support only `Int64`, `Dec64`, `Bool`,
and `Nil`. Link symbols must begin with `snacc_user_`; Rust types, traits,
generics, references, and unwinding never cross this boundary directly.
