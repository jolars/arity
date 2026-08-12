//! Call hierarchy (`textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls`,
//! `callHierarchy/outgoingCalls`).
//!
//! Items are **named function definitions at any scope**: file-scope functions —
//! the names the cross-file index keys on ([`Analysis::workspace_def_sites`],
//! [`Analysis::cross_file_binding`], [`Analysis::visible_def_files`]) — and
//! nested/local ones. An edge is a *callee-position* use `F(...)`, never a value
//! use (`lapply(xs, F)`).
//!
//! A call is attributed to the **innermost enclosing named function**
//! ([`enclosing_function`]). Anonymous function scopes own no item, so their calls
//! fall through to the nearest named ancestor, and a call inside *no* function
//! falls through to the file's synthetic **script-scope** item ([`script_item`]) —
//! in an R script that is where most calls live. Incoming and outgoing share that
//! one predicate, so the two directions can never disagree: an edge is in
//! `outgoing(A)` exactly when `incoming(B)` groups it under `A`. Callees resolve
//! through the scope tree ([`resolve_callee`]), so a nested `helper` shadows a
//! sibling file's top-level `helper` instead of misresolving to it; a free read
//! that several visible siblings define yields one edge per candidate rather than
//! a silent first-wins pick.
//!
//! An item's identity is its enclosing-function **name chain**, round-tripped in
//! [`CallHierarchyItem::data`] (see [`ItemData`]). A bare name cannot identify a
//! nested item — two functions may each contain an `inner` — and a *range* would
//! go stale, because `prepare` parses the live buffer while `incoming`/`outgoing`
//! read the db snapshot, which the lint thread only catches up to asynchronously.
//! A name chain is edit-stable, and is length 1 for a file-scope function, so the
//! old name-only key is its degenerate case.
//!
//! Nested names are file-private (that is [`SemanticModel::binding_is_file_scope`]'s
//! contract, and why [`crate::project::DefIndex`] never keys on them), so a nested
//! item's incoming edges are intra-file by construction.
//!
//! `prepare` parses the live buffer (a deliberate, infrequent action, like
//! [`definition_via_db`]); `incoming`/`outgoing` work purely off the db snapshot.
//! Snapshot reads are wrapped in [`salsa::Cancelled::catch`].

use super::*;

/// A named function definition in one file, at any scope.
#[derive(Debug, Clone)]
struct FnDef {
    /// The chain of enclosing *named* functions ending with this one's own name —
    /// the item's identity within a file. Length 1 for a file-scope function.
    /// Anonymous function scopes contribute no segment.
    ///
    /// Two same-named definitions under one named ancestor collapse to a single
    /// item, the first in binding order winning. For a *reassignment*
    /// (`inner <- function() 1; inner <- function() 2`) that is correct — the
    /// [`variable_occurrences`] cohort already merges their reads. For two sibling
    /// *anonymous* functions that each define `inner` it over-merges; that is the
    /// same first-def-wins rule [`SemanticModel::resolve_local`] itself uses.
    path: Vec<SmolStr>,
    /// The binding this definition introduces — the join key for resolving a
    /// callee or read back to its item, and the handle for its read sites.
    binding: BindingId,
    /// The defining identifier.
    selection: TextRange,
    /// The whole assignment statement.
    full: TextRange,
}

impl FnDef {
    /// The bound name — the last segment of the path.
    fn name(&self) -> &str {
        self.path.last().map_or("", SmolStr::as_str)
    }
}

/// Every named function definition in one file, plus the scope index that
/// attributes a call site to the innermost one containing it.
struct FileFunctions {
    /// In [`SemanticModel::bindings`] order, so a `find` is first-wins — the same
    /// tiebreak [`SemanticModel::resolve_local`] uses.
    defs: Vec<(FnDef, FunctionExpr)>,
    /// A function's body scope -> its index in `defs`. Anonymous function scopes
    /// are absent, which is what makes them transparent to attribution.
    owners: HashMap<ScopeId, usize>,
}

impl FileFunctions {
    fn def(&self, idx: usize) -> &FnDef {
        &self.defs[idx].0
    }

    fn by_path(&self, path: &[SmolStr]) -> Option<usize> {
        self.defs.iter().position(|(d, _)| d.path == path)
    }

    fn by_binding(&self, binding: BindingId) -> Option<usize> {
        self.defs.iter().position(|(d, _)| d.binding == binding)
    }

    fn by_selection(&self, range: TextRange) -> Option<usize> {
        self.defs.iter().position(|(d, _)| d.selection == range)
    }
}

