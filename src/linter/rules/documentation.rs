//! Documentation rules: roxygen2 comment blocks that would generate wrong,
//! incomplete, or silently dropped documentation.

mod roxygen2_compat;
mod roxygen_examples;
mod roxygen_param;
mod roxygen_return;
mod roxygen_title;
mod roxygen_unknown_tag;

pub use roxygen_examples::RoxygenExamples;
pub use roxygen_param::RoxygenParam;
pub use roxygen_return::RoxygenReturn;
pub use roxygen_title::RoxygenTitle;
pub use roxygen_unknown_tag::RoxygenUnknownTag;
pub use roxygen2_compat::Roxygen2Compat;
