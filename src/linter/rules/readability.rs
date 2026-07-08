//! Readability rules — code that is correct but clearer written another way.

mod comparison_negation;
mod outer_negation;
mod string_boundary;
mod true_false_symbol;

pub use comparison_negation::ComparisonNegation;
pub use outer_negation::OuterNegation;
pub use string_boundary::StringBoundary;
pub use true_false_symbol::TrueFalseSymbol;
