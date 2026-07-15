//! The CommonMark inline pass for resolved `@md` emphasis and strong emphasis.
//!
//! Under `@md` the lexer carves `*`/`_` runs as **neutral** `RoxygenMdDelim`
//! leaves — it makes no open/close decision. This pass walks a block's events and,
//! over each maximal single-line run of inline-content tokens, resolves the
//! delimiter runs into `ROXYGEN_MD_EMPH` / `ROXYGEN_MD_STRONG` **nodes** via the
//! CommonMark delimiter-stack algorithm (full flanking — ASCII character classes
//! first —, the rule of three, and `process_emphasis`). Unmatched runs stay
//! literal `ROXYGEN_MD_DELIM` leaves (projected as plain text).
//!
//! Losslessness holds by construction: a matched run is split into `Event::Leaf`
//! delimiter pieces whose texts tile the original run, an unmatched run re-emits
//! its bytes, and every non-delimiter event passes through unchanged.
//!
//! **Scope (slice 1.5).** Emphasis/strong only, resolved at **paragraph
//! granularity**: a run spans every consecutive token of a paragraph (or tag-line)
//! body — including the inter-line trivia (newline, the next `#'` marker, leading
//! whitespace) that a continuation folds in — so a span may cross a *soft* line
//! break (`*foo`\n`bar*` → one `\emph` over `foo bar`). A run is bounded only by a
//! structural event (`Start`/`Finish`/`Leaf`): a paragraph/section/tag boundary, or
//! an inline node (`ROXYGEN_RD_MACRO`) which binds tighter than emphasis. The
//! inter-line trivia present as **whitespace** for flanking (a soft break is a
//! single space; the `#'` marker is treated as whitespace too) and pass through
//! verbatim — landing *inside* the resolved node when the span crosses a line.
//! Code spans, links, images, and single-line raw HTML stay opaque local-span
//! leaves, resolved by the lexer *before* this pass — matching CommonMark
//! precedence (they bind tighter than emphasis), so the pass treats each as one
//! opaque inline. **Multi-line raw HTML and code spans** are the exception the
//! lexer cannot see: [`resolve_multiline_spans`] carves them here, per run,
//! before the delimiter stack — the resolved `ROXYGEN_MD_HTML`/`ROXYGEN_MD_CODE`
//! node then joins the arena as one opaque inline like any leaf.

use crate::parser::events::Event;
use crate::parser::lexer::{TokKind, Token};
use crate::syntax::SyntaxKind;

/// Whether `kind` is a raw markdown-inline-markup token the pass resolves: an
/// emphasis delimiter run or an inline-link bracket. A run carrying neither is
/// re-emitted verbatim.
fn is_inline_markup(kind: &TokKind) -> bool {
    matches!(kind, TokKind::RoxygenMdDelim | TokKind::RoxygenMdBracket)
}

/// Whether token `tok` could open a **multi-line** span the line-scoped lexer
/// missed: a `<` (raw HTML) or a `` ` `` (code span) in plain prose text. A `<`
/// inside any carved leaf was already consumed or rejected by a tighter-binding
/// recognizer; a prose backtick is an opener run left unterminated on its line
/// (a terminated one was carved as a `RoxygenMdCode` leaf) — and its presence is
/// also what can re-split a later line's carved span (cmark scans the whole
/// paragraph leftmost-first), so with no prose backtick the per-line carve is
/// already the whole-paragraph answer.
fn is_multiline_span_candidate(tok: &Token) -> bool {
    tok.kind == TokKind::RoxygenText && tok.text.bytes().any(|b| matches!(b, b'<' | b'`'))
}

/// Resolve markdown emphasis/strong, inline links, and multi-line raw HTML in
/// `events` (in place). A no-op unless the block carries at least one raw
/// delimiter run, link bracket, or (under `md`) a `<` in plain prose text.
///
/// `md` is the block's resolved markdown mode (threaded from the caller — the
/// group phase's `block_md`). The multi-line HTML resolution is additionally
/// suppressed inside a raw-Rd tag's scope (`@rawRd` bodies are verbatim Rd,
/// never markdown — mirroring the lexer's per-tag keying, whose body tokens
/// carry no md leaves; the `<`-in-prose candidate has no mode-carrying leaf, so
/// the pass tracks the tag scope itself: a raw tag's same-line value and its
/// following body paragraphs share the tag's `ROXYGEN_SECTION`).
pub(super) fn resolve_emphasis(tokens: &[Token], events: &mut Vec<Event>, md: bool) {
    let has_markup = events
        .iter()
        .any(|e| matches!(e, Event::Tok(i) if is_inline_markup(&tokens[*i].kind)));
    let has_span_candidate = md
        && events
            .iter()
            .any(|e| matches!(e, Event::Tok(i) if is_multiline_span_candidate(&tokens[*i])));
    if !has_markup && !has_span_candidate {
        return;
    }

    let mut out = Vec::with_capacity(events.len());
    let mut run: Vec<usize> = Vec::new();
    let mut raw_scope = false;
    for ev in std::mem::take(events) {
        match ev {
            // Every paragraph-body token joins the run — content *and* the
            // inter-line trivia (newline / `#'` marker / whitespace) a continuation
            // folds in — so a span resolves across soft line breaks. A structural
            // event (a paragraph/section/tag boundary, or an inline `ROXYGEN_RD_MACRO`
            // which binds tighter than emphasis) bounds the run.
            Event::Tok(i) => {
                if tokens[i].kind == TokKind::RoxygenTagName {
                    raw_scope = super::lex::tag_body_skips_markdown(&tokens[i].text);
                }
                run.push(i);
            }
            other => {
                // A new section leaves any no-markdown tag's scope (the verbatim
                // body paragraphs are siblings inside the tag's own section).
                if matches!(other, Event::Start(SyntaxKind::ROXYGEN_SECTION)) {
                    raw_scope = false;
                }
                flush_run(tokens, &mut run, &mut out, md && !raw_scope);
                out.push(other);
            }
        }
    }
    flush_run(tokens, &mut run, &mut out, md && !raw_scope);
    *events = out;
}

