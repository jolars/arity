//! Call hierarchy (`textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls`,
//! `callHierarchy/outgoingCalls`).
//!
//! Each edge joins two source items at a callee span. Both directions use the
//! same resolution and ownership rules. Symbolic item identities let a client
//! expand a tree after edits have shifted declaration ranges.
//!
//! A call belongs to its innermost named function or method. Calls in anonymous
//! closures fall through to that enclosing item or to the file's script scope.

use super::*;
use crate::project::classes::{
    ClassIndex, OopId, OopSystem, S3DispatchUncertainty, S3MethodFrame, S3MethodSource, S4CallKind,
    class_source_sites,
};

use std::collections::BTreeMap;

mod objects;

/// A named function definition in one file, at any scope.
#[derive(Debug, Clone)]
struct FnDef {
    /// The named enclosing functions, ending with this function.
    path: Vec<SmolStr>,
    /// Distinguish equal name chains in separate binding scopes, such as two
    /// inline methods that each define a local helper.
    occurrence: usize,
    /// The binding this definition introduces — the join key for resolving a
    /// callee or read back to its item, and the handle for its read sites.
    binding: BindingId,
    /// The defining identifier.
    selection: TextRange,
    /// The whole assignment statement.
    full: TextRange,
}

impl FnDef {
    fn item_id(&self) -> ItemId {
        ItemId::Function(self.path.clone(), self.occurrence)
    }

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

    fn by_path(&self, path: &[SmolStr], occurrence: usize) -> Option<usize> {
        self.defs
            .iter()
            .position(|(d, _)| d.path == path && d.occurrence == occurrence)
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
                occurrence: 0,
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
    let mut cohorts: HashMap<Vec<SmolStr>, Vec<ScopeId>> = HashMap::new();
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
        let binding = defs[idx].0.binding;
        if !model.binding_is_file_scope(binding) {
            let scope = model.binding(binding).scope;
            let scopes = cohorts.entry(chain.clone()).or_default();
            let ordinal = scopes
                .iter()
                .position(|candidate| *candidate == scope)
                .unwrap_or_else(|| {
                    scopes.push(scope);
                    scopes.len() - 1
                });
            // Reserve the bare name with ordinal zero for file scope.
            defs[idx].0.occurrence = ordinal + usize::from(chain.len() == 1);
        }
        defs[idx].0.path = chain;
    }

    FileFunctions { defs, owners }
}

/// What an item denotes: a named function, or a file's **script scope** — the
/// top level, which owns every call written inside no function.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemId {
    /// The enclosing-function name chain, ending with the function's own name.
    Function(Vec<SmolStr>, usize),
    /// The file's top level. Never a *callee* (nothing calls a script), so it
    /// appears only as the `from` of an incoming edge; its own outgoing edges are
    /// the file's top-level calls.
    ScriptScope,
    Object(ObjectId),
}

/// Registration identity stays tied to its declaration even when an item
/// points into a separately sourced implementation file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
enum ObjectId {
    Declaration(OopId),
    Namespace { root: PathBuf, occurrence: usize },
}

impl From<OopId> for ObjectId {
    fn from(id: OopId) -> Self {
        Self::Declaration(id)
    }
}

impl From<S3MethodSource> for ObjectId {
    fn from(source: S3MethodSource) -> Self {
        match source {
            S3MethodSource::Declaration(id) => id.into(),
            S3MethodSource::Namespace { root, occurrence } => Self::Namespace { root, occurrence },
        }
    }
}

impl ObjectId {
    fn is_s3_method(&self) -> bool {
        match self {
            Self::Declaration(id) => {
                id.key.system == OopSystem::S3
                    && id.key.kind == crate::project::classes::OopKind::Method
            }
            Self::Namespace { .. } => true,
        }
    }

    fn origin(&self) -> &Path {
        match self {
            Self::Declaration(id) => &id.path,
            Self::Namespace { root, .. } => root,
        }
    }
}

/// The identity payload round-tripped through [`CallHierarchyItem::data`]: the
/// item's enclosing-function name chain, or the script-scope marker. See the
/// module doc for why identity is symbolic rather than positional.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ItemData {
    path: Vec<SmolStr>,
    #[serde(default, skip_serializing_if = "is_zero")]
    occurrence: usize,
    /// Set only on the script-scope item, whose `path` is empty — a name chain
    /// cannot name the top level. Skipped when false so a function item's payload
    /// stays byte-identical to the pre-script-scope encoding.
    #[serde(default, skip_serializing_if = "is_false")]
    script: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    object: Option<ObjectId>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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
        Some(ItemData {
            object: Some(key), ..
        }) => ItemId::Object(key),
        Some(d) if !d.path.is_empty() => ItemId::Function(d.path, d.occurrence),
        _ => ItemId::Function(vec![SmolStr::new(&item.name)], 0),
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
            occurrence: def.occurrence,
            script: false,
            object: None,
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
            occurrence: 0,
            script: true,
            object: None,
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
    occurrence: usize,
    encoding: PositionEncoding,
) -> Option<CallHierarchyItem> {
    let file = snapshot.lookup_file(path)?;
    let uri = uri::from_path(path)?;
    let root = snapshot.parsed_tree(file);
    let model = snapshot.semantic_model(file);
    let functions = function_defs(&root, model);
    let def = functions.def(functions.by_path(fn_path, occurrence)?);
    let line_index = snapshot.line_index(file);
    Some(fn_def_to_item(def, &uri, line_index, encoding))
}

