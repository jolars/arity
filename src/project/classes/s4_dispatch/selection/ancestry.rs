//! Formal class identity and inheritance, separate from method ranking.

use super::*;
use crate::semantic::enclosure::Availability::{Absent, Present, Unknown};
use std::cmp::Ordering;
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FormalType {
    Declaration(ClassId),
    Builtin(String),
    /// A literal class label whose indexed definition has yet to be installed.
    Unregistered(String),
}

pub(super) struct Ancestry {
    distances: BTreeMap<FormalType, u32>,
    pub(super) known: bool,
    complete: bool,
}

struct RelationParent<'a> {
    path: &'a Path,
    ordinal: usize,
    relation: &'a S4Relation,
    targets: BTreeSet<FormalType>,
    complete: bool,
}

pub(super) struct RelationEdges<'a> {
    parents: BTreeMap<FormalType, Vec<RelationParent<'a>>>,
    complete: bool,
}

struct FormalTypes {
    types: BTreeSet<FormalType>,
    known: bool,
}

impl ClassIndex {
    fn s4_class_activity(
        &self,
        context: &S4DispatchContext,
        id: &ClassId,
        loaded: &BTreeSet<PathBuf>,
    ) -> Availability {
        let file = self.s4_file_activity(context, &id.path, loaded);
        self.s4_site_activity(context, &id.path, &id.key.owner, file, |execution| {
            execution.classes.contains(&id.key)
        })
    }

    /// Class strings can reach a loaded namespace's metadata, including private
    /// classes. Ordinary lexical lookup has its own import and export rules.
    fn s4_formal_types(
        &self,
        context: &S4DispatchContext,
        path: &Path,
        owner: &ClassOwner,
        name: &str,
        loaded: &BTreeSet<PathBuf>,
    ) -> FormalTypes {
        let mut types = BTreeSet::new();
        let mut certain = true;
        let mut inactive = false;
        let mut consider = |id: ClassId| {
            let activity = self.s4_class_activity(context, &id, loaded);
            match activity {
                Absent => inactive = true,
                Present | Unknown => certain &= activity == Present,
            }
            (activity != Absent).then_some(FormalType::Declaration(id))
        };
        let reference = ParentRef::Formal(name.into());
        for depth in (0..=owner.0.len()).rev() {
            types.extend(
                self.resolve_reference(
                    path,
                    &ClassOwner(owner.0[..depth].to_vec()),
                    &reference,
                    true,
                )
                .into_iter()
                .filter_map(&mut consider),
            );
            if !types.is_empty() {
                break;
            }
        }
        if types.is_empty() {
            types.extend(
                self.oop_files()
                    .filter(|file| {
                        file.package_root
                            .as_ref()
                            .is_some_and(|root| loaded.contains(root))
                            || context.execution.as_ref().is_some_and(|execution| {
                                execution.source_activity(&file.path) != Absent
                            })
                    })
                    .flat_map(|file| {
                        file.facts.declarations.iter().filter(|def| {
                            def.key.owner.0.is_empty()
                                && matches!(def.system(), ClassSystem::S4 | ClassSystem::RefClass)
                                && matches!(&def.name, ClassName::Known(candidate) if candidate == name)
                        }).map(|def| ClassId {
                                path: file.path.clone(),
                                key: def.key.clone(),
                        })
                    })
                    .filter(|id| self.declarations.contains_key(id))
                    .filter_map(consider),
            );
        }
        if types.is_empty() {
            if builtins::classes().contains_key(name) {
                types.insert(FormalType::Builtin(name.into()));
            } else if inactive {
                types.insert(FormalType::Unregistered(name.into()));
            }
        }
        FormalTypes {
            known: certain && types.len() == 1,
            types,
        }
    }

    fn s4_reference_parent(
        &self,
        context: &S4DispatchContext,
        id: &ClassId,
        loaded: &BTreeSet<PathBuf>,
    ) -> bool {
        let proven_parents = |id: &ClassId| {
            self.parent_candidates
                .get(id)
                .into_iter()
                .flatten()
                .enumerate()
                .filter(|(ordinal, _)| {
                    self.s4_structural_parents
                        .get(id)
                        .is_some_and(|proved| proved.contains(ordinal))
                })
                .flat_map(|(_, parents)| parents.iter().cloned())
                .collect::<Vec<_>>()
        };
        let mut pending = proven_parents(id);
        let mut seen = BTreeSet::new();
        while let Some(parent) = pending.pop() {
            if seen.len() >= 256
                || !seen.insert(parent.clone())
                || self.s4_class_activity(context, &parent, loaded) != Present
            {
                continue;
            }
            let class = &self.declarations[&parent].declaration;
            if class.system() == ClassSystem::RefClass
                || class
                    .parents
                    .contains(&ParentRef::Formal("envRefClass".into()))
            {
                return true;
            }
            pending.extend(proven_parents(&parent));
        }
        false
    }

