//! Editor consumers share the project class index, including live-buffer overlays.

use super::*;
use crate::project::classes::{ClassIndex, file_class_defs};

pub(super) fn with_class_index<T>(
    root: &SyntaxNode,
    context: Option<(&Analysis, &Path)>,
    compute: impl Fn(&ClassIndex, &SemanticModel, &Path) -> T,
) -> T {
    let model = SemanticModel::build(root);
    let facts = file_class_defs(&model, root);
    let path = context.map_or(Path::new("buffer.R"), |(_, path)| path);
    if let Some((snapshot, _)) = context {
        let result = salsa::Cancelled::catch(AssertUnwindSafe(|| {
            let index = snapshot.class_index_in(path, &facts, root, &model);
            compute(&index, &model, path)
        }));
        if let Ok(result) = result {
            return result;
        }
    }
    let index = IncrementalDatabase::default()
        .snapshot()
        .class_index_in(path, &facts, root, &model);
    compute(&index, &model, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::PackageInfo;
    use crate::text::TextBuffer;
    use std::collections::BTreeSet;

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
                PositionEncoding::Utf16
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
                PositionEncoding::Utf16
            )
            .is_none()
        );
    }
}
