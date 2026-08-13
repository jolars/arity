//! Re-export of [`arity_parser::dcf`], the second grammar, so `crate::dcf`
//! paths resolve here the way `crate::parser` and `crate::syntax` already do.
//!
//! Two `SyntaxKind` enums exist in this crate's dependency graph. They are
//! distinct types sharing a name: never glob-import both.
pub use arity_parser::dcf::*;