    fn s4_declaration_parents(
        &self,
        context: &S4DispatchContext,
        id: &ClassId,
        loaded: &BTreeSet<PathBuf>,
        relations: &RelationEdges<'_>,
        templates: &mut BTreeSet<ClassId>,
    ) -> (BTreeSet<FormalType>, bool) {
        let mut parents = BTreeSet::new();
        let def = &self.declarations[id].declaration;
        let mut complete = def.binding_known()
            && self.class_file_complete(&id.path)
            && def
                .parent_vector
                .as_ref()
                .is_none_or(|helper| self.callee_identity(&id.path, helper).is_some());
        let mut parent_count = def.parents.len();
        if let Some(s4) = &def.s4 {
            complete &= s4.parents_complete
                && s4
                    .schema_helpers
                    .iter()
                    .all(|helper| self.callee_identity(&id.path, helper).is_some());
            if let Some(old) = &s4.old_class {
                let bridge = if s4.definition == S4Definition::OldClass {
                    Present
                } else {
                    self.s4_site_activity(context, &id.path, &id.key.owner, Present, |execution| {
                        execution.bridges.contains(&id.key)
                    })
                };
                if bridge == Absent {
                    parent_count = old.parent_start;
                } else {
                    let known = bridge == Present && self.s4_parent_known(id, old.parent_start);
                    complete &= known;
                    if known && old.index + 1 == old.classes.len() {
                        parents.insert(FormalType::Builtin("oldClass".into()));
                    }
                    if s4.definition == S4Definition::OldClass
                        && let Some(reference) = &old.s4_class
                    {
                        let candidates =
                            self.resolve_reference(&id.path, &id.key.owner, reference, true);
                        if known
                            && let [template] = candidates.as_slice()
                            && self.s4_class_activity(context, template, loaded) == Present
                            && templates.len() < 64
                            && templates.insert(template.clone())
                        {
                            let (copied, known) = self.s4_declaration_parents(
                                context, template, loaded, relations, templates,
                            );
                            // The bridge copied the template at its installation,
                            // so present relations on the template may be newer.
                            complete &= !relations
                                .parents
                                .contains_key(&FormalType::Declaration(template.clone()));
                            templates.remove(template);
                            parents.extend(copied);
                            complete &= known;
                        } else {
                            complete = false;
                        }
                    }
                }
            }
        }
        if def.system() == ClassSystem::RefClass && !self.s4_reference_parent(context, id, loaded) {
            parents.insert(FormalType::Builtin("envRefClass".into()));
        }
        for (ordinal, reference) in def.parents.iter().take(parent_count).enumerate() {
            let candidates = self
                .parent_candidates
                .get(id)
                .and_then(|parents| parents.get(ordinal))
                .map(Vec::as_slice)
                .unwrap_or_default();
            if !self.s4_parent_known(id, ordinal) {
                complete = false;
            } else if let [parent] = candidates {
                if self
                    .s4_structural_parents
                    .get(id)
                    .is_some_and(|proved| proved.contains(&ordinal))
                    && self.s4_class_activity(context, parent, loaded) == Present
                {
                    parents.insert(FormalType::Declaration(parent.clone()));
                } else {
                    complete = false;
                }
            } else if candidates.is_empty()
                && let ParentRef::Formal(name) = reference
                && builtins::classes().contains_key(name)
            {
                parents.insert(FormalType::Builtin(name.clone()));
            } else {
                complete = false;
            }
        }
        (parents, complete)
    }

