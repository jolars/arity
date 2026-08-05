//! CLI/LSP bridge over the [`arity_formatter`] engine.
//!
//! The formatting engine lives in the `arity-formatter` crate; this module
//! re-exports it and hosts the CLI-side concerns that do not belong in the
//! published engine: the batch path-walking check API ([`check`]) and the
//! persistent already-formatted cache ([`cache`]).

pub mod cache;
pub mod check;

pub use arity_formatter::formatter::*;

pub use cache::FormatCache;
pub use check::{
    ChangedFile, CheckError, CheckResult, check_paths, check_paths_with_style,
    check_paths_with_style_cached,
};
