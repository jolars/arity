//! Runtime method environment facts share declaration, callee, and inheritance resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::semantic::SemanticModel;
use crate::semantic::enclosure::{Availability, MethodEnclosure, MethodImplementation};
use crate::syntax::SyntaxNode;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct ClassMethodEnvironment {
    pub class: ClassKey,
    pub class_path: PathBuf,
    pub local_class: bool,
    pub implementations: Vec<String>,
    pub methods: Vec<OopKey>,
    pub bindings: [Availability; 3],
    pub custom_parent: bool,
    pub named_bindings: BTreeMap<String, Availability>,
    pub unknown_members: bool,
}

/// Compatibility name for the original R6 environment API.
pub type R6MethodEnvironment = ClassMethodEnvironment;

pub(crate) fn has_method_candidates(model: &SemanticModel) -> bool {
    crate::semantic::calls::has_generator_mutations(model)
        || model
            .referenced_packages()
            .iter()
            .any(|package| matches!(package.as_str(), "R6" | "methods"))
        || model.idents().iter().any(|ident| {
            matches!(
                unbacktick(&ident.name),
                "R6Class"
                    | "setRefClass"
                    | "setGeneric"
                    | "setMethod"
                    | "setReplaceMethod"
                    | "registerS3method"
                    | ".Generic"
                    | ".Method"
                    | ".Class"
                    | ".Group"
                    | ".GenericCallEnv"
                    | ".GenericDefEnv"
                    | ".Methods"
                    | ".defined"
                    | ".target"
            )
        })
        || model
            .qualified_reads()
            .iter()
            .any(|name| unbacktick(name) == "registerS3method")
        || model.bindings().iter().any(|binding| {
            let name = unbacktick(&binding.name);
            name.char_indices()
                .any(|(i, ch)| ch == '.' && i > 0 && i + 1 < name.len())
        })
}

impl ClassIndex {
    /// These facts contain no body locations, so ordinary edits backdate them.
    pub fn r6_method_environments(&self, path: &Path) -> Vec<R6MethodEnvironment> {
        self.method_environments(path)
            .into_iter()
            .filter(|environment| environment.class.system == ClassSystem::R6)
            .collect()
    }

