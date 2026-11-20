//! Source declarations and call targets supplied by the project object model.

use super::*;
use crate::project::classes::*;
use std::collections::BTreeMap;

pub(super) struct ObjectItem {
    pub id: OopId,
    pub item: CallHierarchyItem,
    pub selection: TextRange,
    pub body: Option<TextRange>,
}

impl ObjectItem {
    pub(super) fn item_id(&self) -> ItemId {
        ItemId::Object(self.id.clone().into())
    }
}

pub(super) struct Source<'a> {
    pub index: &'a ClassIndex,
    pub path: &'a Path,
    pub root: &'a SyntaxNode,
    pub model: &'a SemanticModel,
    pub sites: &'a ClassSourceSites,
    dispatch: std::cell::OnceCell<ClassCallSites>,
    has_s3_targets: bool,
    has_s4_targets: bool,
    has_s7_targets: bool,
}

impl<'a> Source<'a> {
    pub(super) fn new(
        index: &'a ClassIndex,
        path: &'a Path,
        root: &'a SyntaxNode,
        model: &'a SemanticModel,
        sites: &'a ClassSourceSites,
    ) -> Self {
        Self {
            index,
            path,
            root,
            model,
            sites,
            dispatch: std::cell::OnceCell::new(),
            has_s3_targets: !index.s3.methods.is_empty()
                || index
                    .s3
                    .generics
                    .iter()
                    .any(|generic| matches!(generic.id, S3GenericId::Declaration(_))),
            has_s4_targets: !index.s4.methods.is_empty()
                || index
                    .s4
                    .generics
                    .iter()
                    .any(|generic| generic.declaration.is_some()),
            has_s7_targets: index.oop_files().any(|file| {
                !file.facts.oop.s7.generics.is_empty() || !file.facts.oop.s7.methods.is_empty()
            }),
        }
    }

    pub(super) fn call_sites(&self) -> &ClassCallSites {
        self.dispatch
            .get_or_init(|| ClassCallSites::new(self.model, self.root))
    }

    pub(super) fn items(
        &self,
        uri: &Uri,
        lines: &LineIndex,
        encoding: PositionEncoding,
    ) -> Vec<ObjectItem> {
        let Self {
            index,
            path,
            root,
            model,
            sites,
            ..
        } = *self;
        let Some(file) = index.file(path) else {
            return Vec::new();
        };
        let declarations =
            oop::presentation::declarations(root, model, &file.facts, sites, index, path);
        if declarations
            .iter()
            .all(|declaration| declaration.key.is_none())
        {
            return Vec::new();
        }
        let implementations = method_navigation_sites(model, root);
        declarations
            .into_iter()
            .filter_map(|declaration| {
                let key = declaration.key?;
                if !matches!(
                    declaration.kind,
                    LspSymbolKind::FUNCTION | LspSymbolKind::METHOD
                ) {
                    return None;
                }
                let id = OopId {
                    path: path.into(),
                    key: key.clone(),
                };
                let implementation = if key.kind == OopKind::Member {
                    index.member(&id)?.method.as_ref()?
                } else {
                    &key
                };
                let body = implementations.implementations.get(implementation).copied();
                let selection = declaration.binding.unwrap_or(declaration.selection);
                let detail = match declaration.container {
                    Some(container) => format!("{container} · {}", declaration.detail),
                    None => declaration.detail,
                };
                Some(ObjectItem {
                    item: CallHierarchyItem {
                        name: declaration.name,
                        kind: declaration.kind,
                        tags: None,
                        detail: Some(detail),
                        uri: uri.clone(),
                        range: text_range_to_lsp_range(lines, declaration.range, encoding),
                        selection_range: text_range_to_lsp_range(lines, selection, encoding),
                        data: serde_json::to_value(ItemData {
                            path: Vec::new(),
                            occurrence: 0,
                            script: false,
                            object: Some(id.clone().into()),
                        })
                        .ok(),
                    },
                    id,
                    selection,
                    body,
                })
            })
            .collect()
    }