/// Resolve one top-level inline run and append its events to `out`, then clear
/// `run`. Multi-line raw-HTML and code spans are carved first (under `md`); a
/// run with no markup and no multi-line span re-emits its tokens verbatim
/// (byte-identical), so only affected runs are rebuilt. The run's edges are the
/// start/end of the inline content (whitespace, for flanking).
fn flush_run(tokens: &[Token], run: &mut Vec<usize>, out: &mut Vec<Event>, md: bool) {
    if run.is_empty() {
        return;
    }
    let has_markup = run.iter().any(|&i| is_inline_markup(&tokens[i].kind));
    let span_items = if md {
        resolve_multiline_spans(tokens, run)
    } else {
        None
    };
    match span_items {
        None if !has_markup => {
            out.extend(run.drain(..).map(Event::Tok));
            return;
        }
        None => {
            let items: Vec<RunItem> = run.iter().map(|&i| RunItem::Tok(i)).collect();
            resolve_run(tokens, &items, None, None, out);
        }
        Some(items) => resolve_run(tokens, &items, None, None, out),
    }
    run.clear();
}

/// One element of an inline run, generalizing a token index so a resolved
/// multi-line span (raw HTML or a code span — and the synthetic pieces of a
/// token it splits) can join the emphasis arena as an opaque inline — an
/// enclosing emphasis span wraps it exactly as cmark does.
pub(super) enum RunItem {
    /// An original lexed token, by index.
    Tok(usize),
    /// A synthetic piece of a split token: literal text, inert to delimiter and
    /// bracket matching (emitted as a `ROXYGEN_TEXT` leaf). A split remainder
    /// never carries an *active* delimiter — the lexer carves those as their own
    /// tokens out of plain prose — though a piece cut from an opaque leaf may
    /// hide markdown cmark would re-scan (a faithful under-handling, backlog).
    Text(String),
    /// A resolved multi-line span (a `ROXYGEN_MD_HTML` or `ROXYGEN_MD_CODE`
    /// node): its pre-built node events, plus the span's cmark-visible text
    /// (for the bracket interior check and flanking edges). Opaque to emphasis,
    /// like any inline leaf.
    Span { events: Vec<Event>, text: String },
}

/// The token behind a run item, when it is one.
fn item_token<'a>(tokens: &'a [Token], item: &RunItem) -> Option<&'a Token> {
    match item {
        RunItem::Tok(i) => Some(&tokens[*i]),
        _ => None,
    }
}

/// The raw text a run item contributes (a resolved multi-line span contributes
/// its cmark-visible text).
fn item_text<'a>(tokens: &'a [Token], item: &'a RunItem) -> &'a str {
    match item {
        RunItem::Tok(i) => &tokens[*i].text,
        RunItem::Text(s) => s,
        RunItem::Span { text, .. } => text,
    }
}

/// The flanking-relevant edge char of a run item (see [`edge_char`]). A resolved
/// multi-line span's edges are its own delimiter bytes — `<`/`>` for raw HTML,
/// backticks for a code span — which are the first/last chars of its text.
fn item_edge_char(tokens: &[Token], item: &RunItem, leading: bool) -> Option<char> {
    match item {
        RunItem::Tok(i) => edge_char(&tokens[*i], leading),
        RunItem::Text(s) | RunItem::Span { text: s, .. } => {
            if leading {
                s.chars().next()
            } else {
                s.chars().next_back()
            }
        }
    }
}

