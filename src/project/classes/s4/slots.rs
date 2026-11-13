use super::*;

fn builtin(name: &str) -> Option<(&str, &str)> {
    Some(match name {
        "numeric" | "integer" | "logical" | "complex" | "character" | "raw" | "list"
        | "expression" | "function" | "vector" | "matrix" | "array" | "pairlist" | "ANY" | "S4" => {
            (".Data", name)
        }
        "environment" | "externalptr" | "name" | "NULL" => (".xData", name),
        "oldClass" => (".S3Class", "character"),
        _ => return None,
    })
}

fn generated(slots: &mut S4Slots, name: &str, class: &str) {
    let slot = S4Slot {
        declaration: None,
        declared_type: if class == "ANY" {
            DispatchClass::Any
        } else {
            DispatchClass::Formal(class.into())
        },
    };
    let candidates = slots.entries.entry(name.into()).or_default();
    if !candidates.contains(&slot) {
        candidates.push(slot);
    }
}

impl ClassIndex {
    pub(in crate::project::classes) fn reuse_s4_old_class_ancestors(&mut self) {
        let file_order = |id: &ClassId| {
            self.declarations[id]
                .package_root
                .as_ref()
                .and_then(|root| self.load_orders.get(root))
                .and_then(Option::as_ref)
                .and_then(|order| order.iter().position(|path| *path == id.path))
        };
        let mut ancestors = self
            .declarations
            .iter()
            .filter_map(|(id, class)| {
                let old = class.declaration.s4.as_ref()?.old_class.as_ref()?;
                (old.index > 0
                    && old.test == Some(false)
                    && class.declaration.binding_known()
                    && self.class_file_complete(&id.path)
                    && self.s4_parent_known(id, 0))
                .then(|| (class.package_root.clone(), file_order(id), id.clone()))
            })
            .collect::<Vec<_>>();
        ancestors.sort();
        for (package_root, order, id) in ancestors {
            let definition = &self.declarations[&id].declaration;
            let ClassName::Known(name) = &definition.name else {
                continue;
            };
            let predecessors = self
                .resolve_reference(
                    &id.path,
                    &id.key.owner,
                    &ParentRef::Formal(name.clone()),
                    !id.key.owner.0.is_empty(),
                )
                .into_iter()
                .filter(|candidate| {
                    let class = &self.declarations[candidate];
                    if !class.declaration.binding_known() || candidate.key.owner != id.key.owner {
                        return false;
                    }
                    if candidate.path == id.path {
                        return candidate.key.system == id.key.system
                            && candidate.key.occurrence < id.key.occurrence;
                    }
                    // Across files, only a complete package with an agreed load
                    // order establishes that the ancestor was already registered.
                    id.key.owner.0.is_empty()
                        && package_root.is_some()
                        && class.package_root == package_root
                        && self.class_file_complete(&candidate.path)
                        && package_root
                            .as_ref()
                            .and_then(|root| self.load_orders.get(root))
                            .and_then(Option::as_ref)
                            .and_then(|files| files.iter().position(|path| *path == candidate.path))
                            .zip(order)
                            .is_some_and(|(earlier, later)| earlier < later)
                })
                .collect::<Vec<_>>();
            if predecessors.len() == 1 {
                self.declarations.remove(&id);
            }
        }
    }

    pub(in crate::project::classes) fn s4_parent_known(
        &self,
        id: &ClassId,
        ordinal: usize,
    ) -> bool {
        self.declarations
            .get(id)
            .and_then(|class| class.declaration.s4.as_ref()?.old_class.as_ref())
            .is_none_or(|old| {
                ordinal < old.parent_start
                    || (old.test == Some(false)
                        && self.callee_identity(&id.path, &old.factory).is_some()
                        && old
                            .helpers
                            .iter()
                            .all(|helper| self.callee_identity(&id.path, helper).is_some()))
            })
    }

