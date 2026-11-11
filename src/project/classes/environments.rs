//! R6 environment facts share declaration, callee, and inheritance resolution.

use std::collections::BTreeSet;
use std::path::Path;

use crate::semantic::SemanticModel;
use crate::semantic::enclosure::{Availability, MethodEnclosure, MethodImplementation};
use crate::syntax::SyntaxNode;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct R6MethodEnvironment {
    pub class: ClassKey,
    pub methods: Vec<OopKey>,
    pub bindings: [Availability; 3],
    pub custom_parent: bool,
}

pub(crate) fn has_r6_candidates(model: &SemanticModel) -> bool {
    model
        .referenced_packages()
        .iter()
        .any(|package| package == "R6")
        || model
            .idents()
            .iter()
            .any(|ident| unbacktick(&ident.name) == "R6Class")
}

impl ClassIndex {
    /// These facts contain no body locations, so ordinary edits backdate them.
    pub fn r6_method_environments(&self, path: &Path) -> Vec<R6MethodEnvironment> {
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
                if class.system() != ClassSystem::R6 || !self.declarations.contains_key(&id) {
                    return None;
                }
                let methods: Vec<_> = file
                    .facts
                    .oop
                    .members
                    .iter()
                    .filter(|member| member.class == class.key)
                    .filter(|member| {
                        let group = match (member.visibility, member.kind) {
                            (_, MemberKind::ActiveBinding) => "active",
                            (MemberVisibility::Private, _) => "private",
                            _ => "public",
                        };
                        file.facts.oop.member_groups.iter().any(|candidate| {
                            candidate.class == class.key
                                && candidate.name == group
                                && self.is_list_group(path, candidate)
                        })
                    })
                    .filter_map(|member| member.method.clone())
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
                Some(R6MethodEnvironment {
                    class: class.key.clone(),
                    methods,
                    bindings: [
                        Availability::Present,
                        self.method_private_binding(&id),
                        super_binding,
                    ],
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
    environments: &[R6MethodEnvironment],
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
            bindings: environment.bindings,
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
    if !has_r6_candidates(&model) {
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
    let mut environments = index.r6_method_environments(path);
    // A cold helper has no NAMESPACE. Keep possible injected names uncertain
    // until the project can prove or reject a bare factory's origin.
    for class in facts
        .declarations
        .iter()
        .filter(|class| class.system() == ClassSystem::R6)
    {
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
        environments.push(R6MethodEnvironment {
            class: class.key.clone(),
            methods,
            bindings: [Availability::Unknown; 3],
            custom_parent: class
                .uncertainty
                .contains(&ClassUncertainty::CustomEnvironment),
        });
    }
    apply_method_environments(&model, root, &environments).unwrap_or(model)
}
