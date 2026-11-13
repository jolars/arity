use arity::incremental::IncrementalDatabase;
use arity::parser::parse;
use arity::project::classes::*;
use arity::project::{PackageInfo, project_classes};
use arity::semantic::SemanticModel;
use std::collections::BTreeSet;
use std::path::Path;

fn workspace_database(packages: &[(&str, &str, &str)]) -> IncrementalDatabase {
    workspace_database_with_scripts(packages, &[])
}

fn workspace_database_with_scripts(
    packages: &[(&str, &str, &str)],
    scripts: &[(&str, &str)],
) -> IncrementalDatabase {
    let mut db = IncrementalDatabase::default();
    let roots = packages
        .iter()
        .map(|(name, _, _)| format!("/{name}").into())
        .collect::<Vec<_>>();
    let mut files: Vec<_> = packages
        .iter()
        .map(|(name, source, _)| {
            db.upsert_file(Path::new(&format!("/{name}/R/classes.R")), *source)
        })
        .collect();
    files.extend(
        scripts
            .iter()
            .map(|(path, source)| db.upsert_file(Path::new(path), *source)),
    );
    db.set_workspace_members(files, roots.clone());
    db.refresh_descriptions(roots);
    for (name, _, _) in packages {
        db.upsert_description(Path::new(&format!("/{name}")), format!("Package: {name}\n"));
    }
    db.set_package_metadata(
        packages
            .iter()
            .map(|(name, _, namespace)| PackageInfo {
                root: format!("/{name}").into(),
                namespace: Some((*namespace).into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect(),
    );
    db
}

fn workspace(packages: &[(&str, &str, &str)]) -> ClassIndex {
    project_classes(&workspace_database(packages)).clone()
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

fn resolve(index: &ClassIndex, name: &str, classes: &[&str]) -> S4MethodResolution {
    let reference = GenericRef {
        system: OopSystem::S4,
        owner: ClassOwner::default(),
        name: ClassName::Known(name.into()),
        package: None,
        by_binding: false,
        local: None,
        lookup: None,
    };
    let generics = index.s4_generic_candidates(Path::new("/s4.R"), &reference, true);
    let generic = generics
        .first()
        .copied()
        .unwrap_or_else(|| panic!("missing generic {name}"));
    let signature = s4_method_signature(
        generic,
        &classes
            .iter()
            .map(|class| DispatchArgument {
                argument: None,
                class: match *class {
                    "ANY" => DispatchClass::Any,
                    "missing" => DispatchClass::Missing,
                    name => DispatchClass::Formal(name.into()),
                },
            })
            .collect::<Vec<_>>(),
    );
    index.s4_resolve_methods(
        &S4DispatchContext::new(Path::new("/s4.R")),
        &signature,
        true,
    )
}

fn defined(index: &ClassIndex, id: &S4MethodId) -> Vec<String> {
    index
        .s4
        .methods
        .iter()
        .find(|method| &method.id == id)
        .unwrap()
        .dispatch[0]
        .arguments
        .iter()
        .map(|arg| match &arg.class {
            DispatchClass::Formal(name) => name.clone(),
            DispatchClass::Any => "ANY".into(),
            DispatchClass::Missing => "missing".into(),
            _ => "?".into(),
        })
        .collect()
}

fn selections(index: &ClassIndex, path: &Path, source: &str) -> Vec<S4DispatchCall> {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    index
        .s4_dispatch_calls(path, &parsed.cst, &SemanticModel::build(&parsed.cst))
        .into_iter()
        .filter(|call| call.kind == S4CallKind::SelectMethod)
        .collect()
}

#[test]
fn removing_the_any_entry_removes_the_generic_default() {
    for removal in [
        "methods::removeMethod('g', 'ANY')",
        "methods::setMethod('g', 'ANY', NULL)",
    ] {
        let source = format!(
            "methods::setGeneric('g', function(x) 0)\n\
             methods::selectMethod('g', 'numeric')\n\
             {removal}\n\
             methods::selectMethod('g', 'numeric', optional = TRUE)\n"
        );
        let index = index(&source);
        let calls = selections(&index, Path::new("/s4.R"), &source);
        assert!(matches!(
            calls[0].resolutions[0].selected,
            Some(S4MethodId::Default(_))
        ));
        let removed = &calls[1].resolutions[0];
        assert!(
            removed.complete() && removed.candidates.is_empty(),
            "{removed:?}"
        );
    }
}

#[test]
fn latest_proven_installation_replaces_the_earlier_method() {
    let classes = "methods::setClass('Parent')\n\
        methods::setClass('Child', contains = 'Parent')\n\
        methods::setGeneric('g', function(x) 0)\n";
    let registrations = "methods::setMethod('g', 'Parent', function(x) 1)\n\
        methods::selectMethod('g', 'Child')\n\
        methods::setMethod('g', c(x = 'Parent'), function(x) 2)\n\
        methods::selectMethod('g', 'Child')\n\
        if (flag) methods::setMethod('g', 'Parent', function(x) 3)\n\
        methods::selectMethod('g', 'Child')\n";
    for source in [
        format!("{classes}{registrations}"),
        format!("{classes}probe <- function() {{\n{registrations}}}\n"),
    ] {
        let index = index(&source);
        let calls = selections(&index, Path::new("/s4.R"), &source);
        let history = index
            .s4
            .methods
            .iter()
            .filter(|method| defined(&index, &method.id) == ["Parent"])
            .map(|method| method.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(history.len(), 3);
        assert_eq!(calls[0].resolutions[0].selected.as_ref(), Some(&history[0]));
        assert_eq!(calls[1].resolutions[0].selected.as_ref(), Some(&history[1]));
        assert_eq!(calls[2].resolutions[0].best, history[1..]);
        assert!(calls[2].resolutions[0].selected.is_none());
    }
}

#[test]
fn equal_distance_group_and_member_methods_keep_distinct_signatures() {
    let source = "methods::setClass('Left')\nmethods::setClass('Right')\n\
        methods::setClass('Child', contains = c('Left', 'Right'))\n\
        methods::setMethod('Math', 'Left', function(x) 1)\n\
        methods::setMethod('abs', 'Right', function(x) 2)\n\
        methods::selectMethod('abs', 'Child')\n";
    let index = index(source);
    let calls = selections(&index, Path::new("/s4.R"), source);
    let result = &calls[0].resolutions[0];
    assert_eq!(
        result
            .best
            .iter()
            .map(|id| defined(&index, id))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([vec!["Left".to_owned()], vec!["Right".to_owned()]]),
        "{result:?}"
    );
}

#[test]
fn qualified_values_load_method_tables_at_their_live_positions() {
    use arity::incremental::QueryKind;

    let query = "methods::selectMethod(one::g, 'Child')\n";
    let access = "marker <- (two::marker)\n";
    let source = format!("{access}{query}");
    let path = Path::new("/scripts/main.R");
    let mut db = workspace_database_with_scripts(
        &[
            (
                "one",
                "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) 0)\n",
                "export(g)\nexportClasses(Parent)\n",
            ),
            (
                "two",
                "marker <- TRUE\nmethods::setMethod('g', 'Parent', function(x) 1)\n",
                "export(marker)\nimportFrom(one, g)\nimportClassesFrom(one, Parent)\n",
            ),
        ],
        &[("/scripts/main.R", &source)],
    );
    let calls = selections(project_classes(&db), path, &source);
    assert!(
        matches!(&calls[0].resolutions[0].selected, Some(S4MethodId::Registration(id))
            if id.path == Path::new("/two/R/classes.R")),
        "{calls:?}"
    );
    db.clear_query_log();
    let moved = format!("{query}{access}");
    db.upsert_file(path, moved.as_str());
    let calls = selections(project_classes(&db), path, &moved);
    assert!(matches!(
        calls[0].resolutions[0].selected,
        Some(S4MethodId::Default(_))
    ));
    assert!(
        !db.query_log()
            .iter()
            .any(|query| query.kind == QueryKind::ProjectClasses)
    );
}

#[test]
fn only_attach_loads_activate_description_depends() {
    let path = Path::new("/scripts/main.R");
    for (load, provider_active) in [
        ("base::loadNamespace('client')", false),
        ("base::requireNamespace('client')", false),
        ("client::marker", false),
        ("base::library(client)", true),
        ("base::require(client)", true),
        ("base::library(client, attach.required = FALSE)", false),
        ("base::library(client, include.only = 'marker')", false),
    ] {
        let source = format!("{load}\nmethods::selectMethod(one::g, 'Child')\n");
        let mut db = workspace_database_with_scripts(
            &[
                (
                    "one",
                    "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) 0)\n",
                    "export(g)\nexportClasses(Parent)\n",
                ),
                (
                    "two",
                    "methods::setMethod('g', 'Parent', function(x) 1)\n",
                    "importFrom(one, g)\nimportClassesFrom(one, Parent)\n",
                ),
                ("client", "marker <- TRUE\n", "export(marker)\n"),
            ],
            &[("/scripts/main.R", &source)],
        );
        db.upsert_description(Path::new("/client"), "Package: client\nDepends: two\n");
        let calls = selections(project_classes(&db), path, &source);
        let selected = calls[0].resolutions[0]
            .selected
            .as_ref()
            .unwrap_or_else(|| panic!("{load}: {calls:?}"));
        assert_eq!(
            matches!(selected, S4MethodId::Registration(_)),
            provider_active,
            "{load}: {calls:?}"
        );
    }
}

#[test]
fn functions_see_earlier_shared_source_calls() {
    let source = "probe <- function() {\n\
          methods::selectMethod(one::g, 'Child')\n\
          base::source('helper.R')\n\
          methods::selectMethod(one::g, 'Child')\n\
          base::source(computed_path)\n\
          methods::selectMethod(one::g, 'Child')\n\
        }\n";
    let path = Path::new("/scripts/main.R");
    let db = workspace_database_with_scripts(
        &[(
            "one",
            "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) 0)\n",
            "export(g)\nexportClasses(Parent)\n",
        )],
        &[
            ("/scripts/main.R", source),
            (
                "/scripts/helper.R",
                "methods::setMethod(one::g, 'Parent', function(x) 1)\n",
            ),
        ],
    );
    let calls = selections(project_classes(&db), path, source);
    assert!(matches!(
        calls[0].resolutions[0].selected,
        Some(S4MethodId::Default(_))
    ));
    let after = &calls[1].resolutions[0];
    assert!(
        matches!(&after.selected, Some(S4MethodId::Registration(id)) if id.path == Path::new("/scripts/helper.R")),
        "{after:?}"
    );
    assert!(!calls[2].resolutions[0].complete(), "{calls:?}");
}

#[test]
fn namespace_only_loads_expose_string_generics() {
    let path = Path::new("/scripts/main.R");
    let source = "methods::selectMethod('g', 'Child', optional = TRUE)\n\
        loadNamespace('one')\n\
        methods::selectMethod('g', 'Child')\n\
        methods::selectMethod(g, 'Child')\n";
    let db = workspace_database_with_scripts(
        &[(
            "one",
            "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) 0)\nmethods::setMethod('g', 'Parent', function(x) 1)\n",
            "",
        )],
        &[("/scripts/main.R", source)],
    );
    let calls = selections(project_classes(&db), path, source);
    assert!(calls[0].generics.is_empty(), "{calls:?}");
    assert!(
        matches!(&calls[1].resolutions.first().and_then(|res| res.selected.as_ref()),
            Some(S4MethodId::Registration(id)) if id.path == Path::new("/one/R/classes.R")),
        "{calls:?}"
    );
    assert!(calls[2].generics.is_empty(), "{calls:?}");
}

#[test]
fn set_as_installs_an_inherited_coerce_method() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\n\
        methods::setClass('Target')\n\
        methods::selectMethod('coerce', c('Child', 'Target'), optional = TRUE)\n\
        methods::setAs('Parent', 'Target', function(x) stop('opaque'))\n\
        methods::selectMethod('coerce', c('Child', 'Target'))\n\
        methods::removeMethod('coerce', c('Parent', 'Target'))\n\
        methods::selectMethod('coerce', c('Child', 'Target'), optional = TRUE)\n";
    let index = index(source);
    let calls = selections(&index, Path::new("/s4.R"), source);
    assert!(calls[0].resolutions[0].best.is_empty(), "{calls:?}");
    let best = &calls[1].resolutions[0].best;
    assert_eq!(best.len(), 1, "{calls:?}");
    assert_eq!(defined(&index, &best[0]), ["Parent", "Target", "ANY"]);
    assert!(calls[2].resolutions[0].best.is_empty(), "{calls:?}");
}

#[test]
fn set_is_without_completion_skips_preexisting_subclasses() {
    let source = "methods::setClass('Root', contains = 'VIRTUAL')\n\
        methods::setClass('Parent', contains = c('Root', 'VIRTUAL'))\n\
        methods::setClass('Middle', slots = c(x = 'numeric'))\n\
        methods::setClass('Earlier', contains = 'Middle')\n\
        methods::setClass('ExtendedLater', slots = c(x = 'numeric'))\n\
        methods::setIs('Middle', 'Parent', doComplete = FALSE)\n\
        methods::setClass('Later', contains = 'Middle')\n\
        methods::setGeneric('g', function(x) 0)\n\
        methods::setMethod('g', 'Root', function(x) 2)\n\
        methods::setMethod('g', 'Parent', function(x) 1)\n\
        methods::selectMethod('g', 'Earlier')\n\
        methods::selectMethod('g', 'Middle')\n\
        methods::selectMethod('g', 'Later')\n\
        methods::setIs('ExtendedLater', 'Middle')\n\
        methods::selectMethod('g', 'ExtendedLater')\n\
        methods::removeMethod('g', 'Parent')\n\
        methods::selectMethod('g', 'Middle')\n\
        methods::selectMethod('g', 'Later')\n";
    let index = index(source);
    let calls = selections(&index, Path::new("/s4.R"), source);
    assert!(
        matches!(
            calls[0].resolutions[0].selected,
            Some(S4MethodId::Default(_))
        ),
        "{calls:?}"
    );
    for call in &calls[1..3] {
        let result = &call.resolutions[0];
        assert_eq!(
            defined(&index, result.selected.as_ref().unwrap()),
            ["Parent"]
        );
    }
    let extended = &calls[3].resolutions[0];
    assert!(
        extended
            .best
            .iter()
            .any(|id| defined(&index, id) == ["Parent"]),
        "{extended:?}"
    );
    assert!(
        extended
            .selected
            .as_ref()
            .is_none_or(|id| defined(&index, id) == ["Parent"])
    );
    for call in &calls[4..] {
        assert!(
            matches!(call.resolutions[0].selected, Some(S4MethodId::Default(_))),
            "{call:?}"
        );
    }
}

#[test]
fn primitive_function_objects_use_the_canonical_generic() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\n\
        methods::setMethod('as.numeric', 'Parent', function(x, ...) 1)\n\
        methods::selectMethod(base::as.double, 'Child')\n\
        methods::selectMethod('as.double', 'Child', optional = TRUE)\n";
    let index = index(source);
    let calls = selections(&index, Path::new("/s4.R"), source);
    assert_eq!(
        calls[0].generics,
        [S4GenericId::Named {
            package: "base".into(),
            name: "as.numeric".into()
        }]
    );
    let result = &calls[0].resolutions[0];
    assert_eq!(result.best.len(), 1, "{calls:?}");
    assert_eq!(defined(&index, &result.best[0]), ["Parent"]);
    assert!(calls[1].resolutions[0].best.is_empty(), "{calls:?}");
}

#[test]
fn dots_selection_uses_the_vector_of_actual_argument_classes() {
    let source = "methods::setGeneric('g', function(...) 0, signature = '...')\n\
        methods::setMethod('g', 'numeric', function(...) 1)\n\
        methods::selectMethod('g', c('integer', 'numeric'))\n\
        methods::selectMethod('g', c('integer', 'numeric'), useInherited = FALSE, optional = TRUE)\n\
        methods::selectMethod('g', c('integer', 'character'))\n\
        methods::selectMethod('g', c('numeric', 'numeric'), useInherited = FALSE)\n\
        methods::setMethod('g', 'integer', function(...) 2)\n\
        methods::selectMethod('g', c('integer', 'numeric'), useInherited = FALSE)\n\
        methods::selectMethod('g', c('numeric', 'integer'))\n";
    let index = index(source);
    let calls = selections(&index, Path::new("/s4.R"), source);
    assert_eq!(calls.len(), 6);
    for (i, class) in [
        (0, "numeric"),
        (3, "numeric"),
        (4, "integer"),
        (5, "numeric"),
    ] {
        let result = &calls[i].resolutions[0];
        assert!(result.complete(), "{result:?}");
        assert_eq!(defined(&index, result.selected.as_ref().unwrap()), [class]);
    }
    assert!(
        calls[1].resolutions[0].complete() && calls[1].resolutions[0].best.is_empty(),
        "{calls:?}"
    );
    assert!(
        matches!(
            calls[2].resolutions[0].selected,
            Some(S4MethodId::Default(_))
        ),
        "{calls:?}"
    );
}

#[test]
fn qualified_reexports_load_the_provider_but_keep_the_generic_owner() {
    let query = "methods::selectMethod(two::g, 'numeric')\n";
    for export in ["export(g)", "exportMethods(g)"] {
        let index = workspace(&[
            (
                "one",
                "methods::setGeneric('g', function(x) 0)\n",
                "export(g)\n",
            ),
            (
                "two",
                "methods::setMethod(one::g, 'numeric', function(x) 1)\n",
                &format!("importFrom(one, g)\n{export}\n"),
            ),
            ("three", query, ""),
        ]);
        let calls = selections(&index, Path::new("/three/R/classes.R"), query);
        let call = &calls[0];
        assert_eq!(
            call.generics,
            [S4GenericId::Named {
                name: "g".into(),
                package: "one".into()
            }],
            "{call:?}"
        );
        assert!(
            matches!(&call.resolutions[0].selected,
            Some(S4MethodId::Registration(id)) if id.path == Path::new("/two/R/classes.R")),
            "{call:?}"
        );
    }
}

#[test]
fn qualified_lookup_needs_a_function_binding_in_the_provider() {
    let query = "methods::selectMethod(two::g, 'numeric')\n";
    for imports in [
        "",
        "import(one, except = g)\n",
        "importFrom(one, other = g)\n",
    ] {
        let index = workspace(&[
            (
                "one",
                "methods::setGeneric('g', function(x) 0)\n",
                "export(g)\n",
            ),
            (
                "two",
                "methods::setMethod(one::g, 'numeric', function(x) 1)\n",
                &format!("{imports}exportMethods(g)\n"),
            ),
            ("three", query, ""),
        ]);
        let calls = selections(&index, Path::new("/three/R/classes.R"), query);
        assert!(calls[0].generics.is_empty(), "{imports}: {calls:?}");
    }
}

#[test]
fn import_exclusions_do_not_load_a_same_named_package() {
    let query = "methods::selectMethod(one::g, 'numeric')\n";
    let index = workspace(&[
        (
            "one",
            "methods::setGeneric('g', function(x) 0)\n",
            "export(g)\n",
        ),
        (
            "two",
            "methods::setMethod(one::g, 'numeric', function(x) 1)\n",
            "exportMethods(g)\n",
        ),
        ("three", query, "import(one, except = two)\n"),
    ]);
    assert_eq!(
        index
            .s4_namespace(Path::new("/three"))
            .unwrap()
            .loaded_packages,
        BTreeSet::from(["one".into()])
    );
    let calls = selections(&index, Path::new("/three/R/classes.R"), query);
    assert!(
        matches!(
            calls[0].resolutions[0].selected,
            Some(S4MethodId::Default(_))
        ),
        "{calls:?}"
    );
}

#[test]
fn partially_indexed_workspace_providers_prevent_a_definite_selection() {
    let query = "methods::selectMethod(one::g, 'numeric')\n";
    let packages = [
        (
            "one",
            "methods::setGeneric('g', function(x) 0)\n",
            "export(g)\n",
        ),
        (
            "two",
            "methods::setClass('Marker')\n",
            "exportClasses(Marker)\n",
        ),
        ("three", query, "importClassesFrom(two, Marker)\n"),
    ];
    let mut db = workspace_database(&packages);
    db.set_package_metadata(
        packages
            .iter()
            .map(|(name, _, namespace)| PackageInfo {
                root: format!("/{name}").into(),
                namespace: Some((*namespace).into()),
                dir_sources: if *name == "two" {
                    BTreeSet::from(["classes.R".into(), "excluded.R".into()])
                } else {
                    BTreeSet::from(["classes.R".into()])
                },
                native_routines: BTreeSet::new(),
            })
            .collect(),
    );
    let calls = selections(project_classes(&db), Path::new("/three/R/classes.R"), query);
    let result = &calls[0].resolutions[0];
    assert!(
        result
            .uncertainty
            .contains(&S4SelectionUncertainty::IncompleteScope),
        "{result:?}"
    );
    assert!(result.selected.is_none(), "{result:?}");
}

#[test]
fn dispatch_can_use_private_classes_from_loaded_namespaces() {
    let query = "methods::selectMethod('g', 'Child')\n";
    let index = workspace(&[
        (
            "one",
            "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) 0)\nmethods::setMethod('g', 'Parent', function(x) 1)\n",
            "export(g)\n",
        ),
        ("two", query, "importFrom(one, g)\n"),
    ]);
    let path = Path::new("/two/R/classes.R");
    assert!(
        index
            .resolve_reference(
                path,
                &ClassOwner::default(),
                &ParentRef::Formal("Child".into()),
                true
            )
            .is_empty()
    );
    let calls = selections(&index, path, query);
    assert_eq!(
        defined(&index, calls[0].resolutions[0].selected.as_ref().unwrap()),
        ["Parent"]
    );
}

#[test]
fn unloaded_namespaces_do_not_install_relations() {
    let query = "methods::selectMethod(one::g, 'Child')\n";
    let index = workspace(&[
        (
            "one",
            "methods::setClass('Root', contains = 'VIRTUAL')\nmethods::setClass('Child')\nmethods::setGeneric('g', function(x) 0)\nmethods::setMethod('g', 'Root', function(x) 1)\n",
            "export(g)\nexportClasses(Child, Root)\n",
        ),
        (
            "two",
            "methods::setClassUnion('Bridge', 'Child')\nmethods::setIs('Bridge', 'Root')\n",
            "importClassesFrom(one, Child, Root)\n",
        ),
        ("three", query, "importClassesFrom(one, Child)\n"),
    ]);
    let calls = selections(&index, Path::new("/three/R/classes.R"), query);
    let result = &calls[0].resolutions[0];
    assert!(
        matches!(result.selected, Some(S4MethodId::Default(_))),
        "{result:?}"
    );
    assert_eq!(result.candidates.len(), 1, "{result:?}");
}

#[test]
fn method_exports_expose_only_active_implicit_generic_bindings() {
    let source = "methods::selectMethod(one::g, 'numeric')\n";
    for replacement in ["", "g <- function(y) y\n", "g <- NULL\n", "g <- unknown\n"] {
        let provider = format!(
            "g <- function(x) x\nmethods::setMethod('g', 'numeric', function(x) x)\n{replacement}"
        );
        let index = workspace(&[
            ("one", &provider, "exportMethods(g)\n"),
            ("three", source, ""),
        ]);
        let parsed = parse(source);
        let calls = index.s4_dispatch_calls(
            Path::new("/three/R/classes.R"),
            &parsed.cst,
            &SemanticModel::build(&parsed.cst),
        );
        if replacement.is_empty() {
            let resolution = &calls[0].resolutions[0];
            assert!(resolution.complete(), "{calls:?}");
            assert_eq!(
                defined(&index, resolution.selected.as_ref().unwrap()),
                ["numeric"]
            );
        } else {
            assert!(calls[0].generics.is_empty(), "{replacement}: {calls:?}");
        }
    }
}

#[test]
fn method_exports_do_not_export_primitive_values() {
    let source = "methods::selectMethod(two::length, 'Sized')\n";
    for redeclaration in ["", "methods::setGeneric('length')\n"] {
        let provider = format!(
            "methods::setClass('Sized')\nmethods::setMethod('length', 'Sized', function(x) 1L)\n{redeclaration}"
        );
        let index = workspace(&[
            ("two", &provider, "exportMethods(length)\n"),
            ("three", source, ""),
        ]);
        let parsed = parse(source);
        let calls = index.s4_dispatch_calls(
            Path::new("/three/R/classes.R"),
            &parsed.cst,
            &SemanticModel::build(&parsed.cst),
        );
        assert!(calls[0].generics.is_empty(), "{calls:?}");
        assert!(
            calls[0]
                .uncertainty
                .contains(&S4CallUncertainty::UnresolvedGeneric)
        );
    }
}

#[test]
fn unanalyzed_loaded_providers_prevent_complete_selection() {
    let source = "probe <- function() methods::selectMethod(one::g, 'numeric')\n";
    let index = workspace(&[
        (
            "one",
            "methods::setGeneric('g', function(x) 'default')\n",
            "export(g)\n",
        ),
        (
            "three",
            source,
            "importFrom(one, g)\nimportClassesFrom(externalProvider, Marker)\n",
        ),
    ]);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/three/R/classes.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let resolution = &calls[0].resolutions[0];
    assert!(!resolution.complete(), "{resolution:?}");
    assert!(resolution.selected.is_none());
}

#[test]
fn sourced_registrations_follow_project_source_visibility() {
    let temp = tempfile::tempdir().unwrap();
    let main = "methods::selectMethod('g', 'Child', optional = TRUE)\nsource('helper.R')\nmethods::selectMethod('g', 'Child')\n";
    let helper = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::setMethod('g', 'Parent', function(x) x)\n";
    let entry = temp.path().join("main.R");
    let helper_path = temp.path().join("helper.R");
    std::fs::write(&entry, main).unwrap();
    std::fs::write(&helper_path, helper).unwrap();
    let mut db = IncrementalDatabase::default();
    let members = vec![
        db.upsert_file(&entry, main),
        db.upsert_file(&helper_path, helper),
    ];
    db.set_workspace_members(members, vec![temp.path().into()]);
    let index = project_classes(&db);
    let calls = selections(index, &entry, main);
    assert!(calls[0].generics.is_empty(), "{calls:?}");
    let resolution = &calls[1].resolutions[0];
    assert!(resolution.complete(), "{calls:?}");
    let selected = resolution.selected.as_ref().unwrap();
    assert_eq!(defined(index, selected), ["Parent"]);
    assert!(matches!(selected, S4MethodId::Registration(id) if id.path == helper_path));
}

#[test]
fn namespace_loads_in_sourced_files_follow_the_query_position() {
    let main = "methods::selectMethod(one::g, 'numeric')\nsource('helper.R')\nmethods::selectMethod(one::g, 'numeric')\n";
    let db = workspace_database_with_scripts(
        &[
            (
                "one",
                "methods::setGeneric('g', function(x) 0)\n",
                "export(g)\n",
            ),
            (
                "two",
                "methods::setMethod(one::g, 'numeric', function(x) 1)\n",
                "exportMethods(g)\n",
            ),
        ],
        &[
            ("/scripts/main.R", main),
            ("/scripts/helper.R", "base::loadNamespace('two')\n"),
        ],
    );
    let calls = selections(project_classes(&db), Path::new("/scripts/main.R"), main);
    assert!(
        matches!(
            calls[0]
                .resolutions
                .first()
                .and_then(|res| res.selected.as_ref()),
            Some(S4MethodId::Default(_))
        ),
        "{calls:?}"
    );
    assert!(
        matches!(calls[1].resolutions.first().and_then(|res| res.selected.as_ref()),
        Some(S4MethodId::Registration(id)) if id.path == Path::new("/two/R/classes.R")),
        "{calls:?}"
    );
}

#[test]
fn preceding_confirmed_library_calls_install_provider_methods() {
    for (source, expected_package, complete) in [
        (
            "library(two)\nmethods::selectMethod(one::g, 'numeric')\n",
            "two",
            true,
        ),
        (
            "methods::selectMethod(one::g, 'numeric')\nlibrary(two)\n",
            "one",
            true,
        ),
        (
            "library <- function(...) NULL\nlibrary(two)\nmethods::selectMethod(one::g, 'numeric')\n",
            "one",
            false,
        ),
        (
            "if (flag) library(two)\nmethods::selectMethod(one::g, 'numeric')\n",
            "one",
            false,
        ),
        (
            "base::requireNamespace('two')\nmethods::selectMethod(one::g, 'numeric')\n",
            "two",
            true,
        ),
        (
            "loadNamespace(package = 'two')\nmethods::selectMethod(one::g, 'numeric')\n",
            "two",
            true,
        ),
        (
            "helpers::library(two)\nmethods::selectMethod(one::g, 'numeric')\n",
            "one",
            true,
        ),
        (
            "base::loadNamespace('externalProvider')\nmethods::selectMethod(one::g, 'numeric')\n",
            "one",
            false,
        ),
        (
            "base::requireNamespace(provider)\nmethods::selectMethod(one::g, 'numeric')\n",
            "one",
            false,
        ),
    ] {
        let index = workspace(&[
            (
                "one",
                "methods::setGeneric('g', function(x) 'default')\n",
                "export(g)\n",
            ),
            (
                "two",
                "methods::setMethod('g', 'numeric', function(x) 'provider')\n",
                "importFrom(one, g)\n",
            ),
            (
                "helpers",
                "library <- function(...) NULL\n",
                "export(library)\n",
            ),
            ("three", source, ""),
        ]);
        let parsed = parse(source);
        let calls = index.s4_dispatch_calls(
            Path::new("/three/R/classes.R"),
            &parsed.cst,
            &SemanticModel::build(&parsed.cst),
        );
        let resolution = &calls[0].resolutions[0];
        assert_eq!(resolution.complete(), complete, "{source}: {calls:?}");
        let method = index
            .s4
            .methods
            .iter()
            .find(|method| resolution.best.contains(&method.id))
            .unwrap();
        assert_eq!(
            method.owner.package.as_deref(),
            Some(expected_package),
            "{source}: {calls:?}"
        );
        assert_eq!(
            resolution.selected.is_some(),
            complete,
            "{source}: {calls:?}"
        );
    }
}

#[test]
fn selection_uses_only_class_definitions_and_bridges_installed_at_the_query() {
    for (setup, installation) in [
        (
            "methods::setClass('Parent')\n",
            "methods::setClass('Child', contains = 'Parent')\n",
        ),
        (
            "methods::setOldClass('Parent')\nmethods::setClass('Child', slots = c(x = 'numeric'))\n",
            "methods::setOldClass(c('Child', 'Parent'), S4Class = 'Child')\n",
        ),
    ] {
        let query = "methods::selectMethod('g', 'Child')\n";
        let source = format!(
            "{setup}methods::setGeneric('g', function(x) 0)\nmethods::setMethod('g', 'Parent', function(x) 1)\n{query}{installation}{query}"
        );
        let index = index(&source);
        let calls = selections(&index, Path::new("/s4.R"), &source);
        assert!(
            matches!(
                calls[0].resolutions[0].selected,
                Some(S4MethodId::Default(_))
            ),
            "{calls:?}"
        );
        assert_eq!(
            defined(&index, calls[1].resolutions[0].selected.as_ref().unwrap()),
            ["Parent"]
        );
    }
}

#[test]
fn method_removals_use_the_active_registration_history() {
    for removal in [
        "methods::removeMethod('g', 'Parent')",
        "methods::setMethod('g', 'Parent', NULL)",
    ] {
        let query = "methods::selectMethod('g', 'Child')\n";
        let source = format!(
            "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) 0)\nmethods::setMethod('g', 'Parent', function(x) 1)\n{query}{removal}\n{query}methods::setMethod('g', 'Parent', function(x) 2)\n{query}"
        );
        let index = index(&source);
        let calls = selections(&index, Path::new("/s4.R"), &source);
        let selected = |i: usize| {
            calls[i].resolutions[0]
                .selected
                .as_ref()
                .unwrap_or_else(|| panic!("{calls:?}"))
        };
        assert_eq!(defined(&index, selected(0)), ["Parent"]);
        assert!(matches!(selected(1), S4MethodId::Default(_)), "{calls:?}");
        assert_eq!(defined(&index, selected(2)), ["Parent"]);
        assert_ne!(selected(0), selected(2));
    }
}

#[test]
fn dynamic_method_removals_prevent_definite_ranking() {
    let source = "methods::setGeneric('g', function(x) 0)\nmethods::setMethod('g', 'numeric', function(x) 1)\nmethods::removeMethod('g', get_signature())\nmethods::selectMethod('g', 'numeric')\n";
    let calls = selections(&index(source), Path::new("/s4.R"), source);
    assert!(calls[0].resolutions[0].selected.is_none(), "{calls:?}");
    assert!(
        calls[0].resolutions[0]
            .uncertainty
            .contains(&S4SelectionUncertainty::RuntimeRegistration),
        "{calls:?}"
    );
}

#[test]
fn a_called_closure_can_install_methods_in_the_enclosing_table() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) 0)\ninstall <- function() methods::setMethod('g', 'Parent', function(x) 1)\ninstall()\nmethods::selectMethod('g', 'Child')\n";
    let calls = selections(&index(source), Path::new("/s4.R"), source);
    let result = &calls[0].resolutions[0];
    assert!(result.selected.is_none(), "{result:?}");
    assert!(
        result
            .uncertainty
            .contains(&S4SelectionUncertainty::RuntimeRegistration),
        "{result:?}"
    );
}

