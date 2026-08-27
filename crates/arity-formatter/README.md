# arity-formatter

The deterministic, rule-based R formatter behind [arity](https://arity.cc),
extracted so that other tools (for example a dprint plugin) can embed it.

Output is decided solely by the formatter's rules and its best-fit layout
engine---the input's existing line breaks never influence the result. The target
style is the tidyverse R style guide.

```rust
use arity_formatter::format;

let formatted = format("x<-(1+2)*3^4\n").unwrap();
assert_eq!(formatted, "x <- (1 + 2) * 3^4\n");
```

Configure via `FormatStyle` and `format_with_style`. The optional `serde`
feature makes `FormatStyle` (de)serializable (kebab-case keys, matching
`arity.toml`); the `schema` feature additionally derives `schemars::JsonSchema`.

Use `format_verified` (or its `_with_style` / `_with_options` variants) when an
integration should additionally check normalized R syntax, ordinary comment
preservation, and formatting idempotence. Ordinary `format` remains the
single-pass path.

This crate's API is still early and may change between releases; it is versioned
independently of the `arity` CLI.
