//! Packaging rules: the package's declared metadata, and whether it matches
//! its code.
//!
//! The one category holding rules over both grammars. That is deliberate: a
//! dependency the code uses and `DESCRIPTION` does not declare, and one
//! `DESCRIPTION` declares and no code reaches, are the same defect seen from
//! two sides, and a reader looking for either should find both in one place.

mod description_duplicate_field;
mod description_malformed_name;
mod description_missing_field;
mod description_package_in_multiple_fields;
mod description_version_constraint;
mod undeclared_dependency;
mod unused_dependency;

pub use description_duplicate_field::DescriptionDuplicateField;
pub use description_malformed_name::DescriptionMalformedName;
pub use description_missing_field::DescriptionMissingField;
pub use description_package_in_multiple_fields::DescriptionPackageInMultipleFields;
pub use description_version_constraint::DescriptionVersionConstraint;
pub use undeclared_dependency::UndeclaredDependency;
pub use unused_dependency::UnusedDependency;
