# Getting Started

## Installation

### Cargo

Install the Arity CLI from [crates.io](https://crates.io/crates/arity) with
Cargo:

```bash
cargo install arity
```

### From Source

Clone the repository and build a release binary:

```bash
git clone https://github.com/jolars/arity
cd arity
cargo build --release
```

The binary is written to `target/release/arity`.

### R Package (CRAN)

To use the formatter inside R, install the [`arity`
package](https://CRAN.R-project.org/package=arity):

```r
install.packages("arity")
```

See the [R package documentation](https://github.com/jolars/arity-r) for details
and [RStudio setup](guide/editors.md#rstudio) for use in the editor.

## First Run

With the CLI installed, run these commands in a terminal.

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