/// Resolve the multi-line raw-HTML and code spans of a run, or `None` when it
/// has none.
///
/// The scan mirrors what roxygen2 hands cmark for the paragraph — the run's
/// **logical text**: content tokens verbatim, a soft break as `\n`, the `#'`
/// markers and marker→content indentation stripped (roxygen2 strips `#' `, and
/// cmark strips a paragraph continuation line's remaining leading whitespace),
/// and an inline Rd macro as its escape-placeholder shape (a leading
/// alphanumeric and a trailing hyphen — `escape_rd_for_md` swaps the macro for
/// an alphanumeric placeholder suffixed `-<i>-` *before* cmark, so a macro
/// never carries a span terminator, and its trailing `-` dash-blocks an
/// abutting `-->` exactly as the real placeholder does; the raw macro text is
/// restored in the rendered output by the projector's node-text walk).
///
/// The joined bytes are scanned left-to-right for `<` (raw HTML) and `` ` ``
/// (code span) candidates with the engine's own inline grammars
/// ([`super::lex::scan_md_html_inline`], [`super::lex::scan_inline_code`]) —
/// one pass, leftmost successful match wins, matching cmark's equal-precedence
/// scan of code spans and raw HTML. A `<` is a candidate only in plain prose
/// text (a `<` inside a carved leaf was consumed by a tighter-binding
/// recognizer). A backtick is a candidate in plain prose *or* inside a carved
/// `RoxygenMdCode` leaf: an unterminated prose opener on an earlier line must
/// re-split a later line's line-scoped carve exactly as cmark's whole-paragraph
/// scan does (``a `open`` ⏎ ``b` and `closed` `` → spans `open b` and
/// `closed`), and the leftover backticks of a split leaf are themselves
/// re-scanned. A code match that coincides exactly with one carved leaf is
/// *skipped* (the leaf already models it — the rebuild keeps the original
/// token), so a paragraph whose spans are all single-line rebuilds
/// byte-identically. An unmatched opener run is literal and the scan continues
/// past it.
///
/// A match becomes a `ROXYGEN_MD_HTML`/`ROXYGEN_MD_CODE` **node**: the covered
/// tokens (inter-line trivia included) move inside it and a partially covered
/// token is split into synthetic `ROXYGEN_TEXT` pieces that tile it exactly
/// (losslessness holds by construction). An unmatched opener stays untouched
/// literal prose, so interior markdown after it still resolves, as cmark's
/// re-scan would.
fn resolve_multiline_spans(tokens: &[Token], run: &[usize]) -> Option<Vec<RunItem>> {
    if !run.iter().any(|&i| is_multiline_span_candidate(&tokens[i])) {
        return None;
    }

    // The logical text and each run position's byte range within it.
    let mut logical = String::new();
    let mut ranges = Vec::with_capacity(run.len());
    for &i in run {
        let s = logical.len();
        match tokens[i].kind {
            TokKind::Newline => logical.push('\n'),
            TokKind::RoxygenMarker | TokKind::Whitespace => {}
            TokKind::RoxygenRdMacro => logical.push_str("x-"),
            _ => logical.push_str(&tokens[i].text),
        }
        ranges.push(s..logical.len());
    }

    // The kind of the token whose logical range covers byte `at`, if any.
    let covering_kind = |at: usize| {
        run.iter()
            .zip(&ranges)
            .find(|(_, r)| r.contains(&at))
            .map(|(&i, _)| &tokens[i].kind)
    };

    // Scan left-to-right, consuming each matched span (cmark's single pass).
    let bytes = logical.as_bytes();
    let mut matches: Vec<(usize, usize, SyntaxKind)> = Vec::new();
    let mut pos = 0;
    while let Some(off) = logical[pos..].find(['<', '`']) {
        let at = pos + off;
        pos = if bytes[at] == b'<' {
            let in_prose = covering_kind(at) == Some(&TokKind::RoxygenText);
            match in_prose
                .then(|| super::lex::scan_md_html_inline(bytes, at))
                .flatten()
            {
                Some(end) => {
                    matches.push((at, end, SyntaxKind::ROXYGEN_MD_HTML));
                    end
                }
                None => at + 1,
            }
        } else {
            let eligible = matches!(
                covering_kind(at),
                Some(TokKind::RoxygenText | TokKind::RoxygenMdCode)
            );
            match eligible
                .then(|| super::lex::scan_inline_code(bytes, at))
                .flatten()
            {
                Some(end) => {
                    let is_carved_leaf = run.iter().zip(&ranges).any(|(&i, r)| {
                        tokens[i].kind == TokKind::RoxygenMdCode && r.start == at && r.end == end
                    });
                    if !is_carved_leaf {
                        matches.push((at, end, SyntaxKind::ROXYGEN_MD_CODE));
                    }
                    end
                }
                None => at + super::lex::run_len(bytes, at, b'`'),
            }
        };
    }
    if matches.is_empty() {
        return None;
    }

    // Rebuild the run around the matches. A macro placeholder never carries a
    // terminator and a match starts at a verbatim `<`, so every span boundary
    // falls inside a verbatim-contributing token — in-token offsets are valid.
    let mut items: Vec<RunItem> = Vec::new();
    // The open node's events and its span's logical range, once entered.
    let mut open: Option<(Vec<Event>, usize, usize)> = None;
    let mut mi = 0;
    for (p, &i) in run.iter().enumerate() {
        let (rs, re) = (ranges[p].start, ranges[p].end);
        let piece = |a: usize, b: usize| tokens[i].text[a - rs..b - rs].to_string();
        // First unconsumed logical offset of this token.
        let mut a = rs;
        loop {
            if let Some((ev, _, end)) = open.as_mut() {
                let end = *end;
                if rs == re {
                    // A zero-width trivia token (marker / stripped whitespace):
                    // inside while strictly before the span's end; at or past it,
                    // close the node and re-route the token outside.
                    if rs < end {
                        ev.push(Event::Tok(i));
                        break;
                    }
                } else if re <= end {
                    // Fully consumed by the span (whole token or its tail).
                    if a == rs {
                        ev.push(Event::Tok(i));
                    } else {
                        ev.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, piece(a, re)));
                    }
                    a = re;
                    if re < end {
                        break;
                    }
                } else {
                    // Straddles the span's end: the covered head goes inside.
                    if a < end {
                        ev.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, piece(a, end)));
                        a = end;
                    }
                }
                // Reached the span's end: close the node, then re-route any
                // remainder of this token through the outside branch.
                let (mut ev, s, e) = open.take().expect("checked above");
                ev.push(Event::Finish);
                items.push(RunItem::Span {
                    events: ev,
                    text: logical[s..e].to_string(),
                });
                if a >= re && rs != re {
                    break;
                }
                continue;
            }
            // Outside any span: does the next match start within this token?
            match matches.get(mi) {
                Some(&(s, e, kind)) if re > s => {
                    // Any pre-piece is partial: the match's opener byte sits
                    // within this token's range (`s < re` by the guard, and
                    // `s >= a` since matches are ordered and consumed; `a == s`
                    // — no pre-piece — when the match starts at a token start).
                    if a < s {
                        items.push(RunItem::Text(piece(a, s)));
                    }
                    open = Some((vec![Event::Start(kind)], s, e));
                    mi += 1;
                    a = s;
                }
                _ => {
                    if a == rs {
                        items.push(RunItem::Tok(i));
                    } else if a < re {
                        items.push(RunItem::Text(piece(a, re)));
                    }
                    break;
                }
            }
        }
    }
    debug_assert!(open.is_none(), "a span never outruns its closer's token");
    Some(items)
}

