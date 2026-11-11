//! Pure project resolution over range-free declaration facts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rowan::TextRange;
use smol_str::SmolStr;

use super::*;

mod namespace;
mod oop;
use crate::project::{PackageDeclarations, ProjectScope, ReadBinding, ReadSite};
use crate::rindex::provider::{IndexedProvider, package_indexed, resolve_origin};
use crate::rindex::remote::RemoteExports;
use crate::semantic::{LoadedPackage, PackageOrigin};

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct ClassFile {
    pub path: PathBuf,
    pub package_root: Option<PathBuf>,
    pub facts: FileClassFacts,
    pub exports: Arc<BTreeSet<String>>,
}

pub struct ClassResolution<'a> {
    pub scope: &'a ProjectScope,
    pub namespaces: &'a [(PathBuf, String)],
    pub package_names: &'a BTreeMap<PathBuf, String>,
    pub packages: &'a [PackageDeclarations],
    pub indexed: &'a IndexedProvider,
    pub remote: &'a RemoteExports,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct ResolvedClass {
    pub declaration: ClassDef,
    pub package_root: Option<PathBuf>,
    pub fingerprint: u64,
}

/// All declaration sites survive aggregation. Only proven edges enter the graph;
/// `parent_candidates` retains ambiguity for consumers that can present it.
#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct ClassIndex {
    pub declarations: BTreeMap<ClassId, ResolvedClass>,
    pub unresolved: BTreeSet<ClassId>,
    pub parent_candidates: BTreeMap<ClassId, Vec<Vec<ClassId>>>,
    pub supertypes: BTreeMap<ClassId, BTreeSet<ClassId>>,
    pub subtypes: BTreeMap<ClassId, BTreeSet<ClassId>>,
    files: BTreeMap<PathBuf, ClassFile>,
    visible: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    namespace: namespace::NamespaceVisibility,
    generic_sources: BTreeMap<(PathBuf, bool, String), BTreeSet<PathBuf>>,
    incomplete: BTreeSet<PathBuf>,
    generator_sources: BTreeMap<(PathBuf, bool, String), BTreeSet<PathBuf>>,
    callees: BTreeMap<(PathBuf, FunctionRef), FunctionIdentity>,
}

impl ClassIndex {
    /// All OOP source facts share the class projection and its package owner.
    /// These include unconfirmed declarations; consumers must inspect their
    /// provenance and uncertainty before treating them as resolved definitions.
    pub fn file(&self, path: &Path) -> Option<&ClassFile> {
        self.files.get(path)
    }

    pub fn oop_files(&self) -> impl Iterator<Item = &ClassFile> {
        self.files.values()
    }

    /// Confirm a symbolic callee using this file's imports and search path.
    /// An unresolved alias never acquires a definite origin from its name.
    pub fn callee_identity(
        &self,
        path: &Path,
        reference: &FunctionRef,
    ) -> Option<&FunctionIdentity> {
        self.callees.get(&(path.to_path_buf(), reference.clone()))
    }

