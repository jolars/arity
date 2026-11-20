//! Implementation targets come from resolved class edges and method registries.

use super::*;
use crate::project::classes::{
    MethodImplementationSource, OopUncertainty, S3GenericId, S3Method, S3MethodSource, S4Method,
    Table, class_name_at, class_source_sites_at, locate_class_def_in, method_navigation_sites,
};

pub(in crate::lsp) fn implementation_locations(
    context: ReadContext<'_>,
    uri: &Uri,
    root: &SyntaxNode,
    line_index: &LineIndex,
    offset: TextSize,
    encoding: PositionEncoding,
) -> Vec<Location> {
    let Some((token, value)) = class_name_at(root, offset) else {
        return Vec::new();
    };
    let range = value.text_range();
    let path = normalize_path(context.path);
    with_live_model(
        root,
        Some(ReadContext {
            path: &path,
            ..context
        }),
        |snapshot, model, path| {
            let (facts, sites, occurrence) = class_source_sites_at(model, root, &value);
            let class = class_candidate(&sites, occurrence, range);
            let mut candidates = sites
                .oop
                .iter()
                .filter(|source| {
                    let site = &source.site;
                    (site.range == range || site.range == token.text_range())
                        && matches!(
                            &site.target,
                            OopTarget::Generic(_)
                                | OopTarget::InternalGeneric(_)
                                | OopTarget::BindingDeclaration(_)
                                | OopTarget::Declaration(OopKey {
                                    kind: OopKind::Generic | OopKind::Method,
                                    ..
                                })
                        )
                })
                .peekable();
            let member = sites.oop.iter().find(|source| {
                source.site.range == token.text_range()
                    && matches!(
                        source.site.target,
                        OopTarget::Member { .. }
                            | OopTarget::Declaration(OopKey {
                                kind: OopKind::Member,
                                ..
                            })
                    )
            });
            if class.is_none() && candidates.peek().is_none() && member.is_none() {
                return Vec::new();
            }
            let index = snapshot.class_index_for_presentation(path, &facts, root, model);
            let classes = class.map_or_else(Vec::new, |class| class.resolve(&index, path));
            let location = |target: &Path, range| {
                if target == path {
                    Some(Location::new(
                        uri.clone(),
                        text_range_to_lsp_range(line_index, range, encoding),
                    ))
                } else {
                    location_in(snapshot, target, range, encoding)
                }
            };
            let mut locations = if let Some(member) =
                member.and_then(|site| resolve_member_site(&index, path, root, model, site))
            {
                member::overrides(&index, &member)
                    .into_iter()
                    .filter_map(|id| {
                        let target = &id.path;
                        let declaration = Identity::Member(id.clone());
                        let range = if target == path {
                            declaration.definition(model, root)?
                        } else {
                            let file = snapshot.lookup_file(target)?;
                            declaration.definition(
                                snapshot.semantic_model(file),
                                &snapshot.parsed_tree(file),
                            )?
                        };
                        location(target, range)
                    })
                    .collect()
            } else if classes.is_empty() {
                let method_sites = |target: &Path| {
                    if target == path {
                        Some(method_navigation_sites(model, root))
                    } else {
                        let tree = snapshot.parsed_tree(snapshot.lookup_file(target)?);
                        let model = snapshot.semantic_model_in(target, &tree);
                        Some(method_navigation_sites(&model, &tree))
                    }
                };
                let mut files = HashMap::new();
                let mut bodies = candidates
                    .clone()
                    .flat_map(|site| method_bodies(&index, path, site))
                    .collect::<Vec<_>>();
                if bodies.is_empty() {
                    let refreshed;
                    let mut index = &*index;
                    if let Some(call) = value.parent().and_then(crate::ast::CallExpr::cast)
                        && call.base().as_ref() == Some(&value)
                    {
                        use crate::rindex::schema::ObjectSystem;
                        let indexed = snapshot.library_data().unwrap_or_default();
                        let remote = snapshot.remote_exports().unwrap_or_default();
                        let read = crate::project::classes::EditorRead::new(
                            index, root, model, path, &indexed, &remote,
                        );
                        match read.indexed_dispatch_system(&value) {
                            Some(ObjectSystem::S3 | ObjectSystem::S4) => {
                                refreshed = snapshot.class_index_in(path, &facts, root, model);
                                index = &refreshed;
                            }
                            Some(ObjectSystem::S7) => {
                                bodies.extend(
                                    read.external_s7_implementations(&call)
                                        .into_iter()
                                        .map(MethodBody::Site),
                                );
                            }
                            _ => {}
                        }
                        bodies.extend(
                            index
                                .s4_external_implementations(path, root, model, &call)
                                .into_iter()
                                .filter_map(|method| s4_body(index, method)),
                        );
                    }
                    for site in candidates {
                        let OopTarget::Generic(reference) = &site.site.target else {
                            continue;
                        };
                        if site.required_callee.is_some() || !reference.by_binding {
                            continue;
                        }
                        if reference.system == OopSystem::S3
                            && let Some(generic) = reference
                                .lookup
                                .as_ref()
                                .and_then(|lookup| index.s3_known_call(path, lookup))
                        {
                            let context = index.s3_dispatch_context_at(path, root, model, &value);
                            bodies.extend(
                                index
                                    .visible_s3_methods(&context, &generic)
                                    .filter_map(|method| s3_body(index, method)),
                            );
                        }
                    }
                }
                bodies
                    .into_iter()
                    .filter_map(|body| {
                        let (path, site) = match body {
                            MethodBody::Site(id) => {
                                let sites = files
                                    .entry(id.path.clone())
                                    .or_insert_with(|| method_sites(&id.path))
                                    .as_ref()?;
                                if let Some(range) = sites.implementations.get(&id.key) {
                                    return location(&id.path, *range);
                                }
                                // S7 registrations can also refer to sourced functions.
                                if id.key.system != OopSystem::S7 {
                                    return None;
                                }
                                index.s7_method_callable_origin(&id, sites.sources.get(&id.key)?)?
                            }
                            MethodBody::Callable(path, origin) => (path, origin),
                        };
                        let range = *files
                            .entry(path.clone())
                            .or_insert_with(|| method_sites(&path))
                            .as_ref()?
                            .callables
                            .get(&site)?;
                        location(&path, range)
                    })
                    .collect()
            } else {
                classes
                    .iter()
                    .filter_map(|class| index.subtypes.get(class))
                    .flatten()
                    .filter_map(|child| {
                        let range = if child.path == path {
                            locate_class_def_in(model, root, &child.key)?.name_range
                        } else {
                            let tree = snapshot.parsed_tree(snapshot.lookup_file(&child.path)?);
                            let model = snapshot.semantic_model_in(&child.path, &tree);
                            locate_class_def_in(&model, &tree, &child.key)?.name_range
                        };
                        location(&child.path, range)
                    })
                    .collect()
            };
            dedup_locations(&mut locations);
            locations
        },
    )
}

