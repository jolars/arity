//! Resolve source-backed class and generic names from their semantic sites.

use super::*;
use crate::incremental::normalize_path;
use crate::project::classes::{
    ClassId, ClassOccurrence, ClassSourceSites, FunctionRef, OopId, OopKey, OopKind, OopSiteRole,
    OopSourceSite, OopSystem, OopTarget, class_source_sites, locate_class_def_in, oop_sites,
};

pub(in crate::lsp) enum NameNavigation {
    Definition,
    References { include_declaration: bool },
}

#[derive(PartialEq, Eq)]
enum Identity {
    Class(ClassId),
    Generic(OopId),
}

impl Identity {
    fn path(&self) -> &Path {
        match self {
            Self::Class(id) => &id.path,
            Self::Generic(id) => &id.path,
        }
    }

    fn definition(&self, model: &SemanticModel, root: &SyntaxNode) -> Option<TextRange> {
        match self {
            Self::Class(id) => Some(locate_class_def_in(model, root, &id.key)?.name_range),
            Self::Generic(id) => oop_sites(model, root)
                .into_iter()
                .find(|site| {
                    site.role == OopSiteRole::Declaration
                        && matches!(&site.target, OopTarget::Declaration(key) if *key == id.key)
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
    request: NameNavigation,
) -> Option<Vec<Location>> {
    let range = root
        .token_at_offset(offset)
        .find(|token| token.kind() == SyntaxKind::STRING)?
        .text_range();
    let path = normalize_path(context.path);
    with_live_model(
        root,
        Some(ReadContext {
            path: &path,
            ..context
        }),
        |snapshot, model, _| {
            let (facts, sites) = class_source_sites(model, root);
            let class = sites.classes.iter().find(|(site, _)| *site == range);
            let site = sites.oop.iter().find(|source| {
                let site = &source.site;
                site.range == range
                    && matches!(
                        &site.target,
                        OopTarget::Generic(_)
                            | OopTarget::ClassReference(_)
                            | OopTarget::Declaration(OopKey {
                                kind: OopKind::Generic,
                                ..
                            })
                    )
            });
            if class.is_none() && site.is_none() {
                return None;
            }

            let index = snapshot.class_index_with_body_lookups(
                &path,
                &facts,
                root,
                model,
                matches!(request, NameNavigation::References { .. }),
            );
            let class_query = class.is_some()
                || site
                    .is_some_and(|site| matches!(site.site.target, OopTarget::ClassReference(_)));
            let ids: Vec<_> = if let Some((_, occurrence)) = class {
                resolve_class_occurrence(
                    &index,
                    &path,
                    occurrence,
                    sites.class_reference_factories.get(&range),
                )
                .into_iter()
                .map(Identity::Class)
                .collect()
            } else if class_query {
                resolve_registration_class(&index, &path, site?)
                    .into_iter()
                    .map(Identity::Class)
                    .collect()
            } else {
                resolve_generic_site(&index, &path, site?)
                    .into_iter()
                    .map(Identity::Generic)
                    .collect()
            };
            if ids.is_empty() {
                return None;
            }
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
                } => {
                    let matching_ranges = |file_path: &Path, sites: &ClassSourceSites| {
                        if class_query {
                            sites
                                .classes
                                .iter()
                                .filter(|(range, occurrence)| {
                                    (include_declaration
                                        || !matches!(occurrence, ClassOccurrence::Declaration(_)))
                                        && resolve_class_occurrence(
                                            &index,
                                            file_path,
                                            occurrence,
                                            sites.class_reference_factories.get(range),
                                        )
                                        .into_iter()
                                        .any(|id| ids.contains(&Identity::Class(id)))
                                })
                                .map(|(range, _)| *range)
                                .chain(sites.oop.iter().filter_map(|site| {
                                    resolve_registration_class(&index, file_path, site)
                                        .into_iter()
                                        .any(|id| ids.contains(&Identity::Class(id)))
                                        .then_some(site.site.range)
                                }))
                                .collect::<Vec<_>>()
                        } else {
                            sites
                                .oop
                                .iter()
                                .filter(|site| {
                                    (include_declaration
                                        || site.site.role != OopSiteRole::Declaration)
                                        && resolve_generic_site(&index, file_path, site)
                                            .into_iter()
                                            .any(|id| ids.contains(&Identity::Generic(id)))
                                })
                                .map(|site| site.site.range)
                                .collect()
                        }
                    };
                    locations.extend(
                        matching_ranges(&path, &sites)
                            .into_iter()
                            .map(live_location),
                    );
                    for other in index.oop_files().filter(|file| file.path != path) {
                        let Some(file) = snapshot.lookup_file(&other.path) else {
                            continue;
                        };
                        locations.extend(
                            matching_ranges(&other.path, snapshot.class_source_sites(file))
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

fn resolve_class_occurrence(
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

fn resolve_generic_site(index: &ClassIndex, path: &Path, site: &OopSourceSite) -> Vec<OopId> {
    match &site.site.target {
        OopTarget::Declaration(key) if key.kind == OopKind::Generic => {
            let id = OopId {
                path: path.to_path_buf(),
                key: key.clone(),
            };
            index
                .generic(&id)
                .is_some_and(|generic| index.generic_confirmed(path, generic))
                .then_some(id)
                .into_iter()
                .collect()
        }
        OopTarget::Generic(reference) if callee_confirmed(index, path, site) => {
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

fn resolve_registration_class(
    index: &ClassIndex,
    path: &Path,
    site: &OopSourceSite,
) -> Vec<ClassId> {
    match &site.site.target {
        OopTarget::ClassReference(reference)
            if callee_confirmed(index, path, site)
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
