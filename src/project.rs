//! Cross-file / project-level analysis.
//!
//! Where [`crate::semantic`] is strictly single-file, this module models how
//! files relate: the `source()` dependency graph for scripts, the implicit
//! shared namespace of an R package, and the per-file export projection that
//! feeds cross-file name resolution.

pub mod exports;
pub mod graph;
pub mod scope;
pub mod sequence;
pub mod source;

pub use exports::{DefKind, file_def_sites, file_exports, file_free_reads, file_qualified_reads};
pub use graph::{
    DefIndex, ExternalResolution, PackageCollation, PackageInfo, Project, ProjectMember, ReadIndex,
    ReverseSources, Visibility, discover_packages, expected_r_sources, external_resolution,
    project_defs, project_graph, project_reads, reverse_source_edges, visible_symbols,
    workspace_project,
};
pub use scope::{FileFacts, FileScope, ProjectScope, ReadBinding, ReadSite, package_root};
pub use sequence::{collect_top_level_events, collect_top_level_events_spanned};
pub use source::{
    LinkLiteral, SourceEdge, SourceEdgeKey, SourceLiteralEdge, SourceTarget, TopLevelEvent,
    collect_link_literals, collect_source_edge_keys, collect_source_edges,
    collect_source_literal_edges, relative_path,
};
