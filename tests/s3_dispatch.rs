use std::path::Path;

use arity::ast::CallExpr;
use arity::incremental::IncrementalDatabase;
use arity::parser::parse;
use arity::project::classes::*;
use arity::semantic::SemanticModel;
use rowan::ast::AstNode as _;

fn dispatch(source: &str) -> S3Dispatch {
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
    let probe = root
        .descendants()
        .filter_map(CallExpr::cast)
        .find(|call| call.callee_name().as_deref() == Some("probe"))
        .unwrap();
    let value = probe
        .arg_list()
        .unwrap()
        .args()
        .next()
        .unwrap()
        .value()
        .unwrap();
    index
        .s3_dispatch_at(path, &root, &model, &value)
        .expect("S3 dispatch")
}

fn selected(result: &S3Dispatch) -> Option<&str> {
    match result.selected.as_ref()?.implementation.as_ref()? {
        MethodImplementationSource::Binding { name, .. }
        | MethodImplementationSource::Reference { name } => Some(name),
        _ => None,
    }
}

#[test]
fn ordered_classes_and_default_are_separate_dispatch_steps() {
    let declarations = "g <- function(x, ...) UseMethod('g')\n\
        g.parent <- function(x, ...) x\n\
        g.child <- function(x, ...) x\n\
        g.default <- function(x, ...) x\n";
    for (classes, method) in [
        ("c('child', 'parent')", "g.child"),
        ("c('parent', 'child')", "g.parent"),
        ("c('absent', 'parent')", "g.parent"),
        ("'absent'", "g.default"),
    ] {
        let result = dispatch(&format!(
            "{declarations}probe(g(structure(list(), class = {classes})))"
        ));
        assert_eq!(selected(&result), Some(method), "{result:#?}");
    }
}

#[test]
fn unknown_classes_and_runtime_registration_never_select_an_implementation() {
    for source in [
        "g <- function(x) UseMethod('g'); g.C <- function(x) x; g.default <- function(x) x; probe(g(x))",
        "g <- function(x) UseMethod('g'); g.C <- function(x) x; install <- function() registerS3method('g', cls, handler); probe(g(structure(list(), class = 'C')))",
    ] {
        let result = dispatch(source);
        assert!(result.selected.is_none(), "{result:#?}");
        assert!(!result.candidates.is_empty());
        assert!(!result.uncertainty.is_empty());
    }
}

#[test]
fn missing_workspace_methods_do_not_prove_a_library_generic_has_no_method() {
    let result = dispatch("probe(print(structure(list(), class = 'external')))");
    assert!(result.selected.is_none());
    assert_eq!(result.fallback, S3Fallback::Unknown);
}

#[test]
fn library_methods_for_earlier_classes_prevent_later_selection() {
    for (classes, method) in [
        ("c('data.frame', 'B')", "print.B"),
        ("'data.frame'", "print.default"),
    ] {
        let result = dispatch(&format!(
            "{method} <- function(...) NULL; probe(print(structure(list(a = 1), class = {classes}, row.names = '1')))"
        ));
        assert!(result.selected.is_none(), "{result:#?}");
        assert!(!result.candidates.is_empty(), "{result:#?}");
        assert!(!result.uncertainty.is_empty(), "{result:#?}");
    }
}

#[test]
fn known_generics_match_their_dispatch_arguments() {
    for (generic, actuals) in [
        ("print", "extra = a, x = b"),
        ("print", "x = b, extra = a"),
        ("round", "digits = a, x = b"),
        ("round", "dig = a, x = b"),
        ("signif", "digits = a, x = b"),
    ] {
        let result = dispatch(&format!(
            "{generic}.A <- function(...) NULL; {generic}.B <- function(...) NULL; a <- structure(1, class = 'A'); b <- structure(1, class = 'B'); probe({generic}({actuals}))"
        ));
        assert_eq!(
            selected(&result),
            Some(format!("{generic}.B").as_str()),
            "{result:#?}"
        );
    }
    let result = dispatch(
        "as.data.frame.A <- function(...) NULL; as.data.frame.B <- function(...) NULL; probe(as.data.frame(extra = structure(1, class = 'A'), x = structure(1, class = 'B')))",
    );
    assert_ne!(selected(&result), Some("as.data.frame.A"), "{result:#?}");
}

#[test]
fn dotted_generic_and_dotted_class_resolve_as_identities() {
    let result = dispatch(
        "as.data.frame.a.b <- function(x, ...) x; probe(as.data.frame(structure(list(), class = 'a.b')))",
    );
    assert_eq!(selected(&result), Some("as.data.frame.a.b"));
}

