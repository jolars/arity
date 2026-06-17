//! Suspicious-pattern rules — code that's almost always a mistake but not
//! a syntax error.

mod assignment_in_condition;
mod redundant_equals;
mod redundant_ifelse;
mod shadowed_builtin;

pub use assignment_in_condition::AssignmentInCondition;
pub use redundant_equals::RedundantEquals;
pub use redundant_ifelse::RedundantIfelse;
pub use shadowed_builtin::ShadowedBuiltin;
