//! OOP lookup shares class ownership, visibility, and callee resolution.

use super::*;

impl ClassIndex {
    pub(super) fn confirm_dispatch_helpers(&mut self) {
        for file in self.files.values() {
            for def in &file.facts.oop.methods {
                if def
                    .dispatch_factory
                    .as_ref()
                    .is_some_and(|helper| self.callee_identity(&file.path, helper).is_none())
                {
                    let mut method = def.clone();
                    method.dispatch = vec![DispatchArgument {
                        argument: None,
                        class: DispatchClass::Unknown,
                    }];
                    method.uncertainty.insert(OopUncertainty::DynamicSignature);
                    self.uncertain_methods.insert(
                        OopId {
                            path: file.path.clone(),
                            key: def.key.clone(),
                        },
                        method,
                    );
                }
            }
            for def in &file.facts.oop.generics {
                if def
                    .dispatch_factory
                    .as_ref()
                    .is_some_and(|helper| self.callee_identity(&file.path, helper).is_none())
                {
                    let mut generic = def.clone();
                    generic.dispatch_arguments = vec![ClassName::Unknown];
                    generic.uncertainty.insert(OopUncertainty::DynamicSignature);
                    self.uncertain_generics.insert(
                        OopId {
                            path: file.path.clone(),
                            key: def.key.clone(),
                        },
                        generic,
                    );
                }
            }
        }
    }

    /// Project-confirmed dispatch shape. Source facts retain literal candidates
    /// and their helper origins even when a mask makes the shape unknown.
    pub fn generic(&self, id: &OopId) -> Option<&GenericDef> {
        if let Some(generic) = self.uncertain_generics.get(id) {
            return Some(generic);
        }
        self.files
            .get(&id.path)?
            .facts
            .oop
            .generics
            .iter()
            .find(|def| def.key == id.key)
    }

    /// Project-confirmed dispatch shape; unconfirmed helpers yield an unknown
    /// signature rather than treating the call's literal arguments as its value.
    pub fn method(&self, id: &OopId) -> Option<&MethodDef> {
        if let Some(method) = self.uncertain_methods.get(id) {
            return Some(method);
        }
        self.files
            .get(&id.path)?
            .facts
            .oop
            .methods
            .iter()
            .chain(self.files.get(&id.path)?.facts.oop.possible_methods.iter())
            .find(|def| def.key == id.key)
    }

    pub fn member(&self, id: &OopId) -> Option<&MemberDef> {
        self.files
            .get(&id.path)?
            .facts
            .oop
            .members
            .iter()
            .find(|def| def.key == id.key)
    }

    /// Candidate identities preserve duplicate declarations and uncertainty.
    /// Qualified and imported references require an exported declaration in a
    /// named workspace package; a matching name alone is not sufficient.
    pub fn generic_candidates(
        &self,
        path: &Path,
        reference: &GenericRef,
        in_function: bool,
    ) -> Vec<OopId> {
        let Some(file) = self.files.get(path) else {
            return Vec::new();
        };
        if let Some(key) = &reference.local {
            let id = OopId {
                path: path.to_path_buf(),
                key: key.clone(),
            };
            return self
                .generic(&id)
                .filter(|def| self.generic_confirmed(path, def))
                .map(|_| id)
                .into_iter()
                .collect();
        }
        let ClassName::Known(name) = &reference.name else {
            return Vec::new();
        };
        if reference.package.is_some() {
            return self
                .namespace
                .generic_files(path, name, reference.package.as_deref())
                .iter()
                .filter_map(|path| self.files.get(path))
                .flat_map(|file| {
                    file.facts
                        .oop
                        .generics
                        .iter()
                        .filter(|def| {
                            def.key.owner.0.is_empty()
                                && def.key.system == reference.system
                                && self.generic_confirmed(&file.path, def)
                                && if reference.by_binding {
                                    file.facts.oop.generic_bindings.iter().any(|binding| {
                                        binding.target == def.key
                                            && &binding.name == name
                                            && binding.owner.0.is_empty()
                                    })
                                } else {
                                    def.name == reference.name
                                }
                        })
                        .map(|def| OopId {
                            path: file.path.clone(),
                            key: def.key.clone(),
                        })
                })
                .collect();
        }
        let matches = |file: &ClassFile, def: &GenericDef, owner: &[OwnerSegment]| {
            def.key.system == reference.system
                && def.key.owner.0 == owner
                && self.generic_confirmed(&file.path, def)
                && if reference.by_binding {
                    file.facts.oop.generic_bindings.iter().any(|binding| {
                        binding.target == def.key
                            && &binding.name == name
                            && binding.owner.0 == owner
                    })
                } else {
                    def.name == reference.name
                }
        };
        for depth in (1..=reference.owner.0.len()).rev() {
            let local: Vec<_> = file
                .facts
                .oop
                .generics
                .iter()
                .filter(|def| matches(file, def, &reference.owner.0[..depth]))
                .map(|def| OopId {
                    path: path.to_path_buf(),
                    key: def.key.clone(),
                })
                .collect();
            if !local.is_empty() {
                return local;
            }
        }
        let sources = reference
            .by_binding
            .then(|| self.definition_files(path, name, in_function));
        let mut candidates: Vec<_> = std::iter::once(file)
            .chain(
                self.scope
                    .sees(path)
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter(|candidate| {
                        !self.incomplete.contains(path)
                            && (!reference.by_binding
                                || sources
                                    .as_ref()
                                    .is_some_and(|sources| sources.contains(*candidate)))
                    })
                    .filter_map(|path| self.files.get(path)),
            )
            .flat_map(|file| {
                file.facts
                    .oop
                    .generics
                    .iter()
                    .filter(|def| matches(file, def, &[]))
                    .map(|def| OopId {
                        path: file.path.clone(),
                        key: def.key.clone(),
                    })
            })
            .collect();
        if candidates.is_empty()
            && !self.incomplete.contains(path)
            && !std::iter::once(file)
                .chain(
                    self.scope
                        .sees(path)
                        .iter()
                        .filter_map(|path| self.files.get(path)),
                )
                .any(|file| file.exports.contains(name))
        {
            for target in self
                .namespace
                .generic_files(path, name, None)
                .iter()
                .filter_map(|path| self.files.get(path))
            {
                candidates.extend(
                    target
                        .facts
                        .oop
                        .generics
                        .iter()
                        .filter(|def| matches(target, def, &[]))
                        .map(|def| OopId {
                            path: target.path.clone(),
                            key: def.key.clone(),
                        }),
                );
            }
        }
        candidates
    }

