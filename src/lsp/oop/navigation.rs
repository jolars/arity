//! Resolve source-backed class and generic names from their semantic sites.

use super::*;
use crate::incremental::normalize_path;
use crate::project::classes::{
    ClassId, ClassOccurrence, ClassSourceSites, FunctionRef, GeneratorRef, OopId, OopKey, OopKind,
    OopSiteRole, OopSourceSite, OopSystem, OopTarget, ParentRef, class_source_sites,
    locate_class_def_in, oop_sites,
};

mod implementation;
mod member;
mod namespace;
pub(in crate::lsp) use implementation::implementation_locations;
pub(in crate::lsp) use namespace::callable_references as namespace_callable_references;

#[derive(Clone, Copy)]
pub(in crate::lsp) enum NameNavigation<'a> {
    Definition,
    References {
        include_declaration: bool,
        open_namespaces: &'a [(PathBuf, Arc<TextBuffer>)],
    },
}

#[derive(PartialEq, Eq)]
enum Identity {
    Class(ClassId),
    Generic(OopId),
    Method(OopId),
    Member(OopId),
    Callable {
        path: PathBuf,
        origin: usize,
        package_root: PathBuf,
        binding: String,
    },
}

impl Identity {
    fn path(&self) -> &Path {
        match self {
            Self::Class(id) => &id.path,
            Self::Callable { path, .. } => path,
            Self::Generic(id) | Self::Method(id) | Self::Member(id) => &id.path,
        }
    }

    fn definition(&self, model: &SemanticModel, root: &SyntaxNode) -> Option<TextRange> {
        match self {
            Self::Callable { origin, .. } => {
                crate::project::classes::method_navigation_sites(model, root)
                    .callables
                    .get(origin)
                    .copied()
            }
            Self::Class(id) => Some(locate_class_def_in(model, root, &id.key)?.name_range),
            Self::Generic(id) | Self::Method(id) | Self::Member(id) => oop_sites(model, root)
                .into_iter()
                .find(|site| {
                    site.role == OopSiteRole::Declaration
                        && matches!(&site.target, OopTarget::Declaration(key) | OopTarget::BindingDeclaration(key) if *key == id.key)
                })
                .map(|site| site.range),
        }
    }
}

