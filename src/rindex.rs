//! R-introspection index: harvest installed-package symbols (exports, formals,
//! help) directly from disk — no R runtime — into a local cache that backs a
//! [`SymbolProvider`](crate::semantic::symbols::SymbolProvider).
//!
//! Installed R packages do not ship raw `man/*.Rd` or `.R` sources; their code
//! and help live in serialized *lazy-load databases* (`.rdb`/`.rdx`) plus
//! `.rds` index files. This module reads those formats natively:
//!
//! - [`lazyload`] — a compatibility adapter over `rd-rds`'s bounded index and
//!   record readers. Index-only harvesting never opens the companion `.rdb`.
//! - [`rd`] — Arity's presentation of canonical `rd-ast` nodes, populated by
//!   `rd-helpdb`, plus the existing public renderer for legacy objects.
//! - [`rds`] and [`deparse`] — retained for code records and default expressions
//!   until the shared inspection API can expose those expressions. Production
//!   metadata, help, and attach-vector reading do not use this decoder.
//!
//! built on top of those:
//!
//! - [`schema`] — the on-disk cache shape.

pub mod attach_probe;
pub mod build;
pub mod cache;
pub mod deparse;
pub mod discover;
pub mod harvest;
pub mod lazyload;
pub mod libpaths;
pub mod provider;
pub mod rd;
pub mod rds;
pub mod remote;
pub mod schema;
