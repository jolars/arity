//! Shared text utilities for the linter, LSP, and CLI.

pub mod buffer;
pub mod line_index;

pub use buffer::TextBuffer;
pub use line_index::{LineCol, LineIndex, PositionEncoding};
