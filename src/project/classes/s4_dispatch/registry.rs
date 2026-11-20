use super::*;
use crate::rindex::provider::{package_indexed, resolve_origin};
use crate::rindex::schema::SymbolKind;
use crate::semantic::enclosure::Availability;
use crate::semantic::{LoadedPackage, PackageOrigin};

pub(super) struct S4Callable {
    signature: CallableSignature,
    package: Option<String>,
    pub(super) path: PathBuf,
    pub(super) binding: usize,
}

enum S4Origin {
    Package(String),
    Missing,
    Unknown,
}

#[derive(Clone, Copy)]
struct GenericLookup<'a> {
    deferred: bool,
    before: Option<&'a OopKey>,
    live: Option<(&'a S4DispatchContext, &'a BTreeSet<PathBuf>)>,
}

fn default_expression(source: &str) -> Option<S4GenericBody> {
    let parsed = crate::parser::parse(source);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    S4GenericBody::from_syntax(SyntaxElement::Node(parsed.cst))
}

fn formals_compatibility(new: &CallableSignature, old: &CallableSignature) -> Option<bool> {
    let names_match = |new: &[CallableParameter], old: &[CallableParameter]| {
        new.iter().map(|p| &p.name).eq(old.iter().map(|p| &p.name))
    };
    // R's .identicalGeneric allows the imported generic's extra trailing dots.
    // In this case it checks names alone, even if the other defaults differ.
    if old.parameters.len() == new.parameters.len() + 1
        && old.parameters.last().is_some_and(|p| p.name == "...")
        && names_match(&new.parameters, &old.parameters[..new.parameters.len()])
    {
        return Some(true);
    }
    if !names_match(&new.parameters, &old.parameters) {
        return Some(false);
    }
    let mut known = true;
    for (new, old) in new.parameters.iter().zip(&old.parameters) {
        let compatible = match (&new.default, &old.default) {
            (new, old) if new == old => Some(true),
            (Some(new), Some(old)) => default_expression(new)
                .zip(default_expression(old))
                .and_then(|(new, old)| new.compatibility(&old)),
            _ => Some(false),
        };
        match compatible {
            Some(false) => return Some(false),
            None => known = false,
            Some(true) => {}
        }
    }
    known.then_some(true)
}

fn reuse_generic(
    registry: &mut S4Registry,
    source: &OopId,
    def: &GenericDef,
    mut generic: S4Generic,
) -> S4Generic {
    // Reusing the object retains its original default method identity, too.
    generic.declaration = Some(source.clone());
    generic.uncertainty.extend(
        def.uncertainty
            .iter()
            .copied()
            .filter(|issue| *issue != OopUncertainty::UnresolvedCallee),
    );
    registry.generics.push(generic.clone());
    generic
}

fn builtin_primitive(generic: &S4Generic) -> bool {
    generic.object.is_none()
        && matches!(&generic.id, S4GenericId::Named { package, name }
            if builtins::callable(package, name).is_some_and(|callable| callable.kind == builtins::CallableKind::Primitive)
                || package == "base" && super::super::dispatch::primitive(name))
}

pub(super) fn s4_event_position(file: &ClassFile, key: &OopKey) -> Option<usize> {
    file.facts.oop.events.iter().position(|event| match event {
        OopEvent::Generic { target, .. }
        | OopEvent::Method(target)
        | OopEvent::S7Method(target)
        | OopEvent::RemoveS4Method(target) => target == key,
        _ => false,
    })
}

impl ClassIndex {
    fn s4_context_package(&self, path: &Path) -> Option<String> {
        let file = self.file(path)?;
        match &file.package_root {
            Some(root) => self.s3_origins.package_names.get(root).cloned(),
            None => Some(".GlobalEnv".into()),
        }
    }

    fn s4_owner(&self, file: &ClassFile, lexical: ClassOwner) -> S4MethodOwner {
        S4MethodOwner {
            path: file.path.clone(),
            package_root: file.package_root.clone(),
            package: file
                .package_root
                .as_ref()
                .and_then(|root| self.s3_origins.package_names.get(root))
                .cloned(),
            lexical,
        }
    }

    fn s4_binding_at(&self, path: &Path, name: &str, point: Option<&OopKey>) -> Option<&OopEvent> {
        let file = self.file(path)?;
        let end = match point {
            Some(key) => s4_event_position(file, key)?,
            None => file.facts.oop.events.len(),
        };
        file.facts.oop.events[..end]
            .iter()
            .rev()
            .find(|event| match event {
                OopEvent::Callable(index) => file.facts.oop.callables[*index].name == name,
                OopEvent::Generic {
                    name: binding,
                    target: key,
                } => {
                    matches!(binding, ClassName::Known(candidate) if candidate == name)
                        && point.map_or_else(
                            || key.owner.0.is_empty(),
                            |point| point.owner.0.starts_with(&key.owner.0),
                        )
                        && file.facts.oop.generics.iter().any(|def| {
                            def.key == *key
                                && def.binding_possible()
                                && self.generic_confirmed(path, def)
                                && self.s4_primitive_redeclaration(file, def).is_none()
                        })
                }
                OopEvent::Binding(binding)
                | OopEvent::UncertainGenerator { name: binding, .. }
                | OopEvent::Generator { name: binding, .. } => binding == name,
                OopEvent::InvalidateBindings => true,
                _ => false,
            })
    }

    fn s4_preceding_binding(
        &self,
        path: &Path,
        name: &str,
        point: Option<&OopKey>,
    ) -> Option<(&ClassFile, &OopEvent)> {
        let file = self.file(path)?;
        if let Some(binding) = self.s4_binding_at(path, name, point) {
            return Some((file, binding));
        }
        if point.is_none() || !self.class_file_complete(path) {
            return None;
        }
        let order = self
            .load_orders
            .get(file.package_root.as_ref()?)?
            .as_ref()?;
        let end = order.iter().position(|candidate| candidate == path)?;
        order[..end]
            .iter()
            .rev()
            .find_map(|path| Some((self.file(path)?, self.s4_binding_at(path, name, None)?)))
    }

    fn s4_final_bindings(&self, path: &Path, name: &str) -> Vec<(&ClassFile, &OopEvent)> {
        let Some(file) = self.file(path) else {
            return Vec::new();
        };
        if let Some(root) = &file.package_root {
            if let Some(order) = self.load_orders.get(root).and_then(Option::as_ref) {
                return order
                    .iter()
                    .rev()
                    .find_map(|path| {
                        Some((self.file(path)?, self.s4_binding_at(path, name, None)?))
                    })
                    .into_iter()
                    .collect();
            }
            // Without Collate, only a unique namespace writer can establish the
            // final value. Keep multiple writers instead of choosing a file.
            return self
                .oop_files()
                .filter(|candidate| candidate.package_root.as_ref() == Some(root))
                .filter_map(|file| Some((file, self.s4_binding_at(&file.path, name, None)?)))
                .collect();
        }
        self.s4_binding_at(path, name, None)
            .map(|binding| (file, binding))
            .into_iter()
            .collect()
    }

