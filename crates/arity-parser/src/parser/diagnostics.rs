use crate::parser::lexer::{TokKind, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub message: String,
    pub start: usize,
    pub end: usize,
}

/// Reported when two expressions sit side by side in a statement list with no
/// newline or `;` between them. R calls this "unexpected <constant>"; the
/// wording here names the missing separator instead, matching the parser's
/// other "expected ..." messages.
pub(crate) const MISSING_STATEMENT_SEPARATOR: &str = "expected a newline or ';' between statements";

pub(crate) fn push_diagnostic(
    diagnostics: &mut Vec<ParseDiagnostic>,
    message: &str,
    start: usize,
    end: usize,
) {
    diagnostics.push(ParseDiagnostic {
        message: message.to_string(),
        start,
        end,
    });
}

pub(crate) fn push_token_diagnostic(
    diagnostics: &mut Vec<ParseDiagnostic>,
    message: &str,
    token: &Token,
) {
    push_diagnostic(diagnostics, message, token.start, token.end);
}

/// Explain rejected pasted characters even when structural recovery swallowed
/// their tokens. Quoted names, strings, comments, and user operators have their
/// own token kinds, so their contents never enter this check.
pub(crate) fn refine_unicode_diagnostics(tokens: &[Token], diagnostics: &mut Vec<ParseDiagnostic>) {
    let specific: Vec<_> = tokens
        .iter()
        .filter(|token| token.kind == TokKind::Unknown)
        .filter_map(|token| {
            let ch = token.text.chars().next()?;
            let (description, advice) = match ch {
                '\u{200b}'..='\u{200d}' | '\u{180e}' | '\u{feff}' => {
                    ("zero-width character", "remove this character")
                }
                '\u{ad}' => ("invisible soft hyphen", "remove this character"),
                '\u{34f}' => (
                    "invisible combining grapheme joiner",
                    "remove this character",
                ),
                '\u{61c}'
                | '\u{200e}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}' => {
                    ("invisible direction control", "remove this character")
                }
                '\u{2060}' => ("invisible word joiner", "remove this character"),
                '\u{2061}'..='\u{2064}' => ("invisible operator", "use an explicit R operator"),
                '\u{a0}' | '\u{2007}' | '\u{202f}' => ("Unicode space", "use an ASCII space"),
                '\u{85}' => ("Unicode next-line character", "use a newline"),
                '\u{2028}' => ("Unicode line separator", "use a newline"),
                '\u{2029}' => ("Unicode paragraph separator", "use a newline"),
                '\u{2212}' => ("Unicode minus sign", "use ASCII '-'"),
                _ => return None,
            };
            Some(ParseDiagnostic {
                message: format!(
                    "unexpected {description} U+{:04X} in R code; {advice}",
                    ch as u32
                ),
                start: token.start,
                end: token.end,
            })
        })
        .collect();
    if specific.is_empty() {
        return;
    }
    // Lexer order makes binary search possible even when recovery emitted
    // structural diagnostics out of source order. Replace generic messages at
    // these spans instead of reporting the same offending character twice.
    diagnostics.retain(|d| {
        specific
            .binary_search_by_key(&(d.start, d.end), |s| (s.start, s.end))
            .is_err()
    });
    diagnostics.extend(specific);
    diagnostics.sort_by_key(|d| (d.start, d.end));
}
