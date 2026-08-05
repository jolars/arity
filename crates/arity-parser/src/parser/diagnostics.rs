use crate::parser::lexer::Token;

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
