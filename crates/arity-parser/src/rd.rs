//! Lossless, static parsing of UTF-8 R documentation (`.Rd`) files.
//!
//! The independent Rd CST retains source spelling, comments, and delimiters.
//! Macro arguments select prose, R-like, or verbatim scanning; R-like text is
//! not parsed as R expressions. Macros and conditionals are never evaluated.
//!
//! ```
//! use arity_parser::rd;
//! let source = "\\title{An example}\n";
//! let parsed = rd::parse(source);
//! assert!(parsed.diagnostics.is_empty());
//! assert_eq!(parsed.document().macros().next().unwrap().name(), "title");
//! assert_eq!(rd::reconstruct(source), source);
//! ```

pub mod ast;
mod macros;
pub mod parser;
pub mod syntax;

pub use ast::{Argument, Conditional, Document, Group, Macro, Option};
pub use parser::{ParseOutput, parse, reconstruct};
pub use syntax::{RdLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

pub use crate::parser::ParseDiagnostic;
