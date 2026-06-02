//! Correctness rules — likely bugs.

mod duplicate_formal;
mod undefined_symbol;
mod unused_binding;

pub use duplicate_formal::DuplicateFormal;
pub use undefined_symbol::UndefinedSymbol;
pub use unused_binding::UnusedBinding;
