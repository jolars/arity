//! Correctness rules — likely bugs.

mod duplicate_formal;
mod duplicated_arguments;
mod equals_na;
mod undefined_symbol;
mod unused_binding;

pub use duplicate_formal::DuplicateFormal;
pub use duplicated_arguments::DuplicatedArguments;
pub use equals_na::EqualsNa;
pub use undefined_symbol::UndefinedSymbol;
pub use unused_binding::UnusedBinding;
