use arity::incremental::IncrementalDatabase;
use arity::parser::parse;
use arity::project::classes::*;
use arity::project::{PackageInfo, project_classes};
use arity::semantic::SemanticModel;
use std::collections::BTreeSet;
use std::path::Path;

fn facts(source: &str) -> FileClassFacts {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.cst.text().to_string(), source);
    file_class_defs(&SemanticModel::build(&parsed.cst), &parsed.cst)
}

fn index(source: &str) -> ClassIndex {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.cst.text().to_string(), source);
    let model = SemanticModel::build(&parsed.cst);
    IncrementalDatabase::default().snapshot().class_index_in(
        Path::new("/s4.R"),
        &file_class_defs(&model, &parsed.cst),
        &parsed.cst,
        &model,
    )
}

#[test]
fn s4_generic_definitions_resolve_callable_aliases() {
    let facts = facts(
        "skeleton <- function(x, y = 1, ...) base::standardGeneric('g')\n\
         methods::setGeneric('g', skeleton, signature = 'x')\n",
    );
    let signature = facts.oop.generics[0].signature.as_ref().unwrap();
    assert_eq!(signature.parameters[0].name, "x");
    assert_eq!(signature.parameters[1].default.as_deref(), Some("1"));
    assert_eq!(signature.parameters[2].name, "...");
}

#[test]
fn s4_empty_registration_signature_is_any_not_dynamic() {
    for registration in [
        "methods::setMethod('g', definition = function(x, y) x)",
        "methods::setMethod('g', character(), function(x, y) x)",
        "methods::setMethod('g', signature(), function(x, y) x)",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x, y) standardGeneric('g'))\n{registration}\n"
        );
        let facts = facts(&source);
        assert!(facts.oop.methods[0].dispatch.is_empty(), "{source}");
        assert!(
            !facts.oop.methods[0]
                .uncertainty
                .contains(&OopUncertainty::DynamicSignature),
            "{source}"
        );
    }
}

fn generic_id(name: &str, package: &str) -> S4GenericId {
    S4GenericId::Named {
        name: name.into(),
        package: package.into(),
    }
}

fn packages(one: &str, two: &str, imports: &str) -> ClassIndex {
    package_files(one, &[("b.R", two)], imports)
}

