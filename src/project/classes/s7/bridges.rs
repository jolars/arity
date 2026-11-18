use super::dispatch::SignatureClass;
use super::*;

impl ClassIndex {
    pub(in crate::project::classes) fn install_s7_bridges(&mut self) {
        let mut registered = BTreeSet::new();
        let mut edges = Vec::new();
        let mut adapters = BTreeMap::new();
        for (id, declaration) in &self.declarations {
            if !declaration.declaration.binding_known()
                || !id.key.owner.0.is_empty()
                || !self.class_file_complete(&id.path)
            {
                continue;
            }
            let development = self.s7_version(&id.path) == S7Version::Development;
            if let Some(spec) = self.s7_class(id).and_then(|class| class.parent.as_ref())
                && let Some(parent) = self.s7_spec_class(&id.path, &id.key.owner, spec)
            {
                let formal_parent =
                    matches!(parent.key.system, ClassSystem::S4 | ClassSystem::RefClass);
                if development && self.s7_has_s4_ancestor(parent.clone()) {
                    registered.insert(id.clone());
                }
                if (development && formal_parent)
                    || (parent.key.system == ClassSystem::S7
                        && matches!(spec, S7ClassSpec::External { .. }))
                {
                    edges.push((id.clone(), parent));
                }
            }
            if !development {
                continue;
            }
            let Some(file) = self.file(&id.path) else {
                continue;
            };
            if let Some(adapter) = file.facts.oop.s7.s4_contains.get(&id.key)
                && adapter.unconditional
                && self.s7_helper(&id.path, &adapter.factory, "S7").is_some()
                && let Some(parent) = self.s7_spec_class(&id.path, &id.key.owner, &adapter.class)
                && parent.key.system == ClassSystem::S7
                && self.s7_registered_before(id, &parent)
            {
                adapters.insert(id.clone(), parent.clone());
                edges.push((id.clone(), parent));
            }
        }
        for file in self.oop_files() {
            for registration in &file.facts.oop.s7.s4_registrations {
                if registration.unconditional
                    && self
                        .s7_helper(&file.path, &registration.factory, "S7")
                        .is_some()
                    && let Some(class) =
                        self.s7_spec_class(&file.path, &ClassOwner::default(), &registration.class)
                    && class.key.system == ClassSystem::S7
                {
                    registered.insert(class);
                }
            }
        }
        edges.sort_by_key(|(id, _)| id.key.system != ClassSystem::S7);
        for (id, parent) in edges {
            let invalid_adapter = adapters.contains_key(&id)
                && !self.s7_api(&parent).is_some_and(|api| {
                    api.complete
                        && api.properties.iter().all(|property| {
                            property.definition.getter == S7Function::Absent
                                && property.definition.setter == S7Function::Absent
                        })
                });
            if invalid_adapter || self.s7_would_cycle(&id, &parent) {
                adapters.remove(&id);
                continue;
            }
            self.supertypes
                .entry(id.clone())
                .or_default()
                .insert(parent.clone());
            self.subtypes.entry(parent).or_default().insert(id);
        }
        self.s7_s4_parents = adapters;
        self.s7_s4_registered = registered;
    }

    fn s7_has_s4_ancestor(&self, mut parent: ClassId) -> bool {
        let mut seen = BTreeSet::new();
        loop {
            if matches!(parent.key.system, ClassSystem::S4 | ClassSystem::RefClass) {
                return true;
            }
            if !seen.insert(parent.clone()) {
                return false;
            }
            let Some(next) = self
                .s7_class(&parent)
                .and_then(|class| class.parent.as_ref())
                .and_then(|spec| self.s7_spec_class(&parent.path, &parent.key.owner, spec))
            else {
                return false;
            };
            parent = next;
        }
    }

    fn s7_would_cycle(&self, child: &ClassId, parent: &ClassId) -> bool {
        let mut pending = vec![parent];
        let mut seen = BTreeSet::new();
        while let Some(candidate) = pending.pop() {
            if candidate == child {
                return true;
            }
            if seen.insert(candidate) {
                pending.extend(self.supertypes.get(candidate).into_iter().flatten());
            }
        }
        false
    }

