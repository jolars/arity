//! Linter: semantic-only rules over the rowan CST.
//!
//! The linter is purely semantic: any check that the formatter's `--check`
//! mode can perform belongs to the formatter, not here. Rules consume a
//! [`crate::semantic::SemanticModel`] and emit [`Diagnostic`]s. Suppression
//! via `# ravel-ignore` comments is honored at the check level — rules
//! always emit unconditionally and the check filter does the rest.

pub mod check;
pub mod diagnostic;
pub mod fix;
pub mod render;
pub mod rules;
pub mod suppression;

pub use check::{
    LintError, LintFileReport, LintResult, LintStatus, PreparedProject, analyze_prepared,
    check_document, check_document_in_project, check_document_with_provider, check_paths,
    check_paths_with_config, check_paths_with_index, prepare_document_in_project,
    seed_workspace_for,
};
pub use diagnostic::{Applicability, Diagnostic, Fix, Severity, ViolationData};
pub use fix::{FixOutcome, apply_fixes};
pub use render::{OutputMode, render_findings};
