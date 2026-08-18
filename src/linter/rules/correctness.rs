//! Correctness rules — likely bugs.

mod download_file;
mod duplicate_formal;
mod duplicated_arguments;
mod empty_assignment;
mod equals_na;
mod equals_nan;
mod equals_null;
mod if_always_true;
mod internal_function;
mod is_numeric;
mod missing_argument;
mod r_compat;
mod undefined_symbol;
mod unreachable_code;
mod unused_binding;
mod vector_logic;

pub use download_file::DownloadFile;
pub use duplicate_formal::DuplicateFormal;
pub use duplicated_arguments::DuplicatedArguments;
pub use empty_assignment::EmptyAssignment;
pub use equals_na::EqualsNa;
pub use equals_nan::EqualsNan;
pub use equals_null::EqualsNull;
pub use if_always_true::IfAlwaysTrue;
pub use internal_function::InternalFunction;
pub use is_numeric::IsNumeric;
pub use missing_argument::MissingArgument;
pub use r_compat::RCompat;
pub use undefined_symbol::UndefinedSymbol;
pub use unreachable_code::UnreachableCode;
pub use unused_binding::UnusedBinding;
pub use vector_logic::VectorLogic;
