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
mod rename;
pub(in crate::lsp) use implementation::implementation_locations;
pub(in crate::lsp) use namespace::callable_references as namespace_callable_references;
pub(in crate::lsp) use rename::{ObjectRename, object_rename};

#[derive(Clone, Copy)]
pub(in crate::lsp) enum NameNavigation<'a> {
    Definition,
    References {
        include_declaration: bool,
        open_namespaces: &'a [(PathBuf, Arc<TextBuffer>)],
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Identity {
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
    pub(in crate::lsp) fn path(&self) -> &Path {
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

pub(in crate::lsp) fn resolve_member_site(
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

pub(in crate::lsp) fn resolve_method_site(
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

#[cfg(test)]
mod rename_tests {
    use super::*;

    #[test]
    fn rename_member_keeps_unrelated_receiver() {
        let source = "A <- R6::R6Class('A', public = list(value = 1))\nB <- R6::R6Class('B', public = list(value = 2))\nA$new()$value\nB$new()$value\n";
        let path = ws_path("a.R");
        let uri = uri::from_path(&path).unwrap();
        let snapshot = rename_workspace(source, "");
        let result = rename_via_db(
            &snapshot,
            &path,
            &uri,
            &TextBuffer::from(source),
            source.find("value").unwrap() + 1,
            "item",
            PositionEncoding::Utf16,
        )
        .expect("source member rename");
        let edits = &result.changes.unwrap()[&uri];
        assert_eq!(
            edits
                .iter()
                .map(|edit| edit.range.start.line)
                .collect::<Vec<_>>(),
            [0, 2]
        );
    }

    fn object_edit(
        snapshot_text: &str,
        live: &str,
        cursor: &str,
        name: &str,
    ) -> Option<Vec<TextEdit>> {
        let path = ws_path("a.R");
        let uri = uri::from_path(&path).unwrap();
        let snapshot = rename_workspace(snapshot_text, "");
        rename_via_db(
            &snapshot,
            &path,
            &uri,
            &TextBuffer::from(live),
            live.find(cursor)? + 1,
            name,
            PositionEncoding::Utf16,
        )?
        .changes?
        .remove(&uri)
    }

    #[test]
    fn split_class_name_and_binding_rename_independently_in_the_live_overlay() {
        let tracked =
            "Builder <- R6::R6Class('Example')\nAlias <- Builder\nAlias$new()\nBuilder$new()\n";
        let live = format!("# shift all ranges\n{tracked}");
        let binding = object_edit(tracked, &live, "Builder <-", "Maker").unwrap();
        assert_eq!(
            binding
                .iter()
                .map(|edit| edit.range.start.line)
                .collect::<Vec<_>>(),
            [1, 2, 4]
        );
        let name = object_edit(tracked, &live, "'Example'", "Widget").unwrap();
        assert_eq!(name.len(), 1);
        assert_eq!(name[0].new_text, "'Widget'");
    }

    #[test]
    fn rename_declines_member_overrides_and_dynamic_indices() {
        let source = "A <- R6::R6Class('A', public = list(value = 1))\nB <- R6::R6Class('B', inherit = A, public = list(value = 2))\n";
        assert!(object_edit(source, source, "value = 1", "item").is_none());
        assert!(object_edit(source, source, "value = 2", "item").is_none());
        let source = "A <- R6::R6Class('A', public = list(value = 1))\nA$new()[[key]]\n";
        assert!(object_edit(source, source, "value = 1", "item").is_none());
    }

    #[test]
    fn rename_s3_generic_keeps_dispatch_and_conventional_method_binding_linked() {
        let source = "fmt <- function(x) UseMethod('fmt')\nfmt.foo <- function(x) NextMethod(\"fmt\")\nfmt.foo(1)\nfmt(1)\n";
        let edits = object_edit(source, source, "fmt <-", "render").unwrap();
        assert_eq!(
            edits
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            [
                "render",
                "'render'",
                "render.foo",
                "\"render\"",
                "render.foo",
                "render"
            ]
        );
    }

    #[test]
    fn rename_s4_class_updates_literal_method_signatures() {
        let source = "methods::setClass('Example')\nmethods::setMethod('show', \"Example\", function(object) object)\nmethods::new('Example')\n";
        let edits = object_edit(source, source, "'Example'", "Widget").unwrap();
        assert_eq!(
            edits
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["'Widget'", "\"Widget\"", "'Widget'"]
        );
    }

    #[test]
    fn rename_s3_generic_updates_implicit_and_explicit_namespace_registrations() {
        use std::collections::BTreeSet;
        let package = ws_path("pkg");
        let path = package.join("R/source.R");
        let namespace = package.join("NAMESPACE");
        let ns = "export(fmt)\nS3method('fmt', foo)\nS3method(fmt, bar, render_bar)\n";
        let source = "fmt <- function(x) UseMethod('fmt')\nfmt.foo <- function(x) x\nrender_bar <- function(x) x\n";
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(&path, source);
        db.set_workspace_members(vec![file], vec![ws_root()]);
        db.upsert_description(&package, "Package: pkg\n");
        db.set_package_metadata(vec![crate::project::PackageInfo {
            root: package,
            namespace: Some(ns.into()),
            dir_sources: BTreeSet::new(),
            native_routines: BTreeSet::new(),
        }]);
        let uri = uri::from_path(&path).unwrap();
        let edit = rename_via_db(
            &db.snapshot(),
            &path,
            &uri,
            &TextBuffer::from(source),
            1,
            "render",
            PositionEncoding::Utf16,
        )
        .unwrap();
        let namespace_uri = uri::from_path(&namespace).unwrap();
        let from_namespace = rename_via_db(
            &db.snapshot(),
            &namespace,
            &namespace_uri,
            &TextBuffer::from(ns),
            ns.find("fmt").unwrap() + 1,
            "render",
            PositionEncoding::Utf16,
        )
        .unwrap();
        assert_eq!(from_namespace, edit);
        let changes = edit.changes.unwrap();
        assert_eq!(
            changes[&uri]
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["render", "'render'", "render.foo"]
        );
        assert_eq!(
            changes[&uri::from_path(&namespace).unwrap()]
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["render", "'render'", "render"]
        );
    }

    #[test]
    fn rename_s7_class_updates_method_signature_through_binding_identity() {
        let source = "K <- S7::new_class('K')\nf <- S7::new_generic('f', 'x')\nS7::method(f, K) <- function(x) x\n";
        let edits = object_edit(source, source, "'K'", "Widget").unwrap();
        assert_eq!(
            edits
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["Widget", "'Widget'", "Widget"]
        );
    }

    #[test]
    fn object_rename_refuses_an_incomplete_package() {
        let source = "A <- R6::R6Class('A', public = list(value = 1))\n";
        let (_dir, snapshot, path) = package_workspace(
            "Package: testpkg\n",
            &[("a.R", source, true), ("b.R", "A$new()$value\n", false)],
        );
        let uri = uri::from_path(&path).unwrap();
        assert!(
            rename_via_db(
                &snapshot,
                &path,
                &uri,
                &TextBuffer::from(source),
                source.find("value").unwrap() + 1,
                "item",
                PositionEncoding::Utf16
            )
            .is_none()
        );
    }

    #[test]
    fn member_rename_is_scoped_to_one_workspace_package() {
        let source = "A <- R6::R6Class('A', public = list(value = 1))\nA$new()$value\n";
        let packages = [ws_path("first"), ws_path("second")];
        let paths = packages.each_ref().map(|root| root.join("R/class.R"));
        let mut db = IncrementalDatabase::default();
        let files = paths.each_ref().map(|path| db.upsert_file(path, source));
        db.set_workspace_members(files.to_vec(), vec![ws_root()]);
        db.upsert_description(&packages[0], "Package: first\n");
        db.upsert_description(&packages[1], "Package: second\n");
        db.set_package_metadata(
            packages
                .into_iter()
                .map(|root| crate::project::PackageInfo {
                    root,
                    namespace: None,
                    dir_sources: Default::default(),
                    native_routines: Default::default(),
                })
                .collect(),
        );
        let path = &paths[0];
        let uri = uri::from_path(path).unwrap();
        let changes = rename_via_db(
            &db.snapshot(),
            path,
            &uri,
            &TextBuffer::from(source),
            source.find("value").unwrap() + 1,
            "item",
            PositionEncoding::Utf16,
        )
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&uri].len(), 2);
    }

    #[test]
    fn object_rename_accounts_for_namespace_directives_and_sibling_methods() {
        let package = ws_path("pkg");
        let path = package.join("R/generic.R");
        let sibling = package.join("R/sibling.R");
        let source = "fmt <- function(x) UseMethod('fmt')\nfmt.default <- function(x) 1\n";
        for (namespace, other, accepted) in [
            ("export(fmt)\nexport(fmt.default)\n", "", true),
            ("if (getRversion() >= '3.6.0') { export(fmt) }\n", "", false),
            ("export(fmt)\n", "render.default <- function(x) 2\n", false),
            ("export(fmt)\n", "render.special <- function(x) 2\n", false),
        ] {
            let mut db = IncrementalDatabase::default();
            let files = vec![
                db.upsert_file(&path, source),
                db.upsert_file(&sibling, other),
            ];
            db.set_workspace_members(files, vec![ws_root()]);
            db.upsert_description(&package, "Package: pkg\n");
            db.set_package_metadata(vec![crate::project::PackageInfo {
                root: package.clone(),
                namespace: Some(namespace.into()),
                dir_sources: Default::default(),
                native_routines: Default::default(),
            }]);
            let edit = rename_via_db(
                &db.snapshot(),
                &path,
                &uri::from_path(&path).unwrap(),
                &TextBuffer::from(source),
                1,
                "render",
                PositionEncoding::Utf16,
            );
            if accepted {
                let changes = edit.unwrap().changes.unwrap();
                let namespace_uri = uri::from_path(&package.join("NAMESPACE")).unwrap();
                assert_eq!(
                    changes[&namespace_uri]
                        .iter()
                        .map(|edit| edit.new_text.as_str())
                        .collect::<Vec<_>>(),
                    ["render", "render.default"]
                );
            } else {
                assert!(edit.is_none(), "{namespace} {other}");
            }
        }
    }

    #[test]
    fn class_rename_declines_r6_dispatch_and_dynamic_inheritance() {
        for source in [
            "A <- R6::R6Class('A')\nprint.A <- function(x, ...) 1\nprint(A$new())\n",
            "A <- R6::R6Class('A')\nB <- R6::R6Class('B', inherit = get('A'))\nB$new()\n",
        ] {
            assert!(
                object_edit(source, source, "'A'", "Widget").is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn member_rename_declines_prototypes_and_named_reference_initializers() {
        for source in [
            "methods::setClass('A', slots = c(value = 'numeric'), prototype = list(value = 1))\n",
            "A <- methods::setRefClass('A', fields = c(value = 'numeric'))\nA$new(value = 1)\n",
            "A <- S7::new_class('A', properties = list(value = S7::class_numeric))\nA(value = 1)\n",
        ] {
            assert!(
                object_edit(source, source, "value =", "item").is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn generic_rename_declines_capture_and_dynamic_registration() {
        let source = "fmt <- function(x) UseMethod('fmt')\nf <- function(render) fmt(1)\n";
        assert!(object_edit(source, source, "fmt <-", "render").is_none());
        let source = "fmt <- function(x) UseMethod('fmt')\nbase::registerS3method(dynamic_name, 'foo', function(x) x)\n";
        assert!(object_edit(source, source, "fmt <-", "render").is_none());
    }
}