    pub(super) fn s4_callable_at(
        &self,
        path: &Path,
        source: &MethodImplementationSource,
        point: Option<&OopKey>,
    ) -> Option<S4Callable> {
        let (name, package) = match source {
            MethodImplementationSource::Binding { owner, name } if owner.0.is_empty() => {
                (name, None)
            }
            MethodImplementationSource::Reference { name } => (name, None),
            MethodImplementationSource::Qualified { package, name } => {
                (name, Some(package.as_str()))
            }
            _ => return None,
        };
        if package.is_none()
            && let Some((file, event)) = self.s4_preceding_binding(path, name, point)
        {
            let OopEvent::Callable(index) = event else {
                return None;
            };
            return Some(S4Callable {
                signature: file.facts.oop.callables[*index].signature.clone(),
                package: self.s4_context_package(&file.path),
                path: file.path.clone(),
                binding: *index,
            });
        }
        let mut files = self.implementation_files(path, name, package);
        if package.is_none() && point.is_some() {
            files.remove(path);
            if files.is_empty() {
                files.extend(self.imported_generic_files(path, name));
            }
        }
        let callables: Vec<_> = files
            .into_iter()
            .filter_map(|path| self.file(&path))
            .filter_map(|file| {
                let OopEvent::Callable(index) = self.s4_binding_at(&file.path, name, None)? else {
                    return None;
                };
                Some((file, *index))
            })
            .collect();
        let [(file, binding)] = callables.as_slice() else {
            return None;
        };
        Some(S4Callable {
            signature: file.facts.oop.callables[*binding].signature.clone(),
            package: self.s4_context_package(&file.path),
            path: file.path.clone(),
            binding: *binding,
        })
    }

    pub(crate) fn s7_method_callable_origin(
        &self,
        method: &OopId,
        source: &MethodImplementationSource,
    ) -> Option<(PathBuf, usize)> {
        if !matches!(
            source,
            MethodImplementationSource::Reference { .. }
                | MethodImplementationSource::Qualified { .. }
        ) {
            return None;
        }
        let callable = self.s4_callable_at(&method.path, source, Some(&method.key))?;
        let origin = self.file(&callable.path)?.facts.oop.callables[callable.binding].origin;
        Some((callable.path, origin))
    }

    fn s4_import_package(&self, path: &Path, name: &str) -> Option<String> {
        let file = self.file(path)?;
        let imports = self
            .s3_origins
            .namespaces
            .get(file.package_root.as_ref()?)?;
        if imports.unknown {
            return None;
        }
        // Resolve wildcard imports and Depends through the same exported files
        // used for callable lookup. Imports in DESCRIPTION alone expose no names.
        let packages = imports.named.get(name).cloned().unwrap_or_else(|| {
            let context = self.s4_context_package(path);
            self.implementation_files(path, name, None)
                .iter()
                .filter_map(|target| self.s4_context_package(target))
                .filter(|package| Some(package) != context.as_ref())
                .collect()
        });
        let mut packages = packages.iter();
        let package = packages.next()?;
        packages.next().is_none().then(|| package.clone())
    }

    fn s4_previous_generics(
        &self,
        source: &OopId,
        def: &GenericDef,
        package: Option<&str>,
    ) -> Vec<OopId> {
        let ClassName::Known(name) = &def.name else {
            return Vec::new();
        };
        // Implicit setGeneric bindings are absent from ordinary definition
        // files. The nearest preceding write in the shared namespace wins.
        if package.is_none()
            && let Some((file, event)) =
                self.s4_preceding_binding(&source.path, name, Some(&source.key))
        {
            return match event {
                OopEvent::Generic { target: key, .. } => vec![OopId {
                    path: file.path.clone(),
                    key: key.clone(),
                }],
                _ => Vec::new(),
            };
        }
        let mut files = self.implementation_files(&source.path, name, package);
        files.remove(&source.path);
        if files.is_empty() && package.is_none() {
            files.extend(self.imported_generic_files(&source.path, name));
        }
        files
            .iter()
            .filter_map(|path| self.file(path))
            .flat_map(|file| {
                file.facts
                    .oop
                    .generics
                    .iter()
                    .filter(move |generic| {
                        generic.key.system == OopSystem::S4
                            && generic.key.owner.0.is_empty()
                            && generic.name == def.name
                            && self.generic_confirmed(&file.path, generic)
                            && self.s4_binding_at(&file.path, name, None).is_some_and(|event| {
                                    matches!(event, OopEvent::Generic { target, .. } if target == &generic.key)
                            })
                    })
                    .map(move |generic| OopId {
                        path: file.path.clone(),
                        key: generic.key.clone(),
                    })
            })
            .collect()
    }