/// Every binding in `model` whose value is a function literal, paired with that
/// function node. The single definition of "what counts as a call-hierarchy
/// function". The CST supplies the function value and the full assignment span;
/// the model supplies the scope nesting and the canonical (unquoted) name.
fn function_defs(root: &SyntaxNode, model: &SemanticModel) -> FileFunctions {
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
    // The builder gives every `Function` scope the `FUNCTION_EXPR` node's exact
    // text range, so that range is an exact join key from a function value back to
    // the scope its body opens.
    let scope_by_range: HashMap<TextRange, ScopeId> = model
        .scopes()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == ScopeKind::Function)
        .map(|(i, s)| (s.range, ScopeId::from_index(i)))
        .collect();

    // Pass A: the definitions themselves, and the body-scope -> index map that
    // pass B needs to name each enclosing function.
    let mut defs: Vec<(FnDef, FunctionExpr)> = Vec::new();
    let mut owners: HashMap<ScopeId, usize> = HashMap::new();
    for (i, b) in model.bindings().iter().enumerate() {
        if !matches!(b.kind, BindingKind::Local | BindingKind::Implicit) {
            continue;
        }
        let Some((assign, func)) = by_target.get(&b.def_range) else {
            continue;
        };
        if let Some(scope) = scope_by_range.get(&func.syntax().text_range()) {
            // First-wins guards the `f <- g <- function() …` shape, where two
            // bindings share one function value.
            owners.entry(*scope).or_insert(defs.len());
        }
        defs.push((
            FnDef {
                // Filled in by pass B, which needs `owners` complete.
                path: vec![b.name.clone()],
                binding: BindingId::from_index(i),
                selection: b.def_range,
                full: assign.text_range(),
            },
            func.clone(),
        ));
    }

    // Pass B: prefix each name with its chain of enclosing named functions. The
    // chain comes from the *binding's* scope, never textual containment: `<<-`
    // binds at an enclosing scope, so `outer <- function() { helper <<- ... }`
    // defines a file-scope `helper` whose path is `["helper"]`.
    for idx in 0..defs.len() {
        let mut chain: Vec<SmolStr> = Vec::new();
        let mut current = Some(model.binding(defs[idx].0.binding).scope);
        while let Some(scope_id) = current {
            let scope = model.scope(scope_id);
            if scope.kind == ScopeKind::Function
                && let Some(&owner) = owners.get(&scope_id)
            {
                chain.push(SmolStr::new(defs[owner].0.name()));
            }
            current = scope.parent;
        }
        chain.reverse();
        chain.push(SmolStr::new(defs[idx].0.name()));
        defs[idx].0.path = chain;
    }

    FileFunctions { defs, owners }
}

/// What an item denotes: a named function, or a file's **script scope** — the
/// top level, which owns every call written inside no function.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemId {
    /// The enclosing-function name chain, ending with the function's own name.
    Function(Vec<SmolStr>),
    /// The file's top level. Never a *callee* (nothing calls a script), so it
    /// appears only as the `from` of an incoming edge; its own outgoing edges are
    /// the file's top-level calls.
    ScriptScope,
}

/// The identity payload round-tripped through [`CallHierarchyItem::data`]: the
/// item's enclosing-function name chain, or the script-scope marker. See the
/// module doc for why identity is symbolic rather than positional.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ItemData {
    path: Vec<SmolStr>,
    /// Set only on the script-scope item, whose `path` is empty — a name chain
    /// cannot name the top level. Skipped when false so a function item's payload
    /// stays byte-identical to the pre-script-scope encoding.
    #[serde(default, skip_serializing_if = "is_false")]
    script: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// What `item` identifies, falling back to its bare `name` as a file-scope
/// function when `data` is absent or undecodable — an item minted before this
/// field existed, or a client that dropped it. The fallback is exactly the old
/// file-scope-only behavior; its one hazard is that a *nested* item stripped of
/// its `data` would then name a same-named file-scope function, if one exists.
/// The LSP spec requires clients to preserve `data`, so this is a degradation
/// path, not a design assumption.
fn item_id(item: &CallHierarchyItem) -> ItemId {
    let data = item
        .data
        .clone()
        .and_then(|data| serde_json::from_value::<ItemData>(data).ok());
    match data {
        Some(d) if d.script => ItemId::ScriptScope,
        Some(d) if !d.path.is_empty() => ItemId::Function(d.path),
        _ => ItemId::Function(vec![SmolStr::new(&item.name)]),
    }
}

/// The enclosing-function chain shown as an item's `detail`, disambiguating
/// same-named nested items in the client's tree (`outer`, `outer/mid`). `None` for
/// a file-scope function — there is nothing to disambiguate.
fn nested_detail(path: &[SmolStr]) -> Option<String> {
    let enclosing = path.split_last()?.1;
    (!enclosing.is_empty()).then(|| {
        enclosing
            .iter()
            .map(SmolStr::as_str)
            .collect::<Vec<_>>()
            .join("/")
    })
}

