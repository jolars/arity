use std::path::Path;

use arity::incremental::IncrementalDatabase;
use arity::parser::parse;
use arity::project::classes::{S3ClassEvidence, file_class_defs};
use arity::semantic::SemanticModel;
use arity::syntax::SyntaxElement;
use rowan::ast::AstNode as _;

fn evidence(source: &str) -> S3ClassEvidence {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{source}");
    assert_eq!(parsed.cst.text().to_string(), source);
    let root = parsed.cst;
    let model = SemanticModel::build(&root);
    let facts = file_class_defs(&model, &root);
    let path = Path::new("/s3.R");
    let index = IncrementalDatabase::default()
        .snapshot()
        .class_index_in(path, &facts, &root, &model);
    assert!(index.declarations.is_empty());
    assert!(index.supertypes.is_empty());
    let value = root
        .descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
        .find(|token| token.text() == "probe")
        .unwrap()
        .parent()
        .and_then(arity::ast::CallExpr::cast)
        .unwrap()
        .arg_list()
        .unwrap()
        .args()
        .next()
        .unwrap()
        .value()
        .unwrap();
    index.s3_class_evidence(path, &root, &model, &value)
}

fn explicit(classes: &[&str]) -> S3ClassEvidence {
    S3ClassEvidence::Explicit(classes.iter().map(|name| name.to_string()).collect())
}

#[test]
fn literal_class_evidence_preserves_order_and_duplicates() {
    assert_eq!(
        evidence("probe(structure(list(), class = c('child', 'parent', 'child')))"),
        explicit(&["child", "parent", "child"])
    );
    assert_eq!(
        evidence("probe(base::structure(list(), class = 'numeric'))"),
        explicit(&["numeric"])
    );
    assert_eq!(
        evidence("probe(structure(list(), class = 'first', class = 'last'))"),
        explicit(&["last"])
    );
}

#[test]
fn class_replacement_is_ordered_and_copies_do_not_mutate_together() {
    for setter in ["class", "oldClass"] {
        assert_eq!(
            evidence(&format!(
                "x <- list(); {setter}(x) <- c('child', 'parent'); probe(x)"
            )),
            explicit(&["child", "parent"])
        );
        assert_eq!(
            evidence(&format!(
                "x <- structure(list(), class = 'before'); y <- x; {setter}(x) <- 'after'; probe(y)"
            )),
            explicit(&["before"])
        );
        assert_eq!(
            evidence(&format!(
                "x <- structure(list(), class = 'before'); {setter}(x) <- NULL; probe(x)"
            )),
            S3ClassEvidence::NoExplicitClass
        );
    }
    assert_eq!(
        evidence("x <- 1; class(x) <- 'numeric'; probe(x)"),
        S3ClassEvidence::NoExplicitClass
    );
    assert_eq!(
        evidence("x <- 1; oldClass(x) <- 'numeric'; probe(x)"),
        explicit(&["numeric"])
    );
    assert_eq!(
        evidence("x <- list(); class(x) <- character(); probe(x)"),
        S3ClassEvidence::NoExplicitClass
    );
    assert_eq!(
        evidence("probe(base::`oldClass<-`(list(), 'literal'))"),
        explicit(&["literal"])
    );
}

#[test]
fn old_class_and_explicit_class_reads_can_supply_a_class_vector() {
    for getter in ["class", "oldClass"] {
        assert_eq!(
            evidence(&format!(
                "x <- structure(list(), class = c('child', 'parent')); probe(structure(list(), class = {getter}(x)))"
            )),
            explicit(&["child", "parent"])
        );
    }
    assert_eq!(
        evidence("x <- list(); probe(structure(list(), class = oldClass(x)))"),
        S3ClassEvidence::NoExplicitClass
    );
    assert_eq!(
        evidence("probe(structure(list(), class = class(runtime_value)))"),
        S3ClassEvidence::Unknown
    );
}

