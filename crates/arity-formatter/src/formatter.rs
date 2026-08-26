pub(crate) mod context;
pub mod core;
pub mod description;
pub mod directive;
pub(crate) mod ir;
pub(crate) mod printer;
pub(crate) mod render;
pub(crate) mod roxygen;
pub(crate) mod rules;
pub mod style;
pub(crate) mod trivia;

pub use core::{
    FormatAnalysis, FormatError, RangeFormatted, analyze_format_with_options, format, format_node,
    format_range, format_with_options, format_with_style,
};
pub use description::{
    DeclineReason, DescriptionFormatError, format_description, format_description_with_style,
};
pub use style::{FormatStyle, LineEnding};
