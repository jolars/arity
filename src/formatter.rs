//! CLI/LSP bridge over the [`arity_formatter`] engine.
//!
//! The formatting engine lives in the `arity-formatter` crate; this module
//! re-exports it and hosts the CLI-side concerns that do not belong in the
//! published engine: the batch path-walking check API ([`check`]) and the
//! persistent already-formatted cache ([`cache`]).

pub mod cache;
pub mod check;
pub mod source;

pub use arity_formatter::formatter::*;

pub use cache::{CacheKey, FormatCache};
pub use check::{
    ChangedFile, CheckError, CheckResult, FailedFile, OutdatedDirective, check_paths,
    check_paths_with_style, check_paths_with_style_cached,
};
pub use source::{FormatSourceError, Formatted, cache_key, format_file, merge};
