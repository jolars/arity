//! Static R6 surfaces, resolved through generator identities rather than S3 names.
//!
//! R6 merges fields, methods, and active bindings separately. Fields are then
//! installed after methods; `super` contains methods and active bindings only.
//! Keep those layers until installation so cross-kind overrides follow R6 too.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct R6Options {
    pub class_attribute: Option<bool>,
    pub cloneable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R6Member {
    /// Generated infrastructure has no editable source declaration.
    pub declaration: Option<OopId>,
    pub kind: MemberKind,
    /// For active bindings these are accessor formals, not a callable field.
    pub signature: Option<CallableSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R6Members {
    /// Duplicates remain candidates; malformed lists never select a winner.
    pub entries: BTreeMap<String, Vec<R6Member>>,
    pub complete: bool,
}

impl Default for R6Members {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            complete: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R6Api {
    pub generator: R6Members,
    pub public: R6Members,
    pub private: R6Members,
    /// Relative to this class's method enclosure, not the most-derived class.
    pub super_members: R6Members,
    /// Effective public initialize formals forwarded by the generator's `new(...)`.
    pub initializer: Option<CallableSignature>,
    /// `Some([])` means no explicit class attribute, as with `class = FALSE`.
    pub class_attribute: Option<Vec<String>>,
}

pub type R6Access = ClassAccess;
pub type R6Receiver = ClassReceiver;

impl R6Api {
    pub fn members(&self, access: R6Access) -> &R6Members {
        match access {
            R6Access::Generator => &self.generator,
            R6Access::Public => &self.public,
            R6Access::Private => &self.private,
            R6Access::Super => &self.super_members,
        }
    }
}

#[derive(Default)]
struct Layers {
    public_fields: R6Members,
    public_methods: R6Members,
    private_fields: R6Members,
    private_methods: R6Members,
    active: R6Members,
    names: Option<Vec<String>>,
}

impl Layers {
    fn incomplete(&mut self) {
        for members in [
            &mut self.public_fields,
            &mut self.public_methods,
            &mut self.private_fields,
            &mut self.private_methods,
            &mut self.active,
        ] {
            members.complete = false;
        }
        self.names = None;
    }

    fn public(&self) -> R6Members {
        let mut out = self.public_methods.clone();
        overlay(&mut out, &self.public_fields);
        install_active(&mut out, &self.active);
        out
    }

    fn private(&self) -> R6Members {
        let mut out = self.private_methods.clone();
        overlay(&mut out, &self.private_fields);
        out
    }

    fn super_members(&self) -> R6Members {
        let mut out = self.public_methods.clone();
        overlay(&mut out, &self.private_methods);
        install_active(&mut out, &self.active);
        out
    }
}

fn overlay(target: &mut R6Members, source: &R6Members) {
    target.entries.extend(source.entries.clone());
    target.complete &= source.complete;
}

fn install_active(target: &mut R6Members, active: &R6Members) {
    // makeActiveBinding refuses an already-installed ordinary binding.
    if active
        .entries
        .keys()
        .any(|name| target.entries.contains_key(name))
    {
        target.complete = false;
    }
    overlay(target, active);
}

fn signature(parameters: &[(&str, Option<&str>)]) -> CallableSignature {
    CallableSignature {
        parameters: parameters
            .iter()
            .map(|(name, default)| CallableParameter {
                name: (*name).into(),
                default: default.map(str::to_owned),
            })
            .collect(),
    }
}

fn generated(kind: MemberKind, parameters: Option<&[(&str, Option<&str>)]>) -> R6Member {
    R6Member {
        declaration: None,
        kind,
        signature: parameters.map(signature),
    }
}

fn generator_members() -> R6Members {
    let mut members = R6Members::default();
    for name in [
        "classname",
        "public_fields",
        "private_fields",
        "public_methods",
        "private_methods",
        "active",
        "inherit",
        "portable",
        "cloneable",
        "class",
        "parent_env",
        "lock_objects",
        "lock_class",
        "debug_names",
        "self",
    ] {
        members
            .entries
            .insert(name.into(), vec![generated(MemberKind::Field, None)]);
    }
    for (name, parameters) in [
        ("new", vec![("...", None)]),
        (
            "set",
            vec![
                ("which", Some("NULL")),
                ("name", Some("NULL")),
                ("value", None),
                ("overwrite", Some("FALSE")),
            ],
        ),
        ("debug", vec![("name", None)]),
        ("undebug", vec![("name", None)]),
        ("lock", vec![]),
        ("unlock", vec![]),
        ("is_locked", vec![]),
        ("get_inherit", vec![]),
        ("has_private", vec![]),
        ("clone_method", vec![("deep", Some("FALSE"))]),
    ] {
        members.entries.insert(
            name.into(),
            vec![generated(MemberKind::Method, Some(&parameters))],
        );
    }
    members
}

impl ClassIndex {
    /// Bounded evidence for generators, direct construction, stable local
    /// aliases, and proven method enclosures. Reassignment or lexical shadowing
    /// prevents a receiver from acquiring an unrelated class's API.
    pub fn r6_receiver(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<R6Receiver> {
        self.class_receiver(path, root, model, value)
            .filter(|receiver| receiver.class.key.system == ClassSystem::R6)
    }

    /// Effective workspace R6 APIs. Unknown parents and member lists remain
    /// incomplete. No source or accessor is evaluated to discover a member.
    pub fn r6_api(&self, id: &ClassId) -> Option<R6Api> {
        let class = &self.declarations.get(id)?.declaration;
        let options = class.r6.as_ref()?;
        let layers = self.r6_layers(id, &mut BTreeSet::new());
        let mut public = layers.public();
        // R6 removes clone when the subclass and its immediate parent disagree,
        // even if an earlier ancestor had a different flag again.
        if !class.parents.is_empty() {
            let parents: Vec<_> = self.supertypes.get(id).into_iter().flatten().collect();
            let parent_flag = match parents.as_slice() {
                [parent] => self
                    .declarations
                    .get(*parent)
                    .and_then(|parent| parent.declaration.r6.as_ref())
                    .and_then(|options| options.cloneable),
                _ => None,
            };
            match (options.cloneable, parent_flag) {
                (Some(own), Some(parent)) if own != parent => {
                    public.entries.remove("clone");
                }
                (Some(_), Some(_)) => {}
                _ => {
                    public.complete = false;
                }
            }
        }
        let super_members = match self
            .supertypes
            .get(id)
            .map(|parents| parents.iter().collect::<Vec<_>>())
            .as_deref()
        {
            Some([parent]) => self.r6_layers(parent, &mut BTreeSet::new()).super_members(),
            _ => R6Members {
                complete: class.parents.is_empty(),
                ..R6Members::default()
            },
        };
        let initializer = if !public.complete {
            None
        } else {
            match public.entries.get("initialize").map(Vec::as_slice) {
                Some([member]) if member.kind == MemberKind::Method => member.signature.clone(),
                Some([member]) if member.kind == MemberKind::Field => Some(signature(&[])),
                None => Some(signature(&[])),
                _ => None,
            }
        };
        let class_attribute = match options.class_attribute {
            Some(false) => Some(Vec::new()),
            Some(true) => layers.names.clone().map(|mut names| {
                names.push("R6".into());
                names
            }),
            None => None,
        };
        Some(R6Api {
            generator: generator_members(),
            public,
            private: layers.private(),
            super_members,
            initializer,
            class_attribute,
        })
    }

    fn r6_layers(&self, id: &ClassId, visiting: &mut BTreeSet<ClassId>) -> Layers {
        let mut layers = Layers {
            names: Some(Vec::new()),
            ..Layers::default()
        };
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            layers.incomplete();
            return layers;
        }
        let (Some(class), Some(file)) = (self.declarations.get(id), self.file(&id.path)) else {
            layers.incomplete();
            return layers;
        };
        let def = &class.declaration;
        if def.system() != ClassSystem::R6 {
            layers.incomplete();
            return layers;
        }
        let parents: Vec<_> = self.supertypes.get(id).into_iter().flatten().collect();
        if let [parent] = parents.as_slice() {
            layers = self.r6_layers(parent, visiting);
            if self
                .declarations
                .get(*parent)
                .is_none_or(|parent| parent.declaration.portable != def.portable)
            {
                layers.incomplete();
            }
        } else if !def.parents.is_empty() {
            layers.incomplete();
        }
        if !def.binding_known() || def.portable.is_none() {
            layers.incomplete();
        }
        if let Some(names) = &mut layers.names {
            match &def.name {
                ClassName::Known(name) => names.insert(0, name.clone()),
                ClassName::Absent => {}
                ClassName::Unknown => layers.names = None,
            }
        }
        let mut own = Layers::default();
        for group in file
            .facts
            .oop
            .member_groups
            .iter()
            .filter(|group| group.class == id.key)
        {
            let valid = group
                .factory
                .as_ref()
                .and_then(|factory| self.callee_identity(&id.path, factory))
                .is_some_and(|factory| factory.package == "base" && factory.name == "list")
                && !group.uncertainty.contains(&OopUncertainty::DynamicMembers);
            if !valid {
                match group.name.as_str() {
                    "public" => {
                        own.public_fields.complete = false;
                        own.public_methods.complete = false;
                    }
                    "private" => {
                        own.private_fields.complete = false;
                        own.private_methods.complete = false;
                    }
                    "active" => own.active.complete = false,
                    _ => {}
                }
            }
        }
        let mut names = BTreeSet::new();
        for member in file
            .facts
            .oop
            .members
            .iter()
            .filter(|member| member.class == id.key)
        {
            let ClassName::Known(name) = &member.name else {
                own.incomplete();
                continue;
            };
            let stored_list = member.kind == MemberKind::Unknown
                && member
                    .value_factory
                    .as_ref()
                    .and_then(|factory| self.callee_identity(&id.path, factory))
                    .is_some_and(|factory| factory.package == "base" && factory.name == "list");
            let kind = if member.kind == MemberKind::Unknown && stored_list {
                MemberKind::Field
            } else {
                member.kind
            };
            if !names.insert(name)
                || matches!(name.as_str(), "clone" | "self" | "super" | "private")
                || (name == "initialize"
                    && (member.visibility == MemberVisibility::Private
                        || member.kind == MemberKind::ActiveBinding))
            {
                own.incomplete();
            }
            let target = match (member.visibility, kind) {
                (_, MemberKind::ActiveBinding) => &mut own.active,
                (MemberVisibility::Public, MemberKind::Method) => &mut own.public_methods,
                (MemberVisibility::Private, MemberKind::Method) => &mut own.private_methods,
                (MemberVisibility::Public, _) => &mut own.public_fields,
                (MemberVisibility::Private, _) => &mut own.private_fields,
            };
            let signature = member
                .method
                .as_ref()
                .and_then(|key| {
                    self.method(&OopId {
                        path: id.path.clone(),
                        key: key.clone(),
                    })
                })
                .and_then(|method| method.signature.clone());
            target
                .entries
                .entry(name.clone())
                .or_default()
                .push(R6Member {
                    declaration: Some(OopId {
                        path: id.path.clone(),
                        key: member.key.clone(),
                    }),
                    kind,
                    signature,
                });
            if member
                .uncertainty
                .contains(&OopUncertainty::UnresolvedImplementation)
                && !stored_list
            {
                target.complete = false;
            }
        }
        match def.r6.as_ref().and_then(|options| options.cloneable) {
            Some(true) => {
                own.public_methods.entries.insert(
                    "clone".into(),
                    vec![generated(
                        MemberKind::Method,
                        Some(&[("deep", Some("FALSE"))]),
                    )],
                );
            }
            Some(false) => {}
            None => own.public_methods.complete = false,
        }
        overlay(&mut layers.public_fields, &own.public_fields);
        overlay(&mut layers.public_methods, &own.public_methods);
        overlay(&mut layers.private_fields, &own.private_fields);
        overlay(&mut layers.private_methods, &own.private_methods);
        overlay(&mut layers.active, &own.active);
        visiting.remove(id);
        layers
    }
}
