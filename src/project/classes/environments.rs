//! Runtime method environment facts share declaration, callee, and inheritance resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::semantic::SemanticModel;
use crate::semantic::enclosure::{Availability, MethodEnclosure, MethodImplementation};
use crate::syntax::SyntaxNode;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct ClassMethodEnvironment {
    pub class: ClassKey,
    pub methods: Vec<OopKey>,
    pub bindings: [Availability; 3],
    pub custom_parent: bool,
    pub named_bindings: BTreeMap<String, Availability>,
    pub unknown_members: bool,
}

/// Compatibility name for the original R6 environment API.
pub type R6MethodEnvironment = ClassMethodEnvironment;

pub(crate) fn has_method_candidates(model: &SemanticModel) -> bool {
    model
        .referenced_packages()
        .iter()
        .any(|package| matches!(package.as_str(), "R6" | "methods"))
        || model
            .idents()
            .iter()
            .any(|ident| matches!(unbacktick(&ident.name), "R6Class" | "setRefClass"))
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
        file.facts
            .declarations
            .iter()
            .filter_map(|class| {
                let id = ClassId {
                    path: path.into(),
                    key: class.key.clone(),
                };
                if !matches!(class.system(), ClassSystem::R6 | ClassSystem::RefClass)
                    || !self.declarations.contains_key(&id)
                {
                    return None;
                }
                let methods: Vec<_> = file
                    .facts
                    .oop
                    .members
                    .iter()
                    .filter(|member| member.class == class.key)
                    .filter(|member| {
                        let group = if class.system() == ClassSystem::RefClass {
                            "methods"
                        } else {
                            match (member.visibility, member.kind) {
                                (_, MemberKind::ActiveBinding) => "active",
                                (MemberVisibility::Private, _) => "private",
                                _ => "public",
                            }
                        };
                        file.facts.oop.member_groups.iter().any(|candidate| {
                            candidate.class == class.key
                                && candidate.name == group
                                && self.is_list_group(path, candidate)
                        })
                    })
                    .filter_map(|member| member.method.clone())
                    .filter(|key| {
                        // External reference methods retain their ordinary closure.
                        class.system() != ClassSystem::RefClass
                            || !file
                                .facts
                                .oop
                                .methods
                                .iter()
                                .find(|method| method.key == *key)
                                .and_then(|method| method.signature.as_ref())
                                .and_then(|signature| signature.parameters.first())
                                .is_some_and(|parameter| parameter.name == ".self")
                    })
                    .collect();
                if methods.is_empty() {
                    return None;
                }
                let super_binding = if class.parents.is_empty() {
                    Availability::Absent
                } else if self
                    .supertypes
                    .get(&id)
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
                        &id,
                        &mut named_bindings,
                        &mut unknown_members,
                        &mut BTreeSet::new(),
                    );
                    // Inherited method copies can see members added by a subclass,
                    // while the original base instance still lacks those names.
                    let mut pending: Vec<_> = self
                        .subtypes
                        .get(&id)
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
                            &mut BTreeSet::new(),
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
                    methods,
                    named_bindings,
                    unknown_members,
                    bindings: if class.system() == ClassSystem::RefClass {
                        [Availability::Absent; 3]
                    } else {
                        [
                            Availability::Present,
                            self.method_private_binding(&id),
                            super_binding,
                        ]
                    },
                    custom_parent: class
                        .uncertainty
                        .contains(&ClassUncertainty::CustomEnvironment),
                })
            })
            .collect()
    }

    fn method_private_binding(&self, id: &ClassId) -> Availability {
        let initial = self.private_binding(id, &mut BTreeSet::new());
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
            if self.private_binding(&class, &mut BTreeSet::new()) != initial {
                return Availability::Unknown;
            }
            pending.extend(self.subtypes.get(&class).into_iter().flatten().cloned());
        }
        initial
    }

    fn is_list_group(&self, path: &Path, group: &MemberGroup) -> bool {
        group
            .factory
            .as_ref()
            .and_then(|factory| self.callee_identity(path, factory))
            .is_some_and(|identity| identity.package == "base" && identity.name == "list")
            && !group.uncertainty.contains(&OopUncertainty::DynamicMembers)
    }

    fn member_bindings(
        &self,
        id: &ClassId,
        bindings: &mut BTreeMap<String, Availability>,
        unknown: &mut bool,
        visiting: &mut BTreeSet<ClassId>,
    ) {
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            *unknown = true;
            return;
        }
        let (Some(class), Some(file)) = (self.declarations.get(id), self.file(&id.path)) else {
            *unknown = true;
            return;
        };
        for group in file
            .facts
            .oop
            .member_groups
            .iter()
            .filter(|group| group.class == id.key)
        {
            let valid = !group.uncertainty.contains(&OopUncertainty::DynamicMembers)
                && group
                    .factory
                    .as_ref()
                    .and_then(|factory| self.callee_identity(&id.path, factory))
                    .is_some_and(|identity| {
                        identity.package == "base"
                            && (identity.name == "list"
                                || (group.name == "fields" && identity.name == "c"))
                    });
            if !valid {
                *unknown = true;
                continue;
            }
            for member in file
                .facts
                .oop
                .members
                .iter()
                .filter(|member| member.class == id.key)
            {
                let member_group = if class.declaration.system() == ClassSystem::RefClass {
                    if member.kind == MemberKind::Field {
                        "fields"
                    } else {
                        "methods"
                    }
                } else {
                    match (member.visibility, member.kind) {
                        (_, MemberKind::ActiveBinding) => "active",
                        (MemberVisibility::Private, _) => "private",
                        _ => "public",
                    }
                };
                if member_group == group.name {
                    if let ClassName::Known(name) = &member.name {
                        bindings.insert(name.clone(), Availability::Present);
                    } else {
                        *unknown = true;
                    }
                }
            }
        }
        let parents = self.supertypes.get(id);
        if parents.map_or(0, BTreeSet::len) < class.declaration.parents.len() {
            *unknown = true;
        }
        for parent in parents.into_iter().flatten() {
            self.member_bindings(parent, bindings, unknown, visiting);
        }
        visiting.remove(id);
    }

    fn private_binding(&self, id: &ClassId, visiting: &mut BTreeSet<ClassId>) -> Availability {
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            return Availability::Unknown;
        }
        let result = self.private_binding_inner(id, visiting);
        visiting.remove(id);
        result
    }

    fn private_binding_inner(
        &self,
        id: &ClassId,
        visiting: &mut BTreeSet<ClassId>,
    ) -> Availability {
        let Some(class) = self.declarations.get(id) else {
            return Availability::Unknown;
        };
        let Some(file) = self.file(&id.path) else {
            return Availability::Unknown;
        };
        let mut presence = Availability::Absent;
        for group in file
            .facts
            .oop
            .member_groups
            .iter()
            .filter(|group| group.class == id.key && group.name == "private")
        {
            if !self.is_list_group(&id.path, group) {
                presence = Availability::Unknown;
            } else if file.facts.oop.members.iter().any(|member| {
                member.class == id.key
                    && member.visibility == MemberVisibility::Private
                    && matches!(member.name, ClassName::Known(_))
            }) {
                return Availability::Present;
            }
        }
        if !class.declaration.parents.is_empty() {
            let parents = self.supertypes.get(id);
            if parents.is_none_or(BTreeSet::is_empty) {
                presence = Availability::Unknown;
            } else {
                for parent in parents.into_iter().flatten() {
                    match self.private_binding(parent, visiting) {
                        Availability::Present => return Availability::Present,
                        Availability::Unknown => presence = Availability::Unknown,
                        Availability::Absent => {}
                    }
                }
            }
        }
        presence
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
        let sites: Vec<_> = sites
            .iter()
            .filter(|site| environment.methods.contains(&site.method))
            .collect();
        let Some(site) = sites.first() else { continue };
        let enclosure = lowered.len() as u32;
        lowered.push(MethodEnclosure {
            parent: scope_at(site.owner_range),
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
                scope: scope_at(site.function_range)?,
                enclosure,
                keep_original: site.keep_original,
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
    if facts.oop.method_implementations.is_empty() {
        return model;
    }
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
            methods,
            bindings: [Availability::Unknown; 3],
            named_bindings: BTreeMap::new(),
            unknown_members: true,
            custom_parent: class
                .uncertainty
                .contains(&ClassUncertainty::CustomEnvironment),
        });
    }
    apply_method_environments(&model, root, &environments).unwrap_or(model)
}
