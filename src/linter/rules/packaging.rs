//! Packaging rules: the package's declared metadata, and whether it matches
//! its code.
//!
//! The one category holding rules over both grammars. That is deliberate: a
//! dependency the code uses and `DESCRIPTION` does not declare, and one
//! `DESCRIPTION` declares and no code reaches, are the same defect seen from
//! two sides, and a reader looking for either should find both in one place.

mod description_duplicate_field;
mod description_missing_field;
mod description_version_constraint;

pub use description_duplicate_field::DescriptionDuplicateField;
pub use description_missing_field::DescriptionMissingField;
pub use description_version_constraint::DescriptionVersionConstraint;
