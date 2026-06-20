//! Incremental reparse: turn a single text edit plus the previous parse tree
//! into a new tree without re-lexing and re-parsing the whole file.
//!
//! Modelled on rust-analyzer's `reparsing.rs`: try the cheapest strategy first
//! and fall back to progressively more work.
//!
//! 1. [`reparse_token`] — the edit lands inside one leaf token (identifier,
//!    string, comment, whitespace) and re-lexing that token's text yields the
//!    same single token kind. Splice a fresh leaf in place.
//! 2. [`reparse_block`] — the edit lands strictly inside a `{ … }` block. Re-lex
//!    and re-parse just that block (it is self-contained — braces delimit it, so
//!    its subtree never depends on the surrounding context) and splice the new
//!    block subtree in place.
//! 3. Otherwise return `None`; the caller does a full [`crate::parser::parse`].
//!
//! **Correctness invariant (Tenet 4):** a successful reparse must yield a green
//! tree *and* diagnostics byte-identical to a full parse of the edited text.
//! Splicing relies on rowan green nodes being position-independent. The invariant
//! is enforced by `tests/incremental_reparse.rs` (an oracle property test over the
//! corpus) and a `debug_assert!` here.

use std::ops::Range;

use rowan::{GreenNode, GreenToken, TextRange, TextSize};

use crate::parser::bracket_balancer::rebalance_brackets;
use crate::parser::diagnostics::ParseDiagnostic;
use crate::parser::expr::parse_expr;
use crate::parser::lexer::lex;
use crate::parser::tree_builder::{build_tree, syntax_kind_for};
use crate::syntax::{SyntaxKind, SyntaxNode};

/// A single contiguous text edit: replace `range` (a byte range in the *old*
/// text) with `insert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub insert: String,
}

impl Edit {
    /// The signed length change this edit applies to text after `range`.
    fn delta(&self) -> isize {
        self.insert.len() as isize - (self.range.end - self.range.start) as isize
    }

    /// Apply the edit to `old`, producing the new text.
    pub fn apply(&self, old: &str) -> String {
        let mut out =
            String::with_capacity(old.len().saturating_sub(self.range.len()) + self.insert.len());
        out.push_str(&old[..self.range.start]);
        out.push_str(&self.insert);
        out.push_str(&old[self.range.end..]);
        out
    }
}

/// Map a `TextRange` taken against the text *before* `edit` to its position in
/// the text *after* `edit`.
///
/// Returns `None` when the edit's replaced range overlaps the node's interior:
/// a `(kind, range)` handle cannot promise the node survived an edit inside it
/// (its text, and possibly its kind, may have changed). Comparisons are
/// half-open, so the boundary cases are deterministic: an edit ending exactly at
/// the node's start shifts the node (it survives), an edit starting exactly at
/// the node's end leaves it unchanged (it survives), and an insertion strictly
/// inside the node returns `None`. A zero-length node sitting at an insertion
/// point is treated as preceding the insertion (unchanged).
///
/// Note: [`parsed_document`](crate::incremental::parsed_document) recovers a
/// single spanning [`diff_edit`], so several disjoint keystrokes coalesce into
/// one wide edit. That only widens the overlap and yields `None` more often
/// (more position-based re-resolution by the caller) — it never produces a wrong
/// mapping. Feeding precise per-change LSP edits would shrink the edit and
/// reduce those fallbacks.
pub fn map_range_through_edit(range: TextRange, edit: &Edit) -> Option<TextRange> {
    let node_start = usize::from(range.start());
    let node_end = usize::from(range.end());

    if node_end <= edit.range.start {
        // Node entirely before the edit: untouched.
        Some(range)
    } else if node_start >= edit.range.end {
        // Node entirely after the edit: every offset shifts by the length delta.
        let start = (node_start as isize + edit.delta()) as usize;
        let end = (node_end as isize + edit.delta()) as usize;
        Some(text_range(start, end))
    } else {
        // The edit's replaced range intersects the node's interior: invalidated.
        None
    }
}

/// Map a range through a sequence of edits applied left-to-right (the order
/// [`Edit::apply`] would run them — each edit is expressed against the text
/// produced by its predecessors). Any single invalidating edit collapses the
/// whole result to `None`.
pub fn map_range_through_edits(range: TextRange, edits: &[Edit]) -> Option<TextRange> {
    edits.iter().try_fold(range, map_range_through_edit)
}