/// Prepare callable items at a declaration or callee in the live editor buffer.
pub(crate) fn prepare_call_hierarchy_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    buffer: &TextBuffer,
    position: Position,
    encoding: PositionEncoding,
    s7_floor: Option<&crate::config::CompatVersion>,
) -> Option<Vec<CallHierarchyItem>> {
    let text = buffer.text();
    let line_index = buffer.line_index();
    let offset = TextSize::new(
        line_index
            .position_to_byte(position, encoding)
            .min(text.len()) as u32,
    );
    let root = parse(text).cst;
    let local = oop::with_live_model(
        &root,
        Some(oop::ReadContext {
            snapshot,
            path,
            s7_floor,
        }),
        |snapshot, model, path| {
            let (facts, sites) = class_source_sites(model, &root);
            let index = snapshot.class_index_for_presentation(path, &facts, &root, model);
            let source = objects::Source::new(&index, path, &root, model, &sites);
            let mut declarations = source.items(uri, line_index, encoding);
            let linked = objects::linked_bodies(snapshot, &index);
            for declaration in &mut declarations {
                if let Some(body) = linked
                    .get(&declaration.id.clone().into())
                    .filter(|body| body.path != path)
                {
                    body.anchor(snapshot, &mut declaration.item, encoding);
                }
            }
            let registered = sites.oop.iter().find_map(|source| {
                source
                    .registration
                    .as_ref()
                    .filter(|_| source.site.range.contains_inclusive(offset))
            });
            let declared: Vec<_> = declarations
                .iter()
                .filter(|declaration| {
                    declaration.selection.contains_inclusive(offset)
                        || registered == Some(&declaration.id.key)
                })
                .map(|declaration| declaration.item.clone())
                .collect();
            if !declared.is_empty() {
                return Some(declared);
            }
            if let Some(token) = pick_name_token(&root, offset)
                && let Some(ident) = model
                    .idents()
                    .iter()
                    .find(|ident| ident.range == token.text_range())
                && let Some(binding) = model.resolve_local(ident)
                && let Some(item) = declarations
                    .iter()
                    .find(|item| item.selection == model.binding(binding).def_range)
            {
                return Some(vec![item.item.clone()]);
            }
            let mut graph = Hierarchy::new(snapshot, encoding);
            let tracked = snapshot.tracked_class_source(path, &root, model).is_some();
            let mut items = linked
                .iter()
                .filter(|(id, body)| {
                    tracked
                        && matches!(id, ObjectId::Namespace { .. })
                        && body.path == path
                        && body.selection.contains_inclusive(offset)
                })
                .filter_map(|(id, body)| body.item(snapshot, id, encoding))
                .collect::<Vec<_>>();
            let mut possible = Vec::new();
            for call in root.descendants().filter_map(CallExpr::cast) {
                let Some(range) =
                    objects::callee_range(&call).filter(|range| range.contains_inclusive(offset))
                else {
                    continue;
                };
                for (id, kind) in source.targets(&call, range) {
                    let item = if id.origin() == path {
                        declarations
                            .iter()
                            .find(|item| item.item_id() == ItemId::Object(id.clone()))
                            .map(|item| item.item.clone())
                    } else {
                        graph.item(id.origin(), &ItemId::Object(id.clone()))
                    };
                    if let Some(mut item) = item {
                        let group = if kind == EdgeKind::Dispatch {
                            &mut possible
                        } else {
                            &mut items
                        };
                        kind.describe(&mut item);
                        if !group.contains(&item) {
                            group.push(item);
                        }
                    }
                }
            }
            if !items.is_empty() {
                Some(items)
            } else {
                let lexical = prepare_local(&root, model, offset, uri, line_index, encoding);
                if possible.is_empty() || lexical.as_ref().is_some_and(|items| !items.is_empty()) {
                    lexical
                } else {
                    Some(possible)
                }
            }
        },
    );
    if local.is_some() {
        return local;
    }

    // Bare lexical names can also resolve through workspace definitions.
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
        let mut graph = Hierarchy::new(snapshot, encoding);
        snapshot
            .workspace_def_sites(&name)
            .into_iter()
            .filter(|(def_path, _)| def_path != path)
            .filter_map(|(def_path, _)| {
                let id = graph.canonical_id(&def_path, ItemId::Function(fn_path.to_vec(), 0));
                graph.item(&def_path, &id)
            })
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

pub(crate) fn incoming_calls_via_db(
    snapshot: &Analysis,
    item: &CallHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    let path = uri::to_path(&item.uri)?;
    let target = item_id(item);
    if target == ItemId::ScriptScope {
        return Some(Vec::new());
    }
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let mut graph = Hierarchy::new(snapshot, encoding);
        let target = graph.canonical_id(&path, target);
        graph.item(&path, &target)?;
        let target_path = match &target {
            ItemId::Object(id) => id.origin(),
            _ => &path,
        };
        let mut files = match &target {
            ItemId::Function(name, 0) if name.len() == 1 => {
                let binding = snapshot.cross_file_binding(&path, &name[0]);
                binding.cohort.into_iter().chain(binding.readers).collect()
            }
            ItemId::Function(..) => Vec::new(),
            _ => graph
                .index
                .oop_files()
                .map(|file| file.path.clone())
                .collect(),
        };
        files.push(path.clone());
        files.sort();
        files.dedup();
        let mut groups: Vec<(EdgeKind, CallHierarchyIncomingCall)> = Vec::new();
        for file in files {
            for edge in graph.edges(&file, None) {
                if edge.path != target_path || edge.to != target {
                    continue;
                }
                let Some(mut from) = graph.item(&file, &edge.from) else {
                    continue;
                };
                edge.kind.describe(&mut from);
                if let Some((_, group)) = groups.iter_mut().find(|(kind, group)| {
                    *kind == edge.kind
                        && group.from.uri == from.uri
                        && item_id(&group.from) == edge.from
                }) {
                    if !group.from_ranges.contains(&edge.range) {
                        group.from_ranges.push(edge.range);
                    }
                } else {
                    groups.push((
                        edge.kind,
                        CallHierarchyIncomingCall {
                            from,
                            from_ranges: vec![edge.range],
                        },
                    ));
                }
            }
        }
        Some(groups.into_iter().map(|(_, group)| group).collect())
    }))
    .ok()
    .flatten()
}

