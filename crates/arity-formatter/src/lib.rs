//! Deterministic, rule-based formatter for the R language.
//!
//! This crate is the formatting engine of [arity](https://arity.cc),
//! extracted so that other tools (e.g. a dprint plugin) can embed it. Output
//! is decided solely by the formatter's rules and its best-fit layout engine;
//! the input's existing line breaks never influence the result. The target
//! style is the tidyverse R style guide.
//!
//! The entry points are [`format`] and [`format_with_style`], configured via
//! [`FormatStyle`]. With the `serde` feature, `FormatStyle` is
//! (de)serializable; the `schema` feature additionally derives
//! `schemars::JsonSchema`.

pub mod formatter;

pub mod ast;
pub mod parser;
pub mod syntax;

pub use formatter::{
    FormatError, FormatStyle, LineEnding, RangeFormatted, format, format_node, format_range,
    format_with_options, format_with_style,
};