pub(in crate::lsp) fn name_locations(
    context: ReadContext<'_>,
    uri: &Uri,
    root: &SyntaxNode,
    line_index: &LineIndex,
    offset: TextSize,
    encoding: PositionEncoding,
    request: NameNavigation<'_>,
) -> Option<Vec<Location>> {
    if context
        .path
        .file_name()
        .is_some_and(|name| name == "NAMESPACE")
    {
        return namespace::navigate(context, uri, root, line_index, offset, encoding, request);
    }
    let (token, value) = crate::project::classes::class_name_at(root, offset)?;
    let range = value.text_range();
    let token_range = token.text_range();
    let path = normalize_path(context.path);
    with_live_model(
        root,
        Some(ReadContext {
            path: &path,
            ..context
        }),
        |snapshot, model, _| {
            let live;
            let (facts, sites) = match snapshot.tracked_class_source(&path, root, model) {
                Some(tracked) => tracked,
                None => {
                    live = class_source_sites(model, root);
                    (&live.0, &live.1)
                }
            };
            let class = sites.classes.iter().find(|(site, _)| *site == range);
            let mut candidates = sites
                .oop
                .iter()
                .filter(|source| {
                    let site = &source.site;
                    site.range == range || site.range == token_range
                })
                .peekable();
            if class.is_none() && candidates.peek().is_none() {
                return None;
            }

            let index = snapshot.class_index_for_presentation(&path, facts, root, model);
            if !index.has_source_navigation_targets() {
                return None;
            }
            let mut ids = Vec::new();
            if let Some((_, occurrence)) = class {
                ids.extend(
                    resolve_class_occurrence(
                        &index,
                        &path,
                        occurrence,
                        sites.class_reference_factories.get(&range),
                    )
                    .into_iter()
                    .map(Identity::Class),
                );
            }
            for site in candidates {
                if class.is_none() {
                    ids.extend(
                        resolve_registration_class(&index, &path, site)
                            .into_iter()
                            .map(Identity::Class),
                    );
                }
                ids.extend(
                    resolve_generic_site(&index, &path, site)
                        .into_iter()
                        .map(Identity::Generic),
                );
                ids.extend(resolve_method_site(&index, &path, site).map(Identity::Method));
                ids.extend(
                    resolve_member_site(&index, &path, root, model, site).map(Identity::Member),
                );
            }
            if ids.is_empty() {
                return None;
            }
            if ids.iter().any(|id| matches!(id, Identity::Member(_))) {
                ids.retain(|id| matches!(id, Identity::Member(_)));
            }
            let class_query = ids.iter().any(|id| matches!(id, Identity::Class(_)));
            let live_location = |range| {
                Location::new(
                    uri.clone(),
                    text_range_to_lsp_range(line_index, range, encoding),
                )
            };
            let mut locations = Vec::new();
            match request {
                NameNavigation::Definition => {
                    for id in ids {
                        let target_path = id.path();
                        if target_path == path {
                            locations.extend(id.definition(model, root).map(live_location));
                        } else if let Some(file) = snapshot.lookup_file(target_path) {
                            let target = snapshot.parsed_tree(file);
                            let model = snapshot.semantic_model_in(target_path, &target);
                            if let Some(range) = id.definition(&model, &target)
                                && let Some(location) =
                                    location_in(snapshot, target_path, range, encoding)
                            {
                                locations.push(location);
                            }
                        }
                    }
                }
                NameNavigation::References {
                    include_declaration,
                    open_namespaces,
                } => {
                    // Confirm body-local helper provenance across the workspace
                    // only after resolving a target that needs it.
                    let workspace_index;
                    let index =
                        if class_query || ids.iter().any(|id| matches!(id, Identity::Member(_))) {
                            workspace_index = snapshot
                                .class_index_with_body_lookups(&path, facts, root, model, true);
                            &workspace_index
                        } else {
                            &*index
                        };
                    let member_names: HashSet<_> = ids
                        .iter()
                        .filter_map(|id| {
                            let Identity::Member(id) = id else {
                                return None;
                            };
                            let crate::project::classes::ClassName::Known(name) =
                                &index.member(id)?.name
                            else {
                                return None;
                            };
                            Some(name.as_str())
                        })
                        .collect();
                    let matching_ranges =
                        |file_path: &Path,
                         sites: &ClassSourceSites,
                         syntax: &SyntaxNode,
                         semantics: &SemanticModel| {
                            if class_query {
                                sites
                                    .classes
                                    .iter()
                                    .filter(|(range, occurrence)| {
                                        (include_declaration
                                            || !matches!(
                                                occurrence,
                                                ClassOccurrence::Declaration(_)
                                            ))
                                            && resolve_class_occurrence(
                                                index,
                                                file_path,
                                                occurrence,
                                                sites.class_reference_factories.get(range),
                                            )
                                            .into_iter()
                                            .any(|id| ids.contains(&Identity::Class(id)))
                                    })
                                    .map(|(range, _)| *range)
                                    .chain(sites.oop.iter().filter_map(|site| {
                                        if !include_declaration
                                            && site.site.role == OopSiteRole::Declaration
                                        {
                                            return None;
                                        }
                                        resolve_registration_class(index, file_path, site)
                                            .into_iter()
                                            .any(|id| ids.contains(&Identity::Class(id)))
                                            .then_some(site.site.range)
                                    }))
                                    .collect::<Vec<_>>()
                            } else if ids.iter().any(|id| matches!(id, Identity::Member(_))) {
                                sites
                                    .oop
                                    .iter()
                                    .filter_map(|site| {
                                        if !include_declaration
                                            && site.site.role == OopSiteRole::Declaration
                                        {
                                            return None;
                                        }
                                        let name = match &site.site.target {
                                            OopTarget::Member { name, .. }
                                            | OopTarget::ClassReference(ParentRef::Generator(
                                                GeneratorRef::Free(name),
                                            )) => Some(name.as_str()),
                                            _ => None,
                                        };
                                        if name.is_some_and(|name| {
                                            name != "callSuper" && !member_names.contains(name)
                                        }) {
                                            return None;
                                        }
                                        resolve_member_site(
                                            index, file_path, syntax, semantics, site,
                                        )
                                        .filter(|id| ids.contains(&Identity::Member(id.clone())))
                                        .map(|_| site.site.range)
                                    })
                                    .collect()
                            } else {
                                sites
                                    .oop
                                    .iter()
                                    .filter(|site| {
                                        (include_declaration
                                            || site.site.role != OopSiteRole::Declaration)
                                            && (resolve_generic_site(index, file_path, site)
                                                .into_iter()
                                                .any(|id| ids.contains(&Identity::Generic(id)))
                                                || resolve_method_site(index, file_path, site)
                                                    .is_some_and(|id| {
                                                        ids.contains(&Identity::Method(id))
                                                    }))
                                    })
                                    .map(|site| site.site.range)
                                    .collect()
                            }
                        };
                    locations.extend(namespace::references(
                        snapshot,
                        index,
                        &ids,
                        encoding,
                        open_namespaces,
                    ));
                    locations.extend(
                        matching_ranges(&path, sites, root, model)
                            .into_iter()
                            .map(live_location),
                    );
                    for other in index.oop_files().filter(|file| file.path != path) {
                        let Some(file) = snapshot.lookup_file(&other.path) else {
                            continue;
                        };
                        locations.extend(
                            matching_ranges(
                                &other.path,
                                snapshot.class_source_sites(file),
                                &snapshot.parsed_tree(file),
                                snapshot.semantic_model(file),
                            )
                            .into_iter()
                            .filter_map(|range| {
                                location_in(snapshot, &other.path, range, encoding)
                            }),
                        );
                    }
                }
            }
            dedup_locations(&mut locations);
            Some(locations)
        },
    )
}

