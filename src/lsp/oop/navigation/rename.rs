use super::*;
use crate::ast::HasArgList;
use crate::project::classes::{ClassName, ClassUncertainty, OopUncertainty};
use crate::syntax::SyntaxToken;

fn spelling(token: &SyntaxToken) -> Option<String> {
    match token.kind() {
        SyntaxKind::STRING => crate::semantic::decode_string_literal(token.text()),
        SyntaxKind::IDENT => Some(crate::semantic::symbols::unbacktick(token.text()).to_owned()),
        _ => None,
    }
}

fn token_in(root: &SyntaxNode, range: TextRange) -> Option<SyntaxToken> {
    let value = root.covering_element(range);
    if let Some(token) = value.as_token() {
        return (token.text_range() == range).then(|| token.clone());
    }
    crate::ast::BinaryExpr::cast(value.into_node()?)?
        .namespace_access()
        .map(|access| access.name_token)
}

fn identities(
    index: &ClassIndex,
    path: &Path,
    root: &SyntaxNode,
    model: &SemanticModel,
    sites: &ClassSourceSites,
    range: TextRange,
    token_range: TextRange,
) -> Vec<Identity> {
    let mut ids = Vec::new();
    for (at, occurrence) in &sites.classes {
        if *at == range || *at == token_range {
            ids.extend(
                resolve_class_occurrence(
                    index,
                    path,
                    occurrence,
                    sites.class_reference_factories.get(at),
                )
                .into_iter()
                .map(Identity::Class),
            );
        }
    }
    for site in sites
        .oop
        .iter()
        .filter(|site| site.site.range == range || site.site.range == token_range)
    {
        ids.extend(
            resolve_registration_class(index, path, site)
                .into_iter()
                .map(Identity::Class),
        );
        ids.extend(
            resolve_generic_site(index, path, site)
                .into_iter()
                .map(Identity::Generic),
        );
        ids.extend(resolve_member_site(index, path, root, model, site).map(Identity::Member));
        ids.extend(resolve_method_site(index, path, site).map(Identity::Method));
    }
    if ids.iter().any(|id| matches!(id, Identity::Member(_))) {
        ids.retain(|id| matches!(id, Identity::Member(_)));
    }
    let mut unique = Vec::new();
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    unique
}