fn package_files(one: &str, two: &[(&str, &str)], imports: &str) -> ClassIndex {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(Path::new("/one/R/a.R"), one);
    let mut members = vec![a];
    members.extend(
        two.iter()
            .map(|(name, source)| db.upsert_file(&Path::new("/two/R").join(name), *source)),
    );
    let roots = vec!["/one".into(), "/two".into()];
    db.set_workspace_members(members, roots.clone());
    db.refresh_descriptions(roots);
    db.upsert_description(Path::new("/one"), "Package: one\n");
    db.upsert_description(
        Path::new("/two"),
        format!(
            "Package: two\nImports: one\nCollate: {}\n",
            two.iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    db.set_package_metadata(
        [("/one", "export(g)\n"), ("/two", imports)]
            .into_iter()
            .map(|(root, namespace)| PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect(),
    );
    project_classes(&db).clone()
}

fn collated_package(files: &[(&str, &str)]) -> ClassIndex {
    let mut db = IncrementalDatabase::default();
    let members = files
        .iter()
        .map(|(name, source)| db.upsert_file(&Path::new("/pkg/R").join(name), *source))
        .collect();
    db.set_workspace_members(members, vec!["/pkg".into()]);
    db.refresh_descriptions(["/pkg".into()]);
    db.upsert_description(
        Path::new("/pkg"),
        format!(
            "Package: pkg\nCollate: {}\n",
            files
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    db.set_package_metadata(vec![PackageInfo {
        root: "/pkg".into(),
        namespace: Some(String::new()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    project_classes(&db).clone()
}

#[test]
fn registrations_before_a_local_generic_use_the_imported_generic() {
    let source = "methods::setMethod('g', 'numeric', function(x, y) x)\n\
        methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y')\n\
        methods::setMethod('g', 'character', function(x, y) y)\n";
    let index = packages(
        "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'x')\n",
        source,
        "importFrom(one, g)\n",
    );
    let methods: Vec<_> = index
        .s4
        .methods
        .iter()
        .filter(|method| matches!(method.id, S4MethodId::Registration(_)))
        .collect();
    assert_eq!(methods.len(), 2);
    for (method, package, argument) in [(methods[0], "one", "x"), (methods[1], "two", "y")] {
        assert_eq!(method.generics, [generic_id("g", package)]);
        assert_eq!(method.dispatch.len(), 1, "{method:?}");
        assert!(method.dispatch[0].complete, "{method:?}");
        assert_eq!(
            method.dispatch[0].arguments[0].argument.as_deref(),
            Some(argument)
        );
    }
}

#[test]
fn registrations_before_a_sibling_generic_use_the_imported_generic() {
    let index = package_files(
        "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'x')\n",
        &[
            (
                "a.R",
                "methods::setMethod('g', 'numeric', function(x, y) x)\n",
            ),
            (
                "b.R",
                "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y')\n",
            ),
            (
                "c.R",
                "methods::setMethod('g', 'character', function(x, y) y)\n",
            ),
        ],
        "importFrom(one, g)\n",
    );
    let methods: Vec<_> = index
        .s4
        .methods
        .iter()
        .filter(|method| matches!(method.id, S4MethodId::Registration(_)))
        .collect();
    assert_eq!(methods.len(), 2);
    for (method, package, argument) in [(methods[0], "one", "x"), (methods[1], "two", "y")] {
        assert_eq!(method.generics, [generic_id("g", package)], "{method:?}");
        assert_eq!(method.dispatch.len(), 1, "{method:?}");
        assert!(method.dispatch[0].complete, "{method:?}");
        assert_eq!(
            method.dispatch[0].arguments[0].argument.as_deref(),
            Some(argument)
        );
    }
}

#[test]
fn exact_methods_do_not_leak_between_replacement_generic_objects() {
    let source = "g <- function(x) x\n\
        methods::setMethod('g', 'numeric', function(x) x)\n\
        g <- function(x) x\n\
        methods::setMethod('g', 'character', function(x) x)\n\
        methods::selectMethod('g', 'numeric', useInherited = FALSE, optional = TRUE)\n\
        methods::selectMethod('g', 'character', useInherited = FALSE)\n";
    let index = index(source);
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert_eq!(calls.len(), 2);
    assert!(calls[0].exact_methods.is_empty(), "{:?}", calls[0]);
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| {
            matches!(method.id, S4MethodId::Registration(_))
                && method.supplied_signature[0].class == DispatchClass::Formal("character".into())
        })
        .unwrap();
    assert_eq!(
        calls[1].exact_methods.as_slice(),
        std::slice::from_ref(&method.id)
    );
}

#[test]
fn exact_methods_follow_reused_imported_generic_objects() {
    let source = "methods::setGeneric('g')\n\
        methods::selectMethod('g', 'numeric', useInherited = FALSE)\n";
    let index = packages(
        "methods::setGeneric('g', function(x) standardGeneric('g'))\n\
        methods::setMethod('g', 'numeric', function(x) x)\n",
        source,
        "importFrom(one, g)\n",
    );
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| matches!(method.id, S4MethodId::Registration(_)))
        .unwrap();
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/two/R/b.R"), &parsed.cst, &model);
    assert_eq!(
        calls[0].exact_methods.as_slice(),
        std::slice::from_ref(&method.id)
    );
}

#[test]
fn exact_methods_follow_redeclarations_of_promoted_generics() {
    let source = "g <- function(x) x\n\
        methods::setMethod('g', 'numeric', function(x) x)\n\
        methods::setGeneric('g')\n\
        methods::selectMethod('g', 'numeric', useInherited = FALSE)\n";
    let index = index(source);
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| matches!(method.id, S4MethodId::Registration(_)))
        .unwrap();
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert_eq!(
        calls[0].exact_methods.as_slice(),
        std::slice::from_ref(&method.id)
    );
    assert_eq!(
        index
            .s4
            .methods
            .iter()
            .filter(|method| matches!(method.id, S4MethodId::Default(_)))
            .count(),
        1
    );
}

#[test]
fn aliases_capture_installed_s4_generic_objects() {
    for installation in [
        "methods::setGeneric('g')",
        "methods::setMethod('g', 'numeric', function(x) x)",
    ] {
        let source = format!(
            "g <- function(x) x\n{installation}\n\
            alias <- g\ng <- function(y) y\n\
            methods::setMethod(alias, 'character', function(x) x)\n\
            methods::selectMethod(alias, 'character', useInherited = FALSE)\n"
        );
        let index = index(&source);
        let method = index
            .s4
            .methods
            .iter()
            .find(|method| {
                matches!(method.id, S4MethodId::Registration(_))
                    && method.supplied_signature[0].class
                        == DispatchClass::Formal("character".into())
            })
            .unwrap();
        assert_eq!(
            method.generics,
            [generic_id("g", ".GlobalEnv")],
            "{installation}: {method:?}"
        );
        assert!(method.dispatch[0].complete, "{installation}: {method:?}");
        assert_eq!(
            method.dispatch[0].arguments[0].argument.as_deref(),
            Some("x")
        );
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        let call = calls.last().unwrap();
        assert_eq!(call.generics, method.generics, "{installation}: {call:?}");
        assert_eq!(
            call.exact_methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
    }
}

#[test]
fn generic_installations_do_not_override_local_shadows() {
    for closure in [
        "function(g) { methods::selectMethod(g, 'numeric', optional = TRUE) }",
        "function() { g <- function(x) x; methods::selectMethod(g, 'numeric', optional = TRUE) }",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x) standardGeneric('g'))\n\
            f <- {closure}\n"
        );
        let index = index(&source);
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        assert!(
            calls.last().unwrap().generics.is_empty(),
            "{source}: {calls:?}"
        );
    }
}

#[test]
fn string_named_groups_survive_ordinary_bindings() {
    let source = "Math <- function(y) y\n\
        methods::setMethod('Math', 'numeric', function(x) x)\n\
        methods::selectMethod('Math', 'numeric', useInherited = FALSE)\n";
    let index = index(source);
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| matches!(method.id, S4MethodId::Registration(_)))
        .unwrap();
    assert_eq!(method.generics, [generic_id("Math", "base")]);
    assert!(method.dispatch[0].complete, "{method:?}");
    assert_eq!(
        method.dispatch[0].arguments[0].argument.as_deref(),
        Some("x")
    );
    assert!(
        !index
            .s4
            .generics
            .iter()
            .any(|generic| generic.id == generic_id("Math", ".GlobalEnv"))
    );
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert_eq!(
        calls[0].exact_methods.as_slice(),
        std::slice::from_ref(&method.id)
    );
}

#[test]
fn registering_a_group_does_not_promote_an_ordinary_same_named_alias() {
    let source = "Math <- function(y) y\n\
        methods::setMethod('Math', 'numeric', function(x) x)\n\
        alias <- Math\n\
        methods::selectMethod(alias, 'numeric', useInherited = FALSE, optional = TRUE)\n";
    let index = index(source);
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert!(calls[0].generics.is_empty(), "{calls:?}");
    assert!(calls[0].exact_methods.is_empty(), "{calls:?}");
}

#[test]
fn sibling_rebinding_retires_the_preceding_generic() {
    for replacement in ["function(y) y", "NULL", "unknown"] {
        let source = "methods::setMethod('g', 'numeric', function(y) y)\n";
        for replacement_file in ["b.R", "z.R"] {
            let index = collated_package(&[
                (
                    "a.R",
                    "methods::setGeneric('g', function(x) standardGeneric('g'))\n",
                ),
                (replacement_file, &format!("g <- {replacement}\n")),
                ("c.R", source),
            ]);
            let method = index
                .s4
                .methods
                .iter()
                .find(|method| matches!(method.id, S4MethodId::Registration(_)))
                .unwrap();
            if replacement.starts_with("function") {
                assert_eq!(method.generics, [generic_id("g", "pkg")]);
                assert_eq!(method.dispatch.len(), 1, "{method:?}");
                assert!(
                    method.dispatch[0].complete && method.dispatch[0].valid,
                    "{method:?}"
                );
                assert_eq!(
                    method.dispatch[0].arguments[0].argument.as_deref(),
                    Some("y")
                );
                let promoted = index
                    .s4
                    .generics
                    .iter()
                    .find(|generic| {
                        generic
                            .declaration
                            .as_ref()
                            .is_some_and(|id| id.path == Path::new("/pkg/R/c.R"))
                    })
                    .unwrap();
                assert_eq!(promoted.signature.as_ref().unwrap().parameters[0].name, "y");
            } else {
                assert!(method.generics.is_empty(), "{replacement}: {method:?}");
                assert!(method.uncertainty.contains(&OopUncertainty::UnprovenMethod));
            }
        }
    }
}

#[test]
fn imported_formal_defaults_compare_expressions() {
    for (original, replacement, expected_package) in [
        ("x = \"a\"", "x = 'a'", "one"),
        (
            "x = c(a = \"a\")",
            "x = c( a = # same expression\n'a' )",
            "one",
        ),
        ("x = \"a\"", "x = 'b'", "two"),
        ("x = \"a\"", "x", "two"),
        ("x", "x = 'a'", "two"),
    ] {
        for generic in [false, true] {
            let one = if generic {
                format!("methods::setGeneric('g', function({original}) standardGeneric('g'))\n")
            } else {
                format!("g <- function({original}) x\n")
            };
            let two = format!(
                "methods::setGeneric('g', function({replacement}) standardGeneric('g'))\nmethods::setMethod('g', 'numeric', function(x) x)\n"
            );
            let index = packages(&one, &two, "importFrom(one, g)\n");
            let method = index
                .s4
                .methods
                .iter()
                .find(|method| matches!(method.id, S4MethodId::Registration(_)))
                .unwrap();
            assert_eq!(
                method.generics,
                [generic_id("g", expected_package)],
                "{one}: {two}"
            );
            assert!(
                method.dispatch[0].complete && method.dispatch[0].valid,
                "{method:?}"
            );
        }
    }
}

#[test]
fn imported_generics_allow_only_the_originals_extra_trailing_dots() {
    for (original, replacement, expected_package) in [
        ("x, ...", "x", "one"),
        ("x = 1, ...", "x = 2", "one"),
        ("x", "x, ...", "two"),
        ("..., x", "x", "two"),
        ("x, y, ...", "x", "two"),
    ] {
        for generic in [false, true] {
            let one = if generic {
                format!("methods::setGeneric('g', function({original}) standardGeneric('g'))\n")
            } else {
                format!("g <- function({original}) x\n")
            };
            let two =
                format!("methods::setGeneric('g', function({replacement}) standardGeneric('g'))\n");
            let index = packages(&one, &two, "importFrom(one, g)\n");
            let installed = index
                .s4
                .generics
                .iter()
                .find(|generic| {
                    generic
                        .declaration
                        .as_ref()
                        .is_some_and(|id| id.path == Path::new("/two/R/b.R"))
                })
                .unwrap();
            assert_eq!(
                installed.id,
                generic_id("g", expected_package),
                "{one}: {two}"
            );
            if expected_package == "one" {
                assert_eq!(
                    installed
                        .signature
                        .as_ref()
                        .unwrap()
                        .parameters
                        .last()
                        .unwrap()
                        .name,
                    "...",
                    "{one}: {installed:?}"
                );
                if generic {
                    let original = index
                        .s4
                        .generics
                        .iter()
                        .find(|generic| {
                            generic
                                .declaration
                                .as_ref()
                                .is_some_and(|id| id.path == Path::new("/one/R/a.R"))
                        })
                        .unwrap();
                    assert_eq!(installed.signature, original.signature);
                    assert_eq!(installed.default, original.default);
                }
            }
        }
    }
}

#[test]
fn sibling_methods_follow_aliases_to_the_captured_closure() {
    for aliases in ["alias <- impl\n", "first <- impl\nalias <- first\n"] {
        let source = format!(
            "methods::setGeneric('g', function(x) standardGeneric('g'))\nimpl <- function(x) methods::callNextMethod()\n{aliases}impl <- function(x) methods::callNextMethod()\n"
        );
        let index = collated_package(&[
            ("a.R", &source),
            ("b.R", "methods::setMethod('g', 'numeric', alias)\n"),
        ]);
        let method = index
            .s4
            .methods
            .iter()
            .find(|method| matches!(method.id, S4MethodId::Registration(_)))
            .unwrap();
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/pkg/R/a.R"), &parsed.cst, &model);
        let calls: Vec<_> = calls
            .iter()
            .filter(|call| call.kind == S4CallKind::NextMethod)
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].methods.as_slice(),
            std::slice::from_ref(&method.id),
            "{aliases}: {calls:?}"
        );
        assert_eq!(calls[0].generics, [generic_id("g", "pkg")]);
        assert!(calls[0].uncertainty.is_empty(), "{:?}", calls[0]);
        assert!(calls[1].methods.is_empty(), "{:?}", calls[1]);
        assert!(
            calls[1]
                .uncertainty
                .contains(&S4CallUncertainty::InvalidFrame)
        );
    }
}

#[test]
fn a_default_preserves_the_existing_generic_skeleton() {
    for definition in ["", ", def = NULL"] {
        for existing in [
            "g <- function(x, y = 1) x\n",
            "methods::setGeneric('g', function(x, y = 1) standardGeneric('g'))\n",
        ] {
            let declaration = format!(
                "methods::setGeneric('g'{definition}, useAsDefault = function(x, y = 2) 0)\n"
            );
            for sibling in [false, true] {
                let index = if sibling {
                    collated_package(&[("a.R", existing), ("b.R", &declaration)])
                } else {
                    index(&format!("{existing}{declaration}"))
                };
                let installed = index
                    .s4
                    .generics
                    .iter()
                    .rfind(|generic| generic.declaration.is_some())
                    .unwrap();
                assert_eq!(
                    installed.signature.as_ref().unwrap().parameters[1]
                        .default
                        .as_deref(),
                    Some("1"),
                    "{existing}: {declaration}: {installed:?}"
                );
                assert_eq!(
                    installed.default.signature.as_ref().unwrap().parameters[1]
                        .default
                        .as_deref(),
                    Some("2")
                );
                assert_eq!(installed.default.kind, S4DefaultKind::Explicit);
            }
        }
    }
}

#[test]
fn standard_redefinitions_recover_existing_generic_defaults() {
    for original in [
        "methods::setGeneric('g', function(x) x + 10)\n",
        "methods::setGeneric('g', function(x) standardGeneric('g'), useAsDefault = function(x) x + 10)\n",
        "g <- function(x) x + 10\nmethods::setGeneric('g')\n",
    ] {
        for (definition, options, retains_default) in [
            ("function(x) standardGeneric('g')", "", true),
            ("function(x = 1) standardGeneric('g')", "", true),
            ("function(y) standardGeneric('g')", "", false),
            (
                "function(x) standardGeneric('g')",
                ", useAsDefault = FALSE",
                false,
            ),
        ] {
            let replacement = format!("methods::setGeneric('g', {definition}{options})\n");
            for sibling in [false, true] {
                let source = if sibling {
                    replacement.clone()
                } else {
                    format!("{original}{replacement}")
                };
                let index = if sibling {
                    collated_package(&[("a.R", original), ("b.R", &source)])
                } else {
                    index(&source)
                };
                let generics: Vec<_> = index
                    .s4
                    .generics
                    .iter()
                    .filter(|generic| generic.declaration.is_some())
                    .collect();
                let [previous, installed] = generics.as_slice() else {
                    panic!("{generics:?}");
                };
                assert_eq!(
                    installed.default.kind != S4DefaultKind::None,
                    retains_default,
                    "{original}: {replacement}: {installed:?}"
                );
                let id = S4MethodId::Default(installed.declaration.clone().unwrap());
                let method = index.s4.methods.iter().find(|method| method.id == id);
                if retains_default {
                    assert_eq!(installed.default.signature, previous.default.signature);
                    assert_eq!(
                        installed.default.implementation,
                        previous.default.implementation
                    );
                    let method = method.unwrap();
                    assert_eq!(method.implementation, previous.default.implementation);
                    assert_eq!(method.signature, previous.default.signature);
                    assert_eq!(method.dispatch[0].object, installed.object);
                } else {
                    assert!(method.is_none(), "{method:?}");
                }
            }
        }
    }
}

#[test]
fn qualified_generic_bindings_are_retired_after_later_writes() {
    let source = "methods::setMethod(one::g, 'numeric', function(x) x)\nmethods::selectMethod(one::g, 'numeric', useInherited = FALSE)\n";
    for replacement in ["function(x) x", "NULL", "unknown"] {
        let index = packages(
            &format!(
                "methods::setGeneric('g', function(x) standardGeneric('g'))\ng <- {replacement}\n"
            ),
            source,
            "",
        );
        let method = index
            .s4
            .methods
            .iter()
            .find(|method| matches!(method.id, S4MethodId::Registration(_)))
            .unwrap();
        assert!(method.generics.is_empty(), "{replacement}: {method:?}");
        assert!(method.uncertainty.contains(&OopUncertainty::UnprovenMethod));
        let parsed = parse(source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/two/R/b.R"), &parsed.cst, &model);
        assert!(calls[0].generics.is_empty(), "{replacement}: {calls:?}");
        assert!(calls[0].exact_methods.is_empty());
    }
}

#[test]
fn method_exports_follow_the_final_binding_across_package_files() {
    let source = "methods::selectMethod(two::g, 'numeric')\n";
    for replacement in ["", "g <- function(y) y\n", "g <- NULL\n"] {
        let index = package_files(
            source,
            &[
                ("a.R", "g <- function(x) x\n"),
                ("b.R", "methods::setMethod('g', 'numeric', function(x) x)\n"),
                ("c.R", replacement),
            ],
            "exportMethods(g)\n",
        );
        let parsed = parse(source);
        let calls = index.s4_dispatch_calls(
            Path::new("/one/R/a.R"),
            &parsed.cst,
            &SemanticModel::build(&parsed.cst),
        );
        assert_eq!(
            calls[0].generics.is_empty(),
            !replacement.is_empty(),
            "{replacement}: {calls:?}"
        );
    }
}

#[test]
fn imported_generic_compatibility_checks_class_and_body() {
    for (original, definition, package) in [
        (
            "function(x) standardGeneric('g')",
            "function(x) { message('new'); standardGeneric('g') }",
            "two",
        ),
        (
            "function(x) { message('old'); standardGeneric('g') }",
            "function(x) { message('new'); standardGeneric('g') }",
            "two",
        ),
        (
            "function(x) { message('old'); standardGeneric('g') }",
            "function(x) standardGeneric('g')",
            "two",
        ),
        (
            "function(x) standardGeneric('g')",
            "function(x) standardGeneric(\"g\")",
            "one",
        ),
        (
            "function(x) { message('same'); standardGeneric('g') }",
            "function(x) {\n# same body\n#' same prose\nmessage(\"same\")\nstandardGeneric(\"g\")\n}",
            "one",
        ),
    ] {
        let source = format!(
            "methods::setGeneric('g', {definition})\nmethods::setMethod('g', 'numeric', function(x) x)\n"
        );
        let index = packages(
            &format!("methods::setGeneric('g', {original})\n"),
            &source,
            "importFrom(one, g)\n",
        );
        let method = index
            .s4
            .methods
            .iter()
            .find(|method| matches!(method.id, S4MethodId::Registration(_)))
            .unwrap();
        assert_eq!(
            method.generics,
            [generic_id("g", package)],
            "{original}: {definition}"
        );
    }
}

#[test]
fn unproven_body_equivalence_does_not_choose_an_imported_identity() {
    for (original, replacement) in [("1", "1.0"), ("NULL", "`NULL`")] {
        let index = packages(
            &format!(
                "methods::setGeneric('g', function(x) {{ message({original}); standardGeneric('g') }})\n"
            ),
            &format!(
                "methods::setGeneric('g', function(x) {{ message({replacement}); standardGeneric('g') }})\n"
            ),
            "importFrom(one, g)\n",
        );
        let generic = index
            .s4
            .generics
            .iter()
            .find(|generic| {
                generic
                    .declaration
                    .as_ref()
                    .is_some_and(|id| id.path == Path::new("/two/R/b.R"))
            })
            .unwrap();
        assert!(
            matches!(generic.id, S4GenericId::Declaration(_)),
            "{original}: {replacement}: {generic:?}"
        );
        assert!(
            generic
                .uncertainty
                .contains(&OopUncertainty::UnresolvedImplementation)
        );
    }
}

#[test]
fn compatible_imported_definitions_reuse_the_original_default() {
    for default in ["", ", useAsDefault = function(x) 'old'"] {
        for replacement in ["function(x) 'new'", "FALSE"] {
            let index = packages(
                &format!("methods::setGeneric('g', function(x) standardGeneric('g'){default})\n"),
                &format!(
                    "methods::setGeneric('g', function(x) standardGeneric('g'), useAsDefault = {replacement})\n"
                ),
                "importFrom(one, g)\n",
            );
            let generics: Vec<_> = index
                .s4
                .generics
                .iter()
                .filter(|generic| generic.declaration.is_some())
                .collect();
            assert_eq!(generics.len(), 2);
            assert_eq!(generics[0].id, generics[1].id);
            assert_eq!(
                generics[0].default, generics[1].default,
                "{default}: {replacement}"
            );
            let defaults: Vec<_> = index
                .s4
                .methods
                .iter()
                .filter(|method| matches!(method.id, S4MethodId::Default(_)))
                .collect();
            assert_eq!(
                defaults.len(),
                usize::from(!default.is_empty()),
                "{default}: {replacement}"
            );
            if let Some(method) = defaults.first() {
                assert_eq!(
                    method.id,
                    S4MethodId::Default(generics[0].declaration.clone().unwrap())
                );
            }
        }
    }
}

#[test]
fn local_function_promotions_shadow_imported_names() {
    let source = "g <- function(x) x\nmethods::setMethod('g', 'numeric', function(x) x)\nmethods::selectMethod('g', 'numeric', useInherited = FALSE)\n";
    let index = packages("g <- function(x) x\n", source, "importFrom(one, g)\n");
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| matches!(method.id, S4MethodId::Registration(_)))
        .unwrap();
    assert_eq!(method.generics, [generic_id("g", "two")]);
    assert!(!method.uncertainty.contains(&OopUncertainty::UnprovenMethod));
    assert!(method.dispatch[0].complete);
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/two/R/b.R"), &parsed.cst, &model);
    assert_eq!(
        calls[0].exact_methods.as_slice(),
        std::slice::from_ref(&method.id)
    );
}

#[test]
fn function_rebinding_reinstalls_an_implicit_generic() {
    let source = "g <- function(x) x\nmethods::setMethod('g', 'numeric', function(x) x)\ng <- function(y) y\nmethods::setMethod('g', 'character', function(y) y)\nmethods::selectMethod('g', 'character', useInherited = FALSE)\n";
    let index = index(source);
    let generics: Vec<_> = index
        .s4
        .generics
        .iter()
        .filter(|generic| generic.declaration.is_some())
        .collect();
    assert_eq!(generics.len(), 2, "{generics:?}");
    let methods: Vec<_> = index
        .s4
        .methods
        .iter()
        .filter(|method| matches!(method.id, S4MethodId::Registration(_)))
        .collect();
    for ((generic, method), argument) in generics.iter().zip(&methods).zip(["x", "y"]) {
        assert_eq!(
            generic.dispatch_arguments,
            [ClassName::Known(argument.into())]
        );
        assert_eq!(method.dispatch.len(), 1, "{method:?}");
        assert_eq!(method.dispatch[0].declaration, generic.declaration);
        assert_eq!(
            method.dispatch[0].arguments[0].argument.as_deref(),
            Some(argument)
        );
        assert!(method.dispatch[0].complete, "{method:?}");
    }
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert_eq!(
        calls[0].exact_methods.as_slice(),
        std::slice::from_ref(&methods[1].id)
    );
}

#[test]
fn dynamic_method_signatures_do_not_taint_promoted_generics() {
    let source = "g <- function(x) x\nclasses <- unknown\nmethods::setMethod('g', classes, function(x) x)\nmethods::setMethod('g', 'character', function(x) x)\nmethods::selectMethod('g', 'character', useInherited = FALSE)\n";
    let index = index(source);
    let generic = index
        .s4
        .generics
        .iter()
        .find(|generic| generic.declaration.is_some())
        .unwrap();
    assert_eq!(generic.dispatch_arguments, [ClassName::Known("x".into())]);
    assert!(
        !generic
            .uncertainty
            .contains(&OopUncertainty::DynamicSignature)
    );
    let methods: Vec<_> = index
        .s4
        .methods
        .iter()
        .filter(|method| matches!(method.id, S4MethodId::Registration(_)))
        .collect();
    assert!(
        methods[0]
            .uncertainty
            .contains(&OopUncertainty::DynamicSignature)
    );
    assert!(!methods[0].dispatch[0].complete);
    assert!(methods[1].dispatch[0].complete);
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert_eq!(
        calls[0].exact_methods.as_slice(),
        std::slice::from_ref(&methods[1].id)
    );
}

#[test]
fn generic_ownership_follows_the_resolved_function() {
    for (source, package) in [
        ("methods::setGeneric('mean')\n", "base"),
        (
            "mean <- function(x) x\nmethods::setGeneric('mean')\n",
            ".GlobalEnv",
        ),
        (
            "methods::setGeneric('mean')\nmean <- function(x) x\n",
            "base",
        ),
    ] {
        let index = index(source);
        let generic = index
            .s4
            .generics
            .iter()
            .find(|g| g.declaration.is_some())
            .unwrap();
        assert_eq!(generic.id, generic_id("mean", package), "{source}");
    }

    for declaration in [
        "methods::setGeneric('g')",
        "methods::setGeneric('g', function(x) standardGeneric('g'))",
    ] {
        let index = packages(
            "g <- function(x) x\n",
            &format!("g <- function(x) x\n{declaration}\n"),
            "importFrom(one, g)\n",
        );
        let generic = index
            .s4
            .generics
            .iter()
            .find(|g| g.declaration.is_some())
            .unwrap();
        assert_eq!(
            generic.id,
            generic_id("g", "two"),
            "{declaration}: {generic:?}"
        );
    }

    let index = packages(
        "g <- unresolved_function\n",
        "methods::setGeneric('g', function(x) standardGeneric('g'))\n",
        "importFrom(one, g)\n",
    );
    let generic = index
        .s4
        .generics
        .iter()
        .find(|g| g.declaration.is_some())
        .unwrap();
    assert!(
        matches!(generic.id, S4GenericId::Declaration(_)),
        "{generic:?}"
    );
}

#[test]
fn external_generic_definitions_require_a_compatible_skeleton() {
    for definition in [
        "function(x, ...) standardGeneric('mean')",
        "function(x, y) standardGeneric('mean')",
    ] {
        for package in ["", ", package = 'base'"] {
            let source = format!("methods::setGeneric('mean', {definition}{package})\n");
            let index = index(&source);
            let generic = index
                .s4
                .generics
                .iter()
                .find(|g| g.declaration.is_some())
                .unwrap();
            assert!(
                matches!(generic.id, S4GenericId::Declaration(_)),
                "{generic:?}"
            );
        }
    }
}

#[test]
fn indexed_external_generic_definitions_check_formals_dispatch_and_groups() {
    use arity::rindex::provider::IndexedProvider;
    use arity::rindex::schema::{Formal, PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};

    for (definition, options, expected_package, default) in [
        ("function(x, ...) standardGeneric('mean')", "", "base", None),
        ("function(x) standardGeneric('mean')", "", "base", None),
        (
            "function(x, y) standardGeneric('mean')",
            "",
            ".GlobalEnv",
            None,
        ),
        (
            "function(x = 1, ...) standardGeneric('mean')",
            "",
            ".GlobalEnv",
            None,
        ),
        (
            "function(x, ...) standardGeneric('mean')",
            ", signature = '...'",
            ".GlobalEnv",
            None,
        ),
        (
            "function(x, ...) standardGeneric('mean')",
            ", group = 'Math'",
            ".GlobalEnv",
            None,
        ),
        (
            "function(x = 'a', ...) standardGeneric('mean')",
            "",
            "base",
            Some("\"a\""),
        ),
        (
            "function(x = c(a = 'a'), ...) standardGeneric('mean')",
            "",
            "base",
            Some("c( a = \"a\" )"),
        ),
        (
            "function(x = 'b', ...) standardGeneric('mean')",
            "",
            ".GlobalEnv",
            Some("\"a\""),
        ),
        (
            "function(x = 2) standardGeneric('mean')",
            "",
            "base",
            Some("1"),
        ),
    ] {
        let mut db = IncrementalDatabase::default();
        db.set_library_index(IndexedProvider::from_indices([PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "base".into(),
            version: "4.6.1".into(),
            lib_path: String::new(),
            title: None,
            r_version: None,
            harvested_at: 0,
            attaches: Vec::new(),
            symbols: [("mean", true), ("internalMean", false)]
                .map(|(name, exported)| SymbolEntry {
                    name: name.into(),
                    kind: SymbolKind::Function,
                    exported,
                    formals: Some(
                        ["x", "..."]
                            .map(|name| Formal {
                                name: name.into(),
                                default: default.filter(|_| name == "x").map(str::to_owned),
                            })
                            .into(),
                    ),
                    help: None,
                })
                .into(),
        }]));
        for (name, package) in [
            ("mean", ""),
            ("mean", ", package = 'base'"),
            ("internalMean", ", package = 'base'"),
        ] {
            let definition = definition.replace("'mean'", &format!("'{name}'"));
            let source = format!(
                "methods::setGeneric('{name}', {definition}{options}{package})\nmethods::setMethod('{name}', 'numeric', {definition})\n"
            );
            let parsed = parse(&source);
            let model = SemanticModel::build(&parsed.cst);
            let index = db.snapshot().class_index_in(
                Path::new("/s4.R"),
                &file_class_defs(&model, &parsed.cst),
                &parsed.cst,
                &model,
            );
            let generic = index
                .s4
                .generics
                .iter()
                .find(|g| g.declaration.is_some())
                .unwrap();
            assert_eq!(generic.id, generic_id(name, expected_package), "{source}");
            let method = index
                .s4
                .methods
                .iter()
                .find(|method| matches!(method.id, S4MethodId::Registration(_)))
                .unwrap();
            assert_eq!(
                method.generics,
                [generic_id(name, expected_package)],
                "{source}"
            );
            if expected_package == "base" {
                assert_eq!(
                    generic
                        .signature
                        .as_ref()
                        .unwrap()
                        .parameters
                        .last()
                        .unwrap()
                        .name,
                    "...",
                    "{source}: {generic:?}"
                );
                assert_eq!(
                    generic.signature.as_ref().unwrap().parameters[0]
                        .default
                        .as_deref(),
                    default
                );
            }
        }
    }
}

