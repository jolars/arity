//! Reference Class APIs. Fields live in the reference environment; S4 slots
//! retain a separate, functional surface even on an S4-derived reference object.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceMember {
    /// Infrastructure has no editable source declaration.
    pub declaration: Option<OopId>,
    pub kind: MemberKind,
    pub declared_type: Option<DispatchClass>,
    /// Accessor formals describe executable reads/writes, not a callable field.
    pub signature: Option<CallableSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceMembers {
    pub entries: BTreeMap<String, Vec<ReferenceMember>>,
    pub complete: bool,
}

impl Default for ReferenceMembers {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            complete: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceApi {
    pub generator: ReferenceMembers,
    pub instance: ReferenceMembers,
    pub slots: ReferenceMembers,
    /// Effective initialize formals forwarded by both Generator(...) and $new(...).
    pub initializer: Option<CallableSignature>,
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

fn generated(kind: MemberKind, parameters: Option<&[(&str, Option<&str>)]>) -> ReferenceMember {
    ReferenceMember {
        declaration: None,
        kind,
        declared_type: None,
        signature: parameters.map(signature),
    }
}

fn object_members() -> ReferenceMembers {
    let mut members = ReferenceMembers::default();
    for name in [".self", ".refClassDef", ".objectPackage", ".objectParent"] {
        members
            .entries
            .insert(name.into(), vec![generated(MemberKind::Field, None)]);
    }
    for (name, parameters) in [
        ("callSuper", vec![("...", None)]),
        ("copy", vec![("shallow", Some("FALSE"))]),
        ("export", vec![("Class", None)]),
        ("field", vec![("name", None), ("value", None)]),
        ("getClass", vec![("...", None)]),
        ("getRefClass", vec![("Class", Some(".refClassDef"))]),
        (
            "import",
            vec![("value", None), ("Class", Some("class(value)"))],
        ),
        ("initFields", vec![("...", None)]),
        ("show", vec![]),
        ("trace", vec![("...", None), ("classMethod", Some("FALSE"))]),
        (
            "untrace",
            vec![("...", None), ("classMethod", Some("FALSE"))],
        ),
        ("usingMethods", vec![("...", None)]),
    ] {
        members.entries.insert(
            name.into(),
            vec![generated(MemberKind::Method, Some(&parameters))],
        );
    }
    members
}

fn generator_members() -> ReferenceMembers {
    let mut members = object_members();
    for name in ["def", "className"] {
        members
            .entries
            .insert(name.into(), vec![generated(MemberKind::Field, None)]);
    }
    for (name, parameters) in [
        ("new", vec![("...", None)]),
        ("methods", vec![("...", None)]),
        ("fields", vec![]),
        ("help", vec![("topic", None)]),
        ("lock", vec![("...", None)]),
        ("accessors", vec![("...", None)]),
    ] {
        members.entries.insert(
            name.into(),
            vec![generated(MemberKind::Method, Some(&parameters))],
        );
    }
    members
}

impl ClassIndex {
    pub fn reference_receiver_api(&self, receiver: &ClassReceiver) -> Option<ReferenceApi> {
        let mut api = self.reference_api_at(&receiver.class, receiver.point.as_ref())?;
        api.instance.complete &= !receiver.uncertain_cache;
        for (name, point) in &receiver.cached_members {
            let cached = self.reference_api_at(&receiver.class, Some(point))?;
            if let Some(members) = cached.instance.entries.get(name)
                && members
                    .iter()
                    .any(|member| member.kind == MemberKind::Method)
            {
                api.instance.entries.insert(name.clone(), members.clone());
                api.instance.complete &= cached.instance.complete;
            }
        }
        Some(api)
    }
    /// The standard base is supplied by methods itself, without a workspace
    /// declaration. A visible declaration with that name still needs resolution.
    pub(super) fn reference_base_parent(&self, id: &ClassId, ordinal: usize) -> bool {
        self.declarations.get(id).is_some_and(|class| {
            class.declaration.system() == ClassSystem::RefClass
                && matches!(class.declaration.parents.get(ordinal), Some(ParentRef::Formal(name)) if name == "envRefClass")
                && self.parent_candidates.get(id).and_then(|parents| parents.get(ordinal)).is_some_and(Vec::is_empty)
        })
    }

