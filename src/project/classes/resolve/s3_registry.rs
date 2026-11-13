//! S3 declaration and registration identities, without simulating dispatch.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, salsa::SalsaValue)]
pub enum S3GenericKind {
    Explicit,
    Known,
    Internal,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, salsa::SalsaValue)]
pub enum S3GenericId {
    Declaration(OopId),
    Known { package: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct S3Generic {
    pub id: S3GenericId,
    pub name: String,
    pub binding: Option<String>,
    pub kind: S3GenericKind,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub enum S3MethodSource {
    Declaration(OopId),
    Namespace { root: PathBuf, occurrence: usize },
}

/// The package installing a registration need not own its generic or export its
/// implementation. Script registrations also retain their lexical owner.
#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct S3RegistrationOwner {
    pub path: PathBuf,
    pub package_root: Option<PathBuf>,
    pub lexical: ClassOwner,
}

#[derive(Debug, Clone, PartialEq, Eq, salsa::SalsaValue)]
pub struct S3Method {
    pub source: S3MethodSource,
    pub owner: S3RegistrationOwner,
    pub generic: GenericRef,
    pub class: ClassName,
    pub implementation: Option<MethodImplementationSource>,
    /// All statically visible generic identities; this never selects a method.
    pub generics: Vec<S3GenericId>,
    pub registered: bool,
    pub delayed: bool,
    pub exported: bool,
    pub uncertainty: BTreeSet<OopUncertainty>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, salsa::SalsaValue)]
pub struct S3Registry {
    pub generics: Vec<S3Generic>,
    pub methods: Vec<S3Method>,
}

impl ClassIndex {
    pub(super) fn build_s3_registry(&self, resolution: &ClassResolution<'_>) -> S3Registry {
        let mut registry = S3Registry::default();
        for line in include_str!("../../../semantic/base_r/s3_generics.tsv")
            .lines()
            .skip(1)
        {
            let columns: Vec<_> = line.split('\t').collect();
            let [name, package, kind] = columns.as_slice() else {
                continue;
            };
            registry.generics.push(S3Generic {
                id: S3GenericId::Known {
                    package: (*package).into(),
                    name: (*name).into(),
                },
                name: (*name).into(),
                binding: None,
                uncertainty: BTreeSet::new(),
                kind: match *kind {
                    "group" => S3GenericKind::Group,
                    "internal" => S3GenericKind::Internal,
                    _ => S3GenericKind::Known,
                },
            });
        }
        for file in self.files.values() {
            for generic in &file.facts.oop.generics {
                if generic.key.system == OopSystem::S3
                    && self.generic_confirmed(&file.path, generic)
                    && let ClassName::Known(name) = &generic.name
                {
                    registry.generics.push(S3Generic {
                        id: S3GenericId::Declaration(OopId {
                            path: file.path.clone(),
                            key: generic.key.clone(),
                        }),
                        name: name.clone(),
                        binding: generic.binding.clone(),
                        kind: S3GenericKind::Explicit,
                        uncertainty: generic
                            .uncertainty
                            .iter()
                            .copied()
                            .filter(|issue| *issue != OopUncertainty::UnresolvedCallee)
                            .collect(),
                    });
                }
            }
        }
        for file in self.files.values() {
            for declaration in &file.facts.oop.methods {
                let id = OopId {
                    path: file.path.clone(),
                    key: declaration.key.clone(),
                };
                let Some(method) = self.method(&id) else {
                    continue;
                };
                if method.system != OopSystem::S3 {
                    continue;
                }
                let Some(reference) = &method.generic else {
                    continue;
                };
                let registered = method.registration.is_some();
                if method.registration.as_ref().is_some_and(|callee| {
                    self.callee_availability(&file.path, callee) == Availability::Absent
                }) {
                    continue;
                }
                let implementation = if registered {
                    file.facts
                        .oop
                        .s3_implementations
                        .iter()
                        .find(|implementation| implementation.method == method.key)
                        .map(|implementation| implementation.source.clone())
                } else {
                    match &method.name {
                        ClassName::Known(name) => Some(MethodImplementationSource::Binding {
                            owner: method.key.owner.clone(),
                            name: name.clone(),
                        }),
                        _ => None,
                    }
                };
                let owner = S3RegistrationOwner {
                    path: file.path.clone(),
                    package_root: file.package_root.clone(),
                    lexical: method.key.owner.clone(),
                };
                let source = S3MethodSource::Declaration(OopId {
                    path: file.path.clone(),
                    key: method.key.clone(),
                });
                let class = method
                    .dispatch
                    .first()
                    .map_or(ClassName::Unknown, |argument| match &argument.class {
                        DispatchClass::S3(name) => ClassName::Known(name.clone()),
                        _ => ClassName::Unknown,
                    });
                let mut pairs = vec![(reference.clone(), class)];
                if !registered && let ClassName::Known(binding) = &method.name {
                    // Both names may contain dots. Preserve each possible split
                    // until generic visibility supplies stronger evidence.
                    for (offset, _) in binding.char_indices().filter(|(offset, ch)| {
                        *ch == '.' && *offset > 0 && *offset + 1 < binding.len()
                    }) {
                        let reference = GenericRef {
                            name: ClassName::Known(binding[..offset].into()),
                            local: None,
                            ..reference.clone()
                        };
                        let pair = (reference, ClassName::Known(binding[offset + 1..].into()));
                        if !pairs.contains(&pair) {
                            pairs.push(pair);
                        }
                    }
                }
                for (reference, class) in pairs {
                    let generics = self.s3_generic_candidates(
                        file,
                        &reference,
                        &registry.generics,
                        resolution,
                    );
                    let mut uncertainty = method.uncertainty.clone();
                    if method
                        .registration
                        .as_ref()
                        .is_some_and(|callee| self.callee_identity(&file.path, callee).is_some())
                    {
                        uncertainty.remove(&OopUncertainty::UnresolvedCallee);
                    }
                    if generics.is_empty() {
                        uncertainty.insert(OopUncertainty::UnprovenMethod);
                    } else {
                        uncertainty.remove(&OopUncertainty::UnprovenMethod);
                    }
                    registry.methods.push(S3Method {
                        source: source.clone(),
                        owner: owner.clone(),
                        generic: reference,
                        class,
                        exported: exported_implementation_name(implementation.as_ref())
                            .is_some_and(|name| {
                                self.scope.for_file(&file.path).exported_by_namespace(name)
                            }),
                        implementation: implementation.clone(),
                        generics,
                        registered,
                        delayed: false,
                        uncertainty,
                    });
                }
            }
        }
        for (root, text) in resolution.namespaces {
            let info = crate::rindex::harvest::parse_namespace(
                text,
                &self
                    .files
                    .values()
                    .filter(|file| file.package_root.as_ref() == Some(root))
                    .flat_map(|file| file.exports.iter().map(String::as_str))
                    .collect::<Vec<_>>(),
            );
            for (occurrence, registration) in info.s3_registrations.into_iter().enumerate() {
                let reference = GenericRef {
                    system: OopSystem::S3,
                    owner: ClassOwner::default(),
                    name: ClassName::Known(registration.generic),
                    package: registration.package.clone(),
                    by_binding: false,
                    local: None,
                    lookup: None,
                };
                let mut generics = BTreeSet::new();
                for file in self
                    .files
                    .values()
                    .filter(|file| file.package_root.as_ref() == Some(root))
                {
                    generics.extend(self.s3_generic_candidates(
                        file,
                        &reference,
                        &registry.generics,
                        resolution,
                    ));
                }
                let mut uncertainty = BTreeSet::new();
                if registration.conditional {
                    uncertainty.insert(OopUncertainty::Conditional);
                }
                if generics.is_empty() {
                    uncertainty.insert(OopUncertainty::UnprovenMethod);
                }
                registry.methods.push(S3Method {
                    source: S3MethodSource::Namespace {
                        root: root.clone(),
                        occurrence,
                    },
                    owner: S3RegistrationOwner {
                        path: root.join("NAMESPACE"),
                        package_root: Some(root.clone()),
                        lexical: ClassOwner::default(),
                    },
                    generic: reference,
                    class: ClassName::Known(registration.class),
                    implementation: Some(MethodImplementationSource::Binding {
                        owner: ClassOwner::default(),
                        name: registration.method.clone(),
                    }),
                    generics: generics.into_iter().collect(),
                    registered: true,
                    delayed: registration.package.is_some(),
                    exported: info.exports.contains(&registration.method),
                    uncertainty,
                });
            }
        }
        registry
    }

