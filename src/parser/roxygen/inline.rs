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
//! Code spans, links, images, and raw HTML stay opaque local-span leaves, resolved
//! by the lexer *before* this pass — matching CommonMark precedence (they bind
//! tighter than emphasis), so the pass treats each as one opaque inline.

use crate::parser::events::Event;
use crate::parser::lexer::{TokKind, Token};
use crate::syntax::SyntaxKind;

/// Whether `kind` is a raw markdown-inline-markup token the pass resolves: an
/// emphasis delimiter run or an inline-link bracket. A run carrying neither is
/// re-emitted verbatim.
fn is_inline_markup(kind: &TokKind) -> bool {
    matches!(kind, TokKind::RoxygenMdDelim | TokKind::RoxygenMdBracket)
}

/// Resolve markdown emphasis/strong and inline links in `events` (in place). A
/// no-op unless the block carries at least one raw delimiter run or link bracket.
pub(super) fn resolve_emphasis(tokens: &[Token], events: &mut Vec<Event>) {
    let has_markup = events
        .iter()
        .any(|e| matches!(e, Event::Tok(i) if is_inline_markup(&tokens[*i].kind)));
    if !has_markup {
        return;
    }

    let mut out = Vec::with_capacity(events.len());
    let mut run: Vec<usize> = Vec::new();
    for ev in std::mem::take(events) {
        match ev {
            // Every paragraph-body token joins the run — content *and* the
            // inter-line trivia (newline / `#'` marker / whitespace) a continuation
            // folds in — so a span resolves across soft line breaks. A structural
            // event (a paragraph/section/tag boundary, or an inline `ROXYGEN_RD_MACRO`
            // which binds tighter than emphasis) bounds the run.
            Event::Tok(i) => run.push(i),
            other => {
                flush_run(tokens, &mut run, &mut out);
                out.push(other);
            }
        }
    }
    flush_run(tokens, &mut run, &mut out);
    *events = out;
}

/// Resolve one top-level inline run and append its events to `out`, then clear
/// `run`. A run with no markup re-emits its tokens verbatim (byte-identical), so
/// only markup-bearing runs are rebuilt. The run's edges are the start/end of the
/// inline content (whitespace, for flanking).
fn flush_run(tokens: &[Token], run: &mut Vec<usize>, out: &mut Vec<Event>) {
    if run.is_empty() {
        return;
    }
    if !run.iter().any(|&i| is_inline_markup(&tokens[i].kind)) {
        out.extend(run.drain(..).map(Event::Tok));
        return;
    }
    resolve_run(tokens, run, None, None, out);
    run.clear();
}

