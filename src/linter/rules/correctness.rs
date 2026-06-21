//! Correctness rules — likely bugs.

mod duplicate_formal;
mod duplicated_arguments;
mod equals_na;
mod undefined_symbol;
mod unreachable_code;
mod unused_binding;
mod vector_logic;

pub use duplicate_formal::DuplicateFormal;
pub use duplicated_arguments::DuplicatedArguments;
pub use equals_na::EqualsNa;
pub use undefined_symbol::UndefinedSymbol;
pub use unreachable_code::UnreachableCode;
pub use unused_binding::UnusedBinding;
pub use vector_logic::VectorLogic;