/// Resolve an inline run (a slice of [`RunItem`]s) into events appended to
/// `out`, given the flanking-relevant characters immediately `before`/`after`
/// the run (`None` = a whitespace boundary). Builds the arena (collapsing
/// inline links into opaque `ROXYGEN_MD_LINK` nodes, their text resolved by a
/// recursive call), resolves emphasis over the resulting top-level node list,
/// and emits.
fn resolve_run(
    tokens: &[Token],
    run: &[RunItem],
    before: Option<char>,
    after: Option<char>,
    out: &mut Vec<Event>,
) {
    let mut arena = Arena::build(tokens, run, before, after);
    arena.process_emphasis();
    arena.emit(out);
}

/// A node in the inline arena: a doubly linked list at the top level, with
/// emphasis nodes owning a child sublist.
struct Node {
    data: NodeData,
    prev: Option<usize>,
    next: Option<usize>,
    first_child: Option<usize>,
    last_child: Option<usize>,
}

enum NodeData {
    /// An opaque passthrough inline (text / code span / link / …): its original
    /// token index, re-emitted verbatim.
    Token(usize),
    /// Residual literal delimiter characters not consumed into emphasis. Emitted
    /// as a `ROXYGEN_MD_DELIM` leaf; dropped when empty.
    Delim(String),
    /// A resolved emphasis (`strong = false`) or strong (`strong = true`) span.
    /// `open`/`close` are the consumed delimiter strings, emitted as the node's
    /// opener/closer `ROXYGEN_MD_DELIM` leaves around its children.
    Emph {
        strong: bool,
        open: String,
        close: String,
    },
    /// A resolved inline link `[text](url)`: `open` is the opener bracket (`[`),
    /// `close` the closer carrying the destination (`](url)`), and `body` the
    /// already-resolved events of the link text (emphasis/code spans inside it
    /// resolved by a recursive [`resolve_run`]). Emitted as a `ROXYGEN_MD_LINK`
    /// **node** with the brackets as `ROXYGEN_MD_DELIM` opener/closer leaves; the
    /// node is opaque to the enclosing emphasis stack (so an outer span wraps the
    /// whole link, exactly as a plain inline does).
    Link {
        open: String,
        close: String,
        body: Vec<Event>,
    },
    /// A synthetic literal-text piece of a split token ([`RunItem::Text`]),
    /// emitted as a `ROXYGEN_TEXT` leaf. Inert to emphasis (never a delimiter).
    Text(String),
    /// A resolved multi-line span ([`RunItem::Span`]): its pre-built
    /// `ROXYGEN_MD_HTML`/`ROXYGEN_MD_CODE` node events, re-emitted verbatim.
    /// Opaque to emphasis.
    Span(Vec<Event>),
}

/// A delimiter-stack entry (its own doubly linked list, threaded by `prev`/`next`
/// over the delimiter Vec). `node` is the arena index of the `Delim` text node it
/// shrinks as delimiters are consumed.
struct Delim {
    node: usize,
    ch: u8,
    length: usize,
    orig: usize,
    can_open: bool,
    can_close: bool,
    prev: Option<usize>,
    next: Option<usize>,
    /// Cleared when the entry is removed from the stack (skipped thereafter).
    active: bool,
}

struct Arena {
    nodes: Vec<Node>,
    delims: Vec<Delim>,
    head: Option<usize>,
    tail: Option<usize>,
    /// Top of the delimiter stack (the last delimiter), like cmark's `last_delim`.
    last_delim: Option<usize>,
}

