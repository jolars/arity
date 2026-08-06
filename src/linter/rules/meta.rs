//! Meta rules: lints about arity's own `# arity-ignore` directives.
//!
//! Unlike every other category, these rules do not read the R code at all —
//! they read the directive list the driver parsed off the file's comments
//! (`RuleContext::suppressions`). A suppression that names a rule that does not
//! exist, names no rule at all, or no longer silences anything is a maintenance
//! bug in the same way dead code is, and it fails silently by nature: the whole
//! point of a suppression is that nothing is reported.
//!
//! One limitation applies to all of them. Their findings are spanned on comment
//! tokens, and a node-level `# arity-ignore` attaches to the next *non-trivia*
//! sibling — which skips comments. So a meta finding cannot be suppressed by a
//! `# arity-ignore` on the line above it; use the file-wide form
//! (`# arity-ignore-file <meta-rule>: …`) or `[lint] ignore`.

mod blanket_suppression;
mod misnamed_suppression;
mod unexplained_suppression;

pub use blanket_suppression::BlanketSuppression;
pub use misnamed_suppression::MisnamedSuppression;
pub use unexplained_suppression::UnexplainedSuppression;
