//! A lossless CST parser for DCF (Debian Control Format) — R's `DESCRIPTION`
//! format.
//!
//! This is a **second grammar** in this crate, independent of the R one. It
//! exists because `DESCRIPTION` is not just a bag of facts to scrape: it wants
//! spans (for diagnostics), record structure, and byte-for-byte round-tripping
//! (for formatting).
//!
//! - [`syntax`] — `SyntaxKind`, `DcfLanguage`, and the node/token aliases.
//! - [`parser`] — `parse`, `reconstruct`, and `ParseOutput`.
//! - [`ast`] — typed wrappers; **the** way to read a document from outside
//!   this module, so no consumer ever has to name the second `SyntaxKind`.
//!
//! Losslessness holds exactly as it does for the R grammar:
//! `reconstruct(text) == text`, byte for byte. Errors never abort the parse —
//! diagnostics ride a side channel and every byte stays in the tree.
//!
//! ```
//! use arity_parser::dcf;
//!
//! let output = dcf::parse("Package: mypkg\nDepends: R (>= 4.1.0)\n");
//! let value = output.document().field("Depends").unwrap().folded_value();
//! assert_eq!(value, "R (>= 4.1.0)");
//! ```
//!
//! Two `SyntaxKind` enums now exist in this crate. They are distinct types
//! sharing a name, disambiguated by module path — never glob-import both.

pub mod ast;
pub mod deps;
pub mod parser;
pub mod syntax;

pub use ast::{CommentLine, Document, Field, MalformedLine, Record, ValueLine};
pub use deps::{DependencyEntry, VersionConstraint, VersionOp, dependency_entries};
pub use parser::{ParseOutput, parse, reconstruct};
pub use syntax::{DcfLanguage, SyntaxKind, SyntaxNode, SyntaxToken};

pub use crate::parser::ParseDiagnostic;