    fn s7_registered_before(&self, child: &ClassId, parent: &ClassId) -> bool {
        let execution = self.execution(&child.path);
        if !execution.complete {
            return false;
        }
        for (path, ordinal) in &execution.events {
            let Some(file) = self.file(path) else {
                continue;
            };
            match &file.facts.oop.events[*ordinal] {
                OopEvent::Class(key) if *path == child.path && *key == child.key => return false,
                OopEvent::S7S4Bridge(ordinal) => {
                    let registration = &file.facts.oop.s7.s4_registrations[*ordinal];
                    if registration.unconditional
                        && self.s7_helper(path, &registration.factory, "S7").is_some()
                        && self
                            .s7_spec_class(path, &ClassOwner::default(), &registration.class)
                            .as_ref()
                            == Some(parent)
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    pub fn s7_registered_with_s4(&self, class: &ClassId) -> bool {
        self.s7_s4_registered.contains(class)
    }

    pub(in crate::project::classes) fn s7_s4_slots(&self, id: &ClassId) -> Option<S4Slots> {
        let parent = self.s7_s4_parents.get(id)?;
        let api = self.s7_api(parent)?;
        let mut slots = S4Slots {
            complete: api.complete,
            ..S4Slots::default()
        };
        for property in api.properties {
            if property.definition.getter != S7Function::Absent
                || property.definition.setter != S7Function::Absent
            {
                continue;
            }
            let declared_type = match property.declared_type(self).as_deref() {
                None | Some("any") => DispatchClass::Any,
                Some("double") => DispatchClass::Formal("numeric".into()),
                Some(name)
                    if matches!(
                        name,
                        "logical"
                            | "integer"
                            | "numeric"
                            | "character"
                            | "complex"
                            | "raw"
                            | "list"
                            | "expression"
                            | "environment"
                            | "function"
                    ) =>
                {
                    DispatchClass::Formal(name.into())
                }
                _ => {
                    slots.complete = false;
                    DispatchClass::Unknown
                }
            };
            slots.slot_entries(&property.name, false).push(S4Slot {
                declaration: property.declaration,
                declared_type,
            });
        }
        Some(slots)
    }

    pub(in crate::project::classes) fn install_s7_s3_registrations(&mut self) {
        let mut registrations = Vec::new();
        for file in self.oop_files() {
            for (key, definition) in &file.facts.oop.s7.methods {
                let [argument] = self
                    .method(&OopId {
                        path: file.path.clone(),
                        key: key.clone(),
                    })
                    .map(|method| method.dispatch.as_slice())
                    .unwrap_or_default()
                else {
                    continue;
                };
                let source = OopId {
                    path: file.path.clone(),
                    key: key.clone(),
                };
                let Some(resolved) = self.s7_registration(&source) else {
                    continue;
                };
                let [S7GenericTarget::S3(target)] = resolved.targets.as_slice() else {
                    continue;
                };
                if definition.removal
                    || resolved.delayed
                        && file
                            .package_root
                            .as_ref()
                            .is_some_and(|root| !self.s7_has_hook(root, S7HookKind::Load))
                {
                    continue;
                }
                let Some(tags) = self.s7_signature_classes(&source, &argument.class) else {
                    continue;
                };
                let method = self.method(&source).expect("S7 registration has a method");
                let Some(mut generic) = method.generic.clone() else {
                    continue;
                };
                generic.system = OopSystem::S3;
                generic.local = None;
                let mut uncertainty = method.uncertainty.clone();
                uncertainty.remove(&OopUncertainty::UnresolvedCallee);
                for tag in tags {
                    let class = match tag {
                        SignatureClass::Tag(tag) if tag != "MISSING" && !tag.starts_with("S4/") => {
                            tag
                        }
                        SignatureClass::Any => "default".into(),
                        _ => continue,
                    };
                    registrations.push(S3Method {
                        source: S3MethodSource::Declaration(source.clone()),
                        owner: S3RegistrationOwner {
                            path: file.path.clone(),
                            package_root: file.package_root.clone(),
                            lexical: key.owner.clone(),
                        },
                        generic: generic.clone(),
                        class: ClassName::Known(class),
                        implementation: resolved.implementation.clone(),
                        generics: vec![target.clone()],
                        registered: true,
                        delayed: resolved.delayed,
                        exported: false,
                        uncertainty: uncertainty.clone(),
                    });
                }
            }
        }
        self.s3.methods.extend(registrations);
    }
}

impl ClassIndex {
    pub(in crate::project::classes) fn install_s7_s4_registrations(&mut self) {
        let mut methods = Vec::new();
        for file in self.oop_files() {
            for (key, definition) in &file.facts.oop.s7.methods {
                if definition.removal {
                    continue;
                }
                let id = OopId {
                    path: file.path.clone(),
                    key: key.clone(),
                };
                let Some(registration) = self.s7_registration(&id) else {
                    continue;
                };
                let [S7GenericTarget::S4(target)] = registration.targets.as_slice() else {
                    continue;
                };
                let Some(method) = self.method(&id) else {
                    continue;
                };
                let Some(generic) = method.generic.as_ref() else {
                    continue;
                };
                if registration.delayed
                    && file
                        .package_root
                        .as_ref()
                        .is_some_and(|root| !self.s7_has_hook(root, S7HookKind::Load))
                {
                    continue;
                }
                let supplied_signature: Vec<_> = method
                    .dispatch
                    .iter()
                    .map(|argument| DispatchArgument {
                        argument: None,
                        class: self
                            .s7_s4_signature_class(&id, &argument.class)
                            .unwrap_or(DispatchClass::Unknown),
                    })
                    .collect();
                let dispatch: Vec<_> = self
                    .s4
                    .generics
                    .iter()
                    .filter(|candidate| candidate.id == *target)
                    .map(|candidate| s4_method_signature(candidate, &supplied_signature))
                    .collect();
                let mut uncertainty = method.uncertainty.clone();
                uncertainty.remove(&OopUncertainty::UnresolvedCallee);
                if supplied_signature
                    .iter()
                    .any(|argument| argument.class == DispatchClass::Unknown)
                {
                    uncertainty.insert(OopUncertainty::DynamicSignature);
                }
                methods.push(S4Method {
                    id: S4MethodId::Registration(id),
                    owner: S4MethodOwner {
                        path: file.path.clone(),
                        package_root: file.package_root.clone(),
                        package: registration.owner_package,
                        lexical: key.owner.clone(),
                    },
                    generic: GenericRef {
                        system: OopSystem::S4,
                        ..generic.clone()
                    },
                    generics: vec![target.clone()],
                    supplied_signature,
                    dispatch,
                    signature: method.signature.clone(),
                    implementation: registration.implementation,
                    uncertainty,
                });
            }
        }
        self.s4.methods.extend(methods);
    }

    fn s7_s4_signature_class(&self, id: &OopId, class: &DispatchClass) -> Option<DispatchClass> {
        let spec = match class {
            DispatchClass::Generator(generator) => S7ClassSpec::Generator(generator.clone()),
            DispatchClass::S7(spec) => spec.clone(),
            _ => return None,
        };
        if let Some(target) = self.s7_spec_class(&id.path, &id.key.owner, &spec) {
            if target.key.system == ClassSystem::S7 {
                return self
                    .s7_registered_with_s4(&target)
                    .then(|| self.s7_class_tag(&target).map(DispatchClass::Formal))
                    .flatten();
            }
            return matches!(target.key.system, ClassSystem::S4 | ClassSystem::RefClass).then(
                || {
                    DispatchClass::Formal(
                        self.declarations[&target]
                            .declaration
                            .display_name()
                            .to_string(),
                    )
                },
            );
        }
        let development = self.s7_version(&id.path) == S7Version::Development;
        let tags = self.s7_signature_classes(id, class)?;
        match tags.as_slice() {
            [SignatureClass::Any] if development => Some(DispatchClass::Any),
            [SignatureClass::Tag(tag)] if tag == "MISSING" && development => {
                Some(DispatchClass::Missing)
            }
            [SignatureClass::Tag(tag)] if tag == "NULL" && development => {
                Some(DispatchClass::Formal(tag.clone()))
            }
            [SignatureClass::Tag(tag)]
                if matches!(spec, S7ClassSpec::Builtin(_))
                    && !matches!(tag.as_str(), "S7_object" | "MISSING") =>
            {
                let name = if development && tag == "double" {
                    "numeric"
                } else {
                    tag
                };
                Some(DispatchClass::Formal(name.into()))
            }
            _ => None,
        }
    }
}
