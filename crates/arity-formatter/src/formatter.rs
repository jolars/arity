pub(crate) mod context;
pub mod core;
pub(crate) mod ir;
pub(crate) mod printer;
pub(crate) mod render;
pub(crate) mod roxygen;
pub(crate) mod rules;
pub mod style;
pub(crate) mod trivia;

pub use core::{
    FormatError, RangeFormatted, format, format_node, format_range, format_with_options,
    format_with_style,
};
pub use style::{FormatStyle, LineEnding};