#[test]
fn constructors_recover_literal_tail_and_explicit_returns() {
    for body in [
        "structure(list(x = x), class = c('child', 'parent'))",
        "return(structure(list(x = x), class = c('child', 'parent')))\nlist()",
        "value <- list(x = x); class(value) <- c('child', 'parent'); value",
        "value <- list(x = x); oldClass(value) <- c('child', 'parent'); return(value)",
        "{ (return(structure(list(), class = c('child', 'parent')))) }; list()",
        "unused <- function() list(return(list())); structure(list(), class = c('child', 'parent'))",
    ] {
        assert_eq!(
            evidence(&format!(
                "new_value <- function(x) {{ {body} }}; alias <- new_value; probe(alias(1))"
            )),
            explicit(&["child", "parent"]),
            "{body}"
        );
    }
    assert_eq!(
        evidence(
            "new_value <- function() { return(); structure(list(), class = 'unreachable') }; probe(new_value())"
        ),
        S3ClassEvidence::NoExplicitClass
    );
}

#[test]
fn calls_propagate_possible_nonlocal_binding_changes() {
    for call in ["g()", "(g)()", "alias()", "do.call(g, list())", "wrapper()"] {
        let source = format!(
            "x <- structure(list(), class = 'before');
             g <- function() class(x) <<- 'after';
             alias <- g; wrapper <- function() g(); {call}; probe(x)"
        );
        assert_eq!(evidence(&source), S3ClassEvidence::Unknown, "{source}");
    }
    for source in [
        "x <- structure(list(), class = 'before'); unknown_call(); probe(x)",
        "x <- structure(list(), class = 'before'); wrapper <- function() unknown_call(); wrapper(); probe(x)",
        "g <- function() structure(list(), class = 'before'); change <- function() assign('g', function() structure(list(), class = 'after'), envir = parent.frame()); change(); wrapper <- function() g(); probe(wrapper())",
        "x <- structure(list(), class = 'before'); if (flag) unknown_call(); probe(x)",
        "new_value <- function() { x <- structure(list(), class = 'before'); g <- function() class(x) <<- 'after'; wrapper <- function() g(); wrapper(); x }; probe(new_value())",
    ] {
        assert_eq!(evidence(source), S3ClassEvidence::Unknown, "{source}");
    }
    assert_eq!(
        evidence(
            "x <- structure(list(), class = 'before'); g <- function() list(); wrapper <- function() g(); wrapper(); probe(x)"
        ),
        explicit(&["before"])
    );
}

#[test]
fn constructors_with_embedded_returns_remain_unknown() {
    for expression in [
        "flag && return(structure(list(), class = 'early'))",
        "flag || return(structure(list(), class = 'early'))",
        "return(structure(list(), class = 'early')) + 1",
        "-return(structure(list(), class = 'early'))",
        "list()[return(structure(list(), class = 'early'))]",
        "list(return(structure(list(), class = 'early')))",
    ] {
        let source = format!(
            "new_value <- function(flag) {{ {expression}; structure(list(), class = 'late') }}; probe(new_value(TRUE))"
        );
        assert_eq!(evidence(&source), S3ClassEvidence::Unknown, "{source}");
        let root = parse(&source).cst;
        assert!(
            file_class_defs(&SemanticModel::build(&root), &root)
                .s3
                .constructors
                .is_empty(),
            "{source}"
        );
    }
}

#[test]
fn constructor_opacity_follows_control_flow_aliases() {
    for origin in ["return", "base::return"] {
        for expression in [
            "list(ret(structure(list(), class = 'early')))",
            "c(ret(structure(list(), class = 'early')))",
            "flag && ret(structure(list(), class = 'early'))",
        ] {
            let source = format!(
                "ret <- {origin}; new_value <- function(flag) {{ {expression}; structure(list(), class = 'late') }}; probe(new_value(TRUE))"
            );
            assert_eq!(evidence(&source), S3ClassEvidence::Unknown, "{source}");
        }
    }
    assert_eq!(
        evidence(
            "dispatch <- UseMethod; new_value <- function(x) { dispatch('generic'); structure(list(), class = 'late') }; generic.early <- function(x) structure(list(), class = 'early'); probe(new_value(structure(list(), class = 'early')))"
        ),
        S3ClassEvidence::Unknown
    );
    assert_eq!(
        evidence(
            "ret <- return; new_value <- function() { ret(structure(list(), class = 'early')); structure(list(), class = 'late') }; probe(new_value())"
        ),
        explicit(&["early"])
    );
}

