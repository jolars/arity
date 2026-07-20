//! Performance rules — correct code with a faster, idiomatic equivalent.

mod any_duplicated;
mod any_is_na;
mod class_equals;
mod crossprod;
mod fixed_regex;
mod lengths;
mod nzchar;
mod seq;
mod sort;

pub use any_duplicated::AnyDuplicated;
pub use any_is_na::AnyIsNa;
pub use class_equals::ClassEquals;
pub use crossprod::Crossprod;
pub use fixed_regex::FixedRegex;
pub use lengths::Lengths;
pub use nzchar::Nzchar;
pub use seq::Seq;
pub use sort::Sort;