#[test]
fn deferred_calls_keep_file_methods_visible_and_runtime_callees_uncertain() {
    let result = dispatch(
        "g <- function(x) UseMethod('g'); f <- function() probe(g(structure(list(), class = 'C'))); g.C <- function(x) x",
    );
    assert_eq!(selected(&result), Some("g.C"));
    let result = dispatch(
        "g <- function(x) UseMethod('g'); g.C <- function(x) x; f <- function(g) probe(g(structure(list(), class = 'C')))",
    );
    assert!(result.selected.is_none());
    assert!(
        result
            .uncertainty
            .contains(&S3DispatchUncertainty::AmbiguousGeneric)
    );
}

#[test]
fn internal_generics_require_an_explicit_class() {
    let result =
        dispatch("length.C <- function(x) 1; probe(length(structure(list(), class = 'C')))");
    assert_eq!(selected(&result), Some("length.C"));
    let result = dispatch("length.integer <- function(x) 1; probe(length(1L))");
    assert!(result.candidates.is_empty());
    assert_eq!(result.fallback, S3Fallback::Internal);
}

#[test]
fn internal_aliases_keep_entry_points_separate_from_dispatch_names() {
    for (call, expected) in [("as.numeric", "as.double.C"), ("seq.int", "seq.C")] {
        let result = dispatch(&format!(
            "as.numeric.C <- function(...) NULL; as.double.C <- function(...) NULL; seq.int.C <- function(...) NULL; seq.C <- function(...) NULL; probe({call}(structure(1, class = 'C')))"
        ));
        assert_eq!(selected(&result), Some(expected), "{result:#?}");
        assert_ne!(result.dispatch_name, call);
        assert_eq!(result.fallback, S3Fallback::Internal);
    }
}

#[test]
fn binding_generics_search_later_arguments_and_use_the_first_method() {
    for generic in ["cbind", "rbind"] {
        for (arguments, expected) in [("1, x", "C"), ("x, y", "C"), ("y, x", "D")] {
            let result = dispatch(&format!(
                "{generic}.C <- function(...) NULL; {generic}.D <- function(...) NULL; x <- structure(1, class = 'C'); y <- structure(1, class = 'D'); probe({generic}({arguments}))"
            ));
            assert_eq!(
                selected(&result),
                Some(format!("{generic}.{expected}").as_str()),
                "{result:#?}"
            );
        }
        let result = dispatch(&format!(
            "{generic}.default <- function(...) NULL; probe({generic}(structure(1, class = 'absent')))"
        ));
        assert!(result.candidates.is_empty());
        assert_eq!(result.fallback, S3Fallback::Internal);
    }
}

#[test]
fn group_methods_and_member_methods_follow_class_order() {
    for (classes, method) in [
        ("c('child', 'parent')", "Math.child"),
        ("c('parent', 'child')", "abs.parent"),
    ] {
        let result = dispatch(&format!(
            "Math.child <- function(x, ...) x; abs.parent <- function(x) x; probe(abs(structure(1, class = {classes})))"
        ));
        assert_eq!(selected(&result), Some(method), "{result:#?}");
    }
    let result = dispatch(
        "Math.C <- function(x, ...) x; abs.C <- function(x) x; probe(abs(structure(1, class = 'C')))",
    );
    assert_eq!(selected(&result), Some("abs.C"));
}

#[test]
fn summary_complex_and_matrix_groups_keep_their_dispatch_arguments() {
    for (generic, method, expression) in [
        ("Summary", "Summary.C", "sum(x, 1)"),
        ("Complex", "Complex.C", "Re(x)"),
        ("matrixOps", "matrixOps.C", "1 %*% x"),
    ] {
        let result = dispatch(&format!(
            "{generic}.C <- function(...) NULL; x <- structure(1, class = 'C'); probe({expression})"
        ));
        assert_eq!(selected(&result), Some(method), "{result:#?}");
    }
    let result = dispatch(
        "Summary.C <- function(...) NULL; x <- structure(1, class = 'C'); probe(sum(1, x))",
    );
    assert!(result.candidates.is_empty());
    assert_eq!(result.fallback, S3Fallback::Internal);
}

#[test]
fn operators_dispatch_on_both_operands_and_keep_conflicts_uncertain() {
    for expression in ["x + 1", "1 + x", "+x", "`+`(right = x, left = 1)"] {
        let result = dispatch(&format!(
            "Ops.C <- function(e1, e2) e1; x <- structure(1, class = 'C'); probe({expression})"
        ));
        assert_eq!(selected(&result), Some("Ops.C"), "{result:#?}");
    }
    let result = dispatch(
        "`+.A` <- function(e1, e2) e1; `+.B` <- function(e1, e2) e2; probe(structure(1, class = 'A') + structure(2, class = 'B'))",
    );
    assert!(result.selected.is_none());
    assert_eq!(result.candidates.len(), 2);
    assert!(
        result
            .uncertainty
            .contains(&S3DispatchUncertainty::ConflictingOperands)
    );
}