#[test]
fn explicit_packages_check_workspace_definitions_before_choosing_an_owner() {
    for (original, compatible_package) in [
        ("g <- function(x) x + 10\n", "one"),
        ("methods::setGeneric('g', function(x) x + 10)\n", "one"),
        (
            "methods::setGeneric('g', function(x) x + 10, package = 'base')\n",
            "two",
        ),
    ] {
        for (definition, package) in [
            ("function(x) standardGeneric('g')", compatible_package),
            ("function(x, y) standardGeneric('g')", "two"),
        ] {
            for imports in ["importFrom(one, g)\n", ""] {
                let source = format!(
                    "methods::setGeneric('g', {definition}, package = 'one')\nmethods::setMethod('g', 'numeric', {definition})\n"
                );
                let index = packages(original, &source, imports);
                let generic = index
                    .s4
                    .generics
                    .iter()
                    .find(|generic| {
                        generic
                            .declaration
                            .as_ref()
                            .is_some_and(|id| id.path == Path::new("/two/R/b.R"))
                    })
                    .unwrap();
                assert_eq!(generic.id, generic_id("g", package), "{original}: {source}");
                if package == "one" {
                    assert_ne!(generic.default.kind, S4DefaultKind::None, "{generic:?}");
                    assert!(generic.default.implementation.is_some());
                }
                let method = index
                    .s4
                    .methods
                    .iter()
                    .find(|method| matches!(method.id, S4MethodId::Registration(_)))
                    .unwrap();
                assert_eq!(method.generics, [generic_id("g", package)], "{source}");
            }
        }
    }
}

