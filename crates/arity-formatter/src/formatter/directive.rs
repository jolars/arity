//! Where the formatter has been told to stand down.
//!
//! `# arity-format skip`, `# arity-format off` … `on`, and `# arity-format
//! skip-file` (and the `# arity` forms, which address both tools) mark source
//! the layout engine must not touch. The marked text is spliced back byte for
//! byte — original column, interior alignment, blank-line runs and all. That is
//! the only reading consistent with Tenet 1: the formatter was told not to
//! decide layout here, so it decides nothing, not even the indent.
//!
//! # Statement positions only
//!
//! A directive is honored where the *statement lists* are — the top level and a
//! block body, which are exactly the lists [`super::trivia::split_lines`]
//! builds. Elsewhere (inside a call's argument list, between the operands of a
//! binary chain) it is inert, and the linter's `misplaced-suppression` reports
//! it rather than letting it fail silently. [`is_honored_position`] is the one
//! predicate both sides ask, so the report and the behavior cannot drift.
//!
//! # Regions are list-local
//!
//! An `off` runs to the matching `on` or to the end of *its own* statement list.
//! It cannot leak past a closing brace, because a skipped span is spliced by the
//! sequencer that owns those lines. Nothing is lost: an `off` at the top level
//! swallows a whole statement — block and all — before that block's own
//! sequencer ever runs.

use rowan::{NodeOrToken, SyntaxElement, TextRange, TextSize};

use arity_parser::directive::{Parsed, Verb, parse};

use super::ir::Ir;
use super::trivia::is_trivia;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxToken};

/// Whether a directive written on `token` is one the formatter acts on.
///
/// True exactly where the statement lists are: a comment that is a direct child
/// of the root or of a block. Exported because the linter reports the directives
/// that land anywhere else, and that report has to mean what the engine does.
pub fn is_honored_position(token: &SyntaxToken) -> bool {
    token
        .parent()
        .is_some_and(|parent| matches!(parent.kind(), SyntaxKind::ROOT | SyntaxKind::BLOCK_EXPR))
}

/// Whether a comment reads as a `skip-file` addressed to the formatter.
///
/// The R grammar asks this once per file, before any formatting, and the answer
/// is to hand the source back untouched. That whole-file walk lives in
/// [`super::core`]'s single token prepass; this is the predicate it applies.
pub(super) fn is_skip_file(text: &str) -> bool {
    matches!(parse(text), Some(Parsed::Directive(d))
        if d.tool.affects_format() && d.verb == Verb::SkipFile)
}

/// Whether a `DESCRIPTION` is `# arity-format skip-file`.
///
/// The other verbs are not honored in DCF: a field's lines are laid out by its
/// class, not sequenced, so there is nothing to splice a span into yet.
pub(super) fn dcf_file_is_skipped(root: &crate::dcf::SyntaxNode) -> bool {
    root.descendants_with_tokens().any(|element| {
        let NodeOrToken::Token(token) = element else {
            return false;
        };
        token.kind() == crate::dcf::SyntaxKind::COMMENT && is_skip_file(token.text())
    })
}

/// The lines of one statement list that must be spliced verbatim.
///
/// Indexed by the line that *starts* a skipped run: the value is the last line
/// the run covers and the source range to splice. Lines strictly inside a run
/// carry no entry — the sequencer jumps over them.
#[derive(Debug, Default)]
pub(super) struct SkipPlan {
    runs: Vec<(usize, usize, TextRange)>,
}

impl SkipPlan {
    /// The run starting at `line`, as `(last line covered, source range)`.
    pub(super) fn run_at(&self, line: usize) -> Option<(usize, TextRange)> {
        self.runs
            .iter()
            .find(|(start, _, _)| *start == line)
            .map(|(_, end, range)| (*end, *range))
    }
}

/// Read the directives of one statement list.
///
/// `lines` is the list as [`super::trivia::split_lines`] produced it, so a
/// directive's target is "the next line that carries code" — the line-wise
/// reading of the linter's next-non-trivia-sibling rule, which in a statement
/// list is the same node.
pub(super) fn plan(lines: &[Vec<SyntaxElement<RLanguage>>]) -> SkipPlan {
    let mut plan = SkipPlan::default();
    let mut pending_skip = false;
    let mut region_start: Option<usize> = None;

    for (idx, line) in lines.iter().enumerate() {
        match line_directive(line) {
            Some(Verb::Skip) => {
                pending_skip = true;
                continue;
            }
            Some(Verb::Off) => {
                region_start.get_or_insert(idx + 1);
                continue;
            }
            Some(Verb::On) => {
                if let Some(start) = region_start.take() {
                    push_run(&mut plan, lines, start, idx.saturating_sub(1));
                }
                continue;
            }
            // A file-scope directive is answered before formatting starts, and
            // an unparseable one is the linter's to report.
            Some(Verb::SkipFile) | None => {}
        }

        if region_start.is_some() || is_blank(line) || is_comment_only(line) {
            // Blank and comment lines never consume a pending skip: the target
            // is the next line that carries code, as in the linter.
            continue;
        }
        if pending_skip {
            pending_skip = false;
            push_run(&mut plan, lines, idx, idx);
        }
    }

    // An `off` that is never closed runs to the end of its own list.
    if let Some(start) = region_start {
        push_run(&mut plan, lines, start, lines.len().saturating_sub(1));
    }
    plan
}