impl Arena {
    /// Build the top-level node list and the delimiter stack from an inline run.
    /// `before`/`after` are the flanking-relevant boundary characters at the run's
    /// edges (`None` = whitespace). An inline-link bracket pair (`[` … `](url)`) is
    /// **collapsed** into one opaque [`NodeData::Link`] whose body is the recursively
    /// resolved link text — so the brackets never reach the emphasis stack and an
    /// enclosing span wraps the whole link.
    fn build(
        tokens: &[Token],
        run: &[RunItem],
        before: Option<char>,
        after: Option<char>,
    ) -> Arena {
        let mut arena = Arena {
            nodes: Vec::new(),
            delims: Vec::new(),
            head: None,
            tail: None,
            last_delim: None,
        };
        // The flanking neighbor char at run position `p` on the given side: an
        // interior position reads its neighbor item's edge char; a run edge uses
        // the passed boundary (the link-text `[`/`]` for a recursive call, else ws).
        let neighbor = |p: usize, leading: bool| -> Option<char> {
            if leading {
                run.get(p + 1)
                    .map_or(after, |it| item_edge_char(tokens, it, true))
            } else {
                match p.checked_sub(1) {
                    Some(q) => item_edge_char(tokens, &run[q], false),
                    None => before,
                }
            }
        };
        // Match the link brackets first (CommonMark `look_for_link_or_image`:
        // backward matching with opener deactivation, so nested links resolve
        // inner-first and the enclosing brackets stay literal), then walk the run.
        let roles = match_brackets(tokens, run);
        let mut p = 0;
        while p < run.len() {
            match &roles[p] {
                // A matched link opener collapses with its closer into one Link
                // node; the inner items resolve recursively, bounded by the bracket
                // chars (`[` before, `]` after) for flanking. A matched link's
                // interior provably contains no further matched link (an inner link
                // would have deactivated this opener), so the recursion only resolves
                // emphasis and literal brackets. The closer text distinguishes the
                // forms: an inline `](url)` leaf, or — for a *reference* link
                // `[text][ref]` — a lone `]` plus the consumed `[ref]` label folded
                // in as `][ref]`, or a bare `]` *shortcut*.
                BracketRole::MatchedOpener {
                    closer,
                    after,
                    close_text,
                } => {
                    let open = item_text(tokens, &run[p]).to_string();
                    let close = close_text.clone();
                    let (closer, after_p) = (*closer, *after);
                    let mut body = Vec::new();
                    resolve_run(
                        tokens,
                        &run[p + 1..closer],
                        open.chars().next_back(),
                        Some(']'),
                        &mut body,
                    );
                    arena.push_node(NodeData::Link { open, close, body });
                    p = after_p;
                    continue;
                }
                // An unmatched/inactive bracket re-emits as literal text (a `Delim`
                // node, projected as plain text) — a deactivated outer opener, a `]`
                // whose opener never appeared, or a closer that formed no link.
                BracketRole::LiteralBracket => {
                    arena.push_node(NodeData::Delim(item_text(tokens, &run[p]).to_string()));
                    p += 1;
                    continue;
                }
                // A closer or reference label already folded into its opener's link.
                BracketRole::Consumed => {
                    p += 1;
                    continue;
                }
                BracketRole::Other => {}
            }
            match &run[p] {
                RunItem::Tok(idx) if tokens[*idx].kind == TokKind::RoxygenMdDelim => {
                    let tok = &tokens[*idx];
                    let ch = tok.text.as_bytes()[0];
                    let len = tok.text.len(); // a same-char ASCII run: bytes == chars
                    let (can_open, can_close) = flanking(ch, neighbor(p, false), neighbor(p, true));
                    let node = arena.push_node(NodeData::Delim(tok.text.clone()));
                    arena.push_delim(node, ch, len, can_open, can_close);
                }
                RunItem::Tok(idx) => {
                    arena.push_node(NodeData::Token(*idx));
                }
                RunItem::Text(s) => {
                    arena.push_node(NodeData::Text(s.clone()));
                }
                RunItem::Span { events, .. } => {
                    arena.push_node(NodeData::Span(events.clone()));
                }
            }
            p += 1;
        }
        arena
    }