fn eligible(index: &ClassIndex, target: &Identity, old: &str, new: &str) -> Option<()> {
    let known = ClassName::Known(old.into());
    match target {
        Identity::Class(id) => {
            let class = &index.declarations.get(id)?.declaration;
            if (class.name != known && class.binding.as_deref() != Some(old))
                || index.unresolved.contains(id)
                || class
                    .uncertainty
                    .iter()
                    .any(|issue| *issue != ClassUncertainty::UnresolvedCallee)
            {
                return None;
            }
            if class.name == known
                && (id.key.system == crate::project::classes::ClassSystem::R6
                    || class.s4.as_ref().is_some_and(|class| {
                        class.definition == crate::project::classes::S4Definition::OldClass
                    }))
                && index.s3.methods.iter().any(|method| {
                    (method.class == known || method.class == ClassName::Known(new.into()))
                        && (index.rename_visibility(&id.path, &method.owner.path)
                            || index
                                .file(&id.path)
                                .and_then(|file| file.package_root.as_ref())
                                .is_some_and(|root| {
                                    method.owner.package_root.as_ref() == Some(root)
                                }))
                })
            {
                return None;
            }
            if index
                .oop_files()
                .filter(|file| index.rename_visibility(&id.path, &file.path))
                .any(|file| {
                    file.facts.declarations.iter().any(|other| {
                        other.key.system == id.key.system
                            && (class.name == known && other.name == ClassName::Unknown
                                || other.uncertainty.contains(&ClassUncertainty::DynamicParent))
                    })
                })
            {
                return None;
            }
            if index.declarations.iter().any(|(other, class)| {
                other != id
                    && other.key.system == id.key.system
                    && index.rename_visibility(&id.path, &other.path)
                    && ([old, new].into_iter().any(|name| {
                        class.declaration.name == ClassName::Known(name.into())
                            || class.declaration.binding.as_deref() == Some(name)
                    }))
            }) {
                return None;
            }
        }
        Identity::Generic(id) => {
            let generic = index.generic(id)?;
            if index
                .oop_files()
                .filter(|file| index.rename_visibility(&id.path, &file.path))
                .any(|file| {
                    file.facts.oop.generics.iter().any(|other| {
                        (file.path != id.path || other.key != id.key)
                            && other.key.system == id.key.system
                            && ([old, new].into_iter().any(|name| {
                                other.name == ClassName::Known(name.into())
                                    || other.binding.as_deref() == Some(name)
                            }))
                    })
                })
                || (generic.name != known && generic.binding.as_deref() != Some(old))
                || generic
                    .uncertainty
                    .iter()
                    .any(|issue| *issue != OopUncertainty::UnresolvedCallee)
            {
                return None;
            }
        }
        Identity::Member(id) => {
            let implicit = [
                "new",
                "initialize",
                "finalize",
                "clone",
                "callSuper",
                "self",
                "private",
                "super",
                ".self",
            ];
            if implicit.contains(&old) || implicit.contains(&new) {
                return None;
            }
            let def = index.member(id)?;
            let owner = ClassId {
                path: id.path.clone(),
                key: def.class.clone()?,
            };
            if def.name != known
                || def
                    .uncertainty
                    .iter()
                    .any(|issue| *issue != OopUncertainty::UnresolvedCallee)
                || index.unresolved.contains(&owner)
            {
                return None;
            }
            let mut seen = HashSet::new();
            let mut pending = vec![owner];
            while let Some(class) = pending.pop() {
                if !seen.insert(class.clone()) {
                    continue;
                }
                if index.unresolved.contains(&class)
                    || !complete_members(index, &class)
                    || index
                        .member_candidates(&class, old)
                        .iter()
                        .any(|other| other != id)
                    || index
                        .member_candidates(&class, new)
                        .iter()
                        .any(|other| other != id)
                {
                    return None;
                }
                if index.declarations.get(&class).is_some_and(|class| {
                    class.declaration.s4.as_ref().is_some_and(|metadata| {
                        let prototype = &metadata.prototype;
                        prototype.provided
                            && (!prototype.complete || prototype.slots.contains_key(old))
                    })
                }) {
                    return None;
                }
                if index.parent_candidates.iter().any(|(child, parents)| {
                    index.unresolved.contains(child)
                        && parents.iter().flatten().any(|parent| parent == &class)
                }) {
                    return None;
                }
                pending.extend(index.supertypes.get(&class).into_iter().flatten().cloned());
                pending.extend(index.subtypes.get(&class).into_iter().flatten().cloned());
            }
        }
        _ => return None,
    }
    Some(())
}

fn complete_members(index: &ClassIndex, class: &ClassId) -> bool {
    use crate::project::classes::ClassSystem;
    match class.key.system {
        ClassSystem::R6 => index
            .r6_api(class)
            .is_some_and(|api| api.public.complete && api.private.complete),
        ClassSystem::RefClass => index
            .reference_api(class)
            .is_some_and(|api| api.instance.complete && api.slots.complete),
        ClassSystem::S4 => index.s4_slots(class).is_some_and(|slots| slots.complete),
        ClassSystem::S7 => index.s7_api(class).is_some_and(|api| api.complete),
    }
}