/// Which strategy produced a [`Reparsed`]. Surfaced for tests and benchmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReparseKind {
    Token,
    Block,
}

/// The result of a successful incremental reparse: the new whole-file green tree
/// and its parse diagnostics (absolute offsets in the new text).
#[derive(Debug, Clone)]
pub struct Reparsed {
    pub green: GreenNode,
    pub diagnostics: Vec<ParseDiagnostic>,
    pub kind: ReparseKind,
}

/// Recover a single contiguous [`Edit`] (deleted range in `old` + inserted text)
/// from a pair of full texts by stripping the common prefix and suffix. Multiple
/// disjoint edits collapse into one spanning edit — still a correct transform,
/// just coarser. Boundaries are clamped to char boundaries.
pub fn diff_edit(old: &str, new: &str) -> Edit {
    let ob = old.as_bytes();
    let nb = new.as_bytes();

    let mut prefix = 0;
    let max_prefix = ob.len().min(nb.len());
    while prefix < max_prefix && ob[prefix] == nb[prefix] {
        prefix += 1;
    }
    while prefix > 0 && !old.is_char_boundary(prefix) {
        prefix -= 1;
    }

    let mut suffix = 0;
    let max_suffix = (ob.len() - prefix).min(nb.len() - prefix);
    while suffix < max_suffix && ob[ob.len() - 1 - suffix] == nb[nb.len() - 1 - suffix] {
        suffix += 1;
    }
    while suffix > 0
        && (!old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix))
    {
        suffix -= 1;
    }

    Edit {
        range: prefix..(old.len() - suffix),
        insert: new[prefix..(new.len() - suffix)].to_string(),
    }
}

/// Attempt an incremental reparse of `old_root` (parsed from `old_text`, with
/// `old_diags`) under `edit`. Returns `None` when no incremental strategy applies
/// — the caller must then do a full parse.
pub fn reparse(
    old_root: &SyntaxNode,
    old_text: &str,
    old_diags: &[ParseDiagnostic],
    edit: &Edit,
) -> Option<Reparsed> {
    let result = reparse_token(old_root, old_text, old_diags, edit)
        .or_else(|| reparse_block(old_root, old_text, old_diags, edit));

    if let Some(reparsed) = &result {
        debug_assert_eq!(
            SyntaxNode::new_root(reparsed.green.clone())
                .text()
                .to_string(),
            edit.apply(old_text),
            "incremental reparse ({:?}) is not lossless",
            reparsed.kind,
        );
    }
    result
}

const TOKEN_REPARSE_KINDS: &[SyntaxKind] = &[
    SyntaxKind::IDENT,
    SyntaxKind::STRING,
    SyntaxKind::COMMENT,
    SyntaxKind::WHITESPACE,
    // Roxygen leaves are deliberately absent: they only arise from the `#'`-line
    // lexer path, so relexing one in isolation (`lex("A number.")`) never yields
    // a roxygen token and the single-token guard would always bail. Edits inside
    // a roxygen line therefore fall back to block reparse (when the block sits in
    // a `{}` body) or a full reparse — correct, just not token-incremental.
];

fn reparse_token(
    old_root: &SyntaxNode,
    old_text: &str,
    old_diags: &[ParseDiagnostic],
    edit: &Edit,
) -> Option<Reparsed> {
    let (s, e) = (edit.range.start, edit.range.end);
    let elem = old_root.covering_element(text_range(s, e));
    let token = elem.into_token()?;
    if !TOKEN_REPARSE_KINDS.contains(&token.kind()) {
        return None;
    }

    let tr = token.text_range();
    let (t0, t1) = (usize::from(tr.start()), usize::from(tr.end()));
    // A pure insertion must land strictly inside the token, so it is
    // unambiguous which token it extends. Edits with a non-empty range that
    // span a token boundary never reach here — `covering_element` returns the
    // parent node, not a token.
    if s == e && !(t0 < s && s < t1) {
        return None;
    }

    // The token's new text, with the edit applied at its relative offset.
    let mut new_text = token.text().to_string();
    new_text.replace_range((s - t0)..(e - t0), &edit.insert);

    // Re-lex in isolation: it must still be exactly one token of the same kind.
    let relexed = lex(&new_text);
    let [only] = relexed.as_slice() else {
        return None;
    };
    if only.start != 0 || only.end != new_text.len() || syntax_kind_for(&only.kind) != token.kind()
    {
        return None;
    }

    // Merge guard: appending the following source character must not extend the
    // token (which would mean the edit merged it with its neighbour, e.g.
    // `for`→`fore`, or an unterminated string swallowing the next line).
    if let Some(next_char) = old_text[t1..].chars().next() {
        let mut probe = new_text.clone();
        probe.push(next_char);
        let probe_toks = lex(&probe);
        let first = probe_toks.first()?;
        if first.end != new_text.len() || syntax_kind_for(&first.kind) != token.kind() {
            return None;
        }
    }

    // Keep diagnostic remapping trivial: only proceed when no diagnostic touches
    // the edited token. A clean single-token relex introduces none of its own.
    if old_diags.iter().any(|d| d.start < t1 && d.end > t0) {
        return None;
    }
    let delta = edit.delta();
    let diagnostics = old_diags
        .iter()
        .map(|d| {
            if d.start >= t1 {
                shift(d, delta)
            } else {
                d.clone()
            }
        })
        .collect();

    let green_token = GreenToken::new(rowan::SyntaxKind::from(token.kind()), &new_text);
    let green = token.replace_with(green_token);
    Some(Reparsed {
        green,
        diagnostics,
        kind: ReparseKind::Token,
    })
}

