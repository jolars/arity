//! Fields whose value is R code — `Authors@R` and `Roxygen`.
//!
//! This is what `desc` cannot do: it round-trips `Authors@R` through R's
//! `deparse()`, we run it through the real formatter. The value arrives folded,
//! which is exactly the dedent `read.dcf` performs, so the R this sees is the R
//! that R sees.

use crate::formatter::core::format_with_style;
use crate::formatter::style::{FormatStyle, LineEnding};

/// Format `source` as R and lay it out under `name`, or `None` when the value
/// must be left alone.
///
/// `None` is not an error: an `Authors@R` with a typo in it is a file the
/// formatter still has to handle, and a default-on formatter that fails a file
/// `R CMD build` accepts would be unusable. The caller falls back to preserving
/// the field's lines.
pub(super) fn render(
    name: &str,
    source: &str,
    style: FormatStyle,
    indent: &str,
) -> Option<Vec<String>> {
    let budget = FormatStyle {
        line_width: style.line_width.saturating_sub(indent.len()).max(1),
        // We split the result on '\n'; the document's real line ending is
        // applied once, at the very end of the outer pass.
        line_ending: LineEnding::Lf,
        ..style
    };
    let formatted = format_with_style(source, budget).ok()?;
    let lines: Vec<String> = formatted
        .trim_end_matches('\n')
        .split('\n')
        .map(str::to_string)
        .collect();

    if lines.is_empty() {
        return None;
    }
    // A blank line inside a value is a *record separator* to `read.dcf`, so
    // emitting one would split the DESCRIPTION in half. Only reachable for a
    // blank line inside a multi-line string literal.
    if lines.iter().any(|line| line.trim().is_empty()) {
        return None;
    }

    // Inline iff the whole thing is one line that still fits after the key. The
    // decision reads the single formatted result rather than re-formatting at an
    // inline budget: a width search is where oscillation would come from.
    if lines.len() == 1 && name.chars().count() + 2 + lines[0].chars().count() <= style.line_width {
        return Some(vec![format!("{name}: {}", lines[0])]);
    }

    let mut out = Vec::with_capacity(lines.len() + 1);
    out.push(format!("{name}:"));
    out.extend(lines.into_iter().map(|line| format!("{indent}{line}")));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDENT: &str = "    ";

    fn style() -> FormatStyle {
        FormatStyle::default()
    }

    #[test]
    fn a_short_call_stays_inline() {
        assert_eq!(
            render("Roxygen", "list(markdown = TRUE)", style(), INDENT),
            Some(vec!["Roxygen: list(markdown = TRUE)".to_string()])
        );
    }

    #[test]
    fn a_long_call_breaks_into_an_indented_block() {
        let source = r#"c(person("Aaaaaaaaaa", "Bbbbbbbbbb", email = "aaaaaaaaaa@example.com", role = c("aut", "cre")), person("Cccccccccc", "Dddddddddd", role = "ctb"))"#;
        let rendered = render("Authors@R", source, style(), INDENT).expect("formats");
        assert_eq!(rendered[0], "Authors@R:");
        assert!(rendered[1..].iter().all(|line| line.starts_with(INDENT)));
        assert!(rendered.iter().all(|line| line.chars().count() <= 80));
    }

    #[test]
    fn unparseable_r_declines_rather_than_erroring() {
        assert_eq!(render("Authors@R", "person(\"Jo\",", style(), INDENT), None);
    }

    #[test]
    fn a_value_that_would_emit_a_blank_line_declines() {
        // A blank line inside the value would read as a record separator.
        assert_eq!(render("Roxygen", "\"a\n\nb\"", style(), INDENT), None);
    }
}
