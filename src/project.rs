//! Cross-file / project-level analysis.
//!
//! Where [`crate::semantic`] is strictly single-file, this module models how
//! files relate: the `source()` dependency graph for scripts, the implicit
//! shared namespace of an R package, and the per-file export projection that
//! feeds cross-file name resolution.

pub mod exports;
pub mod graph;
pub mod scope;
pub mod source;

pub use exports::{DefKind, file_def_sites, file_exports, file_free_reads};
pub use graph::{
    DefIndex, ExternalResolution, Project, ProjectMember, ReverseSources, Visibility,
    external_resolution, project_defs, project_graph, reverse_source_edges, visible_symbols,
    workspace_project,
};
pub use scope::{FileFacts, FileScope, ProjectScope, package_root};
pub use source::{
    SourceEdge, SourceEdgeKey, SourceTarget, collect_source_edge_keys, collect_source_edges,
};