    pub(super) fn s4_relation_edges<'a>(
        &'a self,
        context: &S4DispatchContext,
        loaded: &BTreeSet<PathBuf>,
        simple_only: bool,
    ) -> RelationEdges<'a> {
        let mut edges = BTreeMap::<_, Vec<_>>::new();
        let mut complete = true;
        for file in self
            .oop_files()
            .filter(|file| !file.facts.s4.relations.is_empty())
        {
            let file_activity = self.s4_file_activity(context, &file.path, loaded);
            if file_activity == Absent {
                continue;
            }
            for (ordinal, relation) in file.facts.s4.relations.iter().enumerate() {
                let activity = if context
                    .executed_relations
                    .contains(&(file.path.clone(), ordinal))
                {
                    Present
                } else {
                    self.s4_site_activity(
                        context,
                        &file.path,
                        &relation.owner,
                        file_activity,
                        |execution| execution.relations.contains(&ordinal),
                    )
                };
                if activity == Absent
                    || (simple_only && relation.simple == Some(false))
                    || relation
                        .uncertainty
                        .contains(&OopUncertainty::InvalidArguments)
                    || self.callee_availability(&file.path, &relation.factory) == Absent
                {
                    continue;
                }
                let class_types = |name: &str| {
                    self.s4_formal_types(context, &file.path, &relation.owner, name, loaded)
                };
                let ClassName::Known(name) = &relation.subclass else {
                    complete = false;
                    continue;
                };
                let from = class_types(name);
                let to = match &relation.superclass {
                    ClassName::Known(name) => Some(class_types(name)),
                    _ => None,
                };
                let known = activity == Present
                    && from.known
                    && (!simple_only || relation.simple == Some(true))
                    && !relation.conditional
                    && relation.uncertainty.is_empty()
                    && self.class_file_complete(&file.path)
                    && self
                        .callee_identity(&file.path, &relation.factory)
                        .is_some()
                    && relation
                        .helpers
                        .iter()
                        .all(|helper| self.callee_identity(&file.path, helper).is_some());
                let mut targets = BTreeSet::new();
                let mut targets_complete = false;
                if known && let Some(to) = to.filter(|to| to.known) {
                    targets = to.types;
                    targets_complete = !targets
                        .iter()
                        .any(|target| matches!(target, FormalType::Unregistered(_)));
                    targets.retain(|target| !matches!(target, FormalType::Unregistered(_)));
                }
                for from in from.types {
                    if matches!(from, FormalType::Unregistered(_)) {
                        continue;
                    }
                    edges.entry(from).or_default().push(RelationParent {
                        path: &file.path,
                        ordinal,
                        relation,
                        targets: targets.clone(),
                        complete: targets_complete,
                    });
                }
            }
        }
        RelationEdges {
            parents: edges,
            complete,
        }
    }

    fn s4_inherits_uncompleted_relation(
        &self,
        root: Option<&FormalType>,
        subclass: &FormalType,
        edge: &RelationParent<'_>,
    ) -> crate::semantic::enclosure::Availability {
        if root == Some(subclass) {
            return Present;
        }
        if matches!(root, Some(FormalType::Builtin(_))) {
            return Absent;
        }
        let order =
            || {
                let FormalType::Declaration(id) = root? else {
                    return None;
                };
                if !id.key.owner.0.is_empty() {
                    return None;
                }
                let class =
                    self.file(&id.path)?.facts.oop.events.iter().position(
                        |event| matches!(event, OopEvent::Class(key) if key == &id.key),
                    )?;
                self.s4_event_order((&id.path, class), self.s4_relation_event(edge)?)
            };
        match order() {
            Some(Ordering::Greater) => Present,
            Some(Ordering::Less) => Absent,
            _ => Unknown,
        }
    }

    fn s4_relation_event<'a>(&self, edge: &RelationParent<'a>) -> Option<(&'a Path, usize)> {
        if !edge.relation.owner.0.is_empty() {
            return None;
        }
        let event = self.file(edge.path)?.facts.oop.events.iter().position(
            |event| matches!(event, OopEvent::S4Relation(ordinal) if *ordinal == edge.ordinal),
        )?;
        Some((edge.path, event))
    }

    pub(super) fn s4_ancestry(
        &self,
        context: &S4DispatchContext,
        name: &str,
        loaded: &BTreeSet<PathBuf>,
        relations: Option<&RelationEdges<'_>>,
    ) -> Ancestry {
        let roots = self.s4_formal_types(context, &context.path, &context.owner, name, loaded);
        let mut result = Ancestry {
            distances: BTreeMap::new(),
            known: roots.known,
            complete: roots.known && self.class_file_complete(&context.path),
        };
        let Some(relations) = relations else {
            result
                .distances
                .extend(roots.types.into_iter().map(|root| (root, 0)));
            return result;
        };
        result.complete &= relations.complete;
        let root = roots.known.then(|| roots.types.first()).flatten().cloned();
        let mut edges = BTreeMap::new();
        let mut terminal = BTreeMap::<_, u32>::new();
        let mut skipped_relations = Vec::new();
        let mut completing_relations = Vec::new();
        let mut pending: VecDeque<_> = roots.types.into_iter().map(|root| (root, 0)).collect();
        while let Some((class, distance)) = pending.pop_front() {
            if result.distances.contains_key(&class) {
                continue;
            }
            if result.distances.len() >= 256 {
                result.complete = false;
                break;
            }
            let mut parents = match &class {
                FormalType::Builtin(name) => builtins::classes()
                    .get(name)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .map(FormalType::Builtin)
                    .collect(),
                FormalType::Declaration(id) => {
                    let (parents, complete) = self.s4_declaration_parents(
                        context,
                        id,
                        loaded,
                        relations,
                        &mut BTreeSet::from([id.clone()]),
                    );
                    result.complete &= complete;
                    parents
                }
                FormalType::Unregistered(_) => BTreeSet::new(),
            };
            for relation in relations.parents.get(&class).into_iter().flatten() {
                if relation.relation.do_complete == Some(false) {
                    match self.s4_inherits_uncompleted_relation(root.as_ref(), &class, relation) {
                        Absent => {
                            skipped_relations.push(relation);
                            continue;
                        }
                        Unknown => result.complete = false,
                        Present => {}
                    }
                    // The direct extension is copied into later subclasses.
                    // The superclass's own ancestry was never copied into it.
                    for target in &relation.targets {
                        terminal
                            .entry(target.clone())
                            .and_modify(|previous| *previous = (*previous).min(distance + 1))
                            .or_insert(distance + 1);
                    }
                } else {
                    if relation.relation.kind == S4RelationKind::Is {
                        completing_relations.push(relation);
                    }
                    parents.extend(relation.targets.iter().cloned());
                }
                result.complete &= relation.complete && relation.relation.do_complete.is_some();
            }
            pending.extend(parents.iter().cloned().map(|parent| (parent, distance + 1)));
            result.distances.insert(class.clone(), distance);
            edges.insert(class, parents);
        }
        // A later setIs can make a preexisting class a new subclass. When the
        // ancestry involves such a completion, the class date alone cannot
        // rule out a skipped extension.
        result.complete &= skipped_relations.iter().all(|skipped| {
            completing_relations.iter().all(|completing| {
                self.s4_relation_event(completing)
                    .zip(self.s4_relation_event(skipped))
                    .and_then(|(left, right)| self.s4_event_order(left, right))
                    == Some(Ordering::Less)
            })
        });
        for (class, distance) in terminal {
            result
                .distances
                .entry(class)
                .and_modify(|previous| *previous = (*previous).min(distance))
                .or_insert(distance);
        }
        fn acyclic(
            class: &FormalType,
            edges: &BTreeMap<FormalType, BTreeSet<FormalType>>,
            active: &mut BTreeSet<FormalType>,
            done: &mut BTreeSet<FormalType>,
        ) -> bool {
            if done.contains(class) {
                return true;
            }
            if !active.insert(class.clone()) {
                return false;
            }
            if edges
                .get(class)
                .into_iter()
                .flatten()
                .any(|parent| !acyclic(parent, edges, active, done))
            {
                return false;
            }
            active.remove(class);
            done.insert(class.clone());
            true
        }
        let mut done = BTreeSet::new();
        result.complete &= edges
            .keys()
            .all(|class| acyclic(class, &edges, &mut BTreeSet::new(), &mut done));
        result
    }

    pub(super) fn s4_argument_distance(
        &self,
        context: &S4DispatchContext,
        method: &S4Method,
        actual: &DispatchArgument,
        required: &DispatchClass,
        loaded: &BTreeSet<PathBuf>,
        ancestry: Option<&Ancestry>,
    ) -> (Option<S4Distance>, bool) {
        let actual = &actual.class;
        if matches!(actual, DispatchClass::Unknown | DispatchClass::Generator(_))
            || matches!(
                required,
                DispatchClass::Unknown | DispatchClass::Generator(_)
            )
        {
            return (Some(S4Distance::Unknown), false);
        }
        if required == &DispatchClass::Any {
            return (
                Some(if actual == &DispatchClass::Any {
                    S4Distance::Class(0)
                } else {
                    S4Distance::Any
                }),
                true,
            );
        }
        if actual == &DispatchClass::Missing || required == &DispatchClass::Missing {
            return ((actual == required).then_some(S4Distance::Class(0)), true);
        }
        let (Some(ancestry), DispatchClass::Formal(required)) = (ancestry, required) else {
            return (None, true);
        };
        let required = self.s4_formal_types(
            context,
            &method.owner.path,
            &method.owner.lexical,
            required,
            loaded,
        );
        let certain = ancestry.complete && required.known;
        let distance = required
            .types
            .iter()
            .filter_map(|required| ancestry.distances.get(required))
            .min()
            .copied();
        match distance {
            Some(distance) => (Some(S4Distance::Class(distance)), certain),
            None if certain => (None, true),
            None => (Some(S4Distance::Unknown), false),
        }
    }
}
