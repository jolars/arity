//! Call hierarchy (`textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls`,
//! `callHierarchy/outgoingCalls`).
//!
//! Items are **top-level (file-scope) function definitions** — the names the
//! cross-file index keys on ([`Analysis::workspace_def_sites`],
//! [`Analysis::cross_file_binding`], [`Analysis::visible_def_files`]). Nested/local
//! functions are not items (a follow-up); a call inside a nested function is
//! attributed to its enclosing top-level function. An edge is a *callee-position*
//! use `F(...)`, never a value use (`lapply(xs, F)`); call sites at script
//! top-level (inside no function) are dropped.
//!
//! `prepare` parses the live buffer (a deliberate, infrequent action, like
//! [`definition_via_db`]); `incoming`/`outgoing` work purely off the db snapshot,
//! recovering the target from the round-tripped item's `uri` + `name` (no `data`
//! payload needed). Snapshot reads are wrapped in [`salsa::Cancelled::catch`].

use super::*;

/// A top-level (file-scope) function definition in one file: the bound name, the
/// selection range (the defining identifier), and the full range (the whole
/// assignment statement).
#[derive(Debug, Clone)]
struct FnDef {
    name: SmolStr,
    selection: TextRange,
    full: TextRange,
}

/// Every file-scope binding in `model` whose value is a function literal, paired
/// with that function node. The single definition of "what counts as a
/// call-hierarchy function" — file-scope only (v1). The CST supplies the function
/// value and the full assignment span; the model decides file-scope membership
/// and the canonical (unquoted) name.
fn function_defs(root: &SyntaxNode, model: &SemanticModel) -> Vec<(FnDef, FunctionExpr)> {
    // Assignment target span -> (assignment node, function value).
    let mut by_target: HashMap<TextRange, (SyntaxNode, FunctionExpr)> = HashMap::new();
    for node in root.descendants() {
        let Some(assign) = AssignmentExpr::cast(node.clone()) else {
            continue;
        };
        let Some(name_token) = assign.target_name_token() else {
            continue;
        };
        let Some(NodeOrToken::Node(value)) = assign.value_element() else {
            continue;
        };
        let Some(func) = FunctionExpr::cast(value) else {
            continue;
        };
        by_target.insert(name_token.text_range(), (node, func));
    }
    model
        .bindings()
        .iter()
        .enumerate()
        .filter(|(i, b)| {
            matches!(b.kind, BindingKind::Local | BindingKind::Implicit)
                && model.binding_is_file_scope(BindingId::from_index(*i))
        })
        .filter_map(|(_, b)| {
            let (assign, func) = by_target.get(&b.def_range)?;
            Some((
                FnDef {
                    name: b.name.clone(),
                    selection: b.def_range,
                    full: assign.text_range(),
                },
                func.clone(),
            ))
        })
        .collect()
}

