//! Suspicious-pattern rules — code that's almost always a mistake but not
//! a syntax error.

mod assignment_in_condition;
mod browser;
mod duplicated_function_definition;
mod for_loop_dup_index;
mod for_loop_index;
mod implicit_assignment;
mod redundant_equals;
mod redundant_ifelse;
mod repeat_loop;
mod shadowed_builtin;
mod undesirable_function;
mod unused_function;

pub use assignment_in_condition::AssignmentInCondition;
pub use browser::Browser;
pub use duplicated_function_definition::DuplicatedFunctionDefinition;
pub use for_loop_dup_index::ForLoopDupIndex;
pub use for_loop_index::ForLoopIndex;
pub use implicit_assignment::ImplicitAssignment;
pub use redundant_equals::RedundantEquals;
pub use redundant_ifelse::RedundantIfelse;
pub use repeat_loop::Repeat;
pub use shadowed_builtin::ShadowedBuiltin;
pub use undesirable_function::UndesirableFunction;
pub use unused_function::UnusedFunction;
