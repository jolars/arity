//! Cross-file / project-level analysis.
//!
//! Where [`crate::semantic`] is strictly single-file, this module models how
//! files relate: the `source()` dependency graph for scripts, the implicit
//! shared namespace of an R package, and the per-file export projection that
//! feeds cross-file name resolution.

pub mod exports;
pub mod source;

pub use exports::file_exports;
pub use source::{SourceEdge, SourceTarget, collect_source_edges};