/// Build a [`CallHierarchyItem`] for `def`, mapping its spans through `line_index`.
fn fn_def_to_item(
    def: &FnDef,
    uri: &Uri,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> CallHierarchyItem {
    CallHierarchyItem {
        name: def.name.to_string(),
        kind: LspSymbolKind::FUNCTION,
        tags: None,
        detail: None,
        uri: uri.clone(),
        range: text_range_to_lsp_range(line_index, def.full, encoding),
        selection_range: text_range_to_lsp_range(line_index, def.selection, encoding),
        data: None,
    }
}

/// The call-hierarchy item for the top-level function `name` defined in the
/// workspace file at `path`, off the db snapshot. `None` when the file isn't
/// tracked, has no URI, or has no file-scope function of that name.
fn function_item(
    snapshot: &Analysis,
    path: &Path,
    name: &str,
    encoding: PositionEncoding,
) -> Option<CallHierarchyItem> {
    let file = snapshot.lookup_file(path)?;
    let uri = uri::from_path(path)?;
    let root = snapshot.parsed_tree(file);
    let model = snapshot.semantic_model(file);
    let (def, _) = function_defs(&root, model)
        .into_iter()
        .find(|(d, _)| d.name.as_str() == name)?;
    let line_index = snapshot.line_index(file);
    Some(fn_def_to_item(&def, &uri, line_index, encoding))
}

/// `textDocument/prepareCallHierarchy`: resolve the cursor to the top-level
/// function it names (its definition or a call/reference to it), as one or more
/// items. Intra-file is parsed from the live `text` (like [`definition_via_db`]);
/// a bare name with no local binding falls back to the workspace index. `None`
/// when the cursor names no top-level function.
pub(crate) fn prepare_call_hierarchy_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    text: &str,
    position: Position,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyItem>> {
    let line_index = LineIndex::new(text);
    let offset = TextSize::new(
        line_index
            .position_to_byte(position, encoding)
            .min(text.len()) as u32,
    );
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    if let Some(items) = prepare_local(&root, &model, offset, uri, &line_index, encoding) {
        return Some(items);
    }

    // Cross-file: a bare top-level name defined in sibling workspace files. A
    // namespaced (`pkg::name`) name is a package export with no in-tree function.
    let token = pick_name_token(&root, offset)?;
    if token.kind() != SyntaxKind::IDENT
        || matches!(
            symbol_query_at(&root, offset),
            Some(SymbolQuery::Namespaced { .. })
        )
    {
        return None;
    }
    let name = SmolStr::new(token.text());
    let items = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot
            .workspace_def_sites(&name)
            .into_iter()
            // The current file is handled intra-file above; skip it so a stale
            // tracked copy never shadows the live buffer.
            .filter(|(def_path, _)| def_path != path)
            .filter_map(|(def_path, _)| function_item(snapshot, &def_path, &name, encoding))
            .collect::<Vec<_>>()
    }))
    .unwrap_or_default();
    (!items.is_empty()).then_some(items)
}

/// Intra-file half of prepare: the cursor sits on a file-scope function's
/// definition name, or on a read that resolves to one. `None` when neither holds
/// (so the caller falls back to the workspace index). A file-scope *non-function*
/// name resolves to `Some(vec![])` so the caller does not then chase it
/// cross-file — but we collapse that to `None` after confirming it named a local.
fn prepare_local(
    root: &SyntaxNode,
    model: &SemanticModel,
    offset: TextSize,
    uri: &Uri,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyItem>> {
    let token = pick_name_token(root, offset)?;
    if token.kind() != SyntaxKind::IDENT {
        return None;
    }
    let range = token.text_range();
    let defs = function_defs(root, model);

    // Cursor on a function definition's own name.
    if let Some((def, _)) = defs.iter().find(|(d, _)| d.selection == range) {
        return Some(vec![fn_def_to_item(def, uri, line_index, encoding)]);
    }

    // Cursor on a read that resolves to a local file-scope binding.
    let ident = model.idents().iter().find(|i| i.range == range)?;
    let binding = model.resolve_local(ident)?;
    if !model.binding_is_file_scope(binding) {
        // A nested local: not a call-hierarchy subject, and not a cross-file name.
        return Some(Vec::new());
    }
    let def_range = model.binding(binding).def_range;
    Some(
        defs.iter()
            .find(|(d, _)| d.selection == def_range)
            .map(|(d, _)| vec![fn_def_to_item(d, uri, line_index, encoding)])
            .unwrap_or_default(),
    )
}

/// `callHierarchy/incomingCalls`: every top-level function that calls the
/// function the item denotes, each with the call-site ranges within it. Works off
/// the snapshot; `None` on a non-file URI.
pub(crate) fn incoming_calls_via_db(
    snapshot: &Analysis,
    item: &CallHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    let path = uri::to_path(&item.uri)?;
    let name = item.name.clone();
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        incoming_calls(snapshot, &path, &name, encoding)
    }))
    .ok()
    .flatten()
}