#[test]
fn depth_limits_invalidate_skipped_writes() {
    for depth in [32, 40] {
        for mutation in ["class(x) <- 'after'", "unknown_call()"] {
            for nested in [false, true] {
                let probe = if nested { "; probe(x)" } else { "" };
                let after = if nested { "" } else { "; probe(x)" };
                let source = format!(
                    "x <- structure(list(), class = 'before'); {}{mutation}{probe}{}{after}",
                    "{".repeat(depth),
                    "}".repeat(depth)
                );
                assert_eq!(evidence(&source), S3ClassEvidence::Unknown, "{source}");
            }
        }
    }
    let mut source = String::from(
        "x <- structure(list(), class = 'before'); g0 <- function() class(x) <<- 'after';",
    );
    for depth in 1..=16 {
        source.push_str(&format!("g{depth} <- function() g{}();", depth - 1));
    }
    source.push_str("g16(); probe(x)");
    assert_eq!(evidence(&source), S3ClassEvidence::Unknown, "{source}");
}

#[test]
fn unknown_classes_and_dotted_methods_do_not_establish_classes() {
    for source in [
        "probe(runtime_value)",
        "probe(structure(list(), class = runtime_class))",
        "x <- structure(list(), class = 'before'); class(x) <- runtime_class; probe(x)",
        "x <- structure(list(), class = 'before'); if (flag) class(x) <- 'after'; probe(x)",
        "new_value <- function(x) { if (x) return(list()); structure(list(), class = 'maybe') }; probe(new_value(TRUE))",
        "new_value <- function(x) structure(x, class = x); probe(new_value('computed'))",
        "print.phantom <- function(x) x; probe(phantom())",
        "structure <- function(...) runtime_value; probe(structure(list(), class = 'fake'))",
        "c <- function(...) 'fake'; probe(structure(list(), class = c('child', 'parent')))",
        "`class<-` <- function(x, value) x; x <- runtime_value; class(x) <- 'fake'; probe(x)",
        "x <- structure(runtime_value, class = 'before'); y <- x; class(x) <- 'after'; probe(y)",
        "x <- structure(list(), class = 'before'); attr(x, 'class') <- 'after'; probe(x)",
        "x <- structure(list(), class = 'before'); assign('x', runtime_value); probe(x)",
        "x <- structure(list(), class = 'before'); list(class(x) <- 'after', probe(x))",
        "x <- structure(list(), class = 'before'); for (i in 1:2) class(x) <- 'after'; probe(x)",
        "new_value <- function() { value <- structure(list(), class = 'before'); on.exit(class(value) <- 'after'); value }; probe(new_value())",
        "probe(base::`oldClass<-`(value = 'class', x = list()))",
        "x <- structure(list(), class = 'before'); y <- structure(list(), ignored = { class(x) <- 'after' }); probe(x)",
        "x <- structure(runtime_value, class = 'before'); y <- structure(x, class = 'after'); probe(x)",
        "new_value <- function() { list(return(structure(list(), class = 'early'))); structure(list(), class = 'late') }; probe(new_value())",
        "x <- structure(runtime_value, class = 'before'); opaque_mutator(x); probe(x)",
        "x <- structure(runtime_value, class = 'before'); y <- x; if (flag) class(x) <- 'after'; probe(y)",
        "x <- structure(runtime_value, class = 'before'); y <- x; if (flag) opaque_mutator(x); probe(y)",
    ] {
        assert_eq!(evidence(source), S3ClassEvidence::Unknown, "{source}");
    }
}
