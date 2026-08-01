//! Structural tests for the control-flow graph
//! ([`arity::semantic::FileControlFlow`]). Each case parses a small R program,
//! builds every region's CFG, and snapshots a textual dump of blocks + edges +
//! terminators. `always_diverges` is exercised directly for the reachability
//! predicate the linter consumes.

use arity::parser::parse;
use arity::semantic::FileControlFlow;
use arity::semantic::cfg::always_diverges;
use insta::assert_snapshot;
use rowan::ast::AstNode as _;

fn cfg_dump(src: &str) -> String {
    let parsed = parse(src);
    assert!(
        parsed.diagnostics.is_empty(),
        "parse: {:?}",
        parsed.diagnostics
    );
    FileControlFlow::build(&parsed.cst).render(src)
}

fn region_count(src: &str) -> usize {
    let parsed = parse(src);
    let cfg = FileControlFlow::build(&parsed.cst);
    1 + cfg.functions().len() // top-level + one per function
}

#[test]
fn sequential_statements() {
    assert_snapshot!(cfg_dump("a <- 1\nb <- 2\nf(a, b)\n"));
}

#[test]
fn if_without_else() {
    assert_snapshot!(cfg_dump("if (c) {\n  g()\n}\nh()\n"));
}

#[test]
fn if_with_else() {
    assert_snapshot!(cfg_dump("if (c) {\n  a()\n} else {\n  b()\n}\nafter()\n"));
}

#[test]
fn nested_if() {
    assert_snapshot!(cfg_dump(
        "if (c) {\n  if (d) a() else b()\n} else {\n  e()\n}\n"
    ));
}

#[test]
fn for_loop_with_break_and_next() {
    assert_snapshot!(cfg_dump(
        "for (i in xs) {\n  if (i > 1) break\n  if (i < 0) next\n  use(i)\n}\ndone()\n"
    ));
}

#[test]
fn while_loop() {
    assert_snapshot!(cfg_dump("while (cond) {\n  step()\n}\nafter()\n"));
}

#[test]
fn repeat_without_break_diverges() {
    assert_snapshot!(cfg_dump("repeat {\n  work()\n}\nunreached()\n"));
}

#[test]
fn repeat_with_break() {
    assert_snapshot!(cfg_dump(
        "repeat {\n  if (done) break\n  work()\n}\nafter()\n"
    ));
}

#[test]
fn return_then_unreachable_tail() {
    assert_snapshot!(cfg_dump("f <- function() {\n  return(1)\n  2\n}\n"));
}

#[test]
fn both_branches_return_makes_tail_unreachable() {
    assert_snapshot!(cfg_dump(
        "f <- function() {\n  if (c) return(1) else return(2)\n  3\n}\n"
    ));
}

#[test]
fn regions_are_toplevel_plus_each_function() {
    // Top-level, `outer`, and the nested `inner`.
    assert_eq!(
        region_count("outer <- function() {\n  inner <- function() 1\n  inner()\n}\n"),
        3
    );
    assert_eq!(region_count("x <- 1\n"), 1);
}

// --- always_diverges (the reachability predicate the linter consumes) --------

fn first<N: rowan::ast::AstNode<Language = arity::syntax::RLanguage>>(src: &str) -> N {
    let parsed = parse(src);
    parsed.cst.descendants().find_map(N::cast).expect("a node")
}

#[test]
fn always_diverges_direct_terminators() {
    let ret: arity::ast::CallExpr = first("return(1)");
    assert!(always_diverges(&ret.syntax().clone().into()));
    let stop: arity::ast::CallExpr = first("stop('x')");
    assert!(always_diverges(&stop.syntax().clone().into()));
}

#[test]
fn always_diverges_both_branches() {
    let both: arity::ast::IfExpr = first("if (c) return(1) else stop('x')");
    assert!(always_diverges(&both.syntax().clone().into()));

    // Only one arm diverges — falls through.
    let one: arity::ast::IfExpr = first("if (c) return(1) else 2");
    assert!(!always_diverges(&one.syntax().clone().into()));

    // No else — always falls through.
    let no_else: arity::ast::IfExpr = first("if (c) return(1)");
    assert!(!always_diverges(&no_else.syntax().clone().into()));
}