/// Build a [`CallHierarchyItem`] for `def`, mapping its spans through `line_index`.
fn fn_def_to_item(
    def: &FnDef,
    uri: &Uri,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> CallHierarchyItem {
    CallHierarchyItem {
        name: def.name().to_string(),
        kind: LspSymbolKind::FUNCTION,
        tags: None,
        detail: nested_detail(&def.path),
        uri: uri.clone(),
        range: text_range_to_lsp_range(line_index, def.full, encoding),
        selection_range: text_range_to_lsp_range(line_index, def.selection, encoding),
        data: serde_json::to_value(ItemData {
            path: def.path.clone(),
            script: false,
        })
        .ok(),
    }
}

/// The synthetic item for a file's script scope, named after the file.
///
/// Its `range` is the whole file and its `selection_range` the start of it: there
/// is no defining identifier to point at, and the client uses the selection only
/// to navigate. Neither range participates in identity — [`ItemData::script`]
/// does — so neither can go stale the way a function's would.
fn script_item(
    file_path: &Path,
    uri: &Uri,
    root: &SyntaxNode,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> CallHierarchyItem {
    let start = TextSize::new(0);
    CallHierarchyItem {
        name: file_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<script>".to_string()),
        kind: LspSymbolKind::FILE,
        tags: None,
        detail: Some("top level".to_string()),
        uri: uri.clone(),
        range: text_range_to_lsp_range(
            line_index,
            TextRange::new(start, root.text_range().end()),
            encoding,
        ),
        selection_range: text_range_to_lsp_range(line_index, TextRange::empty(start), encoding),
        data: serde_json::to_value(ItemData {
            path: Vec::new(),
            script: true,
        })
        .ok(),
    }
}

/// The call-hierarchy item for the function `fn_path` names in the workspace file
/// at `path`, off the db snapshot. `None` when the file isn't tracked, has no URI,
/// or defines no such function.
fn function_item(
    snapshot: &Analysis,
    path: &Path,
    fn_path: &[SmolStr],
    encoding: PositionEncoding,
) -> Option<CallHierarchyItem> {
    let file = snapshot.lookup_file(path)?;
    let uri = uri::from_path(path)?;
    let root = snapshot.parsed_tree(file);
    let model = snapshot.semantic_model(file);
    let functions = function_defs(&root, model);
    let def = functions.def(functions.by_path(fn_path)?);
    let line_index = snapshot.line_index(file);
    Some(fn_def_to_item(def, &uri, line_index, encoding))
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
    buffer: &TextBuffer,
    position: Position,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyItem>> {
    let text = buffer.text();
    let line_index = buffer.line_index();
    let offset = TextSize::new(
        line_index
            .position_to_byte(position, encoding)
            .min(text.len()) as u32,
    );
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    if let Some(items) = prepare_local(&root, &model, offset, uri, line_index, encoding) {
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
    let fn_path = std::slice::from_ref(&name);
    let items = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot
            .workspace_def_sites(&name)
            .into_iter()
            // The current file is handled intra-file above; skip it so a stale
            // tracked copy never shadows the live buffer.
            .filter(|(def_path, _)| def_path != path)
            .filter_map(|(def_path, _)| function_item(snapshot, &def_path, fn_path, encoding))
            .collect::<Vec<_>>()
    }))
    .unwrap_or_default();
    (!items.is_empty()).then_some(items)
}

/// Intra-file half of prepare: the cursor sits on a function's definition name (at
/// any scope), or on a read that resolves to one. `None` when the cursor names no
/// local binding at all, so the caller falls back to the workspace index. A local
/// that is *not* a function resolves to `Some(vec![])`: it names no function here,
/// and it is not a cross-file name either, so the caller must not chase it through
/// the workspace index.
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
    let functions = function_defs(root, model);

    // Cursor on a definition's own name. A *function* definition is the item; any
    // other definition ends the search — it names this file's binding, so chasing
    // the name through the workspace index would resolve to an unrelated sibling.
    if model.bindings().iter().any(|b| b.def_range == range) {
        return Some(
            functions
                .by_selection(range)
                .map(|idx| {
                    vec![fn_def_to_item(
                        functions.def(idx),
                        uri,
                        line_index,
                        encoding,
                    )]
                })
                .unwrap_or_default(),
        );
    }

    // Cursor on a read that resolves to a local binding, at any scope.
    let ident = model.idents().iter().find(|i| i.range == range)?;
    let binding = model.resolve_local(ident)?;
    Some(
        functions
            .by_binding(binding)
            .map(|idx| {
                vec![fn_def_to_item(
                    functions.def(idx),
                    uri,
                    line_index,
                    encoding,
                )]
            })
            .unwrap_or_default(),
    )
}

/// `callHierarchy/incomingCalls`: every caller of the function the item denotes —
/// a named function, or a file's script scope — each with the call-site ranges
/// within it. Empty for the script-scope item, which nothing calls. Works off the
/// snapshot; `None` on a non-file URI.
pub(crate) fn incoming_calls_via_db(
    snapshot: &Analysis,
    item: &CallHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    let path = uri::to_path(&item.uri)?;
    let id = item_id(item);
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        incoming_calls(snapshot, &path, &id, encoding)
    }))
    .ok()
    .flatten()
}

