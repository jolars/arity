use std::path::Path;

use arity::config::LintConfig;
use arity::linter::check_document;
use arity::lsp::{
    compute_definition, compute_document_symbols, compute_prepare_rename, compute_references,
    compute_rename,
};
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
fn ordinary_functions_do_not_acquire_dispatch_metadata() {
    let source = "f <- function() list(.Generic, .Method, .Class, .Group, .GenericCallEnv, .GenericDefEnv, .Methods, .defined, .target)\nf()\n";
    assert_eq!(
        findings(source, "undefined-symbol"),
        [
            ".Generic",
            ".Method",
            ".Class",
            ".Group",
            ".GenericCallEnv",
            ".GenericDefEnv",
            ".Methods",
            ".defined",
            ".target",
        ]
    );
}

#[test]
fn s3_methods_supply_metadata_to_defaults_and_nested_closures() {
    let source = "g <- function(x) UseMethod('g')\ng.C <- function(x, meta = .Generic) { nested <- function() list(.Method, .Class, .Group, .GenericCallEnv, .GenericDefEnv); list(meta, nested(), .Methods, .defined, .target, missing_name) }\n";
    assert_eq!(
        findings(source, "undefined-symbol"),
        [".Methods", ".defined", ".target", "missing_name"]
    );
}

#[test]
fn s3_internal_group_and_unproven_conventional_methods_are_conservative() {
    for name in ["print.C", "[.C", "+.C", "Ops.C", "Math.C", "unknown.C"] {
        let source = format!(
            "`{name}` <- function(x, ...) list(.Generic, .Method, .Class, .Group, .GenericCallEnv, .GenericDefEnv, ordinary_missing)\n"
        );
        assert_eq!(
            findings(&source, "undefined-symbol"),
            ["ordinary_missing"],
            "{name}"
        );
    }
}

#[test]
fn registered_methods_use_their_dispatch_mechanism() {
    let source = "g <- function(x) UseMethod('g')\nbase::registerS3method('g', 'C', function(x) list(.Generic, .Group, .Methods))\nmethods::setGeneric('h', function(x, y = 1) standardGeneric('h'), signature = 'x')\nmethods::setMethod('h', 'ANY', function(x, y = .defined) list(.Generic, .Method, .Methods, .defined, .target, .Class, .Group, .GenericCallEnv, .GenericDefEnv))\nmethods::setMethod('length', 'ANY', function(x) list(.Generic, .Method, .Methods, .defined, .target))\n";
    assert_eq!(
        findings(source, "undefined-symbol"),
        [
            ".Methods",
            ".Class",
            ".Group",
            ".GenericCallEnv",
            ".GenericDefEnv",
            ".Method",
            ".Methods",
            ".defined",
            ".target"
        ]
    );
}

#[test]
fn dispatch_identities_have_references_without_editable_definitions() {
    let source = "g <- function(x) UseMethod('g')\ng.C <- function(x) { .Generic; function() .Generic }\ng.D <- function(x) .Generic\n";
    let offset = source.find(".Generic").unwrap();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
    assert!(compute_rename(source, offset, "changed", PositionEncoding::Utf16).is_none());
    assert_eq!(compute_references(source, offset, true).unwrap().len(), 2);
    let symbols = compute_document_symbols(source, PositionEncoding::Utf16);
    assert_eq!(
        symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["g", "g.C", "g.D"]
    );
}

#[test]
fn dispatch_metadata_respects_lexical_shadowing_and_unused_reads() {
    let source = ".Generic <- 1\ng <- function(x) UseMethod('g')\ng.C <- function(x) .Generic\ng.D <- function(x, .Generic) .Generic\ng.E <- function(x) { .Generic <- 2; .Generic }\nprint(list(g, g.C, g.D, g.E))\n";
    let model =
        arity::project::classes::standalone_semantic_model(&arity::parser::parse(source).cst);
    let outer = model
        .bindings()
        .iter()
        .find(|binding| binding.name == ".Generic")
        .unwrap();
    assert!(
        !outer.read,
        "dispatch metadata must not keep the outer variable alive"
    );
    let parameter = source.find("function(x, .Generic)").unwrap() + "function(x, ".len();
    let read = source.find(".Generic\ng.E").unwrap();
    assert_eq!(
        usize::from(compute_definition(source, read).unwrap().start()),
        parameter
    );
    assert!(compute_prepare_rename(source, read, PositionEncoding::Utf16).is_some());
}

#[test]
fn generic_registration_effects_do_not_turn_class_strings_into_variables() {
    let source = "methods::setClass('DispatchClass', slots = c(value = 'numeric'))\nmethods::setGeneric(name = 'read', def = function(x) standardGeneric('read'))\nmethods::setReplaceMethod('names', 'DispatchClass', function(x, value) x)\nread(methods::new('DispatchClass'))\n`names<-`\nDispatchClass\n";
    assert_eq!(findings(source, "undefined-symbol"), ["DispatchClass"]);
    let offset = source.find("read(methods").unwrap();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
}

#[test]
fn aliases_argument_matching_and_replacement_registrations_share_contexts() {
    let source = "define <- methods::setGeneric\ninstall <- methods::setReplaceMethod\ndefine(def = function(x, value) standardGeneric('custom<-'), name = 'custom<-')\ninstall(definition = function(x, value) list(.Generic, .Methods, .target, .Class), f = 'custom', signature = 'ANY')\n`custom<-`(NULL, 1)\n";
    assert_eq!(findings(source, "undefined-symbol"), [".Class"]);
}