    pub(super) fn reference_lookup(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Option<ClassId> {
        let factory = call_function_ref(model, root, call)?;
        let identity = self.callee_identity(path, &factory)?;
        if identity.package != "methods" || identity.name != "getRefClass" {
            return None;
        }
        let args = matched_args(call, &["Class", "where"])?;
        if args.contains_key("where") {
            return None;
        }
        let SyntaxElement::Token(name) = argument(&args, "Class")? else {
            return None;
        };
        if name.kind() != SyntaxKind::STRING {
            return None;
        }
        let name = crate::semantic::decode_string_literal(name.text())?;
        let extracted = extract(model, root, false);
        let owner = owner_at(
            call.syntax(),
            &owner_map(root, &extracted.facts.declarations, &extracted.locations),
        );
        let candidates = self.resolve_reference(
            path,
            &owner,
            &ParentRef::Formal(name),
            call.syntax()
                .ancestors()
                .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR),
        );
        let [id] = candidates.as_slice() else {
            return None;
        };
        if id.path == path {
            let location = extracted
                .facts
                .declarations
                .iter()
                .zip(&extracted.locations)
                .find(|(class, _)| class.key == id.key)?
                .1;
            // A top-level lookup executes immediately; a closure may run after
            // the enclosing registry has been populated.
            if location.call_range.start() >= call.syntax().text_range().start()
                && !call
                    .syntax()
                    .ancestors()
                    .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            {
                return None;
            }
        }
        (self.declarations.get(id)?.declaration.system() == ClassSystem::RefClass
            && self.declarations.get(id)?.declaration.binding_known())
        .then(|| id.clone())
    }