    pub(in crate::project::classes) fn generic_confirmed(
        &self,
        path: &Path,
        def: &GenericDef,
    ) -> bool {
        let (package, name) = match def.key.system {
            OopSystem::S3 => ("base", "UseMethod"),
            OopSystem::S4 => ("methods", "setGeneric"),
            OopSystem::S7 => ("S7", "new_generic"),
            _ => return false,
        };
        self.callee_identity(path, &def.factory)
            .is_some_and(|identity| identity.package == package && identity.name == name)
    }

    /// Return registrations whose symbolic generic can reach this declaration.
    /// This does not select a runtime implementation or rank dispatch signatures.
    pub fn methods_for_generic(&self, generic: &OopId) -> Vec<OopId> {
        if self.generic(generic).is_none() {
            return Vec::new();
        }
        self.files
            .values()
            .flat_map(|file| {
                file.facts
                    .oop
                    .methods
                    .iter()
                    .filter(|method| {
                        method.generic.as_ref().is_some_and(|reference| {
                            self.generic_candidates(
                                &file.path,
                                reference,
                                !method.key.owner.0.is_empty(),
                            )
                            .contains(generic)
                        }) && method.registration.as_ref().is_none_or(|registration| {
                            self.callee_identity(&file.path, registration).is_some()
                        })
                    })
                    .map(|method| OopId {
                        path: file.path.clone(),
                        key: method.key.clone(),
                    })
            })
            .collect()
    }

    /// Collect direct and inherited declarations without choosing between
    /// duplicate members, visibility domains, or competing parent branches.
    /// Consumers apply the method environment's access and shadowing rules.
    pub fn member_candidates(&self, class: &ClassId, name: &str) -> Vec<OopId> {
        if class.key.system == ClassSystem::R6
            && let Some(api) = self.r6_api(class)
        {
            return [&api.public, &api.private]
                .into_iter()
                .filter_map(|members| members.entries.get(name))
                .flatten()
                .filter_map(|member| member.declaration.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        }
        if class.key.system == ClassSystem::RefClass
            && let Some(api) = self.reference_api(class)
        {
            return [&api.instance, &api.slots]
                .into_iter()
                .filter_map(|members| members.entries.get(name))
                .flatten()
                .filter_map(|member| member.declaration.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        }
        let mut pending = vec![class.clone()];
        let mut visited = BTreeSet::new();
        let mut result = BTreeSet::new();
        while let Some(class) = pending.pop() {
            if !visited.insert(class.clone()) || !self.declarations.contains_key(&class) {
                continue;
            }
            if let Some(file) = self.files.get(&class.path) {
                result.extend(
                    file.facts
                        .oop
                        .member_candidates(&class.key, name)
                        .into_iter()
                        .map(|def| OopId {
                            path: class.path.clone(),
                            key: def.key.clone(),
                        }),
                );
            }
            pending.extend(self.supertypes.get(&class).into_iter().flatten().cloned());
        }
        result.into_iter().collect()
    }
}
