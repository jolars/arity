//! Entry points, refusals, and the driver.

use super::{fields, plan};
use crate::dcf;
use crate::formatter::style::{FormatStyle, apply_line_ending};

/// The continuation indent.
///
/// Four spaces is the file format's convention — `desc`, `usethis`, and
/// effectively every package on CRAN write it — and it is deliberately **not**
/// [`FormatStyle::indent_width`], which configures R-code nesting depth. Someone
/// who sets `indent-width = 2` for their R sources is not asking for two-space
/// `DESCRIPTION` continuations, and coupling them would also make the embedded-R
/// width budget depend on the same knob it feeds.
const INDENT: &str = "    ";

/// Why a `DESCRIPTION` was not formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptionFormatError {
    /// Not valid DCF. `read.dcf` would reject it too.
    ParseErrors { count: usize },
    /// Valid input the formatter deliberately leaves alone.
    Declined(DeclineReason),
}

/// Valid input the formatter refuses to restyle.
///
/// Each variant is a case where restyling could change what R reads, not merely
/// how it looks. Refusing is always safe; guessing never is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclineReason {
    /// Valid DCF, but not a `DESCRIPTION`: reordering across a record boundary
    /// would move a field from one record to another.
    MultipleRecords { count: usize },
    /// `read.dcf` takes the last occurrence and arity's reader takes the first,
    /// so sorting would make "last" arbitrary.
    DuplicateField { name: String },
    /// `Package : p` declares a field named `"Package "`. Re-emitting `Package:`
    /// would rename it.
    NameWhitespace { name: String },
    /// The file declares an encoding we would have to guess at to re-wrap.
    Encoding { declared: String },
    /// A line `read.dcf` would choke on.
    MalformedLine,
    /// A BOM binds to the first field name.
    ByteOrderMark,
    /// A tree shape this formatter does not model. Reaching this means the
    /// grammar grew a case; declining is how nothing gets silently dropped.
    UnsupportedStructure,
}

impl DescriptionFormatError {
    /// Whether the input was valid and simply left alone, as opposed to broken.
    ///
    /// Callers map the two differently: a decline is not a failure, so the CLI
    /// leaves the file untouched and still exits 0.
    pub fn is_decline(&self) -> bool {
        matches!(self, Self::Declined(_))
    }
}

impl std::fmt::Display for DescriptionFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseErrors { count } => write!(
                f,
                "input contains {count} DCF diagnostic(s); formatter only supports parseable input"
            ),
            Self::Declined(reason) => write!(f, "left unformatted: {reason}"),
        }
    }
}

impl std::fmt::Display for DeclineReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MultipleRecords { count } => {
                write!(f, "the file holds {count} records, not one")
            }
            Self::DuplicateField { name } => write!(f, "duplicate field {name:?}"),
            Self::NameWhitespace { name } => {
                write!(f, "whitespace before the colon of field {name:?}")
            }
            Self::Encoding { declared } => write!(f, "declared encoding {declared:?} is not UTF-8"),
            Self::MalformedLine => write!(f, "the file has a malformed line"),
            Self::ByteOrderMark => write!(f, "the file starts with a byte order mark"),
            Self::UnsupportedStructure => write!(f, "unrecognized document structure"),
        }
    }
}

impl std::error::Error for DescriptionFormatError {}

/// Format `DESCRIPTION` text with the default style.
pub fn format_description(input: &str) -> Result<String, DescriptionFormatError> {
    format_description_with_style(input, FormatStyle::default())
}

/// Format `DESCRIPTION` text.
///
/// Only [`FormatStyle::line_width`] and [`FormatStyle::line_ending`] apply;
/// `indent_width` is an R-code concern and the continuation indent is fixed at
/// four spaces.
pub fn format_description_with_style(
    input: &str,
    style: FormatStyle,
) -> Result<String, DescriptionFormatError> {
    if input.starts_with('\u{feff}') {
        return Err(DescriptionFormatError::Declined(
            DeclineReason::ByteOrderMark,
        ));
    }

    let parsed = dcf::parse(input);
    if !parsed.diagnostics.is_empty() {
        return Err(DescriptionFormatError::ParseErrors {
            count: parsed.diagnostics.len(),
        });
    }

    let plan = plan::build(&parsed.document()).map_err(DescriptionFormatError::Declined)?;

    let lines = render(&plan, style);
    if lines.is_empty() {
        return Ok(String::new());
    }

    let mut out = lines.join("\n");
    out.push('\n');
    Ok(apply_line_ending(&out, style.line_ending.resolve(input)))
}

fn render(plan: &plan::Plan, style: FormatStyle) -> Vec<String> {
    let mut lines = plan.orphan_comments.clone();
    if let Some(record) = &plan.record {
        for field in &record.fields {
            lines.extend(field.leading_comments.iter().cloned());
            lines.extend(fields::render(field, style, INDENT));
        }
        lines.extend(record.trailing_comments.iter().cloned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs that must never panic and must always round-trip through the
    /// refusal rules cleanly, mirroring the DCF parser's own adversarial table.
    const ADVERSARIAL: &[&str] = &[
        "",
        "\n",
        "Package:",
        "Package: p",
        "Package: p\n",
        ": value\n",
        "# only a comment\n",
        "#\n",
        "Package: p\n# a\n# b\n",
        "Collate:\n",
        "Collate:\n    'a.R'\n",
        "Imports:\n",
        "Imports: ,\n",
        "Imports: a (>= 1.0, < 2.0)\n",
        "Authors@R: person(\n",
        "Description: a\n b\n c\n",
        "Package: p\r\n",
        "\u{feff}Package: p\n",
        "Package: p\n\n\n",
        "Config/x: a\n  b\n",
    ];

    #[test]
    fn adversarial_inputs_never_panic_and_stay_idempotent() {
        for input in ADVERSARIAL {
            let Ok(formatted) = format_description(input) else {
                continue;
            };
            let again = format_description(&formatted)
                .unwrap_or_else(|err| panic!("reformatting {input:?} failed: {err}"));
            assert_eq!(again, formatted, "not idempotent for {input:?}");
            assert_eq!(
                dcf::reconstruct(&formatted),
                formatted,
                "not lossless for {input:?}"
            );
        }
    }

    #[test]
    fn a_bom_is_declined_before_parsing() {
        assert_eq!(
            format_description("\u{feff}Package: p\n"),
            Err(DescriptionFormatError::Declined(
                DeclineReason::ByteOrderMark
            ))
        );
    }

    #[test]
    fn crlf_input_round_trips_as_crlf() {
        assert_eq!(
            format_description("Package: p\r\nImports: b, a\r\n").expect("formats"),
            "Package: p\r\nImports:\r\n    a,\r\n    b\r\n"
        );
    }

    #[test]
    fn the_line_ending_style_overrides_the_source() {
        let style = FormatStyle {
            line_ending: crate::formatter::style::LineEnding::Crlf,
            ..FormatStyle::default()
        };
        assert_eq!(
            format_description_with_style("Package: p\n", style).expect("formats"),
            "Package: p\r\n"
        );
    }
}
