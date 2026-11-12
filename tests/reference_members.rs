use arity::lsp::{compute_completions, compute_signature_help};
use arity::project::classes::{ClassName, DispatchClass, MemberKind, file_class_defs};
use arity::rindex::provider::IndexedProvider;
use arity::semantic::SemanticModel;
use lsp_types::{CompletionItem, CompletionItemKind, CompletionResponse};

const CLASSES: &str = r#"
read_impl <- function(scale = 2) value * scale
Base <- methods::setRefClass("Display", fields = list(value = "numeric",
  doubled = function(value) { if (missing(value)) 42 else stop("read only") }),
  methods = list(read = read_impl, initialize = function(value = 3, ...) initFields(value = value, ...)))
Alias <- Base
Child <- methods::setRefClass("Child", contains = "Display", fields = c(label = "character"),
  methods = list(read = function(scale = 4) callSuper(scale), probe = function() { PROBE }))
Grandchild <- methods::setRefClass("Grandchild", contains = "Child")
Other <- methods::setRefClass("Other", fields = "unrelated")
obj <- Grandchild$new()
"#;

fn completion(marked: &str) -> Vec<CompletionItem> {
    let offset = marked.find('|').unwrap();
    match compute_completions(
        &marked.replacen('|', "", 1),
        offset,
        &IndexedProvider::empty(),
    )
    .unwrap()
    {
        CompletionResponse::List(list) => list.items,
        CompletionResponse::Array(items) => items,
    }
}

fn help(marked: &str) -> Option<String> {
    let offset = marked.find('|').unwrap();
    compute_signature_help(
        &marked.replacen('|', "", 1),
        offset,
        &IndexedProvider::empty(),
    )
    .map(|help| help.signatures[0].label.clone())
}

#[test]
fn reference_field_vectors_types_and_accessors_are_distinct_from_slots() {
    let source = "Formal <- methods::setClass('Formal', slots = c(slot = 'numeric'))\nA <- methods::setRefClass('A', contains = 'Formal', fields = c('first', 'second'))\nB <- methods::setRefClass('B', fields = list(value = 'numeric', active = function(value) 1))\nC <- methods::setRefClass('C', fields = 'single')\n";
    let parsed = arity::parser::parse(source);
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.cst.text().to_string(), source);
    let facts = file_class_defs(&SemanticModel::build(&parsed.cst), &parsed.cst);
    for name in ["first", "second", "single", "value", "active", "slot"] {
        let member = facts
            .oop
            .members
            .iter()
            .find(|m| m.name == ClassName::Known(name.into()))
            .expect(name);
        assert_eq!(
            member.kind,
            match name {
                "active" => MemberKind::ActiveBinding,
                "slot" => MemberKind::Slot,
                _ => MemberKind::Field,
            }
        );
        if matches!(name, "first" | "second" | "single") {
            assert_eq!(member.declared_type, Some(DispatchClass::Any));
        }
        if name == "value" {
            assert_eq!(
                member.declared_type,
                Some(DispatchClass::Formal("numeric".into()))
            );
        }
    }
}

#[test]
fn reference_completion_uses_effective_instance_and_generator_surfaces() {
    for expression in [
        "obj$|",
        "Grandchild()$|",
        "Alias$new()$|",
        "getRefClass('Child')$new()$|",
    ] {
        let items = completion(&format!("{CLASSES}\n{expression}"));
        for (name, kind) in [
            ("value", CompletionItemKind::FIELD),
            ("read", CompletionItemKind::METHOD),
            ("doubled", CompletionItemKind::PROPERTY),
            ("copy", CompletionItemKind::METHOD),
        ] {
            let item = items
                .iter()
                .find(|item| item.label == name)
                .unwrap_or_else(|| panic!("{expression}: missing {name}: {items:?}"));
            assert_eq!(item.kind, Some(kind));
        }
        for name in ["new", "methods", "unrelated", "slot"] {
            assert!(
                !items.iter().any(|item| item.label == name),
                "{expression}: {name}"
            );
        }
    }
    for expression in ["Alias$|", "getRefClass('Display')$|"] {
        let items = completion(&format!("{CLASSES}\n{expression}"));
        for name in [
            "new",
            "methods",
            "fields",
            "accessors",
            "className",
            "def",
            "copy",
        ] {
            assert!(
                items.iter().any(|item| item.label == name),
                "{expression}: {name}"
            );
        }
        assert!(!items.iter().any(|item| item.label == "value"));
    }
    let items = completion(&CLASSES.replace("PROBE", ".self$|"));
    assert!(items.iter().any(|item| item.label == "label"));
    assert!(!items.iter().any(|item| item.label == "unrelated"));
    let items = completion(&format!("{CLASSES}\nobj@|"));
    assert!(!items.iter().any(|item| item.label == "value"));
}

