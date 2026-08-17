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
    Edit, ReparseKind, Reparsed, apply_edits, diff_edit, edits_produce, map_range_through_edit,
    map_range_through_edits, reparse, reparse_edits, reparse_edits_with_options,
    reparse_with_options,
};
pub use validate::{has_r_invalid_name, is_single_expression};

/// Lex `input` and return the number of tokens produced.
///
/// A benchmark-only entry point (`benches/lex.rs`): the lexer and its token
/// type stay crate-internal, so the bench observes only the count while the
/// full token vector is still built and dropped per call. Hidden from docs;
/// not covered by semver stability guarantees.
#[doc(hidden)]
pub fn lex_token_count(input: &str) -> usize {
    lexer::lex_with_md(input, false).len()
}