    pub(super) fn targets(&self, call: &CallExpr, range: TextRange) -> Vec<(ObjectId, EdgeKind)> {
        let Self {
            index,
            path,
            root,
            model,
            sites,
            ..
        } = *self;
        let Some(base) = call.base() else {
            return Vec::new();
        };
        if let Some(binary) = base.as_node().cloned().and_then(BinaryExpr::cast)
            && matches!(binary.op_kind(), Some(SyntaxKind::DOLLAR | SyntaxKind::AT))
        {
            let target = sites
                .oop
                .iter()
                .filter(|site| site.site.range == range)
                .find_map(|site| oop::resolve_member_site(index, path, root, model, site));
            let is_super = binary
                .lhs()
                .and_then(|value| index.class_receiver(path, root, model, &value))
                .is_some_and(|receiver| receiver.access == ClassAccess::Super);
            return target
                .into_iter()
                .map(|id| {
                    (
                        id.into(),
                        if is_super {
                            EdgeKind::Superclass
                        } else {
                            EdgeKind::Direct
                        },
                    )
                })
                .collect();
        }
        if let Some(member) = index.implicit_reference_member(path, root, model, &base)
            && member.kind == MemberKind::Method
        {
            return member
                .declaration
                .into_iter()
                .map(|id| {
                    (
                        id.into(),
                        if call.callee_name().as_deref() == Some("callSuper") {
                            EdgeKind::Superclass
                        } else {
                            EdgeKind::Direct
                        },
                    )
                })
                .collect();
        }

        let mut targets: Vec<_> = sites
            .oop
            .iter()
            .filter(|site| site.site.range == range && site.site.role == OopSiteRole::Use)
            .filter_map(|site| oop::resolve_method_site(index, path, site))
            .map(|id| (id.into(), EdgeKind::Direct))
            .collect();
        if self.has_s3_targets
            && let Some((dispatch, _)) = index.s3_dispatch_with_sites(
                path,
                root,
                model,
                &crate::syntax::SyntaxElement::Node(call.syntax().clone()),
                self.call_sites(),
            )
        {
            targets.extend(
                s3_methods(&dispatch)
                    .map(|method| (method.source.clone().into(), EdgeKind::Dispatch)),
            );
            if let S3GenericId::Declaration(id) = dispatch.generic
                && !dispatch
                    .uncertainty
                    .contains(&S3DispatchUncertainty::AmbiguousGeneric)
            {
                targets.push((id.into(), EdgeKind::Direct));
            }
        }
        if self.has_s7_targets
            && let Some((dispatch, via_super, internal)) =
                index.s7_dispatch_with_sites(path, root, model, call, self.call_sites())
        {
            if !internal {
                targets.push((dispatch.generic.into(), EdgeKind::Direct));
            }
            let kind = if via_super {
                EdgeKind::Superclass
            } else {
                EdgeKind::Dispatch
            };
            targets.extend(
                dispatch
                    .selected
                    .map_or(dispatch.candidates, |id| vec![id])
                    .into_iter()
                    .map(|id| (id.into(), kind)),
            );
        }
        if self.has_s7_targets
            && !targets.iter().any(|(id, _)| {
                matches!(
                    id, ObjectId::Declaration(id) if id.key.system == OopSystem::S7
                )
            })
        {
            targets.extend(
                index
                    .s7_external_call_targets(path, root, model, call, self.call_sites())
                    .into_iter()
                    .map(|id| (id.into(), EdgeKind::Dispatch)),
            );
        }
        if self.has_s4_targets
            && let Some((generic, methods)) =
                index.s4_call_targets(path, root, model, call, self.call_sites())
        {
            targets.extend(generic.map(|id| (id.into(), EdgeKind::Direct)));
            targets.extend(
                methods
                    .into_iter()
                    .map(|id| (id.into(), EdgeKind::Dispatch)),
            );
        }
        targets
    }
}

pub(super) fn callee_range(call: &CallExpr) -> Option<TextRange> {
    let base = call.base()?;
    if let Some(binary) = base.as_node().cloned().and_then(BinaryExpr::cast)
        && matches!(
            binary.op_kind(),
            Some(SyntaxKind::DOLLAR | SyntaxKind::AT | SyntaxKind::COLON2 | SyntaxKind::COLON3)
        )
    {
        return Some(binary.rhs()?.text_range());
    }
    Some(base.text_range())
}

pub(super) fn s4_method_target(id: &S4MethodId) -> Option<&OopId> {
    match id {
        S4MethodId::Registration(id) => Some(id),
        S4MethodId::Default(_) => None,
    }
}

pub(super) fn s3_methods(dispatch: &S3Dispatch) -> impl Iterator<Item = &S3MethodCandidate> {
    dispatch
        .selected
        .as_ref()
        .map_or(dispatch.candidates.as_slice(), std::slice::from_ref)
        .iter()
}

pub(super) fn s3_dispatch_origins(
    snapshot: &Analysis,
    index: &ClassIndex,
) -> Vec<(S3DispatchContext, S3Dispatch)> {
    let mut origins = Vec::new();
    for source in index.oop_files() {
        let Some(file) = snapshot.lookup_file(&source.path) else {
            continue;
        };
        let root = snapshot.parsed_tree(file);
        let model = snapshot.semantic_model(file);
        let sites = ClassCallSites::new(model, &root);
        for call in root.descendants().filter_map(CallExpr::cast) {
            let value = crate::syntax::SyntaxElement::Node(call.syntax().clone());
            if let Some((dispatch, context)) =
                index.s3_dispatch_with_sites(&source.path, &root, model, &value, &sites)
                && s3_methods(&dispatch).any(|method| method.class_index.is_some())
            {
                origins.push((context, dispatch));
            }
        }
    }
    origins
}

