# snacc

Snacc is a statically typed AOT language inspired by C, Luau, and Crystal.

## Build

The repository includes the versioned LLVM 22.1.8 developer distribution under
`vendor/` so the compiler does not require a machine-wide LLVM installation.

```text
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo run -p snacc-workbench
```

The workbench binds to an ephemeral loopback port and prints its local URL.
It executes native programs, so use trusted snippets only.

The main packages are:

- `snacc-compiler`: lexer, parser, checker, and LLVM object emission.
- `snacc-runtime`: stable runtime ABI for generated applications.
- `snacc-driver`: native executable construction.
- `snacc`: direct compiler CLI.
- `cargo-snacc`: Cargo-hosted application tooling.
- `snacc-workbench`: local browser workbench.
