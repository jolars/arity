//! Explicit NAMESPACE operands have source spans and package-scoped identities.

use super::*;
use crate::ast::BinaryExpr;
use crate::namespace::DirectiveKind;
use crate::project::classes::{
    ClassName, ClassSystem, MethodImplementationSource, S3GenericId, S3MethodSource,
};
use crate::semantic::symbols::unbacktick;
use crate::syntax::SyntaxElement;

pub(super) fn sites(index: &ClassIndex, root: &Path, text: &str) -> Vec<(TextRange, Identity)> {
    collect_sites(index, root, text, SiteFilter::All)
}

#[derive(Clone, Copy)]
enum SiteFilter<'a> {
    All,
    Callable(Option<&'a str>),
    References { include_methods: bool },
}

fn collect_sites(
    index: &ClassIndex,
    root: &Path,
    text: &str,
    filter: SiteFilter<'_>,
) -> Vec<(TextRange, Identity)> {
    let parsed = crate::namespace::parse(text);
    // A generic or class reference cannot match an implementation operand.
    // Avoid resolving its callable origin or scanning conventional methods.
    let include_methods = !matches!(
        filter,
        SiteFilter::References {
            include_methods: false
        }
    );
    let mut sites = Vec::new();
    let mut registration = 0;
    for directive in parsed.document().directives() {
        if directive.kind() == DirectiveKind::S3Method {
            let Some(spec) = crate::rindex::harvest::s3_registration(&directive) else {
                continue;
            };
            let occurrence = registration;
            registration += 1;
            if spec.conditional
                || matches!(filter, SiteFilter::Callable(binding) if
                    binding.is_some_and(|binding| binding != unbacktick(&spec.method))
                        || directive.arguments().nth(2).is_none())
            {
                continue;
            }
            let mut arguments = directive.arguments();
            let Some(value) = arguments.next().and_then(|arg| arg.value()) else {
                continue;
            };
            let range = value
                .as_node()
                .cloned()
                .and_then(BinaryExpr::cast)
                .and_then(|binary| binary.namespace_access())
                .map_or(value.text_range(), |access| access.name_token.text_range());
            let source = S3MethodSource::Namespace {
                root: root.into(),
                occurrence,
            };
            let methods = index.s3.methods.iter().filter(|method| {
                method.source == source
                    && method.generic.name == ClassName::Known(spec.generic.clone())
                    && method.generic.package == spec.package
                    && method.class == ClassName::Known(spec.class.clone())
            });
            if let Some(method) = methods.into_iter().next() {
                if !matches!(filter, SiteFilter::Callable(_)) {
                    sites.extend(method.generics.iter().filter_map(|id| match id {
                        S3GenericId::Declaration(id) => {
                            Some((range, Identity::Generic(id.clone())))
                        }
                        S3GenericId::Known { .. } => None,
                    }));
                }
                if include_methods
                    && let Some(span) = arguments.nth(1).and_then(|argument| argument.value_range())
                    && matches!(&method.implementation, Some(MethodImplementationSource::Binding { name, .. }) if *name == spec.method)
                    && let Some((path, origin)) =
                        index.namespace_callable_origin(root, &spec.method)
                {
                    let declarations = conventional_methods(index, &spec.method);
                    let identity = match conventional_method(declarations, &path) {
                        Some(id) => Identity::Method(id),
                        _ => Identity::Callable {
                            path,
                            origin,
                            package_root: root.to_path_buf(),
                            binding: spec.method.clone(),
                        },
                    };
                    sites.push((span, identity));
                }
            }
            continue;
        }
        if matches!(filter, SiteFilter::Callable(_)) {
            continue;
        }
        if directive.r_paren().is_none()
            || directive
                .syntax()
                .ancestors()
                .any(|node| node.kind() == SyntaxKind::IF_EXPR)
        {
            continue;
        }
        let kind = directive.kind();
        let mut arguments = directive.arguments();
        let target = match kind {
            DirectiveKind::ImportFrom
            | DirectiveKind::ImportMethodsFrom
            | DirectiveKind::ImportClassFrom
            | DirectiveKind::ImportClassesFrom => {
                let Some(package) = arguments.next().and_then(|arg| atom(arg.value()?)) else {
                    continue;
                };
                let Some(target) = index.s4_workspace_root(&package) else {
                    continue;
                };
                target.as_path()
            }
            DirectiveKind::Export
            | DirectiveKind::ExportClass
            | DirectiveKind::ExportClasses
            | DirectiveKind::ExportMethods => root,
            _ => continue,
        };
        for argument in arguments {
            let Some(value) = argument.value() else {
                continue;
            };
            let range = value.text_range();
            let Some(name) = atom(value) else { continue };
            if matches!(
                kind,
                DirectiveKind::ExportClass
                    | DirectiveKind::ExportClasses
                    | DirectiveKind::ImportClassFrom
                    | DirectiveKind::ImportClassesFrom
            ) {
                if target != root
                    && !index
                        .s4_namespace(target)
                        .is_some_and(|namespace| namespace.exported_classes.contains(&name))
                {
                    continue;
                }
                sites.extend(
                    index
                        .declarations
                        .iter()
                        .filter(|(id, class)| {
                            class.package_root.as_deref() == Some(target)
                                && id.key.owner.0.is_empty()
                                && matches!(id.key.system, ClassSystem::S4 | ClassSystem::RefClass)
                                && class.declaration.name == ClassName::Known(name.clone())
                        })
                        .map(|(id, _)| (range, Identity::Class(id.clone()))),
                );
                continue;
            }
            if include_methods && matches!(kind, DirectiveKind::Export | DirectiveKind::ImportFrom)
            {
                // Only conventional S3 methods can produce a callable site here.
                // Replay namespace execution only if a matching declaration exists.
                let mut declarations = conventional_methods(index, &name).peekable();
                if declarations.peek().is_some()
                    && let Some((path, _)) = index.namespace_callable_origin(target, &name)
                    && (target == root
                        || index
                            .file(&path)
                            .is_some_and(|file| file.exports.contains(&name)))
                    && let Some(id) = conventional_method(declarations, &path)
                {
                    sites.push((range, Identity::Method(id)));
                }
            }
            for file in index
                .oop_files()
                .filter(|file| file.package_root.as_deref() == Some(target))
            {
                let methods = matches!(
                    kind,
                    DirectiveKind::ExportMethods | DirectiveKind::ImportMethodsFrom
                );
                if target != root {
                    let exported = if methods {
                        index
                            .s4_namespace(target)
                            .is_some_and(|namespace| namespace.exported_methods.contains(&name))
                    } else {
                        file.exports.contains(&name)
                    };
                    if !exported {
                        continue;
                    }
                }
                if methods {
                    let tables = index
                        .s4
                        .methods
                        .iter()
                        .filter(|method| {
                            method.owner.path == file.path
                                && method.generic.name == ClassName::Known(name.clone())
                        })
                        .filter_map(|method| method.unique_table())
                        .collect::<Vec<_>>();
                    sites.extend(
                        index
                            .s4
                            .generics
                            .iter()
                            .filter(|generic| tables.contains(&generic.table()))
                            .filter_map(|generic| generic.declaration.clone())
                            .map(|id| (range, Identity::Generic(id))),
                    );
                }
                for generic in &file.facts.oop.generics {
                    // Check whether this operand can name the generic before resolving its factory.
                    if generic.key.owner.0.is_empty()
                        && (methods
                            && generic.key.system == OopSystem::S4
                            && generic.name == ClassName::Known(name.clone())
                            || !methods
                                && (generic.binding.as_deref() == Some(&name)
                                    || file.facts.oop.generic_bindings.iter().any(|binding| {
                                        binding.owner.0.is_empty()
                                            && binding.name == name
                                            && binding.target == generic.key
                                    })
                                    || generic.key.system == OopSystem::S4
                                        && generic.name == ClassName::Known(name.clone())))
                        && index.generic_confirmed(&file.path, generic)
                    {
                        sites.push((
                            range,
                            Identity::Generic(OopId {
                                path: file.path.clone(),
                                key: generic.key.clone(),
                            }),
                        ));
                    }
                }
                if !methods {
                    sites.extend(
                        index
                            .namespace_generator_targets(file, &name)
                            .into_iter()
                            .map(|id| (range, Identity::Class(id))),
                    );
                }
            }
        }
    }
    sites
}