pub(crate) fn outgoing_calls_via_db(
    snapshot: &Analysis,
    item: &CallHierarchyItem,
    encoding: PositionEncoding,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let path = uri::to_path(&item.uri)?;
    let source = item_id(item);
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let mut graph = Hierarchy::new(snapshot, encoding);
        let source = graph.canonical_id(&path, source);
        let caller = graph.item(&path, &source)?;
        // An item can survive a moved registration. Its old URI cannot
        // describe call sites if the implementation moved to another file.
        if caller.uri != item.uri {
            return Some(Vec::new());
        }
        let mut groups: Vec<(EdgeKind, CallHierarchyOutgoingCall)> = Vec::new();
        for edge in graph.edges(&path, Some(&source)) {
            if edge.from != source {
                continue;
            }
            let Some(mut to) = graph.item(&edge.path, &edge.to) else {
                continue;
            };
            edge.kind.describe(&mut to);
            if let Some((_, group)) = groups.iter_mut().find(|(kind, group)| {
                *kind == edge.kind && group.to.uri == to.uri && item_id(&group.to) == edge.to
            }) {
                if !group.from_ranges.contains(&edge.range) {
                    group.from_ranges.push(edge.range);
                }
            } else {
                groups.push((
                    edge.kind,
                    CallHierarchyOutgoingCall {
                        to,
                        from_ranges: vec![edge.range],
                    },
                ));
            }
        }
        Some(groups.into_iter().map(|(_, group)| group).collect())
    }))
    .ok()
    .flatten()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    Direct,
    Dispatch,
    Superclass,
}

impl EdgeKind {
    fn describe(self, item: &mut CallHierarchyItem) {
        let label = match self {
            Self::Direct => return,
            Self::Dispatch => "possible dispatch target",
            Self::Superclass => "superclass call",
        };
        item.detail = Some(match item.detail.take() {
            Some(detail) => format!("{detail} · {label}"),
            None => label.into(),
        });
    }
}

struct Edge {
    from: ItemId,
    path: PathBuf,
    to: ItemId,
    kind: EdgeKind,
    range: Range,
}

type S3SuperTargets = HashMap<(ObjectId, TextRange), Vec<ObjectId>>;

struct Hierarchy<'a> {
    snapshot: &'a Analysis,
    encoding: PositionEncoding,
    index: &'a ClassIndex,
    files: HashMap<PathBuf, Vec<objects::ObjectItem>>,
    s3_super_targets: Option<S3SuperTargets>,
    linked_bodies: Option<BTreeMap<ObjectId, objects::LinkedBody>>,
}

impl<'a> Hierarchy<'a> {
    fn new(snapshot: &'a Analysis, encoding: PositionEncoding) -> Self {
        Self {
            snapshot,
            encoding,
            index: snapshot.class_index_ref(),
            files: HashMap::new(),
            s3_super_targets: None,
            linked_bodies: None,
        }
    }

    fn linked_bodies(&mut self) -> &BTreeMap<ObjectId, objects::LinkedBody> {
        self.linked_bodies
            .get_or_insert_with(|| objects::linked_bodies(self.snapshot, self.index))
    }

    fn objects(&mut self, path: &Path) -> Option<&[objects::ObjectItem]> {
        let file = self.snapshot.lookup_file(path)?;
        let uri = uri::from_path(path)?;
        Some(self.files.entry(path.into()).or_insert_with(|| {
            let root = self.snapshot.parsed_tree(file);
            let model = self.snapshot.semantic_model(file);
            objects::Source::new(
                self.index,
                path,
                &root,
                model,
                self.snapshot.class_source_sites(file),
            )
            .items(&uri, self.snapshot.line_index(file), self.encoding)
        }))
    }

    fn canonical_id(&mut self, path: &Path, id: ItemId) -> ItemId {
        let ItemId::Function(name, occurrence) = &id else {
            return id;
        };
        let Some(item) = function_item(self.snapshot, path, name, *occurrence, self.encoding)
        else {
            return id;
        };
        self.objects(path)
            .unwrap_or_default()
            .iter()
            .find(|object| object.item.selection_range == item.selection_range)
            .map_or(id, objects::ObjectItem::item_id)
    }

    fn item(&mut self, path: &Path, id: &ItemId) -> Option<CallHierarchyItem> {
        match id {
            ItemId::Function(name, occurrence) => {
                function_item(self.snapshot, path, name, *occurrence, self.encoding)
            }
            ItemId::Object(id) => {
                let linked = self.linked_bodies().get(id).cloned();
                match id {
                    ObjectId::Declaration(id) => {
                        let mut item = self
                            .objects(&id.path)?
                            .iter()
                            .find(|item| item.id == *id)?
                            .item
                            .clone();
                        if let Some(body) = linked.filter(|body| body.path != id.path) {
                            body.anchor(self.snapshot, &mut item, self.encoding)?;
                        }
                        Some(item)
                    }
                    ObjectId::Namespace { .. } => linked?.item(self.snapshot, id, self.encoding),
                }
            }
            ItemId::ScriptScope => {
                let file = self.snapshot.lookup_file(path)?;
                Some(script_item(
                    path,
                    &uri::from_path(path)?,
                    &self.snapshot.parsed_tree(file),
                    self.snapshot.line_index(file),
                    self.encoding,
                ))
            }
        }
    }

