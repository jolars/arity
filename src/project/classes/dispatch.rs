//! Dispatch metadata and registration effects, separate from editable bindings.

use super::*;
use crate::semantic::enclosure::{
    Availability, MethodEnclosure, MethodImplementation, ScopeBinding,
};
use std::path::Path;

const S3_NAMES: &[&str] = &[
    ".Generic",
    ".Method",
    ".Class",
    ".Group",
    ".GenericCallEnv",
    ".GenericDefEnv",
];
const S4_NAMES: &[&str] = &[".Generic", ".Method", ".Methods", ".defined", ".target"];

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct DispatchEnvironment {
    pub method: OopKey,
    pub bindings: BTreeMap<String, Availability>,
    pub custom_parent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct GenericEffect {
    pub declaration: OopKey,
    pub name: String,
    pub availability: Availability,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct NamedDispatchEnvironment {
    pub binding: String,
    pub registration: OopId,
    pub environment: DispatchEnvironment,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct DispatchContexts {
    pub methods: Vec<DispatchEnvironment>,
    pub generics: Vec<GenericEffect>,
    pub registered_s3: Vec<String>,
    pub named: Vec<NamedDispatchEnvironment>,
}

fn primitive(name: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static NAMES: OnceLock<HashSet<&str>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            include_str!("../../semantic/base_r/primitives.txt")
                .lines()
                .collect()
        })
        .contains(name)
}

impl ClassIndex {
    pub fn dispatch_contexts(&self, path: &Path) -> DispatchContexts {
        self.dispatch_contexts_inner(path, true)
    }

    fn dispatch_contexts_inner(
        &self,
        path: &Path,
        resolve_implementations: bool,
    ) -> DispatchContexts {
        let Some(file) = self.file(path) else {
            return DispatchContexts::default();
        };
        let registered_s3 = self
            .registered_s3_names(path)
            .iter()
            .filter(|name| file.exports.contains(*name))
            .filter(|name| {
                !file.facts.oop.methods.iter().any(|method| {
                    method.system == OopSystem::S3
                        && method.name == ClassName::Known((*name).clone())
                })
            })
            .cloned()
            .collect();
        let mut contexts = DispatchContexts {
            registered_s3,
            ..Default::default()
        };
        for method in &file.facts.oop.methods {
            if !matches!(method.system, OopSystem::S3 | OopSystem::S4)
                || method
                    .uncertainty
                    .contains(&OopUncertainty::InvalidArguments)
            {
                continue;
            }
            if method.registration.as_ref().is_some_and(|registration| {
                self.callee_availability(path, registration) == Availability::Absent
            }) {
                continue;
            }
            let confirmed = match &method.registration {
                Some(registration) => self.callee_identity(path, registration).is_some(),
                None => {
                    self.registered_s3_method(
                        path,
                        match &method.name {
                            ClassName::Known(name) => name,
                            _ => "",
                        },
                    ) || method.generic.as_ref().is_some_and(|generic| {
                        !self
                            .generic_candidates(path, generic, !method.key.owner.0.is_empty())
                            .is_empty()
                    })
                }
            };
            let available = if confirmed {
                Availability::Present
            } else {
                Availability::Unknown
            };
            let mut creates_generic = false;
            let mut bindings: BTreeMap<_, _> = if method.system == OopSystem::S3 {
                S3_NAMES
                    .iter()
                    .map(|name| (name.to_string(), available))
                    .collect()
            } else {
                let generic = method.generic.as_ref();
                let candidates = generic
                    .map(|generic| {
                        self.generic_candidates(path, generic, !method.key.owner.0.is_empty())
                    })
                    .unwrap_or_default();
                let known_standard = candidates.iter().any(|id| {
                    self.generic(id)
                        .is_some_and(|generic| generic.signature.is_some())
                });
                let name = generic.and_then(|generic| match &generic.name {
                    ClassName::Known(name) => Some(name.as_str()),
                    _ => None,
                });
                let is_primitive = !known_standard
                    && name_is_known(generic)
                    && name.is_some_and(|name| {
                        primitive(name)
                            && !file.exports.contains(name)
                            && self.definition_files(path, name, true).is_empty()
                    });
                creates_generic = !is_primitive && candidates.is_empty();
                S4_NAMES
                    .iter()
                    .filter(|name| !is_primitive || **name == ".Generic")
                    .map(|name| {
                        (
                            name.to_string(),
                            if *name == ".Generic" || name_is_known(generic) {
                                available
                            } else {
                                Availability::Unknown
                            },
                        )
                    })
                    .collect()
            };
            // Conditional registration changes reachability, not the frame R
            // supplies once dispatch reaches this implementation.
            if !confirmed {
                for value in bindings.values_mut() {
                    *value = Availability::Unknown;
                }
            }
            contexts.methods.push(DispatchEnvironment {
                method: method.key.clone(),
                bindings,
                custom_parent: false,
            });
            if creates_generic
                && let Some(GenericRef {
                    name: ClassName::Known(name),
                    by_binding: false,
                    ..
                }) = &method.generic
            {
                contexts.generics.push(GenericEffect {
                    declaration: method.key.clone(),
                    name: name.clone(),
                    availability: if confirmed
                        && file.exports.contains(name)
                        && method.key.owner.0.is_empty()
                        && !method.uncertainty.contains(&OopUncertainty::Conditional)
                        && !method
                            .uncertainty
                            .contains(&OopUncertainty::CustomEnvironment)
                    {
                        Availability::Present
                    } else {
                        Availability::Unknown
                    },
                });
            }
        }
        for generic in &file.facts.oop.generics {
            if generic.key.system != OopSystem::S4
                || self.callee_availability(path, &generic.factory) == Availability::Absent
                || generic
                    .uncertainty
                    .iter()
                    .any(|issue| matches!(issue, OopUncertainty::InvalidArguments))
            {
                continue;
            }
            let available = if self.generic_confirmed(path, generic) {
                Availability::Present
            } else {
                Availability::Unknown
            };
            contexts.methods.push(DispatchEnvironment {
                method: generic.key.clone(),
                custom_parent: generic
                    .uncertainty
                    .contains(&OopUncertainty::CustomEnvironment),
                // A supplied function may become either the generic itself
                // or its default method. Only .Generic is common to both.
                bindings: S4_NAMES
                    .iter()
                    .map(|name| {
                        (
                            name.to_string(),
                            if *name == ".Generic" {
                                available
                            } else {
                                Availability::Unknown
                            },
                        )
                    })
                    .collect(),
            });
            if let ClassName::Known(name) = &generic.name {
                if primitive(name) && generic.signature.is_none() && !file.exports.contains(name) {
                    continue;
                }
                contexts.generics.push(GenericEffect {
                    declaration: generic.key.clone(),
                    name: name.clone(),
                    availability: if self.generic_confirmed(path, generic)
                        && generic.key.owner.0.is_empty()
                        && !generic.uncertainty.contains(&OopUncertainty::Conditional)
                        && !generic
                            .uncertainty
                            .contains(&OopUncertainty::CustomEnvironment)
                    {
                        Availability::Present
                    } else {
                        Availability::Unknown
                    },
                });
            }
        }
        if resolve_implementations {
            for source in self.oop_files() {
                let references: Vec<_> = source
                    .facts
                    .oop
                    .method_implementations
                    .iter()
                    .filter_map(|implementation| {
                        let MethodImplementationSource::Reference { name } = &implementation.source
                        else {
                            return None;
                        };
                        if !file.exports.contains(name)
                            || !(source.path == path
                                || self
                                    .definition_files(
                                        &source.path,
                                        name,
                                        !implementation.method.owner.0.is_empty(),
                                    )
                                    .contains(path))
                        {
                            return None;
                        }
                        Some((name, &implementation.method))
                    })
                    .collect();
                if references.is_empty() {
                    continue;
                }
                let source_contexts = self.dispatch_contexts_inner(&source.path, false);
                for (name, method) in references {
                    if let Some(environment) = source_contexts
                        .methods
                        .iter()
                        .find(|environment| environment.method == *method)
                    {
                        contexts.named.push(NamedDispatchEnvironment {
                            binding: name.clone(),
                            registration: OopId {
                                path: source.path.clone(),
                                key: method.clone(),
                            },
                            environment: environment.clone(),
                        });
                    }
                }
            }
        }
        contexts
    }
}

fn name_is_known(generic: Option<&GenericRef>) -> bool {
    generic.is_some_and(|generic| {
        matches!(generic.name, ClassName::Known(_))
            && (!generic.by_binding || generic.local.is_some())
    })
}

pub fn apply_dispatch_contexts(
    model: &SemanticModel,
    root: &SyntaxNode,
    contexts: &DispatchContexts,
) -> Option<SemanticModel> {
    if contexts.methods.is_empty()
        && contexts.generics.is_empty()
        && contexts.registered_s3.is_empty()
        && contexts.named.is_empty()
    {
        return None;
    }
    let scope_at = |range| {
        model
            .scopes()
            .iter()
            .position(|scope| scope.range == range)
            .map(crate::semantic::ScopeId::from_index)
    };
    let sites = method_implementation_sites(model, root);
    let mut environments = Vec::new();
    let mut implementations = Vec::new();
    for method in &contexts.methods {
        for site in sites.iter().filter(|site| site.method == method.method) {
            let Some(scope) = scope_at(site.function_range) else {
                continue;
            };
            let enclosure = environments.len() as u32;
            environments.push(MethodEnclosure {
                parent: model.scope(scope).parent,
                parent_is_unknown: method.custom_parent,
                bindings: method
                    .bindings
                    .iter()
                    .map(|(name, available)| (name.as_str().into(), *available))
                    .collect(),
                unknown_members: false,
                call_frame: true,
            });
            implementations.push(MethodImplementation {
                scope,
                enclosure,
                keep_original: site.keep_original,
            });
        }
    }
    let s3_bindings: BTreeMap<String, Availability> = S3_NAMES
        .iter()
        .map(|name| (name.to_string(), Availability::Present))
        .collect();
    for (name, bindings, retain_original, custom_parent) in contexts
        .registered_s3
        .iter()
        .map(|name| (name, &s3_bindings, false, false))
        .chain(contexts.named.iter().map(|named| {
            (
                &named.binding,
                &named.environment.bindings,
                true,
                named.environment.custom_parent,
            )
        }))
    {
        let Some((index, _)) = model.bindings().iter().enumerate().find(|(_, binding)| {
            model.scope(binding.scope).kind == crate::semantic::ScopeKind::File
                && unbacktick(&binding.name) == name
        }) else {
            continue;
        };
        let mut current = BindingId::from_index(index);
        let mut seen = BTreeSet::new();
        let mut keep_original = retain_original;
        let function = loop {
            if seen.len() >= 64 || !seen.insert(current.0) {
                break None;
            }
            let Some(value) = stable_value(model, root, current) else {
                break None;
            };
            if let Some(function) = value.as_node().cloned().and_then(FunctionExpr::cast) {
                break Some(function);
            }
            let Some(binding) = value
                .as_token()
                .and_then(|token| read_binding(model, token))
            else {
                break None;
            };
            current = binding;
            keep_original = true;
        };
        let Some(scope) = function.and_then(|function| scope_at(function.syntax().text_range()))
        else {
            continue;
        };
        let enclosure = environments.len() as u32;
        environments.push(MethodEnclosure {
            parent: model.scope(scope).parent,
            parent_is_unknown: custom_parent,
            bindings: bindings
                .iter()
                .map(|(name, availability)| (name.as_str().into(), *availability))
                .collect(),
            unknown_members: false,
            call_frame: true,
        });
        implementations.push(MethodImplementation {
            scope,
            enclosure,
            keep_original,
        });
    }
    let model = model.with_method_enclosures(environments, &implementations);
    let sites = oop_sites(&model, root);
    let bindings = contexts
        .generics
        .iter()
        .filter_map(|effect| {
            let site = sites.iter().find(|site| {
                site.role == OopSiteRole::Declaration
                    && site.target == OopTarget::Declaration(effect.declaration.clone())
            })?;
            let call = root
                .covering_element(site.range)
                .parent()?
                .ancestors()
                .find_map(CallExpr::cast)?;
            // A qualified constructor proves that this registration targets a
            // fresh environment. Other explicit targets remain possible writes
            // to the file's top environment, without editable definitions.
            if call.arg_list().is_some_and(|args| {
                args.args().any(|arg| {
                    arg.name().as_deref() == Some("where")
                        && arg
                            .value()
                            .and_then(SyntaxElement::into_node)
                            .and_then(CallExpr::cast)
                            .and_then(|call| call.base())
                            .and_then(SyntaxElement::into_node)
                            .and_then(BinaryExpr::cast)
                            .and_then(|binary| binary.namespace_access())
                            .is_some_and(|namespace| {
                                namespace.package == "base" && namespace.name == "new.env"
                            })
                })
            }) {
                return None;
            }
            Some(ScopeBinding {
                // The methods API defaults to topenv(parent.frame()), even for
                // registrations written inside a function.
                scope: crate::semantic::ScopeId::from_index(0),
                name: effect.name.as_str().into(),
                available_from: call.syntax().text_range().end(),
                availability: effect.availability,
            })
        })
        .collect();
    Some(model.with_scope_bindings(bindings))
}