#[derive(Clone)]
pub(super) struct LinkedBody {
    pub path: PathBuf,
    pub function: TextRange,
    pub selection: TextRange,
    range: TextRange,
    name: String,
    detail: Option<String>,
}

impl LinkedBody {
    pub(super) fn anchor(
        &self,
        snapshot: &Analysis,
        item: &mut CallHierarchyItem,
        encoding: PositionEncoding,
    ) -> Option<()> {
        let file = snapshot.lookup_file(&self.path)?;
        let lines = snapshot.line_index(file);
        item.uri = uri::from_path(&self.path)?;
        item.range = text_range_to_lsp_range(lines, self.range, encoding);
        item.selection_range = text_range_to_lsp_range(lines, self.selection, encoding);
        Some(())
    }

    pub(super) fn item(
        &self,
        snapshot: &Analysis,
        id: &ObjectId,
        encoding: PositionEncoding,
    ) -> Option<CallHierarchyItem> {
        let mut item = CallHierarchyItem {
            name: self.name.clone(),
            kind: LspSymbolKind::METHOD,
            tags: None,
            detail: self.detail.clone(),
            uri: uri::from_path(&self.path)?,
            range: Range::default(),
            selection_range: Range::default(),
            data: serde_json::to_value(ItemData {
                path: Vec::new(),
                occurrence: 0,
                script: false,
                object: Some(id.clone()),
            })
            .ok(),
        };
        self.anchor(snapshot, &mut item, encoding)?;
        Some(item)
    }
}

/// A registration can install a named closure defined elsewhere. Use the
/// callable origin at installation time and recover its current CST spans.
pub(super) fn linked_bodies(
    snapshot: &Analysis,
    index: &ClassIndex,
) -> BTreeMap<ObjectId, LinkedBody> {
    let mut bodies = BTreeMap::new();
    let mut functions = HashMap::new();
    let mut add = |id, origin: (PathBuf, usize), detail| {
        let (path, ordinal) = origin;
        let sites = functions.entry(path.clone()).or_insert_with(|| {
            let file = snapshot.lookup_file(&path)?;
            let root = snapshot.parsed_tree(file);
            let model = snapshot.semantic_model(file);
            let navigation = method_navigation_sites(model, &root);
            Some((root, navigation.callables))
        });
        let Some((root, ranges)) = sites else { return };
        let Some(&function) = ranges.get(&ordinal) else {
            return;
        };
        let Some(assignment) = root
            .covering_element(function)
            .ancestors()
            .find_map(AssignmentExpr::cast)
        else {
            return;
        };
        let Some(token) = assignment.target_name_token() else {
            return;
        };
        bodies.insert(
            id,
            LinkedBody {
                path,
                function,
                selection: token.text_range(),
                range: assignment.syntax().text_range(),
                name: crate::ast::token_name(&token).to_string(),
                detail,
            },
        );
    };
    for method in &index.s3.methods {
        let Some(source) = &method.implementation else {
            continue;
        };
        if method.generics.is_empty() || !method.registered {
            continue;
        }
        match &method.source {
            S3MethodSource::Declaration(id) => {
                if let Some(origin) = index.registration_callable_origin(id, source) {
                    add(id.clone().into(), origin, None);
                }
            }
            S3MethodSource::Namespace { root, .. } => {
                let MethodImplementationSource::Binding { name, .. } = source else {
                    continue;
                };
                if let Some(origin) = index.namespace_callable_origin(root, name) {
                    let class = match &method.class {
                        ClassName::Known(name) => name.as_str(),
                        ClassName::Unknown | ClassName::Absent => continue,
                    };
                    let generic = match &method.generic.name {
                        ClassName::Known(name) => name.as_str(),
                        ClassName::Unknown | ClassName::Absent => continue,
                    };
                    add(
                        method.source.clone().into(),
                        origin,
                        Some(format!("S3 method {generic} for {class}")),
                    );
                }
            }
        }
    }
    for method in &index.s4.methods {
        if let S4MethodId::Registration(id) = &method.id
            && let Some(source) = &method.implementation
            && let Some(origin) = index.s4_registration_callable_origin(id, source)
        {
            add(id.clone().into(), origin, None);
        }
    }
    for file in index.oop_files() {
        for implementation in &file.facts.oop.method_implementations {
            if implementation.method.system != OopSystem::S7 {
                continue;
            }
            let id = OopId {
                path: file.path.clone(),
                key: implementation.method.clone(),
            };
            if index.s7_registration(&id).is_some()
                && let Some(origin) = index.s7_method_callable_origin(&id, &implementation.source)
            {
                add(id.into(), origin, None);
            }
        }
    }
    bodies
}