    fn push_node(&mut self, data: NodeData) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            data,
            prev: self.tail,
            next: None,
            first_child: None,
            last_child: None,
        });
        if let Some(t) = self.tail {
            self.nodes[t].next = Some(id);
        } else {
            self.head = Some(id);
        }
        self.tail = Some(id);
        id
    }

    fn push_delim(&mut self, node: usize, ch: u8, length: usize, can_open: bool, can_close: bool) {
        let id = self.delims.len();
        self.delims.push(Delim {
            node,
            ch,
            length,
            orig: length,
            can_open,
            can_close,
            prev: self.last_delim,
            next: None,
            active: true,
        });
        if let Some(t) = self.last_delim {
            self.delims[t].next = Some(id);
        }
        self.last_delim = Some(id);
    }

    /// The CommonMark `process_emphasis` (ported from cmark). Walks the delimiter
    /// stack, matching each closer to the nearest eligible opener (rule of three),
    /// consuming 2 delimiters for strong else 1, wrapping the enclosed nodes.
    fn process_emphasis(&mut self) {
        // `openers_bottom[char][len % 3]` — the lower search bound per delimiter
        // char and closer length class. `*` = index 0, `_` = index 1.
        let mut openers_bottom = [[None; 3]; 2];

        // Find the first closer (move to the bottom of the active stack).
        let mut closer = self.last_delim;
        while let Some(c) = closer {
            match self.delims[c].prev {
                Some(p) => closer = Some(p),
                None => break,
            }
        }

        while let Some(c) = closer {
            if !self.delims[c].active {
                closer = self.delims[c].next;
                continue;
            }
            if !self.delims[c].can_close {
                closer = self.delims[c].next;
                continue;
            }

            let cc = self.delims[c].ch;
            let ci = if cc == b'*' { 0 } else { 1 };
            let bound = openers_bottom[ci][self.delims[c].length % 3];

            // Look back for the first matching opener.
            let mut opener = self.delims[c].prev;
            let mut opener_found = false;
            while let Some(o) = opener {
                if Some(o) == bound {
                    break;
                }
                if self.delims[o].active && self.delims[o].can_open && self.delims[o].ch == cc {
                    let odd_match = (self.delims[c].can_open || self.delims[o].can_close)
                        && !self.delims[c].orig.is_multiple_of(3)
                        && (self.delims[o].orig + self.delims[c].orig).is_multiple_of(3);
                    if !odd_match {
                        opener_found = true;
                        break;
                    }
                }
                opener = self.delims[o].prev;
            }

            if opener_found {
                let o = opener.unwrap();
                let use_delims = if self.delims[c].length >= 2 && self.delims[o].length >= 2 {
                    2
                } else {
                    1
                };
                self.delims[o].length -= use_delims;
                self.delims[c].length -= use_delims;

                let opener_inl = self.delims[o].node;
                let closer_inl = self.delims[c].node;
                let open_str = self.shorten_from_end(opener_inl, use_delims);
                let close_str = self.shorten_from_start(closer_inl, use_delims);

                let emph = self.push_emph(use_delims == 2, open_str, close_str);
                self.wrap_between(opener_inl, closer_inl, emph);

                self.remove_delims_between(o, c);

                if self.delims[o].length == 0 {
                    self.unlink(opener_inl);
                    self.remove_delim(o);
                }
                if self.delims[c].length == 0 {
                    let next = self.delims[c].next;
                    self.unlink(closer_inl);
                    self.remove_delim(c);
                    closer = next;
                }
            } else {
                openers_bottom[ci][self.delims[c].length % 3] = self.delims[c].prev;
                let next = self.delims[c].next;
                if !self.delims[c].can_open {
                    self.remove_delim(c);
                }
                closer = next;
            }
        }
    }

    /// Allocate an `Emph` node (not yet linked into any list).
    fn push_emph(&mut self, strong: bool, open: String, close: String) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            data: NodeData::Emph {
                strong,
                open,
                close,
            },
            prev: None,
            next: None,
            first_child: None,
            last_child: None,
        });
        id
    }

    /// Remove `use_delims` characters from the *end* of a `Delim` node's literal,
    /// returning the removed string (the opener delimiters).
    fn shorten_from_end(&mut self, node: usize, use_delims: usize) -> String {
        let NodeData::Delim(s) = &mut self.nodes[node].data else {
            unreachable!("delimiter stack node is not a Delim")
        };
        let cut = s.len() - use_delims;
        s.split_off(cut)
    }

    /// Remove `use_delims` characters from the *start* of a `Delim` node's literal,
    /// returning the removed string (the closer delimiters).
    fn shorten_from_start(&mut self, node: usize, use_delims: usize) -> String {
        let NodeData::Delim(s) = &mut self.nodes[node].data else {
            unreachable!("delimiter stack node is not a Delim")
        };
        let removed = s[..use_delims].to_string();
        s.drain(..use_delims);
        removed
    }

    /// Move every top-level node strictly between `opener_inl` and `closer_inl`
    /// into `emph`'s child list, then insert `emph` right after `opener_inl`.
    fn wrap_between(&mut self, opener_inl: usize, closer_inl: usize, emph: usize) {
        let mut tmp = self.nodes[opener_inl].next;
        while let Some(t) = tmp {
            if t == closer_inl {
                break;
            }
            let next = self.nodes[t].next;
            self.unlink(t);
            self.append_child(emph, t);
            tmp = next;
        }
        self.insert_after(opener_inl, emph);
    }

    /// Detach `node` from whatever list it is in (top level only here).
    fn unlink(&mut self, node: usize) {
        let prev = self.nodes[node].prev;
        let next = self.nodes[node].next;
        match prev {
            Some(p) => self.nodes[p].next = next,
            None => self.head = next,
        }
        match next {
            Some(n) => self.nodes[n].prev = prev,
            None => self.tail = prev,
        }
        self.nodes[node].prev = None;
        self.nodes[node].next = None;
    }

    fn append_child(&mut self, parent: usize, child: usize) {
        self.nodes[child].prev = self.nodes[parent].last_child;
        self.nodes[child].next = None;
        match self.nodes[parent].last_child {
            Some(l) => self.nodes[l].next = Some(child),
            None => self.nodes[parent].first_child = Some(child),
        }
        self.nodes[parent].last_child = Some(child);
    }

    fn insert_after(&mut self, anchor: usize, node: usize) {
        let next = self.nodes[anchor].next;
        self.nodes[node].prev = Some(anchor);
        self.nodes[node].next = next;
        self.nodes[anchor].next = Some(node);
        match next {
            Some(n) => self.nodes[n].prev = Some(node),
            None => self.tail = Some(node),
        }
    }

    /// Deactivate a delimiter and splice it out of the delimiter stack.
    fn remove_delim(&mut self, d: usize) {
        if !self.delims[d].active {
            return;
        }
        self.delims[d].active = false;
        let prev = self.delims[d].prev;
        let next = self.delims[d].next;
        if let Some(p) = prev {
            self.delims[p].next = next;
        }
        match next {
            Some(n) => self.delims[n].prev = prev,
            None => self.last_delim = prev,
        }
    }

    /// Remove every delimiter strictly between `opener` and `closer`.
    fn remove_delims_between(&mut self, opener: usize, closer: usize) {
        let mut d = self.delims[opener].next;
        while let Some(cur) = d {
            if cur == closer {
                break;
            }
            let next = self.delims[cur].next;
            self.remove_delim(cur);
            d = next;
        }
    }

    /// Emit the resolved top-level node list as events.
    fn emit(&self, out: &mut Vec<Event>) {
        let mut cur = self.head;
        while let Some(n) = cur {
            self.emit_node(n, out);
            cur = self.nodes[n].next;
        }
    }

    fn emit_node(&self, n: usize, out: &mut Vec<Event>) {
        match &self.nodes[n].data {
            NodeData::Token(idx) => out.push(Event::Tok(*idx)),
            NodeData::Delim(s) => {
                if !s.is_empty() {
                    out.push(Event::Leaf(SyntaxKind::ROXYGEN_MD_DELIM, s.clone()));
                }
            }
            NodeData::Emph {
                strong,
                open,
                close,
            } => {
                let kind = if *strong {
                    SyntaxKind::ROXYGEN_MD_STRONG
                } else {
                    SyntaxKind::ROXYGEN_MD_EMPH
                };
                out.push(Event::Start(kind));
                out.push(Event::Leaf(SyntaxKind::ROXYGEN_MD_DELIM, open.clone()));
                let mut child = self.nodes[n].first_child;
                while let Some(c) = child {
                    self.emit_node(c, out);
                    child = self.nodes[c].next;
                }
                out.push(Event::Leaf(SyntaxKind::ROXYGEN_MD_DELIM, close.clone()));
                out.push(Event::Finish);
            }
            NodeData::Link { open, close, body } => {
                // A `ROXYGEN_MD_LINK` node: the brackets are opener/closer
                // `ROXYGEN_MD_DELIM` leaves around the already-resolved link-text
                // events (so the projector skips first/last child, as for emphasis).
                out.push(Event::Start(SyntaxKind::ROXYGEN_MD_LINK));
                out.push(Event::Leaf(SyntaxKind::ROXYGEN_MD_DELIM, open.clone()));
                out.extend(body.iter().cloned());
                out.push(Event::Leaf(SyntaxKind::ROXYGEN_MD_DELIM, close.clone()));
                out.push(Event::Finish);
            }
            NodeData::Text(s) => {
                if !s.is_empty() {
                    out.push(Event::Leaf(SyntaxKind::ROXYGEN_TEXT, s.clone()));
                }
            }
            NodeData::Span(events) => out.extend(events.iter().cloned()),
        }
    }
}

