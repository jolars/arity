use arity::incremental::IncrementalDatabase;
use arity::parser::parse;
use arity::project::classes::{ClassId, ClassIndex, R6Api};
use arity::project::classes::{ClassName, MemberKind, file_class_defs};
use arity::semantic::SemanticModel;
use std::path::Path;

fn index(source: &str) -> ClassIndex {
    let mut db = IncrementalDatabase::default();
    let path = Path::new("/arity-r6-test/R/a.R");
    let file = db.upsert_file(path, source);
    db.set_workspace_members(vec![file], vec!["/arity-r6-test".into()]);
    db.set_package_metadata(vec![arity::project::PackageInfo {
        root: "/arity-r6-test".into(),
        namespace: None,
        dir_sources: Default::default(),
        native_routines: Default::default(),
    }]);
    db.snapshot().class_index()
}

fn class(index: &ClassIndex, binding: &str) -> ClassId {
    index
        .declarations
        .iter()
        .find(|(_, class)| class.declaration.binding.as_deref() == Some(binding))
        .unwrap()
        .0
        .clone()
}

fn api(index: &ClassIndex, binding: &str) -> R6Api {
    index.r6_api(&class(index, binding)).unwrap()
}

#[test]
fn r6_apis_use_generator_identity_and_effective_member_layers() {
    let index = index(
        r#"
read <- function(scale = 2) scale
Original <- R6::R6Class("Display", public = list(
  value = 1, read = read, initialize = function(value = 3) self$value <- value
), private = list(secret = 4, hidden = function(x = 5) x),
active = list(doubled = function(value) self$value * 2))
Alias <- Original
Child <- R6::R6Class("Display", inherit = Alias, class = FALSE,
  public = list(read = function(scale = 6) super$read(scale)),
  private = list(secret = 7))
"#,
    );
    let child = api(&index, "Child");
    assert!(child.public.complete && child.private.complete && child.super_members.complete);
    assert_eq!(child.public.entries["read"].len(), 1);
    let own = child.public.entries["read"][0]
        .declaration
        .as_ref()
        .unwrap();
    assert_eq!(
        index.member(own).unwrap().class,
        Some(class(&index, "Child").key)
    );
    assert_eq!(
        child.public.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("6")
    );
    assert_eq!(
        child.super_members.entries["read"][0]
            .signature
            .as_ref()
            .unwrap()
            .parameters[0]
            .default
            .as_deref(),
        Some("2")
    );
    assert!(!child.super_members.entries.contains_key("value"));
    assert!(child.super_members.entries.contains_key("hidden"));
    assert!(child.super_members.entries.contains_key("doubled"));
    assert!(!child.public.entries.contains_key("secret"));
    assert_eq!(
        child.private.entries["secret"][0]
            .declaration
            .as_ref()
            .unwrap()
            .key,
        index
            .file(Path::new("/arity-r6-test/R/a.R"))
            .unwrap()
            .facts
            .oop
            .members
            .last()
            .unwrap()
            .key
    );
    assert_eq!(
        child.public.entries["doubled"][0].kind,
        MemberKind::ActiveBinding
    );
    assert_eq!(child.initializer.unwrap().parameters[0].name, "value");
    assert_eq!(child.class_attribute, Some(vec![]));
    assert!(child.generator.entries.contains_key("new"));
    assert!(!child.generator.entries.contains_key("read"));
    assert!(!child.public.entries.contains_key("new"));
    assert!(child.public.entries["clone"][0].declaration.is_none());
}

#[test]
fn r6_unnamed_classes_and_clone_flags_follow_r6_instantiation() {
    let index = index(
        r#"
Base <- R6::R6Class(cloneable = FALSE)
Child <- R6::R6Class("Child", inherit = Base)
Grandchild <- R6::R6Class(inherit = Child)
Disabled <- R6::R6Class(inherit = Grandchild, cloneable = FALSE)
"#,
    );
    assert_eq!(api(&index, "Base").class_attribute, Some(vec!["R6".into()]));
    assert_eq!(
        api(&index, "Child").class_attribute,
        Some(vec!["Child".into(), "R6".into()])
    );
    for binding in ["Base", "Child", "Disabled"] {
        assert!(
            !api(&index, binding).public.entries.contains_key("clone"),
            "{binding}"
        );
    }
    assert!(
        api(&index, "Grandchild")
            .public
            .entries
            .contains_key("clone")
    );
    assert!(
        api(&index, "Grandchild")
            .super_members
            .entries
            .contains_key("clone")
    );
    assert_eq!(api(&index, "Base").initializer.unwrap().parameters.len(), 0);
}

#[test]
fn r6_fields_install_after_methods_and_active_collisions_are_incomplete() {
    let index = index(
        r#"
Base <- R6::R6Class(public = list(x = 1, y = function() 2))
Child <- R6::R6Class(inherit = Base, public = list(x = function() 3, y = 4))
Broken <- R6::R6Class(inherit = Child, active = list(x = function() 5))
"#,
    );
    let child = api(&index, "Child");
    assert_eq!(child.public.entries["x"][0].kind, MemberKind::Field);
    assert_eq!(child.public.entries["y"][0].kind, MemberKind::Field);
    assert!(!api(&index, "Broken").public.complete);
}

