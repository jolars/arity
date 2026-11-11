//! Shared OOP declarations. These are source facts, not a simulated R registry.
//!
//! Keys identify declaration sites, so two registrations with the same generic
//! and signature remain two methods. Symbolic references retain their lookup
//! domain; a missing or ambiguous target never becomes an arbitrary declaration.
//! Like class facts, these projections contain no ranges or function bodies.
//! Occurrence ranges are recovered from the current CST by [`oop_sites`].

mod extract;
mod model;

pub use extract::oop_sites;
pub(super) use extract::{declaration_roots, project};
pub use model::*;