    pub fn build(files: Vec<ClassFile>, resolution: &ClassResolution<'_>) -> Self {
        let mut index = Self {
            files: files
                .into_iter()
                .map(|file| (file.path.clone(), file))
                .collect(),
            ..Self::default()
        };
        index.namespace = namespace::NamespaceVisibility::build(&index.files, resolution);
        let generator_names: BTreeSet<_> = index
            .files
            .values()
            .flat_map(|file| file.facts.generators.iter())
            .map(|binding| binding.name.clone())
            .collect();
        let generic_names: BTreeSet<_> = index
            .files
            .values()
            .flat_map(|file| {
                file.facts
                    .oop
                    .generic_bindings
                    .iter()
                    .map(|binding| binding.name.clone())
                    .chain(
                        file.facts
                            .oop
                            .generics
                            .iter()
                            .filter_map(|def| def.binding.clone()),
                    )
            })
            .collect();
        for file in index.files.values() {
            if resolution.scope.for_file(&file.path).resolution_incomplete
                || !resolution.scope.package_complete(&file.path)
            {
                index.incomplete.insert(file.path.clone());
            }
            for name in &generic_names {
                for in_function in [false, true] {
                    index.generic_sources.insert(
                        (file.path.clone(), in_function, name.clone()),
                        definition_files(file, name, in_function, &index.files, resolution.scope),
                    );
                }
            }
            let references = file
                .facts
                .declarations
                .iter()
                .map(|def| &def.factory)
                .chain(
                    file.facts
                        .declarations
                        .iter()
                        .filter_map(|def| def.parent_vector.as_ref()),
                )
                .chain(file.facts.oop.generics.iter().map(|def| &def.factory))
                .chain(
                    file.facts
                        .oop
                        .methods
                        .iter()
                        .filter_map(|def| def.registration.as_ref()),
                )
                .chain(
                    file.facts
                        .oop
                        .member_groups
                        .iter()
                        .filter_map(|group| group.factory.as_ref()),
                );
            for reference in references {
                for identity in possible_functions(reference) {
                    if confirms_function(
                        file,
                        reference,
                        &identity.package,
                        &identity.name,
                        &index.files,
                        resolution,
                    ) {
                        index
                            .callees
                            .insert((file.path.clone(), reference.clone()), identity);
                    }
                }
            }
            index.visible.insert(
                file.path.clone(),
                resolution
                    .scope
                    .sees(&file.path)
                    .iter()
                    .map(Path::to_path_buf)
                    .collect(),
            );
            for name in &generator_names {
                for in_function in [false, true] {
                    index.generator_sources.insert(
                        (file.path.clone(), in_function, name.clone()),
                        definition_files(file, name, in_function, &index.files, resolution.scope),
                    );
                }
            }
            let fingerprint = file.facts.fingerprint();
            for def in &file.facts.declarations {
                let id = ClassId {
                    path: file.path.clone(),
                    key: def.key.clone(),
                };
                if index
                    .callee_identity(&file.path, &def.factory)
                    .is_some_and(|identity| {
                        identity.package == def.system().package()
                            && identity.name == def.system().callee()
                    })
                {
                    index.declarations.insert(
                        id,
                        ResolvedClass {
                            declaration: def.clone(),
                            package_root: file.package_root.clone(),
                            fingerprint,
                        },
                    );
                } else {
                    index.unresolved.insert(id);
                }
            }
        }
        for (id, class) in &index.declarations {
            let def = &class.declaration;
            let deferred = def.system() == ClassSystem::R6 || !id.key.owner.0.is_empty();
            let candidates: Vec<_> = def
                .parents
                .iter()
                .map(|parent| index.resolve_reference(&id.path, &id.key.owner, parent, deferred))
                .collect();
            index
                .parent_candidates
                .insert(id.clone(), candidates.clone());
            let vector_known = def.parent_vector.as_ref().is_none_or(|vector| {
                confirms_function(
                    &index.files[&id.path],
                    vector,
                    "base",
                    "c",
                    &index.files,
                    resolution,
                )
            });
            if !def.binding_known() || !vector_known {
                continue;
            }
            for (reference, parents) in def.parents.iter().zip(candidates) {
                if !index.reference_known(&id.path, &id.key.owner, reference, deferred)
                    || resolution.scope.for_file(&id.path).resolution_incomplete
                {
                    continue;
                }
                if let [parent] = parents.as_slice()
                    && (parent.path == id.path || resolution.scope.package_complete(&id.path))
                    && index.declarations.get(parent).is_some_and(|class| {
                        class.declaration.binding_known()
                            && match def.system() {
                                ClassSystem::R6 | ClassSystem::S7 => {
                                    class.declaration.system() == def.system()
                                }
                                _ => matches!(
                                    class.declaration.system(),
                                    ClassSystem::S4 | ClassSystem::RefClass
                                ),
                            }
                    })
                {
                    index
                        .supertypes
                        .entry(id.clone())
                        .or_default()
                        .insert(parent.clone());
                    index
                        .subtypes
                        .entry(parent.clone())
                        .or_default()
                        .insert(id.clone());
                }
            }
        }
        index
    }

