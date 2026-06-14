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

pub(crate) fn push_token_diagnostic_ctx(
    diagnostics_out: &mut Vec<ParseDiagnostic>,
    message: &str,
    token: &Token,
) {
    diagnostics::push_token_diagnostic(diagnostics_out, message, token);
}