#[test]
fn shared_next_method_closures_preserve_less_specific_registrations() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nimpl <- function(x) methods::callNextMethod()\nmethods::setMethod('g', 'Parent', impl)\nmethods::setMethod('g', 'Child', impl)\nmethods::setMethod('g', 'ANY', function(x) 'fallback')\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let call = calls
        .iter()
        .find(|call| call.kind == S4CallKind::NextMethod)
        .unwrap();
    assert_eq!(call.resolutions.len(), 2);
    for resolution in &call.resolutions {
        assert!(resolution.complete(), "{resolution:?}");
        let expected =
            if resolution.signature.arguments[0].class == DispatchClass::Formal("Child".into()) {
                "Parent"
            } else {
                "ANY"
            };
        assert_eq!(
            defined(&index, resolution.selected.as_ref().unwrap()),
            [expected],
            "{call:?}"
        );
    }
}

#[test]
fn next_method_does_not_revive_overwritten_registrations() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::setMethod('g', 'Parent', function(x) 'parent')\nmethods::setMethod('g', 'Child', function(x) 'overwritten')\nmethods::setMethod('g', 'Child', function(x) methods::callNextMethod())\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let call = calls
        .iter()
        .find(|call| call.kind == S4CallKind::NextMethod)
        .unwrap();
    let resolution = &call.resolutions[0];
    assert!(resolution.complete(), "{resolution:?}");
    assert_eq!(
        defined(&index, resolution.selected.as_ref().unwrap()),
        ["Parent"],
        "{call:?}"
    );
}