    fn edges(&mut self, path: &Path, only_caller: Option<&ItemId>) -> Vec<Edge> {
        let Some(file) = self.snapshot.lookup_file(path) else {
            return Vec::new();
        };
        let root = self.snapshot.parsed_tree(file);
        let model = self.snapshot.semantic_model(file);
        let sites = self.snapshot.class_source_sites(file);
        let lines = self.snapshot.line_index(file);
        let functions = function_defs(&root, model);
        let mut owners: HashMap<_, Vec<_>> = HashMap::new();
        for (scope, idx) in &functions.owners {
            owners.insert(*scope, vec![functions.def(*idx).item_id()]);
        }
        self.objects(path);
        self.linked_bodies();
        let linked = self
            .linked_bodies
            .as_ref()
            .expect("linked bodies initialized");
        for item in self.files.get(path).into_iter().flatten() {
            let id = item.item_id();
            if linked.contains_key(&item.id.clone().into()) {
                continue;
            }
            let Some(body) = item.body.or_else(|| {
                functions
                    .by_selection(item.selection)
                    .map(|idx| functions.defs[idx].1.syntax().text_range())
            }) else {
                continue;
            };
            let scope = model.innermost_scope_at(body.start());
            // Scope ranges are the exact FUNCTION_EXPR spans.
            if model.scope(scope).range != body {
                continue;
            }
            let items = owners.entry(scope).or_default();
            if let Some(idx) = functions.by_selection(item.selection) {
                let ordinary = functions.def(idx).item_id();
                items.retain(|id| *id != ordinary);
            }
            if !items.contains(&id) {
                items.push(id);
            }
        }

        for (id, body) in linked.iter().filter(|(_, body)| body.path == path) {
            let scope = model.innermost_scope_at(body.function.start());
            if model.scope(scope).range == body.function {
                let owners = owners.entry(scope).or_default();
                let item = ItemId::Object(id.clone());
                if !owners.contains(&item) {
                    owners.push(item);
                }
            }
        }
        let (facts, _) = self
            .snapshot
            .tracked_class_source(path, &root, model)
            .expect("tracked tree and semantic model");
        let index = self
            .snapshot
            .class_index_for_presentation(path, facts, &root, model);
        let source = objects::Source::new(&index, path, &root, model, sites);
        let next_methods: HashMap<_, _> = if index.s4.methods.is_empty() {
            HashMap::new()
        } else {
            index
                .s4_dispatch_calls_with_sites(path, &root, model, source.call_sites())
                .into_iter()
                .filter(|(call, _)| call.kind == S4CallKind::NextMethod)
                .map(|(call, targets)| (call.range, targets))
                .collect()
        };
        let mut edges = Vec::new();
        for call in root.descendants().filter_map(CallExpr::cast) {
            let Some(range) = objects::callee_range(&call) else {
                continue;
            };
            let mut scope = Some(model.innermost_scope_at(range.start()));
            let mut callers = None;
            while let Some(id) = scope {
                if let Some(items) = owners.get(&id) {
                    callers = Some(items.as_slice());
                    break;
                }
                scope = model.scope(id).parent;
            }
            let callers = callers.unwrap_or(&[ItemId::ScriptScope]);
            if only_caller.is_some_and(|id| !callers.contains(id)) {
                continue;
            }
            let targets = source.targets(&call, range);
            let mut resolved: Vec<_> = targets
                .into_iter()
                .map(|(id, kind)| (id.origin().to_owned(), ItemId::Object(id), kind))
                .collect();
            if !resolved
                .iter()
                .any(|(_, _, kind)| *kind == EdgeKind::Direct)
                && let Some(callee) = call
                    .base()
                    .and_then(crate::syntax::SyntaxElement::into_token)
                    .filter(|token| token.kind() == SyntaxKind::IDENT)
            {
                resolved.extend(
                    self.lexical_targets(path, model, &functions, &callee)
                        .into_iter()
                        .map(|(path, id)| (path, id, EdgeKind::Direct)),
                );
            }
            let from_range = text_range_to_lsp_range(lines, range, self.encoding);
            if let Some(methods) = next_methods.get(&call.syntax().text_range()) {
                for (from, to) in methods.iter().filter_map(|(from, to)| {
                    Some((
                        objects::s4_method_target(from)?,
                        objects::s4_method_target(to)?,
                    ))
                }) {
                    let from = ItemId::Object(from.clone().into());
                    if callers.contains(&from) {
                        edges.push(Edge {
                            from,
                            path: to.path.clone(),
                            to: ItemId::Object(to.clone().into()),
                            kind: EdgeKind::Superclass,
                            range: from_range,
                        });
                    }
                }
            }
            let frame = S3MethodFrame {
                path,
                root: &root,
                model,
            };
            if callers
                .iter()
                .any(|id| matches!(id, ItemId::Object(id) if id.is_s3_method()))
                && self.index.is_s3_next_method_call(frame, &call)
            {
                for from in callers {
                    let ItemId::Object(id) = from else { continue };
                    let site = (id.clone(), call.syntax().text_range());
                    for to in self.s3_super_targets().get(&site).into_iter().flatten() {
                        edges.push(Edge {
                            from: from.clone(),
                            path: to.origin().to_owned(),
                            to: ItemId::Object(to.clone()),
                            kind: EdgeKind::Superclass,
                            range: from_range,
                        });
                    }
                }
            }

            let mut unique = Vec::new();
            for (target, to, kind) in resolved {
                if unique.contains(&(target.clone(), to.clone(), kind)) {
                    continue;
                }
                unique.push((target.clone(), to.clone(), kind));
                for from in callers {
                    edges.push(Edge {
                        from: from.clone(),
                        path: target.clone(),
                        to: to.clone(),
                        kind,
                        range: from_range,
                    });
                }
            }
        }
        edges
    }

