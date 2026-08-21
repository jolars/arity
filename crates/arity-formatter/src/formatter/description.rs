//! Formatting for R package `DESCRIPTION` files.
//!
//! The grammar is DCF ([`crate::dcf`]); the *style* is R's `DESCRIPTION`
//! dialect — canonical field order, dependency lists one per line, and
//! `Authors@R`/`Roxygen` formatted as the R code they are. That last part is
//! what `desc` cannot do: it round-trips those fields through `deparse()`.
//!
//! Style reference, not oracle. `desc::desc_normalize()` is where the field
//! order and the four-space continuation indent come from, but it silently
//! **drops every comment** and emits a trailing space after a field with an
//! empty own line. We do neither.
//!
//! # Why there is no document IR here
//!
//! The crate's layout engine ([`crate::formatter::printer`]) decides breaks by
//! measuring a group, all-or-nothing. Nothing in this dialect wants that: comma
//! lists always break, `Collate` always breaks, prose wants first-fit rather
//! than best-fit, and embedded R delegates to the R formatter. So this module
//! emits lines directly. Introducing an `Ir` here would add a translation layer
//! that decides nothing.
//!
//! # What is preserved
//!
//! Formatting must not change what `read.dcf` sees. Fields it does not
//! recognize keep their line structure byte for byte, comments are never
//! dropped, and every input where restyling could change meaning is refused
//! outright (see [`DeclineReason`]). One exception to the crate's usual
//! no-trailing-whitespace property: a field frozen by an interior comment
//! replays the source's bytes, trailing whitespace included, because those bytes
//! are part of the value.

mod driver;
mod fields;
mod order;
mod plan;
mod rcode;
mod wrap;

pub use driver::{
    DeclineReason, DescriptionFormatError, format_description, format_description_with_style,
};
pub use order::field_names;
