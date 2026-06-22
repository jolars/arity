# Getting Started

## Installation

### Cargo

The simplest way to install Arity is from
[crates.io](https://crates.io/crates/arity) with Cargo:

```bash
cargo install arity
```

### From source

Clone the repository and build a release binary:

```bash
git clone https://github.com/jolars/arity
cd arity
cargo build --release
```

The binary is written to `target/release/arity`.

## First run

Format a file in place:

```bash
arity format file.R
```

Check formatting without writing changes:

```bash
arity format --check file.R
```

Lint a file (or pipe from stdin):

```bash
arity lint file.R
```

Run the language server over stdio (for editor integration):

```bash
arity lsp
```

See the [CLI Reference](reference/cli.md) for the full set of commands and
options.