/// Resolve an inline run (`run` token indices) into events appended to `out`,
/// given the flanking-relevant characters immediately `before`/`after` the run
/// (`None` = a whitespace boundary). Builds the arena (collapsing inline links
/// into opaque `ROXYGEN_MD_LINK` nodes, their text resolved by a recursive call),
/// resolves emphasis over the resulting top-level node list, and emits.
fn resolve_run(
    tokens: &[Token],
    run: &[usize],
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
    fn build(tokens: &[Token], run: &[usize], before: Option<char>, after: Option<char>) -> Arena {
        let mut arena = Arena {
            nodes: Vec::new(),
            delims: Vec::new(),
            head: None,
            tail: None,
            last_delim: None,
        };
        // The flanking neighbor char at run position `p` on the given side: an
        // interior position reads its neighbor token's edge char; a run edge uses
        // the passed boundary (the link-text `[`/`]` for a recursive call, else ws).
        let neighbor = |p: usize, leading: bool| -> Option<char> {
            if leading {
                run.get(p + 1)
                    .map_or(after, |&j| edge_char(&tokens[j], true))
            } else {
                match p.checked_sub(1) {
                    Some(q) => edge_char(&tokens[run[q]], false),
                    None => before,
                }
            }
        };
        let mut p = 0;
        while p < run.len() {
            let tok = &tokens[run[p]];
            if tok.kind == TokKind::RoxygenMdBracket {
                // An opener (`[`/`![`) collapses with its matching closer into one
                // Link node; the inner tokens resolve recursively, bounded by the
                // bracket chars (`[` before, `]` after) for flanking. The closer is
                // either an inline `](url)` leaf or — for a cross-line *reference*
                // link `[text][ref]` — a lone `]` leaf immediately followed by the
                // `[ref]` label token (consumed as the dropped topic, folded into the
                // closer text as `][ref]` so the projector resolves `\link{text}`).
                if is_bracket_open(&tok.text)
                    && let Some((close_p, close, after_p)) = find_link_closer(tokens, run, p)
                {
                    let open = tok.text.clone();
                    let mut body = Vec::new();
                    resolve_run(
                        tokens,
                        &run[p + 1..close_p],
                        open.chars().next_back(),
                        Some(']'),
                        &mut body,
                    );
                    arena.push_node(NodeData::Link { open, close, body });
                    p = after_p;
                    continue;
                }
                // An unmatched bracket re-emits as literal text (a `Delim` node,
                // projected as plain text) — e.g. a lone `]` reference closer whose
                // opener never appeared, leaving the `[ref]` label a standalone
                // shortcut token. (Same-line link brackets never reach here.)
                arena.push_node(NodeData::Delim(tok.text.clone()));
                p += 1;
                continue;
            }
            if tok.kind == TokKind::RoxygenMdDelim {
                let ch = tok.text.as_bytes()[0];
                let len = tok.text.len(); // a same-char ASCII run: bytes == chars
                let (can_open, can_close) = flanking(ch, neighbor(p, false), neighbor(p, true));
                let node = arena.push_node(NodeData::Delim(tok.text.clone()));
                arena.push_delim(node, ch, len, can_open, can_close);
            } else {
                arena.push_node(NodeData::Token(run[p]));
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
        }
    }
}

/// Whether a `RoxygenMdBracket` token text is an inline-link opener (`[` or `![`).
fn is_bracket_open(text: &str) -> bool {
    text.starts_with('[') || text.starts_with('!')
}

/// Whether a `RoxygenMdBracket` token text is an inline-link closer (`](url)` or a
/// lone `]` reference closer).
fn is_bracket_close(text: &str) -> bool {
    text.starts_with(']')
}

/// Find the matching closer for an opener at `run[p]`, scanning forward for the
/// first valid closer bracket. Returns `(close_p, close_text, after_p)`:
/// `close_p` is the closer's run position, `close_text` the closer string emitted
/// as the link's closer leaf, and `after_p` the run position to resume from.
///
/// Three closer shapes: an inline `](url)` bracket leaf (`after_p = close_p + 1`); a
/// cross-line *reference* closer — a lone `]` bracket leaf immediately followed by a
/// `[ref]` shortcut-link token, consumed (`after_p = close_p + 2`) and folded into
/// the closer text (`][ref]`) so the projector resolves a reference link, dropping
/// the `[ref]` topic; or a cross-line *shortcut* closer — a lone `]` with no following
/// label, kept as the bare closer text `]` (the projector resolves `\link{display}`
/// from the link text itself). An opener with no later closer at all returns `None`,
/// leaving the opener to re-emit as literal text.
fn find_link_closer(tokens: &[Token], run: &[usize], p: usize) -> Option<(usize, String, usize)> {
    (p + 1..run.len()).find_map(|q| {
        let tok = &tokens[run[q]];
        if tok.kind != TokKind::RoxygenMdBracket || !is_bracket_close(&tok.text) {
            return None;
        }
        if tok.text == "]" {
            match run.get(q + 1).map(|&j| &tokens[j]) {
                // A lone `]` immediately followed by a `[ref]` shortcut token closes a
                // cross-line *reference* link; the label folds into the closer text
                // (`][ref]`) and is consumed as the dropped topic.
                Some(label) if label.kind == TokKind::RoxygenMdLink => {
                    Some((q, format!("]{}", label.text), q + 2))
                }
                // A lone `]` with no following label closes a cross-line *shortcut*
                // link (`[text]`): the closer is just `]`.
                _ => Some((q, "]".to_string(), q + 1)),
            }
        } else {
            Some((q, tok.text.clone(), q + 1))
        }
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
fn edge_char(tok: &Token, leading: bool) -> Option<char> {
    if tok.kind == TokKind::RoxygenMarker {
        return Some(' ');
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
