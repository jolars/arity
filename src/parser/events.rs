use crate::syntax::SyntaxKind;

#[derive(Debug, Clone)]
pub(crate) enum Event {
    Start(SyntaxKind),
    Tok(usize),
    /// A synthetic leaf: a `(kind, text)` pair built by the parser rather than
    /// referencing a whole lexed token. Used where a single lexed token must be
    /// split into several CST leaves — e.g. carving a multi-line block Rd macro's
    /// `\itemize{` text into a `ROXYGEN_RD_MACRO_NAME` and a delimiter. The
    /// emitted texts must tile the original tokens exactly (losslessness).
    Leaf(SyntaxKind, String),
    Finish,
}

#[derive(Debug, Clone)]
pub(crate) struct ExprParse {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) events: Vec<Event>,
}

pub(crate) fn push_range(events: &mut Vec<Event>, start: usize, end: usize) {
    for idx in start..end {
        events.push(Event::Tok(idx));
    }
}
