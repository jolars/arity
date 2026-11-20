use super::*;
use crate::project::classes::*;
use crate::syntax::SyntaxElement;
use std::collections::BTreeSet;

pub(in crate::lsp) struct Declaration {
    pub key: Option<OopKey>,
    pub name: String,
    pub kind: LspSymbolKind,
    pub detail: String,
    pub range: TextRange,
    pub selection: TextRange,
    pub binding: Option<TextRange>,
    pub container: Option<String>,
    pub top_level: bool,
}

pub(in crate::lsp) fn member_kind(kind: MemberKind) -> LspSymbolKind {
    match kind {
        MemberKind::Method => LspSymbolKind::METHOD,
        MemberKind::Field => LspSymbolKind::FIELD,
        _ => LspSymbolKind::PROPERTY,
    }
}

pub(in crate::lsp) fn signature(sig: &CallableSignature) -> String {
    let args = sig
        .parameters
        .iter()
        .map(|param| match &param.default {
            Some(default) => format!("{} = {default}", param.name),
            None => param.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("({args})")
}

pub(in crate::lsp) fn declarations(
    root: &SyntaxNode,
    model: &SemanticModel,
    facts: &FileClassFacts,
    sites: &ClassSourceSites,
    index: &ClassIndex,
    path: &Path,
) -> Vec<Declaration> {
    let mut result = Vec::new();
    for (class, location) in facts.declarations.iter().zip(&sites.locations) {
        if class.binding.is_none() && !matches!(class.name, ClassName::Known(_)) {
            continue;
        }
        if !index.declarations.contains_key(&ClassId {
            path: path.into(),
            key: class.key.clone(),
        }) {
            continue;
        }
        let name = match class.display_name() {
            "" => class.binding.as_deref().unwrap_or("<anonymous class>"),
            name => name,
        };
        result.push(Declaration {
            key: None,
            name: name.into(),
            kind: LspSymbolKind::CLASS,
            detail: class.system().label().into(),
            range: location.full_range,
            selection: location.name_range,
            binding: location.binding_range,
            container: None,
            top_level: class.key.owner.0.is_empty(),
        });
    }
    for source in &sites.oop {
        let site = &source.site;
        let OopTarget::Declaration(key) = &site.target else {
            continue;
        };
        if site.role != OopSiteRole::Declaration {
            continue;
        }
        let id = OopId {
            path: path.into(),
            key: key.clone(),
        };
        let (name, kind, role, sig, binding, container, registration_member) = match key.kind {
            OopKind::Generic => {
                let Some(generic) = index.generic(&id) else {
                    continue;
                };
                if !index.generic_confirmed(path, generic) {
                    continue;
                }
                (
                    &generic.name,
                    LspSymbolKind::FUNCTION,
                    "generic",
                    generic.signature.as_ref(),
                    generic
                        .binding
                        .as_deref()
                        .filter(|_| key.system != OopSystem::S4),
                    None,
                    false,
                )
            }
            OopKind::Method => {
                let Some(method) = index.method(&id) else {
                    continue;
                };
                if method.class.is_some()
                    || facts
                        .oop
                        .s7
                        .methods
                        .get(key)
                        .is_some_and(|method| method.removal)
                {
                    continue;
                }
                if let Some(registration) = &method.registration {
                    let expected = match method.system {
                        OopSystem::S3 => "base",
                        OopSystem::S4 => "methods",
                        OopSystem::S7 => "S7",
                        _ => continue,
                    };
                    if index
                        .callee_identity(path, registration)
                        .is_none_or(|origin| origin.package != expected)
                    {
                        continue;
                    }
                } else if method.system == OopSystem::S3
                    && super::navigation::resolve_method_site(index, path, source).is_none()
                {
                    continue;
                }
                let binding = match &method.name {
                    ClassName::Known(name) if method.registration.is_none() => Some(name.as_str()),
                    _ => None,
                };
                (
                    &method.name,
                    LspSymbolKind::METHOD,
                    "method",
                    method.signature.as_ref(),
                    binding,
                    None,
                    false,
                )
            }
            OopKind::Member => {
                let Some(member) = facts.oop.members.iter().find(|item| item.key == *key) else {
                    continue;
                };
                if !index.member_declaration_confirmed(path, member) {
                    continue;
                }
                let class = member.class.as_ref().and_then(|key| facts.declaration(key));
                let sig = member
                    .method
                    .as_ref()
                    .and_then(|key| facts.oop.methods.iter().find(|method| method.key == *key))
                    .and_then(|method| method.signature.as_ref());
                let role = match member.kind {
                    MemberKind::Method => "method",
                    MemberKind::Slot => "slot",
                    MemberKind::Property => "property",
                    MemberKind::ActiveBinding => "active binding",
                    _ => "field",
                };
                (
                    &member.name,
                    member_kind(member.kind),
                    role,
                    sig,
                    None,
                    class.map(|class| class.display_name().to_owned()),
                    key.system == OopSystem::R6 && member.class.is_none(),
                )
            }
            _ => continue,
        };
        let ClassName::Known(name) = name else {
            continue;
        };
        let element = root.covering_element(site.range);
        let mut ancestors = element.ancestors();
        let node = if key.kind == OopKind::Member && !registration_member {
            ancestors.find(|node| node.kind() == SyntaxKind::ARG || CallExpr::can_cast(node.kind()))
        } else if binding.is_some() || key.system == OopSystem::S7 && key.kind == OopKind::Method {
            ancestors
                .find(|node| {
                    AssignmentExpr::cast(node.clone()).is_some()
                        || model.s7_bindings.contains(&node.text_range())
                })
                .or_else(|| {
                    element
                        .ancestors()
                        .find(|node| CallExpr::can_cast(node.kind()))
                })
        } else {
            ancestors
                .find(|node| CallExpr::can_cast(node.kind()))
                .map(|call| {
                    if key.system == OopSystem::S4 && key.kind == OopKind::Generic {
                        let value = SyntaxElement::Node(call.clone());
                        if let Some(assignment) = value.parent().and_then(AssignmentExpr::cast)
                            && assignment.value_element().as_ref() == Some(&value)
                        {
                            return assignment.syntax().clone();
                        }
                    }
                    call
                })
        };
        let range = node.as_ref().map_or(site.range, SyntaxNode::text_range);
        let shorthand = node
            .as_ref()
            .filter(|node| model.s7_bindings.contains(&node.text_range()))
            .and_then(crate::semantic::s7::binding_parts)
            .map(|(target, _)| target);
        let binding = binding.and_then(|name| {
            let token = shorthand
                .clone()
                .or_else(|| AssignmentExpr::cast(node?)?.target_name_token())?;
            (crate::semantic::symbols::unbacktick(token.text()) == name).then(|| token.text_range())
        });
        let selection = if key.kind == OopKind::Generic {
            shorthand.map_or(site.range, |target| target.text_range())
        } else {
            site.range
        };
        let mut detail = format!("{:?} {role}", key.system);
        if key.kind == OopKind::Method
            && let Some(method) = index.method(&id)
            && !method.dispatch.is_empty()
        {
            detail.push_str(" for ");
            detail.push_str(
                &dispatch_labels(&method.dispatch, |key| {
                    facts
                        .declaration(key)
                        .map(|class| class.display_name().to_owned())
                })
                .join(", "),
            );
        }
        if let Some(sig) = sig {
            detail.push(' ');
            detail.push_str(&signature(sig));
        }
        result.push(Declaration {
            key: Some(key.clone()),
            name: name.clone(),
            kind,
            detail,
            range,
            selection,
            binding,
            container,
            top_level: key.owner.0.is_empty(),
        });
    }
    result
}

pub(in crate::lsp) fn source_role(
    index: &ClassIndex,
    path: &Path,
    sites: &ClassSourceSites,
    range: TextRange,
) -> Option<(LspSymbolKind, String, String)> {
    let classes = sites
        .classes
        .iter()
        .filter(|(span, _)| *span == range)
        .flat_map(|(_, occurrence)| {
            super::navigation::resolve_class_occurrence(
                index,
                path,
                occurrence,
                sites.class_reference_factories.get(&range),
            )
        });
    let matched = sites.oop.iter().filter(|source| source.site.range == range);
    let classes: BTreeSet<_> =
        classes
            .chain(matched.clone().flat_map(|source| {
                super::navigation::resolve_registration_class(index, path, source)
            }))
            .collect();
    if classes.len() == 1 {
        let class = &index.declarations.get(classes.first()?)?.declaration;
        return Some((
            LspSymbolKind::CLASS,
            class.display_name().into(),
            class.system().label().into(),
        ));
    }
    if let Some((system, class)) = matched.clone().find_map(|source| {
        if let OopTarget::DispatchClass {
            system,
            class: DispatchClass::S3(class),
        } = &source.site.target
            && confirmed_dispatch_class(index, path, source)
        {
            Some((*system, class))
        } else {
            None
        }
    }) {
        return Some((
            LspSymbolKind::CLASS,
            class.clone(),
            format!("{system:?} dispatch class"),
        ));
    }
    let generics: BTreeSet<_> = matched
        .flat_map(|source| super::navigation::resolve_generic_site(index, path, source))
        .collect();
    if generics.len() != 1 {
        return None;
    }
    let generic = index.generic(generics.first()?)?;
    let ClassName::Known(name) = &generic.name else {
        return None;
    };
    let mut detail = format!("{:?} generic", generic.key.system);
    if let Some(sig) = &generic.signature {
        detail.push(' ');
        detail.push_str(&signature(sig));
    }
    Some((LspSymbolKind::FUNCTION, name.clone(), detail))
}

pub(in crate::lsp) fn token_roles(
    snapshot: &Analysis,
    path: &Path,
    root: &SyntaxNode,
    model: &SemanticModel,
) -> HashMap<TextRange, (LspSymbolKind, bool)> {
    let (facts, sites) = class_source_sites(model, root);
    if sites.oop.is_empty() && sites.classes.is_empty() {
        return HashMap::new();
    }
    let index = snapshot.class_index_for_presentation(path, &facts, root, model);
    let mut roles = HashMap::new();
    for decl in declarations(root, model, &facts, &sites, &index, path) {
        roles.insert(decl.selection, (decl.kind, true));
        if let Some(binding) = decl.binding {
            roles.insert(binding, (decl.kind, true));
        }
    }
    for (range, occurrence) in &sites.classes {
        if !super::navigation::resolve_class_occurrence(
            &index,
            path,
            occurrence,
            sites.class_reference_factories.get(range),
        )
        .is_empty()
        {
            roles.entry(*range).or_insert((
                LspSymbolKind::CLASS,
                matches!(occurrence, ClassOccurrence::Declaration(_)),
            ));
        }
    }
    let indexed = snapshot.library_data().unwrap_or_default();
    let remote = snapshot.remote_exports().unwrap_or_default();
    let read = EditorRead::new(&index, root, model, path, &indexed, &remote);
    for source in &sites.oop {
        let site = &source.site;
        if roles.contains_key(&site.range) {
            continue;
        }
        let kind = match &site.target {
            OopTarget::Class(key)
                if index.declarations.contains_key(&ClassId {
                    path: path.into(),
                    key: key.clone(),
                }) =>
            {
                LspSymbolKind::CLASS
            }
            OopTarget::ClassReference(_)
                if !super::navigation::resolve_registration_class(&index, path, source)
                    .is_empty() =>
            {
                LspSymbolKind::CLASS
            }
            OopTarget::Declaration(_) | OopTarget::Generic(_)
                if site.role != OopSiteRole::Declaration
                    && !super::navigation::resolve_generic_site(&index, path, source)
                        .is_empty() =>
            {
                LspSymbolKind::FUNCTION
            }
            OopTarget::DispatchClass {
                class: DispatchClass::S3(_),
                ..
            } if confirmed_dispatch_class(&index, path, source) => LspSymbolKind::CLASS,
            OopTarget::Member { name, slot, .. } => {
                let Some(member) = member_access(&read, root, site.range, name, *slot) else {
                    continue;
                };
                member_kind(member)
            }
            _ => continue,
        };
        roles.insert(site.range, (kind, site.role != OopSiteRole::Use));
    }
    roles
}

fn confirmed_dispatch_class(index: &ClassIndex, path: &Path, source: &OopSourceSite) -> bool {
    source
        .required_callee
        .as_ref()
        .is_some_and(|(callee, package)| {
            index
                .callee_identity(path, callee)
                .is_some_and(|origin| origin.package == *package)
        })
        && index.dispatch_helper_confirmed(path, source.dispatch_factory.as_ref())
}

fn member_access(
    read: &EditorRead<'_>,
    root: &SyntaxNode,
    range: TextRange,
    name: &str,
    slot: bool,
) -> Option<MemberKind> {
    let value = root.covering_element(range);
    let binary = value.parent().and_then(BinaryExpr::cast)?;
    if binary.rhs().as_ref() != Some(&value)
        || !matches!(binary.op_kind(), Some(SyntaxKind::DOLLAR | SyntaxKind::AT))
    {
        return None;
    }
    Some(read.members(&binary.lhs()?, slot)?.members.get(name)?.kind)
}