    fn s3_super_targets(&mut self) -> &S3SuperTargets {
        if self.s3_super_targets.is_none() {
            self.s3_super_targets = Some(self.compute_s3_super_targets());
        }
        self.s3_super_targets.as_ref().expect("initialized")
    }

    fn compute_s3_super_targets(&mut self) -> S3SuperTargets {
        let mut targets = S3SuperTargets::new();
        for (context, origin) in objects::s3_dispatch_origins(self.snapshot, self.index) {
            // Continue only from witnessed dispatches. A class suffix on a
            // method name supplies neither the class cursor nor its caller.
            let mut pending = vec![origin];
            let mut visited = Vec::new();
            while let Some(dispatch) = pending.pop() {
                for method in objects::s3_methods(&dispatch) {
                    let visit = (dispatch.generic.clone(), method.clone());
                    if visited.contains(&visit) {
                        continue;
                    }
                    visited.push(visit);
                    let from = ObjectId::from(method.source.clone());
                    let Some((path, body)) = self.object_body(&from) else {
                        continue;
                    };
                    let Some(file) = self.snapshot.lookup_file(&path) else {
                        continue;
                    };
                    let root = self.snapshot.parsed_tree(file);
                    let model = self.snapshot.semantic_model(file);
                    let frame = S3MethodFrame {
                        path: &path,
                        root: &root,
                        model,
                    };
                    for call in root
                        .covering_element(body)
                        .into_node()
                        .into_iter()
                        .flat_map(|node| node.descendants())
                        .filter_map(CallExpr::cast)
                    {
                        if !self.index.is_s3_next_method_call(frame, &call) {
                            continue;
                        }
                        let next = self
                            .index
                            .s3_next_method_at(&context, &dispatch, method, frame, &call);
                        if next
                            .uncertainty
                            .contains(&S3DispatchUncertainty::UnknownContinuation)
                        {
                            continue;
                        }
                        let site = targets
                            .entry((from.clone(), call.syntax().text_range()))
                            .or_default();
                        for target in objects::s3_methods(&next) {
                            let id = ObjectId::from(target.source.clone());
                            if !site.contains(&id) {
                                site.push(id);
                            }
                        }
                        pending.push(next);
                    }
                }
            }
        }
        targets
    }

    fn object_body(&mut self, id: &ObjectId) -> Option<(PathBuf, TextRange)> {
        if let Some(body) = self.linked_bodies().get(id) {
            return Some((body.path.clone(), body.function));
        }
        let ObjectId::Declaration(id) = id else {
            return None;
        };
        let body = self
            .objects(&id.path)?
            .iter()
            .find(|item| item.id == *id)?
            .body?;
        Some((id.path.clone(), body))
    }