#[test]
fn generic_redeclarations_process_explicit_options() {
    for options in [
        "signature = 'x', useAsDefault = FALSE",
        "def = NULL, signature = 'x', useAsDefault = FALSE",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y', useAsDefault = function(x, y) y)\nmethods::setGeneric('g', {options})\nmethods::setMethod('g', 'numeric', function(x, y) x)\nmethods::selectMethod('g', 'numeric', useInherited = FALSE)\n"
        );
        let index = index(&source);
        let generic = index
            .s4
            .generics
            .iter()
            .rfind(|g| g.declaration.is_some())
            .unwrap();
        assert_eq!(
            generic.dispatch_arguments,
            [ClassName::Known("x".into())],
            "{options}"
        );
        assert_eq!(generic.default.kind, S4DefaultKind::None, "{options}");
        assert!(generic.signature.is_some(), "{generic:?}");
        assert!(generic.uncertainty.is_empty(), "{generic:?}");
        let method = index
            .s4
            .methods
            .iter()
            .find(|m| matches!(m.id, S4MethodId::Registration(_)))
            .unwrap();
        let dispatch = method
            .dispatch
            .iter()
            .find(|dispatch| dispatch.declaration == generic.declaration)
            .unwrap();
        assert_eq!(dispatch.arguments[0].argument.as_deref(), Some("x"));
        assert!(dispatch.complete);
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        let selection = calls
            .last()
            .unwrap()
            .dispatch
            .iter()
            .find(|dispatch| dispatch.declaration == generic.declaration)
            .unwrap();
        assert_eq!(selection.arguments[0].argument.as_deref(), Some("x"));
        assert_eq!(
            calls.last().unwrap().exact_methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
    }

    let index = packages(
        "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y', useAsDefault = function(x, y) y)\n",
        "methods::setGeneric('g', signature = 'x', useAsDefault = FALSE)\n",
        "importFrom(one, g)\n",
    );
    let generic = index
        .s4
        .generics
        .iter()
        .find(|g| {
            g.declaration
                .as_ref()
                .is_some_and(|id| id.path == Path::new("/two/R/b.R"))
        })
        .unwrap();
    assert_eq!(generic.id, generic_id("g", "two"));
    assert_eq!(generic.dispatch_arguments, [ClassName::Known("x".into())]);
    assert_eq!(generic.default.kind, S4DefaultKind::None);
}