/// Record the run covering `start..=end`, trimmed to the lines that carry
/// something. Trailing and leading blanks stay outside so the sequencer's own
/// separators still decide the gaps around the run.
fn push_run(
    plan: &mut SkipPlan,
    lines: &[Vec<SyntaxElement<RLanguage>>],
    start: usize,
    end: usize,
) {
    let significant = |idx: usize| lines.get(idx).is_some_and(|line| !is_blank(line));
    let Some(first) = (start..=end).find(|&idx| significant(idx)) else {
        return;
    };
    let last = (first..=end)
        .rfind(|&idx| significant(idx))
        .unwrap_or(first);
    let (Some(from), Some(to)) = (line_start(&lines[first]), line_end(&lines[last])) else {
        return;
    };
    plan.runs.push((first, last, TextRange::new(from, to)));
}

/// The verb of a directive written on a line of its own, if the formatter is
/// addressed by it. A directive trailing a statement is *not* one: it would
/// have to be sliced out of the very text it marks.
fn line_directive(line: &[SyntaxElement<RLanguage>]) -> Option<Verb> {
    let elements = significant(line);
    let [NodeOrToken::Token(token)] = elements.as_slice() else {
        return None;
    };
    if token.kind() != SyntaxKind::COMMENT {
        return None;
    }
    match parse(token.text())? {
        Parsed::Directive(d) if d.tool.affects_format() => Some(d.verb),
        _ => None,
    }
}

/// Where the first line of a skipped run begins: at its indentation, not at its
/// first token, so the author's own column is part of what gets spliced.
fn line_start(line: &[SyntaxElement<RLanguage>]) -> Option<TextSize> {
    let first = significant(line).into_iter().next()?;
    let start = match first.prev_sibling_or_token() {
        Some(NodeOrToken::Token(ws)) if ws.kind() == SyntaxKind::WHITESPACE => {
            ws.text_range().start()
        }
        _ => first.text_range().start(),
    };
    Some(start)
}

fn line_end(line: &[SyntaxElement<RLanguage>]) -> Option<TextSize> {
    significant(line).last().map(|el| el.text_range().end())
}

fn significant(line: &[SyntaxElement<RLanguage>]) -> Vec<SyntaxElement<RLanguage>> {
    line.iter()
        .filter(|el| !is_trivia(el.kind()))
        .cloned()
        .collect()
}

fn is_blank(line: &[SyntaxElement<RLanguage>]) -> bool {
    significant(line).is_empty()
}

/// A line carrying nothing but `#` comments. A `#'` block is a node, not a
/// comment token, so it counts as code — which matches the linter, where a
/// directive attaches to a roxygen block rather than reaching past it.
fn is_comment_only(line: &[SyntaxElement<RLanguage>]) -> bool {
    let significant = significant(line);
    !significant.is_empty()
        && significant
            .iter()
            .all(|el| el.kind() == SyntaxKind::COMMENT)
}

/// The IR for a skipped run starting at `idx`, and the last line it covers.
///
/// `None` when nothing is skipped there, which is every line in the common case.
pub(super) fn skipped_at(
    lines: &[Vec<SyntaxElement<RLanguage>>],
    plan: &SkipPlan,
    idx: usize,
) -> Option<(Ir, usize)> {
    let (last, range) = plan.run_at(idx)?;
    let anchor = significant(lines.get(idx)?).into_iter().next()?;
    Some((Ir::skipped(source_text(&anchor, range)), last))
}

/// The text of `range`, read back out of the tree it was parsed from.
///
/// `SyntaxText::slice` takes offsets relative to the node's own start, which is
/// why this climbs to the root first: the root starts at zero, so a range that
/// is absolute in the file is also relative to it.
fn source_text(anchor: &SyntaxElement<RLanguage>, range: TextRange) -> String {
    let root = match anchor {
        NodeOrToken::Node(node) => node.ancestors().last(),
        NodeOrToken::Token(token) => token.parent_ancestors().last(),
    };
    root.map(|root| root.text().slice(range).to_string())
        .unwrap_or_default()
}