    /// Only a resolved function supplies an owner. Unknown imports or dynamic
    /// bindings keep the declaration identity instead of guessing a package.
    fn s4_external_origin(
        &self,
        source: &OopId,
        def: &GenericDef,
        resolution: &ClassResolution<'_>,
    ) -> S4Origin {
        let Some(file) = self.file(&source.path) else {
            return S4Origin::Unknown;
        };
        if let (Some(ClassName::Known(package)), ClassName::Known(name)) = (
            def.s4.as_ref().and_then(|s4| s4.package.as_ref()),
            &def.name,
        ) {
            // An explicit package looks up that namespace independently of
            // local bindings and imports. Names alone cannot prove compatibility.
            let loaded = [LoadedPackage {
                name: package.as_str().into(),
                range: TextRange::default(),
            }];
            let found = resolution
                .indexed
                .lookup(package, name)
                .is_some_and(|symbol| symbol.kind == SymbolKind::Function)
                || match resolve_origin(resolution.indexed, resolution.remote, name, &loaded) {
                    PackageOrigin::Resolved(owner) => owner == package.as_str(),
                    PackageOrigin::Ambiguous(owners) => owners.iter().any(|owner| owner == package),
                    PackageOrigin::Unknown => false,
                };
            return if found {
                S4Origin::Package(package.clone())
            } else {
                S4Origin::Missing
            };
        }
        let Some(FunctionRef::Bare {
            name,
            attached,
            attachment_uncertain: false,
        }) = def.s4.as_ref().and_then(|s4| s4.lookup.as_ref())
        else {
            return S4Origin::Unknown;
        };
        if !self.class_file_complete(&source.path)
            || self
                .s4_preceding_binding(&source.path, name, Some(&source.key))
                .is_some()
        {
            return S4Origin::Unknown;
        }
        let mut visible = self.implementation_files(&source.path, name, None);
        visible.remove(&source.path);
        visible.extend(self.imported_generic_files(&source.path, name));
        if !visible.is_empty() {
            return S4Origin::Unknown;
        }
        let origin = |packages: &[String]| {
            let loaded: Vec<_> = packages
                .iter()
                .map(|package| LoadedPackage {
                    name: package.as_str().into(),
                    range: TextRange::default(),
                })
                .collect();
            match resolve_origin(resolution.indexed, resolution.remote, name, &loaded) {
                PackageOrigin::Resolved(package) => Some(package.to_string()),
                PackageOrigin::Ambiguous(packages) => packages.last().map(ToString::to_string),
                PackageOrigin::Unknown => None,
            }
        };
        let mut loaded = Vec::new();
        if let Some(root) = &file.package_root {
            if let Some(imports) = self.s3_origins.namespaces.get(root) {
                if imports.unknown {
                    return S4Origin::Unknown;
                }
                if let Some(packages) = imports.named.get(name) {
                    return match packages.iter().collect::<Vec<_>>().as_slice() {
                        [package]
                            if package_indexed(resolution.indexed, resolution.remote, package)
                                && origin(std::slice::from_ref(*package)).as_ref()
                                    == Some(*package) =>
                        {
                            S4Origin::Package((*package).clone())
                        }
                        _ => S4Origin::Unknown,
                    };
                }
                let mut owners = BTreeSet::new();
                for (package, exclusions) in &imports.wildcard {
                    if exclusions.contains(name) {
                        continue;
                    }
                    if !package_indexed(resolution.indexed, resolution.remote, package) {
                        // Workspace exports have already been checked above.
                        if self
                            .s3_origins
                            .package_names
                            .values()
                            .any(|name| name == package)
                        {
                            continue;
                        }
                        return S4Origin::Unknown;
                    }
                    if origin(std::slice::from_ref(package)).as_ref() == Some(package) {
                        owners.insert(package.clone());
                    }
                }
                if !owners.is_empty() {
                    return if owners.len() == 1 {
                        S4Origin::Package(owners.into_iter().next().unwrap())
                    } else {
                        S4Origin::Unknown
                    };
                }
            }
            loaded.extend(
                self.s3_origins
                    .attached
                    .get(root)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        loaded.extend(attached.iter().cloned());
        if loaded.iter().any(|package| {
            !package_indexed(resolution.indexed, resolution.remote, package)
                && !self
                    .s3_origins
                    .package_names
                    .values()
                    .any(|name| name == package)
        }) {
            return S4Origin::Unknown;
        }
        origin(&loaded).map_or(S4Origin::Missing, S4Origin::Package)
    }

    fn s4_install_generic(
        &self,
        registry: &mut S4Registry,
        source: &OopId,
        resolution: &ClassResolution<'_>,
        visiting: &mut BTreeSet<OopId>,
    ) -> Option<S4Generic> {
        if let Some(generic) = registry
            .generics
            .iter()
            .find(|generic| generic.declaration.as_ref() == Some(source))
        {
            return Some(generic.clone());
        }
        if visiting.len() >= 64 || !visiting.insert(source.clone()) {
            return None;
        }
        let file = self.file(&source.path)?;
        let def = self.generic(source)?;
        if !self.generic_confirmed(&source.path, def) {
            return None;
        }
        let s4 = def.s4.as_ref()?;
        if let Some(builtin) = self.s4_primitive_redeclaration(file, def) {
            let generic = reuse_generic(registry, source, def, builtin.clone());
            visiting.remove(source);
            return Some(generic);
        }
        let context = self.s4_context_package(&file.path);
        let lookup_package = match &s4.package {
            Some(ClassName::Known(package)) if Some(package) != context.as_ref() => {
                Some(package.as_str())
            }
            _ => None,
        };
        let previous = self.s4_previous_generics(source, def, lookup_package);
        let existing = match previous.as_slice() {
            [previous] => self.s4_install_generic(registry, previous, resolution, visiting),
            _ => None,
        };
        // R also reuses an imported package's implicit generic when both def
        // and signature are NULL, even if other options were supplied.
        let imported_implicit = !s4.definition_supplied
            && !s4.dispatch_explicit
            && s4.package.is_none()
            && !def.uncertainty.contains(&OopUncertainty::CustomEnvironment)
            && existing.as_ref().is_some_and(|generic| {
                matches!((&generic.id, &context),
                    (S4GenericId::Named { package, .. }, Some(context)) if package != context)
            });
        if (s4.simple_redeclaration || imported_implicit)
            && let Some(generic) = existing.clone()
        {
            let generic = reuse_generic(registry, source, def, generic);
            visiting.remove(source);
            return Some(generic);
        }
        let previous_source = match (lookup_package, &def.name) {
            (Some(package), ClassName::Known(name)) => {
                Some(MethodImplementationSource::Qualified {
                    package: package.into(),
                    name: name.clone(),
                })
            }
            _ => s4.previous.clone(),
        };
        let previous_callable = previous_source
            .as_ref()
            .and_then(|previous| self.s4_callable_at(&file.path, previous, Some(&def.key)));
        let mut uncertainty = def.uncertainty.clone();
        uncertainty.remove(&OopUncertainty::UnresolvedCallee);
        let owner = self.s4_owner(file, def.key.owner.clone());
        let mut signature = def
            .signature
            .clone()
            .or_else(|| {
                (if s4.definition_supplied {
                    s4.definition.as_ref()
                } else {
                    previous_source.as_ref()
                })
                .and_then(|source| self.s4_callable_at(&file.path, source, Some(&def.key)))
                .map(|callable| callable.signature)
                .or_else(|| {
                    existing
                        .as_ref()
                        .filter(|_| !s4.definition_supplied)
                        .and_then(|generic| generic.signature.clone())
                })
                .or_else(|| {
                    if s4.definition_supplied {
                        return None;
                    }
                    // A default supplies the skeleton only when neither an
                    // existing function nor a generic already supplies one.
                    s4.default.signature.clone().or_else(|| {
                        s4.default
                            .implementation
                            .as_ref()
                            .and_then(|source| {
                                self.s4_callable_at(&file.path, source, Some(&def.key))
                            })
                            .map(|callable| callable.signature)
                    })
                })
            })
            .or_else(|| {
                if s4.definition_supplied {
                    return None;
                }
                let ClassName::Known(name) = &def.name else {
                    return None;
                };
                let S4Origin::Package(package) = self.s4_external_origin(source, def, resolution)
                else {
                    return None;
                };
                builtins::generics()
                    .iter()
                    .find(|generic| {
                        generic.id
                            == S4GenericId::Named {
                                package: package.clone(),
                                name: name.clone(),
                            }
                    })
                    .and_then(|generic| generic.signature.clone())
            });
        let skeleton = s4.skeleton.clone();
        let mut dispatch_arguments = if s4.dispatch_explicit {
            def.dispatch_arguments.clone()
        } else if let Some(signature) = &signature {
            s4_dispatch_arguments(signature)
        } else {
            uncertainty.insert(OopUncertainty::DynamicSignature);
            vec![ClassName::Unknown]
        };
        if !uncertainty.contains(&OopUncertainty::DynamicSignature)
            && let Some(signature) = &signature
        {
            let mut seen = BTreeSet::new();
            if dispatch_arguments.iter().any(|name| match name {
                ClassName::Known(name) => {
                    !seen.insert(name) || !signature.parameters.iter().any(|p| &p.name == name)
                }
                _ => true,
            }) {
                uncertainty.insert(OopUncertainty::InvalidArguments);
            }
        }
        if dispatch_arguments.len() > 1 {
            dispatch_arguments
                .retain(|name| !matches!(name, ClassName::Known(name) if name == "..."));
        }
        if dispatch_arguments.is_empty() && !uncertainty.contains(&OopUncertainty::DynamicSignature)
        {
            uncertainty.insert(OopUncertainty::InvalidArguments);
        }
        let mut default = if s4.default.kind == S4DefaultKind::Existing
            && let Some(existing) = &existing
        {
            existing.default.clone()
        } else {
            s4.default.clone()
        };
        if default.kind == S4DefaultKind::Existing && existing.is_none() {
            default.implementation = previous_source.clone();
        }
        if default.kind == S4DefaultKind::None
            && !s4.default_disabled
            && s4.standard_dispatch == Some(true)
        {
            let previous = existing
                .as_ref()
                .map(|generic| generic.default.clone())
                .or_else(|| {
                    previous_callable.as_ref().map(|callable| S4DefaultMethod {
                        kind: S4DefaultKind::Existing,
                        signature: Some(callable.signature.clone()),
                        implementation: previous_source.clone(),
                    })
                });
            if let Some(previous) = previous
                && signature
                    .as_ref()
                    .zip(previous.signature.as_ref())
                    .is_some_and(|(new, old)| {
                        new.parameters
                            .iter()
                            .map(|p| &p.name)
                            .eq(old.parameters.iter().map(|p| &p.name))
                    })
            {
                default = previous;
            }
        }
        if default.signature.is_none() {
            default.signature = default
                .implementation
                .as_ref()
                .and_then(|source| self.s4_callable_at(&file.path, source, Some(&def.key)))
                .map(|callable| callable.signature);
        }
        if default.kind != S4DefaultKind::None && default.signature.is_none() {
            uncertainty.insert(OopUncertainty::UnresolvedImplementation);
        }
        if s4
            .standard_factory
            .as_ref()
            .is_some_and(|helper| self.callee_identity(&file.path, helper).is_none())
        {
            if !s4.default_disabled {
                default.kind = S4DefaultKind::Unknown;
            }
            uncertainty.insert(OopUncertainty::UnresolvedCallee);
        }
        let mut groups = s4.groups.clone();
        if s4
            .group_factory
            .as_ref()
            .is_some_and(|helper| self.callee_identity(&file.path, helper).is_none())
        {
            groups = vec![ClassName::Unknown];
        }
        let mut supplied_owner = |package: Option<String>, compatible: Option<bool>| {
            if (!s4.definition_supplied && !s4.dispatch_explicit) || package == context {
                package
            } else {
                // R compares an imported generic's definition before retaining
                // its owner. Incompatible definitions create a local generic;
                // an unresolved skeleton cannot establish either identity.
                match compatible {
                    Some(true) => package,
                    Some(false) => context.clone(),
                    None => {
                        uncertainty.insert(OopUncertainty::UnresolvedImplementation);
                        None
                    }
                }
            }
        };
        let package = match &s4.package {
            Some(ClassName::Known(package)) if Some(package) == context.as_ref() => {
                Some(package.clone())
            }
            Some(ClassName::Unknown | ClassName::Absent) => None,
            None | Some(ClassName::Known(_)) => {
                if let Some(existing) = &existing {
                    let package = match &existing.id {
                        S4GenericId::Named { package, .. } => Some(package.clone()),
                        S4GenericId::Declaration(_) => None,
                    };
                    let compatible = signature
                        .as_ref()
                        .zip(existing.signature.as_ref())
                        .and_then(|(new, old)| {
                            if dispatch_arguments != existing.dispatch_arguments
                                || groups != existing.groups
                                || s4.simple_inheritance_only != existing.simple_inheritance_only
                                || lookup_package.is_some_and(|supplied| {
                                    package.as_deref().is_some_and(|owner| owner != supplied)
                                })
                            {
                                Some(false)
                            } else {
                                match formals_compatibility(new, old) {
                                    Some(true) => skeleton
                                        .as_ref()
                                        .zip(existing.skeleton.as_ref())
                                        .and_then(|(new, old)| new.compatibility(old)),
                                    other => other,
                                }
                            }
                        });
                    if compatible == Some(true) && package != context {
                        let generic = reuse_generic(registry, source, def, existing.clone());
                        visiting.remove(source);
                        return Some(generic);
                    }
                    supplied_owner(lookup_package.map(str::to_owned).or(package), compatible)
                } else if let Some(callable) = &previous_callable {
                    let compatible = signature.as_ref().and_then(|signature| {
                        if dispatch_arguments != s4_dispatch_arguments(&callable.signature)
                            || !groups.is_empty()
                        {
                            Some(false)
                        } else {
                            match formals_compatibility(signature, &callable.signature) {
                                Some(true) => skeleton
                                    .as_ref()
                                    .map(|skeleton| *skeleton == S4GenericSkeleton::Standard),
                                other => other,
                            }
                        }
                    });
                    if compatible == Some(true) && callable.package != context {
                        signature = Some(callable.signature.clone());
                        default = S4DefaultMethod {
                            kind: S4DefaultKind::Existing,
                            signature: signature.clone(),
                            implementation: previous_source.clone(),
                        };
                    }
                    supplied_owner(callable.package.clone(), compatible)
                } else {
                    match self.s4_external_origin(source, def, resolution) {
                        S4Origin::Package(package) => {
                            if !s4.definition_supplied && !s4.dispatch_explicit {
                                Some(package)
                            } else {
                                let imported_signature = match &def.name {
                                    ClassName::Known(name) => resolution
                                        .indexed
                                        .lookup(&package, name)
                                        .filter(|symbol| {
                                            (symbol.exported || lookup_package.is_some())
                                                && symbol.kind == SymbolKind::Function
                                        })
                                        .and_then(|symbol| symbol.formals.as_ref())
                                        .map(|formals| CallableSignature {
                                            parameters: formals
                                                .iter()
                                                .map(|formal| CallableParameter {
                                                    name: formal.name.to_string(),
                                                    default: formal.default.clone(),
                                                })
                                                .collect(),
                                        }),
                                    _ => None,
                                };
                                let compatible = imported_signature
                                    .as_ref()
                                    .zip(signature.as_ref())
                                    .and_then(|(old, new)| {
                                        if dispatch_arguments != s4_dispatch_arguments(old)
                                            || !groups.is_empty()
                                        {
                                            Some(false)
                                        } else {
                                            match formals_compatibility(new, old) {
                                                Some(true) => skeleton.as_ref().map(|skeleton| {
                                                    *skeleton == S4GenericSkeleton::Standard
                                                }),
                                                other => other,
                                            }
                                        }
                                    });
                                if compatible == Some(true) && Some(&package) != context.as_ref() {
                                    signature = imported_signature;
                                    if let ClassName::Known(name) = &def.name {
                                        default = S4DefaultMethod {
                                            kind: S4DefaultKind::Existing,
                                            signature: signature.clone(),
                                            implementation: Some(
                                                MethodImplementationSource::Qualified {
                                                    package: package.clone(),
                                                    name: name.clone(),
                                                },
                                            ),
                                        };
                                    }
                                }
                                supplied_owner(Some(package), compatible)
                            }
                        }
                        S4Origin::Missing if lookup_package.is_some() => {
                            lookup_package.map(str::to_owned)
                        }
                        S4Origin::Missing
                            if s4.definition_supplied
                                || s4.default.kind == S4DefaultKind::Explicit =>
                        {
                            context
                        }
                        _ => None,
                    }
                }
            }
        };
        let id = match (&def.name, package) {
            (ClassName::Known(name), Some(package))
                if def.key.owner.0.is_empty()
                    && (!def.uncertainty.contains(&OopUncertainty::CustomEnvironment)
                        || matches!(&s4.package, Some(ClassName::Known(_)))) =>
            {
                S4GenericId::Named {
                    package,
                    name: name.clone(),
                }
            }
            _ => S4GenericId::Declaration(source.clone()),
        };
        let generic = S4Generic {
            id: id.clone(),
            declaration: Some(source.clone()),
            object: Some(source.clone()),
            signature,
            skeleton,
            dispatch_arguments,
            groups,
            group: false,
            simple_inheritance_only: s4.simple_inheritance_only,
            default: default.clone(),
            uncertainty: uncertainty.clone(),
        };
        if default.kind != S4DefaultKind::None {
            let dispatch = s4_method_signature(&generic, &[]);
            registry.methods.push(S4Method {
                id: S4MethodId::Default(source.clone()),
                owner,
                generic: GenericRef {
                    system: OopSystem::S4,
                    owner: def.key.owner.clone(),
                    name: def.name.clone(),
                    package: None,
                    by_binding: false,
                    local: Some(def.key.clone()),
                    lookup: None,
                },
                generics: vec![id],
                supplied_signature: Vec::new(),
                dispatch: vec![dispatch],
                signature: default.signature.clone(),
                implementation: default.implementation.clone(),
                uncertainty,
            });
        }
        registry.generics.push(generic.clone());
        visiting.remove(source);
        Some(generic)
    }

    fn s4_primitive_redeclaration(
        &self,
        file: &ClassFile,
        def: &GenericDef,
    ) -> Option<&'static S4Generic> {
        let s4 = def.s4.as_ref()?;
        let ClassName::Known(name) = &def.name else {
            return None;
        };
        if !s4.simple_redeclaration && !s4.signature_only {
            return None;
        }
        builtins::generics().iter().find(|generic| {
            builtin_primitive(generic)
                && generic_name(&generic.id) == Some(name)
                && (s4.simple_redeclaration
                    || def.dispatch_arguments == generic.dispatch_arguments
                        && def.dispatch_factory.as_ref().is_none_or(|factory| {
                            self.callee_identity(&file.path, factory).is_some()
                        }))
        })
    }

    pub(in crate::project::classes) fn build_s4_registry(
        &self,
        resolution: &ClassResolution<'_>,
    ) -> S4Registry {
        let mut registry = S4Registry {
            generics: builtins::generics().to_vec(),
            ..S4Registry::default()
        };
        for file in self.oop_files() {
            for declaration in file
                .facts
                .oop
                .generics
                .iter()
                .filter(|def| def.key.system == OopSystem::S4)
            {
                self.s4_install_generic(
                    &mut registry,
                    &OopId {
                        path: file.path.clone(),
                        key: declaration.key.clone(),
                    },
                    resolution,
                    &mut BTreeSet::new(),
                );
            }
        }
        let registrations: Vec<_> = self
            .oop_files()
            .flat_map(|file| {
                file.facts
                    .oop
                    .methods
                    .iter()
                    .filter(|def| def.system == OopSystem::S4)
                    .map(move |declaration| {
                        (
                            file,
                            OopId {
                                path: file.path.clone(),
                                key: declaration.key.clone(),
                            },
                        )
                    })
            })
            .filter_map(|(file, source)| {
                let def = self.method(&source)?;
                if def.registration.as_ref().is_some_and(|callee| {
                    self.callee_availability(&file.path, callee)
                        == crate::semantic::enclosure::Availability::Absent
                }) {
                    return None;
                }
                Some((file, source, def))
            })
            .collect();
        // Discover promoted generics before linking registrations, so qualified
        // consumers do not depend on the order of workspace files.
        for (file, source, def) in &registrations {
            let Some(reference) = &def.generic else {
                continue;
            };
            if self
                .s4_generic_candidates_in(
                    &registry,
                    &file.path,
                    reference,
                    !def.key.owner.0.is_empty(),
                    Some(&def.key),
                )
                .is_empty()
                && !reference.by_binding
                && reference.local.is_none()
                && let ClassName::Known(name) = &reference.name
                && let Some(callable) = self.s4_callable_at(
                    &file.path,
                    &MethodImplementationSource::Reference { name: name.clone() },
                    Some(&def.key),
                )
            {
                let owner = self.s4_owner(file, def.key.owner.clone());
                let package = reference.package.clone().or(callable.package);
                let signature = callable.signature;
                let id = match package {
                    Some(package) if def.key.owner.0.is_empty() => S4GenericId::Named {
                        name: name.clone(),
                        package,
                    },
                    _ => S4GenericId::Declaration(source.clone()),
                };
                let default = S4DefaultMethod {
                    kind: S4DefaultKind::Existing,
                    signature: Some(signature.clone()),
                    implementation: Some(MethodImplementationSource::Reference {
                        name: name.clone(),
                    }),
                };
                let generic = S4Generic {
                    id: id.clone(),
                    declaration: Some(source.clone()),
                    object: Some(source.clone()),
                    dispatch_arguments: s4_dispatch_arguments(&signature),
                    signature: Some(signature.clone()),
                    skeleton: Some(S4GenericSkeleton::Standard),
                    groups: Vec::new(),
                    group: false,
                    simple_inheritance_only: Some(false),
                    default: default.clone(),
                    // The callable fixes the generic's selected arguments.
                    // Class vectors and implementation issues belong only to
                    // the registering method.
                    uncertainty: def
                        .uncertainty
                        .iter()
                        .copied()
                        .filter(|issue| {
                            matches!(
                                issue,
                                OopUncertainty::Conditional
                                    | OopUncertainty::CustomEnvironment
                                    | OopUncertainty::UnresolvedAlias
                            ) || *issue == OopUncertainty::UnresolvedCallee
                                && def.registration.as_ref().is_none_or(|callee| {
                                    self.callee_identity(&file.path, callee).is_none()
                                })
                        })
                        .collect(),
                };
                registry.methods.push(S4Method {
                    id: S4MethodId::Default(source.clone()),
                    owner,
                    generic: reference.clone(),
                    generics: vec![id],
                    supplied_signature: Vec::new(),
                    dispatch: vec![s4_method_signature(&generic, &[])],
                    signature: Some(signature),
                    implementation: default.implementation.clone(),
                    uncertainty: generic.uncertainty.clone(),
                });
                registry.generics.push(generic);
            }
        }
        // Explicit declarations are discovered first. Once promotions exist,
        // a simple setGeneric redeclaration must retain that object's methods
        // and its original default rather than create a replacement object.
        for index in 0..registry.generics.len() {
            let Some(source) = registry.generics[index].declaration.clone() else {
                continue;
            };
            let Some(def) = self
                .generic(&source)
                .filter(|def| def.s4.as_ref().is_some_and(|s4| s4.simple_redeclaration))
            else {
                continue;
            };
            let reference = GenericRef {
                system: OopSystem::S4,
                owner: source.key.owner.clone(),
                name: def.name.clone(),
                package: None,
                by_binding: false,
                local: None,
                lookup: None,
            };
            let candidates = self.s4_generic_candidates_in(
                &registry,
                &source.path,
                &reference,
                !source.key.owner.0.is_empty(),
                Some(&source.key),
            );
            let Some(existing) = candidates.first().filter(|existing| {
                existing
                    .object
                    .as_ref()
                    .is_some_and(|object| object.key.kind == OopKind::Method)
                    && existing.object != registry.generics[index].object
                    && candidates
                        .iter()
                        .all(|candidate| candidate.object == existing.object)
            }) else {
                continue;
            };
            let mut generic = (*existing).clone();
            generic.declaration = Some(source.clone());
            generic.uncertainty.extend(
                def.uncertainty
                    .iter()
                    .copied()
                    .filter(|issue| *issue != OopUncertainty::UnresolvedCallee),
            );
            registry.generics[index] = generic;
            registry
                .methods
                .retain(|method| method.id != S4MethodId::Default(source.clone()));
        }
        for (file, source, def) in registrations {
            let Some(reference) = &def.generic else {
                continue;
            };
            let candidates = self.s4_generic_candidates_in(
                &registry,
                &file.path,
                reference,
                !def.key.owner.0.is_empty(),
                Some(&def.key),
            );
            let generics = candidates
                .iter()
                .map(|generic| generic.id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let mut uncertainty = def.uncertainty.clone();
            if def
                .registration
                .as_ref()
                .is_some_and(|callee| self.callee_identity(&file.path, callee).is_some())
            {
                uncertainty.remove(&OopUncertainty::UnresolvedCallee);
            }
            if candidates.is_empty() {
                uncertainty.insert(OopUncertainty::UnprovenMethod);
            }
            let implementation = file
                .facts
                .oop
                .method_implementations
                .iter()
                .find(|implementation| implementation.method == def.key)
                .map(|implementation| implementation.source.clone());
            let signature = def.signature.clone().or_else(|| {
                implementation
                    .as_ref()
                    .and_then(|source| self.s4_callable_at(&file.path, source, Some(&def.key)))
                    .map(|callable| callable.signature)
            });
            if signature.is_some() {
                uncertainty.remove(&OopUncertainty::UnresolvedImplementation);
            }
            let dispatch = candidates
                .iter()
                .map(|generic| {
                    let mut dispatch = s4_method_signature(generic, &def.dispatch);
                    // conformMethod can rewrite dispatch for omitted formals.
                    // Until that conformance is known, this is not an exact
                    // registration signature, even with literal classes.
                    dispatch.complete &= signature.as_ref().is_some_and(|signature| {
                        dispatch.arguments.iter().all(|argument| {
                            argument.argument.as_ref().is_some_and(|name| {
                                signature
                                    .parameters
                                    .iter()
                                    .any(|formal| &formal.name == name)
                            })
                        })
                    });
                    dispatch
                })
                .collect();
            registry.methods.push(S4Method {
                id: S4MethodId::Registration(source),
                owner: self.s4_owner(file, def.key.owner.clone()),
                generic: reference.clone(),
                generics,
                supplied_signature: def.dispatch.clone(),
                dispatch,
                signature,
                implementation,
                uncertainty,
            });
        }
        self.s4_index_coercions(&mut registry);
        for file in self.oop_files() {
            for removal in &file.facts.oop.s4_removals {
                let mut uncertainty = removal.uncertainty.clone();
                match self.callee_availability(&file.path, &removal.factory) {
                    crate::semantic::enclosure::Availability::Absent => continue,
                    crate::semantic::enclosure::Availability::Present => {
                        uncertainty.remove(&OopUncertainty::UnresolvedCallee);
                    }
                    crate::semantic::enclosure::Availability::Unknown => {}
                }
                if uncertainty.contains(&OopUncertainty::InvalidArguments) {
                    continue;
                }
                let helper_known = removal
                    .dispatch_factory
                    .as_ref()
                    .is_none_or(|factory| self.callee_identity(&file.path, factory).is_some());
                let dispatch = self
                    .s4_generic_candidates_in(
                        &registry,
                        &file.path,
                        &removal.generic,
                        !removal.key.owner.0.is_empty(),
                        Some(&removal.key),
                    )
                    .into_iter()
                    .map(|generic| {
                        let mut dispatch = s4_method_signature(generic, &removal.dispatch);
                        dispatch.complete &= helper_known;
                        dispatch
                    })
                    .collect();
                registry.removals.push(S4Removal {
                    source: OopId {
                        path: file.path.clone(),
                        key: removal.key.clone(),
                    },
                    generic: removal.generic.clone(),
                    dispatch,
                    uncertainty,
                });
            }
        }
        registry
    }

    fn s4_index_coercions(&self, registry: &mut S4Registry) {
        let Some(generic) = builtins::generics().iter().find(|generic| {
            matches!(&generic.id, S4GenericId::Named { package, name }
                if package == "methods" && name == "coerce")
        }) else {
            return;
        };
        for file in self.oop_files() {
            for (ordinal, coercion) in file.facts.s4.coercions.iter().enumerate() {
                if !coercion.coerce
                    || coercion.definition_valid == Some(false)
                    || matches!((&coercion.from, &coercion.to),
                        (ClassName::Known(from), ClassName::Known(to)) if from == to)
                    || coercion
                        .uncertainty
                        .contains(&OopUncertainty::InvalidArguments)
                {
                    continue;
                }
                let mut uncertainty = coercion.uncertainty.clone();
                match self.callee_availability(&file.path, &coercion.factory) {
                    Availability::Absent => continue,
                    Availability::Unknown => {
                        uncertainty.insert(OopUncertainty::UnresolvedCallee);
                    }
                    Availability::Present => {}
                }
                let supplied_signature = [&coercion.from, &coercion.to]
                    .into_iter()
                    .map(|class| DispatchArgument {
                        argument: None,
                        class: match class {
                            ClassName::Known(name) if name == "ANY" => DispatchClass::Any,
                            ClassName::Known(name) if name == "missing" => DispatchClass::Missing,
                            ClassName::Known(name) => DispatchClass::Formal(name.clone()),
                            _ => DispatchClass::Unknown,
                        },
                    })
                    .collect::<Vec<_>>();
                let mut dispatch = s4_method_signature(generic, &supplied_signature);
                if coercion.definition_valid.is_none() {
                    dispatch.complete = false;
                    uncertainty.insert(OopUncertainty::UnresolvedImplementation);
                }
                registry.methods.push(S4Method {
                    id: S4MethodId::Registration(OopId {
                        path: file.path.clone(),
                        key: coercion.method_key(ordinal),
                    }),
                    owner: self.s4_owner(file, coercion.owner.clone()),
                    generic: GenericRef {
                        system: OopSystem::S4,
                        owner: coercion.owner.clone(),
                        name: ClassName::Known("coerce".into()),
                        package: Some("methods".into()),
                        by_binding: false,
                        local: None,
                        lookup: None,
                    },
                    generics: vec![generic.id.clone()],
                    supplied_signature,
                    dispatch: vec![dispatch],
                    signature: generic.signature.clone(),
                    implementation: None,
                    uncertainty,
                });
            }
        }
    }

    fn s4_declaration_visible_at(
        &self,
        path: &Path,
        candidate: &Path,
        declaration: &OopKey,
        point: &OopKey,
    ) -> bool {
        if candidate == path {
            return self.file(path).is_some_and(|file| {
                s4_event_position(file, declaration)
                    .zip(s4_event_position(file, point))
                    .is_some_and(|(installed, current)| installed < current)
            });
        }
        let order = self.file(path).and_then(|file| {
            let root = file.package_root.as_ref()?;
            (self.file(candidate)?.package_root.as_ref() == Some(root))
                .then(|| self.load_orders.get(root)?.as_ref())
                .flatten()
        });
        order.is_none_or(|order| {
            order
                .iter()
                .position(|file| file == candidate)
                .zip(order.iter().position(|file| file == path))
                .is_none_or(|(installed, current)| installed < current)
        })
    }

    fn s4_generic_candidates_in<'a>(
        &self,
        registry: &'a S4Registry,
        path: &Path,
        reference: &GenericRef,
        deferred: bool,
        point: Option<&OopKey>,
    ) -> Vec<&'a S4Generic> {
        self.s4_generic_candidates_inner(
            registry,
            path,
            reference,
            GenericLookup {
                deferred,
                before: point,
                live: None,
            },
            &mut std::collections::HashSet::new(),
        )
    }