fn unsupported_member_access(
    index: &ClassIndex,
    path: &Path,
    root: &SyntaxNode,
    model: &SemanticModel,
    target: &OopId,
    old: &str,
) -> bool {
    let has_member = |class: &ClassId| index.member_candidates(class, old).contains(target);
    let receiver_matches = |value: crate::syntax::SyntaxElement| {
        index
            .class_receiver(path, root, model, &value)
            .is_some_and(|receiver| has_member(&receiver.class))
    };
    let literal_name = |value: crate::syntax::SyntaxElement| {
        value
            .into_token()
            .filter(|token| token.kind() == SyntaxKind::STRING)
            .and_then(|token| spelling(&token))
    };
    root.descendants().any(|node| {
        if let Some(subset) = crate::ast::Subset2Expr::cast(node.clone()) {
            return subset.base().is_some_and(receiver_matches)
                && subset
                    .args()
                    .next()
                    .and_then(|arg| arg.value())
                    .and_then(literal_name)
                    .is_none_or(|name| name == old);
        }
        let Some(call) = crate::ast::CallExpr::cast(node.clone()) else {
            return false;
        };
        if target.key.system != OopSystem::R6
            && call.args().any(|arg| arg.name().as_deref() == Some(old))
            && index
                .class_receiver_evidence(
                    path,
                    root,
                    model,
                    &crate::syntax::SyntaxElement::Node(node.clone()),
                )
                .candidates
                .iter()
                .any(|receiver| {
                    receiver.access != crate::project::classes::ClassAccess::Generator
                        && has_member(&receiver.class)
                })
        {
            return true;
        }
        if let Some((object, name, _)) =
            crate::project::classes::slot_operands(index, path, root, model, &call)
        {
            return literal_name(name).is_none() && receiver_matches(object);
        }
        if target.key.system != OopSystem::S7 {
            return false;
        }
        index
            .s7_property_use(path, root, model, &crate::syntax::SyntaxElement::Node(node))
            .is_some_and(|property| {
                has_member(&property.receiver.class)
                    && (!property.complete
                        || (property.names.iter().any(|name| name == old)
                            && !call.callee_token().is_some_and(|token| {
                                matches!(
                                    crate::semantic::symbols::unbacktick(token.text()),
                                    "prop" | "prop<-"
                                )
                            })))
            })
    })
}

fn derived_methods(
    index: &ClassIndex,
    target: &Identity,
    old: &str,
    new: &str,
) -> Option<Vec<(Identity, String, String)>> {
    use crate::project::classes::{S3GenericId, S3MethodSource};
    let Identity::Generic(id) = target else {
        return Some(Vec::new());
    };
    if index.generic(id)?.name != ClassName::Known(old.into()) {
        return Some(Vec::new());
    }
    let generic = S3GenericId::Declaration(id.clone());
    let mut derived = Vec::new();
    for method in index
        .s3
        .methods
        .iter()
        .filter(|method| method.generics.contains(&generic))
    {
        if method.generics.as_slice() != [generic.clone()] {
            return None;
        }
        if method.registered {
            continue;
        }
        let (S3MethodSource::Declaration(id), ClassName::Known(class)) =
            (&method.source, &method.class)
        else {
            return None;
        };
        let before = format!("{old}.{class}");
        let after = format!("{new}.{class}");
        if index.method(id)?.name != ClassName::Known(before.clone())
            || !is_syntactic_r_name(&after)
        {
            return None;
        }
        if index.s3.methods.iter().any(|other| {
            use crate::project::classes::MethodImplementationSource;
            other.registered
                && other.generics.as_slice() != [generic.clone()]
                && (index.rename_visibility(&id.path, &other.owner.path)
                    || index.file(&id.path).and_then(|file| file.package_root.as_ref()).is_some_and(|root| other.owner.package_root.as_ref() == Some(root)))
                && matches!(&other.implementation,
                    Some(MethodImplementationSource::Binding { name, .. } | MethodImplementationSource::Reference { name })
                    if *name == before)
        }) {
            return None;
        }
        derived.push((Identity::Method(id.clone()), before, after));
    }
    Some(derived)
}

fn replacement(token: &SyntaxToken, name: &str) -> Option<String> {
    let delimiter = token.text().chars().next()?;
    match (token.kind(), delimiter) {
        (SyntaxKind::STRING, '\'' | '"') | (SyntaxKind::IDENT, '`') => {
            Some(format!("{delimiter}{name}{delimiter}"))
        }
        (SyntaxKind::IDENT, _) => Some(name.into()),
        _ => None,
    }
}