fn resolve_member_site(
    index: &ClassIndex,
    path: &Path,
    root: &SyntaxNode,
    model: &SemanticModel,
    site: &OopSourceSite,
) -> Option<OopId> {
    if let OopTarget::Declaration(key) = &site.site.target
        && key.kind == OopKind::Member
    {
        let id = OopId {
            path: path.into(),
            key: key.clone(),
        };
        return index
            .member(&id)
            .filter(|member| index.member_declaration_confirmed(path, member))
            .map(|_| id);
    }
    if site.site.role == OopSiteRole::Use
        && matches!(
            site.site.target,
            OopTarget::ClassReference(ParentRef::Generator(GeneratorRef::Free(_)))
        )
    {
        let value = root.covering_element(site.site.range);
        return index
            .implicit_reference_member(path, root, model, &value)?
            .declaration;
    }
    member::member_at(index, path, root, model, site)
}

pub(in crate::lsp) enum ClassCandidate<'a> {
    Occurrence(ClassOccurrence, Option<&'a FunctionRef>),
    Registration(Vec<&'a OopSourceSite>),
}

impl ClassCandidate<'_> {
    pub(in crate::lsp) fn resolve(self, index: &ClassIndex, path: &Path) -> Vec<ClassId> {
        match self {
            Self::Occurrence(occurrence, factory) => {
                resolve_class_occurrence(index, path, &occurrence, factory)
            }
            Self::Registration(sites) => sites
                .into_iter()
                .flat_map(|site| resolve_registration_class(index, path, site))
                .collect(),
        }
    }
}

pub(in crate::lsp) fn class_candidate(
    sites: &ClassSourceSites,
    occurrence: Option<ClassOccurrence>,
    range: TextRange,
) -> Option<ClassCandidate<'_>> {
    if let Some(occurrence) = occurrence
        && !matches!(
            occurrence,
            ClassOccurrence::Reference {
                reference: ParentRef::Unknown | ParentRef::Generator(GeneratorRef::Unknown),
                ..
            }
        )
    {
        return Some(ClassCandidate::Occurrence(
            occurrence,
            sites.class_reference_factories.get(&range),
        ));
    }
    let registrations: Vec<_> = sites
        .oop
        .iter()
        .filter(|source| {
            source.site.range == range
                && matches!(
                    source.site.target,
                    OopTarget::ClassReference(_) | OopTarget::DispatchClass { .. }
                )
        })
        .collect();
    (!registrations.is_empty()).then_some(ClassCandidate::Registration(registrations))
}

