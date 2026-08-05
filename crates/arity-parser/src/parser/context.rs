use crate::parser::cursor;
use crate::parser::diagnostics;
use crate::parser::diagnostics::ParseDiagnostic;
use crate::parser::lexer::Token;

pub(crate) struct ParserCtx<'a> {
    tokens: &'a [Token],
}

impl<'a> ParserCtx<'a> {
    pub(crate) fn new(tokens: &'a [Token]) -> Self {
        Self { tokens }
    }

    pub(crate) fn token(&self, i: usize) -> Option<&'a Token> {
        self.tokens.get(i)
    }

    pub(crate) fn tokens(&self) -> &'a [Token] {
        self.tokens
    }

    pub(crate) fn skip_ws(&self, i: usize) -> usize {
        cursor::skip_ws(self.tokens, i)
    }

    pub(crate) fn skip_ws_and_newlines(&self, i: usize) -> usize {
        cursor::skip_ws_and_newlines(self.tokens, i)
    }

    pub(crate) fn skip_ws_newlines_comments(&self, i: usize) -> usize {
        cursor::skip_ws_newlines_comments(self.tokens, i)
    }
}

/// Tracks the end of the previous statement in a statement list (the root, or a
/// `{...}` block) so a missing separator between two statements can be flagged.
///
/// R requires a newline or `;` between statements; `12 14` is a syntax error
/// there, not two expressions. Accepting it silently leaves the formatter with a
/// line it has no rule for, so the parser owns the rejection (Tenet 3).
#[derive(Default)]
pub(crate) struct StatementTracker {
    /// Token index just past the previous statement, if any.
    prev_end: Option<usize>,
    /// Whether the previous statement was itself an error recovery. Suppresses a
    /// follow-on separator complaint, which would just be noise on top of the
    /// diagnostic that already fired.
    prev_recovered: bool,
}

impl StatementTracker {
    /// Record a statement spanning `[start, end)`, reporting a diagnostic when no
    /// separator was crossed since the previous one. `recovered` marks a parse
    /// that already reported its own diagnostic.
    pub(crate) fn record(
        &mut self,
        tokens: &[Token],
        start: usize,
        end: usize,
        recovered: bool,
        diagnostics_out: &mut Vec<ParseDiagnostic>,
    ) {
        if let Some(prev_end) = self.prev_end
            && !self.prev_recovered
            && !recovered
            && !cursor::has_statement_separator(tokens, prev_end, start)
            && let Some(token) = tokens.get(start)
        {
            diagnostics::push_token_diagnostic(
                diagnostics_out,
                diagnostics::MISSING_STATEMENT_SEPARATOR,
                token,
            );
        }
        self.prev_end = Some(end);
        self.prev_recovered = recovered;
    }

    /// Record an unparseable token that was kept for losslessness. It ends the
    /// current statement without becoming one, and suppresses the next
    /// separator check.
    pub(crate) fn record_recovery(&mut self, end: usize) {
        self.prev_end = Some(end);
        self.prev_recovered = true;
    }
}

pub(crate) fn push_token_diagnostic_ctx(
    diagnostics_out: &mut Vec<ParseDiagnostic>,
    message: &str,
    token: &Token,
) {
    diagnostics::push_token_diagnostic(diagnostics_out, message, token);
}
