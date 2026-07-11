//! A file's top-level *execution* sequence: the ordered list of `define` /
//! `source()` / `bare read` events as R would run them, top to bottom.
//!
//! R executes a script's top-level statements sequentially, so *position*
//! matters for resolution: a bare read before a `source()` can't see the
//! bindings that call injects, and a local def and a sourced def of the same
//! name shadow by order. [`crate::project::scope`] consumes this to resolve
//! reads through load order instead of treating `source()` as position-blind.
//!
//! The sequence is deliberately **range-free** — order lives in the `Vec`
//! position, never in a span — so a function-body edit re-extracts to an equal
//! value and the salsa firewall backdates, the same posture as
//! [`crate::project::source::SourceEdgeKey`]. Only *top-level* (file-scope)
//! defines and reads are recorded; a read inside a function body runs at call
//! time and sees the final post-execution scope, so it is not position-gated.

use std::collections::HashMap;
use std::path::Path;

use rowan::{NodeOrToken, TextRange};

use crate::project::source::{TopLevelEvent, top_level_source_edge_key};
use crate::semantic::{Binding, BindingKind, IdentRef, ScopeKind, SemanticModel};
use crate::syntax::{SyntaxKind, SyntaxNode};

/// Extract the top-level event sequence of the file rooted at `root`, in
/// document (= execution) order. `base_dir` resolves relative `source()`
/// targets; `model` supplies the file-scope binding/read classification (the
/// same predicates [`crate::project::file_exports`]/`file_free_reads` use).
///
/// Each top-level statement contributes, in order:
/// - a [`TopLevelEvent::SourceEdge`] if it is a `source()`/`sys.source()` call
///   (its argument reads are not separately recorded — the edge *is* the event);
/// - otherwise its file-scope-direct free reads (sorted by offset), then its
///   file-scope definitions. Reads precede the define of the same statement
///   because the right-hand side evaluates before the binding becomes live
///   (`x <- g(y)` reads `g`, `y`, then defines `x`).
pub fn collect_top_level_events(
    root: &SyntaxNode,
    base_dir: Option<&Path>,
    model: &SemanticModel,
) -> Vec<TopLevelEvent> {
    collect_top_level_events_spanned(root, base_dir, model)
        .into_iter()
        .map(|(event, _span)| event)
        .collect()
}

