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