#[test]
fn r6_unknown_shapes_cycles_and_flags_stay_incomplete() {
    for source in [
        "A <- R6::R6Class(public = members)\n",
        "A <- R6::R6Class(inherit = unknown)\n",
        "A <- R6::R6Class(inherit = B)\nB <- R6::R6Class(inherit = A)\n",
        "A <- R6::R6Class(cloneable = flag)\n",
        "A <- R6::R6Class(public = list(x = 1, x = 2))\n",
        "A <- R6::R6Class(parent_env = env)\n",
        "A <- R6::R6Class(public = list(initialize = unknown))\n",
        "list <- function(...) NULL\nA <- R6::R6Class(public = list(x = 1))\n",
    ] {
        let index = index(source);
        let actual = api(&index, "A");
        assert!(!actual.public.complete, "{source}");
        assert!(actual.initializer.is_none(), "{source}");
    }
}

fn completions(marked: &str) -> Vec<lsp_types::CompletionItem> {
    let offset = marked.find('|').unwrap();
    let source = marked.replacen('|', "", 1);
    match arity::lsp::compute_completions(
        &source,
        offset,
        &arity::rindex::provider::IndexedProvider::empty(),
    )
    .unwrap()
    {
        lsp_types::CompletionResponse::List(list) => list.items,
        lsp_types::CompletionResponse::Array(items) => items,
    }
}

fn help(marked: &str) -> Option<lsp_types::SignatureHelp> {
    let offset = marked.find('|').unwrap();
    arity::lsp::compute_signature_help(
        &marked.replacen('|', "", 1),
        offset,
        &arity::rindex::provider::IndexedProvider::empty(),
    )
}

const EDITOR_CLASSES: &str = r#"
Original <- R6::R6Class("Display", public = list(value = 1,
  initialize = function(value = 3, label = "hi") self$value <- value,
  read = function(scale = 2) self$value * scale),
  private = list(secret = 4, hidden = function(x = 5) x),
  active = list(doubled = function(value) self$value * 2))
Alias <- Original
Child <- R6::R6Class(inherit = Alias, class = FALSE,
  public = list(read = function(scale = 6) super$read(scale),
  probe = function() { PROBE }), private = list(own_secret = 8))
Other <- R6::R6Class(public = list(unrelated = 9))
obj <- Child$new()
"#;

#[test]
fn r6_completion_separates_generators_instances_and_method_receivers() {
    for (marked, expected, absent) in [
        (
            "Alias$|",
            vec!["new", "set"],
            vec!["value", "read", "secret"],
        ),
        (
            "obj$|",
            vec!["value", "read", "clone", "doubled"],
            vec!["new", "set", "secret", "unrelated"],
        ),
        (
            "Child$new()$|",
            vec!["value", "probe"],
            vec!["secret", "new"],
        ),
    ] {
        let items = completions(&format!("{EDITOR_CLASSES}\n{marked}"));
        let names: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
        for name in expected {
            assert!(names.contains(&name), "{marked}: {names:?}");
        }
        for name in absent {
            assert!(!names.contains(&name), "{marked}: {names:?}");
        }
        if marked == "obj$|" {
            assert_eq!(
                items.iter().find(|item| item.label == "read").unwrap().kind,
                Some(lsp_types::CompletionItemKind::METHOD)
            );
            assert_eq!(
                items
                    .iter()
                    .find(|item| item.label == "doubled")
                    .unwrap()
                    .kind,
                Some(lsp_types::CompletionItemKind::PROPERTY)
            );
        }
    }
    for (probe, expected, absent) in [
        (
            "self$|",
            vec!["value", "read", "probe"],
            vec!["secret", "unrelated"],
        ),
        (
            "private$|",
            vec!["secret", "own_secret", "hidden"],
            vec!["value", "unrelated"],
        ),
        (
            "super$|",
            vec!["read", "hidden", "doubled"],
            vec!["value", "probe", "own_secret"],
        ),
    ] {
        let marked = EDITOR_CLASSES.replace("PROBE", probe);
        let names: Vec<_> = completions(&marked)
            .into_iter()
            .map(|item| item.label)
            .collect();
        for name in expected {
            assert!(names.iter().any(|item| item == name), "{probe}: {names:?}");
        }
        for name in absent {
            assert!(!names.iter().any(|item| item == name), "{probe}: {names:?}");
        }
    }
}