fn conventional_methods<'a>(
    index: &'a ClassIndex,
    name: &'a str,
) -> impl Iterator<Item = &'a OopId> {
    index.s3.methods.iter().filter_map(move |candidate| {
        let S3MethodSource::Declaration(id) = &candidate.source else {
            return None;
        };
        (!candidate.registered
            && !candidate.generics.is_empty()
            && index.method(id).is_some_and(
                |method| matches!(&method.name, ClassName::Known(binding) if binding == name),
            ))
        .then_some(id)
    })
}

fn conventional_method<'a>(
    declarations: impl Iterator<Item = &'a OopId>,
    path: &Path,
) -> Option<OopId> {
    let declarations: std::collections::BTreeSet<_> =
        declarations.filter(|id| id.path == path).collect();
    (declarations.len() == 1)
        .then(|| declarations.into_iter().next().cloned())
        .flatten()
}

pub(super) fn atom(value: SyntaxElement) -> Option<String> {
    let token = value.into_token()?;
    match token.kind() {
        SyntaxKind::IDENT => Some(crate::semantic::symbols::unbacktick(token.text()).into()),
        SyntaxKind::STRING => crate::semantic::decode_string_literal(token.text()),
        _ => None,
    }
}

pub(super) fn references(
    snapshot: &Analysis,
    index: &ClassIndex,
    ids: &[Identity],
    encoding: PositionEncoding,
    open_namespaces: &[(PathBuf, Arc<TextBuffer>)],
) -> Vec<Location> {
    reference_sites(
        snapshot,
        index,
        encoding,
        open_namespaces,
        SiteFilter::References {
            include_methods: ids
                .iter()
                .any(|id| matches!(id, Identity::Method(_) | Identity::Callable { .. })),
        },
        |id| ids.contains(id),
    )
}