    pub fn method_environments(&self, path: &Path) -> Vec<ClassMethodEnvironment> {
        let Some(file) = self.file(path) else {
            return Vec::new();
        };
        self.declarations
            .iter()
            .filter_map(|(id, resolved)| {
                let class = &resolved.declaration;
                if !matches!(class.system(), ClassSystem::R6 | ClassSystem::RefClass) {
                    return None;
                }
                let installed = self.member_installations(id);
                let methods: Vec<_> = installed
                    .iter()
                    .filter(|member| member.id.path == path)
                    .filter(|member| {
                        class.system() != ClassSystem::RefClass
                            || member.definition.kind == MemberKind::ActiveBinding
                            || !member
                                .signature
                                .as_ref()
                                .and_then(|signature| signature.parameters.first())
                                .is_some_and(|parameter| parameter.name == ".self")
                    })
                    .filter_map(|member| member.definition.method.clone())
                    .collect();
                let implementations: Vec<_> = installed
                    .iter()
                    .filter_map(|member| {
                        let method = member.definition.method.as_ref()?;
                        let source = self.file(&member.id.path)?;
                        let implementation = source
                            .facts
                            .oop
                            .method_implementations
                            .iter()
                            .find(|implementation| &implementation.method == method)?;
                        let (name, package) = match &implementation.source {
                            MethodImplementationSource::Reference { name } => (name, None),
                            MethodImplementationSource::Qualified { package, name } => {
                                (name, Some(package.as_str()))
                            }
                            _ => return None,
                        };
                        if !file
                            .facts
                            .oop
                            .callables
                            .iter()
                            .any(|callable| &callable.name == name)
                        {
                            return None;
                        }
                        if !self
                            .implementation_files(&source.path, name, package)
                            .contains(path)
                        {
                            return None;
                        }
                        if class.system() == ClassSystem::RefClass
                            && member
                                .signature
                                .as_ref()?
                                .parameters
                                .first()
                                .is_some_and(|parameter| parameter.name == ".self")
                        {
                            return None;
                        }
                        Some(name.clone())
                    })
                    .collect();
                if methods.is_empty() && implementations.is_empty() {
                    return None;
                }
                let super_binding = if class.parents.is_empty() {
                    Availability::Absent
                } else if self
                    .supertypes
                    .get(id)
                    .is_some_and(|parents| !parents.is_empty())
                {
                    Availability::Present
                } else {
                    Availability::Unknown
                };
                let mut named_bindings = BTreeMap::new();
                let mut unknown_members = false;
                if class.system() == ClassSystem::RefClass || class.portable != Some(true) {
                    self.member_bindings(
                        id,
                        &mut named_bindings,
                        &mut unknown_members,
                    );
                    // Inherited method copies can see members added by a subclass,
                    // while the original base instance still lacks those names.
                    let mut pending: Vec<_> = self
                        .subtypes
                        .get(id)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .collect();
                    let mut seen = BTreeSet::new();
                    while let Some(child) = pending.pop() {
                        if seen.len() >= 64 {
                            unknown_members = true;
                            break;
                        }
                        if !seen.insert(child.clone()) {
                            continue;
                        }
                        let mut child_bindings = BTreeMap::new();
                        self.member_bindings(
                            &child,
                            &mut child_bindings,
                            &mut unknown_members,
                        );
                        for name in child_bindings.into_keys() {
                            named_bindings.entry(name).or_insert(Availability::Unknown);
                        }
                        pending.extend(self.subtypes.get(&child).into_iter().flatten().cloned());
                    }
                    if class.system() == ClassSystem::R6 && class.portable.is_none() {
                        for availability in named_bindings.values_mut() {
                            *availability = Availability::Unknown;
                        }
                    }
                }
                if class.system() == ClassSystem::RefClass {
                    for name in [
                        ".self",
                        ".refClassDef",
                        ".objectPackage",
                        ".objectParent",
                        "callSuper",
                        "copy",
                        "export",
                        "field",
                        "getClass",
                        "getRefClass",
                        "import",
                        "initFields",
                        "show",
                        "trace",
                        "untrace",
                        "usingMethods",
                    ] {
                        named_bindings.insert(name.into(), Availability::Present);
                    }
                }
                Some(ClassMethodEnvironment {
                    class: class.key.clone(),
                    class_path: id.path.clone(),
                    local_class: id.path == path,
                    implementations,
                    methods,
                    named_bindings,
                    unknown_members,
                    bindings: if class.system() == ClassSystem::RefClass {
                        [Availability::Absent; 3]
                    } else {
                        [
                            Availability::Present,
                            self.method_private_binding(id),
                            super_binding,
                        ]
                    },
                    custom_parent: class.uncertainty.contains(&ClassUncertainty::CustomEnvironment)
                        // A copied closure can enclose another package or a
                        // script caller. Its frame is outside this file's model.
                        || (id.path != path && (resolved.package_root.is_none() || resolved.package_root != file.package_root)),
                })
            })
            .collect()
    }

    fn method_private_binding(&self, id: &ClassId) -> Availability {
        let initial = self.private_binding(id);
        if initial == Availability::Present {
            return initial;
        }
        // Inherited copies enclose the complete instance's private environment.
        // The same source method can therefore have different bindings on a
        // base instance and on a known subclass instance.
        let mut pending = vec![id.clone()];
        let mut seen = BTreeSet::new();
        while let Some(class) = pending.pop() {
            if seen.len() >= 64 {
                return Availability::Unknown;
            }
            if !seen.insert(class.clone()) {
                continue;
            }
            if self.private_binding(&class) != initial {
                return Availability::Unknown;
            }
            pending.extend(self.subtypes.get(&class).into_iter().flatten().cloned());
        }
        initial
    }

    fn member_bindings(
        &self,
        id: &ClassId,
        bindings: &mut BTreeMap<String, Availability>,
        unknown: &mut bool,
    ) {
        if id.key.system == ClassSystem::R6 {
            if let Some(api) = self.r6_api(id) {
                *unknown |= !api.public.complete || !api.private.complete;
                for name in api.public.entries.keys().chain(api.private.entries.keys()) {
                    bindings.insert(name.clone(), Availability::Present);
                }
            } else {
                *unknown = true;
            }
        } else if let Some(api) = self.reference_api(id) {
            *unknown |= !api.instance.complete;
            for name in api.instance.entries.keys() {
                bindings.insert(name.clone(), Availability::Present);
            }
        } else {
            *unknown = true;
        }
    }

    fn private_binding(&self, id: &ClassId) -> Availability {
        match self.r6_api(id) {
            Some(api) if !api.private.entries.is_empty() => Availability::Present,
            Some(api) if api.private.complete => Availability::Absent,
            _ => Availability::Unknown,
        }
    }
}

