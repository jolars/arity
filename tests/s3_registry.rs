use arity::incremental::IncrementalDatabase;
use arity::parser::parse;
use arity::project::classes::*;
use arity::semantic::SemanticModel;
use std::path::Path;

fn index(source: &str) -> ClassIndex {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.cst.text().to_string(), source);
    let model = SemanticModel::build(&parsed.cst);
    let facts = file_class_defs(&model, &parsed.cst);
    IncrementalDatabase::default().snapshot().class_index_in(
        Path::new("/s3.R"),
        &facts,
        &parsed.cst,
        &model,
    )
}

#[test]
fn known_generics_and_dotted_classes_do_not_depend_on_the_last_dot() {
    let index = index(
        "as.data.frame.my.class <- function(x) x\nOps.my.class <- function(e1, e2) e1\n`[<-.my.class` <- function(x, i, value) x\nunknown.my.class <- function(x) x\n",
    );
    for (generic, class, kind) in [
        ("as.data.frame", "my.class", S3GenericKind::Known),
        ("Ops", "my.class", S3GenericKind::Group),
        ("[<-", "my.class", S3GenericKind::Internal),
    ] {
        let method = index
            .s3
            .methods
            .iter()
            .find(|method| {
                method.generic.name == ClassName::Known(generic.into())
                    && method.class == ClassName::Known(class.into())
            })
            .unwrap();
        assert!(method.generics.iter().any(|id| {
            index
                .s3
                .generics
                .iter()
                .any(|def| &def.id == id && def.kind == kind)
        }));
    }
    assert!(
        index
            .s3
            .methods
            .iter()
            .any(|method| method.generics.is_empty()
                && method.uncertainty.contains(&OopUncertainty::UnprovenMethod))
    );
    assert!(index.declarations.is_empty());
}

#[test]
fn registrations_preserve_the_implementation_and_lexical_owner() {
    let index = index(
        "g <- function(x) UseMethod('g')\nhandler <- function(x) x\n.S3method('g', 'a.b', 'handler')\ng.C <- handler\ninstall <- function() base::registerS3method('g', 'C', 'g.C')\n",
    );
    let method = index
        .s3
        .methods
        .iter()
        .find(|method| method.class == ClassName::Known("a.b".into()))
        .unwrap();
    assert_eq!(
        method.implementation,
        Some(MethodImplementationSource::Binding {
            owner: ClassOwner::default(),
            name: "handler".into()
        })
    );
    assert_eq!(method.generics.len(), 1);
    assert!(!method.exported);
    let deferred = index.s3.methods.iter().find(|method| matches!(&method.source, S3MethodSource::Declaration(id) if !id.key.owner.0.is_empty())).unwrap();
    assert!(!deferred.owner.lexical.0.is_empty());
}

#[test]
fn masks_and_dynamic_registration_keep_uncertainty() {
    let index = index(
        "UseMethod <- function(...) NULL\ng <- function(x) UseMethod('g')\ng.C <- function(x) x\nregister <- function(...) NULL\nregister('g', 'C', g.C)\nbase::registerS3method(runtime_generic, runtime_class, runtime_method, envir = target)\n",
    );
    assert!(!index.s3.generics.iter().any(|generic| generic.name == "g"));
    let method = index
        .s3
        .methods
        .iter()
        .find(|method| method.generic.name == ClassName::Unknown)
        .unwrap();
    assert!(method.generics.is_empty());
    assert!(method.uncertainty.contains(&OopUncertainty::DynamicName));
    assert!(
        method
            .uncertainty
            .contains(&OopUncertainty::CustomEnvironment)
    );
}

#[test]
fn registrations_respect_lexical_masks_of_known_generics() {
    for source in [
        "install <- function() { print <- function(x) x; base::registerS3method('print', 'C', function(x) x) }",
        "install <- function(print) base::registerS3method('print', 'C', function(x) x)",
        "install <- function() { print <- function(x) x; base::.S3method('print', 'C', function(x) x) }",
        "print <- function(x) UseMethod('print'); install <- function() { print <- function(x) x; base::registerS3method('print', 'C', function(x) x) }",
    ] {
        let index = index(source);
        let method = index
            .s3
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap();
        assert!(
            method.generics.is_empty(),
            "{source}: {:?}",
            method.generics
        );
        assert!(method.uncertainty.contains(&OopUncertainty::UnprovenMethod));
    }
}

