//! Type hierarchy over shared class declaration identities.
//!
//! Preparation overlays the live file's range-free facts on the project model.
//! Follow-up requests validate the opaque handle against the current declaration
//! projection; display names and stale positions never select a different class.

use super::*;
use crate::incremental::normalize_path;
use crate::project::{
    ClassId, ClassIndex, ClassOccurrence, class_occurrence_at_offset, file_class_defs,
    locate_class_def,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ItemData {
    id: ClassId,
    fingerprint: String,
}

fn class_item(
    snapshot: &Analysis,
    index: &ClassIndex,
    id: &ClassId,
    live: Option<(&Path, &Uri, &SyntaxNode, &LineIndex)>,
    encoding: PositionEncoding,
) -> Option<TypeHierarchyItem> {
    let class = index.declarations.get(id)?;
    let stored_root;
    let stored_index;
    let (root, line_index, uri) = match live.filter(|(path, _, _, _)| *path == id.path) {
        Some((_, uri, root, line_index)) => (root, line_index, uri.clone()),
        None => {
            let file = snapshot.lookup_file(&id.path)?;
            stored_root = snapshot.parsed_tree(file);
            stored_index = snapshot.line_index(file);
            (&stored_root, stored_index, uri::from_path(&id.path)?)
        }
    };
    let model = SemanticModel::build(root);
    let facts = file_class_defs(&model, root);
    if facts.fingerprint() != class.fingerprint {
        return None;
    }
    let loc = locate_class_def(root, &id.key)?;
    let owner = id
        .key
        .owner
        .0
        .iter()
        .map(|segment| segment.name.as_deref().unwrap_or("<anonymous>"))
        .collect::<Vec<_>>()
        .join("::");
    let mut detail = class.declaration.system().label().to_string();
    if let Some(binding) = &class.declaration.binding
        && binding != class.declaration.display_name()
    {
        detail.push_str(&format!(" via {binding}"));
    }
    if !owner.is_empty() {
        detail.push_str(&format!(" in {owner}"));
    }
    if let Some(package) = &class.package_root {
        detail.push_str(&format!(" ({})", package.display()));
    }
    Some(TypeHierarchyItem {
        name: class.declaration.display_name().to_string(),
        kind: LspSymbolKind::CLASS,
        tags: None,
        detail: Some(detail),
        uri,
        range: text_range_to_lsp_range(line_index, loc.full_range, encoding),
        selection_range: text_range_to_lsp_range(line_index, loc.name_range, encoding),
        data: Some(
            serde_json::to_value(ItemData {
                id: id.clone(),
                fingerprint: format!("{:016x}", class.fingerprint),
            })
            .ok()?,
        ),
    })
}

pub(crate) fn prepare_type_hierarchy_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    buffer: &TextBuffer,
    position: Position,
    encoding: PositionEncoding,
) -> Option<Vec<TypeHierarchyItem>> {
    let normalized_path = normalize_path(path);
    let path = normalized_path.as_path();
    let text = buffer.text();
    let line_index = buffer.line_index();
    let offset = TextSize::new(
        line_index
            .position_to_byte(position, encoding)
            .min(text.len()) as u32,
    );
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    let facts = file_class_defs(&model, &root);
    let occurrence = class_occurrence_at_offset(&model, &root, offset)?;
    let prepare = |snapshot: &Analysis| {
        let index = snapshot.class_index_in(path, &facts, &root, &model);
        let ids = match &occurrence {
            ClassOccurrence::Declaration(key) => vec![ClassId {
                path: path.to_path_buf(),
                key: key.clone(),
            }],
            ClassOccurrence::Reference {
                owner,
                reference,
                deferred,
            } => index.resolve_reference(path, owner, reference, *deferred),
        };
        let items: Vec<_> = ids
            .iter()
            .filter_map(|id| {
                class_item(
                    snapshot,
                    &index,
                    id,
                    Some((path, uri, &root, line_index)),
                    encoding,
                )
            })
            .collect();
        (!items.is_empty()).then_some(items)
    };
    salsa::Cancelled::catch(AssertUnwindSafe(|| prepare(snapshot))).unwrap_or_else(|_| {
        // A write elsewhere can cancel this snapshot without changing the live
        // document. Rebuild its local visibility without touching the old DB.
        prepare(&IncrementalDatabase::default().snapshot())
    })
}

