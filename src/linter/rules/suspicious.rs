//! Suspicious-pattern rules — code that's almost always a mistake but not
//! a syntax error.

mod assignment_in_condition;
mod shadowed_builtin;

pub use assignment_in_condition::AssignmentInCondition;
pub use shadowed_builtin::ShadowedBuiltin;
