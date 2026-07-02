//! Roxygen-block analysis built over the lossless CST.
//!
//! Currently this hosts the **CST → Rd-tree projector** ([`project_rd`]), a
//! test-only, faithful diagnostic used by the projector-parity gate
//! (`tests/roxygen_projector.rs`). It is *not* a roxygen2 reimplementation: it
//! projects only what arity's parser is responsible for (the section bodies),
//! never the roclet-generated scaffolding (`\name`, `\usage`, the `\arguments`
//! wrapper). A divergence from the pinned roxygen2 output means the CST (or the
//! encoding translation) is wrong --- the fix belongs in the parser, never here.

mod entities;
pub mod project_rd;