#[test]
fn implicit_generics_capture_the_function_at_registration() {
    for replacement in ["function(a, b) a", "NULL"] {
        let source = format!(
            "g <- function(x) x\nmethods::setMethod('g', 'numeric', function(x) x)\ng <- {replacement}\n"
        );
        let index = index(&source);
        let generic = index
            .s4
            .generics
            .iter()
            .find(|g| g.id == generic_id("g", ".GlobalEnv"))
            .unwrap();
        assert_eq!(
            generic.dispatch_arguments,
            [ClassName::Known("x".into())],
            "{source}"
        );
        assert_eq!(
            generic.default.signature.as_ref().unwrap().parameters.len(),
            1
        );
        let method = index
            .s4
            .methods
            .iter()
            .find(|m| matches!(m.id, S4MethodId::Registration(_)))
            .unwrap();
        assert_eq!(
            method.generics.as_slice(),
            std::slice::from_ref(&generic.id)
        );
        assert!(method.dispatch[0].complete);
        assert_eq!(
            method.dispatch[0].arguments[0].argument.as_deref(),
            Some("x")
        );
    }
}

#[test]
fn qualified_references_resolve_implicitly_promoted_generics() {
    let promoted = "g <- function(x) x\nmethods::setMethod('g', 'numeric', function(x) x)\n";
    for (owner, consumer_path) in [("one", "/two/R/b.R"), ("two", "/one/R/a.R")] {
        let source = format!(
            "methods::setMethod({owner}::g, 'character', function(x) x)\nmethods::selectMethod({owner}::g, 'character', useInherited = FALSE)\n"
        );
        let (one, two) = if owner == "one" {
            (promoted, source.as_str())
        } else {
            (source.as_str(), promoted)
        };
        let index = packages(one, two, "export(g)\n");
        let method = index
            .s4
            .methods
            .iter()
            .find(|m| {
                matches!(m.id, S4MethodId::Registration(_))
                    && m.owner.path == Path::new(consumer_path)
            })
            .unwrap();
        assert_eq!(method.generics, [generic_id("g", owner)]);
        assert!(method.dispatch[0].complete);
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new(consumer_path), &parsed.cst, &model);
        assert_eq!(calls.last().unwrap().generics, method.generics);
        assert_eq!(
            calls.last().unwrap().exact_methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
    }
}

#[test]
fn omitted_method_formals_cannot_claim_an_exact_any_signature() {
    for (implementation, replacement) in [
        ("function(y) y", ""),
        ("impl", "impl <- function(x, y) y\n"),
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x, y) standardGeneric('g'))\nimpl <- function(y) y\nmethods::setMethod('g', c('ANY', 'numeric'), {implementation})\n{replacement}methods::selectMethod('g', c('ANY', 'numeric'), useInherited = FALSE)\n"
        );
        let index = index(&source);
        let method = index
            .s4
            .methods
            .iter()
            .find(|m| matches!(m.id, S4MethodId::Registration(_)))
            .unwrap();
        assert!(!method.dispatch[0].complete);
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        assert!(
            calls.last().unwrap().exact_methods.is_empty(),
            "{implementation}: {calls:?}"
        );
    }
}

#[test]
fn unresolved_qualified_objects_do_not_fall_back_to_s4_groups() {
    for reference in ["one::Math", "unknown::Math", "object"] {
        let source = format!(
            "object <- unknown\nmethods::selectMethod({reference}, 'numeric', useInherited = FALSE)\n"
        );
        let index = packages(
            "Math <- function(x) x\nmethods::setMethod('Math', 'numeric', function(x) x)\n",
            &source,
            "",
        );
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/two/R/b.R"), &parsed.cst, &model);
        assert!(
            calls.last().unwrap().generics.is_empty(),
            "{reference}: {calls:?}"
        );
    }
}

#[test]
fn incompatible_imported_generic_definitions_get_a_new_owner() {
    for original in [
        "g <- function(x) x",
        "methods::setGeneric('g', function(x) standardGeneric('g'))",
    ] {
        for definition in [
            "function(x, y) standardGeneric('g')",
            "function(x = 1) standardGeneric('g')",
        ] {
            let one = format!("{original}\nmethods::setMethod('g', 'numeric', function(x) x)\n");
            let two = format!(
                "methods::setGeneric('g', {definition})\nmethods::setMethod('g', 'numeric', {definition})\nmethods::selectMethod('g', 'numeric', useInherited = FALSE)\n"
            );
            let index = packages(&one, &two, "importFrom(one, g)\n");
            let methods: Vec<_> = index
                .s4
                .methods
                .iter()
                .filter(|method| matches!(method.id, S4MethodId::Registration(_)))
                .collect();
            assert_eq!(methods.len(), 2);
            assert_eq!(methods[0].generics, [generic_id("g", "one")]);
            assert_eq!(
                methods[1].generics,
                [generic_id("g", "two")],
                "{original}: {definition}"
            );
            let parsed = parse(&two);
            let model = SemanticModel::build(&parsed.cst);
            let calls = index.s4_dispatch_calls(Path::new("/two/R/b.R"), &parsed.cst, &model);
            assert_eq!(
                calls.last().unwrap().exact_methods.as_slice(),
                std::slice::from_ref(&methods[1].id),
                "{original}: {definition}"
            );
        }
    }
}

#[test]
fn imported_generic_ownership_checks_selected_arguments_and_groups() {
    for (declaration, package) in [
        (
            "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y', group = 'Math')",
            "one",
        ),
        (
            "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'x', group = 'Math')",
            "two",
        ),
        (
            "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y')",
            "two",
        ),
    ] {
        let index = packages(
            "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y', group = 'Math')\n",
            &format!("{declaration}\n"),
            "importFrom(one, g)\n",
        );
        let generic = index
            .s4
            .generics
            .iter()
            .find(|generic| {
                generic
                    .declaration
                    .as_ref()
                    .is_some_and(|id| id.path == Path::new("/two/R/b.R"))
            })
            .unwrap();
        assert_eq!(generic.id, generic_id("g", package), "{declaration}");
    }
}

#[test]
fn named_implementations_are_captured_at_registration() {
    for replacement in ["function(x)", "function(x, y)"] {
        let source = format!(
            "methods::setGeneric('g', function(x) standardGeneric('g'))\nimpl <- function(x) methods::callNextMethod()\nmethods::setMethod('g', 'numeric', impl)\nimpl <- {replacement} methods::callNextMethod()\n"
        );
        let index = index(&source);
        let method = index
            .s4
            .methods
            .iter()
            .find(|method| matches!(method.id, S4MethodId::Registration(_)))
            .unwrap();
        assert_eq!(
            method
                .signature
                .as_ref()
                .unwrap()
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["x"],
            "{source}"
        );
        assert!(method.uncertainty.is_empty(), "{method:?}");
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        let calls: Vec<_> = calls
            .iter()
            .filter(|call| call.kind == S4CallKind::NextMethod)
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
        assert!(calls[0].uncertainty.is_empty(), "{:?}", calls[0]);
        assert!(calls[1].methods.is_empty(), "{:?}", calls[1]);
        assert!(
            calls[1]
                .uncertainty
                .contains(&S4CallUncertainty::InvalidFrame)
        );
    }
}