fn incoming_calls(
    snapshot: &Analysis,
    def_path: &Path,
    name: &str,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    let binding = snapshot.cross_file_binding(def_path, name);
    // Per caller, keyed by (uri, selection span): the built `from` item and its
    // call-site ranges. Insertion order is preserved for deterministic output.
    let mut groups: Vec<IncomingGroup> = Vec::new();
    // Cohort members read the name through their own file-scope binding; readers
    // through a free read. This is the same split [`cross_file_reference_locations`]
    // uses, narrowed to callee positions.
    for member in &binding.cohort {
        collect_incoming(snapshot, member, name, false, &mut groups, encoding);
    }
    for reader in &binding.readers {
        collect_incoming(snapshot, reader, name, true, &mut groups, encoding);
    }
    Some(
        groups
            .into_iter()
            .map(|g| CallHierarchyIncomingCall {
                from: g.from,
                from_ranges: g.from_ranges,
            })
            .collect(),
    )
}

struct IncomingGroup {
    uri: Uri,
    selection: TextRange,
    from: CallHierarchyItem,
    from_ranges: Vec<Range>,
}

/// Collect `name`'s callee-position references in `file_path` and attribute each
/// to its enclosing top-level function, appending to `groups`. `reader` selects
/// the reference set: free reads for a reader, file-scope-bound reads for a
/// cohort member (which defines the name itself).
fn collect_incoming(
    snapshot: &Analysis,
    file_path: &Path,
    name: &str,
    reader: bool,
    groups: &mut Vec<IncomingGroup>,
    encoding: PositionEncoding,
) {
    let Some(file) = snapshot.lookup_file(file_path) else {
        return;
    };
    let Some(uri) = uri::from_path(file_path) else {
        return;
    };
    let root = snapshot.parsed_tree(file);
    let model = snapshot.semantic_model(file);
    let line_index = snapshot.line_index(file);
    let defs = function_defs(&root, model);

    let ref_ranges: Vec<TextRange> = if reader {
        snapshot.read_ranges_in(file, name)
    } else {
        file_scope_occurrences_in(model, name)
            .map(|(_, reads)| reads)
            .unwrap_or_default()
    };

    for range in ref_ranges {
        if call_at_callee(&root, range).is_none() {
            continue;
        }
        let Some(caller) = enclosing_top_level_function(&root, &defs, range) else {
            continue; // script-level call site: dropped (v1).
        };
        let from_range = text_range_to_lsp_range(line_index, range, encoding);
        match groups
            .iter_mut()
            .find(|g| g.uri == uri && g.selection == caller.selection)
        {
            Some(group) => group.from_ranges.push(from_range),
            None => groups.push(IncomingGroup {
                uri: uri.clone(),
                selection: caller.selection,
                from: fn_def_to_item(&caller, &uri, line_index, encoding),
                from_ranges: vec![from_range],
            }),
        }
    }
}

/// `callHierarchy/outgoingCalls`: every top-level function the item's function
/// calls, each with the call-site ranges within the item's body. Works off the
/// snapshot; `None` on a non-file URI.
pub(crate) fn outgoing_calls_via_db(
    snapshot: &Analysis,
    item: &CallHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let path = uri::to_path(&item.uri)?;
    let name = item.name.clone();
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        outgoing_calls(snapshot, &path, &name, encoding)
    }))
    .ok()
    .flatten()
}

struct OutgoingGroup {
    path: PathBuf,
    name: SmolStr,
    from_ranges: Vec<TextRange>,
}

