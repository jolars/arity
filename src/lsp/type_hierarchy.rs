//! Type hierarchy (`textDocument/prepareTypeHierarchy`, `typeHierarchy/supertypes`,
//! `typeHierarchy/subtypes`).
//!
//! The class-hierarchy analog of [`call_hierarchy`](super::call_hierarchy): items
//! are **class definitions** — S4 (`setClass`), R6 (`R6Class`), and reference
//! classes (`setRefClass`) — and an edge is a declared inheritance relation
//! (`contains=` / `inherit=`), keyed on the string class name the class index
//! ([`Analysis::class_def_sites`], [`Analysis::class_supertypes`],
//! [`Analysis::class_subtypes`]) records.
//!
//! `prepare` parses the live buffer (like [`prepare_call_hierarchy_via_db`]);
//! `supertypes`/`subtypes` work purely off the db snapshot, recovering the target
//! from the round-tripped item's `name` (no `data` payload needed). Snapshot
//! reads are wrapped in [`salsa::Cancelled::catch`]. A relative class with no
//! definition site in the workspace (e.g. a base R virtual class, or an R6 base
//! from a package) yields no item — omitted, like call hierarchy's unresolved
//! callees.

use super::*;
use crate::project::{ClassLocation, class_name_at_offset, locate_class_def};

/// Build a [`TypeHierarchyItem`] for the class `name` located at `loc`, mapping
/// its spans through `line_index`.
fn location_to_item(
    name: &str,
    loc: &ClassLocation,
    uri: &Uri,
    line_index: &LineIndex,
) -> TypeHierarchyItem {
    TypeHierarchyItem {
        name: name.to_string(),
        kind: LspSymbolKind::CLASS,
        tags: None,
        detail: Some(loc.system.label().to_string()),
        uri: uri.clone(),
        range: text_range_to_lsp_range(line_index, loc.full_range),
        selection_range: text_range_to_lsp_range(line_index, loc.name_range),
        data: None,
    }
}

/// The type-hierarchy item for the class `name` defined in the workspace file at
/// `path`, off the db snapshot. `None` when the file isn't tracked, has no URI,
/// or defines no class of that name.
fn class_item(snapshot: &Analysis, path: &Path, name: &str) -> Option<TypeHierarchyItem> {
    let file = snapshot.lookup_file(path)?;
    let uri = uri::from_path(path)?;
    let root = snapshot.parsed_tree(file);
    let loc = locate_class_def(&root, name)?;
    let line_index = LineIndex::new(snapshot.file_text(file));
    Some(location_to_item(name, &loc, &uri, &line_index))
}

/// `textDocument/prepareTypeHierarchy`: resolve the cursor to the class it names
/// (its definition, a `contains=`/`inherit=` reference, or a class-generator
/// binding), as one or more items. Intra-file is parsed from the live `text`
/// (like [`prepare_call_hierarchy_via_db`]); sibling definitions of the same
/// class come from the workspace class index. `None` when the cursor names no
/// known class.
pub(crate) fn prepare_type_hierarchy_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    text: &str,
    position: Position,
) -> Option<Vec<TypeHierarchyItem>> {
    let line_index = LineIndex::new(text);
    let offset = TextSize::new(line_index.position_to_byte(position).min(text.len()) as u32);
    let root = parse(text).cst;
    let name = class_name_at_offset(&root, offset)?;

    let mut items: Vec<TypeHierarchyItem> = Vec::new();
    // Local: the live buffer's own definition (freshest — never a stale copy).
    if let Some(loc) = locate_class_def(&root, &name) {
        items.push(location_to_item(&name, &loc, uri, &line_index));
    }
    // Cross-file: sibling workspace files that define the same class name.
    let cross = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot
            .class_def_sites(&name)
            .into_iter()
            .filter(|(def_path, _)| def_path != path)
            .filter_map(|(def_path, _)| class_item(snapshot, &def_path, &name))
            .collect::<Vec<_>>()
    }))
    .unwrap_or_default();
    items.extend(cross);
    (!items.is_empty()).then_some(items)
}

/// `typeHierarchy/supertypes`: the declared parent classes of the item's class,
/// each as an item. Works off the snapshot from the item's `name`.
pub(crate) fn supertypes_via_db(
    snapshot: &Analysis,
    item: &TypeHierarchyItem,
) -> Option<Vec<TypeHierarchyItem>> {
    class_relatives(snapshot, item, Edge::Super)
}

