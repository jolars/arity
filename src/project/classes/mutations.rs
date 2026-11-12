//! Replay only statically known generator operations. This is a source-fact
//! projection, not an R evaluator: accessors and method bodies never run here.

use std::path::{Path, PathBuf};

use super::*;
use crate::project::source::{SourceEdgeKey, SourceTarget};

#[derive(Clone)]
pub(super) struct InstalledMember {
    pub id: OopId,
    pub definition: MemberDef,
    pub signature: Option<CallableSignature>,
}

pub(super) struct OwnMembers {
    pub entries: Vec<InstalledMember>,
    potential: Vec<InstalledMember>,
    pub removals: BTreeSet<String>,
    pub complete: bool,
    pub class_locked: Option<bool>,
    pub locked_fields: BTreeSet<String>,
}

#[derive(Default)]
struct Execution {
    events: Vec<(PathBuf, usize)>,
    complete: bool,
}

#[derive(Default)]
struct LiveBindings {
    generators: BTreeMap<String, ClassId>,
    possible: BTreeMap<String, BTreeSet<ClassId>>,
    callables: BTreeMap<String, CallableSignature>,
}

impl ClassIndex {
    pub(super) fn declaration_point(&self, id: &ClassId) -> Option<OopPoint> {
        let event = self
            .file(&id.path)?
            .facts
            .oop
            .events
            .iter()
            .position(|event| matches!(event, OopEvent::Class(key) if key == &id.key))?;
        Some(OopPoint {
            path: id.path.clone(),
            event,
        })
    }
    pub(super) fn member_installations(&self, id: &ClassId) -> Vec<InstalledMember> {
        let view = self.own_members(id, None);
        let mut installed = view.entries;
        installed.extend(view.potential);
        // Older instances retain replaced implementations too.
        if let Some(file) = self.file(&id.path) {
            installed.extend(
                file.facts
                    .oop
                    .members
                    .iter()
                    .filter(|member| member.class.as_ref() == Some(&id.key))
                    .map(|member| self.install_member(&id.path, member, &BTreeMap::new())),
            );
        }
        for file in self.oop_files() {
            for (event, value) in file.facts.oop.events.iter().enumerate() {
                let OopEvent::Mutation(index) = value else {
                    continue;
                };
                let mutation = &file.facts.oop.mutations[*index];
                let view = self.own_members(
                    id,
                    Some(&OopPoint {
                        path: file.path.clone(),
                        event: event + 1,
                    }),
                );
                installed.extend(
                    view.entries
                        .into_iter()
                        .chain(view.potential)
                        .filter(|member| {
                            member.id.path == file.path && mutation.members.contains(&member.id.key)
                        }),
                );
            }
        }
        installed.sort_by(|a, b| a.id.cmp(&b.id));
        installed.dedup_by(|a, b| a.id == b.id);
        installed
    }