#[test]
fn eager_dispatch_preserves_effects_between_operands() {
    for expression in [
        "{ y <- structure(1, class = 'B'); 1 } + y",
        "`+`({ y <- structure(1, class = 'B'); 1 }, y)",
        "{ class(y) <- 'B'; 1 } + y",
        "cbind({ y <- structure(1, class = 'B'); 1 }, y)",
    ] {
        let result = dispatch(&format!(
            "Ops.A <- function(...) NULL; Ops.B <- function(...) NULL; cbind.A <- function(...) NULL; cbind.B <- function(...) NULL; y <- structure(1, class = 'A'); probe({expression})"
        ));
        let expected = if expression.starts_with("cbind") {
            "cbind.B"
        } else {
            "Ops.B"
        };
        assert_eq!(
            selected(&result),
            Some(expected),
            "{expression}: {result:#?}"
        );
    }
    let result = dispatch(
        "Ops.A <- function(...) NULL; Ops.B <- function(...) NULL; y <- structure(1, class = 'A'); probe(y + { y <- structure(1, class = 'B'); 1 })",
    );
    assert_eq!(selected(&result), Some("Ops.A"), "{result:#?}");
    let result = dispatch(
        "Ops.A <- function(...) NULL; Ops.B <- function(...) NULL; y <- structure(new.env(), class = 'A'); probe(y + { class(y) <- 'B'; 1 })",
    );
    assert!(result.selected.is_none(), "{result:#?}");
    assert!(
        result
            .uncertainty
            .contains(&S3DispatchUncertainty::UnknownClass)
    );
    let result = dispatch(
        "round.A <- function(...) NULL; round.B <- function(...) NULL; y <- structure(1, class = 'A'); probe(round(digits = { y <- structure(1, class = 'B'); 1 }, x = y))",
    );
    assert_eq!(selected(&result), Some("round.B"), "{result:#?}");
}

#[test]
fn subset_and_replacement_methods_use_their_own_generics() {
    for (expression, method) in [
        ("x[1]", "[.C"),
        ("x[1] <- 2", "[<-.C"),
        ("x$field <- 2", "$<-.C"),
    ] {
        let result = dispatch(&format!(
            "`[.C` <- function(x, ...) x; `[<-.C` <- function(x, ..., value) x; `$<-.C` <- function(x, name, value) x; x <- structure(list(), class = 'C'); probe({expression})"
        ));
        assert_eq!(selected(&result), Some(method), "{result:#?}");
    }
}

#[test]
fn next_method_keeps_the_original_class_vector_and_advances_by_occurrence() {
    let parsed = parse(
        "g <- function(x) UseMethod('g'); g.C <- function(x) NextMethod(); g.D <- function(x) x; g.default <- function(x) x",
    );
    let model = SemanticModel::build(&parsed.cst);
    let path = Path::new("/s3.R");
    let index = IncrementalDatabase::default().snapshot().class_index_in(
        path,
        &file_class_defs(&model, &parsed.cst),
        &parsed.cst,
        &model,
    );
    let id = &index
        .s3
        .generics
        .iter()
        .find(|generic| generic.name == "g")
        .unwrap()
        .id;
    let context = S3DispatchContext::new(path);
    let classes = [S3ClassEvidence::Explicit(vec![
        "C".into(),
        "C".into(),
        "D".into(),
    ])];
    let first = index.s3_dispatch(&context, id, &classes);
    assert_eq!(selected(&first), Some("g.C"));
    let second = index.s3_next_method(&context, &first, first.selected.as_ref().unwrap(), None);
    assert_eq!(selected(&second), Some("g.C"));
    assert_eq!(second.selected.as_ref().unwrap().class_index, Some(1));
    let third = index.s3_next_method(&context, &second, second.selected.as_ref().unwrap(), None);
    assert_eq!(selected(&third), Some("g.D"));
    let default = index.s3_next_method(&context, &third, third.selected.as_ref().unwrap(), None);
    assert_eq!(selected(&default), Some("g.default"));
    let exhausted =
        index.s3_next_method(&context, &default, default.selected.as_ref().unwrap(), None);
    assert!(exhausted.candidates.is_empty());
    assert_eq!(exhausted.fallback, S3Fallback::NoMethod);
}