#[test]
fn explicit_dispatch_signatures_remove_dots_when_other_arguments_are_selected() {
    for signature in ["c('x', '...')", "c('...', 'x')", "'...'"] {
        let source = format!(
            "methods::setGeneric('g', function(x, ...) standardGeneric('g'), signature = {signature})\nmethods::setMethod('g', 'numeric', function(x, ...) x)\nmethods::selectMethod('g', 'numeric', useInherited = FALSE)\n"
        );
        let index = index(&source);
        let generic = index
            .s4
            .generics
            .iter()
            .find(|generic| generic.id == generic_id("g", ".GlobalEnv"))
            .unwrap();
        assert_eq!(
            generic.dispatch_arguments,
            [ClassName::Known(
                if signature == "'...'" { "..." } else { "x" }.into()
            )],
            "{signature}"
        );
        assert!(generic.uncertainty.is_empty(), "{generic:?}");
        let method = index.s4.methods_for_generic(&generic.id)[0];
        assert!(method.dispatch[0].valid && method.dispatch[0].complete);
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        assert_eq!(
            calls.last().unwrap().exact_methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
    }
}

#[test]
fn dots_must_be_a_formal_before_explicit_dispatch_is_normalized() {
    let index = index(
        "methods::setGeneric('g', function(x) standardGeneric('g'), signature = c('x', '...'))\nmethods::setMethod('g', 'numeric', function(x) x)\n",
    );
    let generic = index
        .s4
        .generics
        .iter()
        .find(|generic| generic.id == generic_id("g", ".GlobalEnv"))
        .unwrap();
    assert!(
        generic
            .uncertainty
            .contains(&OopUncertainty::InvalidArguments)
    );
    assert!(!index.s4.methods_for_generic(&generic.id)[0].dispatch[0].valid);
}

#[test]
fn a_null_definition_uses_the_default_or_existing_skeleton() {
    for (before, default, kind) in [
        (
            "",
            ", useAsDefault = function(x, y) x",
            S4DefaultKind::Explicit,
        ),
        ("g <- function(x, y) x\n", "", S4DefaultKind::Existing),
    ] {
        let source = format!(
            "{before}methods::setGeneric('g', def = NULL{default})\nmethods::setMethod('g', c('numeric', 'ANY'), function(x, y) x)\nmethods::selectMethod('g', c('numeric', 'ANY'), useInherited = FALSE)\n"
        );
        let index = index(&source);
        let generic = index
            .s4
            .generics
            .iter()
            .find(|generic| generic.id == generic_id("g", ".GlobalEnv"))
            .unwrap();
        assert_eq!(generic.default.kind, kind, "{source}");
        assert_eq!(
            generic.dispatch_arguments,
            [ClassName::Known("x".into()), ClassName::Known("y".into())],
            "{source}"
        );
        assert!(generic.uncertainty.is_empty(), "{generic:?}");
        let method = index
            .s4
            .methods_for_generic(&generic.id)
            .into_iter()
            .find(|method| matches!(method.id, S4MethodId::Registration(_)))
            .unwrap();
        assert!(method.dispatch[0].valid && method.dispatch[0].complete);
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        assert_eq!(
            calls.last().unwrap().exact_methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
    }
}

#[test]
fn a_sole_dots_formal_is_an_implicit_dispatch_argument() {
    for declaration in [
        "methods::setGeneric('g', function(...) standardGeneric('g'))\n",
        "g <- function(...) NULL\nmethods::setGeneric('g')\n",
        "g <- function(...) NULL\n",
    ] {
        let source = format!(
            "{declaration}methods::setMethod('g', 'numeric', function(...) 1)\nmethods::selectMethod('g', 'numeric', useInherited = FALSE)\n"
        );
        let index = index(&source);
        let generic = index
            .s4
            .generics
            .iter()
            .find(|g| g.id == generic_id("g", ".GlobalEnv"))
            .unwrap();
        assert_eq!(
            generic.dispatch_arguments,
            [ClassName::Known("...".into())],
            "{source}"
        );
        let method = index
            .s4
            .methods
            .iter()
            .find(|m| matches!(m.id, S4MethodId::Registration(_)))
            .unwrap();
        assert!(
            method.dispatch[0].valid && method.dispatch[0].complete,
            "{source}: {method:?}"
        );
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        assert_eq!(
            calls.last().unwrap().exact_methods.as_slice(),
            std::slice::from_ref(&method.id),
            "{source}"
        );
    }
}

#[test]
fn redeclaring_an_imported_generic_reuses_its_metadata() {
    let source = "methods::setGeneric('g')\nmethods::setMethod('g', 'numeric', function(x, y) y)\nmethods::selectMethod('g', 'numeric', useInherited = FALSE)\n";
    for default in ["", ", useAsDefault = function(x, y) y"] {
        let index = packages(
            &format!(
                "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y', group = 'Math'{default})\n"
            ),
            source,
            "importFrom(one, g)\n",
        );
        let generics: Vec<_> = index
            .s4
            .generics
            .iter()
            .filter(|g| g.id == generic_id("g", "one"))
            .collect();
        assert_eq!(generics.len(), 2, "{:?}", index.s4.generics);
        for generic in generics {
            assert_eq!(generic.dispatch_arguments, [ClassName::Known("y".into())]);
            assert_eq!(generic.groups, [ClassName::Known("Math".into())]);
            assert!(generic.uncertainty.is_empty(), "{generic:?}");
            assert_eq!(
                generic.default.kind,
                if default.is_empty() {
                    S4DefaultKind::None
                } else {
                    S4DefaultKind::Explicit
                }
            );
        }
        let method = index
            .s4
            .methods
            .iter()
            .find(|m| matches!(m.id, S4MethodId::Registration(_)))
            .unwrap();
        assert!(method.dispatch[0].complete, "{method:?}");
        assert_eq!(
            method.dispatch[0].arguments[0].argument.as_deref(),
            Some("y")
        );
        let parsed = parse(source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/two/R/b.R"), &parsed.cst, &model);
        assert_eq!(
            calls.last().unwrap().exact_methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
        assert_eq!(
            index
                .s4
                .methods
                .iter()
                .filter(|m| matches!(m.id, S4MethodId::Default(_)))
                .count(),
            usize::from(!default.is_empty())
        );
    }
}

#[test]
fn imported_implicit_generics_retain_metadata_without_a_new_signature() {
    for options in [
        "def = NULL",
        "signature = NULL",
        "useAsDefault = FALSE",
        "group = 'Math2'",
    ] {
        let index = packages(
            "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = 'y', group = 'Math', useAsDefault = function(x, y) y)\n",
            &format!("methods::setGeneric('g', {options})\n"),
            "importFrom(one, g)\n",
        );
        let generics: Vec<_> = index
            .s4
            .generics
            .iter()
            .filter(|g| g.declaration.is_some())
            .collect();
        assert_eq!(generics.len(), 2);
        let (original, imported) = (generics[0], generics[1]);
        assert_eq!(imported.id, original.id, "{options}");
        assert_eq!(
            imported.dispatch_arguments, original.dispatch_arguments,
            "{options}"
        );
        assert_eq!(imported.groups, original.groups, "{options}");
        assert_eq!(imported.default, original.default, "{options}");
    }
}

#[test]
fn qualified_references_resolve_promoted_generic_bindings() {
    let source = "methods::setMethod(one::g, 'numeric', function(x) x)\nmethods::selectMethod(one::g, 'numeric', useInherited = FALSE)\n";
    let index = packages("g <- function(x) x\nmethods::setGeneric('g')\n", source, "");
    let method = index
        .s4
        .methods
        .iter()
        .find(|m| matches!(m.id, S4MethodId::Registration(_)))
        .unwrap();
    assert_eq!(method.generics, [generic_id("g", "one")]);
    assert!(method.dispatch[0].complete);
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index.s4_dispatch_calls(Path::new("/two/R/b.R"), &parsed.cst, &model);
    assert_eq!(calls[0].generics, method.generics);
    assert_eq!(
        calls[0].exact_methods.as_slice(),
        std::slice::from_ref(&method.id)
    );
}

#[test]
fn previous_defaults_are_resolved_at_the_generic_declaration() {
    for (before, after, expected) in [
        ("", "g <- function(x) x\n", S4DefaultKind::None),
        (
            "g <- function(x) x\n",
            "g <- function(x, y) y\n",
            S4DefaultKind::Existing,
        ),
        ("g <- function(x) x\ng <- NULL\n", "", S4DefaultKind::None),
    ] {
        let source = format!(
            "{before}methods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::selectMethod('g', 'ANY', optional = TRUE, useInherited = FALSE)\n{after}"
        );
        let index = index(&source);
        let generic = index
            .s4
            .generics
            .iter()
            .find(|g| g.declaration.is_some())
            .unwrap();
        assert_eq!(generic.default.kind, expected, "{source}: {generic:?}");
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        assert_eq!(
            calls.last().unwrap().exact_methods.len(),
            usize::from(expected == S4DefaultKind::Existing),
            "{source}"
        );
    }
}

#[test]
fn generic_skeletons_do_not_acquire_a_separate_default_method_frame() {
    for source in [
        "methods::setGeneric('g', function(x) { methods::callNextMethod(); standardGeneric('g') }, useAsDefault = function(x) methods::callNextMethod())\n",
        "skeleton <- function(x) { methods::callNextMethod(); standardGeneric('g') }\nfallback <- function(x) methods::callNextMethod()\nmethods::setGeneric('g', skeleton, useAsDefault = fallback)\n",
    ] {
        let parsed = parse(source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index(source).s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        let calls: Vec<_> = calls
            .iter()
            .filter(|call| call.kind == S4CallKind::NextMethod)
            .collect();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].methods.is_empty(), "{source}: {:?}", calls[0]);
        assert!(
            calls[0]
                .uncertainty
                .contains(&S4CallUncertainty::InvalidFrame)
        );
        assert_eq!(calls[1].methods.len(), 1, "{source}: {:?}", calls[1]);
        assert!(calls[1].uncertainty.is_empty(), "{source}: {:?}", calls[1]);
    }
}

#[test]
fn selected_arguments_normalize_named_and_positional_signatures() {
    let index = index(
        "methods::setGeneric('g', function(x, y, z = 1, ...) standardGeneric('g'), signature = c('y', 'x'))\n\
         methods::setMethod('g', 'numeric', function(x, y, z = 1, ...) x)\n\
         methods::setMethod('g', methods::signature(x = 'numeric', y = 'missing'), function(x, y, z = 1, ...) x)\n\
         methods::setMethod('g', c('numeric', 'character'), function(x, y, z = 1, ...) x)\n",
    );
    let methods = index.s4.methods_for_generic(&generic_id("g", ".GlobalEnv"));
    assert_eq!(methods.len(), 3);
    for (method, classes) in methods.iter().zip([
        vec![DispatchClass::Formal("numeric".into()), DispatchClass::Any],
        vec![
            DispatchClass::Missing,
            DispatchClass::Formal("numeric".into()),
        ],
        vec![
            DispatchClass::Formal("numeric".into()),
            DispatchClass::Formal("character".into()),
        ],
    ]) {
        let [dispatch] = method.dispatch.as_slice() else {
            panic!("{method:?}");
        };
        assert!(dispatch.complete && dispatch.valid, "{dispatch:?}");
        assert_eq!(
            dispatch
                .arguments
                .iter()
                .map(|arg| arg.argument.as_deref())
                .collect::<Vec<_>>(),
            [Some("y"), Some("x")]
        );
        assert_eq!(
            dispatch
                .arguments
                .iter()
                .map(|arg| arg.class.clone())
                .collect::<Vec<_>>(),
            classes
        );
    }
}

#[test]
fn mixed_signatures_match_names_before_positions() {
    let index = index(
        "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = c('y', 'x'))\nmethods::setMethod('g', c('numeric', x = 'character'), function(x, y) x)\n",
    );
    let method = index.s4.methods_for_generic(&generic_id("g", ".GlobalEnv"))[0];
    assert!(method.dispatch[0].complete);
    assert_eq!(
        method.dispatch[0].arguments[0].class,
        DispatchClass::Formal("numeric".into())
    );
    assert_eq!(
        method.dispatch[0].arguments[1].class,
        DispatchClass::Formal("character".into())
    );
}

#[test]
fn registration_signatures_accept_unambiguous_partial_names() {
    for signature in [
        "methods::signature(obj = 'numeric', oth = 'character')",
        "c('character', obj = 'numeric')",
        "c(oth = 'character', object = 'numeric')",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(object, other) standardGeneric('g'))\n\
             methods::setMethod('g', {signature}, function(object, other) object)\n\
             methods::selectMethod('g', c('numeric', 'character'), useInherited = FALSE)\n"
        );
        let index = index(&source);
        let method = index.s4.methods_for_generic(&generic_id("g", ".GlobalEnv"))[0];
        let dispatch = &method.dispatch[0];
        assert!(
            dispatch.valid && dispatch.complete,
            "{signature}: {dispatch:?}"
        );
        assert_eq!(
            dispatch.arguments[0].class,
            DispatchClass::Formal("numeric".into())
        );
        assert_eq!(
            dispatch.arguments[1].class,
            DispatchClass::Formal("character".into())
        );
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        assert_eq!(
            calls.last().unwrap().exact_methods.as_slice(),
            std::slice::from_ref(&method.id)
        );
    }
    for signature in [
        "c(o = 'numeric')",
        "c(obj = 'numeric', object = 'character')",
        "c(obj = 'numeric', obje = 'character')",
    ] {
        let index = index(&format!(
            "methods::setGeneric('g', function(object, other) standardGeneric('g'))\n\
             methods::setMethod('g', {signature}, function(object, other) object)\n"
        ));
        let method = index.s4.methods_for_generic(&generic_id("g", ".GlobalEnv"))[0];
        assert!(
            !method.dispatch[0].valid && !method.dispatch[0].complete,
            "{signature}: {method:?}"
        );
    }
}

#[test]
fn select_method_ignores_signature_names() {
    for signature in [
        "methods::signature(y = 'numeric', x = 'character')",
        "c(y = 'numeric', x = 'character')",
        "c(unknown = 'numeric', unknown = 'character')",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x, y) standardGeneric('g'))\n\
             methods::setMethod('g', c('numeric', 'character'), function(x, y) x)\n\
             methods::setMethod('g', c('character', 'numeric'), function(x, y) y)\n\
             methods::selectMethod('g', {signature}, useInherited = FALSE)\n"
        );
        let index = index(&source);
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        let call = calls.last().unwrap();
        assert!(call.uncertainty.is_empty(), "{signature}: {call:?}");
        let method = index.s4.methods_for_generic(&generic_id("g", ".GlobalEnv"))[0];
        assert_eq!(call.dispatch, method.dispatch, "{signature}");
        assert_eq!(
            call.exact_methods.as_slice(),
            std::slice::from_ref(&method.id),
            "{signature}"
        );
    }
}