enum MethodBody {
    Site(OopId),
    Callable(PathBuf, usize),
}

fn method_bodies(index: &ClassIndex, path: &Path, site: &OopSourceSite) -> Vec<MethodBody> {
    if let OopTarget::Declaration(key) = &site.site.target
        && key.kind == OopKind::Method
    {
        let id = OopId {
            path: path.to_path_buf(),
            key: key.clone(),
        };
        return match key.system {
            OopSystem::S4 => {
                let Some(method) = index.s4.methods.iter().find(|method| {
                    method.confirmed_registration() == Some(&id)
                }) else {
                    return Vec::new();
                };
                if site.site.role == OopSiteRole::Declaration {
                    s4_body(index, method).into_iter().collect()
                } else {
                    // A call or alias may name the registration that installed
                    // the generic instead of a setGeneric declaration.
                    method.unique_table().map_or_else(Vec::new, |table| s4_bodies(index, &[table]))
                }
            }
            OopSystem::S3 => index
                .s3
                .methods
                .iter()
                .filter(|method| matches!(&method.source, S3MethodSource::Declaration(source) if *source == id))
                .filter_map(|method| s3_body(index, method))
                .collect(),
            _ => resolve_method_site(index, path, site)
                .into_iter()
                .map(MethodBody::Site)
                .collect(),
        };
    }

    // The generic argument of a registration names the object at its
    // installation site, even if later source code replaces that binding.
    if let Some(key) = &site.registration
        && matches!(key.system, OopSystem::S4 | OopSystem::S3)
    {
        if !callee_confirmed(index, path, site) {
            return Vec::new();
        }
        let id = OopId {
            path: path.to_path_buf(),
            key: key.clone(),
        };
        if key.system == OopSystem::S4 {
            return index
                .s4
                .methods
                .iter()
                .find(|method| method.confirmed_registration() == Some(&id))
                .and_then(S4Method::unique_table)
                .map_or_else(Vec::new, |table| s4_bodies(index, &[table]));
        }
        let generics: Vec<_> = index.s3.methods.iter()
            .filter(|method| matches!(&method.source, S3MethodSource::Declaration(source) if *source == id))
            .flat_map(|method| method.generics.iter().cloned())
            .collect();
        return s3_bodies(index, &generics);
    }

    if let Some(id) = resolve_method_site(index, path, site) {
        return index
            .s3
            .methods
            .iter()
            .find(|method| method.source == S3MethodSource::Declaration(id.clone()))
            .and_then(|method| s3_body(index, method))
            .into_iter()
            .collect();
    }

    let generics = resolve_generic_site(index, path, site);
    let mut bodies = Vec::new();
    for id in generics {
        match id.key.system {
            OopSystem::S4 => {
                let tables: Vec<_> = index
                    .s4
                    .generics
                    .iter()
                    .filter(|generic| generic.declaration.as_ref() == Some(&id))
                    .map(|generic| generic.table())
                    .collect();
                bodies.extend(s4_bodies(index, &tables));
            }
            OopSystem::S3 => {
                bodies.extend(s3_bodies(index, &[S3GenericId::Declaration(id)]));
            }
            _ => bodies.extend(
                index
                    .methods_for_generic(&id)
                    .into_iter()
                    .map(MethodBody::Site),
            ),
        }
    }
    bodies
}

