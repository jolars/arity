use std::path::Path;

use arity::config::LintConfig;
use arity::linter::check_document;
use arity::lsp::{compute_definition, compute_prepare_rename, compute_references, compute_rename};
use arity::text::PositionEncoding;

fn findings(source: &str, rule: &str) -> Vec<String> {
    let parsed = arity::parser::parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.cst.text().to_string(), source);
    let config = LintConfig {
        select: Some(vec![rule.into()]),
        ..Default::default()
    };
    check_document(Path::new("test.R"), source, &config)
        .unwrap()
        .iter()
        .map(|d| source[usize::from(d.range.start())..usize::from(d.range.end())].into())
        .collect()
}

#[test]
fn reference_methods_resolve_members_helpers_defaults_and_nested_closures() {
    let source = r#"
Base <- methods::setRefClass("Base", fields = list(value = "numeric"),
    methods = list(read = function() value))
Child <- methods::setRefClass("Child", contains = "Base", fields = c(label = "character"),
    methods = list(
        initialize = function(...) initFields(...),
        read = function(scale = value) {
            nested <- function() list(.self, .refClassDef, value, label)
            list(nested(), callSuper(), scale, copy(), missing_member)
        },
        other = function() read()))
print(Child)
"#;
    assert_eq!(findings(source, "undefined-symbol"), ["missing_member"]);
}

#[test]
fn external_reference_methods_keep_their_ordinary_enclosure() {
    let source = r#"
value <- 10
A <- methods::setRefClass("A", fields = list(value = "numeric"), methods = list(
    ordinary = function() list(.self, .refClassDef, value),
    external = function(.self) list(.self$value, value, .refClassDef, initFields()),
    later = function(x, .self) list(x, .self, value)))
print(A)
"#;
    assert_eq!(
        findings(source, "undefined-symbol"),
        [".refClassDef", "initFields"]
    );
    let offset = source.find("value, .refClassDef").unwrap();
    assert_eq!(
        usize::from(compute_definition(source, offset).unwrap().start()),
        source.find("value <-").unwrap()
    );
    assert!(findings(source, "unused-binding").is_empty());
}

#[test]
fn reference_member_names_do_not_keep_lexical_bindings_alive() {
    let source = "value <- 1\nread <- function() 1\nA <- methods::setRefClass('A', fields = list(value = 'numeric'), methods = list(read = function() value, other = function() read()))\nprint(A)\n";
    assert!(findings(source, "undefined-symbol").is_empty());
    assert_eq!(findings(source, "unused-binding"), ["value", "read"]);
    let offset = source.rfind("read()").unwrap();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
    assert!(compute_rename(source, offset, "renamed", PositionEncoding::Utf16).is_none());
}

#[test]
fn reference_field_updates_are_runtime_writes_and_local_assignments_stay_local() {
    let source = r#"
A <- methods::setRefClass("A", fields = list(value = "numeric"), methods = list(
    update = function() { value <<- value + 1; value <<- 2; value },
    nested = function() { f <- function() { 3 ->> value }; f() },
    local = function() { value <- 4; value },
    unused = function() { value <- 5 }))
print(A)
value
"#;
    assert_eq!(findings(source, "undefined-symbol"), ["value"]);
    assert_eq!(findings(source, "unused-binding"), ["value"]);
    let root = arity::parser::parse(source).cst;
    let model = arity::project::classes::standalone_semantic_model(&root);
    assert!(!arity::project::file_exports(&model).contains("value"));
    let offset = source.find("value <<-").unwrap();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
    assert_eq!(compute_references(source, offset, true).unwrap().len(), 5);
}

#[test]
fn reused_reference_methods_preserve_possible_external_writes() {
    for write in ["value <<- 1", "1 ->> value"] {
        let source = format!(
            "helper <- function() {{ {write} }}\n\
             A <- methods::setRefClass('A', fields = list(value = 'numeric'),\n\
                 methods = list(update = helper))\nhelper()\nprint(value)\n"
        );
        assert!(findings(&source, "undefined-symbol").is_empty(), "{source}");
        let root = arity::parser::parse(&source).cst;
        let model = arity::project::classes::standalone_semantic_model(&root);
        assert!(arity::project::file_exports(&model).contains("value"));
        let offset = source.find(write).unwrap() + write.find("value").unwrap();
        assert!(compute_definition(&source, offset).is_none());
        for offset in [offset, source.rfind("value").unwrap()] {
            assert!(compute_prepare_rename(&source, offset, PositionEncoding::Utf16).is_none());
            assert!(compute_rename(&source, offset, "changed", PositionEncoding::Utf16).is_none());
        }
    }
}