fn outgoing_calls(
    snapshot: &Analysis,
    path: &Path,
    name: &str,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let file = snapshot.lookup_file(path)?;
    let root = snapshot.parsed_tree(file);
    let model = snapshot.semantic_model(file);
    let line_index = snapshot.line_index(file);

    let defs = function_defs(&root, model);
    let func = defs
        .iter()
        .find(|(d, _)| d.name.as_str() == name)
        .map(|(_, f)| f.clone())?;
    let local_fn_names: HashSet<&str> = defs.iter().map(|(d, _)| d.name.as_str()).collect();
    let Some(NodeOrToken::Node(body)) = func.body() else {
        return Some(Vec::new());
    };

    let mut groups: Vec<OutgoingGroup> = Vec::new();
    for call_node in body.descendants() {
        if call_node.kind() != SyntaxKind::CALL_EXPR {
            continue;
        }
        let Some(call) = CallExpr::cast(call_node.clone()) else {
            continue;
        };
        let Some(callee) = call.callee_token() else {
            continue; // computed callee.
        };
        if callee.kind() != SyntaxKind::IDENT {
            continue;
        }
        // Skip `pkg::name(...)` namespaced calls: package exports have no in-tree
        // location (definition declines them too).
        if call_node
            .parent()
            .and_then(BinaryExpr::cast)
            .and_then(|b| b.namespace_access())
            .is_some()
        {
            continue;
        }
        let callee_name = SmolStr::new(callee.text());
        let Some(target) = resolve_callee(snapshot, path, &local_fn_names, &callee_name, encoding)
        else {
            continue;
        };
        let range = callee.text_range();
        match groups
            .iter_mut()
            .find(|g| g.path == target && g.name == callee_name)
        {
            Some(group) => group.from_ranges.push(range),
            None => groups.push(OutgoingGroup {
                path: target,
                name: callee_name,
                from_ranges: vec![range],
            }),
        }
    }

    Some(
        groups
            .into_iter()
            .filter_map(|g| {
                let to = function_item(snapshot, &g.path, &g.name, encoding)?;
                Some(CallHierarchyOutgoingCall {
                    to,
                    from_ranges: g
                        .from_ranges
                        .iter()
                        .map(|r| text_range_to_lsp_range(line_index, *r, encoding))
                        .collect(),
                })
            })
            .collect(),
    )
}

/// The file that defines `callee_name` as a top-level function, resolved from
/// `from_path`: its own file when it is a local file-scope function there
/// (`sees` excludes self, so this must be checked separately), else the first
/// visible sibling definition. `None` when nothing resolves.
fn resolve_callee(
    snapshot: &Analysis,
    from_path: &Path,
    local_fn_names: &HashSet<&str>,
    callee_name: &str,
    encoding: PositionEncoding,
) -> Option<PathBuf> {
    if local_fn_names.contains(callee_name) {
        return Some(from_path.to_path_buf());
    }
    snapshot
        .visible_def_files(from_path, callee_name)
        .into_iter()
        .find(|p| function_item(snapshot, p, callee_name, encoding).is_some())
}

/// The `CALL_EXPR` node `range` is the callee of, when `range` is exactly that
/// call's simple-name callee and the call is not a `pkg::name(...)` namespaced
/// call. `None` for a value use of the name (an argument, a bare read) or a
/// namespaced call.
fn call_at_callee(root: &SyntaxNode, range: TextRange) -> Option<SyntaxNode> {
    let token = match root.token_at_offset(range.start()) {
        TokenAtOffset::None => return None,
        TokenAtOffset::Single(t) => t,
        TokenAtOffset::Between(left, right) => {
            if left.text_range() == range {
                left
            } else {
                right
            }
        }
    };
    let call = token.parent()?;
    if call.kind() != SyntaxKind::CALL_EXPR {
        return None;
    }
    if CallExpr::cast(call.clone())?.callee_token()?.text_range() != range {
        return None;
    }
    if call
        .parent()
        .and_then(BinaryExpr::cast)
        .and_then(|b| b.namespace_access())
        .is_some()
    {
        return None;
    }
    Some(call)
}