pub(in crate::lsp) fn callable_references(
    snapshot: &Analysis,
    encoding: PositionEncoding,
    open_namespaces: &[(PathBuf, Arc<TextBuffer>)],
    binding: Option<&str>,
    lexical_with_declarations: impl FnOnce() -> Vec<Location>,
) -> Vec<Location> {
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        if !snapshot
            .namespace_sources()
            .iter()
            .any(|(_, text)| text.contains("S3method"))
            && !open_namespaces
                .iter()
                .any(|(_, buffer)| buffer.text().contains("S3method"))
        {
            return Vec::new();
        }
        let binding = binding.map(unbacktick);
        let index = snapshot.class_index_ref();
        // A live registration can only produce a callable site when its
        // implementation matches the registry entry at that occurrence.
        if binding.is_some_and(|binding| {
            !index.s3.methods.iter().any(|method| {
                matches!(method.source, S3MethodSource::Namespace { .. })
                    && matches!(&method.implementation, Some(MethodImplementationSource::Binding { name, .. }) if unbacktick(name) == binding)
            })
        }) {
            return Vec::new();
        }
        let lexical = lexical_with_declarations();
        if lexical.is_empty() {
            return Vec::new();
        }
        reference_sites(snapshot, index, encoding, open_namespaces, SiteFilter::Callable(binding), |id| {
            let Identity::Callable {
                binding,
                package_root,
                ..
            } = id
            else {
                return false;
            };
            callable_binding_files(index, package_root, binding).any(|path| {
                let Some(file) = snapshot.lookup_file(path) else {
                    return false;
                };
                let Some((definitions, _)) =
                    file_scope_occurrences_in(snapshot.runtime_semantic_model(file), binding)
                else {
                    return false;
                };
                definitions.into_iter().any(|range| {
                    location_in(snapshot, path, range, encoding)
                        .is_some_and(|definition| lexical.contains(&definition))
                })
            })
        })
    }))
    .unwrap_or_default()
}