    fn slot_type_candidates(&self, context: &ClassId, slot: &S4Slot) -> Vec<ClassId> {
        let DispatchClass::Formal(name) = &slot.declared_type else {
            return Vec::new();
        };
        let (path, owner) = slot
            .declaration
            .as_ref()
            .map_or((&context.path, &context.key.owner), |declaration| {
                (&declaration.path, &declaration.key.owner)
            });
        self.resolve_reference(path, owner, &ParentRef::Formal(name.clone()), true)
    }

    fn slot_type_extends(&self, context: &ClassId, actual: &S4Slot, required: &S4Slot) -> bool {
        if required.declared_type == DispatchClass::Any {
            return true;
        }
        if actual.declared_type == DispatchClass::Unknown
            || required.declared_type == DispatchClass::Unknown
        {
            return false;
        }
        let actual_classes = self.slot_type_candidates(context, actual);
        let required_classes = self.slot_type_candidates(context, required);
        if let ([actual], [required]) = (actual_classes.as_slice(), required_classes.as_slice()) {
            let mut pending = vec![actual.clone()];
            let mut visited = BTreeSet::new();
            while let Some(class) = pending.pop() {
                if &class == required {
                    return true;
                }
                if visited.insert(class.clone()) {
                    pending.extend(self.supertypes.get(&class).into_iter().flatten().cloned());
                }
            }
            return false;
        }
        actual_classes.is_empty()
            && required_classes.is_empty()
            && (actual.declared_type == required.declared_type
                || matches!((&actual.declared_type, &required.declared_type), (DispatchClass::Formal(actual), DispatchClass::Formal(required)) if actual == "integer" && required == "numeric"))
    }

    fn inherit_s4_slots(&self, context: &ClassId, slots: &mut S4Slots, inherited: S4Slots) {
        slots.complete &= inherited.complete;
        for (name, entries) in inherited.entries {
            let target = slots.entries.entry(name).or_default();
            for entry in entries {
                if target.is_empty() {
                    target.push(entry);
                    continue;
                }
                let narrows = target
                    .iter()
                    .all(|actual| self.slot_type_extends(context, actual, &entry));
                if !narrows {
                    slots.complete = false;
                }
                let same_type = target
                    .iter()
                    .all(|actual| actual.declared_type == entry.declared_type);
                if (!narrows || same_type) && !target.contains(&entry) {
                    target.push(entry);
                }
            }
        }
    }

