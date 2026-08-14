//! Cross-file / project-level analysis.
//!
//! Where [`crate::semantic`] is strictly single-file, this module models how
//! files relate: the `source()` dependency graph for scripts, the implicit
//! shared namespace of an R package, and the per-file export projection that
//! feeds cross-file name resolution.

pub mod classes;
pub mod deps;
pub mod description;
pub mod exports;
pub mod graph;
pub mod native;
pub mod roxygen;
pub mod scope;
pub mod sequence;
pub mod source;

pub use classes::{
    ClassDef, ClassLocation, ClassSystem, class_name_at_offset, file_class_defs, locate_class_def,
};
pub use deps::{PackageReferences, file_package_references};
pub use description::{
    Dependency, DependencyField, DescriptionCache, DescriptionCompat, DescriptionFacts,
};
pub use exports::{DefKind, file_def_sites, file_exports, file_free_reads, file_qualified_reads};
pub use graph::{
    ClassIndex, DefIndex, ExternalResolution, PackageCollation, PackageDeclarations, PackageInfo,
    PackageTopics, PackageUsage, Project, ProjectMember, ReadIndex, ReverseSources,
    RoxygenTopicIndex, Visibility, attached_names, discover_packages, expected_r_sources,
    external_resolution, package_facts_for, package_usage, package_usage_for, project_classes,
    project_defs, project_graph, project_reads, project_roxygen_topics, reverse_source_edges,
    roxygen_topics_for, visible_symbols, workspace_project,
};
pub use native::{dynlib_bound_names, registered_routines};
pub use roxygen::{
    ParamDoc, TopicMember, documented_binding_name, documented_function, file_roxygen_topics,
    has_title, inherits_params, joins_other_topic, param_doc, topic_key, topic_member,
};
pub use scope::{
    FileFacts, FileScope, ProjectScope, ReadBinding, ReadSite, is_package_root, package_root,
};
pub use sequence::{collect_top_level_events, collect_top_level_events_spanned};
pub use source::{
    LinkLiteral, SourceEdge, SourceEdgeKey, SourceLiteralEdge, SourceTarget, StringLiteral,
    TopLevelEvent, collect_link_literals, collect_source_edge_keys, collect_source_edges,
    collect_source_literal_edges, collect_string_literals, relative_path,
};
