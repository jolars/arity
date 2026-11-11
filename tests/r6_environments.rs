use std::path::Path;

use arity::config::LintConfig;
use arity::linter::check_document;
use arity::lsp::{
    compute_definition, compute_document_highlights, compute_prepare_rename, compute_references,
    compute_rename,
};
use arity::text::PositionEncoding;

fn findings(source: &str, rule: &str) -> Vec<String> {
    let parsed = arity::parser::parse(source);
    assert!(
        parsed.diagnostics.is_empty(),
        "{source}: {:?}",
        parsed.diagnostics
    );
    assert_eq!(arity::parser::reconstruct(source), source);
    let config = LintConfig {
        select: Some(vec![rule.into()]),
        ..Default::default()
    };
    check_document(Path::new("test.R"), source, &config)
        .unwrap()
        .iter()
        .map(|diagnostic| {
            source[usize::from(diagnostic.range.start())..usize::from(diagnostic.range.end())]
                .to_string()
        })
        .collect()
}

#[test]
fn r6_method_environments_cover_methods_active_bindings_defaults_and_closures() {
    for portable in ["TRUE", "FALSE"] {
        let source = format!(
            r#"
Base <- R6::R6Class("Base", portable = {portable}, private = list(value = 1))
Child <- R6::R6Class("Child", portable = {portable}, inherit = Base,
    public = list(
        initialize = function(value = private$value) self$value <- value,
        read = function(value = self$value) {{
            nested <- function() list(self, private, super, value)
            nested()
        }},
        construct = function() {{ self$fn <- function() private$value; self$fn }}),
    private = list(private_read = function() list(self, private, super)),
    active = list(value = function(value) {{
        if (missing(value)) private$value else private$value <- value
    }}))
print(Child)
"#
        );
        assert_eq!(findings(&source, "undefined-symbol"), Vec::<String>::new());
    }
    let source = include_str!("fixtures/oop/r6/issue_183.R");
    assert!(findings(source, "undefined-symbol").is_empty());
}

#[test]
fn r6_bindings_are_limited_to_the_actual_method_enclosure() {
    let source = r#"
A <- R6::R6Class("A", public = list(
    value = self,
    boxed = list(fn = function() self),
    read = function() list(self, private, super, unrelated)))
ordinary <- function() list(self, private, super)
print(list(A, ordinary))
"#;
    assert_eq!(
        findings(source, "undefined-symbol"),
        [
            "self",
            "self",
            "private",
            "super",
            "unrelated",
            "self",
            "private",
            "super"
        ]
    );
    for source in [
        "A <- R6::R6Class(public = list(read = function() private), private = list())\n",
        "A <- R6::R6Class(public = list(read = function() super), inherit = NULL)\n",
        "R6Class <- function(...) list(...)\nA <- R6Class(public = list(read = function() self))\n",
    ] {
        assert_eq!(findings(source, "undefined-symbol").len(), 1, "{source}");
    }
}

#[test]
fn r6_local_shadowing_and_lazy_defaults_preserve_def_use() {
    let source = r#"
self <- 1
private <- 2
super <- 3
Parent <- R6::R6Class()
A <- R6::R6Class(inherit = Parent, private = list(x = 1), public = list(
    read = function(private, x = self) {
        before <- self
        self <- 4
        nested <- function() self
        list(before, self, private, nested(), x, super)
    }))
print(A)
"#;
    assert!(findings(source, "undefined-symbol").is_empty());
    assert_eq!(
        findings(source, "unused-binding"),
        ["self", "private", "super"]
    );

    let source =
        "A <- R6::R6Class(public = list(read = function(x = self) { self <- 2; x }))\nprint(A)\n";
    assert!(findings(source, "unused-binding").is_empty());
}

#[test]
fn r6_missing_generated_names_fall_back_to_the_lexical_owner() {
    let source = "private <- 1\nsuper <- 2\nA <- R6::R6Class(public = list(read = function() list(self, private, super)))\nprint(A)\n";
    assert!(findings(source, "undefined-symbol").is_empty());
    assert!(findings(source, "unused-binding").is_empty());
    let offset = source.rfind("private, super").unwrap();
    assert_eq!(
        compute_definition(source, offset).unwrap().start(),
        0.into()
    );
}

#[test]
fn r6_unknown_private_members_and_inheritance_remain_conservative() {
    for source in [
        "make <- function(parent) R6::R6Class(inherit = parent, public = list(read = function() list(self, private, super)))\n",
        "make <- function(members) R6::R6Class(private = members, public = list(read = function() private))\n",
        "A <- R6::R6Class(inherit = B, public = list(read = function() private))\nB <- R6::R6Class(inherit = A)\n",
        "make <- function(env) R6::R6Class(parent_env = env, public = list(read = function() self))\n",
    ] {
        assert!(findings(source, "undefined-symbol").is_empty(), "{source}");
    }
}

