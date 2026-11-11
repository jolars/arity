//! Workspace packages retain their own namespaces. Exported declarations cross
//! package boundaries through imports or attachment, never through names alone.

use super::*;
use crate::namespace::DirectiveKind as Kind;

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub(super) struct NamespaceVisibility {
    classes: BTreeMap<(PathBuf, String), BTreeSet<PathBuf>>,
    generics: BTreeMap<(PathBuf, String), BTreeSet<PathBuf>>,
    qualified: BTreeMap<(String, String), BTreeSet<PathBuf>>,
}

impl NamespaceVisibility {
    pub fn build(files: &BTreeMap<PathBuf, ClassFile>, resolution: &ClassResolution<'_>) -> Self {
        let mut result = Self::default();
        let namespaces: BTreeMap<_, _> = resolution
            .namespaces
            .iter()
            .filter_map(|(root, text)| directives(text).map(|directives| (root, directives)))
            .collect();
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
            for (name, formal) in target
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
                            Some((name.clone(), true))
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
                                names.push((name.clone(), false));
                            }
                            if let Some(binding) = &def.binding {
                                names.push((binding.clone(), false));
                            }
                            names.extend(
                                target
                                    .facts
                                    .oop
                                    .generic_bindings
                                    .iter()
                                    .filter(|binding| binding.target == def.key)
                                    .map(|binding| (binding.name.clone(), false)),
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
                        .map(|binding| (binding.name.clone(), false)),
                )
            {
                if !exported(namespace, &name, formal) {
                    continue;
                }
                if !formal {
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
                    if imported(source_namespace, package, &name, formal)
                        || resolution.packages.iter().any(|declaration| {
                            &declaration.root == source_root
                                && declaration.attached.contains(package)
                        })
                    {
                        let map = if formal {
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
        }
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
}

fn directives(text: &str) -> Option<Vec<(Kind, Vec<String>)>> {
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
    Some(
        parsed
            .document()
            .directives()
            .map(|directive| {
                (
                    directive.kind(),
                    directive
                        .arguments()
                        .filter_map(|arg| arg.value())
                        .filter_map(namespace_name)
                        .collect(),
                )
            })
            .collect(),
    )
}

fn exported(directives: &[(Kind, Vec<String>)], name: &str, formal: bool) -> bool {
    directives.iter().any(|(kind, args)| {
        let explicit = if formal {
            matches!(kind, Kind::ExportClass | Kind::ExportClasses)
        } else {
            matches!(kind, Kind::Export | Kind::ExportMethods)
        };
        (explicit && args.iter().any(|arg| arg == name))
            || (matches!(kind, Kind::ExportPattern) && !formal
                || matches!(kind, Kind::ExportClassPattern) && formal)
                && args.iter().any(|pattern| {
                    regex::Regex::new(pattern).is_ok_and(|regex| regex.is_match(name))
                })
    })
}

fn imported(directives: &[(Kind, Vec<String>)], package: &str, name: &str, formal: bool) -> bool {
    directives.iter().any(|(kind, args)| {
        if *kind == Kind::Import {
            return args.iter().any(|arg| arg == package);
        }
        args.first().is_some_and(|arg| arg == package)
            && args.iter().skip(1).any(|arg| arg == name)
            && if formal {
                matches!(kind, Kind::ImportClassFrom | Kind::ImportClassesFrom)
            } else {
                matches!(kind, Kind::ImportFrom | Kind::ImportMethodsFrom)
            }
    })
}
