//! Performance rules — correct code with a faster, idiomatic equivalent.

mod any_duplicated;
mod any_is_na;
mod call_rewrites;
mod class_equals;
mod coalesce;
mod crossprod;
mod fixed_regex;
mod lengths;
mod nzchar;
mod seq;
mod sort;

pub use any_duplicated::AnyDuplicated;
pub use any_is_na::AnyIsNa;
pub use call_rewrites::{LengthLevels, List2df, MatrixApply, RepLen, SystemFile, WhichGrepl};
pub use class_equals::ClassEquals;
pub use coalesce::Coalesce;
pub use crossprod::Crossprod;
pub use fixed_regex::FixedRegex;
pub use lengths::Lengths;
pub use nzchar::Nzchar;
pub use seq::Seq;
pub use sort::Sort;