    pub(in crate::project::classes) fn install_s4_relations(&mut self) {
        let templates = self
            .declarations
            .iter()
            .filter_map(|(id, class)| {
                let metadata = class.declaration.s4.as_ref()?;
                let old = metadata.old_class.as_ref()?;
                if metadata.definition != S4Definition::OldClass
                    || old.test != Some(false)
                    || !class.declaration.binding_known()
                    || !self.class_file_complete(&id.path)
                    || self.callee_identity(&id.path, &old.factory).is_none()
                    || old
                        .helpers
                        .iter()
                        .any(|helper| self.callee_identity(&id.path, helper).is_none())
                {
                    return None;
                }
                let reference = old.s4_class.as_ref()?;
                let candidates = self.resolve_reference(
                    &id.path,
                    &id.key.owner,
                    reference,
                    !id.key.owner.0.is_empty(),
                );
                let [template] = candidates.as_slice() else {
                    return None;
                };
                Some((id.clone(), template.clone()))
            })
            .collect::<Vec<_>>();
        let relations = self
            .oop_files()
            .flat_map(|file| {
                file.facts
                    .s4
                    .relations
                    .iter()
                    .map(|relation| (file.path.clone(), relation.clone()))
            })
            .collect::<Vec<_>>();
        for (path, relation) in relations {
            if relation.conditional
                || !relation.uncertainty.is_empty()
                || !self.class_file_complete(&path)
                || self.callee_identity(&path, &relation.factory).is_none()
                || relation
                    .helpers
                    .iter()
                    .any(|helper| self.callee_identity(&path, helper).is_none())
            {
                continue;
            }
            let (ClassName::Known(from), ClassName::Known(to)) =
                (&relation.subclass, &relation.superclass)
            else {
                continue;
            };
            let from = self.resolve_reference(
                &path,
                &relation.owner,
                &ParentRef::Formal(from.clone()),
                !relation.owner.0.is_empty(),
            );
            let to = self.resolve_reference(
                &path,
                &relation.owner,
                &ParentRef::Formal(to.clone()),
                !relation.owner.0.is_empty(),
            );
            let ([from], [to]) = (from.as_slice(), to.as_slice()) else {
                continue;
            };
            if [from, to].iter().any(|id| {
                self.declarations
                    .get(*id)
                    .is_none_or(|class| !class.declaration.binding_known())
            }) {
                continue;
            }
            self.supertypes
                .entry(from.clone())
                .or_default()
                .insert(to.clone());
            self.subtypes
                .entry(to.clone())
                .or_default()
                .insert(from.clone());
        }
        // A copied definition retains structural parents and installed relation
        // edges, without adding the template's own name to its ancestry.
        for _ in 0..templates.len().min(64) {
            let mut changed = false;
            for (id, template) in &templates {
                let parents = self.supertypes.get(template).cloned().unwrap_or_default();
                for parent in parents {
                    changed |= self
                        .supertypes
                        .entry(id.clone())
                        .or_default()
                        .insert(parent.clone());
                    self.subtypes.entry(parent).or_default().insert(id.clone());
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Only structural parents contribute slots. A setIs coercion can establish
    /// method inheritance while leaving the source object's storage unchanged.
    pub fn s4_slots(&self, id: &ClassId) -> Option<S4Slots> {
        if !matches!(id.key.system, ClassSystem::S4 | ClassSystem::RefClass) {
            return None;
        }
        self.declarations.get(id)?;
        Some(self.collect_s4_slots(id, &mut BTreeSet::new(), &mut BTreeMap::new()))
    }

    fn collect_s4_slots(
        &self,
        id: &ClassId,
        visiting: &mut BTreeSet<ClassId>,
        cache: &mut BTreeMap<ClassId, S4Slots>,
    ) -> S4Slots {
        if let Some(slots) = cache.get(id) {
            return slots.clone();
        }
        let mut slots = S4Slots::default();
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            slots.complete = false;
            return slots;
        }
        let Some(class) = self.declarations.get(id) else {
            slots.complete = false;
            return slots;
        };
        let def = &class.declaration;
        slots.complete = def.binding_known() && self.class_file_complete(&id.path);
        slots.complete &= match &def.name {
            ClassName::Known(name) => {
                self.resolve_reference(
                    &id.path,
                    &id.key.owner,
                    &ParentRef::Formal(name.clone()),
                    true,
                )
                .as_slice()
                    == std::slice::from_ref(id)
            }
            _ => false,
        };
        let own = self.own_s4_slots(id, def);
        slots.complete &= own.complete;
        slots.entries = own.entries;
        if let Some(s4) = &def.s4 {
            slots.complete &= s4.slots_complete
                && s4.parents_complete
                && s4
                    .schema_helpers
                    .iter()
                    .all(|helper| self.callee_identity(&id.path, helper).is_some());
            if let Some(old) = &s4.old_class {
                match self.callee_availability(&id.path, &old.factory) {
                    crate::semantic::enclosure::Availability::Present => {
                        generated(&mut slots, ".S3Class", "character");
                        slots.complete &= old.test == Some(false)
                            && old
                                .helpers
                                .iter()
                                .all(|helper| self.callee_identity(&id.path, helper).is_some());
                        if let Some(reference) = &old.s4_class
                            && !matches!(reference, ParentRef::Formal(name) if def.name == ClassName::Known(name.clone()))
                        {
                            let candidates = self.resolve_reference(
                                &id.path,
                                &id.key.owner,
                                reference,
                                !id.key.owner.0.is_empty(),
                            );
                            if let [template] = candidates.as_slice() {
                                self.inherit_s4_slots(
                                    id,
                                    &mut slots,
                                    self.collect_s4_slots(template, visiting, cache),
                                );
                            } else {
                                slots.complete = false;
                            }
                        }
                    }
                    crate::semantic::enclosure::Availability::Unknown => slots.complete = false,
                    crate::semantic::enclosure::Availability::Absent => {}
                }
            }
        }
        if let Some(vector) = &def.parent_vector {
            slots.complete &= self.callee_identity(&id.path, vector).is_some();
        }
        for (ordinal, reference) in def.parents.iter().enumerate() {
            let candidates = self
                .parent_candidates
                .get(id)
                .and_then(|parents| parents.get(ordinal))
                .map(Vec::as_slice)
                .unwrap_or_default();
            if let [parent] = candidates {
                if self
                    .supertypes
                    .get(id)
                    .is_some_and(|parents| parents.contains(parent))
                {
                    self.inherit_s4_slots(
                        id,
                        &mut slots,
                        self.collect_s4_slots(parent, visiting, cache),
                    );
                } else {
                    slots.complete = false;
                }
            } else if candidates.is_empty() {
                if let ParentRef::Formal(name) = reference {
                    if def.system() == ClassSystem::RefClass && name == "envRefClass" {
                        generated(&mut slots, ".xData", "environment");
                    } else if let Some((name, class)) = builtin(name) {
                        generated(&mut slots, name, class);
                    } else {
                        slots.complete = false;
                    }
                } else {
                    slots.complete = false;
                }
            } else {
                slots.complete = false;
            }
        }
        if def.system() == ClassSystem::RefClass {
            generated(&mut slots, ".xData", "environment");
        }
        visiting.remove(id);
        cache.insert(id.clone(), slots.clone());
        slots
    }

    fn own_s4_slots(&self, id: &ClassId, def: &ClassDef) -> S4Slots {
        let mut slots = S4Slots::default();
        let mut own: BTreeMap<String, Vec<S4Slot>> = BTreeMap::new();
        let own_slots_known = def.s4.as_ref().is_none_or(|s4| {
            s4.slot_helpers
                .iter()
                .all(|helper| self.callee_identity(&id.path, helper).is_some())
        });
        slots.complete &= own_slots_known;
        if let Some(file) = self.file(&id.path).filter(|_| own_slots_known) {
            for member in file.facts.oop.members.iter().filter(|member| {
                member.class.as_ref() == Some(&id.key) && member.kind == MemberKind::Slot
            }) {
                let ClassName::Known(name) = &member.name else {
                    slots.complete = false;
                    continue;
                };
                let class = member
                    .declared_type
                    .clone()
                    .unwrap_or(DispatchClass::Unknown);
                if class == DispatchClass::Unknown
                    || name.is_empty()
                    || matches!(name.as_str(), "class" | "Class")
                {
                    slots.complete = false;
                }
                own.entry(name.clone()).or_default().push(S4Slot {
                    declaration: Some(OopId {
                        path: id.path.clone(),
                        key: member.key.clone(),
                    }),
                    declared_type: class,
                });
            }
        }
        for (name, entries) in own {
            if entries.len() != 1 {
                slots.complete = false;
            }
            slots.entries.insert(name, entries);
        }
        slots
    }

    /// Prototype values inherit through contains/representation. Implicit defaults
    /// and computed values stay unknown rather than evaluating R constructors.
    pub fn s4_prototype(&self, id: &ClassId) -> Option<S4Prototype> {
        self.declarations.get(id)?;
        let mut result = self.collect_s4_prototype(id, &mut BTreeSet::new(), &mut BTreeMap::new());
        result.complete &= result
            .slots
            .values()
            .all(|value| *value != S4Default::Unknown)
            && result.data != Some(S4Default::Unknown);
        Some(result)
    }

    fn collect_s4_prototype(
        &self,
        id: &ClassId,
        visiting: &mut BTreeSet<ClassId>,
        cache: &mut BTreeMap<ClassId, S4Prototype>,
    ) -> S4Prototype {
        if let Some(prototype) = cache.get(id) {
            return prototype.clone();
        }
        let mut result = S4Prototype {
            complete: true,
            ..Default::default()
        };
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            result.complete = false;
            return result;
        }
        let Some(definition) = self.declarations.get(id).map(|class| &class.declaration) else {
            result.complete = false;
            return result;
        };
        result.complete &= definition.binding_known() && self.class_file_complete(&id.path);
        if let Some(metadata) = &definition.s4 {
            result.complete &= metadata.parents_complete
                && metadata
                    .schema_helpers
                    .iter()
                    .all(|helper| self.callee_identity(&id.path, helper).is_some());
            if let Some(old) = &metadata.old_class
                && let Some(reference) = &old.s4_class
                && !matches!(reference, ParentRef::Formal(name) if definition.name == ClassName::Known(name.clone()))
            {
                let templates = self.resolve_reference(
                    &id.path,
                    &id.key.owner,
                    reference,
                    !id.key.owner.0.is_empty(),
                );
                if let [template] = templates.as_slice() {
                    let inherited = self.collect_s4_prototype(template, visiting, cache);
                    result.complete &= inherited.complete;
                    result.slots = inherited.slots;
                    result.data = inherited.data;
                } else {
                    result.complete = false;
                }
            }
        }
        for (ordinal, parent) in self
            .parent_candidates
            .get(id)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let [parent] = parent.as_slice() {
                if !self
                    .supertypes
                    .get(id)
                    .is_some_and(|parents| parents.contains(parent))
                {
                    result.complete = false;
                    continue;
                }
                let inherited = self.collect_s4_prototype(parent, visiting, cache);
                result.complete &= inherited.complete;
                for (name, value) in inherited.slots {
                    result.slots.entry(name).or_insert(value);
                }
                result.data = result.data.or(inherited.data);
            } else {
                // Built-in data classes have no source prototype declarations.
                result.complete &= matches!(definition.parents.get(ordinal), Some(ParentRef::Formal(name)) if builtin(name).is_some());
            }
        }
        if let Some(class) = self
            .declarations
            .get(id)
            .and_then(|class| class.declaration.s4.as_ref())
        {
            let own = &class.prototype;
            result.provided = own.provided;
            result.complete &= own.complete
                && own
                    .helpers
                    .iter()
                    .all(|helper| self.callee_identity(&id.path, helper).is_some());
            result.slots.extend(own.slots.clone());
            if own.data.is_some() {
                result.data = own.data.clone();
            }
        }
        // Claim every storage slot, including implicit defaults, before a later
        // parent can offer a competing explicit value for that slot.
        if let Some(slots) = self.s4_slots(id) {
            result.complete &= slots.complete;
            for name in slots.entries.into_keys() {
                if name == ".Data" {
                    result.data.get_or_insert(S4Default::Unknown);
                } else {
                    result.slots.entry(name).or_insert(S4Default::Unknown);
                }
            }
        }
        visiting.remove(id);
        cache.insert(id.clone(), result.clone());
        result
    }

    pub(in crate::project::classes) fn s4_named_class(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<ClassId> {
        let token = value
            .as_token()
            .filter(|token| token.kind() == SyntaxKind::STRING)?;
        let name = crate::semantic::decode_string_literal(token.text())?;
        let extracted = super::super::extract(model, root, false);
        let node = value.parent()?;
        let owner = owner_at(&node, &extracted.owners);
        let candidates =
            self.resolve_reference(path, &owner, &ParentRef::Formal(name), !owner.0.is_empty());
        let [id] = candidates.as_slice() else {
            return None;
        };
        let class = &self.declarations.get(id)?.declaration;
        if !class.binding_known() {
            return None;
        }
        if id.path == path {
            let location = extracted
                .facts
                .declarations
                .iter()
                .zip(&extracted.locations)
                .find(|(class, _)| class.key == id.key)?
                .1;
            if location.call_range.start() >= value.text_range().start()
                && !node
                    .ancestors()
                    .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            {
                return None;
            }
        }
        Some(id.clone())
    }

    pub(in crate::project::classes) fn s4_constructible(&self, id: &ClassId) -> bool {
        self.s4_class(id)
            .is_some_and(|class| class.virtual_class == Some(false))
    }

    /// setOldClass copies an explicitly supplied S4 definition. The template
    /// supplies metadata and slots without making its class a superclass.
    pub fn s4_class(&self, id: &ClassId) -> Option<S4Class> {
        self.s4_class_inner(id, &mut BTreeSet::new())
    }

    fn s4_class_inner(&self, id: &ClassId, visiting: &mut BTreeSet<ClassId>) -> Option<S4Class> {
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            return None;
        }
        let mut metadata = self.declarations.get(id)?.declaration.s4.clone()?;
        if metadata.old_class.as_ref().is_some_and(|old| {
            self.callee_availability(&id.path, &old.factory)
                == crate::semantic::enclosure::Availability::Absent
        }) {
            metadata.old_class = None;
        }
        if metadata.definition == S4Definition::OldClass
            && let Some(reference) = metadata.old_class.as_ref()?.s4_class.as_ref()
        {
            let candidates = self.resolve_reference(
                &id.path,
                &id.key.owner,
                reference,
                !id.key.owner.0.is_empty(),
            );
            let [template] = candidates.as_slice() else {
                return Some(metadata);
            };
            if let Some(template) = self.s4_class_inner(template, visiting) {
                metadata.virtual_class = template.virtual_class;
                metadata.sealed = template.sealed;
            }
        }
        if metadata.definition == S4Definition::OldClass
            && metadata
                .prototype
                .helpers
                .iter()
                .any(|helper| self.callee_identity(&id.path, helper).is_none())
        {
            metadata.virtual_class = None;
        }
        Some(metadata)
    }

    pub(in crate::project::classes) fn s4_slot_class(
        &self,
        receiver: &ClassReceiver,
        name: &str,
    ) -> Option<ClassId> {
        if receiver.access == ClassAccess::Generator {
            return None;
        }
        let slots = self.s4_slots(&receiver.class)?;
        if !slots.complete {
            return None;
        }
        let candidates = slots.entries.get(name)?;
        let DispatchClass::Formal(name) = &candidates.first()?.declared_type else {
            return None;
        };
        if candidates
            .iter()
            .any(|candidate| candidate.declared_type != DispatchClass::Formal(name.clone()))
        {
            return None;
        }
        let candidates = candidates
            .iter()
            .flat_map(|slot| self.slot_type_candidates(&receiver.class, slot))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let [id] = candidates.as_slice() else {
            return None;
        };
        Some(id.clone())
    }
}

pub(in super::super) fn slot_call(
    index: &ClassIndex,
    path: &std::path::Path,
    root: &SyntaxNode,
    model: &SemanticModel,
    call: &CallExpr,
) -> Option<(SyntaxElement, String, bool)> {
    let reference = call_function_ref(model, root, call)?;
    let identity = index.callee_identity(path, &reference)?;
    let replacement = matches!(identity.name.as_str(), "slot<-" | "@<-");
    if !matches!(
        (identity.package.as_str(), identity.name.as_str()),
        ("methods", "slot" | "slot<-") | ("base", "@" | "@<-")
    ) {
        return None;
    }
    let args = matched_args(
        call,
        if identity.name == "slot<-" {
            &["object", "name", "check", "value"]
        } else if replacement {
            &["object", "name", "value"]
        } else {
            &["object", "name"]
        },
    )?;
    let name = argument(&args, "name")?
        .into_token()
        .filter(|token| token.kind() == SyntaxKind::STRING)
        .and_then(|token| crate::semantic::decode_string_literal(token.text()))?;
    Some((argument(&args, "object")?, name, replacement))
}