/// The same sequence as [`collect_top_level_events`], but each event is paired
/// with the span of the identifier that produced it: `Some` for a
/// [`TopLevelEvent::Read`] (the read occurrence's range), `None` for a
/// `Define`/`SourceEdge`. Stripping the spans yields exactly
/// [`collect_top_level_events`]'s range-free output (which is what backs the
/// salsa firewall) — the two stay in lockstep by construction.
///
/// Computed ad hoc at refactor time off a fresh tree+model (never a salsa
/// query): order-aware rename uses the spans to correlate each top-level read to
/// the binding it resolves to under load order. See
/// [`crate::project::ProjectScope::top_level_read_provenance`].
pub fn collect_top_level_events_spanned(
    root: &SyntaxNode,
    base_dir: Option<&Path>,
    model: &SemanticModel,
) -> Vec<(TopLevelEvent, Option<TextRange>)> {
    // Top-level statement nodes, in document order. Their ranges are disjoint
    // and sorted, so one binary search assigns any model range to its covering
    // statement — bucketing the whole model in a single pass each for idents
    // and bindings, instead of rescanning both per statement (which is
    // quadratic in file size).
    let stmt_ranges: Vec<TextRange> = root.children().map(|n| n.text_range()).collect();
    let covering = |range: TextRange| -> Option<usize> {
        let idx = stmt_ranges.partition_point(|r| r.start() <= range.start());
        (idx > 0 && stmt_ranges[idx - 1].contains_range(range)).then(|| idx - 1)
    };

    // File-scope-direct idents bucketed by covering statement. A read inside a
    // function/block body has a non-`File` scope and is skipped (it runs at
    // call time, against the final post-execution scope). An ident covered by
    // no statement is a *bare* top-level identifier — a direct IDENT token of
    // the root, not wrapped in a node — kept aside keyed by its exact span.
    let mut reads_by_stmt: Vec<Vec<&IdentRef>> = vec![Vec::new(); stmt_ranges.len()];
    let mut bare_reads: HashMap<TextRange, &IdentRef> = HashMap::new();
    for ident in model.idents() {
        match covering(ident.range) {
            Some(i) if model.scope(ident.scope).kind == ScopeKind::File => {
                reads_by_stmt[i].push(ident);
            }
            Some(_) => {}
            None => {
                bare_reads.insert(ident.range, ident);
            }
        }
    }

    // File-scope definitions bucketed the same way, preserving the model's
    // binding order within each statement.
    let mut defines_by_stmt: Vec<Vec<&Binding>> = vec![Vec::new(); stmt_ranges.len()];
    for binding in model.bindings() {
        if matches!(binding.kind, BindingKind::Local | BindingKind::Implicit)
            && model.scope(binding.scope).kind == ScopeKind::File
            && let Some(i) = covering(binding.def_range)
        {
            defines_by_stmt[i].push(binding);
        }
    }

    let mut events: Vec<(TopLevelEvent, Option<TextRange>)> = Vec::new();
    // Iterate *elements* (nodes and tokens) so bare top-level identifiers are
    // also seen as reads, in document order alongside the statements.
    let mut stmt_idx = 0usize;
    for element in root.children_with_tokens() {
        match element {
            NodeOrToken::Token(tok) if tok.kind() == SyntaxKind::IDENT => {
                // A bare top-level identifier is a file-scope free read iff the
                // model recorded it as an unresolved read at that exact span.
                if let Some(ident) = bare_reads.get(&tok.text_range())
                    && model.resolve_local(ident).is_none()
                {
                    events.push((
                        TopLevelEvent::Read(ident.name.to_string()),
                        Some(ident.range),
                    ));
                }
            }
            NodeOrToken::Node(child) => {
                let i = stmt_idx;
                stmt_idx += 1;
                // A `source()`/`sys.source()` call is one edge event; don't
                // also emit reads for its arguments.
                if let Some(key) = top_level_source_edge_key(&child, base_dir) {
                    events.push((TopLevelEvent::SourceEdge(key), None));
                    continue;
                }
                // The statement's free reads, in source order, then its
                // definitions (after the reads: the right-hand side evaluates
                // before the binding becomes live).
                let mut reads = std::mem::take(&mut reads_by_stmt[i]);
                reads.sort_by_key(|ident| ident.range.start());
                events.extend(
                    reads
                        .into_iter()
                        .filter(|ident| model.resolve_local(ident).is_none())
                        .map(|ident| {
                            (
                                TopLevelEvent::Read(ident.name.to_string()),
                                Some(ident.range),
                            )
                        }),
                );
                events.extend(
                    defines_by_stmt[i]
                        .iter()
                        .map(|binding| (TopLevelEvent::Define(binding.name.to_string()), None)),
                );
            }
            _ => {}
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::project::source::{SourceTarget, TopLevelEvent};

    fn events(src: &str) -> Vec<TopLevelEvent> {
        let cst = parse(src).cst;
        let model = SemanticModel::build(&cst);
        collect_top_level_events(&cst, None, &model)
    }

    fn define(name: &str) -> TopLevelEvent {
        TopLevelEvent::Define(name.to_string())
    }
    fn read(name: &str) -> TopLevelEvent {
        TopLevelEvent::Read(name.to_string())
    }

    #[test]
    fn read_before_source_orders_before_the_edge() {
        let e = events("before\nsource(\"a.R\")\n");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], read("before"));
        assert!(matches!(e[1], TopLevelEvent::SourceEdge(_)));
    }

    #[test]
    fn read_after_source_orders_after_the_edge() {
        let e = events("source(\"a.R\")\nafter\n");
        assert_eq!(e.len(), 2);
        assert!(matches!(e[0], TopLevelEvent::SourceEdge(_)));
        assert_eq!(e[1], read("after"));
    }

    #[test]
    fn body_reads_are_excluded() {
        // `bar` is read inside a function body (call-time), so it is not a
        // top-level event; only the top-level def `f` is.
        let e = events("f <- function() bar\n");
        assert_eq!(e, vec![define("f")]);
    }

    #[test]
    fn reads_precede_the_define_of_their_statement() {
        let e = events("x <- g(y)\n");
        assert_eq!(e, vec![read("g"), read("y"), define("x")]);
    }

    #[test]
    fn mixed_statement_shapes_keep_document_order() {
        // Bare top-level reads, ordinary statements, source edges, and a
        // multi-define statement, interleaved: events must follow document
        // order, with each statement's reads before its defines and the
        // edge statement contributing nothing but the edge itself.
        let src = "\
top1
x <- g(y)
source(paste0(prefix, \"a.R\"))
a <- b <- h(z)
top2
";
        let e = events(src);
        let mut it = e.into_iter();
        assert_eq!(it.next(), Some(read("top1")));
        assert_eq!(it.next(), Some(read("g")));
        assert_eq!(it.next(), Some(read("y")));
        assert_eq!(it.next(), Some(define("x")));
        assert!(matches!(it.next(), Some(TopLevelEvent::SourceEdge(_))));
        assert_eq!(it.next(), Some(read("h")));
        assert_eq!(it.next(), Some(read("z")));
        // Defines follow the model's binding order (innermost assignment
        // first for a chained `a <- b <- ...`), pinned here as-is.
        assert_eq!(it.next(), Some(define("b")));
        assert_eq!(it.next(), Some(define("a")));
        assert_eq!(it.next(), Some(read("top2")));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn sys_source_is_a_dynamic_edge() {
        let e = events("sys.source(\"a.R\")\n");
        assert_eq!(e.len(), 1);
        match &e[0] {
            TopLevelEvent::SourceEdge(key) => {
                assert_eq!(key.target, SourceTarget::Dynamic);
            }
            other => panic!("expected a dynamic source edge, got {other:?}"),
        }
    }

    #[test]
    fn local_true_edge_keeps_its_local_flag() {
        let e = events("source(\"a.R\", local = TRUE)\n");
        match &e[0] {
            TopLevelEvent::SourceEdge(key) => assert!(key.local),
            other => panic!("expected a source edge, got {other:?}"),
        }
    }

    #[test]
    fn body_edit_leaves_the_sequence_unchanged() {
        // The two sources differ only inside a function body; the top-level event
        // sequence must be byte-identical (the firewall/backdate precondition).
        let a = events("g <- function() 1\nsource(\"x.R\")\nbar\n");
        let b = events("g <- function() 1 + 2 + 3\nsource(\"x.R\")\nbar\n");
        assert_eq!(a, b);
    }

    fn spanned(src: &str) -> Vec<(TopLevelEvent, Option<rowan::TextRange>)> {
        let cst = parse(src).cst;
        let model = SemanticModel::build(&cst);
        collect_top_level_events_spanned(&cst, None, &model)
    }

    #[test]
    fn stripping_spans_yields_the_range_free_sequence() {
        // The spanned collector is the source of truth; the range-free one is its
        // span-stripped projection. They must agree event-for-event.
        let src = "x <- g(y)\nsource(\"a.R\")\nbar\n";
        let stripped: Vec<TopLevelEvent> = spanned(src).into_iter().map(|(e, _)| e).collect();
        assert_eq!(stripped, events(src));
    }

    #[test]
    fn only_reads_carry_a_span() {
        // Reads anchor to the identifier occurrence; defines and source edges are
        // position-free (`None`).
        for (event, span) in spanned("x <- g(y)\nsource(\"a.R\")\n") {
            match event {
                TopLevelEvent::Read(_) => assert!(span.is_some(), "a read carries its span"),
                TopLevelEvent::Define(_) | TopLevelEvent::SourceEdge(_) => {
                    assert!(span.is_none(), "a define/edge has no span")
                }
            }
        }
    }

    #[test]
    fn read_span_indexes_the_identifier() {
        // The recovered span must cover exactly the read's text.
        let src = "x <- foo\n";
        let (event, span) = spanned(src)
            .into_iter()
            .find(|(e, _)| matches!(e, TopLevelEvent::Read(n) if n == "foo"))
            .expect("a top-level read of foo");
        assert_eq!(event, read("foo"));
        let span = span.expect("a read span");
        assert_eq!(&src[span], "foo");
    }

    #[test]
    fn body_edit_leaves_the_spanned_sequence_events_unchanged() {
        // A function-body edit shifts spans but must not change the *events* — the
        // span-stripped projection (the firewall input) stays byte-identical.
        let strip =
            |s: &str| -> Vec<TopLevelEvent> { spanned(s).into_iter().map(|(e, _)| e).collect() };
        let a = strip("g <- function() 1\nsource(\"x.R\")\nbar\n");
        let b = strip("g <- function() 1 + 2 + 3\nsource(\"x.R\")\nbar\n");
        assert_eq!(a, b);
    }
}