#[test]
fn use_method_forwards_original_promises_and_rematches_expanded_names() {
    let result = dispatch(
        "g <- function(alpha, beta = 1, ...) { alpha <- structure(list(), class = 'changed'); local <- 1; UseMethod('g') }; g.C <- function(alpha, beta, ...) alpha; g.changed <- function(...) NULL; probe(g(be = 2, al = structure(list(), class = 'C'), extra = 3))",
    );
    assert_eq!(selected(&result), Some("g.C"));
    let forwarding = result.forwarding.unwrap();
    assert!(forwarding.nonreturning);
    assert!(!forwarding.uncertain);
    assert_eq!(
        forwarding.arguments,
        vec![
            S3ForwardedArgument {
                name: Some("alpha".into()),
                source: S3ArgumentSource::Original { argument: 1 }
            },
            S3ForwardedArgument {
                name: Some("beta".into()),
                source: S3ArgumentSource::Original { argument: 0 }
            },
            S3ForwardedArgument {
                name: Some("extra".into()),
                source: S3ArgumentSource::Original { argument: 2 }
            },
        ]
    );
    assert!(!forwarding.generic_locals.contains_key("alpha"));
    assert_eq!(
        forwarding.generic_locals["local"],
        arity::semantic::enclosure::Availability::Unknown
    );
}

#[test]
fn use_method_dispatches_on_the_first_supplied_argument_after_matching() {
    for (formals, actuals) in [
        ("x, y", "y = object"),
        ("...", "object"),
        ("x, y", "y = 1, x = object"),
    ] {
        let result = dispatch(&format!(
            "g <- function({formals}) UseMethod('g'); g.C <- function(...) NULL; object <- structure(list(), class = 'C'); probe(g({actuals}))"
        ));
        assert_eq!(selected(&result), Some("g.C"), "{result:#?}");
    }
}

#[test]
fn explicit_use_method_object_uses_the_current_generic_frame() {
    let result = dispatch(
        "g <- function(x) { x <- structure(list(), class = 'changed'); UseMethod('g', x) }; g.C <- function(x) x; g.changed <- function(x) x; probe(g(structure(list(), class = 'C')))",
    );
    assert_eq!(selected(&result), Some("g.changed"), "{result:#?}");
}

#[test]
fn implicit_use_method_loses_evidence_after_reference_mutations() {
    for mutation in [
        "class(x) <- 'B'",
        "alias <- x; class(alias) <- 'B'",
        "structure(x, class = 'B')",
    ] {
        let result = dispatch(&format!(
            "g <- function(x) {{ {mutation}; UseMethod('g') }}; g.A <- function(...) NULL; g.B <- function(...) NULL; probe(g(structure(new.env(), class = 'A')))"
        ));
        assert_ne!(selected(&result), Some("g.A"), "{mutation}: {result:#?}");
        if result.selected.is_none() {
            assert!(!result.uncertainty.is_empty(), "{result:#?}");
        }
    }
    let result = dispatch(
        "g <- function(x) { x <- structure(list(), class = 'B'); UseMethod('g') }; g.A <- function(...) NULL; g.B <- function(...) NULL; probe(g(structure(new.env(), class = 'A')))",
    );
    assert_eq!(selected(&result), Some("g.A"), "{result:#?}");
}

#[test]
fn generic_locals_follow_the_r_44_boundary_without_forwarding_defaults() {
    use arity::semantic::enclosure::Availability;
    let signature = CallableSignature {
        parameters: vec![
            CallableParameter {
                name: "x".into(),
                default: None,
            },
            CallableParameter {
                name: "y".into(),
                default: Some("1".into()),
            },
        ],
    };
    for (version, availability) in [
        (S3RVersion::Before44, Availability::Present),
        (S3RVersion::Since44, Availability::Absent),
        (S3RVersion::Unknown, Availability::Unknown),
    ] {
        let forwarding = S3Forwarding::use_method(
            &signature,
            &[None],
            &[("local".into(), Availability::Present)].into(),
            version,
        );
        assert_eq!(forwarding.arguments.len(), 1);
        assert_eq!(forwarding.generic_locals["local"], availability);
    }
}

#[test]
fn generic_local_forwarding_excludes_formals_and_actuals() {
    use arity::semantic::enclosure::Availability;
    let signature = CallableSignature {
        parameters: ["x", "..."]
            .map(|name| CallableParameter {
                name: name.into(),
                default: None,
            })
            .into(),
    };
    let locals = [
        ("x".into(), Availability::Present),
        ("supplied".into(), Availability::Present),
        ("conditional".into(), Availability::Unknown),
    ]
    .into();
    let forwarding = S3Forwarding::use_method(
        &signature,
        &[None, Some("supplied".into())],
        &locals,
        S3RVersion::Before44,
    );
    assert_eq!(
        forwarding.generic_locals,
        [("conditional".into(), Availability::Unknown)].into()
    );
}