    fn imported_binding_files(&self, path: &Path, name: &str) -> BTreeSet<PathBuf> {
        if self.incomplete.contains(path)
            || std::iter::once(path)
                .chain(
                    self.visible
                        .get(path)
                        .into_iter()
                        .flatten()
                        .map(PathBuf::as_path),
                )
                .filter_map(|path| self.files.get(path))
                .any(|file| file.exports.contains(name))
        {
            return BTreeSet::new();
        }
        self.namespace.generic_files(path, name, None)
    }

    fn reference_known(
        &self,
        path: &Path,
        owner: &ClassOwner,
        reference: &ParentRef,
        deferred: bool,
    ) -> bool {
        match reference {
            ParentRef::Generator(GeneratorRef::Free(name)) => {
                self.generator_sources
                    .get(&(path.to_path_buf(), deferred, name.clone()))
                    .is_some_and(|sources| sources.len() == 1)
                    || self.imported_binding_files(path, name).len() == 1
            }
            ParentRef::Generator(GeneratorRef::Qualified { .. }) => {
                self.resolve_reference(path, owner, reference, deferred)
                    .len()
                    == 1
            }
            ParentRef::Unknown | ParentRef::Generator(GeneratorRef::Unknown) => false,
            _ => true,
        }
    }

    pub fn resolve_reference(
        &self,
        path: &Path,
        owner: &ClassOwner,
        reference: &ParentRef,
        in_function: bool,
    ) -> Vec<ClassId> {
        match reference {
            ParentRef::Generator(GeneratorRef::Local(key)) => {
                let id = ClassId {
                    path: path.to_path_buf(),
                    key: key.clone(),
                };
                self.files
                    .get(path)
                    .is_some_and(|file| file.facts.declaration(key).is_some())
                    .then_some(id)
                    .into_iter()
                    .collect()
            }
            ParentRef::Generator(GeneratorRef::Free(name)) => {
                let Some(sources) =
                    self.generator_sources
                        .get(&(path.to_path_buf(), in_function, name.clone()))
                else {
                    return Vec::new();
                };
                let mut sources = sources.clone();
                if sources.is_empty() && !self.incomplete.contains(path) {
                    sources.extend(self.imported_binding_files(path, name));
                }
                sources
                    .iter()
                    .flat_map(|path| {
                        self.files.get(path).into_iter().flat_map(move |file| {
                            file.facts
                                .generators
                                .iter()
                                .filter(move |binding| {
                                    binding.owner.0.is_empty() && &binding.name == name
                                })
                                .map(move |binding| ClassId {
                                    path: path.clone(),
                                    key: binding.target.clone(),
                                })
                        })
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            }
            ParentRef::Formal(name) => {
                let Some(file) = self.files.get(path) else {
                    return Vec::new();
                };
                // Formal classes and Reference Classes share R's formal class
                // registry. R6's string label never participates in that lookup.
                for depth in (1..=owner.0.len()).rev() {
                    let local: Vec<_> = file
                        .facts
                        .declarations
                        .iter()
                        .filter(|def| {
                            matches!(def.system(), ClassSystem::S4 | ClassSystem::RefClass)
                                && def.name == ClassName::Known(name.clone())
                                && def.key.owner.0 == owner.0[..depth]
                        })
                        .map(|def| ClassId {
                            path: path.to_path_buf(),
                            key: def.key.clone(),
                        })
                        .filter(|id| self.declarations.contains_key(id))
                        .collect();
                    if !local.is_empty() {
                        return local;
                    }
                }
                std::iter::once(file)
                    .chain(
                        self.visible
                            .get(path)
                            .into_iter()
                            .flatten()
                            .flat_map(|path| self.files.get(path)),
                    )
                    .chain(
                        self.namespace
                            .class_files(path, name)
                            .iter()
                            .filter_map(|path| self.files.get(path)),
                    )
                    .flat_map(|file| {
                        file.facts
                            .declarations
                            .iter()
                            .filter(|def| {
                                matches!(def.system(), ClassSystem::S4 | ClassSystem::RefClass)
                                    && def.key.owner.0.is_empty()
                                    && def.name == ClassName::Known(name.clone())
                            })
                            .map(|def| ClassId {
                                path: file.path.clone(),
                                key: def.key.clone(),
                            })
                    })
                    .filter(|id| self.declarations.contains_key(id))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            }
            ParentRef::Generator(GeneratorRef::Qualified { package, name }) => self
                .namespace
                .generic_files(path, name, Some(package))
                .iter()
                .filter_map(|path| self.files.get(path))
                .flat_map(|file| {
                    file.facts
                        .generators
                        .iter()
                        .filter(|binding| binding.owner.0.is_empty() && &binding.name == name)
                        .map(|binding| ClassId {
                            path: file.path.clone(),
                            key: binding.target.clone(),
                        })
                })
                .collect(),
            ParentRef::Generator(GeneratorRef::Unknown) | ParentRef::Unknown => Vec::new(),
        }
    }
}

fn definition_files(
    file: &ClassFile,
    name: &str,
    in_function: bool,
    files: &BTreeMap<PathBuf, ClassFile>,
    scope: &ProjectScope,
) -> BTreeSet<PathBuf> {
    let visible: BTreeSet<_> = scope
        .sees(&file.path)
        .iter()
        .filter(|path| {
            files
                .get(*path)
                .is_some_and(|file| file.exports.contains(name))
        })
        .map(Path::to_path_buf)
        .collect();
    if scope.for_file(&file.path).resolution_incomplete {
        return BTreeSet::new();
    }
    let read = if in_function {
        match scope.final_scope_binding(&file.path, name) {
            ReadSite::Bound(path) => ReadBinding::Resolved(path),
            ReadSite::Unbound => ReadBinding::Unresolved,
            ReadSite::Unknown => ReadBinding::OrderUnknown,
        }
    } else {
        scope.top_level_read_binding(&file.path, name)
    };
    match read {
        ReadBinding::Resolved(path) => visible
            .contains(&path)
            .then_some(path)
            .into_iter()
            .collect(),
        ReadBinding::OrderUnknown => BTreeSet::new(),
        ReadBinding::Unresolved => visible
            .into_iter()
            .filter(|path| {
                file.package_root.is_some() && files[path].package_root == file.package_root
            })
            .collect(),
        ReadBinding::NoTopLevelRead => visible,
    }
}

fn confirms_function(
    file: &ClassFile,
    reference: &FunctionRef,
    package: &str,
    name: &str,
    files: &BTreeMap<PathBuf, ClassFile>,
    resolution: &ClassResolution<'_>,
) -> bool {
    match reference {
        FunctionRef::Qualified {
            package: actual_package,
            name: actual_name,
        } => actual_package == package && actual_name == name,
        FunctionRef::Unresolved(_) => false,
        FunctionRef::Bare {
            name: actual_name,
            attached,
            attachment_uncertain,
        } => {
            if *attachment_uncertain
                || actual_name != name
                || resolution.scope.for_file(&file.path).resolution_incomplete
            {
                return false;
            }
            // A workspace binding wins over imports and the search path, even
            // when its value is not a class factory we know how to summarize.
            if resolution.scope.sees(&file.path).iter().any(|path| {
                files.get(path).is_some_and(|file| {
                    file.exports.iter().any(|export| unbacktick(export) == name)
                        || (!attached.is_empty()
                            && (file.exports.contains("library")
                                || file.exports.contains("require")))
                })
            }) {
                return false;
            }
            if let Some(root) = &file.package_root
                && let Some((_, text)) = resolution.namespaces.iter().find(|(path, _)| path == root)
            {
                let namespace = crate::namespace::parse(text);
                if !namespace.diagnostics.is_empty() {
                    return false;
                }
                let mut imports = BTreeSet::new();
                let mut wildcard = Vec::new();
                for directive in namespace.document().directives() {
                    let args: Vec<_> = directive.arguments().collect();
                    let Some(source) = args
                        .first()
                        .and_then(|arg| arg.value())
                        .and_then(namespace_name)
                    else {
                        continue;
                    };
                    match directive.kind() {
                        crate::namespace::DirectiveKind::ImportFrom => {
                            if args
                                .iter()
                                .skip(1)
                                .filter_map(|arg| arg.value())
                                .filter_map(namespace_name)
                                .any(|imported| imported == name)
                            {
                                if directive
                                    .syntax()
                                    .ancestors()
                                    .any(|node| node.kind() == SyntaxKind::IF_EXPR)
                                {
                                    return false;
                                }
                                imports.insert(source);
                            }
                        }
                        crate::namespace::DirectiveKind::Import => {
                            if directive
                                .syntax()
                                .ancestors()
                                .any(|node| node.kind() == SyntaxKind::IF_EXPR)
                            {
                                return false;
                            }
                            for source in args
                                .iter()
                                .filter_map(|arg| arg.value())
                                .filter_map(namespace_name)
                            {
                                wildcard.push(source);
                            }
                        }
                        _ => {}
                    }
                }
                if wildcard
                    .iter()
                    .any(|package| !package_indexed(resolution.indexed, resolution.remote, package))
                {
                    return false;
                }
                imports.extend(package_origins(resolution, name, &wildcard));
                if !imports.is_empty() {
                    return imports.len() == 1 && imports.contains(package);
                }
            }
            let mut loaded = Vec::new();
            if let Some(root) = &file.package_root
                && let Some(declaration) = resolution
                    .packages
                    .iter()
                    .find(|declaration| &declaration.root == root)
            {
                loaded.extend(declaration.attached.iter().cloned());
                if package_origins(resolution, name, &loaded).len() > 1 {
                    return false;
                }
            }
            // Depends attaches before this file executes its own library calls.
            loaded.extend(attached.iter().cloned());
            let loaded: Vec<_> = loaded
                .into_iter()
                .map(|name| LoadedPackage {
                    name: SmolStr::new(name),
                    range: TextRange::default(),
                })
                .collect();
            if loaded.iter().any(|package| {
                !package_indexed(resolution.indexed, resolution.remote, &package.name)
            }) {
                return false;
            }
            match resolve_origin(resolution.indexed, resolution.remote, name, &loaded) {
                PackageOrigin::Resolved(origin) => origin == package,
                PackageOrigin::Ambiguous(origins) => {
                    origins.last().is_some_and(|origin| origin == package)
                }
                PackageOrigin::Unknown => false,
            }
        }
    }
}

fn package_origins(
    resolution: &ClassResolution<'_>,
    name: &str,
    packages: &[String],
) -> BTreeSet<String> {
    packages
        .iter()
        .filter(|package| {
            let loaded = [LoadedPackage {
                name: SmolStr::new(package),
                range: TextRange::default(),
            }];
            if loaded.iter().any(|package| {
                !package_indexed(resolution.indexed, resolution.remote, &package.name)
            }) {
                return false;
            }
            match resolve_origin(resolution.indexed, resolution.remote, name, &loaded) {
                PackageOrigin::Resolved(origin) => origin == package.as_str(),
                PackageOrigin::Ambiguous(origins) => {
                    origins.iter().any(|origin| origin == package.as_str())
                }
                PackageOrigin::Unknown => false,
            }
        })
        .cloned()
        .collect()
}

fn namespace_name(element: SyntaxElement) -> Option<String> {
    let token = element.into_token()?;
    match token.kind() {
        SyntaxKind::IDENT => Some(unbacktick(token.text()).to_string()),
        SyntaxKind::STRING => crate::semantic::decode_string_literal(token.text()),
        _ => None,
    }
}