    fn lexical_targets(
        &mut self,
        path: &Path,
        model: &SemanticModel,
        functions: &FileFunctions,
        callee: &SyntaxToken<RLanguage>,
    ) -> Vec<(PathBuf, ItemId)> {
        let range = callee.text_range();
        if let Some(ident) = model.idents().iter().find(|ident| ident.range == range)
            && let Some(binding) = model.resolve_local(ident)
        {
            return functions
                .by_binding(binding)
                .map(|idx| {
                    let def = functions.def(idx);
                    let item = self
                        .objects(path)
                        .unwrap_or_default()
                        .iter()
                        .find(|item| item.selection == def.selection)
                        .map_or_else(|| def.item_id(), objects::ObjectItem::item_id);
                    (path.to_path_buf(), item)
                })
                .into_iter()
                .collect();
        }
        let name = SmolStr::new(callee.text());
        self.snapshot
            .visible_def_files(path, &name)
            .into_iter()
            .filter_map(|file| {
                let ordinary = function_item(
                    self.snapshot,
                    &file,
                    std::slice::from_ref(&name),
                    0,
                    self.encoding,
                )?;
                let item = self
                    .objects(&file)
                    .unwrap_or_default()
                    .iter()
                    .find(|item| item.item.selection_range == ordinary.selection_range)
                    .map_or_else(
                        || ItemId::Function(vec![name.clone()], 0),
                        objects::ObjectItem::item_id,
                    );
                Some((file, item))
            })
            .collect()
    }
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
            None,
        )
        .unwrap_or_default()
    }

    fn item_named(snapshot: &Analysis, path: &Path, name: &str) -> CallHierarchyItem {
        nested_item(snapshot, path, &[name])
    }

    /// The item for the function `fn_path` names in `path`, off the db snapshot.
    fn nested_item(snapshot: &Analysis, path: &Path, fn_path: &[&str]) -> CallHierarchyItem {
        let fn_path: Vec<SmolStr> = fn_path.iter().copied().map(SmolStr::new).collect();
        function_item(snapshot, path, &fn_path, 0, PositionEncoding::Utf16).expect("function item")
    }

    /// The name chain an item carries in its `data` payload — its identity.
    fn path_of(item: &CallHierarchyItem) -> Vec<String> {
        match item_id(item) {
            ItemId::Function(path, _) => path.iter().map(SmolStr::to_string).collect(),
            ItemId::Object(_) => panic!("expected an ordinary function"),
            ItemId::ScriptScope => panic!("expected a function item, got the script scope"),
        }
    }

    #[test]
    fn r6_super_calls_connect_method_items_in_both_directions() {
        let parent = "Parent <- R6::R6Class('Parent', public = list(run = function() 1))";
        let child = "source('a.R')\nChild <- R6::R6Class('Child', inherit = Parent, public = list(run = function() super$run()))";
        let snapshot = rename_workspace(parent, child);
        let encoding = PositionEncoding::Utf16;
        let parent = prepare_at(
            &snapshot,
            &ws_path("a.R"),
            parent,
            parent.find("run").unwrap(),
        );
        let child = prepare_at(
            &snapshot,
            &ws_path("b.R"),
            child,
            child.find("run").unwrap(),
        );
        assert_eq!(parent[0].kind, LspSymbolKind::METHOD);
        assert_eq!(child[0].kind, LspSymbolKind::METHOD);
        let outgoing = outgoing_calls_via_db(&snapshot, &child[0], encoding).unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].to.uri, parent[0].uri);
        assert_eq!(outgoing[0].to.selection_range, parent[0].selection_range);
        assert!(
            outgoing[0]
                .to
                .detail
                .as_deref()
                .unwrap()
                .contains("superclass")
        );
        let incoming = incoming_calls_via_db(&snapshot, &parent[0], encoding).unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].from.selection_range, child[0].selection_range);
        assert_eq!(incoming[0].from_ranges, outgoing[0].from_ranges);
    }

    #[test]
    fn s3_generic_and_registered_dispatch_have_separate_edges() {
        let src = r#"g <- function(x) UseMethod("g")
helper <- function() 1
base::registerS3method("g", "example", function(x) helper())
caller <- function() g(structure(1, class = "example"))
"#;
        let snapshot = rename_workspace(src, "");
        let path = ws_path("a.R");
        let caller = item_named(&snapshot, &path, "caller");
        let encoding = PositionEncoding::Utf16;
        let outgoing = outgoing_calls_via_db(&snapshot, &caller, encoding).unwrap();
        assert_eq!(outgoing.len(), 2, "{outgoing:?}");
        let generic = outgoing
            .iter()
            .find(|edge| edge.to.kind == LspSymbolKind::FUNCTION)
            .unwrap();
        let method = outgoing
            .iter()
            .find(|edge| edge.to.kind == LspSymbolKind::METHOD)
            .unwrap();
        assert!(
            !generic
                .to
                .detail
                .as_deref()
                .unwrap()
                .contains("dispatch target")
        );
        assert!(
            method
                .to
                .detail
                .as_deref()
                .unwrap()
                .contains("possible dispatch target")
        );
        assert_eq!(generic.from_ranges, method.from_ranges);
        for edge in &outgoing {
            let incoming = incoming_calls_via_db(&snapshot, &edge.to, encoding).unwrap();
            assert_eq!(incoming.len(), 1, "{incoming:?}");
            assert_eq!(incoming[0].from.selection_range, caller.selection_range);
            assert_eq!(incoming[0].from_ranges, edge.from_ranges);
        }
        let outgoing = outgoing_calls_via_db(&snapshot, &method.to, encoding).unwrap();
        assert_eq!(outgoing.len(), 1, "{outgoing:?}");
        assert_eq!(outgoing[0].to.name, "helper");
        let incoming = incoming_calls_via_db(&snapshot, &outgoing[0].to, encoding).unwrap();
        assert_eq!(incoming.len(), 1, "{incoming:?}");
        assert_eq!(incoming[0].from.selection_range, method.to.selection_range);
    }

    #[test]
    fn s4_next_method_uses_the_resolved_parent_registration() {
        let src = r#"methods::setClass("Parent")
methods::setClass("Child", contains = "Parent")
methods::setGeneric("g", function(x) standardGeneric("g"))
methods::setMethod("g", "Parent", function(x) 1)
methods::setMethod("g", "Child", function(x) methods::callNextMethod())
g(methods::new("Child"))
"#;
        let snapshot = rename_workspace(src, "");
        let path = ws_path("a.R");
        let parent = &prepare_at(&snapshot, &path, src, src.find("g\", \"Parent").unwrap())[0];
        let child = &prepare_at(&snapshot, &path, src, src.find("g\", \"Child").unwrap())[0];
        let encoding = PositionEncoding::Utf16;
        let outgoing = outgoing_calls_via_db(&snapshot, child, encoding).unwrap();
        assert_eq!(outgoing.len(), 1, "{outgoing:?}");
        assert_eq!(outgoing[0].to.selection_range, parent.selection_range);
        assert!(
            outgoing[0]
                .to
                .detail
                .as_deref()
                .unwrap()
                .contains("superclass call")
        );
        let incoming = incoming_calls_via_db(&snapshot, parent, encoding).unwrap();
        assert_eq!(incoming.len(), 1, "{incoming:?}");
        assert_eq!(incoming[0].from.selection_range, child.selection_range);
        assert_eq!(incoming[0].from_ranges, outgoing[0].from_ranges);
    }

    #[test]
    fn s3_next_method_uses_the_observed_class_vector() {
        let src = r#"g <- function(x) UseMethod("g")
g.child <- function(x) NextMethod()
g.parent <- function(x) NextMethod()
g.root <- function(x) 1
g.other <- function(x) 2
g(structure(1, class = c("child", "parent", "root")))
"#;
        let snapshot = rename_workspace(src, "");
        let path = ws_path("a.R");
        let child = &prepare_at(&snapshot, &path, src, src.find("g.child").unwrap())[0];
        let parent = &prepare_at(&snapshot, &path, src, src.find("g.parent").unwrap())[0];
        let encoding = PositionEncoding::Utf16;
        let outgoing = outgoing_calls_via_db(&snapshot, child, encoding).unwrap();
        assert_eq!(outgoing.len(), 1, "{outgoing:?}");
        assert_eq!(outgoing[0].to.selection_range, parent.selection_range);
        assert!(
            outgoing[0]
                .to
                .detail
                .as_deref()
                .unwrap()
                .contains("superclass call")
        );
        let incoming = incoming_calls_via_db(&snapshot, parent, encoding).unwrap();
        assert_eq!(incoming.len(), 1, "{incoming:?}");
        assert_eq!(incoming[0].from.selection_range, child.selection_range);
        assert_eq!(incoming[0].from_ranges, outgoing[0].from_ranges);
        let grandparent = &prepare_at(&snapshot, &path, src, src.find("g.root").unwrap())[0];
        let outgoing = outgoing_calls_via_db(&snapshot, parent, encoding).unwrap();
        assert_eq!(outgoing.len(), 1, "{outgoing:?}");
        assert_eq!(outgoing[0].to.selection_range, grandparent.selection_range);
        let incoming = incoming_calls_via_db(&snapshot, grandparent, encoding).unwrap();
        assert_eq!(incoming[0].from.selection_range, parent.selection_range);
        assert_eq!(incoming[0].from_ranges, outgoing[0].from_ranges);
    }

    #[test]
    fn reference_class_methods_keep_direct_and_superclass_calls_distinct() {
        let src = r#"Parent <- methods::setRefClass("Parent", methods = list(run = function() 1))
Child <- methods::setRefClass("Child", contains = "Parent", methods = list(
  run = function() callSuper(),
  again = function() run()
))
caller <- function() Child()$run()
"#;
        let snapshot = rename_workspace(src, "");
        let path = ws_path("a.R");
        let parent = &prepare_at(&snapshot, &path, src, src.find("run =").unwrap())[0];
        let child = &prepare_at(&snapshot, &path, src, src.rfind("run =").unwrap())[0];
        let again = &prepare_at(&snapshot, &path, src, src.find("again =").unwrap())[0];
        let encoding = PositionEncoding::Utf16;
        let calls = |item| outgoing_calls_via_db(&snapshot, item, encoding).unwrap();
        assert_eq!(calls(child)[0].to.selection_range, parent.selection_range);
        assert!(
            calls(child)[0]
                .to
                .detail
                .as_deref()
                .unwrap()
                .contains("superclass call")
        );
        assert_eq!(calls(again)[0].to.selection_range, child.selection_range);
        let incoming = incoming_calls_via_db(&snapshot, child, encoding).unwrap();
        assert_eq!(incoming.len(), 2, "{incoming:?}");
        assert_eq!(incoming[0].from.selection_range, again.selection_range);
        assert_eq!(incoming[0].from_ranges, calls(again)[0].from_ranges);
        assert_eq!(incoming[1].from.name, "caller");
    }

    #[test]
    fn registered_method_uses_the_sourced_body_file_for_its_calls() {
        let registration = r#"source("a.R")
base::registerS3method("print", "example", implementation)
caller <- function() base::print(structure(1, class = "example"))
"#;
        let body = "helper <- function() 1\nimplementation <- function(x) helper()\n";
        let snapshot = rename_workspace(body, registration);
        let registration_path = ws_path("b.R");
        let body_path = ws_path("a.R");
        let encoding = PositionEncoding::Utf16;
        let caller = item_named(&snapshot, &registration_path, "caller");
        let edges = outgoing_calls_via_db(&snapshot, &caller, encoding).unwrap();
        let method = &edges
            .iter()
            .find(|edge| edge.to.kind == LspSymbolKind::METHOD)
            .unwrap_or_else(|| panic!("{edges:?}"))
            .to;
        assert_eq!(method.uri, uri::from_path(&body_path).unwrap());
        assert_eq!(method.selection_range.start, Position::new(1, 0));
        let prepared = prepare_at(
            &snapshot,
            &registration_path,
            registration,
            registration.find("registerS3method").unwrap(),
        );
        assert_eq!(item_id(&prepared[0]), item_id(method));
        assert_eq!(prepared[0].uri, method.uri);
        let outgoing = outgoing_calls_via_db(&snapshot, method, encoding).unwrap();
        assert_eq!(outgoing.len(), 1, "{outgoing:?}");
        assert_eq!(outgoing[0].from_ranges[0].start, Position::new(1, 30));
        let incoming = incoming_calls_via_db(&snapshot, &outgoing[0].to, encoding).unwrap();
        assert!(
            incoming
                .iter()
                .any(|edge| item_id(&edge.from) == item_id(method)
                    && edge.from_ranges == outgoing[0].from_ranges)
        );
    }

    #[test]
    fn s7_super_dispatch_keeps_the_ordinary_generic_edge() {
        let src = r#"Parent <- S7::new_class("Parent")
Child <- S7::new_class("Child", parent = Parent)
g <- S7::new_generic("g", "x")
S7::method(g, Parent) <- function(x) 1
S7::method(g, Child) <- function(x) 2
caller <- function() g(S7::super(Child(), Parent))
"#;
        let snapshot = rename_workspace(src, "");
        let path = ws_path("a.R");
        let caller = item_named(&snapshot, &path, "caller");
        let encoding = PositionEncoding::Utf16;
        let outgoing = outgoing_calls_via_db(&snapshot, &caller, encoding).unwrap();
        assert_eq!(outgoing.len(), 2, "{outgoing:?}");
        assert_eq!(
            outgoing
                .iter()
                .filter(|edge| edge.to.kind == LspSymbolKind::FUNCTION)
                .count(),
            1
        );
        let method = outgoing
            .iter()
            .find(|edge| edge.to.kind == LspSymbolKind::METHOD)
            .unwrap();
        assert_eq!(method.to.selection_range.start.line, 3);
        assert!(
            method
                .to
                .detail
                .as_deref()
                .unwrap()
                .contains("superclass call")
        );
        let incoming = incoming_calls_via_db(&snapshot, &method.to, encoding).unwrap();
        assert_eq!(incoming[0].from.selection_range, caller.selection_range);
        assert_eq!(incoming[0].from_ranges, method.from_ranges);
    }

    #[test]
    fn namespace_registration_gets_a_method_item_at_its_backing_closure() {
        let source = r#"g <- function(x) UseMethod("g")
handler <- function(x) helper()
helper <- function() 1
caller <- function() g(structure(1, class = "example"))
"#;
        let path = ws_path("R/a.R");
        let root = ws_root();
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![root.clone()]);
        db.upsert_description(&root, "Package: example\nVersion: 1.0\n");
        db.set_package_metadata(vec![crate::project::PackageInfo {
            root,
            namespace: Some("export(g)\nS3method(g, example, handler)\n".into()),
            dir_sources: Default::default(),
            native_routines: Default::default(),
        }]);
        let snapshot = db.snapshot();
        let encoding = PositionEncoding::Utf16;
        let caller = item_named(&snapshot, &path, "caller");
        let outgoing = outgoing_calls_via_db(&snapshot, &caller, encoding).unwrap();
        let edge = outgoing
            .iter()
            .find(|edge| edge.to.kind == LspSymbolKind::METHOD)
            .unwrap();
        assert_eq!(edge.to.name, "handler");
        assert_eq!(edge.to.selection_range.start, Position::new(1, 0));
        let prepared = prepare_at(&snapshot, &path, source, source.find("handler").unwrap());
        assert_eq!(item_id(&prepared[0]), item_id(&edge.to));
        let incoming = incoming_calls_via_db(&snapshot, &edge.to, encoding).unwrap();
        assert_eq!(incoming[0].from.selection_range, caller.selection_range);
        assert_eq!(incoming[0].from_ranges, edge.from_ranges);
        let body = outgoing_calls_via_db(&snapshot, &prepared[0], encoding).unwrap();
        assert_eq!(body[0].to.name, "helper");
    }

    #[test]
    fn method_item_reuses_its_identity_when_declarations_move() {
        let source = r#"A <- R6::R6Class("A", public = list(run = function() helper()))
B <- R6::R6Class("B", public = list(run = function() 0))
helper <- function() 1
caller <- function() A$new()$run()
unrelated <- function() B$new()$run()
"#;
        let original = rename_workspace(source, "");
        let path = ws_path("a.R");
        let method = &prepare_at(&original, &path, source, source.find("run").unwrap())[0];
        let edited = format!("# comment\n{source}");
        // Prepare uses the live source even when the analysis snapshot is stale.
        let live = &prepare_at(&original, &path, &edited, edited.find("run").unwrap())[0];
        assert_eq!(live.selection_range.start.line, 1);
        assert_eq!(item_id(live), item_id(method));
        let updated = rename_workspace(&edited, "");
        let encoding = PositionEncoding::Utf16;
        let incoming = incoming_calls_via_db(&updated, method, encoding).unwrap();
        assert_eq!(incoming.len(), 1, "{incoming:?}");
        assert_eq!(incoming[0].from.name, "caller");
        let outgoing = outgoing_calls_via_db(&updated, &incoming[0].from, encoding).unwrap();
        assert_eq!(outgoing[0].to.selection_range, live.selection_range);
        assert_eq!(outgoing[0].from_ranges, incoming[0].from_ranges);
        let outgoing = outgoing_calls_via_db(&updated, method, encoding).unwrap();
        assert_eq!(outgoing[0].to.name, "helper");
        assert_eq!(outgoing[0].from_ranges[0].start.line, 1);
    }

    #[test]
    fn same_named_helpers_in_separate_methods_resolve_by_scope() {
        let source = r#"A <- R6::R6Class("A", public = list(run = function() {
  helper <- function() 1
  helper()
}))
B <- R6::R6Class("B", public = list(run = function() {
  helper <- function() 2
  helper()
}))
"#;
        let snapshot = rename_workspace(source, "");
        let path = ws_path("a.R");
        let method = &prepare_at(&snapshot, &path, source, source.rfind("run").unwrap())[0];
        let encoding = PositionEncoding::Utf16;
        let outgoing = outgoing_calls_via_db(&snapshot, method, encoding).unwrap();
        assert_eq!(outgoing.len(), 1, "{outgoing:?}");
        assert_eq!(outgoing[0].to.selection_range.start.line, 5);
        let incoming = incoming_calls_via_db(&snapshot, &outgoing[0].to, encoding).unwrap();
        assert_eq!(incoming.len(), 1, "{incoming:?}");
        assert_eq!(incoming[0].from.selection_range, method.selection_range);
        assert_eq!(incoming[0].from_ranges, outgoing[0].from_ranges);
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
