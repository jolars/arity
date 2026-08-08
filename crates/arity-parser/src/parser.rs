pub(crate) mod bracket_balancer;
pub(crate) mod context;
pub mod core;
pub(crate) mod cursor;
pub(crate) mod diagnostics;
pub(crate) mod events;
/// Expression-level helpers. Low-level surface consumed by the `arity` crate;
/// not covered by semver stability guarantees.
pub mod expr;
pub(crate) mod lexer;
pub(crate) mod recovery;
pub mod reparse;
/// Roxygen sub-lexing and classification helpers. Low-level surface consumed
/// by the `arity` crate; not covered by semver stability guarantees.
pub mod roxygen;
pub(crate) mod structural;
pub(crate) mod tree_builder;
pub(crate) mod validate;

pub use core::{
    ParseDiagnostic, ParseOptions, ParseOutput, parse, parse_with_options, reconstruct,
};
pub use reparse::{
    Edit, ReparseKind, Reparsed, apply_edits, diff_edit, map_range_through_edit,
    map_range_through_edits, reparse, reparse_edits, reparse_edits_with_options,
    reparse_with_options,
};
pub use validate::{has_r_invalid_name, is_single_expression};