#[test]
fn next_method_uses_current_formals_and_preserves_dots_promises() {
    let signature = CallableSignature {
        parameters: ["x", "y", "..."]
            .map(|name| CallableParameter {
                name: name.into(),
                default: None,
            })
            .into(),
    };
    let incoming = S3Forwarding::use_method(
        &signature,
        &[None, Some("y".into()), Some("extra".into())],
        &Default::default(),
        S3RVersion::Since44,
    );
    let next = incoming.next_method(&signature, &[Some("y".into()), None, Some("new".into())]);
    assert!(!next.nonreturning);
    assert!(!next.uncertain);
    assert_eq!(
        next.arguments
            .iter()
            .map(|arg| &arg.source)
            .collect::<Vec<_>>(),
        vec![
            &S3ArgumentSource::CurrentFormal { name: "x".into() },
            &S3ArgumentSource::NextArgument { argument: 0 },
            &S3ArgumentSource::Original { argument: 2 },
            &S3ArgumentSource::NextArgument { argument: 1 },
            &S3ArgumentSource::NextArgument { argument: 2 },
        ]
    );
}

#[test]
fn script_registration_is_visible_after_installation_and_keeps_its_alias() {
    let result = dispatch(
        "g <- function(x) UseMethod('g'); handler <- function(x) x; alias <- handler; .S3method('g', 'C', alias); probe(g(structure(list(), class = 'C')))",
    );
    assert_eq!(selected(&result), Some("alias"), "{result:#?}");
    let result = dispatch(
        "g <- function(x) UseMethod('g'); handler <- function(x) x; probe(g(structure(list(), class = 'C'))); .S3method('g', 'C', handler)",
    );
    assert!(result.candidates.is_empty(), "{result:#?}");
}