fn callable_binding_files<'a>(
    index: &'a ClassIndex,
    package_root: &'a Path,
    binding: &'a str,
) -> impl Iterator<Item = &'a Path> {
    index
        .oop_files()
        .filter(move |file| {
            file.package_root.as_deref() == Some(package_root) && file.exports.contains(binding)
        })
        .map(|file| file.path.as_path())
}

fn reference_sites(
    snapshot: &Analysis,
    index: &ClassIndex,
    encoding: PositionEncoding,
    open_namespaces: &[(PathBuf, Arc<TextBuffer>)],
    filter: SiteFilter<'_>,
    matches: impl Fn(&Identity) -> bool,
) -> Vec<Location> {
    let mut locations = Vec::new();
    // The request carries the open documents themselves. A closed NAMESPACE
    // falls back to package metadata even if salsa still tracks its old text.
    for (root, text) in snapshot.namespace_sources() {
        let path = root.join("NAMESPACE");
        let Some(uri) = uri::from_path(&path) else {
            continue;
        };
        let live = open_namespaces.iter().find(|(open, _)| *open == path);
        let disk_index;
        let (text, line_index) = if let Some((_, buffer)) = live {
            (buffer.text(), buffer.line_index())
        } else {
            disk_index = LineIndex::new(text);
            (text.as_str(), &disk_index)
        };
        locations.extend(
            collect_sites(index, root, text, filter)
                .into_iter()
                .filter(|(_, id)| matches(id))
                .map(|(range, _)| {
                    Location::new(
                        uri.clone(),
                        text_range_to_lsp_range(line_index, range, encoding),
                    )
                }),
        );
    }
    locations
}

pub(super) fn navigate(
    context: ReadContext<'_>,
    uri: &Uri,
    root: &SyntaxNode,
    line_index: &LineIndex,
    offset: TextSize,
    encoding: PositionEncoding,
    request: NameNavigation<'_>,
) -> Option<Vec<Location>> {
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let path = normalize_path(context.path);
        let package = path.parent()?;
        let snapshot = context.snapshot;
        let index = snapshot.class_index();
        let text = root.to_string();
        let sites = sites(&index, package, &text);
        let mut locations = Vec::new();
        for (_, id) in sites.iter().filter(|(range, _)| range.contains(offset)) {
            let target = id.path();
            let Some(file) = snapshot.lookup_file(target) else {
                continue;
            };
            let tree = snapshot.parsed_tree(file);
            let model = snapshot.semantic_model(file);
            let Some(definition) = id.definition(model, &tree) else {
                continue;
            };
            match request {
                NameNavigation::Definition => {
                    locations.extend(location_in(snapshot, target, definition, encoding));
                }
                NameNavigation::References {
                    include_declaration,
                    open_namespaces,
                } => {
                    if let Some(target_uri) = uri::from_path(target) {
                        if let Identity::Callable {
                            binding,
                            package_root,
                            ..
                        } = id
                        {
                            for file in callable_binding_files(&index, package_root, binding) {
                                locations.extend(cross_file_reference_locations(
                                    snapshot,
                                    file,
                                    binding,
                                    include_declaration,
                                    None,
                                    encoding,
                                ));
                            }
                            locations.extend(
                                references(
                                    snapshot,
                                    &index,
                                    std::slice::from_ref(id),
                                    encoding,
                                    open_namespaces,
                                )
                                .into_iter()
                                .filter(|location| location.uri != *uri),
                            );
                        } else {
                            locations.extend(
                                name_locations(
                                    ReadContext {
                                        path: target,
                                        ..context
                                    },
                                    &target_uri,
                                    &tree,
                                    snapshot.line_index(file),
                                    definition.start(),
                                    encoding,
                                    request,
                                )
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|location| location.uri != *uri),
                            );
                        }
                    }
                    locations.extend(sites.iter().filter(|(_, candidate)| candidate == id).map(
                        |(range, _)| {
                            Location::new(
                                uri.clone(),
                                text_range_to_lsp_range(line_index, *range, encoding),
                            )
                        },
                    ));
                }
            }
        }
        dedup_locations(&mut locations);
        Some(locations)
    }))
    .ok()
    .flatten()
}