    pub fn reference_receiver(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<ClassReceiver> {
        self.class_receiver(path, root, model, value)
            .filter(|receiver| receiver.class.key.system == ClassSystem::RefClass)
    }

    pub fn reference_api(&self, id: &ClassId) -> Option<ReferenceApi> {
        self.reference_api_at(id, None)
    }

    pub fn reference_api_at(&self, id: &ClassId, point: Option<&OopPoint>) -> Option<ReferenceApi> {
        (self.declarations.get(id)?.declaration.system() == ClassSystem::RefClass).then_some(())?;
        let (mut instance, mut slots) =
            self.reference_members(id, &mut BTreeSet::new(), &mut BTreeMap::new(), point);
        let class = &self.declarations.get(id)?.declaration;
        let registry_known = match &class.name {
            ClassName::Known(name) => {
                self.resolve_reference(
                    &id.path,
                    &id.key.owner,
                    &ParentRef::Formal(name.clone()),
                    true,
                )
                .len()
                    == 1
            }
            _ => false,
        };
        if !registry_known {
            instance.complete = false;
            slots.complete = false;
        }
        let initializer = instance
            .complete
            .then(
                || match instance.entries.get("initialize").map(Vec::as_slice) {
                    Some([member]) if member.kind == MemberKind::Method => member.signature.clone(),
                    None => Some(signature(&[("...", None)])),
                    _ => None,
                },
            )
            .flatten();
        Some(ReferenceApi {
            generator: generator_members(),
            instance,
            slots,
            initializer,
        })
    }

    /// First inherited implementation in the declaring class's contains order.
    /// An inherited copy keeps its declaring method's super target.
    pub fn reference_super_method(&self, id: &ClassId, name: &str) -> Option<ReferenceMember> {
        let own_api = self.reference_api(id)?;
        if !own_api.instance.complete {
            return None;
        }
        let class = &self.declarations.get(id)?.declaration;
        let parents = self.parent_candidates.get(id)?;
        for (ordinal, (reference, candidates)) in class.parents.iter().zip(parents).enumerate() {
            if self.reference_base_parent(id, ordinal) {
                continue;
            }
            let [parent] = candidates.as_slice() else {
                return None;
            };
            if !self.supertypes.get(id)?.contains(parent) || matches!(reference, ParentRef::Unknown)
            {
                return None;
            }
            if self.declarations.get(parent)?.declaration.system() != ClassSystem::RefClass {
                continue;
            }
            let point = self.declaration_point(id);
            let api = self.reference_api_at(parent, point.as_ref())?;
            if !api.instance.complete {
                return None;
            }
            if let Some(candidates) = api.instance.entries.get(name) {
                let [member] = candidates.as_slice() else {
                    return None;
                };
                return (member.kind == MemberKind::Method).then(|| member.clone());
            }
        }
        let builtins = object_members();
        // R gives a new initializer a special super target: the effective
        // initFields helper, which can itself be overridden or inherited.
        let candidates = if name == "initialize" {
            own_api.instance.entries.get("initFields")
        } else {
            builtins.entries.get(name)
        }?;
        let [member] = candidates.as_slice() else {
            return None;
        };
        (member.kind == MemberKind::Method).then(|| member.clone())
    }

    fn reference_members(
        &self,
        id: &ClassId,
        visiting: &mut BTreeSet<ClassId>,
        cache: &mut BTreeMap<(ClassId, Option<OopPoint>), (ReferenceMembers, ReferenceMembers)>,
        point: Option<&OopPoint>,
    ) -> (ReferenceMembers, ReferenceMembers) {
        let cache_key = (id.clone(), point.cloned());
        if let Some(members) = cache.get(&cache_key) {
            return members.clone();
        }
        let mut members = ReferenceMembers::default();
        let mut slots = ReferenceMembers::default();
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            members.complete = false;
            slots.complete = false;
            return (members, slots);
        }
        let (Some(class), Some(file)) = (self.declarations.get(id), self.file(&id.path)) else {
            members.complete = false;
            slots.complete = false;
            return (members, slots);
        };
        let def = &class.declaration;
        if def.system() == ClassSystem::S4 {
            if let Some(schema) = self.s4_slots(id) {
                slots.complete = schema.complete;
                slots.entries = schema
                    .entries
                    .into_iter()
                    .map(|(name, candidates)| {
                        (
                            name,
                            candidates
                                .into_iter()
                                .map(|slot| ReferenceMember {
                                    declaration: slot.declaration,
                                    kind: MemberKind::Slot,
                                    declared_type: Some(slot.declared_type),
                                    signature: None,
                                })
                                .collect(),
                        )
                    })
                    .collect();
            } else {
                slots.complete = false;
            }
            visiting.remove(id);
            return (members, slots);
        }
        if !def.binding_known()
            || def
                .parent_vector
                .as_ref()
                .is_some_and(|factory| self.callee_identity(&id.path, factory).is_none())
        {
            members.complete = false;
            slots.complete = false;
        }
        for (ordinal, candidates) in self
            .parent_candidates
            .get(id)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if self.reference_base_parent(id, ordinal) {
                continue;
            }
            let [parent] = candidates.as_slice() else {
                members.complete = false;
                slots.complete = false;
                continue;
            };
            if !self
                .supertypes
                .get(id)
                .is_some_and(|parents| parents.contains(parent))
            {
                members.complete = false;
                slots.complete = false;
                continue;
            }
            let declaration = self.declaration_point(id);
            let (inherited, inherited_slots) =
                self.reference_members(parent, visiting, cache, declaration.as_ref());
            members.complete &= inherited.complete;
            slots.complete &= inherited_slots.complete;
            for (name, candidates) in inherited.entries {
                members.entries.entry(name).or_insert(candidates);
            }
            for (name, candidates) in inherited_slots.entries {
                slots.entries.entry(name).or_insert(candidates);
            }
        }
        if def.system() == ClassSystem::RefClass {
            for (name, candidates) in object_members().entries {
                members.entries.entry(name).or_insert(candidates);
            }
            slots
                .entries
                .entry(".xData".into())
                .or_insert_with(|| vec![generated(MemberKind::Slot, None)]);
        }
        for group in file
            .facts
            .oop
            .member_groups
            .iter()
            .filter(|group| group.class == id.key)
        {
            let valid = !group.uncertainty.contains(&OopUncertainty::DynamicMembers)
                && match &group.factory {
                    None => group.name == "fields",
                    Some(factory) => {
                        self.callee_identity(&id.path, factory)
                            .is_some_and(|identity| {
                                identity.package == "base"
                                    && (identity.name == "list"
                                        || (identity.name == "character" && group.name == "fields")
                                        || (identity.name == "c" && group.name != "methods"))
                            })
                    }
                };
            if !valid {
                members.complete = false;
                if group.name == "slots" {
                    slots.complete = false;
                }
            }
        }
        let mut own = BTreeMap::<String, Vec<ReferenceMember>>::new();
        let mut own_slots = BTreeMap::<String, Vec<ReferenceMember>>::new();
        let installed = self.own_members(id, point);
        members.complete &= installed.complete;
        members.entries.retain(|name, candidates| {
            !installed.removals.contains(name)
                || candidates
                    .iter()
                    .any(|member| member.kind != MemberKind::Method)
        });
        for installed in &installed.entries {
            let member = &installed.definition;
            let ClassName::Known(name) = &member.name else {
                members.complete = false;
                continue;
            };
            let mut signature = installed.signature.clone();
            if member.kind == MemberKind::Method
                && let Some(signature) = &mut signature
                && signature
                    .parameters
                    .first()
                    .is_some_and(|parameter| parameter.name == ".self")
            {
                signature.parameters.remove(0);
            }
            let target = if member.kind == MemberKind::Slot {
                &mut own_slots
            } else {
                &mut own
            };
            target
                .entry(name.clone())
                .or_default()
                .push(ReferenceMember {
                    declaration: Some(installed.id.clone()),
                    kind: member.kind,
                    declared_type: member.declared_type.clone(),
                    signature,
                });
            if member.uncertainty.iter().any(|issue| {
                matches!(
                    issue,
                    OopUncertainty::UnresolvedImplementation | OopUncertainty::DynamicMembers
                )
            }) {
                members.complete = false;
            }
        }
        for (name, candidates) in own {
            if candidates.len() != 1
                || name.is_empty()
                || matches!(name.as_str(), ".self" | ".refClassDef")
            {
                members.complete = false;
            }
            members.entries.insert(name, candidates);
        }
        for (name, candidates) in own_slots {
            if candidates.len() != 1 || name.is_empty() {
                slots.complete = false;
            }
            slots.entries.insert(name, candidates);
        }
        visiting.remove(id);
        cache.insert(cache_key, (members.clone(), slots.clone()));
        (members, slots)
    }
}