#[test]
fn r6_signature_help_uses_initializers_overrides_and_owning_superclass() {
    for (expression, expected, active) in [
        (
            "Child$new(label = |)",
            "new(value = 3, label = \"hi\")",
            Some(1),
        ),
        ("obj$read(|)", "read(scale = 6)", Some(0)),
        ("obj$clone(|)", "clone(deep = FALSE)", Some(0)),
    ] {
        let actual = help(&format!("{EDITOR_CLASSES}\n{expression}")).expect(expression);
        assert_eq!(actual.signatures[0].label, expected);
        assert_eq!(actual.active_parameter, active);
    }
    let marked = EDITOR_CLASSES.replace("PROBE", "super$read(|)");
    assert_eq!(
        help(&marked).unwrap().signatures[0].label,
        "read(scale = 2)"
    );
    let marked = EDITOR_CLASSES.replace("PROBE", "private$hidden(|)");
    assert_eq!(help(&marked).unwrap().signatures[0].label, "hidden(x = 5)");
    assert!(help(&format!("{EDITOR_CLASSES}\nobj$doubled(|)")).is_none());
    assert!(help(&format!("{EDITOR_CLASSES}\nobj$value(|)")).is_none());
    assert!(help(&format!("{EDITOR_CLASSES}\nobj <- unknown\nobj$read(|)")).is_none());
}

#[test]
fn r6_receivers_find_later_generator_bindings_across_closure_boundaries() {
    let declaration = "A <- R6::R6Class(public = list(value = 1, initialize = function(value = 2) self$value <- value))";
    for body in [
        "make <- function() { obj <- A$new(); obj$| }",
        "make <- function() { A$new()$| }",
    ] {
        for source in [
            format!("{body}\n{declaration}\n"),
            format!("factory <- function() {{ {body}\n{declaration}\nmake }}\n"),
        ] {
            let items = completions(&source);
            assert!(items.iter().any(|item| item.label == "value"), "{source}");
        }
    }
    for source in [
        format!("make <- function() A$new(|)\n{declaration}\n"),
        format!("factory <- function() {{ make <- function() A$new(|)\n{declaration}\nmake }}\n"),
    ] {
        assert_eq!(
            help(&source).unwrap().signatures[0].label,
            "new(value = 2)",
            "{source}"
        );
    }
    for source in [
        format!("A$new(|)\n{declaration}\n"),
        format!("make <- function() {{ A$new(|)\n{declaration} }}\n"),
        format!("Alias <- A\n{declaration}\nmake <- function() Alias$new(|)\n"),
    ] {
        assert!(help(&source).is_none(), "{source}");
    }
}

#[test]
fn r6_receivers_respect_lexical_shadowing_and_unknown_accessors() {
    let marked = EDITOR_CLASSES.replace("PROBE", "self <- list(local = 1); self$|");
    let labels: Vec<_> = completions(&marked)
        .into_iter()
        .map(|item| item.label)
        .collect();
    assert!(labels.contains(&"local".to_string()));
    assert!(!labels.contains(&"value".to_string()));
    assert!(!labels.contains(&"unrelated".to_string()));
    let marked = EDITOR_CLASSES.replace("PROBE", "function(self) self$read(|)");
    assert!(help(&marked).is_none());
    let marked = EDITOR_CLASSES.replace("PROBE", "function(private) private$hidden(|)");
    assert!(help(&marked).is_none());
    let unknown = "A <- R6::R6Class(public = unknown_members)\nobj <- A$new()\nobj$|";
    let items = completions(unknown);
    assert!(
        !items
            .iter()
            .any(|item| item.label == "arity_completion_placeholder")
    );
    assert!(help("A <- R6::R6Class(public = unknown_members)\nA$new(|)").is_none());
    for source in [
        "if (TRUE) [ $|",
        "A <- R6::R6Class(public = list(x = 1, read = function() self$|",
        "f(][ $|",
    ] {
        let offset = source.find('|').unwrap();
        let _ = arity::lsp::compute_completions(
            &source.replacen('|', "", 1),
            offset,
            &arity::rindex::provider::IndexedProvider::empty(),
        );
    }
}

#[test]
fn r6_stored_lists_are_fields_and_resolved_functions_are_methods() {
    let source = "read <- function(scale = 2) scale\nAlias <- read\nGenerator <- R6::R6Class(public = base::list(payload = base::list(), read = Alias))\n";
    let root = parse(source).cst;
    let facts = file_class_defs(&SemanticModel::build(&root), &root);
    let member = |name: &str| {
        facts
            .oop
            .members
            .iter()
            .find(|member| member.name == ClassName::Known(name.into()))
            .unwrap()
    };
    assert_eq!(member("payload").kind, MemberKind::Field);
    assert_eq!(member("read").kind, MemberKind::Method);
    let method = &facts.oop.methods[0];
    assert_eq!(
        method.signature.as_ref().unwrap().parameters[0]
            .default
            .as_deref(),
        Some("2")
    );
}

#[test]
fn r6_stored_list_classification_confirms_the_constructor_origin() {
    for (prefix, complete) in [("", true), ("list <- function(...) function() 1\n", false)] {
        let index = index(&format!(
            "{prefix}A <- R6::R6Class(public = base::list(payload = list()))\n"
        ));
        let api = api(&index, "A");
        assert_eq!(api.public.complete, complete);
        if complete {
            assert_eq!(api.public.entries["payload"][0].kind, MemberKind::Field);
        }
    }
}
