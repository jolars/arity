//! Workspace packages retain their own namespaces. Exported declarations cross
//! package boundaries through imports or attachment, never through names alone.

use super::*;
use crate::namespace::DirectiveKind as Kind;

/// Formal metadata imports and exports are independent of lexical values. In
/// particular, a method table is keyed by the generic's name and original
/// package. The package exporting it may only own registrations for that generic.
#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct S4Namespace {
    pub exported_classes: BTreeSet<String>,
    pub exported_methods: BTreeSet<String>,
    pub imported_classes: BTreeMap<String, BTreeSet<String>>,
    pub imported_methods: BTreeMap<String, BTreeSet<String>>,
    pub loaded_packages: BTreeSet<String>,
    pub complete: bool,
}

impl S4Namespace {
    fn from_directives(directives: &[NamespaceDirective]) -> Self {
        let mut result = Self {
            complete: true,
            ..Self::default()
        };
        for directive in directives {
            let args = &directive.operands;
            match directive.kind {
                Kind::ExportClass | Kind::ExportClasses => {
                    result.exported_classes.extend(args.iter().cloned())
                }
                Kind::ExportMethods => result.exported_methods.extend(args.iter().cloned()),
                Kind::Import => result.loaded_packages.extend(args.iter().cloned()),
                Kind::ImportFrom
                | Kind::ImportClassFrom
                | Kind::ImportClassesFrom
                | Kind::ImportMethodsFrom => {
                    if let Some((package, names)) = args.split_first() {
                        result.loaded_packages.insert(package.clone());
                        let imports = match directive.kind {
                            Kind::ImportClassFrom | Kind::ImportClassesFrom => {
                                Some(&mut result.imported_classes)
                            }
                            Kind::ImportMethodsFrom => Some(&mut result.imported_methods),
                            _ => None,
                        };
                        if let Some(imports) = imports {
                            imports
                                .entry(package.clone())
                                .or_default()
                                .extend(names.iter().cloned());
                        }
                    } else {
                        result.complete = false;
                    }
                }
                _ => {}
            }
        }
        result
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Class,
    S4Generic,
    Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub(super) struct NamespaceVisibility {
    classes: BTreeMap<(PathBuf, String), BTreeSet<PathBuf>>,
    generics: BTreeMap<(PathBuf, String), BTreeSet<PathBuf>>,
    qualified: BTreeMap<(String, String), BTreeSet<PathBuf>>,
    methods: BTreeMap<(PathBuf, String), BTreeSet<PathBuf>>,
    qualified_methods: BTreeMap<(String, String), BTreeSet<PathBuf>>,
    reexports: BTreeMap<String, Reexports>,
    pub s4: BTreeMap<PathBuf, S4Namespace>,
}

impl NamespaceVisibility {
    pub fn build(files: &BTreeMap<PathBuf, ClassFile>, resolution: &ClassResolution<'_>) -> Self {
        let namespaces: BTreeMap<_, _> = resolution
            .namespaces
            .iter()
            .filter_map(|(root, text)| directives(text).map(|directives| (root, directives)))
            .collect();
        let mut result = Self {
            s4: resolution
                .namespaces
                .iter()
                .map(|(root, _)| {
                    let namespace = namespaces
                        .get(root)
                        .map_or_else(S4Namespace::default, |directives| {
                            S4Namespace::from_directives(directives)
                        });
                    (root.clone(), namespace)
                })
                .collect(),
            ..Self::default()
        };
        for target in files.values() {
            let Some(root) = &target.package_root else {
                continue;
            };
            let Some(package) = resolution.package_names.get(root) else {
                continue;
            };
            let Some(namespace) = namespaces.get(root) else {
                continue;
            };
            if !resolution.scope.package_complete(&target.path) {
                continue;
            }
            for (name, surface) in target
                .facts
                .declarations
                .iter()
                .filter_map(|def| {
                    if !def.key.owner.0.is_empty() {
                        return None;
                    }
                    match &def.name {
                        ClassName::Known(name)
                            if matches!(def.system(), ClassSystem::S4 | ClassSystem::RefClass) =>
                        {
                            Some((name.clone(), Surface::Class))
                        }
                        _ => None,
                    }
                })
                .chain(
                    target
                        .facts
                        .oop
                        .generics
                        .iter()
                        .filter(|def| def.key.owner.0.is_empty())
                        .flat_map(|def| {
                            let mut names = Vec::new();
                            if let ClassName::Known(name) = &def.name {
                                names.push((
                                    name.clone(),
                                    if def.key.system == OopSystem::S4 {
                                        Surface::S4Generic
                                    } else {
                                        Surface::Value
                                    },
                                ));
                            }
                            if let Some(binding) = &def.binding {
                                names.push((binding.clone(), Surface::Value));
                            }
                            names.extend(
                                target
                                    .facts
                                    .oop
                                    .generic_bindings
                                    .iter()
                                    .filter(|binding| binding.target == def.key)
                                    .map(|binding| (binding.name.clone(), Surface::Value)),
                            );
                            names
                        }),
                )
                .chain(
                    target
                        .facts
                        .generators
                        .iter()
                        .filter(|binding| binding.owner.0.is_empty())
                        .map(|binding| (binding.name.clone(), Surface::Value)),
                )
                .chain(
                    target
                        .facts
                        .oop
                        .callables
                        .iter()
                        .map(|binding| (binding.name.clone(), Surface::Value)),
                )
                .chain(
                    target
                        .facts
                        .generator_aliases
                        .iter()
                        .filter(|binding| binding.owner.0.is_empty())
                        .map(|binding| (binding.name.clone(), Surface::Value)),
                )
            {
                if !exported(namespace, &name, surface) {
                    continue;
                }
                if surface != Surface::Class {
                    result
                        .qualified
                        .entry((package.clone(), name.clone()))
                        .or_default()
                        .insert(target.path.clone());
                }
                for source in files
                    .values()
                    .filter(|source| source.package_root != target.package_root)
                {
                    let Some(source_root) = &source.package_root else {
                        continue;
                    };
                    let Some(source_namespace) = namespaces.get(source_root) else {
                        continue;
                    };
                    if imported(source_namespace, package, &name, surface)
                        || resolution.packages.iter().any(|declaration| {
                            &declaration.root == source_root
                                && declaration.attached.contains(package)
                        })
                    {
                        let map = if surface == Surface::Class {
                            &mut result.classes
                        } else {
                            &mut result.generics
                        };
                        map.entry((source.path.clone(), name.clone()))
                            .or_default()
                            .insert(target.path.clone());
                    }
                }
            }
            // Method tables can be exported by a package that has no generic
            // declaration of its own. Keep those provider files separate from
            // lexical function exports, then resolve their generic references.
            let method_names: BTreeSet<_> = target
                .facts
                .oop
                .methods
                .iter()
                .filter(|method| method.system == OopSystem::S4 && method.key.owner.0.is_empty())
                .filter_map(|method| method.generic.as_ref())
                .filter_map(|generic| match &generic.name {
                    ClassName::Known(name) => Some(name.clone()),
                    _ => None,
                })
                .chain(
                    target
                        .facts
                        .oop
                        .generics
                        .iter()
                        .filter(|generic| {
                            generic.key.system == OopSystem::S4 && generic.key.owner.0.is_empty()
                        })
                        .filter_map(|generic| match &generic.name {
                            ClassName::Known(name) => Some(name.clone()),
                            _ => None,
                        }),
                )
                .chain(
                    result
                        .s4
                        .get(root)
                        .into_iter()
                        .flat_map(|namespace| namespace.exported_methods.iter().cloned()),
                )
                .collect();
            for name in method_names {
                if !exported(namespace, &name, Surface::S4Generic) {
                    continue;
                }
                result
                    .qualified_methods
                    .entry((package.clone(), name.clone()))
                    .or_default()
                    .insert(target.path.clone());
                for source in files
                    .values()
                    .filter(|source| source.package_root != target.package_root)
                {
                    let Some(source_namespace) = source
                        .package_root
                        .as_ref()
                        .and_then(|root| namespaces.get(root))
                    else {
                        continue;
                    };
                    if imported(source_namespace, package, &name, Surface::S4Generic)
                        || resolution.packages.iter().any(|declaration| {
                            Some(&declaration.root) == source.package_root.as_ref()
                                && declaration.attached.contains(package)
                        })
                    {
                        result
                            .methods
                            .entry((source.path.clone(), name.clone()))
                            .or_default()
                            .insert(target.path.clone());
                    }
                }
            }
        }
        result.reexports = namespaces
            .into_iter()
            .filter_map(|(root, directives)| {
                Some((
                    resolution.package_names.get(root)?.clone(),
                    Reexports::new(directives),
                ))
            })
            .collect();
        result
    }

    pub fn class_files(&self, path: &Path, name: &str) -> BTreeSet<PathBuf> {
        self.classes
            .get(&(path.to_path_buf(), name.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    pub fn generic_files(
        &self,
        path: &Path,
        name: &str,
        package: Option<&str>,
    ) -> BTreeSet<PathBuf> {
        match package {
            Some(package) => self.qualified.get(&(package.to_string(), name.to_string())),
            None => self.generics.get(&(path.to_path_buf(), name.to_string())),
        }
        .cloned()
        .unwrap_or_default()
    }

    pub fn method_files(
        &self,
        path: &Path,
        name: &str,
        package: Option<&str>,
    ) -> BTreeSet<PathBuf> {
        match package {
            Some(package) => self.qualified_methods.get(&(package.into(), name.into())),
            None => self.methods.get(&(path.into(), name.into())),
        }
        .cloned()
        .unwrap_or_default()
    }

    /// A re-export must have an imported lexical value. A method table alone
    /// only provides registrations for a generic that lives elsewhere.
    pub fn s4_reexports(&self, package: &str, name: &str) -> Option<ReexportSources> {
        let reexports = self.reexports.get(package)?;
        let value_export =
            reexports.exports.contains(name) || matches_pattern(&reexports.export_patterns, name);
        if !value_export && !reexports.method_exports.contains(name) {
            return None;
        }
        let bindings = reexports
            .named
            .get(name)
            .into_iter()
            .flatten()
            .cloned()
            .chain(
                reexports
                    .wildcard
                    .iter()
                    .filter(|(_, exclusions)| !exclusions.contains(name))
                    .map(|(package, _)| (package.clone(), name.into())),
            )
            .collect();
        Some(ReexportSources {
            bindings,
            value_export,
        })
    }
}

pub(in crate::project::classes) struct ReexportSources {
    pub bindings: BTreeSet<(String, String)>,
    pub value_export: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
struct Reexports {
    exports: BTreeSet<String>,
    method_exports: BTreeSet<String>,
    export_patterns: Vec<String>,
    named: BTreeMap<String, BTreeSet<(String, String)>>,
    wildcard: Vec<(String, BTreeSet<String>)>,
}

impl Reexports {
    fn new(directives: Vec<NamespaceDirective>) -> Self {
        let mut result = Self::default();
        for directive in directives {
            match directive.kind {
                Kind::Export => result.exports.extend(directive.operands),
                Kind::ExportMethods => result.method_exports.extend(directive.operands),
                Kind::ExportPattern => result.export_patterns.extend(directive.operands),
                Kind::Import => result.wildcard.extend(
                    directive
                        .operands
                        .into_iter()
                        .map(|package| (package, directive.exclusions.clone())),
                ),
                Kind::ImportFrom => {
                    if let Some(package) = directive.operands.first() {
                        for (local, remote) in directive.imports {
                            result
                                .named
                                .entry(local)
                                .or_default()
                                .insert((package.clone(), remote));
                        }
                    }
                }
                _ => {}
            }
        }
        result
    }
}

fn matches_pattern(patterns: &[String], name: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| regex::Regex::new(pattern).is_ok_and(|regex| regex.is_match(name)))
}

struct NamespaceDirective {
    kind: Kind,
    operands: Vec<String>,
    exclusions: BTreeSet<String>,
    imports: Vec<(String, String)>,
}

impl NamespaceDirective {
    fn imports_name(&self, name: &str) -> bool {
        !self.exclusions.contains(name)
    }
}

fn directives(text: &str) -> Option<Vec<NamespaceDirective>> {
    let parsed = crate::namespace::parse(text);
    if !parsed.diagnostics.is_empty()
        || parsed.document().directives().any(|directive| {
            directive
                .syntax()
                .ancestors()
                .any(|node| node.kind() == SyntaxKind::IF_EXPR)
        })
    {
        // Conditional imports can mask an otherwise unconditional origin.
        return None;
    }
    parsed
        .document()
        .directives()
        .map(|directive| {
            let kind = directive.kind();
            if kind == Kind::Import {
                let (operands, exclusions) = wildcard_import(&directive)?;
                return Some(NamespaceDirective {
                    kind,
                    operands,
                    exclusions,
                    imports: Vec::new(),
                });
            }
            let mut operands = Vec::new();
            let mut imports = Vec::new();
            for arg in directive.arguments() {
                if let Some(operand) = arg.value().and_then(namespace_name) {
                    if kind == Kind::ImportFrom && !operands.is_empty() {
                        let local = arg
                            .name()
                            .map_or_else(|| operand.clone(), |name| name.to_string());
                        imports.push((local, operand.clone()));
                    }
                    operands.push(operand);
                }
            }
            Some(NamespaceDirective {
                kind,
                operands,
                exclusions: BTreeSet::new(),
                imports,
            })
        })
        .collect()
}

/// R removes the named exclusion before interpreting package operands.
pub(in crate::project::classes) fn wildcard_import(
    directive: &crate::namespace::Directive,
) -> Option<(Vec<String>, BTreeSet<String>)> {
    let mut packages = Vec::new();
    let mut exclusions = BTreeSet::new();
    for arg in directive.arguments() {
        let value = arg.value()?;
        if arg.name().as_deref() != Some("except") {
            packages.push(namespace_name(value)?);
        } else if constant_atom(&value, "NULL") {
            continue;
        } else if let Some(name) = namespace_name(value.clone()) {
            exclusions.insert(name);
        } else {
            let call = CallExpr::cast(value.into_node()?)?;
            if call.callee_name().as_deref() != Some("c") {
                return None;
            }
            for arg in call.arg_list()?.args() {
                exclusions.insert(namespace_name(arg.value()?)?);
            }
        }
    }
    Some((packages, exclusions))
}

fn exported(directives: &[NamespaceDirective], name: &str, surface: Surface) -> bool {
    directives.iter().any(|directive| {
        let kind = directive.kind;
        let args = &directive.operands;
        let explicit = match surface {
            Surface::Class => matches!(kind, Kind::ExportClass | Kind::ExportClasses),
            Surface::S4Generic => matches!(kind, Kind::Export | Kind::ExportMethods),
            Surface::Value => kind == Kind::Export,
        };
        (explicit && args.iter().any(|arg| arg == name))
            || (matches!(kind, Kind::ExportPattern) && surface != Surface::Class
                || matches!(kind, Kind::ExportClassPattern) && surface == Surface::Class)
                && matches_pattern(args, name)
    })
}

fn imported(
    directives: &[NamespaceDirective],
    package: &str,
    name: &str,
    surface: Surface,
) -> bool {
    directives.iter().any(|directive| {
        let kind = directive.kind;
        let args = &directive.operands;
        if kind == Kind::Import {
            return directive.imports_name(name) && args.iter().any(|arg| arg == package);
        }
        if kind == Kind::ImportFrom {
            return surface != Surface::Class
                && args.first().is_some_and(|arg| arg == package)
                && directive
                    .imports
                    .iter()
                    .any(|(local, remote)| local == name && remote == name);
        }
        args.first().is_some_and(|arg| arg == package)
            && args.iter().skip(1).any(|arg| arg == name)
            && match surface {
                Surface::Class => matches!(kind, Kind::ImportClassFrom | Kind::ImportClassesFrom),
                Surface::S4Generic => kind == Kind::ImportMethodsFrom,
                Surface::Value => false,
            }
    })
}