fn reparse_block(
    old_root: &SyntaxNode,
    old_text: &str,
    old_diags: &[ParseDiagnostic],
    edit: &Edit,
) -> Option<Reparsed> {
    let (s, e) = (edit.range.start, edit.range.end);
    let elem = old_root.covering_element(text_range(s, e));
    let start_node = match elem {
        rowan::NodeOrToken::Node(n) => n,
        rowan::NodeOrToken::Token(t) => t.parent()?,
    };

    // Smallest enclosing `{ … }` whose *interior* (between the braces) fully
    // contains the edit, so the brace delimiters themselves are untouched.
    let block = start_node.ancestors().find(|node| {
        if node.kind() != SyntaxKind::BLOCK_EXPR {
            return false;
        }
        let r = node.text_range();
        let (bstart, bend) = (usize::from(r.start()), usize::from(r.end()));
        // First/last tokens are `{`/`}` (one byte each); require interior containment.
        bend.saturating_sub(bstart) >= 2 && s > bstart && e < bend
    })?;

    let r = block.text_range();
    let (bstart, bend) = (usize::from(r.start()), usize::from(r.end()));

    // New block text: the old block slice with the edit applied at its relative
    // offset. The edit is interior, so the slice still starts with `{`/ends `}`.
    let mut block_text = old_text[bstart..bend].to_string();
    block_text.replace_range((s - bstart)..(e - bstart), &edit.insert);

    let (block_green, block_diags) = parse_block_in_isolation(&block_text)?;

    let delta = edit.delta();
    let mut diagnostics: Vec<ParseDiagnostic> =
        Vec::with_capacity(old_diags.len() + block_diags.len());
    for d in old_diags {
        if d.end <= bstart {
            diagnostics.push(d.clone());
        } else if d.start >= bend {
            diagnostics.push(shift(d, delta));
        }
        // diagnostics inside the old block are dropped; the reparse regenerates them
    }
    for d in &block_diags {
        diagnostics.push(ParseDiagnostic {
            message: d.message.clone(),
            start: d.start + bstart,
            end: d.end + bstart,
        });
    }
    diagnostics.sort_by_key(|d| (d.start, d.end));

    let green = block.replace_with(block_green);
    Some(Reparsed {
        green,
        diagnostics,
        kind: ReparseKind::Block,
    })
}

/// Parse `{ … }` text on its own and return the green node for the single
/// `BLOCK_EXPR` it produces, with block-relative diagnostics. Returns `None` if
/// the text does not parse to exactly one block consuming all tokens (e.g. the
/// edit transiently unbalanced the braces) — the caller falls back.
fn parse_block_in_isolation(block_text: &str) -> Option<(GreenNode, Vec<ParseDiagnostic>)> {
    let tokens = rebalance_brackets(lex(block_text));
    let mut diagnostics = Vec::new();
    let expr = parse_expr(&tokens, 0, 0, &mut diagnostics)?;
    if expr.start != 0 || expr.end != tokens.len() {
        return None;
    }

    let root = build_tree(&tokens, &expr.events);
    let mut children = root.children();
    let block = children.next()?;
    if children.next().is_some() || block.kind() != SyntaxKind::BLOCK_EXPR {
        return None;
    }

    // The block must still be properly brace-delimited by its own *direct*
    // delimiters. If the edit opened an unterminated string/comment or an
    // unclosed inner `(`/`[` that swallowed the closing `}` (re-parenting it
    // under a child, or leaking past the old block boundary in a full parse),
    // the block is no longer self-contained — fall back.
    let first = block.first_child_or_token().map(|e| e.kind());
    let last = block.last_child_or_token().map(|e| e.kind());
    if first != Some(SyntaxKind::LBRACE) || last != Some(SyntaxKind::RBRACE) {
        return None;
    }

    Some((block.green().into_owned(), diagnostics))
}