/// Whether a `RoxygenMdBracket` token text is an inline-link opener (`[` or `![`).
fn is_bracket_open(text: &str) -> bool {
    text.starts_with('[') || text.starts_with('!')
}

/// The role of a run position after bracket matching: a matched link opener
/// (collapsed into a Link node), a literal bracket (an unmatched/deactivated
/// opener, or a closer that formed no link), a consumed closer or reference label
/// (emitted as part of its opener's link), or a non-bracket position (`Other`).
enum BracketRole {
    MatchedOpener {
        closer: usize,
        after: usize,
        close_text: String,
    },
    LiteralBracket,
    Consumed,
    Other,
}

/// Match the link brackets in `run` left-to-right with CommonMark opener
/// deactivation (`look_for_link_or_image`): each `]` closes the nearest *active*
/// `[` opener, and forming a link deactivates every opener still below it on the
/// stack — so nested links resolve inner-first and the enclosing brackets stay
/// literal (`[a [b] c](url)` → literal `[a `, `\link{b}`, literal ` c](url)`).
/// Images are opaque leaves and autolinks are not brackets, so neither reaches
/// here; every opener is a link opener. Returns a role per run position.
fn match_brackets(tokens: &[Token], run: &[RunItem]) -> Vec<BracketRole> {
    let mut roles = Vec::with_capacity(run.len());
    roles.resize_with(run.len(), || BracketRole::Other);
    // Stack of (run position of an open `[`, still active).
    let mut stack: Vec<(usize, bool)> = Vec::new();
    let mut q = 0;
    while q < run.len() {
        let Some(tok) = item_token(tokens, &run[q]) else {
            q += 1;
            continue;
        };
        if tok.kind != TokKind::RoxygenMdBracket {
            q += 1;
            continue;
        }
        if is_bracket_open(&tok.text) {
            stack.push((q, true));
            roles[q] = BracketRole::LiteralBracket; // until matched below
            q += 1;
            continue;
        }
        // A closer (`]`-shaped): pop the nearest opener.
        let Some((o_pos, active)) = stack.pop() else {
            roles[q] = BracketRole::LiteralBracket;
            q += 1;
            continue;
        };
        if !active {
            roles[q] = BracketRole::LiteralBracket;
            q += 1;
            continue;
        }
        match classify_closer(tokens, run, o_pos, q) {
            Some((close_text, after)) => {
                roles[o_pos] = BracketRole::MatchedOpener {
                    closer: q,
                    after,
                    close_text,
                };
                // Consume the closer and any folded-in `[ref]` label tokens
                // (`q + 1 .. after`); for an inline/shortcut closer the range is just
                // the closer itself (`after == q + 1`).
                for role in &mut roles[q..after] {
                    *role = BracketRole::Consumed;
                }
                // A link (never an image here) deactivates the openers below it.
                for e in stack.iter_mut() {
                    e.1 = false;
                }
                q = after;
            }
            None => {
                roles[q] = BracketRole::LiteralBracket;
                q += 1;
            }
        }
    }
    roles
}

/// Classify the closer at `run[closer_q]` for an active opener at `run[o_pos]`,
/// returning `(close_text, after)` for a valid link or `None` for none, where
/// `after` is the run index just past the closer and any folded-in `[ref]` label.
/// An inline `](url)` composite closer is always a valid link. A lone `]` is a
/// *reference* link when immediately followed by a `[ref]` label — neutral `[`/`]`
/// bracket tokens on the lookahead, folded in as `][ref]` (or, for a `\`-bearing
/// label, the legacy opaque `scan_md_link` leaf). Otherwise it is a *shortcut* link
/// iff the opener's raw interior is bracket-free (roxygen synthesizes a reference
/// only for a bracket-free shortcut label) — a bracket-bearing shortcut is not a link.
fn classify_closer(
    tokens: &[Token],
    run: &[RunItem],
    o_pos: usize,
    closer_q: usize,
) -> Option<(String, usize)> {
    let close_tok = item_token(tokens, &run[closer_q])?;
    if close_tok.text != "]" {
        return Some((close_tok.text.clone(), closer_q + 1));
    }
    // A reference label `[ref]` on the lookahead, as neutral bracket tokens.
    if let Some((label, after)) = neutral_ref_label(tokens, run, closer_q + 1) {
        return Some((format!("][{label}]"), after));
    }
    // Legacy: a `\`-bearing label still carved as one opaque `scan_md_link` leaf.
    if let Some(next) = run.get(closer_q + 1)
        && let Some(tok) = item_token(tokens, next)
        && tok.kind == TokKind::RoxygenMdLink
    {
        return Some((format!("]{}", tok.text), closer_q + 2));
    }
    interior_bracket_free(tokens, run, o_pos, closer_q).then(|| ("]".to_string(), closer_q + 1))
}

