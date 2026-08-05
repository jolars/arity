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
//! 3. [`reparse_toplevel`] — the edit lands inside a single top-level statement
//!    (a direct child of the `ROOT` node) that isn't inside any `{ … }`. Unlike a
//!    block, a bare statement has no delimiter tokens, so its boundary is pinned
//!    by two guards: a *consume-all* check (the reparse must consume every token
//!    of the statement text — this rules out the statement shrinking and releasing
//!    trailing tokens) and a *forward-merge* guard (re-parsing the statement with
//!    the next sibling appended must still end at the statement's original length
//!    — this rules out the statement growing to absorb what follows, e.g. a
//!    trailing binary operator). The backward direction is inherently safe: R only
//!    continues a statement across a newline via a *trailing* operator, never a
//!    *leading* one, and the previous sibling's tokens are untouched by the edit.
//! 4. Otherwise return `None`; the caller does a full [`crate::parser::parse`].
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

/// Apply `edits` to `old` left-to-right, each expressed against the text its
/// predecessors produced — the inverse operation to [`map_range_through_edits`].
///
/// Used as an apply-and-verify guard by span-mapping consumers: reconstructing
/// the current text from an old snapshot plus an accumulated edit slice proves
/// the slice is the exact transform between them before a range is folded
/// through it, so a stale or misaligned slice can be rejected in favor of the
/// whole-text [`diff_edit`] fallback.
pub fn apply_edits(old: &str, edits: &[Edit]) -> String {
    edits.iter().fold(old.to_string(), |t, e| e.apply(&t))
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
/// Note: the `arity` crate's `incremental::parsed_document` recovers a
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
    TopLevel,
    /// A chained multi-edit reparse ([`reparse_edits`]): several single-edit
    /// strategies applied in sequence. The final tree is still byte-identical to
    /// a full parse.
    Multi,
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
        .or_else(|| reparse_block(old_root, old_text, old_diags, edit))
        .or_else(|| reparse_toplevel(old_root, old_text, old_diags, edit));

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

/// Chain the single-edit [`reparse`] over a sequence of `edits` (each expressed
/// against the text its predecessors produced, in the order [`Edit::apply`]
/// would run them). Each step reparses off the previous step's whole-file tree,
/// so N disjoint edits each reparse their own tight region instead of collapsing
/// into one wide [`diff_edit`] span (the whole point of feeding precise LSP
/// edits through — multi-cursor and find-replace).
///
/// Returns `None` unless *every* step is incrementally reparseable **and** the
/// fully-applied text equals `target`. The `== target` check is the Tenet-4
/// safety net: an edit sequence that does not actually transform `old_text` into
/// the current buffer (a coalescing gap, an interleaved disk write, a
/// full-document replacement) is rejected here so the caller falls back to the
/// always-correct [`diff_edit`] path. A successful result is therefore
/// byte-identical to a full [`parse`](crate::parser::parse) of `target` — the
/// composition of correct single-edit reparses over the true edit sequence.
pub fn reparse_edits(
    old_root: &SyntaxNode,
    old_text: &str,
    old_diags: &[ParseDiagnostic],
    edits: &[Edit],
    target: &str,
) -> Option<Reparsed> {
    let (first, rest) = edits.split_first()?;

    // Step 0 off the caller's tree; each later step off the prior step's tree.
    let mut reparsed = reparse(old_root, old_text, old_diags, first)?;
    let mut text = first.apply(old_text);
    for edit in rest {
        let root = SyntaxNode::new_root(reparsed.green.clone());
        reparsed = reparse(&root, &text, &reparsed.diagnostics, edit)?;
        text = edit.apply(&text);
    }

    // Tenet-4 guard: the edits must reconstruct the current buffer exactly.
    if text != target {
        return None;
    }
    reparsed.kind = ReparseKind::Multi;
    Some(reparsed)
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
    // token (which would mean the edit merged it with its neighbor, e.g.
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

/// Reparse a single top-level statement (a direct child of `ROOT`) that isn't
/// inside any `{ … }` block. See the module doc for the boundary argument.
fn reparse_toplevel(
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

    // The top-level statement: the ancestor whose parent is `ROOT`. When the
    // covering element is a trivia token directly under `ROOT` (an edit in
    // inter-statement whitespace), `start_node` *is* the root and there is no such
    // ancestor — fall back.
    let stmt = start_node
        .ancestors()
        .find(|node| node.parent().map(|p| p.kind()) == Some(SyntaxKind::ROOT))?;

    // Roxygen blocks come from the dedicated `emit_roxygen_block` path, not
    // `parse_expr`, so reparsing one via `parse_expr` would be wrong. (The kind
    // match in `parse_stmt_in_isolation` would also reject it, but bail early.)
    if stmt.kind() == SyntaxKind::ROXYGEN_BLOCK {
        return None;
    }

    let r = stmt.text_range();
    let (ss, se) = (usize::from(r.start()), usize::from(r.end()));
    // The covering-element walk guarantees the statement contains the edit.
    debug_assert!(ss <= s && e <= se);

    // New statement text: the old statement slice with the edit applied at its
    // relative offset. The edit is interior, so the slice still begins at the
    // statement's first significant token.
    let mut stmt_text = old_text[ss..se].to_string();
    stmt_text.replace_range((s - ss)..(e - ss), &edit.insert);

    // Isolation parse: one expression consuming every token, of the same node
    // kind. Consume-all rejects the *shrink* case (the edit split the statement,
    // e.g. removing a trailing operator, so some tokens leak past the boundary).
    let (stmt_green, stmt_diags) = parse_stmt_in_isolation(&stmt_text, stmt.kind())?;

    // Forward-merge guard: reparsing the statement with the next sibling's text
    // appended must still end exactly at the statement's length. Rejects the
    // *grow* case (the edit made the statement continue onto what follows, e.g. a
    // trailing binary operator). One sibling of context is sufficient and bounded:
    // `parse_expr` reads a single expression, so a merge past the boundary is
    // detected immediately; a statement that doesn't merge into its immediate
    // successor cannot reach anything further.
    let forward_end = match stmt.next_sibling() {
        Some(next) => usize::from(next.text_range().end()),
        None => old_text.len(),
    };
    if !toplevel_boundary_stable(&stmt_text, &old_text[se..forward_end]) {
        return None;
    }

    let delta = edit.delta();
    let mut diagnostics: Vec<ParseDiagnostic> =
        Vec::with_capacity(old_diags.len() + stmt_diags.len());
    for d in old_diags {
        if d.end <= ss {
            diagnostics.push(d.clone());
        } else if d.start >= se {
            diagnostics.push(shift(d, delta));
        }
        // diagnostics inside the old statement are dropped; the reparse regenerates them
    }
    for d in &stmt_diags {
        diagnostics.push(ParseDiagnostic {
            message: d.message.clone(),
            start: d.start + ss,
            end: d.end + ss,
        });
    }
    diagnostics.sort_by_key(|d| (d.start, d.end));

    let green = stmt.replace_with(stmt_green);
    Some(Reparsed {
        green,
        diagnostics,
        kind: ReparseKind::TopLevel,
    })
}

/// Parse a bare top-level statement's text on its own and return the green node
/// for the single top-level node it produces (which must be of `expected_kind`),
/// with statement-relative diagnostics. Returns `None` unless the text parses to
/// exactly one expression consuming every token — the consume-all guard.
fn parse_stmt_in_isolation(
    stmt_text: &str,
    expected_kind: SyntaxKind,
) -> Option<(GreenNode, Vec<ParseDiagnostic>)> {
    let tokens = rebalance_brackets(lex(stmt_text));
    let mut diagnostics = Vec::new();
    let expr = parse_expr(&tokens, 0, 0, &mut diagnostics)?;
    if expr.start != 0 || expr.end != tokens.len() {
        return None;
    }

    let root = build_tree(&tokens, &expr.events);
    let mut children = root.children();
    let node = children.next()?;
    if children.next().is_some() || node.kind() != expected_kind {
        return None;
    }
    Some((node.green().into_owned(), diagnostics))
}

/// Whether `stmt_text` still ends at the same boundary once `forward_ctx` (the
/// text up to the end of the next top-level sibling) is appended. `parse_expr`
/// over the concatenation must consume a first expression that ends exactly at
/// `stmt_text.len()`; anything else means the edit made the statement merge
/// forward. Empty context (statement at end of file) is trivially stable.
fn toplevel_boundary_stable(stmt_text: &str, forward_ctx: &str) -> bool {
    if forward_ctx.is_empty() {
        return true;
    }
    let mut combined = String::with_capacity(stmt_text.len() + forward_ctx.len());
    combined.push_str(stmt_text);
    combined.push_str(forward_ctx);

    let tokens = rebalance_brackets(lex(&combined));
    let mut diagnostics = Vec::new();
    let Some(expr) = parse_expr(&tokens, 0, 0, &mut diagnostics) else {
        return false;
    };
    // Byte offset one past the last token the first expression consumed. A stable
    // boundary lands it exactly at the (unchanged) end of the statement text.
    expr.end
        .checked_sub(1)
        .and_then(|last| tokens.get(last))
        .is_some_and(|last| last.end == stmt_text.len())
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

    #[test]
    fn apply_edits_folds_each_against_prior_text() {
        // Two disjoint edits, each expressed against the text its predecessor
        // produced: prepend "ab", then insert "Z" before "ef" in the now-longer
        // text (index 4, not 2 — the prepend shifted it).
        let edits = [edit(0, 0, "ab"), edit(4, 4, "Z")];
        assert_eq!(apply_edits("cdef", &edits), "abcdZef");
    }

    #[test]
    fn apply_edits_empty_is_identity() {
        assert_eq!(apply_edits("xyz", &[]), "xyz");
    }
}