/// Which reference set to scan in a file for incoming edges.
enum RefSet<'a> {
    /// Reads bound to this file's own top-level definition of the name — a cohort
    /// member, which defines it itself.
    FileScope(&'a str),
    /// Free reads of the name — a reader file, which binds it from elsewhere.
    Free(&'a str),
    /// Reads of the local definition this path names. Always in the defining file:
    /// a nested name is file-private.
    Local(&'a [SmolStr]),
}

fn incoming_calls(
    snapshot: &Analysis,
    def_path: &Path,
    id: &ItemId,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    // Nothing calls a script's top level.
    let ItemId::Function(fn_path) = id else {
        return Some(Vec::new());
    };
    // Per caller, keyed by (uri, caller identity): the built `from` item and its
    // call-site ranges. Insertion order is preserved for deterministic output.
    let mut groups: Vec<IncomingGroup> = Vec::new();
    match fn_path.as_slice() {
        // File-scope: the name is in the cross-file index, so walk the visibility
        // component. Cohort members read it through their own file-scope binding,
        // readers through a free read — the same split
        // [`cross_file_reference_locations`] uses, narrowed to callee positions.
        [name] => {
            let binding = snapshot.cross_file_binding(def_path, name);
            for member in &binding.cohort {
                collect_incoming(
                    snapshot,
                    member,
                    RefSet::FileScope(name),
                    &mut groups,
                    encoding,
                );
            }
            for reader in &binding.readers {
                collect_incoming(snapshot, reader, RefSet::Free(name), &mut groups, encoding);
            }
        }
        // Nested: file-private, so the defining file is the whole world.
        nested => collect_incoming(
            snapshot,
            def_path,
            RefSet::Local(nested),
            &mut groups,
            encoding,
        ),
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
    /// The caller's identity, so a function and its file's script scope stay
    /// distinct groups even though both are `from` items in the same file.
    id: ItemId,
    from: CallHierarchyItem,
    from_ranges: Vec<Range>,
}

/// Collect the callee-position references `refs` selects in `file_path` and
/// attribute each to its innermost enclosing named function, appending to `groups`.
fn collect_incoming(
    snapshot: &Analysis,
    file_path: &Path,
    refs: RefSet<'_>,
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
    let functions = function_defs(&root, model);

    let ref_ranges: Vec<TextRange> = match refs {
        RefSet::Free(name) => snapshot.read_ranges_in(file, name),
        RefSet::FileScope(name) => file_scope_occurrences_in(model, name)
            .map(|(_, reads)| reads)
            .unwrap_or_default(),
        // `variable_occurrences` (not the raw read sites) so a reassigned nested
        // function merges its cohort's reads, exactly as the file-scope path does.
        RefSet::Local(path) => functions
            .by_path(path)
            .map(|idx| variable_occurrences(model, functions.def(idx).binding).1)
            .unwrap_or_default(),
    };

    for range in ref_ranges {
        if call_at_callee(&root, range).is_none() {
            continue;
        }
        // A call inside no function belongs to the file's script scope, not to
        // nothing: in an R *script* that is where most calls live.
        let (id, from) = match enclosing_function(model, &functions.owners, range) {
            Some(idx) => {
                let caller = functions.def(idx);
                (
                    ItemId::Function(caller.path.clone()),
                    fn_def_to_item(caller, &uri, line_index, encoding),
                )
            }
            None => (
                ItemId::ScriptScope,
                script_item(file_path, &uri, &root, line_index, encoding),
            ),
        };
        let from_range = text_range_to_lsp_range(line_index, range, encoding);
        match groups.iter_mut().find(|g| g.uri == uri && g.id == id) {
            Some(group) => group.from_ranges.push(from_range),
            None => groups.push(IncomingGroup {
                uri: uri.clone(),
                id,
                from,
                from_ranges: vec![from_range],
            }),
        }
    }
}

/// `callHierarchy/outgoingCalls`: every function the item calls, each with the
/// call-site ranges within it — the item's body for a function, the file's top
/// level for the script scope. Works off the snapshot; `None` on a non-file URI.
pub(crate) fn outgoing_calls_via_db(
    snapshot: &Analysis,
    item: &CallHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let path = uri::to_path(&item.uri)?;
    let id = item_id(item);
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        outgoing_calls(snapshot, &path, &id, encoding)
    }))
    .ok()
    .flatten()
}

/// A resolved outgoing callee: a definition in the calling file (identified by its
/// name chain, so a nested one is addressable) or a sibling file's top-level
/// function.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CalleeTarget {
    Local { path: Vec<SmolStr> },
    CrossFile { file: PathBuf, name: SmolStr },
}

struct OutgoingGroup {
    target: CalleeTarget,
    from_ranges: Vec<TextRange>,
}

