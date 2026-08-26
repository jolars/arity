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
//!
//! A package's `DESCRIPTION` is a second grammar with its own entry point,
//! [`format_description`]; see [`formatter::description`].

pub mod formatter;

pub mod ast;
pub mod dcf;
pub mod parser;
pub mod syntax;

/// The `rowan` version this crate's CST types are built on.
///
/// [`format_range`] takes a `rowan::TextRange`, so an embedder has to be able
/// to name it. Re-exporting the dependency keeps that caller version-matched
/// with us instead of making them guess a compatible `rowan` in their own
/// `Cargo.toml`.
pub use rowan;

pub use formatter::{
    DeclineReason, DescriptionFormatError, FormatAnalysis, FormatError, FormatStyle, LineEnding,
    RangeFormatted, analyze_format_with_options, format, format_description,
    format_description_with_style, format_node, format_range, format_with_options,
    format_with_style,
};
