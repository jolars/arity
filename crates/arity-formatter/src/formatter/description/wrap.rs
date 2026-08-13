//! Greedy fill for free-form fields — the analogue of R's
//! `strwrap(x, exdent = 4)`.
//!
//! Deliberately not the crate's best-fit layout engine. That engine decides
//! breaks by measuring a *group*, all-or-nothing; prose wants first-fit, where
//! each word joins the current line if it fits. The two produce different
//! output, and the one `DESCRIPTION` readers expect is first-fit.

/// Lay `value`'s words out after `name: `, wrapping at `width` with continuation
/// lines indented by `indent`.
///
/// Width is counted in `char`s. A word wider than the budget gets a line to
/// itself and overflows rather than being split — breaking a URL to make a
/// column fit would be a worse outcome than a long line.
pub(super) fn fill(name: &str, value: &str, width: usize, indent: &str) -> Vec<String> {
    let mut words = value.split_whitespace();
    let Some(first) = words.next() else {
        return vec![format!("{name}:")];
    };

    let mut lines = Vec::new();
    let head = format!("{name}: ");
    let mut current = if head.chars().count() + first.chars().count() <= width {
        format!("{head}{first}")
    } else {
        // R's own behavior: an oversized first word leaves the key alone on its
        // line rather than starting a line that is already over budget.
        lines.push(format!("{name}:"));
        format!("{indent}{first}")
    };

    for word in words {
        if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = format!("{indent}{word}");
        }
    }
    lines.push(current);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDENT: &str = "    ";

    #[test]
    fn a_short_value_stays_on_the_key_line() {
        assert_eq!(fill("Title", "A Thing", 80, INDENT), vec!["Title: A Thing"]);
    }

    #[test]
    fn an_empty_value_leaves_a_bare_key() {
        assert_eq!(fill("Encoding", "", 80, INDENT), vec!["Encoding:"]);
        assert_eq!(fill("Encoding", "   ", 80, INDENT), vec!["Encoding:"]);
    }

    #[test]
    fn continuation_lines_are_indented() {
        assert_eq!(
            fill("Description", "alpha beta gamma delta", 25, INDENT),
            vec!["Description: alpha beta", "    gamma delta"]
        );
    }

    #[test]
    fn an_oversized_first_word_leaves_the_key_alone() {
        let url = "https://example.com/a-very-long-path-indeed";
        assert_eq!(
            fill("URL", url, 20, INDENT),
            vec!["URL:".to_string(), format!("{INDENT}{url}")]
        );
    }

    #[test]
    fn width_counts_characters_not_bytes() {
        // Four two-byte characters must count as four columns, not eight.
        assert_eq!(
            fill("Title", "ünïcöde word", 18, INDENT),
            vec!["Title: ünïcöde", "    word"]
        );
    }

    #[test]
    fn the_budget_is_inclusive() {
        // "Title: abcd" is exactly 11 characters.
        assert_eq!(fill("Title", "abcd", 11, INDENT), vec!["Title: abcd"]);
        assert_eq!(
            fill("Title", "abcd", 10, INDENT),
            vec!["Title:", "    abcd"]
        );
    }
}