#[test]
fn reference_lookup_respects_shadowing_and_does_not_escape_methods() {
    let source = r#"
A <- methods::setRefClass("A", fields = list(value = "numeric"), methods = list(
    read = function(value) value,
    local = function() { before <- value; value <- 1; list(before, value) }))
f <- function() list(value, .self, .refClassDef, callSuper(), initFields())
print(list(A, f))
"#;
    assert_eq!(
        findings(source, "undefined-symbol"),
        ["value", ".self", ".refClassDef", "callSuper", "initFields"]
    );
    let offset = source.find("function(value) value").unwrap() + "function(value) ".len();
    assert!(compute_definition(source, offset).is_some());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_some());
}

#[test]
fn reference_unknown_inheritance_and_fields_are_conservative_but_not_global() {
    for source in [
        "A <- methods::setRefClass('A', contains = 'Unknown', methods = list(read = function() inherited))\n",
        "make <- function(fields) methods::setRefClass('A', fields = fields, methods = list(read = function() unknown))\n",
    ] {
        assert!(findings(source, "undefined-symbol").is_empty(), "{source}");
    }
    let source = "setRefClass <- function(...) NULL\nA <- setRefClass('A', methods = list(read = function() .self))\n";
    assert_eq!(findings(source, "undefined-symbol"), [".self"]);
}

#[test]
fn r6_nonportable_member_lookup_shares_enclosures_without_reference_helpers() {
    for (portable, expected) in [
        ("FALSE", vec![".self", "initFields"]),
        (
            "TRUE",
            vec!["value", "secret", "read", ".self", "initFields"],
        ),
    ] {
        let source = format!(
            "A <- R6::R6Class(portable = {portable}, public = list(value = 1, read = function() list(value, secret, read, self, private, .self, initFields())), private = list(secret = 2))\n"
        );
        assert_eq!(findings(&source, "undefined-symbol"), expected);
    }
}

#[test]
fn reference_external_only_methods_do_not_acquire_runtime_names() {
    let source = "A <- methods::setRefClass('A', fields = list(value = 'numeric'), methods = list(read = function(.self) list(.self, value, .refClassDef)))\n";
    assert_eq!(
        findings(source, "undefined-symbol"),
        ["value", ".refClassDef"]
    );
}

#[test]
fn reference_repeated_nested_writes_do_not_create_locals_or_file_definitions() {
    let source = "A <- methods::setRefClass('A', fields = list(value = 'numeric'), methods = list(update = function() { f <- function() { value <<- 1; value <<- 2; value }; f(); value }))\nprint(A)\nvalue\n";
    assert_eq!(findings(source, "undefined-symbol"), ["value"]);
    assert!(findings(source, "unused-binding").is_empty());
    let offset = source.find("value <<-").unwrap();
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
}

#[test]
fn reference_nonlocal_writes_skip_method_locals_but_find_enclosing_closure_locals() {
    let source = "A <- methods::setRefClass('A', fields = list(value = 'numeric'), methods = list(update = function() { value <- 1; value <<- 2; f <- function() { value <<- 3 }; f(); value }))\nprint(A)\n";
    assert!(findings(source, "undefined-symbol").is_empty());
    let local = source.find("value <-").unwrap();
    let field_write = source.find("value <<- 2").unwrap();
    assert!(compute_definition(source, field_write).is_none());
    let closure_write = source.find("value <<- 3").unwrap();
    assert_eq!(
        usize::from(compute_definition(source, closure_write).unwrap().start()),
        local
    );
}

#[test]
fn reference_base_methods_can_read_fields_supplied_by_known_subclasses() {
    let source = "Base <- methods::setRefClass('Base', methods = list(read = function() child_field))\nChild <- methods::setRefClass('Child', contains = 'Base', fields = list(child_field = 'numeric'))\nprint(Child$new()$read())\n";
    assert!(findings(source, "undefined-symbol").is_empty());
}