#[test]
fn top_level_selection_does_not_assume_later_inheritance_relations() {
    let source = "methods::setClass('A')\nmethods::setClass('B')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::setMethod('g', 'B', function(x) 'B')\nmethods::selectMethod('g', 'A', optional = TRUE)\nmethods::setIs('A', 'B')\nmethods::selectMethod('g', 'A')\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let selections: Vec<_> = calls
        .iter()
        .filter(|call| call.kind == S4CallKind::SelectMethod)
        .collect();
    assert!(
        selections[0].resolutions[0].selected.is_none(),
        "{selections:?}"
    );
    let after = &selections[1].resolutions[0];
    assert!(after.complete(), "{after:?}");
    assert_eq!(defined(&index, after.selected.as_ref().unwrap()), ["B"]);
}

#[test]
fn relation_execution_order_uses_collate_across_package_files() {
    let definitions = "methods::setClass('A')\nmethods::setClass('B')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::setMethod('g', 'B', function(x) 'B')\n";
    let query = "methods::selectMethod('g', 'A', optional = TRUE)\n";
    for (collate, established) in [
        ("classes.R relations.R query.R", true),
        ("classes.R query.R relations.R", false),
        ("", false),
    ] {
        let mut db = workspace_database(&[("one", definitions, "")]);
        let members = vec![
            db.upsert_file(Path::new("/one/R/classes.R"), definitions),
            db.upsert_file(
                Path::new("/one/R/relations.R"),
                "methods::setIs('A', 'B')\n",
            ),
            db.upsert_file(Path::new("/one/R/query.R"), query),
        ];
        db.set_workspace_members(members, vec!["/one".into()]);
        db.refresh_descriptions(["/one".into()]);
        db.upsert_description(
            Path::new("/one"),
            format!("Package: one\nCollate: {collate}\n"),
        );
        db.set_package_metadata(vec![PackageInfo {
            root: "/one".into(),
            namespace: Some(String::new()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let index = project_classes(&db);
        let parsed = parse(query);
        let calls = index.s4_dispatch_calls(
            Path::new("/one/R/query.R"),
            &parsed.cst,
            &SemanticModel::build(&parsed.cst),
        );
        assert!(!calls[0].resolutions.is_empty(), "{collate}: {calls:?}");
        let resolution = &calls[0].resolutions[0];
        assert_eq!(
            resolution.selected.is_some(),
            established,
            "{collate}: {resolution:?}"
        );
        if established {
            assert!(resolution.complete(), "{resolution:?}");
            assert_eq!(defined(index, resolution.selected.as_ref().unwrap()), ["B"]);
        }
    }
}

#[test]
fn moving_a_query_across_a_relation_reuses_the_graph_and_refreshes_selection() {
    use arity::incremental::QueryKind;
    let definitions = "methods::setClass('A')\nmethods::setClass('B')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::setMethod('g', 'B', function(x) 'B')\n";
    let query = "methods::selectMethod('g', 'A', optional = TRUE)\n";
    let relation = "methods::setIs('A', 'B')\n";
    let before = format!("{definitions}{query}{relation}");
    let after = format!("{definitions}{relation}{query}");
    let mut db = workspace_database(&[("one", &before, "")]);
    let index = project_classes(&db).clone();
    db.clear_query_log();
    db.upsert_file(Path::new("/one/R/classes.R"), after.as_str());
    assert_eq!(&index, project_classes(&db));
    assert!(
        !db.query_log()
            .iter()
            .any(|query| query.kind == QueryKind::ProjectClasses)
    );
    for (source, established) in [(&before, false), (&after, true)] {
        let parsed = parse(source);
        let calls = index.s4_dispatch_calls(
            Path::new("/one/R/classes.R"),
            &parsed.cst,
            &SemanticModel::build(&parsed.cst),
        );
        let selection = calls
            .iter()
            .find(|call| call.kind == S4CallKind::SelectMethod)
            .unwrap();
        assert_eq!(
            selection.resolutions[0].selected.is_some(),
            established,
            "{calls:?}"
        );
    }
}

#[test]
fn inherited_selection_uses_distances_in_every_dispatch_argument() {
    let index = index(
        "methods::setClass('Root')\n\
         methods::setClass('Parent', contains = 'Root')\n\
         methods::setClass('Child', contains = 'Parent')\n\
         methods::setGeneric('g', function(x, y) standardGeneric('g'))\n\
         methods::setMethod('g', c('Root', 'ANY'), function(x, y) 1)\n\
         methods::setMethod('g', c('Parent', 'ANY'), function(x, y) 2)\n",
    );
    let result = resolve(&index, "g", &["Child", "missing"]);
    assert!(result.complete(), "{result:?}");
    assert_eq!(
        defined(&index, result.selected.as_ref().unwrap()),
        ["Parent", "ANY"]
    );
    assert_eq!(result.candidates.len(), 2);
    assert_eq!(result.best.len(), 1);
}

#[test]
fn multiple_inheritance_and_crossed_signatures_remain_ambiguous() {
    for (parents, signatures, query) in [
        (
            "c('Left', 'Right')",
            ["c('Left', 'ANY')", "c('Right', 'ANY')"],
            ["Child", "missing"],
        ),
        (
            "'Left'",
            ["c('Left', 'ANY')", "c('ANY', 'Left')"],
            ["Child", "Child"],
        ),
    ] {
        let index = index(&format!(
            "methods::setClass('Left')\nmethods::setClass('Right')\n\
             methods::setClass('Child', contains = {parents})\n\
             methods::setGeneric('g', function(x, y) standardGeneric('g'))\n\
             methods::setMethod('g', {}, function(x, y) 1)\n\
             methods::setMethod('g', {}, function(x, y) 2)\n",
            signatures[0], signatures[1],
        ));
        let result = resolve(&index, "g", &query);
        assert!(
            result
                .uncertainty
                .contains(&S4SelectionUncertainty::AmbiguousMethod),
            "{result:?}"
        );
        assert_eq!(result.best.len(), 2);
        assert!(result.selected.is_none());
    }
}

#[test]
fn unknown_ancestry_and_conditional_relationships_prevent_a_definite_selection() {
    for definition in [
        "methods::setClass('Child', contains = external_parent)",
        "methods::setClass('Child')\nmethods::setIs('Child', 'Parent', test = function(x) TRUE)",
        "methods::setClass('Child', contains = 'UnknownExternal')",
    ] {
        let index = index(&format!(
            "methods::setClass('Parent')\n{definition}\n\
             methods::setGeneric('g', function(x) standardGeneric('g'))\n\
             methods::setMethod('g', 'Parent', function(x) 1)\n\
             methods::setMethod('g', 'ANY', function(x) 2)\n",
        ));
        let result = resolve(&index, "g", &["Child"]);
        assert!(!result.complete(), "{definition}: {result:?}");
        assert!(result.selected.is_none(), "{definition}: {result:?}");
        assert!(
            result
                .candidates
                .iter()
                .any(|candidate| defined(&index, &candidate.method) == ["Parent"]),
            "{definition}: {result:?}"
        );
    }
}

#[test]
fn reference_classes_and_old_class_bridges_share_formal_dispatch_but_keep_storage_separate() {
    let index = index(
        "methods::setOldClass(c('OldChild', 'OldParent'))\n\
         methods::setClass('Storage', slots = c(payload = 'numeric'))\n\
         Ref <- methods::setRefClass('Ref', contains = 'Storage', fields = list(value = 'numeric'))\n\
         Child <- methods::setRefClass('Child', contains = 'Ref')\n\
         methods::setGeneric('g', function(x) standardGeneric('g'))\n\
         methods::setMethod('g', 'Storage', function(x) 1)\n\
         methods::setMethod('g', 'OldParent', function(x) 2)\n",
    );
    for (class, expected) in [("Child", "Storage"), ("OldChild", "OldParent")] {
        let result = resolve(&index, "g", &[class]);
        assert!(result.complete(), "{result:?}");
        assert_eq!(
            defined(&index, result.selected.as_ref().unwrap()),
            [expected]
        );
    }
    let child = index
        .declarations
        .iter()
        .find(|(_, class)| class.declaration.name == ClassName::Known("Child".into()))
        .unwrap()
        .0;
    let api = index.reference_api(child).unwrap();
    assert!(api.instance.entries.contains_key("value"));
    assert!(!api.instance.entries.contains_key("payload"));
    assert!(api.slots.entries.contains_key("payload"));
}

#[test]
fn primitive_generics_keep_base_identity_and_dispatch_formals() {
    let index = index(
        "methods::setClass('Sized')\n\
         methods::setMethod('length', 'Sized', function(x) 1L)\n\
         methods::setMethod('+', c('Sized', 'numeric'), function(e1, e2) e2)\n",
    );
    for (name, classes, arguments) in [
        ("length", vec!["Sized"], vec!["x"]),
        ("+", vec!["Sized", "integer"], vec!["e1", "e2"]),
    ] {
        let generic = index
            .s4
            .generics
            .iter()
            .find(|generic| {
                generic.id
                    == S4GenericId::Named {
                        package: "base".into(),
                        name: name.into(),
                    }
            })
            .unwrap();
        assert_eq!(
            generic.dispatch_arguments,
            arguments
                .into_iter()
                .map(|name| ClassName::Known(name.into()))
                .collect::<Vec<_>>()
        );
        let result = resolve(&index, name, &classes);
        assert_eq!(result.best.len(), 1, "{result:?}");
        assert!(
            result
                .uncertainty
                .contains(&S4SelectionUncertainty::ExternalMethods)
        );
    }
}

#[test]
fn foreign_method_imports_retain_table_provider_and_generic_owner() {
    let source =
        "methods::setClass('Child', contains = 'Parent')\nmethods::selectMethod('g', 'Child')\n";
    let index = workspace(&[
        (
            "one",
            "methods::setClass('Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\n",
            "exportClasses(Parent)\nexport(g)\n",
        ),
        (
            "two",
            "methods::setMethod('g', 'Parent', function(x) 1)\n",
            "importClassesFrom(one, Parent)\nimportFrom(one, g)\nexportMethods(g)\n",
        ),
        (
            "three",
            source,
            "importClassesFrom(one, Parent)\nimportMethodsFrom(two, g)\n",
        ),
    ]);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/three/R/classes.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let call = calls.last().unwrap();
    assert_eq!(
        call.generics,
        [S4GenericId::Named {
            package: "one".into(),
            name: "g".into()
        }],
        "{call:?}"
    );
    let resolution = &call.resolutions[0];
    assert!(resolution.complete(), "{resolution:?}");
    let selected = resolution.selected.as_ref().unwrap();
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| &method.id == selected)
        .unwrap();
    assert_eq!(method.owner.package.as_deref(), Some("two"));
    let provider = index.s4_namespace(Path::new("/two")).unwrap();
    assert_eq!(provider.exported_methods, BTreeSet::from(["g".into()]));
    assert!(provider.exported_classes.is_empty());
    let client = index.s4_namespace(Path::new("/three")).unwrap();
    assert_eq!(client.imported_methods["two"], BTreeSet::from(["g".into()]));
    assert_eq!(
        client.imported_classes["one"],
        BTreeSet::from(["Parent".into()])
    );
}

#[test]
fn method_directives_do_not_export_arbitrary_values_or_class_generators() {
    let index = workspace(&[
        (
            "one",
            "Parent <- R6::R6Class('Parent')\n",
            "exportMethods(Parent)\n",
        ),
        (
            "two",
            "Child <- R6::R6Class('Child', inherit = Parent)\n",
            "importMethodsFrom(one, Parent)\n",
        ),
    ]);
    let child = index
        .declarations
        .iter()
        .find(|(_, class)| class.declaration.name == ClassName::Known("Child".into()))
        .unwrap()
        .0;
    assert!(index.parent_candidates[child][0].is_empty());
}

#[test]
fn loaded_namespaces_install_methods_without_lexical_imports() {
    for imports in ["importClassesFrom(two, Marker)\n", ""] {
        let source = "methods::selectMethod(one::g, 'numeric')\n";
        let index = workspace(&[
            (
                "one",
                "methods::setGeneric('g', function(x) standardGeneric('g'))\n",
                "export(g)\n",
            ),
            (
                "two",
                "methods::setClass('Marker')\nmethods::setMethod('g', 'numeric', function(x) 1)\n",
                "importFrom(one, g)\nexportClasses(Marker)\n",
            ),
            ("three", source, imports),
        ]);
        let parsed = parse(source);
        let calls = index.s4_dispatch_calls(
            Path::new("/three/R/classes.R"),
            &parsed.cst,
            &SemanticModel::build(&parsed.cst),
        );
        let resolution = &calls[0].resolutions[0];
        if imports.is_empty() {
            assert!(resolution.candidates.is_empty(), "{resolution:?}");
        } else {
            assert!(resolution.complete(), "{resolution:?}");
            assert!(resolution.selected.is_some(), "{resolution:?}");
        }
    }
}

#[test]
fn unions_include_builtin_members_and_old_classes_reach_old_class() {
    let index = index(
        "methods::setClass('A')\nmethods::setClassUnion('Either', c('A', 'NULL'))\n\
         methods::setOldClass('Old')\n\
         methods::setGeneric('g', function(x) standardGeneric('g'))\n\
         methods::setMethod('g', 'Either', function(x) 1)\n\
         methods::setMethod('g', 'oldClass', function(x) 2)\n",
    );
    for (actual, required) in [("A", "Either"), ("NULL", "Either"), ("Old", "oldClass")] {
        let result = resolve(&index, "g", &[actual]);
        assert!(result.complete(), "{actual}: {result:?}");
        assert_eq!(
            defined(&index, result.selected.as_ref().unwrap()),
            [required]
        );
    }
}

#[test]
fn unknown_registrations_cannot_disappear_from_the_candidate_set() {
    for registration in [
        "methods::setMethod(computed_name(), 'numeric', function(x) 1)",
        "methods::setMethod('g', computed_signature(), function(x) 1)",
        "if (flag) methods::setMethod('g', 'numeric', function(x) 1)",
    ] {
        let index = index(&format!(
            "methods::setGeneric('g', function(x) x)\n{registration}\n",
        ));
        let result = resolve(&index, "g", &["numeric"]);
        assert!(!result.complete(), "{registration}: {result:?}");
        assert!(result.selected.is_none(), "{registration}: {result:?}");
        assert_eq!(result.candidates.len(), 2, "{registration}: {result:?}");
    }
}

#[test]
fn explicit_any_methods_override_the_generic_default() {
    let index = index(
        "methods::setGeneric('g', function(x) 1)\n\
         methods::setMethod('g', 'ANY', function(x) 2)\n",
    );
    let result = resolve(&index, "g", &["numeric"]);
    assert!(result.complete(), "{result:?}");
    assert!(matches!(result.selected, Some(S4MethodId::Registration(_))));
}

#[test]
fn explicit_primitive_redeclarations_recover_the_implicit_signature() {
    let source = "methods::setClass('A')\nmethods::setGeneric('length')\nmethods::setMethod('length', 'A', function(x) 1L)\nmethods::selectMethod('length', 'A')\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let call = calls.last().unwrap();
    assert!(call.dispatch[0].complete, "{call:?}");
    assert_eq!(call.dispatch[0].arguments[0].argument.as_deref(), Some("x"));
    assert_eq!(call.resolutions[0].best.len(), 1, "{call:?}");
}

#[test]
fn simple_primitive_redeclarations_retain_existing_methods() {
    for declaration in [
        "methods::setGeneric('length')",
        "methods::setGeneric('length', signature = 'x')",
    ] {
        let source = format!(
            "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setMethod('length', 'Parent', function(x) 1L)\n{declaration}\nmethods::selectMethod('length', 'Child')\n"
        );
        let index = index(&source);
        let calls = selections(&index, Path::new("/s4.R"), &source);
        let resolution = &calls[0].resolutions[0];
        assert_eq!(resolution.best.len(), 1, "{resolution:?}");
        assert_eq!(defined(&index, &resolution.best[0]), ["Parent"]);
        assert!(resolution.signature.object.is_none(), "{resolution:?}");
    }
}

#[test]
fn simple_inheritance_excludes_coercing_relations() {
    let source = "methods::setClass('Child', slots = c(x = 'numeric'))\nmethods::setClass('Parent', slots = c(y = 'numeric'))\nmethods::setIs('Child', 'Parent', coerce = function(from) methods::new('Parent', y = from@x))\nmethods::setGeneric('simple', function(x) 0, simpleInheritanceOnly = TRUE)\nmethods::setMethod('simple', 'Parent', function(x) 1)\nmethods::setGeneric('ordinary', function(x) 0, simpleInheritanceOnly = FALSE)\nmethods::setMethod('ordinary', 'Parent', function(x) 1)\nmethods::selectMethod('simple', 'Child')\nmethods::selectMethod('ordinary', 'Child')\n";
    let index = index(source);
    let calls = selections(&index, Path::new("/s4.R"), source);
    assert!(
        matches!(
            calls[0].resolutions[0].selected,
            Some(S4MethodId::Default(_))
        ),
        "{calls:?}"
    );
    assert_eq!(
        defined(&index, calls[1].resolutions[0].selected.as_ref().unwrap()),
        ["Parent"]
    );
}

#[test]
fn unresolved_group_tables_prevent_a_definite_selection() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGroupGeneric('customGroup', function(x) standardGeneric('customGroup'))\nmethods::setGeneric('g', function(x) 0, group = 'customGroup')\nmethods::setMethod('customGroup', 'Parent', function(x) 1)\nmethods::selectMethod('g', 'Child')\n";
    let calls = selections(&index(source), Path::new("/s4.R"), source);
    assert!(calls[0].resolutions[0].selected.is_none(), "{calls:?}");
}

#[test]
fn omitted_use_inherited_preserves_the_coerce_destination() {
    let source = "methods::setClass('Source')\nmethods::setClass('TargetParent')\nmethods::setClass('TargetChild', contains = 'TargetParent')\nmethods::setAs('Source', 'TargetParent', function(from) methods::new('TargetParent'))\nmethods::setMethod('coerce', c('Source', 'TargetParent'), function(from, to, strict = TRUE) methods::new('TargetParent'))\nmethods::selectMethod('coerce', c('Source', 'TargetChild'), optional = TRUE)\nmethods::selectMethod('coerce', c('Source', 'TargetChild'), optional = TRUE, useInherited = TRUE)\n";
    let index = index(source);
    let calls = selections(&index, Path::new("/s4.R"), source);
    assert!(calls[0].resolutions[0].best.is_empty(), "{calls:?}");
    assert_eq!(
        defined(&index, &calls[1].resolutions[0].best[0]),
        ["Source", "TargetParent", "ANY"]
    );
}

#[test]
fn next_method_can_fall_back_to_the_original_default() {
    let source = "methods::setGeneric('g', function(x) 0)\nmethods::setMethod('g', 'ANY', function(x) methods::callNextMethod())\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let resolution = &calls[0].resolutions[0];
    assert!(
        matches!(resolution.selected, Some(S4MethodId::Default(_))),
        "{resolution:?}"
    );
}

#[test]
fn next_method_skips_the_active_signature_in_group_tables() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setMethod('+', c('Parent', 'Parent'), function(e1, e2) 1)\nmethods::setMethod('Arith', c('Child', 'Child'), function(e1, e2) 2)\nmethods::setMethod('+', c('Child', 'Child'), function(e1, e2) methods::callNextMethod())\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let resolution = &calls[0].resolutions[0];
    assert_eq!(resolution.best.len(), 1, "{resolution:?}");
    assert_eq!(defined(&index, &resolution.best[0]), ["Parent", "Parent"]);
}

#[test]
fn next_method_in_a_group_preserves_possible_member_continuations() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setMethod('+', c('Parent', 'Parent'), function(e1, e2) 1)\nmethods::setMethod('Arith', c('Parent', 'Parent'), function(e1, e2) 2)\nmethods::setMethod('Arith', c('Child', 'Child'), function(e1, e2) methods::callNextMethod())\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let call = &calls[0];
    for (member, provider) in [("+", "+"), ("-", "Arith")] {
        let id = |name: &str| S4GenericId::Named {
            name: name.into(),
            package: "base".into(),
        };
        let resolution = call
            .resolutions
            .iter()
            .find(|res| res.signature.generic == id(member))
            .unwrap_or_else(|| panic!("{call:?}"));
        assert_eq!(resolution.best.len(), 1, "{resolution:?}");
        assert!(
            resolution
                .candidates
                .iter()
                .any(|candidate| resolution.best.contains(&candidate.method)
                    && candidate.signature.generic == id(provider)),
            "{resolution:?}"
        );
    }
}

