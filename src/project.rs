//! Cross-file / project-level analysis.
//!
//! Where [`crate::semantic`] is strictly single-file, this module models how
//! files relate: the `source()` dependency graph for scripts, the implicit
//! shared namespace of an R package, and the per-file export projection that
//! feeds cross-file name resolution.

pub mod exports;
pub mod scope;
pub mod source;

pub use exports::{file_exports, file_free_reads};
pub use scope::{FileFacts, FileScope, ProjectScope, package_root};
pub use source::{SourceEdge, SourceTarget, collect_source_edges};