fn outgoing_calls(
    snapshot: &Analysis,
    path: &Path,
    id: &ItemId,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let file = snapshot.lookup_file(path)?;
    let uri = uri::from_path(path)?;
    let root = snapshot.parsed_tree(file);
    let model = snapshot.semantic_model(file);
    let line_index = snapshot.line_index(file);

    let functions = function_defs(&root, model);
    // What to walk, and which owner a call must attribute to. `None` is the script
    // scope, so the same `enclosing_function` predicate decides both cases —
    // which is what keeps outgoing in step with incoming.
    let (scan, owner) = match id {
        ItemId::Function(fn_path) => {
            let idx = functions.by_path(fn_path)?;
            // The whole `FUNCTION_EXPR`, not just its body: a call in a parameter
            // default (`main <- function(x = helper()) x`) is a call this function
            // makes, and incoming already reports it.
            (functions.defs[idx].1.syntax().clone(), Some(idx))
        }
        ItemId::ScriptScope => (root.clone(), None),
    };

    let mut groups: Vec<OutgoingGroup> = Vec::new();
    for call_node in scan.descendants() {
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
        // Only this item's *own* calls: a call inside a nested named function is
        // that function's outgoing edge, not ours. Anonymous bodies own no item, so
        // their calls stay here — the same predicate incoming attributes by, which
        // is what makes the two directions agree.
        if enclosing_function(model, &functions.owners, callee.text_range()) != owner {
            continue;
        }
        let range = callee.text_range();
        // One call site can yield several targets when the callee is ambiguous.
        for target in resolve_callee(snapshot, model, &functions, path, &callee, encoding) {
            match groups.iter_mut().find(|g| g.target == target) {
                Some(group) => group.from_ranges.push(range),
                None => groups.push(OutgoingGroup {
                    target,
                    from_ranges: vec![range],
                }),
            }
        }
    }

    Some(
        groups
            .into_iter()
            .filter_map(|g| {
                let to = match &g.target {
                    CalleeTarget::Local { path } => {
                        let idx = functions.by_path(path)?;
                        fn_def_to_item(functions.def(idx), &uri, line_index, encoding)
                    }
                    CalleeTarget::CrossFile { file, name } => {
                        function_item(snapshot, file, std::slice::from_ref(name), encoding)?
                    }
                };
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

/// Where `callee` resolves. **Scope-aware**: the callee's read is resolved through
/// the scope tree, so a callee bound to a *local* definition (at any scope)
/// resolves to that definition and never to a sibling file's same-named top-level
/// function — a nested `helper` shadows `b.R`'s `helper`.
///
/// A callee bound to a local *non-function* (a parameter holding a callback, a
/// value) resolves to nothing. R would look past a non-function binding when
/// resolving a callee, but arity models no values, so we decline rather than guess
/// a sibling file's definition.
///
/// A local binding resolves to exactly one target. A genuinely *free* read falls
/// through to the cross-file index, where **every** visible definition of the name
/// is a target: with more than one in scope, which one R would reach is a runtime
/// fact ([`Analysis::visible_def_files`] treats >1 as unresolved for the same
/// reason). Reporting them all keeps the ambiguity visible — and matches
/// [`prepare_call_hierarchy_via_db`], which already returns one item per candidate
/// — instead of silently picking the first.
fn resolve_callee(
    snapshot: &Analysis,
    model: &SemanticModel,
    functions: &FileFunctions,
    from_path: &Path,
    callee: &SyntaxToken<RLanguage>,
    encoding: PositionEncoding,
) -> Vec<CalleeTarget> {
    let range = callee.text_range();
    if let Some(ident) = model.idents().iter().find(|i| i.range == range)
        && let Some(binding) = model.resolve_local(ident)
    {
        // Bound locally: a function here, or nothing.
        return functions
            .by_binding(binding)
            .map(|idx| CalleeTarget::Local {
                path: functions.def(idx).path.clone(),
            })
            .into_iter()
            .collect();
    }
    let name = SmolStr::new(callee.text());
    snapshot
        .visible_def_files(from_path, &name)
        .into_iter()
        .filter(|p| function_item(snapshot, p, std::slice::from_ref(&name), encoding).is_some())
        .map(|file| CalleeTarget::CrossFile {
            file,
            name: name.clone(),
        })
        .collect()
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

/// The index in `defs` of the innermost enclosing *named* function whose scope
/// contains `range` — the item a call there belongs to. Walks outward from
/// [`SemanticModel::innermost_scope_at`] via `Scope::parent`, stepping past
/// anonymous function scopes (which have no `owners` entry), so a call in
/// `lapply(xs, function(x) foo(x))` belongs to the enclosing named function.
///
/// `None` at file scope: a call site at script top-level, inside no function,
/// which belongs to the file's script-scope item ([`script_item`]).
fn enclosing_function(
    model: &SemanticModel,
    owners: &HashMap<ScopeId, usize>,
    range: TextRange,
) -> Option<usize> {
    let mut current = Some(model.innermost_scope_at(range.start()));
    while let Some(scope_id) = current {
        if let Some(&idx) = owners.get(&scope_id) {
            return Some(idx);
        }
        current = model.scope(scope_id).parent;
    }
    None
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
            &buf(text),
            pos_at(text, offset),
            PositionEncoding::Utf16,
        )
        .unwrap_or_default()
    }

    fn item_named(snapshot: &Analysis, path: &Path, name: &str) -> CallHierarchyItem {
        nested_item(snapshot, path, &[name])
    }

    /// The item for the function `fn_path` names in `path`, off the db snapshot.
    fn nested_item(snapshot: &Analysis, path: &Path, fn_path: &[&str]) -> CallHierarchyItem {
        let fn_path: Vec<SmolStr> = fn_path.iter().copied().map(SmolStr::new).collect();
        function_item(snapshot, path, &fn_path, PositionEncoding::Utf16).expect("function item")
    }

    /// The name chain an item carries in its `data` payload — its identity.
    fn path_of(item: &CallHierarchyItem) -> Vec<String> {
        match item_id(item) {
            ItemId::Function(path) => path.iter().map(SmolStr::to_string).collect(),
            ItemId::ScriptScope => panic!("expected a function item, got the script scope"),
        }
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
    fn prepare_on_a_nested_definition_yields_its_item() {
        let src = "outer <- function() {\n  inner <- function() 1\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let items = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            src,
            src.find("inner <-").unwrap(),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "inner");
        assert_eq!(items[0].kind, LspSymbolKind::FUNCTION);
        assert_eq!(items[0].detail.as_deref(), Some("outer"));
        assert_eq!(path_of(&items[0]), ["outer", "inner"]);
    }

    #[test]
    fn prepare_on_a_nested_call_site_yields_the_nested_item() {
        let src = "outer <- function() {\n  inner <- function() 1\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let items = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            src,
            src.find("inner()").unwrap(),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "inner");
        assert_eq!(path_of(&items[0]), ["outer", "inner"]);
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

    #[test]
    fn prepare_declines_a_nested_non_function_local() {
        // `x` is a nested value; b.R's top-level `x` must not be chased.
        let a_src = "outer <- function() {\n  x <- 1\n  print(x)\n}\n";
        let b_src = "x <- function() 1\n";
        let snapshot = rename_workspace(a_src, b_src);
        let items = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            a_src,
            a_src.find("x <- 1").unwrap(),
        );
        assert!(items.is_empty(), "a nested non-function is not an item");
    }

    #[test]
    fn prepare_prefers_a_shadowing_nested_function_over_a_sibling_file() {
        let a_src =
            "source(\"b.R\")\nouter <- function() {\n  helper <- function() 1\n  helper()\n}\n";
        let b_src = "helper <- function() 2\n";
        let snapshot = rename_workspace(a_src, b_src);
        let a = ws_path("a.R");
        let items = prepare_at(&snapshot, &a, a_src, a_src.find("helper()").unwrap());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].uri, uri::from_path(&a).unwrap());
        assert_eq!(path_of(&items[0]), ["outer", "helper"]);
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

    #[test]
    fn outgoing_reports_a_nested_function_as_a_callee() {
        let src = "outer <- function() {\n  inner <- function() 1\n  inner()\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "outer"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.name, "inner");
        assert_eq!(calls[0].to.detail.as_deref(), Some("outer"));
        assert_eq!(calls[0].to.uri, uri::from_path(&a).unwrap());
        assert_eq!(calls[0].from_ranges.len(), 2);
    }

    #[test]
    fn outgoing_stops_at_a_nested_function_boundary() {
        let src = "foo <- function() 1\nouter <- function() {\n  inner <- function() foo()\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let outer = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "outer"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert_eq!(outer.len(), 1, "foo() belongs to inner, not outer");
        assert_eq!(outer[0].to.name, "inner");

        let inner = outgoing_calls_via_db(
            &snapshot,
            &nested_item(&snapshot, &a, &["outer", "inner"]),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].to.name, "foo");
    }

    #[test]
    fn outgoing_prefers_a_shadowing_local_over_a_sibling_definition() {
        let a_src =
            "source(\"b.R\")\nouter <- function() {\n  helper <- function() 1\n  helper()\n}\n";
        let b_src = "helper <- function() 2\n";
        let snapshot = rename_workspace(a_src, b_src);
        let a = ws_path("a.R");
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "outer"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.uri, uri::from_path(&a).unwrap());
        assert_eq!(path_of(&calls[0].to), ["outer", "helper"]);
    }

    #[test]
    fn outgoing_skips_a_call_through_a_non_function_local() {
        // `helper` is a parameter holding a callback, not b.R's function.
        let a_src = "source(\"b.R\")\nouter <- function(helper) {\n  helper()\n}\n";
        let b_src = "helper <- function() 2\n";
        let snapshot = rename_workspace(a_src, b_src);
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "outer"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert!(
            calls.is_empty(),
            "a parameter shadows the sibling definition"
        );
    }

    #[test]
    fn outgoing_includes_a_call_in_a_parameter_default() {
        let src = "helper <- function() 1\nmain <- function(x = helper()) x\n";
        let snapshot = rename_workspace(src, "");
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "main"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.name, "helper");
    }

    #[test]
    fn prepare_round_trips_a_nested_item_into_outgoing() {
        // The item goes to outgoing exactly as prepare built it, so this pins the
        // real `data` payload rather than a test-reconstructed one.
        let src = "foo <- function() 1\nouter <- function() {\n  inner <- function() foo()\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let items = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            src,
            src.find("inner <-").unwrap(),
        );
        assert_eq!(items.len(), 1);
        let calls =
            outgoing_calls_via_db(&snapshot, &items[0], PositionEncoding::Utf16).expect("outgoing");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to.name, "foo");
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

    // --- script scope -------------------------------------------------------

    #[test]
    fn incoming_reports_a_script_level_call_site() {
        // The calls to foo are at script top level, inside no function: they are
        // attributed to the file's synthetic script-scope item.
        let src = "foo <- function() 1\nfoo()\nfoo()\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1, "both sites group under one script item");
        assert_eq!(calls[0].from.name, "a.R");
        assert_eq!(calls[0].from.kind, LspSymbolKind::FILE);
        assert_eq!(calls[0].from.uri, uri::from_path(&a).unwrap());
        assert_eq!(calls[0].from_ranges.len(), 2);
    }

    #[test]
    fn incoming_keeps_the_script_scope_separate_from_a_function_caller() {
        let src = "foo <- function() 1\nbar <- function() foo()\nfoo()\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        let names: Vec<&str> = calls.iter().map(|c| c.from.name.as_str()).collect();
        assert_eq!(names, ["bar", "a.R"]);
    }

    #[test]
    fn incoming_reports_a_script_level_caller_across_a_source_edge() {
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nfoo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "b.R");
        assert_eq!(calls[0].from.uri, uri::from_path(&ws_path("b.R")).unwrap());
    }

    #[test]
    fn script_item_round_trips_from_incoming_into_outgoing() {
        // The script item goes to outgoing exactly as incoming built it, so this
        // pins the real `data` payload rather than a test-reconstructed one.
        let src = "foo <- function() 1\nbar <- function() 2\nfoo()\nbar()\nfoo()\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let incoming = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(incoming.len(), 1);
        let script = &incoming[0].from;

        let calls = outgoing_calls_via_db(&snapshot, script, PositionEncoding::Utf16)
            .expect("outgoing from the script item");
        let names: Vec<&str> = calls.iter().map(|c| c.to.name.as_str()).collect();
        assert_eq!(
            names,
            ["foo", "bar"],
            "every top-level call, in source order"
        );
        assert_eq!(calls[0].from_ranges.len(), 2, "both foo() sites");
    }

    #[test]
    fn outgoing_from_the_script_item_excludes_calls_inside_functions() {
        // `helper()` is called from inside `main`, never at top level.
        let src = "helper <- function() 1\nmain <- function() helper()\nmain()\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let incoming = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "main"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(incoming.len(), 1);
        let calls = outgoing_calls_via_db(&snapshot, &incoming[0].from, PositionEncoding::Utf16)
            .expect("outgoing");
        let names: Vec<&str> = calls.iter().map(|c| c.to.name.as_str()).collect();
        assert_eq!(names, ["main"], "helper() belongs to main, not the script");
    }

    #[test]
    fn incoming_for_the_script_item_is_empty() {
        // Nothing calls a script's top level.
        let src = "foo <- function() 1\nfoo()\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let incoming = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        let calls = incoming_calls_via_db(&snapshot, &incoming[0].from, PositionEncoding::Utf16)
            .expect("incoming");
        assert!(calls.is_empty(), "nothing calls a script top level");
    }

    #[test]
    fn script_scope_attributes_an_anonymous_top_level_call() {
        // The anonymous function owns no item and sits at top level, so its call
        // belongs to the script scope.
        let src = "foo <- function() 1\nlapply(xs, function(x) foo(x))\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.kind, LspSymbolKind::FILE);
    }

    // --- ambiguous cross-file callees ---------------------------------------

    #[test]
    fn outgoing_reports_every_visible_definition_of_an_ambiguous_callee() {
        // c.R sources both a.R and b.R, and each defines `foo`. Which one wins is
        // not statically decidable, so report both rather than silently picking.
        let snapshot = rename_workspace_files(&[
            ("a.R", "foo <- function() 1\n"),
            ("b.R", "foo <- function() 2\n"),
            (
                "c.R",
                "source(\"a.R\")\nsource(\"b.R\")\nbar <- function() foo()\n",
            ),
        ]);
        let calls = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("c.R"), "bar"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        let mut uris: Vec<String> = calls.iter().map(|c| c.to.uri.to_string()).collect();
        uris.sort();
        assert_eq!(calls.len(), 2, "both visible definitions are reported");
        assert!(uris[0].ends_with("a.R"), "got {uris:?}");
        assert!(uris[1].ends_with("b.R"), "got {uris:?}");
        assert!(calls.iter().all(|c| c.to.name == "foo"));
        assert!(
            calls.iter().all(|c| c.from_ranges.len() == 1),
            "one call site, reported under each candidate"
        );
    }

    #[test]
    fn incoming_attributes_a_call_to_the_innermost_named_function() {
        let src = "foo <- function() 1\nouter <- function() {\n  inner <- function() foo()\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "inner");
        assert_eq!(calls[0].from.detail.as_deref(), Some("outer"));
        assert_eq!(path_of(&calls[0].from), ["outer", "inner"]);
    }

    #[test]
    fn incoming_attributes_an_anonymous_function_call_to_the_enclosing_named_function() {
        // The anonymous function owns no item, so its call belongs to `outer`.
        let src = "foo <- function() 1\nouter <- function() lapply(xs, function(x) foo(x))\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &ws_path("a.R"), "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "outer");
        assert_eq!(path_of(&calls[0].from), ["outer"]);
    }

    #[test]
    fn incoming_distinguishes_same_named_nested_functions() {
        let src = "outer1 <- function() {\n  inner <- function() 1\n  inner()\n}\n\
                   outer2 <- function() {\n  inner <- function() 2\n  inner()\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let first = incoming_calls_via_db(
            &snapshot,
            &nested_item(&snapshot, &a, &["outer1", "inner"]),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].from.name, "outer1");
        assert_eq!(first[0].from_ranges.len(), 1);

        let second = incoming_calls_via_db(
            &snapshot,
            &nested_item(&snapshot, &a, &["outer2", "inner"]),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].from.name, "outer2");
        assert_eq!(second[0].from_ranges.len(), 2);
    }

    #[test]
    fn incoming_finds_callers_of_a_nested_function() {
        let src = "outer <- function() {\n  inner <- function() 1\n  inner()\n  inner()\n}\n";
        let snapshot = rename_workspace(src, "");
        let calls = incoming_calls_via_db(
            &snapshot,
            &nested_item(&snapshot, &ws_path("a.R"), &["outer", "inner"]),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "outer");
        assert_eq!(calls[0].from_ranges.len(), 2);
    }

    #[test]
    fn incoming_for_a_nested_function_is_intra_file() {
        // b.R has its own top-level `inner` with a caller; a nested name is
        // file-private, so it must never pick that up.
        let a_src = "outer <- function() {\n  inner <- function() 1\n  inner()\n}\n";
        let b_src = "inner <- function() 2\ncaller <- function() inner()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let a = ws_path("a.R");
        let calls = incoming_calls_via_db(
            &snapshot,
            &nested_item(&snapshot, &a, &["outer", "inner"]),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.uri, uri::from_path(&a).unwrap());
        assert_eq!(calls[0].from.name, "outer");
    }

    #[test]
    fn super_assigned_function_is_a_file_scope_item() {
        // `<<-` binds at file scope, so `helper`'s path is ["helper"] despite
        // being written inside `outer`.
        let src =
            "outer <- function() {\n  helper <<- function() 1\n}\nmain <- function() helper()\n";
        let snapshot = rename_workspace(src, "");
        let a = ws_path("a.R");
        let item = item_named(&snapshot, &a, "helper");
        assert_eq!(path_of(&item), ["helper"]);
        assert_eq!(item.detail, None);

        let calls =
            incoming_calls_via_db(&snapshot, &item, PositionEncoding::Utf16).expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "main");

        let outgoing = outgoing_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a, "outer"),
            PositionEncoding::Utf16,
        )
        .expect("outgoing");
        assert!(
            outgoing.is_empty(),
            "outer defines helper, it does not call it"
        );
    }

    #[test]
    fn package_sibling_incoming_attributes_to_a_nested_caller() {
        // A real on-disk package: a.R and b.R share one flat namespace, so this
        // exercises the cohort/reader path *and* nested attribution together.
        let a_src = "foo <- function() 1\n";
        let b_src = "outer <- function() {\n  inner <- function() foo()\n  inner()\n}\n";
        let (_dir, snapshot, a_path, b_path) = rename_package(a_src, b_src);
        let calls = incoming_calls_via_db(
            &snapshot,
            &item_named(&snapshot, &a_path, "foo"),
            PositionEncoding::Utf16,
        )
        .expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "inner");
        assert_eq!(calls[0].from.detail.as_deref(), Some("outer"));
        assert_eq!(calls[0].from.uri, uri::from_path(&b_path).unwrap());
        assert_eq!(path_of(&calls[0].from), ["outer", "inner"]);
    }

    #[test]
    fn an_item_without_data_falls_back_to_the_file_scope_name() {
        let src = "foo <- function() 1\nbar <- function() foo()\n";
        let snapshot = rename_workspace(src, "");
        let mut item = item_named(&snapshot, &ws_path("a.R"), "foo");
        item.data = None;
        let calls =
            incoming_calls_via_db(&snapshot, &item, PositionEncoding::Utf16).expect("incoming");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].from.name, "bar");
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