#[test]
fn next_method_resolves_from_the_declared_signature_and_preserves_storage() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::setMethod('g', 'Parent', function(x) 1)\nmethods::setMethod('g', 'Child', function(x) methods::callNextMethod('changed'))\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let next = calls
        .iter()
        .find(|call| call.kind == S4CallKind::NextMethod)
        .unwrap();
    let result = &next.resolutions[0];
    assert!(result.complete(), "{result:?}");
    assert_eq!(
        defined(&index, result.selected.as_ref().unwrap()),
        ["Parent"]
    );
}

#[test]
fn exact_selection_respects_use_inherited_and_top_level_registration_order() {
    let source = "methods::setClass('Parent')\nmethods::setClass('Child', contains = 'Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::selectMethod('g', 'Child')\nmethods::setMethod('g', 'Parent', function(x) 1)\nmethods::selectMethod('g', 'Child', useInherited = FALSE)\nmethods::selectMethod('g', 'Child', useInherited = flag)\n";
    let index = index(source);
    let parsed = parse(source);
    let calls = index.s4_dispatch_calls(
        Path::new("/s4.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let selections = calls
        .iter()
        .filter(|call| call.kind == S4CallKind::SelectMethod)
        .collect::<Vec<_>>();
    assert!(
        selections[0].resolutions[0].candidates.is_empty(),
        "{:?}",
        selections[0]
    );
    assert!(
        selections[1].resolutions[0].candidates.is_empty(),
        "{:?}",
        selections[1]
    );
    assert!(
        !selections[2].resolutions[0].complete(),
        "{:?}",
        selections[2]
    );
    assert!(selections[2].resolutions[0].selected.is_none());
}

#[test]
fn group_methods_use_member_precedence_and_group_ancestry() {
    let index = index(
        "methods::setClass('A')\n\
         methods::setMethod('Ops', c('A', 'A'), function(e1, e2) 1)\n\
         methods::setMethod('Arith', c('A', 'A'), function(e1, e2) 2)\n\
         methods::setMethod('+', c('A', 'A'), function(e1, e2) 3)\n",
    );
    let result = resolve(&index, "+", &["A", "A"]);
    assert_eq!(result.best.len(), 1, "{result:?}");
    let best = result
        .candidates
        .iter()
        .find(|candidate| result.best.contains(&candidate.method))
        .unwrap();
    assert!(best.group.is_none());
    let result = resolve(&index, "-", &["A", "A"]);
    assert_eq!(result.best.len(), 1, "{result:?}");
    let best = result
        .candidates
        .iter()
        .find(|candidate| result.best.contains(&candidate.method))
        .unwrap();
    assert_eq!(
        best.group,
        Some(S4GenericId::Named {
            package: "base".into(),
            name: "Arith".into()
        })
    );
}

#[test]
fn cycles_stay_incomplete() {
    let index = index(
        "methods::setClass('A')\nmethods::setClass('B', contains = 'A')\nmethods::setIs('A', 'B')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\nmethods::setMethod('g', 'B', function(x) 1)\n",
    );
    let result = resolve(&index, "g", &["A"]);
    assert!(!result.complete(), "{result:?}");
    assert!(result.selected.is_none());
}

#[test]
fn uncertain_generic_installations_do_not_prove_a_selected_method() {
    for declaration in [
        "if (flag) methods::setGeneric('g', function(x) standardGeneric('g'))",
        "methods::setGeneric('g', function(x) standardGeneric('g'), where = target)",
    ] {
        let index = index(&format!(
            "{declaration}\nmethods::setMethod('g', 'numeric', function(x) 1)\n"
        ));
        let result = resolve(&index, "g", &["numeric"]);
        assert!(!result.complete(), "{declaration}: {result:?}");
        assert!(result.selected.is_none());
    }
}

#[test]
fn local_generics_with_external_groups_keep_external_methods_incomplete() {
    let index = index(
        "methods::setGeneric('g', function(x) standardGeneric('g'), group = 'Math')\n\
         methods::setMethod('g', 'ANY', function(x) 1)\n",
    );
    let result = resolve(&index, "g", &["numeric"]);
    assert!(
        result
            .uncertainty
            .contains(&S4SelectionUncertainty::ExternalMethods),
        "{result:?}"
    );
    assert!(result.selected.is_none());
}

#[test]
fn reference_inheritance_preserves_the_distance_to_the_implicit_base() {
    let index = index(
        "Ref <- methods::setRefClass('Ref')\nChild <- methods::setRefClass('Child', contains = 'Ref')\n\
         methods::setGeneric('g', function(x) standardGeneric('g'))\n\
         methods::setMethod('g', 'envRefClass', function(x) 1)\n\
         methods::setMethod('g', 'Ref', function(x) 2)\n",
    );
    let result = resolve(&index, "g", &["Child"]);
    assert!(result.complete(), "{result:?}");
    assert_eq!(defined(&index, result.selected.as_ref().unwrap()), ["Ref"]);
    let base = result
        .candidates
        .iter()
        .find(|candidate| defined(&index, &candidate.method) == ["envRefClass"])
        .unwrap();
    assert_eq!(base.distances, [S4Distance::Class(2)]);
}

#[test]
fn pinned_r_inherited_selections_stay_among_the_best_static_candidates() {
    let source = include_str!("fixtures/s4_inheritance/input.R");
    let index = index(source);
    for row in include_str!("fixtures/s4_inheritance/expected.tsv").lines() {
        let columns = row.split('\t').collect::<Vec<_>>();
        match columns[0] {
            "selection" => {
                let result = resolve(
                    &index,
                    columns[2],
                    &columns[3].split(',').collect::<Vec<_>>(),
                );
                assert!(
                    result.candidates.iter().any(|candidate| result
                        .best
                        .contains(&candidate.method)
                        && defined(&index, &candidate.method).join(",") == columns[4]
                        && match &candidate.signature.generic {
                            S4GenericId::Named { name, .. } => name == columns[5],
                            _ => false,
                        }),
                    "{row}: {result:?}"
                );
                if let Some(selected) = &result.selected {
                    assert_eq!(defined(&index, selected).join(","), columns[4], "{row}");
                }
                if columns[6] == "TRUE" || columns[1] == "equal" {
                    assert!(
                        result
                            .uncertainty
                            .contains(&S4SelectionUncertainty::AmbiguousMethod),
                        "{row}: {result:?}"
                    );
                }
            }
            "next" => {
                let parsed = parse(source);
                let calls = index.s4_dispatch_calls(
                    Path::new("/s4.R"),
                    &parsed.cst,
                    &SemanticModel::build(&parsed.cst),
                );
                let next = calls
                    .iter()
                    .find(|call| call.kind == S4CallKind::NextMethod)
                    .unwrap();
                assert_eq!(columns[1], "root");
                assert_eq!(
                    defined(&index, next.resolutions[0].selected.as_ref().unwrap()),
                    ["DistanceRoot"]
                );
            }
            "storage" => {
                let class = index
                    .declarations
                    .iter()
                    .find(|(_, class)| {
                        class.declaration.name == ClassName::Known("RefChild".into())
                    })
                    .unwrap()
                    .0;
                let api = index.reference_api(class).unwrap();
                assert_eq!(
                    api.slots
                        .entries
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(","),
                    columns[1]
                );
                let fields = api
                    .instance
                    .entries
                    .iter()
                    .filter(|(name, _)| !name.starts_with('.'))
                    .filter(|(_, members)| {
                        members
                            .iter()
                            .any(|member| member.kind == MemberKind::Field)
                    })
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>()
                    .join(",");
                assert_eq!(fields, columns[2]);
            }
            _ => panic!("{row}"),
        }
    }
}

fn live_observations(script: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/s4_inheritance");
    let output =
        std::process::Command::new(std::env::var_os("RSCRIPT").unwrap_or_else(|| "Rscript".into()))
            .arg("--vanilla")
            .arg(root.join(script))
            .arg(&root)
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
#[ignore = "requires Rscript; checks inherited dispatch against the pinned R observations"]
fn live_r_s4_inheritance_observations() {
    assert_eq!(
        live_observations("observe.R"),
        include_str!("fixtures/s4_inheritance/expected.tsv")
    );
}

#[test]
#[ignore = "requires Rscript and a temporary R package library"]
fn live_r_s4_namespace_observations() {
    assert_eq!(
        live_observations("namespace.R"),
        include_str!("fixtures/s4_inheritance/namespace.tsv")
    );
}

#[test]
#[ignore = "requires the pinned R version; regenerates built-in metadata in a temporary directory"]
fn live_r_s4_builtin_metadata() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temporary = tempfile::tempdir().unwrap();
    let output =
        std::process::Command::new(std::env::var_os("RSCRIPT").unwrap_or_else(|| "Rscript".into()))
            .arg("--vanilla")
            .arg(root.join("scripts/dump_s4_builtins.R"))
            .arg(temporary.path())
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for name in [
        "classes.tsv",
        "generics.tsv",
        "formals.tsv",
        "callables.tsv",
    ] {
        assert_eq!(
            std::fs::read_to_string(temporary.path().join(name)).unwrap(),
            std::fs::read_to_string(root.join("src/project/classes/s4_dispatch").join(name))
                .unwrap(),
            "{name}"
        );
    }
}

#[test]
fn pinned_r_namespace_relationships_resolve_without_r() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/s4_inheritance/packages");
    let names = [
        "arityS4Owner",
        "arityS4Provider",
        "arityS4Client",
        "arityS4ClassClient",
    ];
    let sources = names
        .iter()
        .map(|name| std::fs::read_to_string(root.join(name).join("R/classes.R")).unwrap())
        .collect::<Vec<_>>();
    let namespaces = names
        .iter()
        .map(|name| std::fs::read_to_string(root.join(name).join("NAMESPACE")).unwrap())
        .collect::<Vec<_>>();
    let mut db = workspace_database(
        &names
            .iter()
            .enumerate()
            .map(|(i, name)| (*name, sources[i].as_str(), namespaces[i].as_str()))
            .collect::<Vec<_>>(),
    );
    for name in names {
        db.upsert_description(
            Path::new(&format!("/{name}")),
            std::fs::read_to_string(root.join(name).join("DESCRIPTION")).unwrap(),
        );
    }
    let index = project_classes(&db);
    let client = Path::new("/arityS4Client/R/classes.R");
    let parsed = parse(&sources[2]);
    let calls = index.s4_dispatch_calls(client, &parsed.cst, &SemanticModel::build(&parsed.cst));
    let call = &calls[0];
    let selected = call.resolutions[0].selected.as_ref().unwrap();
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| &method.id == selected)
        .unwrap();
    let observations = include_str!("fixtures/s4_inheritance/namespace.tsv")
        .lines()
        .map(|line| {
            let (name, values) = line.split_once('\t').unwrap();
            (name, values)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        call.generics,
        [S4GenericId::Named {
            package: observations["generic_owner"].into(),
            name: "g".into()
        }]
    );
    assert_eq!(
        method.owner.package.as_deref(),
        Some(observations["method_owner"])
    );
    assert_eq!(defined(index, selected), [observations["inherited"]]);
    assert_eq!(
        calls[1].generics,
        [S4GenericId::Named {
            package: observations["primitive_owner"].into(),
            name: "length".into()
        }]
    );
    assert_eq!(
        defined(index, &calls[1].resolutions[0].best[0]),
        [observations["primitive_method"]]
    );
    let owner = ClassOwner::default();
    let class = index.resolve_reference(client, &owner, &ParentRef::Formal("Parent".into()), true);
    assert_eq!(
        (!class.is_empty()).to_string().to_uppercase(),
        observations["class_import"]
    );
    let lexical = index.resolve_reference(
        client,
        &owner,
        &ParentRef::Generator(GeneratorRef::Free("Parent".into())),
        true,
    );
    assert_eq!(
        (!lexical.is_empty()).to_string().to_uppercase(),
        observations["class_value_import"]
    );
    let qualified = index.resolve_reference(
        client,
        &owner,
        &ParentRef::Generator(GeneratorRef::Qualified {
            package: "arityS4Owner".into(),
            name: "Parent".into(),
        }),
        true,
    );
    assert_eq!(
        (!qualified.is_empty()).to_string().to_uppercase(),
        observations["class_value_export"]
    );
    let namespace = index.s4_namespace(Path::new("/arityS4Client")).unwrap();
    assert_eq!(
        namespace.imported_methods["arityS4Provider"],
        BTreeSet::from(["g".into(), "length".into()])
    );
    assert_eq!(
        namespace.imported_classes["arityS4Owner"],
        BTreeSet::from(["Parent".into()])
    );
    let parsed = parse(&sources[3]);
    let calls = index.s4_dispatch_calls(
        Path::new("/arityS4ClassClient/R/classes.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let selected = calls[0].resolutions[0].selected.as_ref().unwrap();
    let method = index
        .s4
        .methods
        .iter()
        .find(|method| &method.id == selected)
        .unwrap();
    assert_eq!(
        format!(
            "{}\t{}",
            method.owner.package.as_deref().unwrap(),
            defined(index, selected).join(",")
        ),
        observations["loaded_method"]
    );
}

#[test]
fn inherited_candidates_refresh_for_declarations_and_namespace_edits_but_backdate_bodies() {
    use arity::incremental::QueryKind;
    let one =
        "methods::setClass('Parent')\nmethods::setGeneric('g', function(x) standardGeneric('g'))\n";
    let two = "methods::setMethod('g', 'Parent', function(x) 1)\n";
    let three = "methods::setClass('Child', contains = 'Parent')\nprobe <- function() methods::selectMethod('g', 'Child')\n";
    let entries = [
        ("one", one, "exportClasses(Parent)\nexport(g)\n"),
        (
            "two",
            two,
            "importClassesFrom(one, Parent)\nimportFrom(one, g)\nexportMethods(g)\n",
        ),
        (
            "three",
            three,
            "importClassesFrom(one, Parent)\nimportMethodsFrom(two, g)\n",
        ),
    ];
    let mut db = workspace_database(&entries);
    let before = project_classes(&db).clone();
    db.clear_query_log();
    db.upsert_file(
        Path::new("/two/R/classes.R"),
        two.replace("function(x) 1", "function(x) { x; 2 }"),
    );
    assert_eq!(&before, project_classes(&db));
    assert!(
        !db.query_log()
            .iter()
            .any(|query| query.kind == QueryKind::ProjectClasses)
    );
    db.clear_query_log();
    db.upsert_file(
        Path::new("/two/R/classes.R"),
        two.replace("'Parent'", "'numeric'"),
    );
    let parsed = parse(three);
    let calls = project_classes(&db).s4_dispatch_calls(
        Path::new("/three/R/classes.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    assert!(calls.last().unwrap().resolutions[0].candidates.is_empty());
    assert_eq!(
        db.query_log()
            .iter()
            .filter(|query| query.kind == QueryKind::ProjectClasses)
            .count(),
        1
    );
    db.upsert_file(Path::new("/two/R/classes.R"), two);
    db.set_package_metadata(
        entries
            .iter()
            .map(|(name, _, namespace)| PackageInfo {
                root: format!("/{name}").into(),
                namespace: Some(if *name == "three" {
                    "importClassesFrom(one, Parent)\n".into()
                } else {
                    (*namespace).into()
                }),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect(),
    );
    let index = project_classes(&db);
    let calls = index.s4_dispatch_calls(
        Path::new("/three/R/classes.R"),
        &parsed.cst,
        &SemanticModel::build(&parsed.cst),
    );
    let call = calls.last().unwrap();
    assert_eq!(
        call.generics,
        [S4GenericId::Named {
            package: "one".into(),
            name: "g".into()
        }]
    );
    assert!(call.resolutions[0].candidates.is_empty());
    assert_eq!(
        index
            .resolve_reference(
                Path::new("/three/R/classes.R"),
                &ClassOwner::default(),
                &ParentRef::Formal("Parent".into()),
                true
            )
            .len(),
        1
    );
}