#[test]
fn reference_signatures_forward_initializers_and_hide_external_self() {
    for (expression, expected) in [
        ("Alias(|)", "Alias(value = 3, ...)"),
        ("Grandchild$new(|)", "new(value = 3, ...)"),
        ("getRefClass('Child')$new(|)", "new(value = 3, ...)"),
        ("obj$read(|)", "read(scale = 4)"),
        ("obj$copy(|)", "copy(shallow = FALSE)"),
        ("obj$field(|)", "field(name, value)"),
        ("obj$initFields(|)", "initFields(...)"),
    ] {
        assert_eq!(
            help(&format!("{CLASSES}\n{expression}")).as_deref(),
            Some(expected),
            "{expression}"
        );
    }
    let source = "A <- methods::setRefClass('A', methods = list(read = function(.self, x = 2) .self))\na <- A()\na$read(|)";
    assert_eq!(help(source).as_deref(), Some("read(x = 2)"));
    for name in ["doubled", "value"] {
        assert!(help(&format!("{CLASSES}\nobj${name}(|)")).is_none());
    }
}

#[test]
fn reference_super_signatures_are_relative_to_the_owning_method() {
    assert_eq!(
        help(&CLASSES.replace("callSuper(scale)", "callSuper(|)")).as_deref(),
        Some("callSuper(scale = 2)")
    );
    assert_eq!(
        help(&CLASSES.replace("PROBE", "read(|)")).as_deref(),
        Some("read(scale = 4)")
    );
    assert_eq!(
        help(&CLASSES.replace("PROBE", ".self$read(|)")).as_deref(),
        Some("read(scale = 4)")
    );
    let source = "A <- methods::setRefClass('A', methods = list(read = function(a = 1) a))\nB <- methods::setRefClass('B', contains = 'A', methods = list(read = function(b = 2) { f <- function() callSuper(|); f() }))\nC <- methods::setRefClass('C', contains = 'B', methods = list(read = function(c = 3) c))\n";
    assert_eq!(help(source).as_deref(), Some("callSuper(a = 1)"));
}

#[test]
fn reference_super_calls_include_builtin_methods_and_skip_s4_slot_parents() {
    for (method, expected) in [
        ("copy", "callSuper(shallow = FALSE)"),
        ("show", "callSuper()"),
        ("initialize", "callSuper(...)"),
    ] {
        let source = format!(
            "A <- methods::setRefClass('A', methods = list({method} = function() callSuper(|)))\n"
        );
        assert_eq!(help(&source).as_deref(), Some(expected), "{source}");
    }
    let source = "Formal <- methods::setClass('Formal', slots = c(slot = 'numeric'))\nBase <- methods::setRefClass('Base', methods = list(read = function(scale = 2) scale))\nChild <- methods::setRefClass('Child', contains = c('Formal', 'Base'), methods = list(read = function(scale = 4) callSuper(|)))\n";
    assert_eq!(help(source).as_deref(), Some("callSuper(scale = 2)"));
    assert_eq!(
        help("A <- methods::setRefClass('A', contains = 'envRefClass')\nA$new(|)").as_deref(),
        Some("new(...)")
    );
}

#[test]
fn reference_initialization_super_uses_effective_init_fields() {
    for (parent, contains) in [
        ("", ""),
        (
            "Parent <- methods::setRefClass('Parent', methods = list(initFields = function(x = 7) .self))\n",
            "contains = 'Parent', ",
        ),
    ] {
        let helper = if parent.is_empty() {
            "initFields = function(x = 7) .self, "
        } else {
            ""
        };
        let source = format!(
            "{parent}A <- methods::setRefClass('A', {contains}methods = list({helper}initialize = function(...) callSuper(|)))\n"
        );
        assert_eq!(
            help(&source).as_deref(),
            Some("callSuper(x = 7)"),
            "{source}"
        );
    }
}

