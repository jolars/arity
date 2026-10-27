# Parsing

`arity parse` displays a concrete syntax tree (CST) for debugging and parser
development. It parses R source and Rd help files. An `.Rd` extension selects
the Rd grammar, regardless of case:

```bash
arity parse man/foo.Rd
arity parse --verify --quiet man/foo.Rd
```

`--verify` checks that the tree reconstructs the input byte for byte, including
line endings. `--quiet` suppresses the tree. Syntax diagnostics go to stderr;
the command returns 1 for a syntax or verification failure and 2 for an input or
usage error. A syntax error still produces a recovered tree.

Stdin defaults to R. Use `--stdin-filename` to select Rd:

```bash
cat man/foo.Rd | arity parse - --stdin-filename foo.Rd --verify
```

The first Rd parser covers standard sections, nested markup, links and options,
lists, tables, equations, example controls, and conditional constructs. It
distinguishes prose, R-like, and verbatim arguments, retaining comments,
whitespace, escapes, and delimiters. R-like arguments are Rd syntax, so parsing
does not require complete R expressions inside them.

Arity preserves `\Sexpr`, custom macro definitions, and macro calls without
evaluating or expanding them. Unrecognized macro names remain in the tree
quietly. The parser checks syntax rather than package documentation policy: it
does not require particular sections or resolve package-defined macros.

Rd support currently provides parsing only. The formatter, linter, and language
server do not analyze Rd syntax. Input must be UTF-8, and `\encoding` does not
cause Arity to convert input bytes.

Library users can access the same parser through `arity_parser::rd` or
`arity::rd`, with typed views over the lossless tree. See the [parser
crate](https://docs.rs/arity-parser) for its API.
