//! Linter: semantic-only rules over the rowan CST.
//!
//! The linter is purely semantic: any check that the formatter's `--check`
//! mode can perform belongs to the formatter, not here. Rules consume a
//! [`crate::semantic::SemanticModel`] and emit [`Diagnostic`]s. Suppression
//! via `# arity-ignore` comments is honored inside
//! [`rules::run_rules`], after the rules have run — rules always emit
//! unconditionally and the driver's filter does the rest. The `meta/*-suppression`
//! rules then lint the directives themselves, reading the parsed directive list
//! off [`rules::RuleContext::suppressions`].

pub mod check;
pub mod diagnostic;
pub mod docs;
pub mod fix;
pub mod render;
pub mod rules;
pub mod suppression;

pub use check::{
    LintError, LintFileReport, LintResult, LintStatus, PreparedProject, SYNTAX_ERROR_RULE,
    analyze_prepared, check_document, check_document_in_project, check_document_with_provider,
    check_paths, check_paths_with_config, check_paths_with_index, prepare_document_in_project,
    seed_workspace_for, syntax_error_diagnostics,
};
pub use diagnostic::{Applicability, Diagnostic, Fix, Severity, ViolationData};
pub use fix::{FixOutcome, apply_fixes};
pub use render::{OutputMode, render_findings};
