# arity-parser

The lossless CST parser, typed AST wrappers, and incremental reparser for the R
language, extracted from [arity](https://arity.cc).

This crate is the parsing engine behind the `arity` CLI and language server. It
is published so that other tools can build on it, but its API surface is still
early and may change between releases; it is versioned independently of the
`arity` CLI.

```rust
use arity_parser::parser::{parse, reconstruct};

let text = "f <- function(x) x + 1\n";
let output = parse(text);
assert!(output.diagnostics.is_empty());
assert_eq!(reconstruct(text), text);
```

The parser preserves all source text (whitespace, comments, roxygen structure),
so reconstructing any parse tree yields the input byte-for-byte.

Package `NAMESPACE` files use the same lossless CST through a typed surface that
finds directives only where R permits them and never evaluates conditional
expressions:

```rust
use arity_parser::namespace::{self, DirectiveKind};

let text = "export(foo)\nimportFrom(stats, predict)\n";
let output = namespace::parse(text);
assert!(output.diagnostics.is_empty());
assert_eq!(
    output
        .document()
        .directives()
        .map(|directive| directive.kind())
        .collect::<Vec<_>>(),
    [DirectiveKind::Export, DirectiveKind::ImportFrom]
);
assert_eq!(namespace::reconstruct(text), text);
```