/// Lower stable project identities to the current file's lexical scope IDs.
pub fn apply_method_environments(
    model: &SemanticModel,
    root: &SyntaxNode,
    environments: &[ClassMethodEnvironment],
) -> Option<SemanticModel> {
    if environments.is_empty() {
        return None;
    }
    let sites = method_implementation_sites(model, root);
    let scope_at = |range| {
        model
            .scopes()
            .iter()
            .position(|scope| scope.range == range)
            .map(crate::semantic::ScopeId::from_index)
    };
    let mut lowered = Vec::new();
    let mut implementations = Vec::new();
    for environment in environments {
        let mut sites: Vec<_> = sites
            .iter()
            .filter(|site| environment.methods.contains(&site.method))
            .map(|site| (site.function_range, site.owner_range, site.keep_original))
            .collect();
        for (index, binding) in model.bindings().iter().enumerate() {
            // Cross-file implementation lookup resolves a file-scope binding.
            // A nested binding with the same name is a separate closure.
            if model.scope(binding.scope).kind != crate::semantic::ScopeKind::File
                || !environment
                    .implementations
                    .iter()
                    .any(|name| name == unbacktick(&binding.name))
            {
                continue;
            }
            let Some(function) = stable_value(model, root, BindingId::from_index(index))
                .and_then(SyntaxElement::into_node)
                .and_then(FunctionExpr::cast)
            else {
                continue;
            };
            sites.push((function.syntax().text_range(), root.text_range(), true));
        }
        let Some(site) = sites.first() else { continue };
        let parent_range = environment
            .local_class
            .then(|| locate_class_def(root, &environment.class))
            .flatten()
            .and_then(|location| root.covering_element(location.call_range).parent())
            .and_then(|node| {
                node.ancestors()
                    .find(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            })
            .map_or_else(
                || {
                    if environment.local_class {
                        root.text_range()
                    } else {
                        site.1
                    }
                },
                |node| node.text_range(),
            );
        let enclosure = lowered.len() as u32;
        lowered.push(MethodEnclosure {
            call_frame: false,
            parent: scope_at(parent_range),
            parent_is_unknown: environment.custom_parent,
            bindings: ["self", "private", "super"]
                .into_iter()
                .zip(environment.bindings)
                .map(|(name, availability)| (name.into(), availability))
                .chain(
                    environment
                        .named_bindings
                        .iter()
                        .map(|(name, availability)| (name.as_str().into(), *availability)),
                )
                .collect(),
            unknown_members: environment.unknown_members,
        });
        implementations.extend(sites.into_iter().filter_map(|site| {
            Some(MethodImplementation {
                scope: scope_at(site.0)?,
                enclosure,
                keep_original: site.2,
            })
        }));
    }
    (!implementations.is_empty()).then(|| model.with_method_enclosures(lowered, &implementations))
}

/// Cold and pathless consumers use the same project resolver with one live file.
pub fn standalone_semantic_model(root: &SyntaxNode) -> SemanticModel {
    let model = SemanticModel::build(root);
    if !has_method_candidates(&model) {
        return model;
    }
    let facts = file_class_defs(&model, root);
    let path = Path::new("buffer.R");
    let index = crate::incremental::IncrementalDatabase::default()
        .snapshot()
        .class_index_in(path, &facts, root, &model);
    let mut environments = index.method_environments(path);
    // A cold helper has no NAMESPACE. Keep possible injected names uncertain
    // until the project can prove or reject a bare factory's origin.
    for class in facts
        .declarations
        .iter()
        .filter(|class| matches!(class.system(), ClassSystem::R6 | ClassSystem::RefClass))
    {
        if index.declarations.contains_key(&ClassId {
            path: path.into(),
            key: class.key.clone(),
        }) {
            continue;
        }
        if environments
            .iter()
            .any(|environment| environment.class == class.key)
        {
            continue;
        }
        let methods = facts
            .oop
            .methods
            .iter()
            .filter(|method| method.class.as_ref() == Some(&class.key))
            .map(|method| method.key.clone())
            .collect();
        environments.push(ClassMethodEnvironment {
            class: class.key.clone(),
            class_path: path.into(),
            local_class: true,
            implementations: Vec::new(),
            methods,
            bindings: [Availability::Unknown; 3],
            named_bindings: BTreeMap::new(),
            unknown_members: true,
            custom_parent: class
                .uncertainty
                .contains(&ClassUncertainty::CustomEnvironment),
        });
    }
    let model = apply_method_environments(&model, root, &environments).unwrap_or(model);
    apply_dispatch_contexts(&model, root, &index.dispatch_contexts(path)).unwrap_or(model)
}