pub(crate) fn supertypes_via_db(
    snapshot: &Analysis,
    item: &TypeHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<TypeHierarchyItem>> {
    class_relatives(snapshot, item, true, encoding)
}

pub(crate) fn subtypes_via_db(
    snapshot: &Analysis,
    item: &TypeHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<TypeHierarchyItem>> {
    class_relatives(snapshot, item, false, encoding)
}

fn class_relatives(
    snapshot: &Analysis,
    item: &TypeHierarchyItem,
    super_edge: bool,
    encoding: PositionEncoding,
) -> Option<Vec<TypeHierarchyItem>> {
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let Some(data) = item
            .data
            .clone()
            .and_then(|value| serde_json::from_value::<ItemData>(value).ok())
        else {
            return Vec::new();
        };
        if uri::to_path(&item.uri)
            .map(|path| normalize_path(&path))
            .as_deref()
            != Some(data.id.path.as_path())
        {
            return Vec::new();
        }
        let index = snapshot.class_index();
        if !index
            .declarations
            .get(&data.id)
            .is_some_and(|class| format!("{:016x}", class.fingerprint) == data.fingerprint)
        {
            return Vec::new();
        }
        let edges = if super_edge {
            &index.supertypes
        } else {
            &index.subtypes
        };
        edges
            .get(&data.id)
            .into_iter()
            .flatten()
            .filter_map(|id| class_item(snapshot, &index, id, None, encoding))
            .collect()
    }))
    .ok()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn prepare_at(
        snapshot: &Analysis,
        path: &Path,
        text: &str,
        offset: usize,
    ) -> Vec<TypeHierarchyItem> {
        let uri = uri::from_path(path).unwrap();
        prepare_type_hierarchy_via_db(
            snapshot,
            path,
            &uri,
            &buf(text),
            pos_at(text, offset),
            PositionEncoding::Utf16,
        )
        .unwrap_or_default()
    }

    fn item_named(snapshot: &Analysis, path: &Path, name: &str) -> TypeHierarchyItem {
        {
            let index = snapshot.class_index();
            let id = index
                .declarations
                .iter()
                .find(|(id, class)| id.path == path && class.declaration.display_name() == name)
                .map(|(id, _)| id)
                .expect("class identity");
            class_item(snapshot, &index, id, None, PositionEncoding::Utf16).expect("class item")
        }
    }

    // --- prepare ------------------------------------------------------------

    #[test]
    fn hierarchy_round_trips_lexically_equivalent_paths() {
        let source = "Base <- R6::R6Class('Base')\nChild <- R6::R6Class('Child', inherit = Base)\n";
        let snapshot = rename_workspace(source, "");
        let normalized = ws_path("a.R");
        let path = ws_root().join("sub").join("..").join("a.R");
        let child = prepare_at(
            &snapshot,
            &path,
            source,
            source.find("'Child'").unwrap() + 1,
        );
        assert_eq!(child.len(), 1);
        assert_eq!(child[0].uri, uri::from_path(&path).unwrap());
        assert_eq!(
            child[0].data,
            item_named(&snapshot, &normalized, "Child").data
        );
        let parents = supertypes_via_db(&snapshot, &child[0], PositionEncoding::Utf16).unwrap();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].name, "Base");
        let base = prepare_at(&snapshot, &path, source, source.find("'Base'").unwrap() + 1);
        let children = subtypes_via_db(&snapshot, &base[0], PositionEncoding::Utf16).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Child");
    }

    #[test]
    fn unnormalized_paths_overlay_the_tracked_file() {
        let source = "Base <- R6::R6Class('Live')\nChild <- R6::R6Class('Child', inherit = Base)\n";
        let snapshot = rename_workspace(&source.replace("Live", "Stored"), "");
        let path = ws_root().join("sub").join("..").join("a.R");
        let items = prepare_at(&snapshot, &path, source, source.rfind("Base").unwrap());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Live");
        assert_eq!(items[0].uri, uri::from_path(&path).unwrap());
        assert_eq!(
            items[0].data,
            prepare_at(
                &snapshot,
                &ws_path("a.R"),
                source,
                source.rfind("Base").unwrap()
            )[0]
            .data
        );
    }

    #[test]
    fn closure_mutations_do_not_prove_class_factories() {
        for mutation in [
            "body(make) <- quote(NULL)",
            "formals(make) <- alist(x = )",
            "environment(make) <- baseenv()",
            "base::body(make) <- quote(NULL)",
            "`body<-`(make, value = quote(NULL))",
        ] {
            let source = format!("make <- R6::R6Class\n{mutation}\nA <- make('Phantom')\n");
            let snapshot = rename_workspace(&source, "");
            assert!(snapshot.class_index().declarations.is_empty(), "{source}");
            assert!(
                prepare_at(
                    &snapshot,
                    &ws_path("a.R"),
                    &source,
                    source.find("Phantom").unwrap()
                )
                .is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn closure_reads_prepare_generators_declared_later() {
        for source in [
            "f <- function() A$new()\nA <- R6::R6Class('A')\n",
            "f <- function() Alias$new()\nA <- R6::R6Class('A')\nAlias <- A\n",
            "f <- function() { g <- function() A$new(); A <- R6::R6Class('A') }\n",
        ] {
            let snapshot = rename_workspace(source, "");
            let items = prepare_at(
                &snapshot,
                &ws_path("a.R"),
                source,
                source.find("$new").unwrap() - 1,
            );
            assert_eq!(items.len(), 1, "{source}");
            assert_eq!(items[0].name, "A");
        }
        let source = "A$new()\nA <- R6::R6Class('A')\n";
        assert!(prepare_at(&rename_workspace(source, ""), &ws_path("a.R"), source, 0).is_empty());
    }

    #[test]
    fn cancelled_preparation_keeps_fresh_local_classes() {
        let source = "f <- function() A$new()\nA <- R6::R6Class('A')\n";
        let mut db = IncrementalDatabase::default();
        let path = ws_path("a.R");
        let a = db.upsert_file(&path, source);
        let b = db.upsert_file(&ws_path("b.R"), "x <- 1\n");
        db.set_workspace_members(vec![a, b], vec![ws_root()]);
        let snapshot = db.snapshot();
        let writer = std::thread::spawn(move || db.set_file_text(b, "x <- 2\n"));
        // Wait for the pending write to cancel this reader before making the
        // request, so the regression does not depend on request timing.
        while salsa::Cancelled::catch(AssertUnwindSafe(|| snapshot.class_index())).is_ok() {
            std::thread::yield_now();
        }
        let declaration = prepare_at(&snapshot, &path, source, source.find("'A'").unwrap() + 1);
        let reference = prepare_at(&snapshot, &path, source, source.find("A$new").unwrap());
        drop(snapshot);
        writer.join().unwrap();
        for items in [declaration, reference] {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].name, "A");
            assert_eq!(items[0].uri, uri::from_path(&path).unwrap());
            assert_eq!(items[0].selection_range.start.line, 1);
            assert!(items[0].data.is_some());
        }
    }

    #[test]
    fn untitled_preparation_preserves_the_live_uri() {
        let source = "A <- R6::R6Class('A')\n";
        let snapshot = rename_workspace("", "");
        let uri: Uri = "untitled:Untitled-1".parse().unwrap();
        let items = prepare_type_hierarchy_via_db(
            &snapshot,
            Path::new(DocumentKind::R.placeholder_file_name()),
            &uri,
            &buf(source),
            pos_at(source, source.find("'A'").unwrap() + 1),
            PositionEncoding::Utf16,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].uri, uri);
    }

    #[test]
    fn qualified_parent_prepares_from_its_member_token() {
        let source = "Child <- R6::R6Class('Child', inherit = one::Parent)\n";
        let mut db = IncrementalDatabase::default();
        let root = ws_path("one");
        let parent = root.join("R/a.R");
        let child = ws_path("child.R");
        let a = db.upsert_file(&parent, "Parent <- R6::R6Class('Display')\n");
        let b = db.upsert_file(&child, source);
        db.set_workspace_members(vec![a, b], vec![ws_root()]);
        db.refresh_descriptions([root.clone()]);
        db.upsert_description(&root, "Package: one\n");
        db.set_package_metadata(vec![crate::project::PackageInfo {
            root,
            namespace: Some("export(Parent)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let snapshot = db.snapshot();
        assert_eq!(snapshot.class_index().supertypes.len(), 1);
        let items = prepare_at(&snapshot, &child, source, source.find("Parent").unwrap());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Display");
        assert_eq!(items[0].uri, uri::from_path(&parent).unwrap());
        assert!(prepare_at(&snapshot, &child, source, source.find("one::").unwrap()).is_empty());
    }

    #[test]
    fn r6_parent_lookup_is_deferred_but_alias_assignments_are_eager() {
        for (source, expected) in [
            (
                "Child <- R6::R6Class('Child', inherit = Parent)\nParent <- R6::R6Class('Parent')\n",
                1,
            ),
            (
                "Child <- R6::R6Class('Child', inherit = Alias)\nAlias <- Parent\nParent <- R6::R6Class('Parent')\n",
                0,
            ),
            (
                "Child <- S7::new_class('Child', parent = Parent)\nParent <- S7::new_class('Parent')\n",
                0,
            ),
        ] {
            let snapshot = rename_workspace(source, "");
            let child = item_named(&snapshot, &ws_path("a.R"), "Child");
            let parents = supertypes_via_db(&snapshot, &child, PositionEncoding::Utf16).unwrap();
            assert_eq!(parents.len(), expected, "{source}");
        }
    }

    #[test]
    fn r6_inheritance_uses_the_final_sourced_generator() {
        let source =
            "source('b.R')\nChild <- R6::R6Class('Child', inherit = Parent)\nsource('c.R')\n";
        let snapshot = rename_workspace3(
            source,
            "Parent <- R6::R6Class('Earlier')\n",
            "Parent <- R6::R6Class('Later')\n",
        );
        let child = item_named(&snapshot, &ws_path("a.R"), "Child");
        let parents = supertypes_via_db(&snapshot, &child, PositionEncoding::Utf16).unwrap();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].name, "Later");
        let prepared = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            source,
            source.find("inherit = Parent").unwrap() + 10,
        );
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].name, "Later");
    }

    #[test]
    fn class_identity_resolves_generator_aliases() {
        let src = "Generator <- R6::R6Class(\"Display\")\nAlias <- Generator\nChild <- R6::R6Class(\"Child\", inherit = Alias)\n";
        let snapshot = rename_workspace(src, "");
        let child = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            src,
            src.find("\"Child\"").unwrap() + 1,
        );
        let parents = supertypes_via_db(&snapshot, &child[0], PositionEncoding::Utf16).unwrap();
        assert_eq!(
            parents
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["Display"]
        );
        let alias = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            src,
            src.find("inherit = Alias").unwrap() + 10,
        );
        assert_eq!(alias.len(), 1);
        assert_eq!(alias[0].name, "Display");
    }

    #[test]
    fn class_identity_matches_named_arguments_and_callee_aliases() {
        let src = "make <- R6::R6Class\nPoint <- make(public = list(), classname = \"Point\")\nmethods::setClass(slots = c(x = \"numeric\"), Class = \"Formal\")\n";
        let snapshot = rename_workspace(src, "");
        for name in ["Point", "Formal"] {
            let needle = format!("\"{name}\"");
            let items = prepare_at(
                &snapshot,
                &ws_path("a.R"),
                src,
                src.find(&needle).unwrap() + 1,
            );
            assert_eq!(items.len(), 1, "{name}");
            assert_eq!(items[0].name, name);
        }
    }

    #[test]
    fn class_identity_rejects_shadowed_and_wrong_namespace_calls() {
        for src in [
            "setClass <- function(...) NULL\nsetClass(\"Phantom\")\n",
            "other::setClass(\"Phantom\")\n",
            "R6Class <- function(...) NULL\nR6Class(\"Phantom\")\n",
            "f <- function(setRefClass) setRefClass(\"Phantom\")\n",
            "library <- function(...) NULL\nlibrary(R6)\nR6Class(\"Phantom\")\n",
            "if (flag) library(R6)\nR6Class(\"Phantom\")\n",
        ] {
            let snapshot = rename_workspace(src, "");
            assert!(
                prepare_at(
                    &snapshot,
                    &ws_path("a.R"),
                    src,
                    src.find("Phantom").unwrap()
                )
                .is_empty(),
                "{src}"
            );
        }
    }

    #[test]
    fn class_identity_preserves_duplicate_declaration_locations() {
        let src = "A <- R6::R6Class(\"Same\")\nB <- R6::R6Class(\"Same\")\n";
        let snapshot = rename_workspace(src, "");
        let first = prepare_at(&snapshot, &ws_path("a.R"), src, src.find("Same").unwrap());
        let second = prepare_at(&snapshot, &ws_path("a.R"), src, src.rfind("Same").unwrap());
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].selection_range, second[0].selection_range);
        assert_ne!(first[0].data, second[0].data);
    }

    #[test]
    fn class_identity_omits_ambiguous_formal_parent_edges() {
        let src = "methods::setClass(\"Parent\")\nmethods::setClass(\"Parent\")\nmethods::setClass(\"Child\", contains = \"Parent\")\n";
        let snapshot = rename_workspace(src, "");
        let child = prepare_at(&snapshot, &ws_path("a.R"), src, src.find("Child").unwrap());
        assert!(
            supertypes_via_db(&snapshot, &child[0], PositionEncoding::Utf16)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn class_identity_supports_unnamed_r6_generators() {
        let src = "Generator <- R6::R6Class(public = list())\nChild <- R6::R6Class(\"Child\", inherit = Generator)\n";
        let snapshot = rename_workspace(src, "");
        let generator = prepare_at(&snapshot, &ws_path("a.R"), src, 1);
        assert_eq!(generator.len(), 1);
        assert_eq!(generator[0].name, "Generator");
        let child = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            src,
            src.find("\"Child\"").unwrap() + 1,
        );
        let parents = supertypes_via_db(&snapshot, &child[0], PositionEncoding::Utf16).unwrap();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].name, "Generator");
    }

    #[test]
    fn prepare_on_a_class_name_yields_its_item() {
        let src = "setClass(\"Animal\")\n";
        let snapshot = rename_workspace(src, "");
        let items = prepare_at(&snapshot, &ws_path("a.R"), src, src.find("Animal").unwrap());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Animal");
        assert_eq!(items[0].kind, LspSymbolKind::CLASS);
        assert_eq!(items[0].detail.as_deref(), Some("S4 class"));
    }

    #[test]
    fn prepare_on_a_contains_reference_yields_the_parent_item() {
        let src = "setClass(\"Animal\")\nsetClass(\"Dog\", contains = \"Animal\")\n";
        let snapshot = rename_workspace(src, "");
        let offset = src.find("\"Animal\")").unwrap() + 1; // the contains= reference
        let items = prepare_at(&snapshot, &ws_path("a.R"), src, offset);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Animal");
    }

    #[test]
    fn prepare_declines_a_non_class_string() {
        let src = "x <- \"Animal\"\n";
        let snapshot = rename_workspace(src, "");
        let items = prepare_at(&snapshot, &ws_path("a.R"), src, src.find("Animal").unwrap());
        assert!(items.is_empty());
    }

    // --- supertypes ---------------------------------------------------------

    #[test]
    fn supertypes_reports_s4_contains_parents() {
        let src = "setClass(\"Animal\")\nsetClass(\"Dog\", contains = c(\"Animal\", \"Pet\"))\nsetClass(\"Pet\")\n";
        let snapshot = rename_workspace(src, "");
        let supers = supertypes_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "Dog"),
            PositionEncoding::Utf16,
        )
        .expect("supertypes");
        let mut names: Vec<&str> = supers.iter().map(|i| i.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Animal", "Pet"]);
    }

    #[test]
    fn supertypes_reports_r6_inherit_parent() {
        let src =
            "Animal <- R6::R6Class(\"Animal\")\nDog <- R6::R6Class(\"Dog\", inherit = Animal)\n";
        let snapshot = rename_workspace(src, "");
        let supers = supertypes_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "Dog"),
            PositionEncoding::Utf16,
        )
        .expect("supertypes");
        assert_eq!(supers.len(), 1);
        assert_eq!(supers[0].name, "Animal");
        assert_eq!(supers[0].detail.as_deref(), Some("R6 class"));
    }

    #[test]
    fn supertypes_omits_a_parent_defined_nowhere() {
        // `Base` is referenced but never defined in the workspace.
        let src = "setClass(\"Dog\", contains = \"Base\")\n";
        let snapshot = rename_workspace(src, "");
        let supers = supertypes_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "Dog"),
            PositionEncoding::Utf16,
        )
        .expect("supertypes");
        assert!(supers.is_empty(), "undefined parent has no item");
    }

    // --- subtypes -----------------------------------------------------------

    #[test]
    fn subtypes_reports_direct_children() {
        let src = "setClass(\"Animal\")\nsetClass(\"Dog\", contains = \"Animal\")\n";
        let snapshot = rename_workspace(src, "");
        let subs = subtypes_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "Animal"),
            PositionEncoding::Utf16,
        )
        .expect("subtypes");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "Dog");
    }

    #[test]
    fn subtypes_resolves_a_cross_file_child() {
        // Parent in a.R, child in b.R; the class index spans the whole workspace.
        let a_src = "setClass(\"Animal\")\n";
        let b_src = "source(\"a.R\")\nsetClass(\"Dog\", contains = \"Animal\")\n";
        let snapshot = rename_workspace(a_src, b_src);
        let subs = subtypes_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "Animal"),
            PositionEncoding::Utf16,
        )
        .expect("subtypes");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "Dog");
        assert_eq!(subs[0].uri, uri::from_path(&ws_path("b.R")).unwrap());
    }

    #[test]
    fn leaf_class_has_no_subtypes() {
        let src = "setClass(\"Animal\")\nsetClass(\"Dog\", contains = \"Animal\")\n";
        let snapshot = rename_workspace(src, "");
        let subs = subtypes_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "Dog"),
            PositionEncoding::Utf16,
        )
        .expect("subtypes");
        assert!(subs.is_empty());
    }
    #[test]
    fn class_identity_scopes_nested_and_anonymous_declarations() {
        for (source, parents) in [
            (
                "f <- function() { Base <- R6::R6Class(\"One\"); Child <- R6::R6Class(\"Child\", inherit = Base) }\ng <- function() { Base <- R6::R6Class(\"Two\"); Child <- R6::R6Class(\"Child\", inherit = Base) }\n",
                ["One", "Two"],
            ),
            (
                "list(function() { Base <- R6::R6Class(\"One\"); Child <- R6::R6Class(\"Child\", inherit = Base) }, function() { Base <- R6::R6Class(\"Two\"); Child <- R6::R6Class(\"Child\", inherit = Base) })\n",
                ["One", "Two"],
            ),
        ] {
            let snapshot = rename_workspace(source, "");
            for ((offset, _), expected) in source.match_indices("\"Child\"").zip(parents) {
                let items = prepare_at(&snapshot, &ws_path("a.R"), source, offset + 1);
                assert_eq!(items.len(), 1);
                let parents =
                    supertypes_via_db(&snapshot, &items[0], PositionEncoding::Utf16).unwrap();
                assert_eq!(
                    parents
                        .iter()
                        .map(|item| item.name.as_str())
                        .collect::<Vec<_>>(),
                    [expected]
                );
            }
        }
    }

    #[test]
    fn class_identity_allows_formal_and_reference_class_inheritance() {
        let source = "Formal <- methods::setClass(\"Base\")\nReference <- methods::setRefClass(\"Reference\", contains = \"Base\")\nmethods::setClass(\"Child\", contains = \"Reference\")\nOther <- R6::R6Class(\"Base\")\n";
        let snapshot = rename_workspace(source, "");
        for (name, parent) in [("Reference", "Base"), ("Child", "Reference")] {
            let item = item_named(&snapshot, &ws_path("a.R"), name);
            let parents = supertypes_via_db(&snapshot, &item, PositionEncoding::Utf16).unwrap();
            assert_eq!(parents.len(), 1);
            assert_eq!(parents[0].name, parent);
            assert!(!parents[0].detail.as_ref().unwrap().contains("R6"));
        }
    }

    #[test]
    fn class_identity_declines_missing_malformed_and_stale_handles() {
        let source = "A <- R6::R6Class(\"Base\")\nB <- R6::R6Class(\"Child\", inherit = A)\n";
        let mut db = IncrementalDatabase::default();
        let path = ws_path("a.R");
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![ws_root()]);
        let item =
            prepare_at(&db.snapshot(), &path, source, source.find("Child").unwrap()).remove(0);
        db.set_file_text(file, format!("# Prose.\n{source}"));
        let parents = supertypes_via_db(&db.snapshot(), &item, PositionEncoding::Utf16).unwrap();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].selection_range.start.line, 1);
        db.set_file_text(file, source.replace("Base", "Renamed"));
        assert!(
            supertypes_via_db(&db.snapshot(), &item, PositionEncoding::Utf16)
                .unwrap()
                .is_empty()
        );
        for data in [None, Some(serde_json::json!({"id": "invalid"}))] {
            let mut invalid = item.clone();
            invalid.data = data;
            assert!(
                supertypes_via_db(&db.snapshot(), &invalid, PositionEncoding::Utf16)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn class_identity_prepares_from_a_live_buffer_before_the_snapshot_catches_up() {
        let snapshot = rename_workspace("x <- 1\n", "");
        let source = "New <- R6::R6Class(classname = \"Live\")\n";
        let items = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            source,
            source.find("Live").unwrap(),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Live");
        assert_eq!(
            items[0].selection_range.start,
            pos_at(source, source.find("\"Live\"").unwrap())
        );
    }
    #[test]
    fn class_identity_live_buffer_drops_removed_source_visibility() {
        let stored = "source(\"b.R\")\nChild <- R6::R6Class(\"Child\", inherit = Parent)\n";
        let snapshot = rename_workspace(stored, "Parent <- R6::R6Class(\"Parent\")\n");
        let live = stored.replace("source(\"b.R\")\n", "");
        let items = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            &live,
            live.find("inherit = Parent").unwrap() + 10,
        );
        assert!(items.is_empty());
    }
    #[test]
    fn class_identity_live_package_recovers_imports_from_a_parse_error_snapshot() {
        let root = ws_path("pkg");
        let path = root.join("R/a.R");
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(&path, "(");
        db.set_workspace_members(vec![file], vec![root.clone()]);
        db.set_package_metadata(vec![crate::project::PackageInfo {
            root,
            namespace: Some("importFrom(R6, R6Class)\n".into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let source = "A <- R6Class(\"A\")\n";
        let items = prepare_at(
            &db.snapshot(),
            &path,
            source,
            source.find("\"A\"").unwrap() + 1,
        );
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn class_identity_cannot_inherit_from_binding_created_by_the_same_call() {
        let source = "Self <- R6::R6Class(\"Self\", inherit = Self)\n";
        let snapshot = rename_workspace(source, "");
        let item = item_named(&snapshot, &ws_path("a.R"), "Self");
        assert!(
            supertypes_via_db(&snapshot, &item, PositionEncoding::Utf16)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn class_identity_function_reads_use_the_final_sourced_generator() {
        let source = "source(\"b.R\")\nprint(Parent)\nsource(\"c.R\")\nf <- function() Parent\n";
        let snapshot = rename_workspace3(
            source,
            "Parent <- R6::R6Class(\"Earlier\")\n",
            "Parent <- R6::R6Class(\"Later\")\n",
        );
        let items = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            source,
            source.rfind("Parent").unwrap(),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Later");
    }
}