#[test]
fn registrations_retain_proven_local_and_qualified_generic_identities() {
    let local = index(
        "install <- function() { print <- function(x) UseMethod('print'); base::registerS3method('print', 'C', function(x) x) }",
    );
    let method = local
        .s3
        .methods
        .iter()
        .find(|method| method.registered)
        .unwrap();
    assert!(matches!(
        method.generics.as_slice(),
        [S3GenericId::Declaration(_)]
    ));

    for source in [
        "install <- function() { print <- base::print; base::registerS3method('print', 'C', function(x) x) }",
        "print <- function(x) UseMethod('print'); install <- function() { print <- base::print; base::registerS3method('print', 'C', function(x) x) }",
    ] {
        let qualified = index(source);
        let method = qualified
            .s3
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap();
        assert_eq!(
            method.generics,
            vec![S3GenericId::Known {
                package: "base".into(),
                name: "print".into()
            }],
            "{source}"
        );
    }
}

#[test]
fn registrations_preserve_attachment_order_and_uncertainty() {
    for (source, known) in [
        (
            "library(maskpkg); base::registerS3method('print', 'C', function(x) x)",
            false,
        ),
        (
            "if (flag) library(maskpkg); base::registerS3method('print', 'C', function(x) x)",
            false,
        ),
        (
            "base::registerS3method('print', 'C', function(x) x); library(maskpkg)",
            true,
        ),
        (
            "install <- function() base::registerS3method('print', 'C', function(x) x); library(maskpkg)",
            false,
        ),
    ] {
        let index = index(source);
        let method = index
            .s3
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap();
        assert_eq!(!method.generics.is_empty(), known, "{source}");
        assert_eq!(
            method.uncertainty.contains(&OopUncertainty::UnprovenMethod),
            !known,
            "{source}"
        );
    }
}

#[test]
fn ordinary_bindings_do_not_mask_group_generic_identities() {
    for name in ["Ops", "Math", "Summary", "Complex", "matrixOps"] {
        for declaration in [
            format!("{name}.C <- function(x) x"),
            format!("base::registerS3method('{name}', 'C', function(x) x)"),
        ] {
            let index = index(&format!("{name} <- function(...) NULL; {declaration}"));
            let method = index
                .s3
                .methods
                .iter()
                .find(|method| method.class == ClassName::Known("C".into()))
                .unwrap();
            assert_eq!(
                method.generics,
                vec![S3GenericId::Known {
                    package: "base".into(),
                    name: name.into()
                }]
            );
            assert!(!method.uncertainty.contains(&OopUncertainty::UnprovenMethod));
        }
    }
}