/// If `run[label_open]` is a neutral `[` bracket opening a bracket-free reference
/// label, return `(label_text, after)` where `after` is the run index just past the
/// label's closing `]`. The label runs to the next neutral `]` bracket (the opener
/// carve guarantees a bracket-free interior, so the first `]` closes it). `None`
/// when `run[label_open]` is not a neutral `[` opener or no closing `]` follows.
fn neutral_ref_label(
    tokens: &[Token],
    run: &[RunItem],
    label_open: usize,
) -> Option<(String, usize)> {
    let open = run.get(label_open).and_then(|it| item_token(tokens, it))?;
    if open.kind != TokKind::RoxygenMdBracket || !open.text.starts_with('[') {
        return None;
    }
    let mut label = String::new();
    let mut k = label_open + 1;
    while let Some(item) = run.get(k) {
        if let Some(tok) = item_token(tokens, item)
            && tok.kind == TokKind::RoxygenMdBracket
        {
            return (tok.text == "]").then_some((label, k + 1));
        }
        label.push_str(item_text(tokens, item));
        k += 1;
    }
    None
}

/// Whether the raw interior between an opener at `run[o_pos]` and a closer at
/// `run[closer_q]` carries no *active* `[`/`]` in any item — the cmark
/// label-content validity test (a bare-bracket label is not a link). A
/// backslash-escaped `\[` is content (mirrors `is_shortcut_content` in the
/// lexer); any `]`, even escaped, still rejects (the escaped-close backlog).
fn interior_bracket_free(tokens: &[Token], run: &[RunItem], o_pos: usize, closer_q: usize) -> bool {
    run[o_pos + 1..closer_q].iter().all(|item| {
        let bytes = item_text(tokens, item).as_bytes();
        bytes.iter().enumerate().all(|(k, &b)| match b {
            b']' => false,
            b'[' => k > 0 && bytes[k - 1] == b'\\',
            _ => true,
        })
    })
}

/// CommonMark flanking for a delimiter run of char `ch`, given the characters
/// immediately before and after the run (`None` = start/end of the inline run,
/// treated as whitespace). ASCII punctuation classification (the Unicode-class
/// refinement is a noted backlog item — it only differs when a non-ASCII
/// punctuation char abuts a delimiter).
fn flanking(ch: u8, before: Option<char>, after: Option<char>) -> (bool, bool) {
    let before_ws = is_ws(before);
    let after_ws = is_ws(after);
    let before_punct = is_punct(before);
    let after_punct = is_punct(after);

    let left_flanking = !after_ws && (!after_punct || before_ws || before_punct);
    let right_flanking = !before_ws && (!before_punct || after_ws || after_punct);

    match ch {
        b'_' => (
            left_flanking && (!right_flanking || before_punct),
            right_flanking && (!left_flanking || after_punct),
        ),
        // `*` (and any other delimiter char routed here).
        _ => (left_flanking, right_flanking),
    }
}

/// The flanking-relevant edge char of a run neighbor: the first char (`leading`)
/// or the last char of its text. A `#'` marker token is mapped to a space — an
/// inter-line continuation is a soft break, which CommonMark treats as whitespace
/// (newline/whitespace tokens already yield a whitespace char). `None` (an empty
/// token) falls back to the start/end boundary, also whitespace.
///
/// An inline Rd macro (`\code{…}`, `\link{…}`, `\emph{…}`, …) is **opaque** to the
/// markdown pass exactly as it is to roxygen2's cmark: roxygen2 replaces a fragile
/// tag with an alphanumeric placeholder (`make_random_string`) suffixed `-<i>-`
/// (`str_sub_same`) *before* parsing, and a non-fragile tag is plain text whose
/// markdown the arg-recursion handles. So for flanking a macro must present that
/// placeholder's shape, **not** its own `\`/`}` punctuation bytes: an alphanumeric
/// at the **leading** edge (so a `*` opener abutting the macro can open —
/// `a*\code{x} y*` → `\emph{\code{x} y}`) and the trailing `-` at the **trailing**
/// edge (so a `*` closer abutting the macro stays blocked, as the placeholder's `-`
/// blocks it — `a*\code{x}*b` keeps both `*` literal). Without this the macro's raw
/// `\` (leading) / `}` (trailing) punctuation would wrongly suppress a span.
fn edge_char(tok: &Token, leading: bool) -> Option<char> {
    if tok.kind == TokKind::RoxygenMarker {
        return Some(' ');
    }
    if tok.kind == TokKind::RoxygenRdMacro {
        return Some(if leading { 'x' } else { '-' });
    }
    if leading {
        tok.text.chars().next()
    } else {
        tok.text.chars().next_back()
    }
}

/// A start/end boundary (`None`) counts as whitespace, per CommonMark.
fn is_ws(c: Option<char>) -> bool {
    c.is_none_or(char::is_whitespace)
}

/// ASCII-punctuation classification (CommonMark's punctuation set, ASCII subset).
fn is_punct(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_punctuation())
}