#[test]
fn reference_receivers_respect_shadowing_and_ambiguous_evidence() {
    for prefix in [
        "obj <- unknown",
        "obj <- if (flag) Grandchild() else Other()",
    ] {
        assert!(help(&format!("{CLASSES}\n{prefix}\nobj$read(|)")).is_none());
    }
    for source in [
        "A <- methods::setRefClass('A', fields = fields)\nA$new(|)",
        "A <- methods::setRefClass('A', contains = 'Unknown')\nA$new(|)",
        "A <- methods::setRefClass('A', fields = c(named = 'numeric', 'unnamed'))\nA$new(|)",
        "A <- methods::setRefClass('A', fields = list(value = unknown_type))\nA$new(|)",
        "getRefClass <- function(...) NULL\nA <- methods::setRefClass('A', methods = list(read = function(x) x))\ngetRefClass('A')$new()$read(|)",
        "A <- methods::setRefClass('A', methods = list(read = function(x) x))\ngetRefClass('A', where = env)$new()$read(|)",
    ] {
        assert!(help(source).is_none(), "{source}");
    }
    assert!(help(&CLASSES.replace("PROBE", "function(.self) .self$read(|)")).is_none());
    assert!(
        help(&CLASSES.replace("PROBE", "callSuper <- function(...) 1; callSuper(|)")).is_none()
    );
    assert!(help("getRefClass('Later')$new(|)\nA <- methods::setRefClass('Later')\n").is_none());
}

#[test]
fn reference_api_preserves_parent_order_types_and_s4_slot_identity() {
    use arity::incremental::IncrementalDatabase;
    use arity::project::classes::ClassId;
    use std::path::Path;
    let source = "Formal <- methods::setClass('Formal', slots = c(slot = 'numeric'))\nFirst <- methods::setRefClass('First', fields = c(value = 'numeric'), methods = list(read = function(first = 1) first))\nSecond <- methods::setRefClass('Second', fields = c('other'), methods = list(read = function(second = 2) second))\nChild <- methods::setRefClass('Child', contains = c('First', 'Second', 'Formal'), fields = 'own', methods = list(read = function(child = 3) callSuper()))\n";
    let root = arity::parser::parse(source).cst;
    let model = SemanticModel::build(&root);
    let facts = file_class_defs(&model, &root);
    let path = Path::new("/reference-api.R");
    let index = IncrementalDatabase::default()
        .snapshot()
        .class_index_in(path, &facts, &root, &model);
    let id = ClassId {
        path: path.into(),
        key: facts.declarations.last().unwrap().key.clone(),
    };
    let api = index.reference_api(&id).unwrap();
    assert!(api.instance.complete && api.slots.complete);
    for name in ["value", "other", "own", "read", "copy"] {
        assert!(api.instance.entries.contains_key(name), "{name}");
    }
    assert!(!api.instance.entries.contains_key("slot"));
    assert_eq!(api.slots.entries["slot"][0].kind, MemberKind::Slot);
    assert_eq!(
        api.instance.entries["value"][0].declared_type,
        Some(DispatchClass::Formal("numeric".into()))
    );
    assert_eq!(
        api.instance.entries["other"][0].declared_type,
        Some(DispatchClass::Any)
    );
    assert_eq!(
        index
            .reference_super_method(&id, "read")
            .unwrap()
            .signature
            .unwrap()
            .parameters[0]
            .name,
        "first"
    );
    assert_eq!(
        api.instance.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .name,
        "child"
    );
    let items = completion(&format!("{source}\nChild()@|"));
    assert!(items.iter().any(|item| item.label == "slot"));
    assert!(!items.iter().any(|item| item.label == "value"));
}

#[test]
fn reference_empty_fields_and_default_initialization_accept_dots() {
    for fields in ["character()", "base::character()", "c()", "list()"] {
        assert_eq!(
            help(&format!(
                "A <- methods::setRefClass('A', fields = {fields})\nA$new(|)"
            ))
            .as_deref(),
            Some("new(...)")
        );
    }
    assert!(help("character <- function() 'surprise'\nA <- methods::setRefClass('A', fields = character())\nA$new(|)").is_none());
}

#[test]
fn reference_accessors_execute_in_the_object_enclosure_without_becoming_methods() {
    let source = "A <- methods::setRefClass('A', fields = list(value = 'numeric', doubled = function(new_value) { if (missing(new_value)) value * 2 else value <<- new_value / 2 }))\nprint(A)\n";
    let config = arity::config::LintConfig {
        select: Some(vec!["undefined-symbol".into()]),
        ..Default::default()
    };
    let diagnostics =
        arity::linter::check_document(std::path::Path::new("test.R"), source, &config).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(help(&format!("{source}\nA()$doubled(|)")).is_none());
    let items = completion(&format!("{source}\nA()$|"));
    let item = items.iter().find(|item| item.label == "doubled").unwrap();
    assert_eq!(item.kind, Some(CompletionItemKind::PROPERTY));
    assert!(
        item.label_details
            .as_ref()
            .and_then(|detail| detail.detail.as_ref())
            .is_none()
    );
    assert_eq!(
        items
            .iter()
            .find(|item| item.label == "value")
            .unwrap()
            .label_details
            .as_ref()
            .unwrap()
            .detail
            .as_deref(),
        Some(": numeric")
    );
}
