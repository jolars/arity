//! Performance rules — correct code with a faster, idiomatic equivalent.

mod any_duplicated;
mod any_is_na;
mod crossprod;
mod fixed_regex;
mod lengths;

pub use any_duplicated::AnyDuplicated;
pub use any_is_na::AnyIsNa;
pub use crossprod::Crossprod;
pub use fixed_regex::FixedRegex;
pub use lengths::Lengths;
