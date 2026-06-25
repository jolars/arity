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

/// Resolve markdown emphasis/strong in `events` (in place). A no-op unless the
/// block carries at least one raw delimiter run.
pub(super) fn resolve_emphasis(tokens: &[Token], events: &mut Vec<Event>) {
    let has_delim = events
        .iter()
        .any(|e| matches!(e, Event::Tok(i) if tokens[*i].kind == TokKind::RoxygenMdDelim));
    if !has_delim {
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

/// Resolve one inline run (the token indices in `run`) and append its events to
/// `out`, then clear `run`. A run with no delimiter re-emits its tokens verbatim
/// (byte-identical), so only delimiter-bearing runs are rebuilt.
fn flush_run(tokens: &[Token], run: &mut Vec<usize>, out: &mut Vec<Event>) {
    if run.is_empty() {
        return;
    }
    if !run
        .iter()
        .any(|&i| tokens[i].kind == TokKind::RoxygenMdDelim)
    {
        out.extend(run.drain(..).map(Event::Tok));
        return;
    }
    let mut arena = Arena::build(tokens, run);
    arena.process_emphasis();
    arena.emit(out);
    run.clear();
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
    fn build(tokens: &[Token], run: &[usize]) -> Arena {
        let mut arena = Arena {
            nodes: Vec::new(),
            delims: Vec::new(),
            head: None,
            tail: None,
            last_delim: None,
        };
        // Precompute, per run position, the first/last char of each token's text,
        // for flanking (the char immediately before/after a delimiter run).
        for (p, &idx) in run.iter().enumerate() {
            let tok = &tokens[idx];
            if tok.kind == TokKind::RoxygenMdDelim {
                let ch = tok.text.as_bytes()[0];
                let len = tok.text.len(); // a same-char ASCII run: bytes == chars
                // For flanking, the char immediately before/after the run. Inter-line
                // trivia (a soft break: newline, the `#'` marker, leading whitespace)
                // counts as whitespace — the newline/whitespace bytes already are, and
                // the marker is mapped to one (`edge_char`).
                let before = p
                    .checked_sub(1)
                    .and_then(|q| edge_char(&tokens[run[q]], false));
                let after = run.get(p + 1).and_then(|&j| edge_char(&tokens[j], true));
                let (can_open, can_close) = flanking(ch, before, after);
                let node = arena.push_node(NodeData::Delim(tok.text.clone()));
                arena.push_delim(node, ch, len, can_open, can_close);
            } else {
                arena.push_node(NodeData::Token(idx));
            }
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
        }
    }
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