    pub(super) fn reference_locked_fields(
        &self,
        id: &ClassId,
        point: Option<&OopPoint>,
        visiting: &mut BTreeSet<ClassId>,
    ) -> BTreeSet<String> {
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            return BTreeSet::new();
        }
        let mut fields = self.own_members(id, point).locked_fields;
        let declaration = self.declaration_point(id);
        for parent in self.supertypes.get(id).into_iter().flatten() {
            fields.extend(self.reference_locked_fields(parent, declaration.as_ref(), visiting));
        }
        visiting.remove(id);
        fields
    }

    pub(super) fn method_class(&self, path: &Path, method: &OopKey) -> Option<ClassId> {
        let candidates: Vec<_> = self
            .declarations
            .keys()
            .filter(|id| {
                self.member_installations(id).iter().any(|member| {
                    member.id.path == path && member.definition.method.as_ref() == Some(method)
                })
            })
            .cloned()
            .collect();
        match candidates.as_slice() {
            [id] => Some(id.clone()),
            _ => None,
        }
    }
    fn source_path(path: &Path, edge: &SourceEdgeKey) -> Option<PathBuf> {
        let SourceTarget::Path(target) = &edge.target else {
            return None;
        };
        let joined = if target.is_absolute() {
            target.clone()
        } else {
            path.parent()?.join(target)
        };
        let mut normalized = PathBuf::new();
        for component in joined.components() {
            match component {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::CurDir => {}
                component => normalized.push(component.as_os_str()),
            }
        }
        Some(normalized)
    }

    fn append_execution(&self, path: &Path, out: &mut Execution, visiting: &mut BTreeSet<PathBuf>) {
        if visiting.len() >= 64 || !visiting.insert(path.into()) {
            out.complete = false;
            return;
        }
        let Some(file) = self.file(path) else {
            out.complete = false;
            return;
        };
        for (index, event) in file.facts.oop.events.iter().enumerate() {
            out.events.push((path.into(), index));
            if let OopEvent::Source(edge) = event {
                if !edge.local
                    && let Some(target) = Self::source_path(path, edge)
                {
                    self.append_execution(&target, out, visiting);
                } else {
                    out.complete = false;
                }
            }
        }
        visiting.remove(path);
    }

    fn execution(&self, path: &Path) -> Execution {
        let mut out = Execution {
            complete: true,
            ..Execution::default()
        };
        if let Some(root) = self.file(path).and_then(|file| file.package_root.as_ref()) {
            match self.load_orders.get(root) {
                Some(Some(order)) => {
                    for path in order {
                        self.append_execution(path, &mut out, &mut BTreeSet::new());
                    }
                }
                _ => {
                    out.complete = false;
                    for file in self
                        .oop_files()
                        .filter(|file| file.package_root.as_ref() == Some(root))
                    {
                        self.append_execution(&file.path, &mut out, &mut BTreeSet::new());
                    }
                }
            }
            return out;
        }
        // A sourced file executes in its caller's sequence. Independent scripts
        // must never acquire one another's mutations merely by sharing a folder.
        let sourced: BTreeSet<_> = self
            .oop_files()
            .flat_map(|file| {
                file.facts.oop.events.iter().filter_map(|event| {
                    if let OopEvent::Source(edge) = event {
                        Self::source_path(&file.path, edge)
                    } else {
                        None
                    }
                })
            })
            .collect();
        let mut candidates = Vec::new();
        for file in self
            .oop_files()
            .filter(|file| file.package_root.is_none() && !sourced.contains(&file.path))
        {
            let mut candidate = Execution {
                complete: true,
                ..Execution::default()
            };
            self.append_execution(&file.path, &mut candidate, &mut BTreeSet::new());
            if candidate.events.iter().any(|(file, _)| file == path) {
                candidates.push(candidate);
            }
        }
        if candidates.len() == 1 {
            return candidates.pop().unwrap();
        }
        self.append_execution(path, &mut out, &mut BTreeSet::new());
        if candidates.len() > 1 {
            out.complete = false;
        }
        out
    }

    fn mutation_targets(
        &self,
        path: &Path,
        mutation: &MemberMutation,
        live: &BTreeMap<String, ClassId>,
        possible: &BTreeMap<String, BTreeSet<ClassId>>,
    ) -> (Vec<ClassId>, bool) {
        if mutation
            .lookup
            .as_ref()
            .is_some_and(|lookup| self.callee_identity(path, lookup).is_none())
        {
            return (Vec::new(), false);
        }
        let mut targets = if let ParentRef::Generator(GeneratorRef::Free(name)) = &mutation.target
            && let Some(id) = live.get(name)
        {
            vec![id.clone()]
        } else {
            self.resolve_reference(path, &mutation.owner, &mutation.target, mutation.deferred)
        };
        let uncertain = if let ParentRef::Generator(GeneratorRef::Free(name)) = &mutation.target
            && let Some(candidates) = possible.get(name)
        {
            targets.extend(candidates.iter().cloned());
            true
        } else {
            false
        };
        targets.sort();
        targets.dedup();
        (targets, uncertain)
    }

    fn install_member(
        &self,
        path: &Path,
        member: &MemberDef,
        callables: &BTreeMap<String, CallableSignature>,
    ) -> InstalledMember {
        let id = OopId {
            path: path.into(),
            key: member.key.clone(),
        };
        let mut definition = member.clone();
        let mut signature = member
            .method
            .as_ref()
            .and_then(|key| {
                self.method(&OopId {
                    path: path.into(),
                    key: key.clone(),
                })
            })
            .and_then(|method| method.signature.clone());
        if signature.is_none()
            && let Some(method) = &member.method
            && let Some(implementation) = self.file(path).and_then(|file| {
                file.facts
                    .oop
                    .method_implementations
                    .iter()
                    .find(|implementation| &implementation.method == method)
            })
        {
            let reference = match &implementation.source {
                MethodImplementationSource::Reference { name } => Some((name, None)),
                MethodImplementationSource::Qualified { package, name } => {
                    Some((name, Some(package.as_str())))
                }
                _ => None,
            };
            if let Some((name, package)) = reference {
                signature = package
                    .is_none()
                    .then(|| callables.get(name).cloned())
                    .flatten();
                if signature.is_none() {
                    let sources = self.implementation_files(path, name, package);
                    if let Some(source) = sources.iter().next().filter(|_| sources.len() == 1)
                        && self
                            .file(source)
                            .and_then(|file| file.package_root.as_ref())
                            .is_some()
                        && self
                            .file(source)
                            .and_then(|file| file.package_root.as_ref())
                            != self.file(path).and_then(|file| file.package_root.as_ref())
                    {
                        signature = self.external_callable(source, name);
                    }
                }
            }
        }
        if signature.is_some() {
            definition
                .uncertainty
                .remove(&OopUncertainty::UnresolvedImplementation);
            if definition.kind == MemberKind::Unknown {
                definition.kind = MemberKind::Method;
            }
        } else if definition.key.system == OopSystem::R6
            && definition.kind == MemberKind::Unknown
            && definition
                .value_factory
                .as_ref()
                .and_then(|factory| self.callee_identity(path, factory))
                .is_some_and(|factory| factory.package == "base" && factory.name == "list")
        {
            definition.kind = MemberKind::Field;
            definition
                .uncertainty
                .remove(&OopUncertainty::UnresolvedImplementation);
        }
        InstalledMember {
            id,
            definition,
            signature,
        }
    }

    fn external_callable(&self, path: &Path, name: &str) -> Option<CallableSignature> {
        let execution = self.execution(path);
        if !execution.complete || !self.class_file_complete(path) {
            return None;
        }
        let mut callables = BTreeMap::new();
        for (path, index) in &execution.events {
            let file = self.file(path)?;
            match &file.facts.oop.events[*index] {
                OopEvent::Callable(index) => {
                    let callable = &file.facts.oop.callables[*index];
                    callables.insert(callable.name.clone(), callable.signature.clone());
                }
                OopEvent::Generator { name, target } => {
                    let signature = match target {
                        GeneratorRef::Free(name) => callables.get(name).cloned(),
                        _ => None,
                    };
                    callables.remove(name);
                    if let Some(signature) = signature {
                        callables.insert(name.clone(), signature);
                    }
                }
                OopEvent::Binding(name) => {
                    callables.remove(name);
                }
                OopEvent::UncertainGenerator { name, .. } => {
                    callables.remove(name);
                }
                OopEvent::InvalidateBindings => callables.clear(),
                _ => {}
            }
        }
        callables.remove(name)
    }

    pub(super) fn own_members(&self, id: &ClassId, point: Option<&OopPoint>) -> OwnMembers {
        let Some(class) = self.declarations.get(id) else {
            return OwnMembers {
                entries: Vec::new(),
                potential: Vec::new(),
                removals: BTreeSet::new(),
                complete: false,
                class_locked: None,
                locked_fields: BTreeSet::new(),
            };
        };
        let def = &class.declaration;
        let context = point.map_or(id.path.as_path(), |point| point.path.as_path());
        let mut execution = self.execution(context);
        if class.package_root.is_some()
            && self
                .file(context)
                .and_then(|file| file.package_root.as_ref())
                != class.package_root.as_ref()
        {
            let mut dependency = self.execution(&id.path);
            dependency.complete &= execution.complete;
            dependency.events.extend(execution.events);
            execution = dependency;
        }
        let mut out = OwnMembers {
            entries: Vec::new(),
            potential: Vec::new(),
            removals: BTreeSet::new(),
            complete: execution.complete && self.class_file_complete(&id.path),
            class_locked: if def.system() == ClassSystem::R6 {
                def.r6.as_ref().and_then(|options| options.lock_class)
            } else {
                Some(false)
            },
            locked_fields: BTreeSet::new(),
        };
        // Dependencies execute first, but their private bindings never become
        // bare names in the namespace that installs a member.
        let mut namespaces = BTreeMap::<Option<PathBuf>, LiveBindings>::new();
        let mut declared = BTreeSet::new();
        let mut captured = BTreeMap::new();
        let mut saw_class = false;
        for (path, event_index) in &execution.events {
            if point.is_some_and(|point| point.path == *path && *event_index >= point.event) {
                break;
            }
            let file = self.file(path).unwrap();
            let LiveBindings {
                generators: live,
                possible,
                callables,
            } = namespaces.entry(file.package_root.clone()).or_default();
            match &file.facts.oop.events[*event_index] {
                OopEvent::Class(key) => {
                    let current = ClassId {
                        path: path.clone(),
                        key: key.clone(),
                    };
                    declared.insert(current.clone());
                    if key.owner.0.is_empty()
                        && let Some(name) = file
                            .facts
                            .declaration(key)
                            .and_then(|class| class.binding.as_ref())
                    {
                        live.insert(name.clone(), current.clone());
                    }
                    if &current == id {
                        saw_class = true;
                        out.entries = file
                            .facts
                            .oop
                            .members
                            .iter()
                            .filter(|member| member.class.as_ref() == Some(key))
                            .map(|member| self.install_member(path, member, callables))
                            .collect();
                    }
                }
                OopEvent::Generator { name, target } => {
                    let targets = if let GeneratorRef::Free(name) = target {
                        live.get(name).cloned().into_iter().collect()
                    } else {
                        self.resolve_reference(
                            path,
                            &ClassOwner::default(),
                            &ParentRef::Generator(target.clone()),
                            false,
                        )
                    };
                    let signature = match target {
                        GeneratorRef::Free(name) => callables.get(name).cloned(),
                        _ => None,
                    };
                    let candidates = match target {
                        GeneratorRef::Free(name) => possible.get(name).cloned(),
                        _ => None,
                    };
                    possible.remove(name);
                    if let Some(candidates) = candidates {
                        possible.insert(name.clone(), candidates);
                    }
                    live.remove(name);
                    callables.remove(name);
                    if let Some(signature) = signature {
                        callables.insert(name.clone(), signature);
                    }
                    if let [target] = targets.as_slice() {
                        live.insert(name.clone(), target.clone());
                    }
                }
                OopEvent::Callable(index) => {
                    let callable = &file.facts.oop.callables[*index];
                    callables.insert(callable.name.clone(), callable.signature.clone());
                    live.remove(&callable.name);
                    possible.remove(&callable.name);
                }
                OopEvent::UncertainGenerator { name, target } => {
                    let mut candidates = possible.get(name).cloned().unwrap_or_default();
                    candidates.extend(live.remove(name));
                    if let GeneratorRef::Free(name) = target {
                        candidates.extend(live.get(name).cloned());
                    } else {
                        candidates.extend(self.resolve_reference(
                            path,
                            &ClassOwner::default(),
                            &ParentRef::Generator(target.clone()),
                            false,
                        ));
                    }
                    possible.insert(name.clone(), candidates);
                    callables.remove(name);
                }
                OopEvent::Mutator(index) => {
                    let mutation = &file.facts.oop.mutations[*index];
                    captured.insert(
                        (path.clone(), *index),
                        self.mutation_targets(path, mutation, live, possible),
                    );
                }
                OopEvent::Mutation(index) => {
                    let mutation = &file.facts.oop.mutations[*index];
                    let (targets, uncertain_target) = captured
                        .get(&(path.clone(), *index))
                        .cloned()
                        .unwrap_or_else(|| self.mutation_targets(path, mutation, live, possible));
                    if !targets.contains(id) {
                        continue;
                    }
                    if def.system() == ClassSystem::RefClass
                        && class.package_root.is_some()
                        && file.package_root != class.package_root
                    {
                        continue;
                    }
                    // A top-level call before its generator exists cannot install
                    // members. Deferred calls retain a possible later mutation.
                    if !declared.contains(id) && !mutation.deferred {
                        continue;
                    }
                    if matches!(
                        mutation.kind,
                        MutationKind::R6Set { .. } | MutationKind::ReferenceMethods
                    ) && out.class_locked == Some(true)
                    {
                        continue;
                    }
                    if mutation
                        .uncertainty
                        .contains(&OopUncertainty::InvalidArguments)
                    {
                        continue;
                    }
                    if targets.len() != 1
                        || uncertain_target
                        || mutation.uncertainty.contains(&OopUncertainty::Conditional)
                        || mutation
                            .uncertainty
                            .contains(&OopUncertainty::DynamicMembers)
                        || mutation
                            .uncertainty
                            .contains(&OopUncertainty::CustomEnvironment)
                    {
                        out.complete = false;
                        out.potential.extend(
                            file.facts
                                .oop
                                .members
                                .iter()
                                .filter(|member| mutation.members.contains(&member.key))
                                .map(|member| self.install_member(path, member, callables)),
                        );
                        if matches!(
                            mutation.kind,
                            MutationKind::Lock { .. } | MutationKind::ClassBindingLock(_)
                        ) {
                            out.class_locked = None;
                        }
                        continue;
                    }
                    if mutation.factory.as_ref().is_some_and(|factory| {
                        !self.callee_identity(path, factory).is_some_and(|factory| {
                            factory.package == "base"
                                && if matches!(mutation.kind, MutationKind::ClassBindingLock(_)) {
                                    matches!(factory.name.as_str(), "environment" | "globalenv")
                                } else {
                                    factory.name == "list"
                                }
                        })
                    }) {
                        out.complete = false;
                        continue;
                    }
                    match &mutation.kind {
                        MutationKind::Lock { locked, fields }
                            if def.system() == ClassSystem::R6 =>
                        {
                            if fields.is_empty() {
                                out.class_locked = Some(*locked);
                            }
                        }
                        MutationKind::Lock { fields, .. }
                            if def.system() == ClassSystem::RefClass =>
                        {
                            for field in fields {
                                if let ClassName::Known(name) = field {
                                    out.locked_fields.insert(name.clone());
                                } else {
                                    out.complete = false;
                                }
                            }
                        }
                        MutationKind::ClassBindingLock(locked)
                            if def.system() == ClassSystem::RefClass =>
                        {
                            out.class_locked = Some(*locked);
                        }
                        MutationKind::R6Set { group, overwrite }
                            if def.system() == ClassSystem::R6 =>
                        {
                            if out.class_locked.is_none() {
                                out.complete = false;
                                continue;
                            }
                            if !matches!(group.as_deref(), Some("public" | "private" | "active")) {
                                out.complete = false;
                                continue;
                            }
                            for key in &mutation.members {
                                let Some(member) = file
                                    .facts
                                    .oop
                                    .members
                                    .iter()
                                    .find(|member| &member.key == key)
                                else {
                                    continue;
                                };
                                let installed = self.install_member(path, member, callables);
                                let member = &installed.definition;
                                let ClassName::Known(name) = &member.name else {
                                    out.complete = false;
                                    continue;
                                };
                                if member.kind == MemberKind::Unknown {
                                    out.complete = false;
                                    continue;
                                }
                                if member.kind == MemberKind::ActiveBinding
                                    && installed.signature.is_none()
                                {
                                    out.complete = false;
                                    continue;
                                }
                                let existing: Vec<_> = out
                                    .entries
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, entry)| entry.definition.name == member.name)
                                    .collect();
                                let same_group = |other: &MemberDef| {
                                    other.visibility == member.visibility
                                        && other.kind == member.kind
                                };
                                let generated_clone = name == "clone"
                                    && def
                                        .r6
                                        .as_ref()
                                        .is_some_and(|options| options.cloneable == Some(true));
                                if existing
                                    .iter()
                                    .any(|(_, entry)| !same_group(&entry.definition))
                                    || (!existing.is_empty() && *overwrite == Some(false))
                                    || generated_clone
                                        && (*overwrite == Some(false)
                                            || member.visibility != MemberVisibility::Public
                                            || member.kind != MemberKind::Method)
                                {
                                    continue;
                                }
                                if overwrite.is_none() {
                                    out.complete = false;
                                    continue;
                                }
                                out.entries
                                    .retain(|entry| entry.definition.name != member.name);
                                out.entries.push(installed);
                            }
                        }
                        MutationKind::ReferenceMethods if def.system() == ClassSystem::RefClass => {
                            if out.class_locked.is_none() {
                                out.complete = false;
                                continue;
                            }
                            for key in &mutation.members {
                                let Some(member) = file
                                    .facts
                                    .oop
                                    .members
                                    .iter()
                                    .find(|member| &member.key == key)
                                else {
                                    continue;
                                };
                                let installed = self.install_member(path, member, callables);
                                if installed.signature.is_none()
                                    || installed.definition.name == ClassName::Unknown
                                {
                                    out.complete = false;
                                    continue;
                                }
                                out.entries.retain(|entry| {
                                    entry.definition.name != installed.definition.name
                                });
                                out.entries.push(installed);
                            }
                            // NULL removes inherited methods. Own definitions
                            // survive the method table rebuild and are layered
                            // over these removals by reference_members.
                            out.removals.extend(mutation.removals.iter().cloned());
                        }
                        MutationKind::Dynamic => out.complete = false,
                        _ => {}
                    }
                }
                OopEvent::Source(_) => {}
                OopEvent::Binding(name) => {
                    live.remove(name);
                    possible.remove(name);
                    callables.remove(name);
                }
                OopEvent::InvalidateBindings => {
                    callables.clear();
                    out.complete = false;
                }
            }
        }
        if !saw_class {
            let callables = &namespaces
                .entry(class.package_root.clone())
                .or_default()
                .callables;
            out.entries = self
                .file(&id.path)
                .into_iter()
                .flat_map(|file| &file.facts.oop.members)
                .filter(|member| member.class.as_ref() == Some(&id.key))
                .map(|member| self.install_member(&id.path, member, callables))
                .collect();
            out.complete = false;
        }
        out
    }
}