/// `typeHierarchy/subtypes`: the classes that declare the item's class a
/// supertype, each as an item. Works off the snapshot from the item's `name`.
pub(crate) fn subtypes_via_db(
    snapshot: &Analysis,
    item: &TypeHierarchyItem,
) -> Option<Vec<TypeHierarchyItem>> {
    class_relatives(snapshot, item, Edge::Sub)
}

#[derive(Clone, Copy)]
enum Edge {
    Super,
    Sub,
}

/// The related classes of `item` along `edge`, each resolved to its first
/// locatable workspace definition. Unresolved relatives (no in-workspace
/// definition) are omitted.
fn class_relatives(
    snapshot: &Analysis,
    item: &TypeHierarchyItem,
    edge: Edge,
) -> Option<Vec<TypeHierarchyItem>> {
    let name = item.name.clone();
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let related = match edge {
            Edge::Super => snapshot.class_supertypes(&name),
            Edge::Sub => snapshot.class_subtypes(&name),
        };
        related
            .into_iter()
            .filter_map(|rel| {
                snapshot
                    .class_def_sites(&rel)
                    .into_iter()
                    .find_map(|(path, _)| class_item(snapshot, &path, &rel))
            })
            .collect::<Vec<_>>()
    }))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepare_at(
        snapshot: &Analysis,
        path: &Path,
        text: &str,
        offset: usize,
    ) -> Vec<TypeHierarchyItem> {
        let uri = uri::from_path(path).unwrap();
        prepare_type_hierarchy_via_db(snapshot, path, &uri, text, pos_at(text, offset))
            .unwrap_or_default()
    }

    fn item_named(snapshot: &Analysis, path: &Path, name: &str) -> TypeHierarchyItem {
        class_item(snapshot, path, name).expect("class item")
    }

    // --- prepare ------------------------------------------------------------

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
        let supers = supertypes_via_db(&snapshot, &item_named(&snapshot, &ws_path("a.R"), "Dog"))
            .expect("supertypes");
        let mut names: Vec<&str> = supers.iter().map(|i| i.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Animal", "Pet"]);
    }

    #[test]
    fn supertypes_reports_r6_inherit_parent() {
        let src = "Animal <- R6Class(\"Animal\")\nDog <- R6Class(\"Dog\", inherit = Animal)\n";
        let snapshot = rename_workspace(src, "");
        let supers = supertypes_via_db(&snapshot, &item_named(&snapshot, &ws_path("a.R"), "Dog"))
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
        let supers = supertypes_via_db(&snapshot, &item_named(&snapshot, &ws_path("a.R"), "Dog"))
            .expect("supertypes");
        assert!(supers.is_empty(), "undefined parent has no item");
    }

    // --- subtypes -----------------------------------------------------------

    #[test]
    fn subtypes_reports_direct_children() {
        let src = "setClass(\"Animal\")\nsetClass(\"Dog\", contains = \"Animal\")\n";
        let snapshot = rename_workspace(src, "");
        let subs = subtypes_via_db(&snapshot, &item_named(&snapshot, &ws_path("a.R"), "Animal"))
            .expect("subtypes");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "Dog");
    }

    #[test]
    fn subtypes_resolves_a_cross_file_child() {
        // Parent in a.R, child in b.R; the class index spans the whole workspace.
        let a_src = "setClass(\"Animal\")\n";
        let b_src = "setClass(\"Dog\", contains = \"Animal\")\n";
        let snapshot = rename_workspace(a_src, b_src);
        let subs = subtypes_via_db(&snapshot, &item_named(&snapshot, &ws_path("a.R"), "Animal"))
            .expect("subtypes");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "Dog");
        assert_eq!(subs[0].uri, uri::from_path(&ws_path("b.R")).unwrap());
    }

    #[test]
    fn leaf_class_has_no_subtypes() {
        let src = "setClass(\"Animal\")\nsetClass(\"Dog\", contains = \"Animal\")\n";
        let snapshot = rename_workspace(src, "");
        let subs = subtypes_via_db(&snapshot, &item_named(&snapshot, &ws_path("a.R"), "Dog"))
            .expect("subtypes");
        assert!(subs.is_empty());
    }
}