#[test]
fn generic_defaults_use_the_syntax_of_standard_generic_calls() {
    for (body, kind) in [
        ("base::standardGeneric('g')", S4DefaultKind::Definition),
        ("{ base::standardGeneric('g') }", S4DefaultKind::Definition),
        ("dispatch('g')", S4DefaultKind::Definition),
        ("standardGeneric('g')", S4DefaultKind::None),
        ("{ standardGeneric('g') }", S4DefaultKind::None),
        ("{ function() standardGeneric('g') }", S4DefaultKind::None),
    ] {
        let index = index(&format!(
            "dispatch <- base::standardGeneric\nmethods::setGeneric('g', function(x) {body})\n"
        ));
        let id = generic_id("g", ".GlobalEnv");
        let generic = index
            .s4
            .generics
            .iter()
            .find(|generic| generic.id == id)
            .unwrap();
        assert_eq!(generic.default.kind, kind, "{body}: {generic:?}");
        let defaults = index.s4.methods_for_generic(&id);
        if kind == S4DefaultKind::None {
            assert!(defaults.is_empty(), "{body}: {defaults:?}");
        } else {
            assert_eq!(defaults.len(), 1, "{body}: {defaults:?}");
            assert!(generic.default.signature.is_some(), "{body}: {generic:?}");
            assert!(
                generic.default.implementation.is_some(),
                "{body}: {generic:?}"
            );
        }
    }
}

#[test]
fn a_default_can_supply_the_generic_skeleton() {
    let index = index(
        "fallback <- function(x, y = 1) x\nmethods::setGeneric('g', useAsDefault = fallback)\n",
    );
    let generic = index
        .s4
        .generics
        .iter()
        .find(|generic| generic.id == generic_id("g", ".GlobalEnv"))
        .unwrap();
    assert_eq!(
        generic.dispatch_arguments,
        [ClassName::Known("x".into()), ClassName::Known("y".into())]
    );
    assert_eq!(generic.default.kind, S4DefaultKind::Explicit);
    assert_eq!(
        generic.signature.as_ref().unwrap().parameters[1]
            .default
            .as_deref(),
        Some("1")
    );
}

#[test]
fn set_method_can_promote_a_known_function_to_a_generic() {
    let index = index(
        "g <- function(x, y = 1) x\nmethods::setMethod('g', 'numeric', function(x, y = 1) x)\nmethods::setMethod('g', 'character', function(x, y = 1) x)\n",
    );
    let id = generic_id("g", ".GlobalEnv");
    let methods = index.s4.methods_for_generic(&id);
    assert_eq!(methods.len(), 3, "{methods:?}");
    assert!(matches!(methods[0].id, S4MethodId::Default(_)));
    assert!(methods.iter().all(|method| method.dispatch[0].complete));
    assert_eq!(
        methods[1].dispatch[0].arguments[0].class,
        DispatchClass::Formal("numeric".into())
    );
    assert_eq!(
        methods[2].dispatch[0].arguments[0].class,
        DispatchClass::Formal("character".into())
    );
}

#[test]
fn defaults_keep_their_implementation_and_generic_identity() {
    let index = index(
        "fallback <- function(x, y = 1) x\n\
         methods::setGeneric('from_def', function(x, y = 1) x)\n\
         methods::setGeneric('explicit', function(x, y = 1) standardGeneric('explicit'), useAsDefault = fallback)\n\
         methods::setGeneric('absent', function(x) standardGeneric('absent'))\n\
         existing <- function(x) x\nmethods::setGeneric('existing')\n\
         methods::setGeneric('disabled', function(x) x, useAsDefault = FALSE)\n",
    );
    for (name, kind) in [
        ("from_def", S4DefaultKind::Definition),
        ("explicit", S4DefaultKind::Explicit),
        ("existing", S4DefaultKind::Existing),
    ] {
        let id = generic_id(name, ".GlobalEnv");
        let generic = index
            .s4
            .generics
            .iter()
            .find(|generic| generic.id == id)
            .unwrap();
        assert_eq!(generic.default.kind, kind);
        assert!(generic.default.signature.is_some(), "{generic:?}");
        let defaults = index.s4.methods_for_generic(&id);
        let [default] = defaults.as_slice() else {
            panic!("{defaults:?}");
        };
        assert_eq!(
            default.id,
            S4MethodId::Default(generic.declaration.clone().unwrap())
        );
        assert!(
            default.dispatch[0]
                .arguments
                .iter()
                .all(|arg| arg.class == DispatchClass::Any)
        );
    }
    for name in ["absent", "disabled"] {
        assert!(
            index
                .s4
                .methods_for_generic(&generic_id(name, ".GlobalEnv"))
                .is_empty()
        );
    }
}

#[test]
fn unions_and_replacements_keep_their_full_signature() {
    let index = index(
        "methods::setClass('A')\nmethods::setClassUnion('Either', c('A', 'NULL'))\n\
         methods::setGeneric('value<-', function(object, value) standardGeneric('value<-'))\n\
         methods::setReplaceMethod('value', c('Either', 'missing'), function(object, value) object)\n",
    );
    let methods = index
        .s4
        .methods_for_generic(&generic_id("value<-", ".GlobalEnv"));
    assert_eq!(methods.len(), 1);
    assert_eq!(
        methods[0].dispatch[0].arguments[0].class,
        DispatchClass::Formal("Either".into())
    );
    assert_eq!(
        methods[0].dispatch[0].arguments[1].class,
        DispatchClass::Missing
    );
    assert!(index.declarations.values().any(|class| {
        class
            .declaration
            .s4
            .as_ref()
            .is_some_and(|s4| s4.definition == S4Definition::Union)
    }));
}

#[test]
fn s4_group_signatures_stay_distinct_from_s3_groups() {
    let index = index("methods::setMethod('Arith', c(e2 = 'numeric'), function(e1, e2) e2)\n");
    let id = generic_id("Arith", "base");
    let methods = index.s4.methods_for_generic(&id);
    assert_eq!(methods.len(), 1);
    let signature = &methods[0].dispatch[0];
    assert_eq!(signature.arguments[0].class, DispatchClass::Any);
    assert_eq!(
        signature.arguments[1].class,
        DispatchClass::Formal("numeric".into())
    );
    assert!(signature.complete);
    let generic = index
        .s4
        .generics
        .iter()
        .find(|generic| generic.id == id)
        .unwrap();
    assert_eq!(generic.groups, [ClassName::Known("Ops".into())]);
    assert!(generic.group);
}

#[test]
fn malformed_and_dynamic_signatures_preserve_uncertainty() {
    for signature in [
        "c(x = 'numeric', x = 'character')",
        "c(z = 'numeric')",
        "c('numeric', 'character', 'logical')",
        "computed()",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x, y) standardGeneric('g'))\nmethods::setMethod('g', {signature}, function(x, y) x)\n"
        );
        let index = index(&source);
        let methods = index.s4.methods_for_generic(&generic_id("g", ".GlobalEnv"));
        assert_eq!(methods.len(), 1, "{source}");
        assert!(
            !methods[0].dispatch[0].complete,
            "{source}: {:?}",
            methods[0]
        );
    }
}

