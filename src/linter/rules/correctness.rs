//! Correctness rules — likely bugs.

mod duplicate_formal;
mod duplicated_arguments;
mod empty_assignment;
mod equals_na;
mod if_always_true;
mod is_numeric;
mod undefined_symbol;
mod unreachable_code;
mod unused_binding;
mod vector_logic;

pub use duplicate_formal::DuplicateFormal;
pub use duplicated_arguments::DuplicatedArguments;
pub use empty_assignment::EmptyAssignment;
pub use equals_na::EqualsNa;
pub use if_always_true::IfAlwaysTrue;
pub use is_numeric::IsNumeric;
pub use undefined_symbol::UndefinedSymbol;
pub use unreachable_code::UnreachableCode;
pub use unused_binding::UnusedBinding;
pub use vector_logic::VectorLogic;