pub(super) fn resolve_class_occurrence(
    index: &ClassIndex,
    path: &Path,
    occurrence: &ClassOccurrence,
    factory: Option<&FunctionRef>,
) -> Vec<ClassId> {
    match occurrence {
        ClassOccurrence::Declaration(key) => {
            let id = ClassId {
                path: path.to_path_buf(),
                key: key.clone(),
            };
            index
                .declarations
                .contains_key(&id)
                .then_some(id)
                .into_iter()
                .collect()
        }
        ClassOccurrence::Reference {
            owner,
            reference,
            deferred,
        } if factory.is_none_or(|callee| callee_from_package(index, path, callee, "methods")) => {
            index.resolve_reference(path, owner, reference, *deferred)
        }
        _ => Vec::new(),
    }
}

pub(super) fn resolve_generic_site(
    index: &ClassIndex,
    path: &Path,
    site: &OopSourceSite,
) -> Vec<OopId> {
    match &site.site.target {
        OopTarget::Declaration(key) | OopTarget::BindingDeclaration(key)
            if key.kind == OopKind::Generic =>
        {
            let id = OopId {
                path: path.to_path_buf(),
                key: key.clone(),
            };
            index
                .generic(&id)
                .is_some_and(|generic| {
                    index.generic_confirmed(path, generic)
                        && !generic
                            .uncertainty
                            .contains(&crate::project::classes::OopUncertainty::DynamicName)
                })
                .then_some(id)
                .into_iter()
                .collect()
        }
        OopTarget::InternalGeneric(reference) => {
            use crate::project::classes::ClassName;
            let (Some(package), ClassName::Known(_)) = (&reference.package, &reference.name) else {
                return Vec::new();
            };
            let Some(root) = index.s4_workspace_root(package) else {
                return Vec::new();
            };
            index
                .oop_files()
                .filter(|file| file.package_root.as_ref() == Some(root))
                .flat_map(|file| {
                    index
                        .generic_candidates(
                            &file.path,
                            &crate::project::classes::GenericRef {
                                package: None,
                                owner: Default::default(),
                                ..reference.clone()
                            },
                            false,
                        )
                        .into_iter()
                        .filter(|id| id.path == file.path)
                })
                .collect()
        }
        OopTarget::Generic(reference)
            if callee_confirmed(index, path, site)
                || (site.required_callee.is_none()
                    && site.registration.is_none()
                    && reference.by_binding
                    && reference.lookup.is_some()) =>
        {
            if let Some(key) = &site.registration {
                use crate::project::classes::{S3GenericId, S3MethodSource, S7GenericTarget};
                let id = OopId {
                    path: path.into(),
                    key: key.clone(),
                };
                match key.system {
                    OopSystem::S3 => {
                        return index
                            .s3
                            .methods
                            .iter()
                            .filter(|method| {
                                method.source == S3MethodSource::Declaration(id.clone())
                            })
                            .flat_map(|method| &method.generics)
                            .filter_map(|generic| match generic {
                                S3GenericId::Declaration(id) => Some(id.clone()),
                                _ => None,
                            })
                            .collect();
                    }
                    OopSystem::S4 => {
                        if let Some(table) = index
                            .s4
                            .methods
                            .iter()
                            .find(|method| method.confirmed_registration() == Some(&id))
                            .and_then(|method| method.unique_table())
                        {
                            return index
                                .s4
                                .generics
                                .iter()
                                .filter(|generic| generic.table() == table)
                                .filter_map(|generic| generic.declaration.clone())
                                .collect();
                        }
                    }
                    OopSystem::S7 => {
                        if let Some(registration) = index.s7_registration(&id) {
                            return registration
                                .targets
                                .into_iter()
                                .flat_map(|target| match target {
                                    S7GenericTarget::S7(id)
                                    | S7GenericTarget::S3(S3GenericId::Declaration(id)) => vec![id],
                                    S7GenericTarget::S4(id) => index
                                        .s4
                                        .generics
                                        .iter()
                                        .filter(|generic| generic.id == id)
                                        .filter_map(|generic| generic.declaration.clone())
                                        .collect(),
                                    _ => Vec::new(),
                                })
                                .collect();
                        }
                    }
                    _ => {}
                }
            }
            if reference.system == OopSystem::S4 {
                index
                    .s4_generic_candidates(path, reference, site.site.in_function)
                    .into_iter()
                    .filter_map(|generic| generic.declaration.clone())
                    .collect()
            } else {
                index.generic_candidates(path, reference, site.site.in_function)
            }
        }
        _ => Vec::new(),
    }
}