fn validate_edits(
    text: &str,
    edits: &[TextEdit],
    encoding: PositionEncoding,
    namespace: bool,
) -> bool {
    if edits
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start)
    {
        return false;
    }
    let lines = LineIndex::new(text);
    let mut edited = text.to_owned();
    for edit in edits.iter().rev() {
        let start = lines.position_to_byte(edit.range.start, encoding);
        let end = lines.position_to_byte(edit.range.end, encoding);
        edited.replace_range(start..end, &edit.new_text);
    }
    if namespace {
        crate::namespace::parse(&edited).diagnostics.is_empty()
    } else {
        parse(&edited).diagnostics.is_empty()
    }
}

pub(in crate::lsp) struct ObjectRename {
    pub identity: Identity,
    pub spelling: String,
    pub range: TextRange,
    pub declaration: TextRange,
    pub declaration_text: Arc<str>,
    pub edit: WorkspaceEdit,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::lsp) fn object_rename(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    buffer: &TextBuffer,
    offset: TextSize,
    new: Option<&str>,
    encoding: PositionEncoding,
) -> Option<Option<ObjectRename>> {
    let path = normalize_path(path);
    if path.file_name().is_some_and(|name| name == "NAMESPACE") {
        return Some(
            salsa::Cancelled::catch(AssertUnwindSafe(|| {
                let package = path.parent()?;
                let text = snapshot
                    .namespace_sources()
                    .iter()
                    .find(|(root, _)| root == package)?
                    .1
                    .as_str();
                if text != buffer.text() {
                    return None;
                }
                let index = snapshot.class_index_ref();
                let sites = namespace::sites(index, package, text);
                let mut matches = sites.iter().filter(|(range, _)| range.contains(offset));
                let (range, target) = matches.next()?;
                if matches.any(|(_, other)| other != target) {
                    return None;
                }
                let parsed = crate::namespace::parse(text);
                let name = spelling(&token_in(parsed.document().syntax(), *range)?)?;
                let file = snapshot.lookup_file(target.path())?;
                let root = snapshot.parsed_tree(file);
                let model = snapshot.semantic_model(file);
                let sites = snapshot.class_source_sites(file);
                let declaration = sites
                    .oop
                    .iter()
                    .filter(|site| site.site.role == OopSiteRole::Declaration)
                    .filter_map(|site| token_in(&root, site.site.range))
                    .find(|token| {
                        spelling(token).as_deref() == Some(&name)
                            && identities(
                                index,
                                target.path(),
                                &root,
                                model,
                                sites,
                                token.text_range(),
                                token.text_range(),
                            )
                            .as_slice()
                                == [target.clone()]
                    })?;
                let mut result = object_rename(
                    snapshot,
                    target.path(),
                    &uri::from_path(target.path())?,
                    &TextBuffer::from(snapshot.file_text(file)),
                    declaration.text_range().start(),
                    new,
                    encoding,
                )
                .flatten()?;
                result.range = *range;
                Some(result)
            }))
            .ok()
            .flatten(),
        );
    }
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let tracked = snapshot.lookup_file(&path).filter(|&file| {
            snapshot.file_path(file) == Some(path.as_path())
                && snapshot.file_text_is(file, &buffer.text_arc())
        });
        let (root, has_errors) = if let Some(file) = tracked {
            (
                snapshot.parsed_tree(file),
                !snapshot.parse_diagnostics(file).is_empty(),
            )
        } else {
            let parsed = parse(buffer.text());
            (parsed.cst, !parsed.diagnostics.is_empty())
        };
        let (token, value) = crate::project::classes::class_name_at(&root, offset)?;
        let old = spelling(&token)?;
        let new = new.unwrap_or(&old);
        let (live_model, live_source);
        let (model, sites, source) = if let Some(file) = tracked {
            (snapshot.semantic_model(file), snapshot.class_source_sites(file), Ok(file))
        } else {
            live_model = snapshot.semantic_model_in(&path, &root);
            live_source = class_source_sites(&live_model, &root);
            (&live_model, &live_source.1, Err(&live_source.0))
        };
        let range = value.text_range();
        let token_range = token.text_range();
        if !sites
            .classes
            .iter()
            .any(|(at, _)| *at == range || *at == token_range)
            && !sites
                .oop
                .iter()
                .any(|site| site.site.range == range || site.site.range == token_range)
        {
            return None;
        }
        let live_index;
        let index = match source {
            Ok(file) => snapshot.class_index_for_rename(file, &path),
            Err(facts) => {
                live_index =
                    snapshot.class_index_with_body_lookups(&path, facts, &root, model, true);
                &live_index
            }
        };
        let ids = identities(index, &path, &root, model, sites, range, token_range);
        if ids.is_empty() {
            // The extractor records generic lookup candidates for ordinary identifiers.
            return sites
                .oop
                .iter()
                .any(|site| {
                    (site.site.range == range || site.site.range == token_range)
                        && (site.required_callee.is_some()
                            || matches!(
                                site.site.target,
                                OopTarget::Class(_)
                                    | OopTarget::BindingDeclaration(_)
                                    | OopTarget::Member { .. }
                            )
                            || matches!(&site.site.target, OopTarget::Declaration(key) if key.kind != OopKind::Method))
                })
                .then_some(None);
        }
        Some((|| {
            let [target] = ids.as_slice() else {
                return None;
            };
            if has_errors {
                return None;
            }
            eligible(index, target, &old, new)?;
            let renames_binding = match target {
                Identity::Class(id) => index.declarations.get(id)?.declaration.binding.as_deref() == Some(&old),
                Identity::Generic(id) => {
                    let generic = index.generic(id)?;
                    generic.binding.as_deref() == Some(&old)
                        || (id.key.system == OopSystem::S4 && generic.name == ClassName::Known(old.clone()))
                }
                _ => false,
            };
            if renames_binding
                && target.path().is_absolute()
            {
                let binding = snapshot.cross_file_binding(target.path(), &old);
                if binding.conflict || binding.dynamic_source_risk || binding.source_environment_risk {
                    return None;
                }
            }
            let derived = derived_methods(index, target, &old, new)?;
            let new_method_prefix = match target {
                Identity::Generic(id)
                    if old != new
                        && id.key.system == OopSystem::S3
                        && index.generic(id)?.name == ClassName::Known(old.clone()) =>
                {
                    Some(format!("{new}."))
                }
                _ => None,
            };
            let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
            let mut collect = |path: &Path,
                               root: &SyntaxNode,
                               model: &SemanticModel,
                               sites: &ClassSourceSites,
                               lines: &LineIndex,
                               uri: Uri| {
                let visible = index.rename_visibility(target.path(), path);
                if new_method_prefix.as_ref().is_some_and(|prefix| {
                    (visible || index.rename_visibility(path, target.path()))
                        && model.bindings().iter().enumerate().any(|(index, binding)| {
                            model.binding_is_file_scope(crate::semantic::BindingId::from_index(index))
                                && crate::semantic::symbols::unbacktick(&binding.name).starts_with(prefix)
                        })
                }) {
                    return None;
                }
                if visible && !index.class_file_complete(path) {
                    return None;
                }
                if visible
                    && sites
                        .oop
                        .iter()
                        .any(|site| match (&site.site.target, target) {
                            (OopTarget::Generic(reference), Identity::Generic(id))
                                if (site.registration.is_some()
                                    || site.required_callee.is_some())
                                    && reference.system == id.key.system =>
                            {
                                reference.name == ClassName::Unknown
                                    || (reference.name == ClassName::Known(old.clone())
                                        && resolve_generic_site(index, path, site).is_empty()
                                        && callee_confirmed(index, path, site))
                            }
                            _ => false,
                        })
                {
                    return None;
                }
                if let Identity::Member(id) = target
                    && unsupported_member_access(index, path, root, model, id, &old)
                {
                    return None;
                }
                let ranges: HashSet<_> = sites
                    .classes
                    .iter()
                    .map(|(range, _)| *range)
                    .chain(sites.oop.iter().map(|site| site.site.range))
                    .collect();
                let mut edits = Vec::new();
                let mut edited_ranges = HashSet::new();
                let mut binding_definitions = HashSet::new();
                for range in ranges {
                    let Some(token) = token_in(root, range) else {
                        continue;
                    };
                    let Some(spelling) = spelling(&token) else {
                        continue;
                    };
                    if spelling != old && !derived.iter().any(|(_, before, _)| *before == spelling)
                    {
                        continue;
                    }
                    let ids =
                        identities(index, path, root, model, sites, range, token.text_range());
                    let mut renamed = derived.iter().filter_map(|(id, before, after)| {
                        (*before == spelling && ids.contains(id)).then_some(after.as_str())
                    });
                    let replacement_name = if spelling == old && ids.contains(target) {
                        new
                    } else if let Some(after) = renamed.next() {
                        if renamed.next().is_some()
                            || (old != new
                                && model.bindings().iter().any(|binding| binding.name == after))
                        {
                            return None;
                        }
                        after
                    } else {
                        if visible
                            && ids.is_empty()
                            && matches!(target, Identity::Member(_))
                            && sites.oop.iter().any(|site| {
                                site.site.range == range
                                    && matches!(site.site.target, OopTarget::Member { .. })
                            })
                        {
                            return None;
                        }
                        continue;
                    };
                    if ids.len() != 1 || !index.class_file_complete(path) {
                        return None;
                    }
                    edited_ranges.insert(token.text_range());
                    if !matches!(target, Identity::Member(_))
                        && token.kind() == SyntaxKind::IDENT
                        && model
                            .bindings()
                            .iter()
                            .any(|binding| binding.def_range == token.text_range())
                    {
                        binding_definitions.insert(token.text_range());
                    }
                    edits.push(TextEdit::new(
                        text_range_to_lsp_range(lines, token.text_range(), encoding),
                        replacement(&token, replacement_name)?,
                    ));
                }
                for range in binding_definitions {
                    let (_, occurrences) = local_occurrences(root, model, range.start())?;
                    if !occurrences
                        .defs
                        .into_iter()
                        .chain(occurrences.reads)
                        .all(|range| edited_ranges.contains(&range))
                    {
                        return None;
                    }
                }
                if (visible || !edits.is_empty())
                    && old != new && renames_binding
                    && (model.bindings().iter().any(|binding| crate::semantic::symbols::unbacktick(&binding.name) == new)
                        || model.idents().iter().any(|ident| ident.name == new))
                {
                    return None;
                }
                if !edits.is_empty() {
                    changes.entry(uri).or_default().extend(edits);
                }
                Some(())
            };
            collect(
                &path,
                &root,
                model,
                sites,
                buffer.line_index(),
                uri.clone(),
            )?;
            for other in index.oop_files().filter(|file| file.path != path) {
                let file = snapshot.lookup_file(&other.path)?;
                if index.rename_visibility(target.path(), &other.path)
                    && !snapshot.parse_diagnostics(file).is_empty()
                {
                    return None;
                }
                collect(
                    &other.path,
                    &snapshot.parsed_tree(file),
                    snapshot.semantic_model(file),
                    snapshot.class_source_sites(file),
                    snapshot.line_index(file),
                    uri::from_path(&other.path)?,
                )?;
            }
            for (package, text) in snapshot.namespace_sources() {
                let parsed = crate::namespace::parse(text);
                let root = parsed.document().syntax().clone();
                let lines = LineIndex::new(text);
                let sites = namespace::sites(index, package, text);
                for directive in parsed.document().directives() {
                    use crate::namespace::DirectiveKind;
                    let owner_package = index
                        .file(target.path())
                        .and_then(|file| file.package_root.as_deref());
                    if owner_package == Some(package.as_path())
                        && matches!(
                            directive.kind(),
                            DirectiveKind::ExportPattern | DirectiveKind::ExportClassPattern
                        )
                        && old != new
                    {
                        return None;
                    }
                    if old != new
                        && directive.syntax().ancestors().any(|node| node.kind() == SyntaxKind::IF_EXPR)
                    {
                        let mut arguments = directive.arguments();
                        let provider = match directive.kind() {
                            DirectiveKind::Export | DirectiveKind::ExportClass
                            | DirectiveKind::ExportClasses | DirectiveKind::ExportMethods => {
                                Some(package.as_path())
                            }
                            DirectiveKind::ImportFrom | DirectiveKind::ImportClassFrom
                            | DirectiveKind::ImportClassesFrom | DirectiveKind::ImportMethodsFrom => {
                                arguments.next()
                                    .and_then(|arg| namespace::atom(arg.value()?))
                                    .and_then(|name| index.s4_workspace_root(&name))
                                    .map(PathBuf::as_path)
                            }
                            _ => None,
                        };
                        if provider.is_some() && provider == owner_package
                            && arguments.filter_map(|arg| namespace::atom(arg.value()?)).any(|name| {
                                name == old || derived.iter().any(|(_, before, _)| *before == name)
                            })
                        {
                            return None;
                        }
                    }
                    if directive.kind() == DirectiveKind::S3Method
                        && let Some(spec) = crate::rindex::harvest::s3_registration(&directive)
                        && spec.conditional
                        && (spec.generic == old
                            || derived.iter().any(|(_, before, _)| *before == spec.method))
                        && (owner_package == Some(package.as_path())
                            || spec.package.as_ref().is_some_and(|name| {
                                index.s4_workspace_root(name).map(PathBuf::as_path) == owner_package
                            }))
                        && matches!(target, Identity::Generic(_))
                    {
                        return None;
                    }
                    if directive.kind() != DirectiveKind::S3Method
                        || directive.arguments().count() != 2
                    {
                        continue;
                    }
                    let Some(spec) = crate::rindex::harvest::s3_registration(&directive) else {
                        continue;
                    };
                    if spec.generic == old
                        && sites.iter().any(|(range, id)| {
                            id == target && directive.syntax().text_range().contains_range(*range)
                        })
                        && !derived.iter().any(|(_, before, _)| *before == spec.method)
                    {
                        return None;
                    }
                }
                for (range, id) in &sites {
                    if id != target && !derived.iter().any(|(method, _, _)| method == id) {
                        continue;
                    }
                    let token = token_in(&root, *range)?;
                    let name = spelling(&token)?;
                    let replacement_name = if id == target && name == old {
                        new
                    } else if let Some((_, _, after)) = derived
                        .iter()
                        .find(|(method, before, _)| method == id && *before == name)
                    {
                        after
                    } else {
                        continue;
                    };
                    if sites
                        .iter()
                        .any(|(other, other_id)| other == range && other_id != id)
                    {
                        return None;
                    }
                    changes
                        .entry(uri::from_path(&package.join("NAMESPACE"))?)
                        .or_default()
                        .push(TextEdit::new(
                            text_range_to_lsp_range(&lines, *range, encoding),
                            replacement(&token, replacement_name)?,
                        ));
                }
            }
            let result = finalize_rename(changes)?;
            for (edit_uri, edits) in result.changes.as_ref()? {
                let edit_path = uri::to_path(edit_uri);
                let text = if edit_uri == uri {
                    buffer.text()
                } else if edit_path
                    .as_ref()?
                    .file_name()
                    .is_some_and(|name| name == "NAMESPACE")
                {
                    let parent = edit_path.as_ref()?.parent()?;
                    snapshot
                        .namespace_sources()
                        .iter()
                        .find(|(root, _)| root == parent)?
                        .1
                        .as_str()
                } else {
                    snapshot.file_text(snapshot.lookup_file(edit_path.as_ref()?)?)
                };
                if !validate_edits(
                    text,
                    edits,
                    encoding,
                    edit_path
                        .as_ref()
                        .is_some_and(|path| path.ends_with("NAMESPACE")),
                ) {
                    return None;
                }
            }
            let (declaration, declaration_text) = if target.path() == path {
                (target.definition(model, &root)?, buffer.text_arc())
            } else {
                let file = snapshot.lookup_file(target.path())?;
                (
                    target.definition(snapshot.semantic_model(file), &snapshot.parsed_tree(file))?,
                    snapshot.file_text_arc(file),
                )
            };
            Some(ObjectRename {
                declaration,
                declaration_text,
                identity: target.clone(),
                spelling: old.clone(),
                range: token_range,
                edit: result,
            })
        })())
    }))
    .unwrap_or(Some(None))
}
