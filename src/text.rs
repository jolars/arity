//! Shared text utilities for the linter, LSP, and CLI.

pub mod line_index;

pub use line_index::{LineCol, LineIndex, PositionEncoding};