#[test]
fn r6_resolvable_implementation_aliases_keep_alternative_environments() {
    let source = r#"
self <- 1
helper <- function() self
alias <- helper
A <- R6::R6Class(public = list(read = alias))
B <- R6::R6Class(public = list(read = helper))
print(list(A, B, helper()))
"#;
    assert!(findings(source, "undefined-symbol").is_empty());
    assert!(findings(source, "unused-binding").is_empty());
    let offset = source.find("function() self").unwrap() + "function() ".len();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_rename(source, 0, "other", PositionEncoding::Utf16).is_none());
}

#[test]
fn r6_generated_names_have_references_without_editable_definitions() {
    let source = r#"
self <- 1
A <- R6::R6Class(public = list(
    read = function() self,
    nested = function() function() self,
    shadow = function(self) self))
B <- R6::R6Class(public = list(read = function() self))
print(list(A, B))
"#;
    let offset = source.find("function() self").unwrap() + "function() ".len();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
    assert!(compute_rename(source, offset, "other", PositionEncoding::Utf16).is_none());
    let references = compute_references(source, offset, true).unwrap();
    assert_eq!(references.len(), 2);
    let highlights = compute_document_highlights(source, offset).unwrap();
    assert_eq!(highlights.len(), 2);
    assert!(
        highlights
            .iter()
            .all(|(_, kind)| *kind == lsp_types::DocumentHighlightKind::READ)
    );
    let shadow = source.find("function(self) self").unwrap() + "function(self) ".len();
    let definition = compute_definition(source, shadow).unwrap();
    assert_eq!(usize::from(definition.start()), shadow - "self) ".len());
    assert_eq!(compute_references(source, 1, true).unwrap().len(), 1);
}

#[test]
fn r6_conditional_shadowing_cannot_rewrite_a_possible_generated_read() {
    let source =
        "A <- R6::R6Class(public = list(read = function(flag) { if (flag) self <- 1; self }))\n";
    let offset = source.find("self <-").unwrap();
    assert!(compute_rename(source, offset, "other", PositionEncoding::Utf16).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
    assert!(findings(source, "undefined-symbol").is_empty());
}

#[test]
fn r6_inherited_methods_can_acquire_a_subclass_private_environment() {
    let source = "Base <- R6::R6Class(public = list(read = function() private))\nChild <- R6::R6Class(inherit = Base, private = list(value = 1))\nprint(Child$new()$read())\n";
    assert!(findings(source, "undefined-symbol").is_empty());
}

#[test]
fn r6_cold_navigation_does_not_guess_an_unconfirmed_factory_origin() {
    let source = "self <- 1\nA <- R6Class(public = list(read = function() self))\n";
    let offset = source.rfind("self").unwrap();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
    assert!(compute_rename(source, 0, "other", PositionEncoding::Utf16).is_none());
}

#[test]
fn r6_deferred_reads_keep_the_environment_before_a_future_local_write() {
    for source in [
        "A <- R6::R6Class(public = list(read = function(x = self) { print(x); self <- 1 }))\n",
        "A <- R6::R6Class(public = list(read = function() { f <- function() self; f(); self <- 1 }))\n",
        "A <- R6::R6Class(public = list(read = function(values) { for (self in values) print(self); self }))\n",
    ] {
        let offset = source
            .rfind("self <-")
            .or_else(|| source.find("self in"))
            .unwrap();
        assert!(
            compute_rename(source, offset, "other", PositionEncoding::Utf16).is_none(),
            "{source}"
        );
    }
}

#[test]
fn r6_assignment_rhs_uses_the_environment_before_the_local_write() {
    let source = "A <- R6::R6Class(public = list(read = function() { self <- self; self }))\n";
    let lhs = source.find("self <-").unwrap();
    let rhs = lhs + "self <- ".len();
    assert!(compute_definition(source, rhs).is_none());
    let edits = compute_rename(source, lhs, "other", PositionEncoding::Utf16).unwrap();
    let starts: Vec<_> = edits
        .iter()
        .map(|edit| edit.range.start.character as usize)
        .collect();
    assert_eq!(starts, [lhs, source.rfind("self").unwrap()]);
    let recursive = "A <- R6::R6Class(public = list(read = function() A$new()))\n";
    assert_eq!(
        compute_rename(recursive, 0, "Other", PositionEncoding::Utf16)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn r6_custom_parents_preserve_possible_lexical_uses_without_navigation_proofs() {
    let source = "value <- 1\nA <- R6::R6Class(parent_env = environment(), public = list(read = function() value))\nprint(A)\n";
    assert!(findings(source, "unused-binding").is_empty());
    assert!(findings(source, "undefined-symbol").is_empty());
    assert!(compute_definition(source, source.rfind("value").unwrap()).is_none());
    assert!(compute_rename(source, 0, "other", PositionEncoding::Utf16).is_none());
}