    fn s3_generic_candidates(
        &self,
        file: &ClassFile,
        reference: &GenericRef,
        generics: &[S3Generic],
        resolution: &ClassResolution<'_>,
    ) -> Vec<S3GenericId> {
        // Group dispatch and registerS3method special-case these identities
        // without consulting ordinary bindings of the same name.
        if let Some(group) = generics.iter().find(|generic| {
            generic.kind == S3GenericKind::Group
                && reference.name == ClassName::Known(generic.name.clone())
                && matches!(&generic.id, S3GenericId::Known { package, .. }
                    if reference.package.as_ref().is_none_or(|qualified| qualified == package))
        }) {
            return vec![group.id.clone()];
        }
        if reference.local.is_none() && matches!(reference.lookup, Some(FunctionRef::Unresolved(_)))
        {
            return Vec::new();
        }
        let qualified = if reference.local.is_none()
            && let Some(FunctionRef::Qualified { package, name }) = &reference.lookup
        {
            if reference.name != ClassName::Known(name.clone()) {
                return Vec::new();
            }
            Some(GenericRef {
                package: Some(package.clone()),
                ..reference.clone()
            })
        } else {
            None
        };
        let reference = qualified.as_ref().unwrap_or(reference);
        let mut candidates: Vec<_> = self
            .generic_candidates(&file.path, reference, !reference.owner.0.is_empty())
            .into_iter()
            .map(S3GenericId::Declaration)
            .collect();
        let ClassName::Known(name) = &reference.name else {
            return candidates;
        };
        if !candidates.is_empty() {
            return candidates;
        }
        for generic in generics.iter().filter(|generic| &generic.name == name) {
            let S3GenericId::Known { package, .. } = &generic.id else {
                continue;
            };
            let known = if let Some(qualified) = &reference.package {
                qualified == package
            } else if file.exports.contains(name)
                || !self.definition_files(&file.path, name, true).is_empty()
            {
                false
            } else {
                let fallback = FunctionRef::Bare {
                    name: name.clone(),
                    attached: Vec::new(),
                    attachment_uncertain: false,
                };
                function_availability(
                    file,
                    reference.lookup.as_ref().unwrap_or(&fallback),
                    package,
                    name,
                    &self.files,
                    resolution,
                ) == Availability::Present
            };
            if known {
                candidates.push(generic.id.clone());
            }
        }
        candidates
    }
}

fn exported_implementation_name(
    implementation: Option<&MethodImplementationSource>,
) -> Option<&str> {
    match implementation? {
        MethodImplementationSource::Binding { owner, name } if owner.0.is_empty() => Some(name),
        MethodImplementationSource::Reference { name } => Some(name),
        _ => None,
    }
}