pub(super) fn resolve_method_site(
    index: &ClassIndex,
    path: &Path,
    site: &OopSourceSite,
) -> Option<OopId> {
    if let OopTarget::Generic(reference) | OopTarget::InternalGeneric(reference) = &site.site.target
        && reference.system == OopSystem::S3
        && reference.by_binding
        && reference.lookup.is_some()
        && site.registration.is_none()
        && site.required_callee.is_none()
    {
        return index.s3_method_binding(
            path,
            reference,
            site.site.in_function,
            matches!(&site.site.target, OopTarget::InternalGeneric(_)),
        );
    }
    let OopTarget::Declaration(key) = &site.site.target else {
        return None;
    };
    if key.kind != OopKind::Method {
        return None;
    }
    let id = OopId {
        path: path.to_path_buf(),
        key: key.clone(),
    };
    let method = index.method(&id)?;
    let confirmed = match key.system {
        OopSystem::S3 => index.s3.methods.iter().any(|method| {
            matches!(&method.source,
            crate::project::classes::S3MethodSource::Declaration(source) if *source == id)
                && !method.generics.is_empty()
        }),
        OopSystem::S4 => index.s4.methods.iter().any(|method| {
            matches!(&method.id,
            crate::project::classes::S4MethodId::Registration(source) if *source == id)
        }),
        OopSystem::S7 => index.s7_registration(&id).is_some(),
        _ => method.registration.is_none(),
    };
    confirmed.then_some(id)
}

pub(super) fn resolve_registration_class(
    index: &ClassIndex,
    path: &Path,
    site: &OopSourceSite,
) -> Vec<ClassId> {
    match &site.site.target {
        OopTarget::Class(key) => resolve_class_occurrence(
            index,
            path,
            &ClassOccurrence::Declaration(key.clone()),
            None,
        ),
        OopTarget::DispatchClass { class, .. }
            if callee_confirmed(index, path, site)
                && index.dispatch_helper_confirmed(path, site.dispatch_factory.as_ref()) =>
        {
            use crate::project::classes::{DispatchClass, S4Definition};
            match class {
                DispatchClass::S7(spec) => index
                    .s7_spec_class(path, &site.site.owner, spec)
                    .into_iter()
                    .collect(),
                DispatchClass::S3(name) => index
                    .resolve_reference(
                        path,
                        &site.site.owner,
                        &ParentRef::Formal(name.clone()),
                        site.site.in_function,
                    )
                    .into_iter()
                    .filter(|id| {
                        index
                            .declarations
                            .get(id)
                            .and_then(|class| class.declaration.s4.as_ref())
                            .is_some_and(|class| class.definition == S4Definition::OldClass)
                    })
                    .collect(),
                _ => Vec::new(),
            }
        }
        OopTarget::ClassReference(reference)
            if (callee_confirmed(index, path, site)
                || (site.required_callee.is_none()
                    && matches!(reference, crate::project::classes::ParentRef::Generator(_))))
                && index.dispatch_helper_confirmed(path, site.dispatch_factory.as_ref()) =>
        {
            index.resolve_reference(path, &site.site.owner, reference, site.site.in_function)
        }
        _ => Vec::new(),
    }
}

fn callee_confirmed(index: &ClassIndex, path: &Path, site: &OopSourceSite) -> bool {
    site.required_callee
        .as_ref()
        .is_some_and(|(callee, package)| callee_from_package(index, path, callee, package))
}

fn callee_from_package(
    index: &ClassIndex,
    path: &Path,
    callee: &FunctionRef,
    package: &str,
) -> bool {
    index
        .callee_identity(path, callee)
        .is_some_and(|identity| identity.package == package)
}