#[test]
fn exported_implementation_is_the_resolved_lexical_binding() {
    use arity::project::{PackageInfo, project_classes};
    use std::collections::BTreeSet;

    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(
        Path::new("/pkg/R/input.R"),
        "g <- function(x) UseMethod('g')\nhandler <- function(x) x\n.onLoad <- function(lib, pkg) {\n  handler <- function(x) x + 1\n  base::registerS3method('g', 'local', handler)\n  base::registerS3method('g', 'quoted', 'handler')\n}\ninstall <- function() base::registerS3method('g', 'public', handler)\n",
    );
    db.set_workspace_members(vec![file], vec!["/pkg".into()]);
    db.set_package_metadata(vec![PackageInfo {
        root: "/pkg".into(),
        namespace: Some("export(handler)\n".into()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    let index = project_classes(&db);
    for class in ["local", "quoted", "public"] {
        let method = index
            .s3
            .methods
            .iter()
            .find(|method| method.class == ClassName::Known(class.into()))
            .unwrap();
        assert_eq!(method.exported, class == "public", "{class}");
        let Some(MethodImplementationSource::Binding { owner, name }) = &method.implementation
        else {
            panic!(
                "missing implementation binding: {:?}",
                method.implementation
            );
        };
        assert_eq!(name, "handler");
        assert_eq!(owner.0.is_empty(), class == "public");
        assert!(!method.owner.lexical.0.is_empty());
    }
}

#[test]
fn parentheses_preserve_supplied_implementation_identity() {
    for (value, expected) in [
        (
            "(handler)",
            MethodImplementationSource::Binding {
                owner: ClassOwner::default(),
                name: "handler".into(),
            },
        ),
        (
            "(('handler'))",
            MethodImplementationSource::Binding {
                owner: ClassOwner::default(),
                name: "handler".into(),
            },
        ),
        ("((function(x) x))", MethodImplementationSource::Inline),
        (
            "((base::identity))",
            MethodImplementationSource::Qualified {
                package: "base".into(),
                name: "identity".into(),
            },
        ),
    ] {
        let index = index(&format!(
            "g <- function(x) UseMethod('g'); handler <- function(x) x; base::.S3method('g', 'C', {value})"
        ));
        let method = index
            .s3
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap();
        assert_eq!(method.implementation.as_ref(), Some(&expected), "{value}");
    }
}

#[test]
fn implicit_implementation_uses_the_supplied_generic_name() {
    let index = index(
        "g <- function(x) UseMethod('dispatch'); g.C <- function(x) x; base::.S3method('g', 'C')",
    );
    let method = index
        .s3
        .methods
        .iter()
        .find(|method| method.registered)
        .unwrap();
    assert_eq!(method.generic.name, ClassName::Known("g".into()));
    assert_eq!(
        method.implementation,
        Some(MethodImplementationSource::Binding {
            owner: ClassOwner::default(),
            name: "g.C".into()
        })
    );
}

#[test]
fn registry_matches_pinned_r_registration_observations() {
    use arity::project::{PackageInfo, project_classes};
    use std::collections::BTreeSet;
    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(
        Path::new("/fixture/R/input.R"),
        include_str!("fixtures/s3_registry/input.R"),
    );
    db.set_workspace_members(vec![file], vec!["/fixture".into()]);
    db.set_package_metadata(vec![PackageInfo {
        root: "/fixture".into(),
        namespace: Some(include_str!("fixtures/s3_registry/NAMESPACE").into()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    let registry = &project_classes(&db).s3;
    let mut rows = Vec::new();
    for method in registry.methods.iter().filter(|method| method.registered) {
        let ClassName::Known(generic) = &method.generic.name else {
            panic!("unknown generic")
        };
        let ClassName::Known(class) = &method.class else {
            panic!("unknown class")
        };
        let binding = match method.implementation.as_ref().unwrap() {
            MethodImplementationSource::Binding { name, .. }
            | MethodImplementationSource::Reference { name } => name,
            other => panic!("unexpected implementation: {other:?}"),
        };
        let source = if matches!(method.source, S3MethodSource::Namespace { .. }) {
            "namespace"
        } else {
            "script"
        };
        rows.push(format!(
            "{source}\t{generic}\t{class}\t{binding}\t{}",
            method.generic.package.as_deref().unwrap_or("-")
        ));
        assert_eq!(method.delayed, method.generic.package.is_some());
        assert_eq!(method.exported, binding == "handler");
    }
    rows.sort();
    let mut expected: Vec<_> = include_str!("fixtures/s3_registry/expected.tsv")
        .lines()
        .map(String::from)
        .collect();
    expected.sort();
    assert_eq!(rows, expected);
}

#[test]
#[ignore = "requires Rscript; compares R's namespace parser and registration table"]
fn live_r_registration_observations() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/s3_registry");
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
        include_str!("fixtures/s3_registry/expected.tsv")
    );
}

#[test]
fn namespace_duplicates_and_conditions_retain_separate_registration_sites() {
    use arity::rindex::harvest::parse_namespace;
    let info = parse_namespace(
        "S3method(pkg::g, C, handler)\nS3method(pkg::g, C, handler)\nif (flag) S3method(g, C, alternative)\nS3method(g, , missing_class)\nS3method(g, C, build())\n",
        &[],
    );
    assert_eq!(info.s3_registrations.len(), 3);
    assert_eq!(info.s3_registrations[0], info.s3_registrations[1]);
    assert!(info.s3_registrations[2].conditional);
    assert!(info.exports.is_empty());
    assert!(!info.s3_methods.contains("missing_class"));
}

#[test]
fn supplied_alias_binding_is_distinct_from_its_closure_origin() {
    let index = index(
        "g <- function(x) UseMethod('g')\nhandler <- function(x) x\nalias <- handler\nbase::registerS3method('g', 'C', alias)\n",
    );
    let method = index
        .s3
        .methods
        .iter()
        .find(|method| method.registered)
        .unwrap();
    assert_eq!(
        method.implementation,
        Some(MethodImplementationSource::Binding {
            owner: ClassOwner::default(),
            name: "alias".into()
        })
    );
    let source = index.oop_files().next().unwrap();
    assert!(source.facts.oop.method_implementations.iter().any(|implementation| matches!(&implementation.source, MethodImplementationSource::Binding { name, .. } if name == "handler")));
}

#[test]
fn unconfirmed_class_vectors_never_become_a_definite_registration_class() {
    for source in [
        "g <- function(x) UseMethod('g'); c <- function(...) 'other'; base::registerS3method('g', c('C'), function(x) x)",
        "g <- function(x) UseMethod('g'); base::registerS3method('g', c('C', 'D'), function(x) x)",
    ] {
        let index = index(source);
        let method = index
            .s3
            .methods
            .iter()
            .find(|method| method.registered)
            .unwrap();
        assert_eq!(method.class, ClassName::Unknown, "{source}");
        assert!(
            method
                .uncertainty
                .contains(&OopUncertainty::DynamicSignature)
        );
    }
}
