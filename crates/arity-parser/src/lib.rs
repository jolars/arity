//! Lossless CST parser, typed AST wrappers, and incremental reparser for the
//! R language.
//!
//! This crate is the parsing engine of [arity](https://arity.cc), extracted so
//! that other tools can build on it. The public surface mirrors the module
//! layout of the `arity` crate itself:
//!
//! - [`syntax`] — `SyntaxKind`, the rowan language definition, and
//!   position-independent node pointers.
//! - [`parser`] — the lossless parser (`parse`/`reconstruct`), diagnostics,
//!   and the incremental reparse entry points (`reparse`, `apply_edits`).
//! - [`ast`] — zero-cost typed wrappers over the CST, in rust-analyzer's
//!   mould.
//!
//! The parser preserves all source text: `reconstruct(&parse(text).syntax())`
//! is always `text`.

pub mod ast;
pub mod parser;
pub mod syntax;
