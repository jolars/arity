//! Editor consumers share the project class index, including live-buffer overlays.

use super::*;
use crate::config::CompatVersion;
use crate::project::classes::{ClassIndex, file_class_defs};

#[derive(Clone, Copy)]
pub(super) struct ReadContext<'a> {
    pub snapshot: &'a Analysis,
    pub path: &'a Path,
    pub s7_floor: Option<&'a CompatVersion>,
}

pub(super) fn with_live_model<T>(
    root: &SyntaxNode,
    context: Option<ReadContext<'_>>,
    compute: impl Fn(&Analysis, &SemanticModel, &Path) -> T,
) -> T {
    let path = context.map_or(Path::new("buffer.R"), |context| context.path);
    let floor = context.and_then(|context| context.s7_floor);
    if let Some(ReadContext { snapshot, .. }) = context {
        let result = salsa::Cancelled::catch(AssertUnwindSafe(|| {
            let model = snapshot.semantic_model_in_with_floor(path, root, floor);
            compute(snapshot, &model, path)
        }));
        if let Ok(result) = result {
            return result;
        }
    }
    let snapshot = IncrementalDatabase::default().snapshot();
    let model = snapshot.semantic_model_in_with_floor(path, root, floor);
    compute(&snapshot, &model, path)
}

pub(super) fn with_class_index<T>(
    root: &SyntaxNode,
    context: Option<ReadContext<'_>>,
    compute: impl Fn(&ClassIndex, &SemanticModel, &Path) -> T,
) -> T {
    with_live_model(root, context, |snapshot, model, path| {
        let facts = file_class_defs(model, root);
        let index = snapshot.class_index_in(path, &facts, root, model);
        compute(&index, model, path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::PackageInfo;
    use crate::text::TextBuffer;
    use std::collections::BTreeSet;

    #[test]
    fn known_receivers_never_borrow_usage_from_unknown_branches() {
        let source = "A <- R6::R6Class('A', public = list(`odd name` = 1))\nobj <- if (flag) A$new() else unknown\nobj$guessed\nobj$";
        let CompletionResponse::List(list) =
            completion::compute_completions(source, source.len(), &IndexedProvider::empty())
                .unwrap()
        else {
            panic!("expected a completion list");
        };
        assert!(!list.items.iter().any(|item| item.label == "guessed"));
        let member = list
            .items
            .iter()
            .find(|item| item.label == "odd name")
            .unwrap();
        assert_eq!(member.insert_text.as_deref(), Some("`odd name`"));
    }

    #[test]
    fn source_generics_show_formals_and_only_proven_dispatch() {
        for (source, label, selected) in [
            (
                "describe <- function(x, ...) UseMethod('describe')\ndescribe.demo <- function(x, ..., extra = 1) x\ndescribe(structure(1, class = 'demo'))",
                "describe(x, ...)",
                true,
            ),
            (
                "describe <- function(x, ...) UseMethod('describe')\ndescribe.demo <- function(x, ..., extra = 1) x\ndescribe(unknown)",
                "describe(x, ...)",
                false,
            ),
            (
                "methods::setClass('Demo', slots = c(x = 'numeric'))\nmethods::setGeneric('describe', function(x, scale = 1) standardGeneric('describe'))\nmethods::setMethod('describe', 'Demo', function(x, scale) x)\ndescribe(methods::new('Demo'))",
                "describe(x, scale = 1)",
                true,
            ),
            (
                "methods::setGeneric('describe', function(x, scale = 1) standardGeneric('describe'))\ndescribe(",
                "describe(x, scale = 1)",
                false,
            ),
            (
                "methods::setGeneric('describe', function(x) standardGeneric('describe'))\nmethods::setMethod('describe', 'missing', function(x) 0)\ndescribe(",
                "describe(x)",
                false,
            ),
        ] {
            let offset = source.rfind("describe(").unwrap() + "describe(".len();
            let help = signature::compute_signature_help(source, offset, &IndexedProvider::empty())
                .unwrap_or_else(|| panic!("no generic signature for {source}"));
            let [info] = help.signatures.as_slice() else {
                panic!("expected the generic as the only signature");
            };
            assert_eq!(info.label, label);
            assert_eq!(help.active_parameter, Some(0));
            assert_eq!(
                matches!(&info.documentation, Some(Documentation::MarkupContent(markdown))
                    if markdown.value.contains("Selected method")),
                selected,
                "{source}"
            );
        }
    }

    #[test]
    fn project_generic_signatures_respect_exports_and_imports() {
        let mut db = IncrementalDatabase::default();
        let provider = Path::new("/generic-provider/R/describe.R");
        let consumer = Path::new("/generic-consumer/R/use.R");
        let provider_file = db.upsert_file(
            provider,
            "describe <- function(x, ..., scale = 1) UseMethod('describe')\n",
        );
        let consumer_file = db.upsert_file(consumer, "describe(");
        db.set_workspace_members(
            vec![provider_file, consumer_file],
            vec!["/generic-provider".into(), "/generic-consumer".into()],
        );
        db.refresh_descriptions(["/generic-provider".into(), "/generic-consumer".into()]);
        db.upsert_description(Path::new("/generic-provider"), "Package: provider\n");
        db.upsert_description(
            Path::new("/generic-consumer"),
            "Package: consumer\nImports: provider\n",
        );
        for exported in [true, false] {
            db.set_package_metadata(
                [
                    (
                        "/generic-provider",
                        if exported { "export(describe)" } else { "" },
                    ),
                    ("/generic-consumer", "importFrom(provider, describe)"),
                ]
                .into_iter()
                .map(|(root, namespace)| PackageInfo {
                    root: root.into(),
                    namespace: Some(namespace.into()),
                    dir_sources: BTreeSet::new(),
                    native_routines: BTreeSet::new(),
                })
                .collect(),
            );
            for callee in ["describe", "provider::describe"] {
                let text = format!("{callee}(");
                let buffer = TextBuffer::new(text.clone());
                let position = buffer
                    .line_index()
                    .byte_to_position(text.len(), PositionEncoding::Utf16);
                let help = signature::signature_help_via_db(
                    &db.snapshot(),
                    consumer,
                    &buffer,
                    position,
                    PositionEncoding::Utf16,
                    None,
                );
                let expected = format!("{callee}(x, ..., scale = 1)");
                assert_eq!(
                    help.as_ref().map(|help| help.signatures[0].label.as_str()),
                    exported.then_some(expected.as_str()),
                    "{text}",
                );
            }
        }
    }

    #[test]
    fn installed_inherited_members_resolve_the_declaring_help_topic() {
        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let package = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let indexed = IndexedProvider::from_indices([package]);
        let source = "obj <- oopfixture::Counter$new()\nobj$";
        let CompletionResponse::List(list) =
            completion::compute_completions(source, source.len(), &indexed).unwrap()
        else {
            panic!("expected a completion list");
        };
        assert!(!list.items.iter().any(|item| item.label == "secret"));
        let item = list
            .items
            .into_iter()
            .find(|item| item.label == "base_add")
            .unwrap();
        assert_eq!(item.kind, Some(CompletionItemKind::METHOD));
        assert!(
            completion::resolve_completion(item, &indexed)
                .documentation
                .is_some()
        );
        let call = format!("{source}base_add(");
        let help = signature::compute_signature_help(&call, call.len(), &indexed).unwrap();
        assert_eq!(help.signatures[0].label, "base_add(n = 2)");
        assert!(help.signatures[0].documentation.is_some());

        let call = "oopfixture::area(";
        let help = signature::compute_signature_help(call, call.len(), &indexed).unwrap();
        assert_eq!(help.signatures[0].label, "oopfixture::area(x, scale = 1)");
    }

    #[test]
    fn names_only_imports_can_make_installed_receiver_identity_ambiguous() {
        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        use crate::rindex::remote::RemoteExports;

        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let package = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let mut db = IncrementalDatabase::default();
        db.set_library_index(IndexedProvider::from_indices([package]));
        let path = Path::new("/consumer/R/use.R");
        let source = "obj <- Counter$new()\nobj$";
        let file = db.upsert_file(path, source);
        db.set_workspace_members(vec![file], vec!["/consumer".into()]);
        db.upsert_description(
            Path::new("/consumer"),
            "Package: consumer\nImports: oopfixture, other\n",
        );
        db.set_package_metadata(vec![PackageInfo {
            root: "/consumer".into(),
            namespace: Some("import(oopfixture)\nimport(other)".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let buffer = TextBuffer::new(source.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(source.len(), PositionEncoding::Utf16);
        for ambiguous in [false, true] {
            let mut remote = RemoteExports::new();
            remote.insert_package("other", ambiguous.then(|| "Counter".into()));
            db.set_remote_exports(remote);
            let CompletionResponse::List(list) = completion::completion_via_db(
                &db.snapshot(),
                path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap() else {
                panic!("expected a completion list");
            };
            assert_eq!(
                list.items.iter().any(|item| item.label == "base_add"),
                !ambiguous
            );
        }
    }

    #[test]
    fn s4_constructor_signature_lists_inherited_slots_in_an_open_call() {
        let source = "methods::setClass('Parent', slots = c(size = 'numeric'))\nmethods::setClass('Child', contains = 'Parent', slots = c(label = 'character'))\nmethods::new('Child', ";
        let help =
            signature::compute_signature_help(source, source.len(), &IndexedProvider::empty())
                .unwrap();
        let info = &help.signatures[0];
        assert_eq!(info.label, "methods::new(Class, ...)");
        assert_eq!(help.active_parameter, Some(1));
        let Some(Documentation::MarkupContent(markdown)) = &info.documentation else {
            panic!("expected declared slot documentation");
        };
        assert!(markdown.value.contains("size"));
        assert!(markdown.value.contains("label"));
    }

    #[test]
    fn canceled_read_keeps_the_description_floor() {
        use std::cell::Cell;

        let package = Path::new("/s7-cancel");
        let path = package.join("R/point.R");
        let root = parse("S7::`:=`(Point, S7::new_class())").cst;
        let mut db = IncrementalDatabase::default();
        db.refresh_descriptions([package.into()]);
        db.upsert_description(package, "Package: shapes\nImports: S7 (>= 0.2.2.9000)\n");
        let floor =
            crate::project::package_facts_for(&db, &path).and_then(|facts| facts.compat.s7.clone());
        let context = ReadContext {
            snapshot: &db.snapshot(),
            path: &path,
            s7_floor: floor.as_ref(),
        };
        let first = Cell::new(true);
        let bound = with_live_model(&root, Some(context), |_, model, _| {
            if first.replace(false) {
                std::panic::resume_unwind(Box::new(salsa::Cancelled::PendingWrite));
            }
            model
                .bindings()
                .iter()
                .any(|binding| binding.name == "Point")
        });
        assert!(bound);
    }

    #[test]
    fn s7_bare_completion_uses_confirmed_named_bindings() {
        let path = Path::new("/s7-bare.R");
        let source = "library(S7)\nPoint := new_class()\nPoi";
        let buffer = TextBuffer::new(source.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(source.len(), PositionEncoding::Utf16);
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(path, buffer.text_arc());
        let floor = CompatVersion::parse("0.2.2.9000");
        db.set_s7_floor(file, floor.clone());
        let CompletionResponse::List(items) = completion::completion_via_db(
            &db.snapshot(),
            path,
            &buffer,
            position,
            PositionEncoding::Utf16,
            floor.as_ref(),
        )
        .unwrap() else {
            panic!("completion list");
        };
        assert!(items.items.iter().any(|item| item.label == "Point"));
    }

    #[test]
    fn s7_dev_names_and_properties_follow_live_buffers() {
        let path = Path::new("/s7-live.R");
        let source = "library(S7)\nPoint := S7::new_class(properties = list(x = S7::class_integer))\nobj <- Point()\nobj@";
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(path, source);
        let floor = CompatVersion::parse("0.2.2.9000");
        db.set_s7_floor(file, floor.clone());
        for (text, expected) in [
            (source.to_string(), "x"),
            (source.replace("x =", "label ="), "label"),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let snapshot = db.snapshot();
            let got = completion::completion_via_db(
                &snapshot,
                path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                floor.as_ref(),
            )
            .unwrap();
            let CompletionResponse::List(items) = got else {
                panic!("completion list")
            };
            assert!(
                items.items.iter().any(|item| item.label == expected
                    && item.kind == Some(CompletionItemKind::PROPERTY))
            );
            read_jobs::with_document_semantics(
                &snapshot,
                path,
                &buffer,
                floor.as_ref(),
                |_, model| {
                    assert!(
                        model
                            .bindings()
                            .iter()
                            .any(|binding| binding.name == "Point")
                    );
                },
            );
        }
    }

    #[test]
    fn s4_slot_completion_uses_imports_and_live_buffer_class_facts() {
        let mut db = IncrementalDatabase::default();
        let parent_path = Path::new("/s4-parent/R/classes.R");
        let child_path = Path::new("/s4-child/R/classes.R");
        let parent = db.upsert_file(
            parent_path,
            "methods::setClass('Inner', slots = c(hidden = 'numeric'))\nMaker <- methods::setClass('Display', slots = c(value = 'numeric', inner = 'Inner'))\n",
        );
        let source = "Child <- methods::setClass('Child', contains = 'Display', slots = c(own = 'logical'))\nobj <- Child()\nobj@";
        let child = db.upsert_file(child_path, source);
        db.set_workspace_members(
            vec![parent, child],
            vec!["/s4-parent".into(), "/s4-child".into()],
        );
        db.refresh_descriptions(["/s4-parent".into(), "/s4-child".into()]);
        db.upsert_description(Path::new("/s4-parent"), "Package: parent\n");
        db.upsert_description(Path::new("/s4-child"), "Package: child\nImports: parent\n");
        let metadata = |exported| {
            [
                (
                    "/s4-parent",
                    if exported {
                        "exportClasses(Display)\nexport(Maker)\n"
                    } else {
                        ""
                    },
                ),
                (
                    "/s4-child",
                    "importClassesFrom(parent, Display)\nimportFrom(parent, Maker)\n",
                ),
            ]
            .into_iter()
            .map(|(root, namespace)| PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect()
        };
        db.set_package_metadata(metadata(true));
        for (text, expected) in [
            (source.to_string(), vec!["inner", "own", "value"]),
            (
                source.replace("own = 'logical'", "edited = 'character'"),
                vec!["edited", "inner", "value"],
            ),
            ("parent::Maker()@".to_string(), vec!["inner", "value"]),
            (
                "methods::new('Display')@".to_string(),
                vec!["inner", "value"],
            ),
            ("methods::new('Display')@inner@".to_string(), vec!["hidden"]),
            ("methods::new('Inner')@".to_string(), vec![]),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let CompletionResponse::List(list) = completion::completion_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap() else {
                panic!("completion list");
            };
            assert!(!list.is_incomplete, "{text}: {list:?}");
            assert_eq!(
                list.items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{text}"
            );
        }
        db.set_package_metadata(metadata(false));
        let text = "methods::new('Display')@";
        let buffer = TextBuffer::new(text.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(text.len(), PositionEncoding::Utf16);
        let CompletionResponse::List(list) = completion::completion_via_db(
            &db.snapshot(),
            child_path,
            &buffer,
            position,
            PositionEncoding::Utf16,
            None,
        )
        .unwrap() else {
            panic!("completion list");
        };
        assert!(list.items.is_empty());
    }
    #[test]
    fn s4_completion_recovers_named_method_frames_from_live_buffers() {
        let package = Path::new("/s4-method-frame");
        let path = package.join("R/implementation.R");
        let source = "impl <- function(x) x@first\nalias <- impl\nother <- function(x) x@first\n";
        let mut db = IncrementalDatabase::default();
        let implementation = db.upsert_file(&path, source);
        let registration = db.upsert_file(
            &package.join("R/methods.R"),
            "methods::setClass('A', slots = c(first = 'numeric', second = 'numeric'))\n\
             methods::setGeneric('inspect', function(x) standardGeneric('inspect'))\n\
             methods::setMethod('inspect', 'A', alias)\n",
        );
        db.set_workspace_members(vec![implementation, registration], vec![package.into()]);
        db.set_package_metadata(vec![PackageInfo {
            root: package.into(),
            namespace: None,
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let edited = source.replace(
            "impl <- function(x) x@first",
            "impl <- (function(x) x@first)",
        );
        for text in [
            source.to_string(),
            format!("# A live edit shifts the callable.\n{edited}"),
        ] {
            let buffer = TextBuffer::new(text);
            for ((offset, _), registered) in buffer.text().match_indices('@').zip([true, false]) {
                let position = buffer
                    .line_index()
                    .byte_to_position(offset + 1, PositionEncoding::Utf16);
                let CompletionResponse::List(list) = completion::completion_via_db(
                    &db.snapshot(),
                    &path,
                    &buffer,
                    position,
                    PositionEncoding::Utf16,
                    None,
                )
                .unwrap() else {
                    panic!("completion list");
                };
                assert_eq!(
                    list.items.iter().any(|item| item.label == "second"),
                    registered
                );
            }
        }
    }

    #[test]
    fn generator_mutations_use_live_signatures_and_keep_older_instances() {
        let mut db = IncrementalDatabase::default();
        let definitions = db.upsert_file(Path::new("/mutation-live/R/a.R"), "G <- R6::R6Class(public = list(read = function(original) original))\nRef <- methods::setRefClass('LiveMutation', methods = list(read = function(original) original))\n");
        let path = Path::new("/mutation-live/R/b.R");
        let mutations = db.upsert_file(path, "G$set('public', 'read', function(stored) stored, overwrite = TRUE)\nRef$methods(read = function(stored) stored)\n");
        db.set_workspace_members(vec![definitions, mutations], vec!["/mutation-live".into()]);
        db.set_package_metadata(vec![PackageInfo {
            root: "/mutation-live".into(),
            namespace: None,
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        for (text, expected) in [
            (
                "G$set('public', 'read', function(live = 2) live, overwrite = TRUE)\nG$new()$read()",
                "read(live = 2)",
            ),
            (
                "old <- G$new()\nG$set('public', 'read', function(live) live, overwrite = TRUE)\nold$read()",
                "read(original)",
            ),
            (
                "Ref$methods(read = function(live = 3) live)\nRef()$read()",
                "read(live = 3)",
            ),
        ] {
            let buffer = TextBuffer::new(text);
            let position = buffer
                .line_index()
                .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
            let help = signature::signature_help_via_db(
                &db.snapshot(),
                path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap();
            assert_eq!(help.signatures[0].label, expected);
        }
    }

    #[test]
    fn reference_reads_share_imported_class_identity_and_live_buffer_overlays() {
        let mut db = IncrementalDatabase::default();
        let parent_path = Path::new("/ref-parent/R/parent.R");
        let child_path = Path::new("/ref-child/R/child.R");
        let parent = db.upsert_file(parent_path, "Original <- methods::setRefClass('Display', fields = list(value = 'numeric'), methods = list(initialize = function(value = 3) initFields(value = value), read = function(scale = 2) value * scale))\nAlias <- Original\n");
        let source =
            "Child <- methods::setRefClass('Child', contains = 'Display')\nobj <- Child()\nobj$r";
        let child = db.upsert_file(child_path, source);
        db.set_workspace_members(
            vec![parent, child],
            vec!["/ref-parent".into(), "/ref-child".into()],
        );
        db.refresh_descriptions(["/ref-parent".into(), "/ref-child".into()]);
        db.upsert_description(Path::new("/ref-parent"), "Package: parent\n");
        db.upsert_description(Path::new("/ref-child"), "Package: child\nImports: parent\n");
        let metadata = |exported| {
            [
                (
                    "/ref-parent",
                    if exported {
                        "exportClasses(Display)\nexport(Alias)\n"
                    } else {
                        ""
                    },
                ),
                (
                    "/ref-child",
                    "importClassesFrom(parent, Display)\nimportFrom(parent, Alias)\n",
                ),
            ]
            .into_iter()
            .map(|(root, namespace)| PackageInfo {
                root: root.into(),
                namespace: Some(namespace.into()),
                dir_sources: BTreeSet::new(),
                native_routines: BTreeSet::new(),
            })
            .collect()
        };
        db.set_package_metadata(metadata(true));
        for (text, expected) in [
            (source.to_string(), "read"),
            (source.replace("obj$r", "obj$v"), "value"),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let CompletionResponse::List(items) = completion::completion_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap() else {
                panic!("completion list")
            };
            assert!(
                items.items.iter().any(|item| item.label == expected),
                "{items:?}"
            );
        }
        for (text, expected) in [
            (
                "Child <- methods::setRefClass('Child', contains = 'Display')\nChild$new()",
                "new(value = 3)",
            ),
            (
                "Child <- methods::setRefClass('Child', contains = 'Display', methods = list(initialize = function(label = 'live') initFields()))\nChild$new()",
                "new(label = 'live')",
            ),
            (
                "getRefClass('Display')()",
                "getRefClass('Display')(value = 3)",
            ),
            ("parent::Alias()", "parent::Alias(value = 3)"),
            (
                "Child <- methods::setRefClass('Child', contains = 'Display', methods = list(read = function(scale = 6) callSuper()))",
                "callSuper(scale = 2)",
            ),
        ] {
            let buffer = TextBuffer::new(text.to_string());
            let offset = if text.contains("callSuper()") {
                text.find("callSuper()").unwrap() + "callSuper(".len()
            } else {
                text.len() - 1
            };
            let position = buffer
                .line_index()
                .byte_to_position(offset, PositionEncoding::Utf16);
            let help = signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap_or_else(|| panic!("{text}"));
            assert_eq!(help.signatures[0].label, expected);
        }
        db.set_package_metadata(metadata(false));
        let text = "Child <- methods::setRefClass('Child', contains = 'Display')\nChild$new()";
        let buffer = TextBuffer::new(text.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
        assert!(
            signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .is_none()
        );
    }

    #[test]
    fn r6_reads_resolve_qualified_parent_aliases_and_overlay_live_buffers() {
        let mut db = IncrementalDatabase::default();
        let parent_path = Path::new("/r6-parent/R/parent.R");
        let child_path = Path::new("/r6-child/R/child.R");
        let parent = db.upsert_file(parent_path, "Original <- R6::R6Class('Display', public = list(value = 1, initialize = function(value = 3) self$value <- value, read = function(scale = 2) self$value * scale))\nAlias <- Original\n");
        let source = "Child <- R6::R6Class(inherit = parent::Alias)\nobj <- Child$new()\nobj$r";
        let child = db.upsert_file(child_path, source);
        db.set_workspace_members(
            vec![parent, child],
            vec!["/r6-parent".into(), "/r6-child".into()],
        );
        db.refresh_descriptions(["/r6-parent".into(), "/r6-child".into()]);
        db.upsert_description(Path::new("/r6-parent"), "Package: parent\n");
        db.upsert_description(Path::new("/r6-child"), "Package: child\nImports: parent\n");
        db.set_package_metadata(
            [("/r6-parent", "export(Alias)\n"), ("/r6-child", "")]
                .into_iter()
                .map(|(root, namespace)| PackageInfo {
                    root: root.into(),
                    namespace: Some(namespace.into()),
                    dir_sources: BTreeSet::new(),
                    native_routines: BTreeSet::new(),
                })
                .collect(),
        );
        // Cached syntax and a live buffer overlay must use the same parent identity.
        for (text, expected) in [
            (source.to_string(), "read"),
            (source.replace("obj$r", "obj$v"), "value"),
        ] {
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16);
            let result = completion::completion_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap();
            let CompletionResponse::List(items) = result else {
                panic!("completion list");
            };
            assert!(
                items.items.iter().any(|item| item.label == expected),
                "{items:?}"
            );
        }
        for (declaration, expected) in [
            (
                "Child <- R6::R6Class(inherit = parent::Alias)",
                "new(value = 3)",
            ),
            (
                "Child <- R6::R6Class(inherit = parent::Alias, public = list(initialize = function(label = 'live') label))",
                "new(label = 'live')",
            ),
        ] {
            let text = format!("{declaration}\nChild$new()");
            let buffer = TextBuffer::new(text.clone());
            let position = buffer
                .line_index()
                .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
            let help = signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .unwrap();
            assert_eq!(help.signatures[0].label, expected);
        }
        // Removing the export invalidates the same lookup instead of matching Display.
        db.set_package_metadata(
            [("/r6-parent", ""), ("/r6-child", "")]
                .into_iter()
                .map(|(root, namespace)| PackageInfo {
                    root: root.into(),
                    namespace: Some(namespace.into()),
                    dir_sources: BTreeSet::new(),
                    native_routines: BTreeSet::new(),
                })
                .collect(),
        );
        let text = "Child <- R6::R6Class(inherit = parent::Alias)\nChild$new()";
        let buffer = TextBuffer::new(text.to_string());
        let position = buffer
            .line_index()
            .byte_to_position(text.len() - 1, PositionEncoding::Utf16);
        assert!(
            signature::signature_help_via_db(
                &db.snapshot(),
                child_path,
                &buffer,
                position,
                PositionEncoding::Utf16,
                None,
            )
            .is_none()
        );
    }
}
