pub mod check;
pub(crate) mod context;
pub mod core;
pub(crate) mod ir;
pub(crate) mod printer;
pub(crate) mod render;
pub(crate) mod rules;
pub mod style;
pub(crate) mod trivia;

pub use check::{CheckError, CheckResult, check_paths, check_paths_with_style};
pub use core::{FormatError, RangeFormatted, format, format_node, format_range, format_with_style};
pub use style::FormatStyle;
