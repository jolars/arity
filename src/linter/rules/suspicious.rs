//! Suspicious-pattern rules — code that's almost always a mistake but not
//! a syntax error.

mod all_equal;
mod assignment_in_condition;
mod bare_return;
mod browser;
mod confusable_identifier;
mod duplicated_function_definition;
mod for_loop_dup_index;
mod for_loop_index;
mod function_return_assignment;
mod implicit_assignment;
mod partial_argument_match;
mod pipe_return;
mod redundant_equals;
mod redundant_ifelse;
mod repeat_loop;
mod shadowed_builtin;
mod strings_as_factors;
mod terminal_close;
mod undesirable_function;
mod unused_function;

pub use all_equal::AllEqual;
pub use assignment_in_condition::AssignmentInCondition;
pub use bare_return::BareReturn;
pub use browser::Browser;
pub use confusable_identifier::ConfusableIdentifier;
pub use duplicated_function_definition::DuplicatedFunctionDefinition;
pub use for_loop_dup_index::ForLoopDupIndex;
pub use for_loop_index::ForLoopIndex;
pub use function_return_assignment::FunctionReturnAssignment;
pub use implicit_assignment::ImplicitAssignment;
pub use partial_argument_match::PartialArgumentMatch;
pub use pipe_return::PipeReturn;
pub use redundant_equals::RedundantEquals;
pub use redundant_ifelse::RedundantIfelse;
pub use repeat_loop::Repeat;
pub use shadowed_builtin::ShadowedBuiltin;
pub use strings_as_factors::StringsAsFactors;
pub use terminal_close::TerminalClose;
pub use undesirable_function::UndesirableFunction;
pub use unused_function::UnusedFunction;
