//! R-introspection index: harvest installed-package symbols (exports, formals,
//! help) directly from disk — no R runtime — into a local cache that backs a
//! [`SymbolProvider`](crate::semantic::symbols::SymbolProvider).
//!
//! Installed R packages do not ship raw `man/*.Rd` or `.R` sources; their code
//! and help live in serialized *lazy-load databases* (`.rdb`/`.rdx`) plus
//! `.rds` index files. This module reads those formats natively:
//!
//! - [`rds`] — a minimal reader for R's RDS serialization format.
//! - [`lazyload`] — decode named objects out of an `.rdb`/`.rdx` pair.
//!
//! built on top of those:
//!
//! - [`schema`] — the on-disk cache shape.

pub mod build;
pub mod cache;
pub mod deparse;
pub mod discover;
pub mod harvest;
pub mod lazyload;
pub mod libpaths;
pub mod provider;
pub mod rds;
pub mod schema;