#[test]
fn namespace_registration_crosses_package_boundaries_without_exporting_the_method() {
    use arity::project::PackageInfo;
    use std::collections::BTreeSet;
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(
        Path::new("/a/R/g.R"),
        "g <- function(x) UseMethod('g'); handler <- function(x) x; g.C <- function(x) x",
    );
    let source = "probe(g(structure(list(), class = 'C')))";
    let b = db.upsert_file(Path::new("/b/R/call.R"), source);
    db.set_workspace_members(vec![a, b], vec!["/a".into(), "/b".into()]);
    db.refresh_descriptions(["/a".into(), "/b".into()]);
    db.upsert_description(Path::new("/a"), "Package: a\nVersion: 1.0\n");
    db.upsert_description(Path::new("/b"), "Package: b\nVersion: 1.0\nImports: a\n");
    let metadata = |namespace: &str| {
        vec![
            PackageInfo {
                root: "/a".into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            },
            PackageInfo {
                root: "/b".into(),
                namespace: Some("importFrom(a, g)\n".into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            },
        ]
    };
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let value = parsed
        .cst
        .descendants()
        .filter_map(CallExpr::cast)
        .find(|call| call.callee_name().as_deref() == Some("g"))
        .unwrap()
        .syntax()
        .clone()
        .into();
    db.set_package_metadata(metadata("export(g)\n"));
    let index = arity::project::project_classes(&db);
    let result = index
        .s3_dispatch_at(Path::new("/b/R/call.R"), &parsed.cst, &model, &value)
        .unwrap();
    assert!(
        result.candidates.is_empty(),
        "private methods in the generic namespace are not caller-visible: {result:#?}"
    );
    db.set_package_metadata(metadata("export(g)\nS3method(g, C, handler)\n"));
    let result = arity::project::project_classes(&db)
        .s3_dispatch_at(Path::new("/b/R/call.R"), &parsed.cst, &model, &value)
        .unwrap();
    assert_eq!(selected(&result), Some("handler"), "{result:#?}");
    assert_eq!(
        result.selected.unwrap().visibility,
        S3MethodVisibility::Registered
    );
}

#[test]
fn namespace_loading_does_not_attach_exported_methods() {
    use arity::project::PackageInfo;
    use std::collections::BTreeSet;

    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(
        Path::new("/a/R/g.R"),
        "g <- function(x) UseMethod('g'); g.C <- function(x) x",
    );
    let path = Path::new("/b/R/call.R");
    let b = db.upsert_file(path, "");
    db.set_workspace_members(vec![a, b], vec!["/a".into(), "/b".into()]);
    db.refresh_descriptions(["/a".into(), "/b".into()]);
    db.upsert_description(Path::new("/a"), "Package: a\nVersion: 1.0\n");
    db.upsert_description(Path::new("/b"), "Package: b\nVersion: 1.0\nImports: a\n");
    db.set_package_metadata(vec![
        PackageInfo {
            root: "/a".into(),
            namespace: Some("export(g)\nexport(g.C)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        },
        PackageInfo {
            root: "/b".into(),
            namespace: Some("importFrom(a, g)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        },
    ]);
    for (load, expected) in [
        ("", "g.default"),
        ("requireNamespace('a');", "g.default"),
        ("loadNamespace('a');", "g.default"),
        ("library(a);", "g.C"),
    ] {
        let source = format!(
            "{load} g.default <- function(x) x; probe(g(base::structure(base::list(), class = 'C')))"
        );
        db.set_file_text(b, source.as_str());
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let value = parsed
            .cst
            .descendants()
            .filter_map(CallExpr::cast)
            .find(|call| call.callee_name().as_deref() == Some("g"))
            .unwrap()
            .syntax()
            .clone()
            .into();
        let result = arity::project::project_classes(&db)
            .s3_dispatch_at(path, &parsed.cst, &model, &value)
            .unwrap();
        assert_eq!(selected(&result), Some(expected), "{load}: {result:#?}");
    }
}

#[test]
fn namespace_registered_implementations_require_their_actual_binding() {
    use arity::project::PackageInfo;
    use std::collections::BTreeSet;

    let source = "g <- function(x) UseMethod('g'); handler <- function(x) NextMethod(); g.D <- function(x) x; wrapper <- function() { handler <- function() NextMethod(); handler() }; probe(g(structure(1, class = c('C', 'D'))))";
    let path = Path::new("/a/R/g.R");
    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(path, source);
    db.set_workspace_members(vec![file], vec!["/a".into()]);
    db.refresh_descriptions(["/a".into()]);
    db.upsert_description(Path::new("/a"), "Package: a\nVersion: 1.0\n");
    db.set_package_metadata(vec![PackageInfo {
        root: "/a".into(),
        namespace: Some("export(g)\nS3method(g, C, handler)\n".into()),
        dir_sources: BTreeSet::new(),
        native_routines: BTreeSet::new(),
    }]);
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let index = arity::project::project_classes(&db);
    let value = parsed
        .cst
        .descendants()
        .filter_map(CallExpr::cast)
        .find(|call| call.callee_name().as_deref() == Some("g"))
        .unwrap()
        .syntax()
        .clone()
        .into();
    let first = index
        .s3_dispatch_at(path, &parsed.cst, &model, &value)
        .unwrap();
    assert_eq!(selected(&first), Some("handler"), "{first:#?}");
    let context = index.s3_dispatch_context_at(path, &parsed.cst, &model, &value);
    let calls: Vec<_> = parsed
        .cst
        .descendants()
        .filter_map(CallExpr::cast)
        .filter(|call| call.callee_name().as_deref() == Some("NextMethod"))
        .collect();
    assert_eq!(calls.len(), 2);
    for (call, registered_frame) in calls.iter().zip([true, false]) {
        let next = index.s3_next_method_at(
            &context,
            &first,
            first.selected.as_ref().unwrap(),
            S3MethodFrame {
                path,
                root: &parsed.cst,
                model: &model,
            },
            call,
        );
        if registered_frame {
            assert_eq!(selected(&next), Some("g.D"), "{next:#?}");
        } else {
            assert!(next.selected.is_none(), "{next:#?}");
            assert!(
                next.uncertainty
                    .contains(&S3DispatchUncertainty::UnknownContinuation)
            );
        }
    }
}

#[test]
fn inherited_signature_safety_gates_and_next_method_formal_reads_stay_in_place() {
    use arity::project::promises::file_promise_seeds;
    let source =
        "g <- function(alpha) UseMethod('g'); g.C <- function(alpha) { alpha <- 2; NextMethod() }";
    let parsed = parse(source);
    let model = SemanticModel::build(&parsed.cst);
    let seeds = file_promise_seeds(&model, &parsed.cst);
    assert!(!seeds.functions["g"].signature_known);
    assert!(!seeds.functions["g.C"].signature_known);
    let reassigned = model
        .bindings()
        .iter()
        .enumerate()
        .find(|(_, binding)| {
            binding.name == "alpha" && binding.kind == arity::semantic::BindingKind::Local
        })
        .unwrap()
        .0;
    assert!(
        !model
            .unused_local_bindings()
            .any(|id| model.binding(id).def_range == model.bindings()[reassigned].def_range)
    );
}

#[test]
fn next_method_handles_internal_fallbacks_groups_and_current_metadata() {
    for (generic, first, next, classes) in [
        ("length", "length.C", None, vec!["C"]),
        ("abs", "Math.C", Some("abs.D"), vec!["C", "D"]),
        ("log10", "Math.C", Some("Math.D"), vec!["C", "D"]),
    ] {
        let parsed = parse(
            "length.C <- function(x) NextMethod(); Math.C <- function(x, ...) NextMethod(); abs.D <- function(x) x; Math.D <- function(x, ...) x",
        );
        let model = SemanticModel::build(&parsed.cst);
        let path = Path::new("/s3.R");
        let index = IncrementalDatabase::default().snapshot().class_index_in(
            path,
            &file_class_defs(&model, &parsed.cst),
            &parsed.cst,
            &model,
        );
        let context = S3DispatchContext::new(path);
        let id = S3GenericId::Known {
            package: "base".into(),
            name: generic.into(),
        };
        let first_result = index.s3_dispatch(
            &context,
            &id,
            &[S3ClassEvidence::Explicit(
                classes.into_iter().map(String::from).collect(),
            )],
        );
        assert_eq!(selected(&first_result), Some(first));
        let result = index.s3_next_method(
            &context,
            &first_result,
            first_result.selected.as_ref().unwrap(),
            None,
        );
        assert_eq!(selected(&result), next);
        assert_eq!(result.fallback, S3Fallback::Internal);
    }
    for (write, uncertain) in [
        ("class(x) <- 'changed'", false),
        (".Class <- 'changed'", true),
    ] {
        let source = format!(
            "g <- function(x, y, ...) UseMethod('g'); g.C <- function(x, y, ...) {{ {write}; y <- 'current'; NextMethod(y = 'extra', added = 1) }}; g.D <- function(x, y, ...) x; probe(g(structure(1, class = c('C', 'D')), y = 2, kept = 3))"
        );
        let parsed = parse(&source);
        let model = SemanticModel::build(&parsed.cst);
        let path = Path::new("/s3.R");
        let index = IncrementalDatabase::default().snapshot().class_index_in(
            path,
            &file_class_defs(&model, &parsed.cst),
            &parsed.cst,
            &model,
        );
        let value = parsed
            .cst
            .descendants()
            .filter_map(CallExpr::cast)
            .find(|call| call.callee_name().as_deref() == Some("g"))
            .unwrap()
            .syntax()
            .clone()
            .into();
        let first = index
            .s3_dispatch_at(path, &parsed.cst, &model, &value)
            .unwrap();
        let call = parsed
            .cst
            .descendants()
            .filter_map(CallExpr::cast)
            .find(|call| call.callee_name().as_deref() == Some("NextMethod"))
            .unwrap();
        let context = index.s3_dispatch_context_at(path, &parsed.cst, &model, &value);
        let next = index.s3_next_method_at(
            &context,
            &first,
            first.selected.as_ref().unwrap(),
            S3MethodFrame {
                path,
                root: &parsed.cst,
                model: &model,
            },
            &call,
        );
        assert_eq!(next.selected.is_none(), uncertain, "{next:#?}");
        assert_eq!(
            next.forwarding.as_ref().unwrap().arguments[1].source,
            S3ArgumentSource::NextArgument { argument: 0 }
        );
        if uncertain {
            assert!(
                next.uncertainty
                    .contains(&S3DispatchUncertainty::UnknownContinuation)
            );
        } else {
            assert_eq!(selected(&next), Some("g.D"));
        }
    }
}

#[test]
fn next_method_keeps_the_selected_operator_operand_and_replacement_vector() {
    let parsed = parse(
        "Ops.C <- function(e1, e2) NextMethod(); Ops.D <- function(e1, e2) e1; `[<-.C` <- function(x, ..., value) NextMethod(); `[<-.D` <- function(x, ..., value) x",
    );
    let model = SemanticModel::build(&parsed.cst);
    let path = Path::new("/s3.R");
    let index = IncrementalDatabase::default().snapshot().class_index_in(
        path,
        &file_class_defs(&model, &parsed.cst),
        &parsed.cst,
        &model,
    );
    let context = S3DispatchContext::new(path);
    for (generic, classes, expected) in [
        (
            "+",
            vec![
                S3ClassEvidence::NoExplicitClass,
                S3ClassEvidence::Explicit(vec!["C".into(), "D".into()]),
            ],
            "Ops.D",
        ),
        (
            "[<-",
            vec![S3ClassEvidence::Explicit(vec!["C".into(), "D".into()])],
            "[<-.D",
        ),
    ] {
        let result = index.s3_dispatch(
            &context,
            &S3GenericId::Known {
                package: "base".into(),
                name: generic.into(),
            },
            &classes,
        );
        let next = index.s3_next_method(&context, &result, result.selected.as_ref().unwrap(), None);
        assert_eq!(selected(&next), Some(expected), "{next:#?}");
    }
}

#[test]
fn next_method_inside_an_anonymous_helper_has_no_proven_method_frame() {
    let parsed = parse(
        "g <- function(x) UseMethod('g'); g.C <- function(x) (function() NextMethod())(); g.D <- function(x) x; probe(g(structure(1, class = c('C', 'D'))))",
    );
    let model = SemanticModel::build(&parsed.cst);
    let path = Path::new("/s3.R");
    let index = IncrementalDatabase::default().snapshot().class_index_in(
        path,
        &file_class_defs(&model, &parsed.cst),
        &parsed.cst,
        &model,
    );
    let value = parsed
        .cst
        .descendants()
        .filter_map(CallExpr::cast)
        .find(|call| call.callee_name().as_deref() == Some("g"))
        .unwrap()
        .syntax()
        .clone()
        .into();
    let first = index
        .s3_dispatch_at(path, &parsed.cst, &model, &value)
        .unwrap();
    let next_call = parsed
        .cst
        .descendants()
        .filter_map(CallExpr::cast)
        .find(|call| call.callee_name().as_deref() == Some("NextMethod"))
        .unwrap();
    let context = index.s3_dispatch_context_at(path, &parsed.cst, &model, &value);
    let next = index.s3_next_method_at(
        &context,
        &first,
        first.selected.as_ref().unwrap(),
        S3MethodFrame {
            path,
            root: &parsed.cst,
            model: &model,
        },
        &next_call,
    );
    assert!(next.selected.is_none());
    assert!(
        next.uncertainty
            .contains(&S3DispatchUncertainty::UnknownContinuation)
    );
}

#[test]
fn later_static_registrations_replace_earlier_ones_and_lexical_methods_win() {
    let result = dispatch(
        "g <- function(x) UseMethod('g'); one <- function(x) x; two <- function(x) x; .S3method('g', 'C', one); .S3method('g', 'C', two); probe(g(structure(list(), class = 'C')))",
    );
    assert_eq!(selected(&result), Some("two"), "{result:#?}");
    let result = dispatch(
        "g <- function(x) UseMethod('g'); g.C <- function(x) x; handler <- function(x) x; .S3method('g', 'C', handler); probe(g(structure(list(), class = 'C')))",
    );
    assert_eq!(selected(&result), Some("g.C"), "{result:#?}");
}

#[test]
fn dispatch_matches_pinned_r_observations() {
    let source = include_str!("fixtures/s3_dispatch/input.R");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty());
    let probes: Vec<_> = parsed
        .cst
        .descendants()
        .filter_map(CallExpr::cast)
        .filter(|call| call.callee_name().as_deref() == Some("probe"))
        .collect();
    let expected = include_str!("fixtures/s3_dispatch/expected.tsv");
    for probe in &probes {
        // Each observation starts with the declarations and one dispatch. The
        // test recorder's other calls are not part of that call's effect prefix.
        let mut bytes = source.as_bytes().to_vec();
        for other in probes
            .iter()
            .filter(|other| other.syntax() != probe.syntax())
        {
            for byte in &mut bytes[std::ops::Range::<usize>::from(other.syntax().text_range())] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
        }
        let text = String::from_utf8(bytes).unwrap();
        let parsed = parse(&text);
        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.cst.text().to_string(), text);
        let model = SemanticModel::build(&parsed.cst);
        let path = Path::new("/s3.R");
        let index = IncrementalDatabase::default().snapshot().class_index_in(
            path,
            &file_class_defs(&model, &parsed.cst),
            &parsed.cst,
            &model,
        );
        let probe = parsed
            .cst
            .descendants()
            .filter_map(CallExpr::cast)
            .find(|call| call.callee_name().as_deref() == Some("probe"))
            .unwrap();
        let args: Vec<_> = probe.arg_list().unwrap().args().collect();
        let name = args[0]
            .value()
            .unwrap()
            .to_string()
            .trim_matches('"')
            .to_string();
        let mut value = args[1].value().unwrap();
        if name == "replacement" {
            value = value
                .as_node()
                .unwrap()
                .descendants()
                .filter_map(CallExpr::cast)
                .find(|call| call.callee_name().as_deref() == Some("`[<-`"))
                .unwrap()
                .syntax()
                .clone()
                .into();
        }
        let result = index
            .s3_dispatch_at(path, &parsed.cst, &model, &value)
            .unwrap();
        let method = if matches!(name.as_str(), "internal-default" | "group-parent") {
            // R can select these candidates, but this registry cannot rule out
            // an external method for an earlier class.
            assert!(result.selected.is_none(), "{name}: {result:#?}");
            assert_eq!(
                result.uncertainty,
                [S3DispatchUncertainty::ExternalMethods].into()
            );
            result.candidates.first().and_then(|candidate| {
                match candidate.implementation.as_ref()? {
                    MethodImplementationSource::Binding { name, .. } => Some(name.as_str()),
                    _ => None,
                }
            })
        } else {
            selected(&result)
        };
        let mut method = method
            .unwrap_or_else(|| panic!("{name}: {result:#?}"))
            .to_string();
        if name == "next-method" {
            let context = index.s3_dispatch_context_at(path, &parsed.cst, &model, &value);
            let next =
                index.s3_next_method(&context, &result, result.selected.as_ref().unwrap(), None);
            method.push(',');
            method.push_str(selected(&next).unwrap());
        }
        assert!(
            expected
                .lines()
                .any(|row| row == format!("{name}\t{method}")),
            "{name}: {method}"
        );
    }
}

#[test]
#[ignore = "requires Rscript; verifies dispatch, promise forwarding, and the R 4.4 frame change"]
fn live_r_dispatch_observations() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/s3_dispatch");
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
        include_str!("fixtures/s3_dispatch/expected.tsv")
    );
}