#[test]
fn dispatch_calls_share_declarations_and_do_not_use_forwarded_classes() {
    let source = "methods::setGeneric('g', function(x, y) base::standardGeneric('g'), signature = c('y', 'x'))\n\
        methods::setMethod('g', methods::signature(x = 'numeric', y = 'missing'), function(x, y) { x <- 'changed'; methods::callNextMethod('replacement', y) })\n\
        methods::selectMethod('g', methods::signature(y = 'missing', x = 'numeric'), useInherited = FALSE)\n";
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let index = index(source);
    let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert_eq!(calls.len(), 3, "{calls:?}");
    let id = generic_id("g", ".GlobalEnv");
    for call in &calls {
        assert_eq!(
            call.generics.as_slice(),
            std::slice::from_ref(&id),
            "{call:?}"
        );
        assert!(call.uncertainty.is_empty(), "{call:?}");
    }
    assert_eq!(calls[0].kind, S4CallKind::StandardGeneric);
    assert_eq!(calls[1].kind, S4CallKind::NextMethod);
    assert_eq!(
        calls[1].methods,
        [index
            .s4
            .methods_for_generic(&id)
            .into_iter()
            .find(|method| matches!(method.id, S4MethodId::Registration(_)))
            .unwrap()
            .id
            .clone()]
    );
    assert_eq!(
        calls[1].dispatch[0].arguments[0].class,
        DispatchClass::Missing
    );
    assert_eq!(
        calls[1].dispatch[0].arguments[1].class,
        DispatchClass::Formal("numeric".into())
    );
    assert_eq!(calls[2].kind, S4CallKind::SelectMethod);
    assert_eq!(calls[2].use_inherited, Some(false));
    assert_eq!(calls[2].dispatch, calls[1].dispatch);
    assert_eq!(calls[2].exact_methods, calls[1].methods);
    let sites = oop_sites(&model, &parsed.cst);
    for needle in ["standardGeneric('g')", "selectMethod('g'"] {
        let offset = source.find(needle).unwrap() + needle.find("'g'").unwrap();
        assert!(sites.iter().any(|site| usize::from(site.range.start()) == offset && matches!(&site.target, OopTarget::Generic(reference) if reference.system == OopSystem::S4)));
    }
}

#[test]
fn s4_dispatch_rejects_nested_frames_and_masked_helpers() {
    let source = "methods::setGeneric('g', function(x) standardGeneric('g'))\n\
         methods::setMethod('g', 'numeric', function(x) { inner <- function() methods::callNextMethod(); inner() })\n\
         methods::callNextMethod()\n\
         base::standardGeneric('g')\n\
         selectMethod <- function(...) NULL\nselectMethod('g', 'numeric')\n";
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index(source).s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    assert_eq!(calls.len(), 4, "{calls:?}");
    assert!(calls[0].uncertainty.is_empty());
    for call in calls.iter().skip(1) {
        assert!(
            call.uncertainty.contains(&S4CallUncertainty::InvalidFrame),
            "{call:?}"
        );
    }
}

#[test]
fn next_method_in_generic_defaults_keeps_the_default_identity() {
    for source in [
        "methods::setGeneric('g', function(x) methods::callNextMethod())\n",
        "methods::setGeneric('g', function(x) standardGeneric('g'), useAsDefault = function(x) methods::callNextMethod())\n",
        "fallback <- function(x) methods::callNextMethod()\nmethods::setGeneric('g', function(x) standardGeneric('g'), useAsDefault = fallback)\n",
        "g <- function(x) methods::callNextMethod()\nmethods::setMethod('g', 'numeric', function(x) x)\n",
    ] {
        let parsed = parse(source);
        let model = SemanticModel::build(&parsed.cst);
        let index = index(source);
        let calls = index.s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        let call = calls
            .iter()
            .find(|call| call.kind == S4CallKind::NextMethod)
            .unwrap();
        assert_eq!(
            call.generics,
            [generic_id("g", ".GlobalEnv")],
            "{source}: {call:?}"
        );
        assert!(call.uncertainty.is_empty(), "{source}: {call:?}");
        let default = index
            .s4
            .methods_for_generic(&generic_id("g", ".GlobalEnv"))
            .into_iter()
            .find(|method| matches!(method.id, S4MethodId::Default(_)))
            .unwrap();
        assert_eq!(
            call.methods.as_slice(),
            std::slice::from_ref(&default.id),
            "{source}"
        );
    }
}

#[test]
fn select_method_confirms_its_own_signature_helper() {
    for signature in [
        "signature(x = 'numeric')",
        "c(x = 'numeric')",
        "character()",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x) standardGeneric('g'))\ncheck <- function() selectMethod('g', {signature})\n"
        );
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let calls = index(&source).s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
        let call = calls
            .iter()
            .find(|call| call.kind == S4CallKind::SelectMethod)
            .unwrap();
        assert!(call.uncertainty.is_empty(), "{source}: {call:?}");
        assert!(call.dispatch[0].complete, "{source}: {call:?}");
    }
}

#[test]
fn a_nested_same_named_closure_cannot_acquire_a_default_method_frame() {
    let source = "fallback <- function(x) methods::callNextMethod()\nmethods::setGeneric('g', function(x) standardGeneric('g'), useAsDefault = fallback)\nmethods::setMethod('g', 'numeric', function(x) { fallback <- function() methods::callNextMethod(); fallback() })\n";
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let calls = index(source).s4_dispatch_calls(Path::new("/s4.R"), &parsed.cst, &model);
    let calls: Vec<_> = calls
        .iter()
        .filter(|call| call.kind == S4CallKind::NextMethod)
        .collect();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].uncertainty.is_empty());
    assert!(calls[1].methods.is_empty(), "{:?}", calls[1]);
    assert!(
        calls[1]
            .uncertainty
            .contains(&S4CallUncertainty::InvalidFrame)
    );
}

#[test]
fn unknown_selected_arguments_preserve_the_whole_supplied_signature() {
    let index = index(
        "methods::setGeneric('g', function(x, y) standardGeneric('g'), signature = dispatch_args)\nmethods::setMethod('g', c(x = 'numeric', y = 'missing'), function(x, y) x)\n",
    );
    let method = index.s4.methods_for_generic(&generic_id("g", ".GlobalEnv"))[0];
    assert_eq!(method.supplied_signature.len(), 2);
    assert_eq!(method.dispatch[0].arguments, method.supplied_signature);
    assert!(!method.dispatch[0].complete);
    assert!(method.dispatch[0].valid);
}

#[test]
fn duplicate_registrations_and_generic_owners_are_not_collapsed() {
    let index = index(
        "methods::setGeneric('g', function(x) standardGeneric('g'), package = 'one')\nmethods::setGeneric('g', function(x) standardGeneric('g'), package = 'two')\nmethods::setMethod('g', 'numeric', function(x) x)\nmethods::setMethod('g', 'numeric', function(x) 2)\n",
    );
    let methods = &index.s4.methods;
    assert_eq!(methods.len(), 2);
    assert_ne!(methods[0].id, methods[1].id);
    assert_eq!(
        methods[0].generics,
        [generic_id("g", "one"), generic_id("g", "two")]
    );
    assert!(methods[0].owner.package.is_none());
}

#[test]
fn pinned_r_s4_dispatch_observations() {
    let source = include_str!("fixtures/s4_dispatch/input.R");
    let index = index(source);
    let mut methods = index
        .s4
        .methods
        .iter()
        .filter(|method| matches!(method.id, S4MethodId::Registration(_)));
    let packed = |names: &[ClassName]| {
        if names.is_empty() {
            "-".to_string()
        } else {
            names
                .iter()
                .map(|name| match name {
                    ClassName::Known(name) => name.as_str(),
                    _ => "?",
                })
                .collect::<Vec<_>>()
                .join(",")
        }
    };
    for row in include_str!("fixtures/s4_dispatch/expected.tsv").lines() {
        let columns: Vec<_> = row.split('\t').collect();
        if columns[0] == "owner" {
            continue;
        }
        let id = generic_id(columns[1], columns[2]);
        match columns[0] {
            "generic" | "group" => {
                let generic = index
                    .s4
                    .generics
                    .iter()
                    .find(|generic| generic.id == id)
                    .unwrap_or_else(|| panic!("{row}"));
                assert_eq!(packed(&generic.dispatch_arguments), columns[3], "{row}");
                assert_eq!(packed(&generic.groups), columns[4], "{row}");
                if columns[0] == "generic" {
                    let default = generic
                        .default
                        .signature
                        .as_ref()
                        .map(|sig| {
                            sig.parameters
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_else(|| "-".into());
                    assert_eq!(default, columns[5], "{row}: {generic:?}");
                } else {
                    assert!(generic.group, "{row}");
                }
            }
            "method" => {
                let method = methods.next().unwrap_or_else(|| panic!("{row}"));
                assert_eq!(method.generics, [id], "{row}");
                let [signature] = method.dispatch.as_slice() else {
                    panic!("{row}: {method:?}");
                };
                assert!(signature.complete, "{row}: {signature:?}");
                let actual = signature
                    .arguments
                    .iter()
                    .map(|arg| {
                        let class = match &arg.class {
                            DispatchClass::Formal(name) => name.as_str(),
                            DispatchClass::Any => "ANY",
                            DispatchClass::Missing => "missing",
                            _ => "?",
                        };
                        format!("{}:{class}", arg.argument.as_deref().unwrap_or("?"))
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                assert_eq!(actual, columns[3], "{row}");
            }
            _ => panic!("{row}"),
        }
    }
    assert!(methods.next().is_none());
}

#[test]
#[ignore = "requires Rscript; checks the pinned S4 dispatch observations"]
fn live_r_s4_dispatch_observations() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/s4_dispatch");
    let output =
        std::process::Command::new(std::env::var_os("RSCRIPT").unwrap_or_else(|| "Rscript".into()))
            .arg("--vanilla")
            .arg(fixture.join("observe.R"))
            .arg(&fixture)
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        include_str!("fixtures/s4_dispatch/expected.tsv")
    );
}