    fn s4_generic_candidates_inner<'a>(
        &self,
        registry: &'a S4Registry,
        path: &Path,
        reference: &GenericRef,
        lookup: GenericLookup<'_>,
        visited: &mut std::collections::HashSet<(PathBuf, GenericRef, Option<OopKey>)>,
    ) -> Vec<&'a S4Generic> {
        let point = lookup.before;
        if visited.len() >= 64 || !visited.insert((path.into(), reference.clone(), point.cloned()))
        {
            return Vec::new();
        }
        if reference.system != OopSystem::S4 {
            return Vec::new();
        }
        let visible_now = |candidate: &Path, key: &OopKey| {
            lookup.live.is_none_or(|(context, loaded)| {
                self.s4_generic_activity(
                    context,
                    &OopId {
                        path: candidate.into(),
                        key: key.clone(),
                    },
                    loaded,
                ) != crate::semantic::enclosure::Availability::Absent
            })
        };
        let mut reference = reference;
        let mut point = point;
        let mut captured_callable = false;
        // Aliases may capture a setMethod installation rather than an explicit
        // setGeneric declaration. Follow registrations at their own events so
        // later replacements cannot change the captured object's identity.
        let mut followed = 0;
        while let Some(key) = reference
            .local
            .as_ref()
            .filter(|key| key.kind == OopKind::Method)
        {
            if followed >= 64 {
                return Vec::new();
            }
            followed += 1;
            let source = OopId {
                path: path.into(),
                key: key.clone(),
            };
            let Some(method) = self.method(&source).filter(|_| visible_now(path, key)) else {
                return Vec::new();
            };
            if method
                .registration
                .as_ref()
                .is_none_or(|callee| self.callee_identity(path, callee).is_none())
            {
                return Vec::new();
            }
            if let Some(generic) = registry
                .generics
                .iter()
                .find(|generic| generic.declaration.as_ref() == Some(&source))
            {
                return vec![generic];
            }
            let Some(previous) = method.generic.as_ref() else {
                return Vec::new();
            };
            if let ClassName::Known(name) = &previous.name {
                captured_callable |= self
                    .s4_callable_at(
                        path,
                        &MethodImplementationSource::Reference { name: name.clone() },
                        Some(&method.key),
                    )
                    .is_some();
            }
            reference = previous;
            point = Some(&method.key);
        }
        let mut declarations =
            self.generic_candidates_where(path, reference, lookup.deferred, |candidate, def| {
                visible_now(candidate, &def.key)
                    && point.is_none_or(|point| {
                        self.s4_declaration_visible_at(path, candidate, &def.key, point)
                    })
            });
        if reference.local.is_none()
            && reference.package.is_none()
            && !reference.by_binding
            && point.is_some()
            && let ClassName::Known(name) = &reference.name
            && let Some((_, binding)) = self.s4_preceding_binding(path, name, point)
        {
            // A later ordinary write starts a new promotion. Competing explicit
            // declarations still retain their separate candidate identities.
            if !matches!(binding, OopEvent::Generic { .. }) {
                declarations.clear();
            }
        }
        if !declarations.is_empty() {
            let declared: Vec<_> = registry
                .generics
                .iter()
                .filter(|generic| {
                    generic
                        .declaration
                        .as_ref()
                        .is_some_and(|source| declarations.contains(source))
                        && !(reference.by_binding
                            && reference.package.is_some()
                            && builtin_primitive(generic))
                })
                .collect();
            if !declared.is_empty() {
                return declared;
            }
        }
        if reference.local.is_some() {
            return Vec::new();
        }
        let ClassName::Known(name) = &reference.name else {
            return Vec::new();
        };
        if reference.by_binding
            && let Some(package) = reference.package.as_deref()
            && matches!(package, "base" | "methods")
        {
            let Some(callable) = builtins::callable(package, name) else {
                return Vec::new();
            };
            return registry
                .generics
                .iter()
                .filter(|generic| {
                    generic.declaration.is_none()
                        && matches!(&generic.id, S4GenericId::Named { package, name }
                        if package == callable.package && name == callable.name)
                })
                .collect();
        }
        if reference.package.is_some() || self.s4_preceding_binding(path, name, point).is_none() {
            let mut imported = Vec::new();
            for provider in self.s4_method_files(path, name, reference.package.as_deref()) {
                let Some(file) = self.file(&provider) else {
                    continue;
                };
                // Function-valued lookups must respect the provider's final
                // binding. Its own writes are handled by declaration/promotion
                // lookup, rather than reviving a retired object via metadata.
                let bindings = if reference.by_binding {
                    self.s4_final_bindings(&provider, name)
                } else {
                    Vec::new()
                };
                if !bindings.is_empty() {
                    let callable_source =
                        MethodImplementationSource::Reference { name: name.clone() };
                    if let [(binding_file, OopEvent::Callable(binding))] = bindings.as_slice() {
                        // exportMethods also exports a function promoted by
                        // setMethod, provided that promotion still owns the
                        // final callable binding.
                        imported.extend(registry.generics.iter().filter(|generic| {
                            generic.declaration.as_ref().is_some_and(|id| {
                                id.key.kind == OopKind::Method
                                    && visible_now(&id.path, &id.key)
                                    && id.key.owner.0.is_empty()
                                    && self.file(&id.path).is_some_and(|promotion| {
                                        promotion.package_root == file.package_root
                                    })
                                    && self
                                        .method(id)
                                        .is_some_and(|method| method.name == reference.name)
                                    && self
                                        .s4_callable_at(&id.path, &callable_source, Some(&id.key))
                                        .is_some_and(|previous| {
                                            previous.path == binding_file.path
                                                && previous.binding == *binding
                                        })
                            })
                        }));
                    }
                    continue;
                }
                if reference.by_binding && reference.package.is_some() {
                    continue;
                }
                let registrations = file
                    .facts
                    .oop
                    .methods
                    .iter()
                    .filter(|method| {
                        method.system == OopSystem::S4
                            && method.key.owner.0.is_empty()
                            && method
                                .generic
                                .as_ref()
                                .is_some_and(|generic| generic.name == reference.name)
                    })
                    .collect::<Vec<_>>();
                if registrations.is_empty() {
                    let forwarded = GenericRef {
                        owner: ClassOwner::default(),
                        package: None,
                        by_binding: false,
                        local: None,
                        ..reference.clone()
                    };
                    imported.extend(self.s4_generic_candidates_inner(
                        registry,
                        &provider,
                        &forwarded,
                        GenericLookup {
                            deferred: true,
                            before: None,
                            ..lookup
                        },
                        visited,
                    ));
                } else {
                    for method in registrations {
                        imported.extend(self.s4_generic_candidates_inner(
                            registry,
                            &provider,
                            method.generic.as_ref().unwrap(),
                            GenericLookup {
                                deferred: false,
                                before: Some(&method.key),
                                ..lookup
                            },
                            visited,
                        ));
                    }
                }
            }
            // Primitive method tables do not export a function-valued generic
            // in the provider's namespace. String-based method imports still
            // reach the original table.
            if reference.by_binding {
                imported.retain(|generic| !builtin_primitive(generic));
            }
            if !imported.is_empty() {
                let mut seen = BTreeSet::new();
                imported.retain(|generic| {
                    seen.insert((
                        generic.id.clone(),
                        generic.declaration.clone(),
                        generic.object.clone(),
                    ))
                });
                return imported;
            }
        }
        if reference.by_binding
            && let Some(package) = &reference.package
            && let Some(reexports) = self.s4_reexports(package, name)
            && let Some(root) = self.s4_workspace_root(package)
            && self
                .oop_files()
                .find(|file| file.package_root.as_ref() == Some(root))
                .is_some_and(|file| self.s4_final_bindings(&file.path, name).is_empty())
        {
            let mut seen = BTreeSet::new();
            let reexported: Vec<_> = reexports
                .bindings
                .into_iter()
                .flat_map(|(package, name)| {
                    let reference = GenericRef {
                        package: Some(package),
                        name: ClassName::Known(name),
                        ..reference.clone()
                    };
                    self.s4_generic_candidates_inner(
                        registry,
                        path,
                        &reference,
                        GenericLookup {
                            deferred: true,
                            before: None,
                            ..lookup
                        },
                        visited,
                    )
                })
                .filter(|generic| reexports.value_export || !builtin_primitive(generic))
                .filter(|generic| seen.insert(generic.table()))
                .collect();
            if !reexported.is_empty() {
                return reexported;
            }
        }
        // Qualified objects require exported values even if their method table
        // is visible through a registration or a methods import.
        if reference.by_binding
            && reference.package.as_deref().is_none_or(|package| {
                self.implementation_files(path, name, Some(package))
                    .is_empty()
            })
        {
            return Vec::new();
        }
        let callable_source = match &reference.package {
            Some(package) => MethodImplementationSource::Qualified {
                package: package.clone(),
                name: name.clone(),
            },
            None => MethodImplementationSource::Reference { name: name.clone() },
        };
        let callable = self.s4_callable_at(path, &callable_source, point);
        let package = reference
            .package
            .clone()
            .or_else(|| {
                callable
                    .as_ref()
                    .and_then(|callable| callable.package.clone())
            })
            .or_else(|| self.s4_import_package(path, name))
            .or_else(|| self.s4_context_package(path));
        let implicit: Vec<_> = registry
            .generics
            .iter()
            .filter(|generic| {
                let Some(id) = &generic.declaration else {
                    return false;
                };
                let Some(current) = &callable else {
                    return false;
                };
                if id.key.kind != OopKind::Method
                    || !visible_now(&id.path, &id.key)
                    || !self
                        .s4_callable_at(
                            &id.path,
                            &MethodImplementationSource::Reference { name: name.clone() },
                            Some(&id.key),
                        )
                        .is_some_and(|previous| {
                            previous.path == current.path && previous.binding == current.binding
                        })
                {
                    return false;
                }
                // Linking happens after discovery, so a later registration may
                // already have promoted a different binding in the same file.
                if let Some(point) = point
                    && id.path == path
                    && !self.file(path).is_some_and(|file| {
                        s4_event_position(file, &id.key)
                            .zip(s4_event_position(file, point))
                            .is_some_and(|(installed, current)| installed <= current)
                    })
                {
                    return false;
                }
                if reference.by_binding
                    && !reference.package.as_ref().is_some_and(|package| {
                        self.s4_context_package(&id.path).as_ref() == Some(package)
                    })
                {
                    return false;
                }
                if id.path != path
                    && !self.sees_file(path, &id.path)
                    && reference.package.is_none()
                    && self.s4_import_package(path, name).is_none()
                {
                    return false;
                }
                match (&generic.id, &package) {
                    (
                        S4GenericId::Named {
                            package: actual,
                            name: actual_name,
                        },
                        Some(package),
                    ) => actual == package && actual_name == name,
                    (S4GenericId::Declaration(id), _) => {
                        reference.owner.0.starts_with(&id.key.owner.0)
                            && self
                                .method(id)
                                .is_some_and(|method| method.name == reference.name)
                    }
                    _ => false,
                }
            })
            .collect();
        if !implicit.is_empty() {
            return implicit;
        }
        // Unresolved objects cannot acquire a group identity from their name.
        if (reference.by_binding && reference.package.is_none()) || captured_callable {
            return Vec::new();
        }
        if !reference.by_binding
            && reference.package.is_none()
            && point.is_none()
            && let Some((context, loaded)) = lookup.live
        {
            // String lookup can consult S4 metadata in loaded namespaces and
            // sourced files even when there is no visible function binding.
            let mut seen = BTreeSet::new();
            let installed: Vec<_> = registry
                .generics
                .iter()
                .filter(|generic| generic_name(&generic.id) == Some(name))
                .filter(|generic| {
                    generic.declaration.as_ref().is_some_and(|id| {
                        id.key.owner.0.is_empty()
                            && visible_now(&id.path, &id.key)
                            && (self
                                .file(&id.path)
                                .and_then(|file| file.package_root.as_ref())
                                .is_some_and(|root| loaded.contains(root))
                                || context.execution.as_ref().is_some_and(|execution| {
                                    execution.source_activity(&id.path)
                                        != crate::semantic::enclosure::Availability::Absent
                                }))
                    })
                })
                .filter(|generic| seen.insert(generic.table()))
                .collect();
            if !installed.is_empty() {
                return installed;
            }
        }
        // String lookup still finds registered group objects through an
        // ordinary same-named binding. Replacement S4 declarations were
        // resolved above and take precedence over this fallback.
        registry
            .generics
            .iter()
            .filter(|generic| {
                !reference.by_binding && generic.declaration.is_none()
                    && matches!(&generic.id, S4GenericId::Named { name: candidate, package }
                    if candidate == name && reference.package.as_ref().is_none_or(|wanted| wanted == package))
            })
            .collect()
    }

    pub(super) fn s4_generic_candidates_at(
        &self,
        context: &S4DispatchContext,
        reference: &GenericRef,
        loaded: &BTreeSet<PathBuf>,
    ) -> Vec<&S4Generic> {
        self.s4_generic_candidates_inner(
            &self.s4,
            &context.path,
            reference,
            GenericLookup {
                deferred: true,
                before: None,
                live: Some((context, loaded)),
            },
            &mut std::collections::HashSet::new(),
        )
    }

    pub fn s4_generic_candidates(
        &self,
        path: &Path,
        reference: &GenericRef,
        deferred: bool,
    ) -> Vec<&S4Generic> {
        self.s4_generic_candidates_in(&self.s4, path, reference, deferred, None)
    }
}