/// The top-level function definition whose body contains `range`, or `None` when
/// `range`'s enclosing top-level statement is not a function definition (a
/// script-level call). A call nested inside a local function is attributed to the
/// enclosing top-level function (the top-level statement is its outermost def).
fn enclosing_top_level_function(
    root: &SyntaxNode,
    defs: &[(FnDef, FunctionExpr)],
    range: TextRange,
) -> Option<FnDef> {
    let stmt = root
        .children()
        .find(|c| c.text_range().contains_range(range))?;
    defs.iter()
        .find(|(d, _)| d.full == stmt.text_range())
        .map(|(d, _)| d.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prepare against the live buffer for the cursor at `offset`.
    fn prepare_at(
        snapshot: &Analysis,
        path: &Path,
        text: &str,
        offset: usize,
    ) -> Vec<CallHierarchyItem> {
        let uri = uri::from_path(path).unwrap();
        prepare_call_hierarchy_via_db(
            snapshot,
            path,
            &uri,
            text,
            pos_at(text, offset),
            PositionEncoding::Utf16,
        )
        .unwrap_or_default()
    }

    fn item_named(snapshot: &Analysis, path: &Path, name: &str) -> CallHierarchyItem {
        function_item(snapshot, path, name, PositionEncoding::Utf16).expect("function item")
    }

    // --- prepare ------------------------------------------------------------

    #[test]
    fn prepare_on_a_definition_yields_its_item() {
        let src = "foo <- function() 1\n";
        let snapshot = rename_workspace(src, "");
        let items = prepare_at(&snapshot, &ws_path("a.R"), src, src.find("foo").unwrap());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "foo");
        assert_eq!(items[0].kind, LspSymbolKind::FUNCTION);
    }

    #[test]
    fn prepare_on_a_call_site_yields_the_callee_item() {
        let src = "foo <- function() 1\nbar <- function() foo()\n";
        let snapshot = rename_workspace(src, "");
        let offset = src.find("foo()").unwrap();
        let items = prepare_at(&snapshot, &ws_path("a.R"), src, offset);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "foo");
    }

    #[test]
    fn prepare_declines_a_non_function_binding() {
        let src = "x <- 1\nprint(x)\n";
        let snapshot = rename_workspace(src, "");
        let items = prepare_at(&snapshot, &ws_path("a.R"), src, src.find("x <-").unwrap());
        assert!(items.is_empty());
    }

    #[test]
    fn prepare_resolves_a_cross_file_callee() {
        // b.R sources a.R and calls foo; prepare on the read resolves to a.R.
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let items = prepare_at(
            &snapshot,
            &ws_path("b.R"),
            b_src,
            b_src.find("foo()").unwrap(),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "foo");
        assert_eq!(items[0].uri, uri::from_path(&ws_path("a.R")).unwrap());
    }

    // --- outgoing -----------------------------------------------------------

    #[test]
    fn outgoing_collects_intra_file_calls() {
        let src = "helper <- function() 1\nmain <- function() {\n  helper()\n  helper()\n}\n";
        let snapshot = rename_workspace(src, "");
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "main"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.name, "helper");
        assert_eq!(calls[0].from_ranges.len(), 2, "both call sites reported");
    }

    #[test]
    fn outgoing_skips_namespaced_and_unresolved_calls() {
        let src = "main <- function() {\n  dplyr::filter(x)\n  undefined_fn()\n}\n";
        let snapshot = rename_workspace(src, "");
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "main"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert!(calls.is_empty(), "no top-level function callee resolves");
    }

    #[test]
    fn outgoing_resolves_a_cross_file_callee() {
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("b.R"), "bar"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.name, "foo");
        assert_eq!(calls[0].to.uri, uri::from_path(&ws_path("a.R")).unwrap());
    }

    // --- incoming -----------------------------------------------------------

    #[test]
    fn incoming_finds_callers_across_a_source_edge() {
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "bar");
        assert_eq!(calls[0].from.uri, uri::from_path(&ws_path("b.R")).unwrap());
        assert_eq!(calls[0].from_ranges.len(), 1);
    }

    #[test]
    fn incoming_drops_script_level_calls() {
        // The call to foo is at script top level, inside no function.
        let src = "foo <- function() 1\nfoo()\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert!(calls.is_empty(), "script-level call site is dropped in v1");
    }

    #[test]
    fn incoming_attributes_a_nested_call_to_the_top_level_function() {
        let src = "foo <- function() 1\nouter <- function() {\n  inner <- function() foo()\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "outer");
    }

    #[test]
    fn incoming_excludes_a_value_use() {
        // foo passed as a value, not called: not an incoming call edge.
        let src = "foo <- function() 1\nbar <- function() lapply(xs, foo)\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert!(calls.is_empty(), "a value use is not a call");
    }

    #[test]
    fn incoming_excludes_a_disjoint_same_name_def() {
        // Two unconnected scripts each define their own foo; calling one must not
        // surface the other's caller.
        let a_src = "foo <- function() 1\n";
        let b_src = "foo <- function() 2\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert!(
            calls.is_empty(),
            "b.R's foo is a disjoint binding; a.R's foo has no callers"
        );
    }
}