fn s4_bodies(index: &ClassIndex, tables: &[Table<'_>]) -> Vec<MethodBody> {
    index
        .s4
        .methods
        .iter()
        .filter(|method| {
            method
                .unique_table()
                .is_some_and(|table| tables.contains(&table))
        })
        .filter_map(|method| s4_body(index, method))
        .collect()
}

fn s4_body(index: &ClassIndex, method: &S4Method) -> Option<MethodBody> {
    let id = method.confirmed_registration()?;
    if id.key.system == OopSystem::S7 {
        return Some(MethodBody::Site(id.clone()));
    }
    registered_body(index, id, method.implementation.as_ref()?)
}

fn s3_bodies(index: &ClassIndex, generics: &[S3GenericId]) -> Vec<MethodBody> {
    index
        .s3
        .methods
        .iter()
        .filter(|method| {
            method
                .generics
                .iter()
                .any(|generic| generics.contains(generic))
        })
        .filter_map(|method| s3_body(index, method))
        .collect()
}

fn s3_body(index: &ClassIndex, method: &S3Method) -> Option<MethodBody> {
    if method.generics.is_empty()
        || method
            .uncertainty
            .contains(&OopUncertainty::UnresolvedCallee)
    {
        return None;
    }
    match &method.source {
        S3MethodSource::Declaration(id) if !method.registered || id.key.system == OopSystem::S7 => {
            Some(MethodBody::Site(id.clone()))
        }
        S3MethodSource::Declaration(id) => {
            registered_body(index, id, method.implementation.as_ref()?)
        }
        S3MethodSource::Namespace { root, .. } => {
            let MethodImplementationSource::Binding { name, .. } =
                method.implementation.as_ref()?
            else {
                return None;
            };
            let (path, origin) = index.namespace_callable_origin(root, name)?;
            Some(MethodBody::Callable(path, origin))
        }
    }
}

fn registered_body(
    index: &ClassIndex,
    id: &OopId,
    source: &MethodImplementationSource,
) -> Option<MethodBody> {
    match source {
        MethodImplementationSource::Inline => Some(MethodBody::Site(id.clone())),
        MethodImplementationSource::Binding { owner, .. } if !owner.0.is_empty() => {
            Some(MethodBody::Site(id.clone()))
        }
        _ => {
            let (path, origin) = index.registration_callable_origin(id, source)?;
            Some(MethodBody::Callable(path, origin))
        }
    }
}