#[test]
fn registered_aliases_keep_ordinary_closure_uses_and_block_mixed_renames() {
    let source = ".Generic <- 'outer'\nimplementation <- function(x) { .Generic; function() .Generic }\nmethods::setMethod('show', 'ANY', implementation)\nimplementation(NULL)\n";
    assert!(findings(source, "undefined-symbol").is_empty());
    let model =
        arity::project::classes::standalone_semantic_model(&arity::parser::parse(source).cst);
    assert!(
        model
            .bindings()
            .iter()
            .find(|binding| binding.name == ".Generic")
            .unwrap()
            .read
    );
    assert!(compute_definition(source, source.find(".Generic;").unwrap()).is_none());
    assert!(compute_prepare_rename(source, 0, PositionEncoding::Utf16).is_none());
}

#[test]
fn conventional_s3_aliases_keep_ordinary_closure_uses_and_block_mixed_renames() {
    for implementation in ["helper", "(helper)", "alias"] {
        let source = format!(
            ".Group <- 'outer'\ng <- function(x) UseMethod('g')\n\
             helper <- function(x) .Group\nalias <- helper\n\
             g.C <- {implementation}\nhelper(1)\n"
        );
        assert!(findings(&source, "undefined-symbol").is_empty());
        let model =
            arity::project::classes::standalone_semantic_model(&arity::parser::parse(&source).cst);
        assert!(
            model
                .bindings()
                .iter()
                .find(|binding| binding.name == ".Group")
                .unwrap()
                .read,
            "{source}"
        );
        let read = source.find(".Group\nalias").unwrap();
        assert!(compute_definition(&source, read).is_none());
        for offset in [0, read] {
            assert!(compute_prepare_rename(&source, offset, PositionEncoding::Utf16).is_none());
            assert!(compute_rename(&source, offset, "changed", PositionEncoding::Utf16).is_none());
        }
    }
}

#[test]
fn registration_effects_respect_order_shadowing_and_custom_environments() {
    let source = "created\nmethods::setGeneric('created', function(x) standardGeneric('created'))\ncreated(1)\ncreated <- function(x) x\ncreated(2)\nmethods::setGeneric('elsewhere', function(x) x, where = base::new.env())\nelsewhere(1)\nsetGeneric <- function(...) NULL\nsetGeneric('fake', function(x) x)\nfake(1)\n";
    assert_eq!(
        findings(source, "undefined-symbol"),
        ["created", "elsewhere", "fake"]
    );
    let read = source.rfind("created(2)").unwrap();
    assert_eq!(
        usize::from(compute_definition(source, read).unwrap().start()),
        source.find("created <-").unwrap()
    );
}

#[test]
fn method_registration_uses_existing_function_without_an_editable_new_definition() {
    let source = "plain <- function(x) x\nmethods::setMethod('plain', 'ANY', function(x) list(.Generic, .Method, .Methods, .defined, .target))\nplain(1)\n";
    assert!(findings(source, "undefined-symbol").is_empty());
    assert!(findings(source, "unused-binding").is_empty());
    assert!(compute_prepare_rename(source, 0, PositionEncoding::Utf16).is_none());
}

#[test]
fn method_nonlocal_writes_distinguish_the_call_frame_from_the_lexical_parent() {
    let source = ".Generic <- 'outer'\ng <- function(x) UseMethod('g')\ng.C <- function(x) { .Generic <<- 'outer update'; f <- function() { .Generic <<- 'method update'; .Generic }; f(); .Generic }\n";
    let direct = source.find(".Generic <<-").unwrap();
    let nested = source.find(".Generic <<- 'method").unwrap();
    assert_eq!(
        usize::from(compute_definition(source, direct).unwrap().start()),
        0
    );
    assert!(compute_definition(source, nested).is_none());
    assert!(compute_prepare_rename(source, nested, PositionEncoding::Utf16).is_none());
}

#[test]
fn generic_effects_preserve_unrelated_lexical_resolution() {
    let source = "methods::setGeneric('g', function(x) standardGeneric('g'))\nf <- function() { global <<- 1 }\nf()\nprint(global)\n";
    assert!(findings(source, "undefined-symbol").is_empty());
}

#[test]
fn generic_bodies_and_computed_registrations_keep_uncertainty_local() {
    let source = "make <- function(name, where) { methods::setGeneric(name, function(x) standardGeneric(.Generic), where = where); methods::setMethod(name, 'ANY', function(x) list(.Generic, .Methods, .target, .Class, ordinary_missing)) }\n";
    assert_eq!(
        findings(source, "undefined-symbol"),
        [".Class", "ordinary_missing"]
    );
}

#[test]
fn bare_aliases_of_qualified_s3_registration_are_detected() {
    let source = "install <- base::registerS3method\ninstall('print', 'C', function(x) list(.Generic, .GenericCallEnv, .Methods))\n";
    assert_eq!(findings(source, "undefined-symbol"), [".Methods"]);
}

#[test]
fn unknown_registration_environments_preserve_possible_generic_bindings() {
    let source = "install <- function(target) methods::setGeneric('possible', function(x) x, where = target)\ninstall(globalenv())\npossible(1)\n";
    assert!(findings(source, "undefined-symbol").is_empty());
}

#[test]
fn reused_implementations_recompute_nonlocal_targets_for_all_method_contexts() {
    let source = ".target <- 1\nhelper <- function(x) { nested <- function() { .target <<- 2 }; nested() }\nA <- R6::R6Class(public = list(read = helper))\nmethods::setMethod('show', 'ANY', helper)\n";
    let offset = source.find(".target <<-").unwrap();
    assert!(compute_definition(source, offset).is_none());
    assert!(compute_prepare_rename(source, offset, PositionEncoding::Utf16).is_none());
}