fn shift(d: &ParseDiagnostic, delta: isize) -> ParseDiagnostic {
    ParseDiagnostic {
        message: d.message.clone(),
        start: (d.start as isize + delta) as usize,
        end: (d.end as isize + delta) as usize,
    }
}

fn text_range(start: usize, end: usize) -> TextRange {
    TextRange::new(TextSize::new(start as u32), TextSize::new(end as u32))
}

#[cfg(test)]
mod map_range_tests {
    use super::*;

    fn edit(start: usize, end: usize, insert: &str) -> Edit {
        Edit {
            range: start..end,
            insert: insert.to_string(),
        }
    }

    #[test]
    fn node_before_edit_is_unchanged() {
        let node = text_range(0, 5);
        assert_eq!(
            map_range_through_edit(node, &edit(10, 12, "xxxx")),
            Some(node)
        );
    }

    #[test]
    fn node_after_insertion_shifts_by_delta() {
        let node = text_range(10, 15);
        // Insert 4 chars where 2 stood before the node: net +2.
        assert_eq!(
            map_range_through_edit(node, &edit(0, 2, "xxxx")),
            Some(text_range(12, 17))
        );
    }

    #[test]
    fn node_after_deletion_shifts_back() {
        let node = text_range(10, 15);
        assert_eq!(
            map_range_through_edit(node, &edit(0, 4, "")),
            Some(text_range(6, 11))
        );
    }

    #[test]
    fn edit_ending_at_node_start_shifts_node() {
        // ee == ns: the node survives and shifts by the delta.
        let node = text_range(10, 15);
        assert_eq!(
            map_range_through_edit(node, &edit(8, 10, "abcd")),
            Some(text_range(12, 17))
        );
    }

    #[test]
    fn insertion_at_node_start_shifts_node() {
        let node = text_range(10, 15);
        assert_eq!(
            map_range_through_edit(node, &edit(10, 10, "ab")),
            Some(text_range(12, 17))
        );
    }

    #[test]
    fn insertion_at_node_end_leaves_node_unchanged() {
        let node = text_range(10, 15);
        assert_eq!(
            map_range_through_edit(node, &edit(15, 15, "ab")),
            Some(node)
        );
    }

    #[test]
    fn insertion_strictly_inside_invalidates() {
        let node = text_range(10, 15);
        assert_eq!(map_range_through_edit(node, &edit(12, 12, "z")), None);
    }

    #[test]
    fn overlapping_deletion_invalidates() {
        let node = text_range(10, 15);
        assert_eq!(map_range_through_edit(node, &edit(12, 18, "")), None);
    }

    #[test]
    fn zero_length_node_at_insertion_point_is_unchanged() {
        // Deterministic boundary rule: a zero-length node at the insertion point
        // is treated as preceding the insertion.
        let node = text_range(10, 10);
        assert_eq!(
            map_range_through_edit(node, &edit(10, 10, "ab")),
            Some(node)
        );
    }

    #[test]
    fn append_at_eof_leaves_earlier_node_unchanged() {
        let node = text_range(0, 5);
        assert_eq!(
            map_range_through_edit(node, &edit(5, 5, "\n# trailing")),
            Some(node)
        );
    }

    #[test]
    fn map_through_edits_folds_in_order() {
        let node = text_range(10, 15);
        // Two insertions before the node, each expressed against the prior text.
        let edits = [edit(0, 0, "ab"), edit(0, 0, "cd")];
        assert_eq!(
            map_range_through_edits(node, &edits),
            Some(text_range(14, 19))
        );
    }

    #[test]
    fn map_through_edits_invalidates_on_any_overlap() {
        let node = text_range(10, 15);
        // After the first edit the node is at [12, 17); the second lands inside.
        let edits = [edit(0, 0, "ab"), edit(13, 13, "z")];
        assert_eq!(map_range_through_edits(node, &edits), None);
    }
}
