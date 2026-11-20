//! Rank complete signature vectors by componentwise inheritance distance. R can
//! break an ambiguity arbitrarily; static analysis exposes every best candidate.

use super::execution::S4Execution;
use super::*;
use crate::semantic::enclosure::Availability;
use std::cmp::Ordering;
mod ancestry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum S4SelectionUncertainty {
    InvalidSignature,
    DynamicSignature,
    DynamicInheritance,
    UnknownClass,
    IncompleteAncestry,
    IncompleteScope,
    ConditionalRegistration,
    UnresolvedRegistration,
    AmbiguousGeneric,
    AmbiguousMethod,
    ExternalMethods,
    RuntimeRegistration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum S4Distance {
    Class(u32),
    /// ANY is farther away than every declared superclass, including for missing.
    Any,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4MethodCandidate {
    pub method: S4MethodId,
    pub signature: S4MethodSignature,
    pub distances: Vec<S4Distance>,
    /// The group that supplied a method for a member generic.
    pub group: Option<S4GenericId>,
    pub group_depth: usize,
    pub certain: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4MethodResolution {
    pub signature: S4MethodSignature,
    pub candidates: Vec<S4MethodCandidate>,
    /// The undominated candidates. Competing registrations keep distinct IDs.
    pub best: Vec<S4MethodId>,
    pub selected: Option<S4MethodId>,
    pub uncertainty: BTreeSet<S4SelectionUncertainty>,
}

impl S4MethodResolution {
    pub fn complete(&self) -> bool {
        self.uncertainty.is_empty()
    }
}

/// The caller can add loaded namespaces and execution facts without evaluating
/// R. Package imports load namespaces; workspace membership alone does not.
#[derive(Debug, Clone, Default)]
pub struct S4DispatchContext {
    pub path: PathBuf,
    pub owner: ClassOwner,
    /// Package namespaces established by the caller.
    pub loaded_packages: BTreeSet<PathBuf>,
    /// Packages loaded with attachment of their DESCRIPTION Depends.
    pub attached_packages: BTreeSet<PathBuf>,
    pub executed_registrations: BTreeSet<S4MethodId>,
    /// Established relation ordinals in each file's range-free S4 projection.
    pub executed_relations: BTreeSet<(PathBuf, usize)>,
    pub runtime_registration: bool,
    pub deferred: bool,
    /// Sites explicitly excluded by the caller.
    pub excluded: BTreeSet<S4MethodId>,
    pub(super) execution: Option<S4Execution>,
    pub(super) namespace_load_incomplete: bool,
    pub(super) continuation: bool,
}

impl S4DispatchContext {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.into(),
            deferred: true,
            ..Self::default()
        }
    }
}

pub(super) fn generic_name(id: &S4GenericId) -> Option<&str> {
    match id {
        S4GenericId::Named { name, .. } => Some(name),
        S4GenericId::Declaration(_) => None,
    }
}

pub(crate) type Table<'a> = (&'a S4GenericId, Option<&'a OopId>);

fn matches_table_name(reference: &GenericRef, tables: &BTreeMap<Table<'_>, usize>) -> bool {
    reference.name == ClassName::Unknown
        || tables.keys().any(|(id, _)| {
            matches!((&reference.name, generic_name(id)), (ClassName::Known(name), Some(table)) if name == table)
        })
}

impl S4Generic {
    pub(crate) fn table(&self) -> Table<'_> {
        (&self.id, self.object.as_ref())
    }
}

impl S4Method {
    pub(crate) fn confirmed_registration(&self) -> Option<&OopId> {
        let S4MethodId::Registration(id) = &self.id else {
            return None;
        };
        (!self.uncertainty.contains(&OopUncertainty::UnresolvedCallee)).then_some(id)
    }

    /// A redeclaration can contribute another signature for the same table.
    /// The package and name alone cannot distinguish replacement objects.
    pub(crate) fn unique_table(&self) -> Option<Table<'_>> {
        let (first, rest) = self.dispatch.split_first()?;
        let table = first.table();
        rest.iter()
            .all(|signature| signature.table() == table)
            .then_some(table)
    }
}

impl S4MethodSignature {
    pub(super) fn table(&self) -> Table<'_> {
        (&self.generic, self.object.as_ref())
    }

    fn same_classes(&self, other: &Self) -> bool {
        self.complete && other.complete && self.classes().eq(other.classes())
    }

    fn classes(&self) -> impl Iterator<Item = &DispatchClass> {
        self.arguments.iter().map(|arg| &arg.class)
    }
}

fn registration_issue(issue: OopUncertainty) -> Option<S4SelectionUncertainty> {
    match issue {
        OopUncertainty::Conditional => Some(S4SelectionUncertainty::ConditionalRegistration),
        OopUncertainty::CustomEnvironment
        | OopUncertainty::UnresolvedCallee
        | OopUncertainty::UnprovenMethod
        | OopUncertainty::UnresolvedAlias
        | OopUncertainty::DynamicName => Some(S4SelectionUncertainty::UnresolvedRegistration),
        OopUncertainty::DynamicSignature => Some(S4SelectionUncertainty::DynamicSignature),
        OopUncertainty::InvalidArguments => Some(S4SelectionUncertainty::InvalidSignature),
        _ => None,
    }
}

fn dominates(left: &S4MethodCandidate, right: &S4MethodCandidate) -> bool {
    if !left.certain || !right.certain || left.distances.len() != right.distances.len() {
        return false;
    }
    if left
        .distances
        .iter()
        .chain(&right.distances)
        .any(|d| *d == S4Distance::Unknown)
    {
        return false;
    }
    let no_worse = left
        .distances
        .iter()
        .zip(&right.distances)
        .all(|(l, r)| l <= r);
    let closer = left
        .distances
        .iter()
        .zip(&right.distances)
        .any(|(l, r)| l < r);
    no_worse
        && (closer
            || left.signature.same_classes(&right.signature)
                && (left.group_depth < right.group_depth
                    || left.signature.table() == right.signature.table()
                        && matches!(left.method, S4MethodId::Registration(_))
                        && matches!(right.method, S4MethodId::Default(_))))
}

impl ClassIndex {
    pub(super) fn s4_generic_activity(
        &self,
        context: &S4DispatchContext,
        id: &OopId,
        loaded: &BTreeSet<PathBuf>,
    ) -> Availability {
        let file = self.s4_file_activity(context, &id.path, loaded);
        self.s4_site_activity(context, &id.path, &id.key.owner, file, |execution| {
            execution.oop.contains(&id.key)
        })
    }

    pub(super) fn s4_group_depths<'a>(
        &'a self,
        context: &S4DispatchContext,
        generics: &[&S4Generic],
        loaded: &BTreeSet<PathBuf>,
    ) -> (BTreeMap<Table<'a>, usize>, bool) {
        let mut tables = BTreeMap::new();
        let mut complete = true;
        let mut pending: Vec<_> = generics
            .iter()
            .flat_map(|generic| &generic.groups)
            .map(|group| (group, 1))
            .collect();
        while let Some((group, depth)) = pending.pop() {
            let ClassName::Known(name) = group else {
                complete = false;
                continue;
            };
            let mut matches = BTreeSet::new();
            for group in self
                .s4
                .generics
                .iter()
                .filter(|generic| generic.group && generic_name(&generic.id) == Some(name))
            {
                if let Some(id) = &group.declaration {
                    match self.s4_generic_activity(context, id, loaded) {
                        Availability::Absent => continue,
                        Availability::Unknown => complete = false,
                        Availability::Present => {}
                    }
                }
                let table = group.table();
                matches.insert(table);
                if tables.get(&table).is_none_or(|previous| *previous > depth) {
                    tables.insert(table, depth);
                    pending.extend(group.groups.iter().map(|group| (group, depth + 1)));
                }
            }
            complete &= matches.len() == 1;
        }
        (tables, complete)
    }

    fn s4_method_activity(
        &self,
        context: &S4DispatchContext,
        method: &S4Method,
        loaded: &BTreeSet<PathBuf>,
    ) -> Availability {
        if context.executed_registrations.contains(&method.id) {
            return Availability::Present;
        }
        let (S4MethodId::Registration(id) | S4MethodId::Default(id)) = &method.id;
        self.s4_generic_activity(context, id, loaded)
    }

    fn s4_registration_removed(
        &self,
        context: &S4DispatchContext,
        method: &S4Method,
        defined: &S4MethodSignature,
        removals: &[(&S4Removal, bool)],
    ) -> Availability {
        let mut status = Availability::Absent;
        for &(removal, certain) in removals {
            if !removal.dispatch.iter().any(|removed| {
                removed.table() == defined.table()
                    && (removed.same_classes(defined) || !removed.complete || !defined.complete)
            }) {
                continue;
            }
            match self.s4_installation_order(context, method.id.source(), &removal.source) {
                Some(Ordering::Less) if certain && defined.complete => {
                    return Availability::Present;
                }
                Some(Ordering::Greater) => continue,
                _ => {}
            }
            status = Availability::Unknown;
        }
        status
    }

    fn s4_installation_order(
        &self,
        context: &S4DispatchContext,
        left: &OopId,
        right: &OopId,
    ) -> Option<Ordering> {
        // Different runtime frames and separately loaded namespaces can write
        // the same table without giving us a relative installation order.
        if !left.key.owner.0.is_empty() || !right.key.owner.0.is_empty() {
            let same_frame = left.path == context.path
                && right.path == context.path
                && left.key.owner == context.owner
                && right.key.owner == context.owner
                && context.execution.as_ref().is_some_and(|execution| {
                    execution.oop.contains(&left.key) && execution.oop.contains(&right.key)
                });
            if !same_frame {
                return None;
            }
        }
        let position =
            |id: &OopId| super::registry::s4_event_position(self.file(&id.path)?, &id.key);
        self.s4_event_order(
            (&left.path, position(left)?),
            (&right.path, position(right)?),
        )
    }

    /// Resolve every dispatch position. Unknown facts stay candidates, and no
    /// ordering or superclass spelling is used to choose an ambiguous method.
    pub fn s4_resolve_methods(
        &self,
        context: &S4DispatchContext,
        signature: &S4MethodSignature,
        inherited: bool,
    ) -> S4MethodResolution {
        self.s4_resolve_methods_with(context, signature, S4Inheritance::Explicit(inherited))
    }

    pub(super) fn s4_resolve_methods_with(
        &self,
        context: &S4DispatchContext,
        signature: &S4MethodSignature,
        inheritance: S4Inheritance,
    ) -> S4MethodResolution {
        let mut result = S4MethodResolution {
            signature: signature.clone(),
            candidates: Vec::new(),
            best: Vec::new(),
            selected: None,
            uncertainty: BTreeSet::new(),
        };
        if !signature.valid {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::InvalidSignature);
            return result;
        }
        if !signature.complete {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::DynamicSignature);
        }
        if !self.class_file_complete(&context.path)
            || context.namespace_load_incomplete
            || context
                .execution
                .as_ref()
                .is_some_and(|execution| execution.source_incomplete)
        {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::IncompleteScope);
        }
        if context.runtime_registration {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::RuntimeRegistration);
        }
        let mut explicit = context.loaded_packages.clone();
        if let Some(root) = signature
            .declaration
            .as_ref()
            .and_then(|declaration| self.file(&declaration.path))
            .and_then(|file| file.package_root.as_ref())
        {
            explicit.insert(root.clone());
        }
        let (loaded, complete) = self.s4_dispatch_packages(context, &explicit);
        if !complete {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::IncompleteScope);
        }
        let coerce = matches!(&signature.generic, S4GenericId::Named { package, name }
            if package == "methods" && name == "coerce" && signature.object.is_none());
        let dots = signature
            .arguments
            .first()
            .is_some_and(|argument| argument.argument.as_deref() == Some("..."));
        let inherits_at = |index: usize, actual: &DispatchArgument| match inheritance {
            S4Inheritance::Default => {
                dots || actual.class != DispatchClass::Any && (!coerce || index != 1)
            }
            S4Inheritance::Explicit(value) => value,
            S4Inheritance::Dynamic => true,
        };
        let inherits = signature
            .arguments
            .iter()
            .enumerate()
            .any(|(index, argument)| inherits_at(index, argument));
        if matches!(inheritance, S4Inheritance::Dynamic) {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::DynamicInheritance);
        }
        let generics: Vec<_> = self
            .s4
            .generics
            .iter()
            .filter(|generic| generic.id == signature.generic && generic.object == signature.object)
            .collect();
        if generics.is_empty() {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::AmbiguousGeneric);
        }
        if signature.declaration.as_ref().is_some_and(|id| {
            self.s4_generic_activity(context, id, &loaded) != Availability::Present
        }) {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::AmbiguousGeneric);
        }
        for generic in &generics {
            result.uncertainty.extend(
                generic
                    .uncertainty
                    .iter()
                    .copied()
                    .filter_map(registration_issue),
            );
        }
        let (mut tables, groups_complete) = if inherits {
            self.s4_group_depths(context, &generics, &loaded)
        } else {
            (BTreeMap::new(), true)
        };
        tables.insert(signature.table(), 0);
        if !groups_complete {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::AmbiguousGeneric);
        }
        if tables.keys().any(|(identity, _)| matches!(identity, S4GenericId::Named { package, .. } if package != ".GlobalEnv" && !self.s4_workspace_package(package))) {
            result.uncertainty.insert(S4SelectionUncertainty::ExternalMethods);
        }
        let simple_only = inherits
            && generics
                .iter()
                .any(|generic| generic.simple_inheritance_only == Some(true));
        if inherits
            && generics
                .iter()
                .any(|generic| generic.simple_inheritance_only.is_none())
        {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::DynamicInheritance);
        }
        let relations = inherits.then(|| self.s4_relation_edges(context, &loaded, simple_only));
        let ancestry: Vec<_> = signature
            .arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let DispatchClass::Formal(name) = &argument.class else {
                    return None;
                };
                let ancestry = self.s4_ancestry(
                    context,
                    name,
                    &loaded,
                    relations.as_ref().filter(|_| inherits_at(index, argument)),
                );
                if !ancestry.known {
                    result
                        .uncertainty
                        .insert(S4SelectionUncertainty::UnknownClass);
                }
                Some(ancestry)
            })
            .collect();
        let removals: Vec<_> = self.s4.removals.iter().filter_map(|removal| {
            if !removal.dispatch.iter().any(|defined| tables.contains_key(&defined.table()))
                && !(removal.dispatch.is_empty() && matches_table_name(&removal.generic, &tables))
            {
                return None;
            }
            let activity = self.s4_generic_activity(context, &removal.source, &loaded);
            if activity == Availability::Absent {
                return None;
            }
            let certain = activity == Availability::Present
                && removal.uncertainty.is_empty()
                && matches!(removal.dispatch.as_slice(), [defined] if defined.complete && defined.valid);
            if !certain {
                result.uncertainty.insert(S4SelectionUncertainty::RuntimeRegistration);
            }
            Some((removal, certain))
        }).collect();
        let active: Vec<_> = self
            .s4
            .methods
            .iter()
            .filter_map(|method| {
                let activity = self.s4_method_activity(context, method, &loaded);
                (activity != Availability::Absent
                    && !method
                        .uncertainty
                        .contains(&OopUncertainty::InvalidArguments))
                .then_some((method, activity))
            })
            .collect();
        let mut installations = HashMap::<_, Vec<_>>::new();
        for &(method, activity) in &active {
            if let S4MethodId::Registration(id) = &method.id
                && activity == Availability::Present
                && let [defined] = method.dispatch.as_slice()
                && defined.valid
                && defined.complete
                && tables.contains_key(&defined.table())
                && self.class_file_complete(&method.owner.path)
                && method
                    .uncertainty
                    .iter()
                    .copied()
                    .all(|issue| registration_issue(issue).is_none())
            {
                installations
                    .entry((defined.table(), defined.classes().collect::<Vec<_>>()))
                    .or_default()
                    .push(id);
            }
        }
        let mut direct_dots = Vec::new();
        for (method, activity) in active {
            if context.excluded.contains(&method.id) {
                continue;
            }
            let unresolved;
            let dispatch = if method.dispatch.is_empty()
                && method.generics.is_empty()
                && matches_table_name(&method.generic, &tables)
            {
                result
                    .uncertainty
                    .insert(S4SelectionUncertainty::UnresolvedRegistration);
                unresolved = vec![S4MethodSignature {
                    arguments: signature
                        .arguments
                        .iter()
                        .map(|argument| DispatchArgument {
                            argument: argument.argument.clone(),
                            class: DispatchClass::Unknown,
                        })
                        .collect(),
                    complete: false,
                    ..signature.clone()
                }];
                &unresolved
            } else {
                &method.dispatch
            };
            for defined in dispatch {
                let Some(&group_depth) = tables.get(&defined.table()) else {
                    continue;
                };
                if !defined.valid
                    || context.continuation
                        && matches!(method.id, S4MethodId::Registration(_))
                        && signature.same_classes(defined)
                {
                    continue;
                }
                if let S4MethodId::Registration(id) = &method.id
                    && defined.complete
                    && installations
                        .get(&(defined.table(), defined.classes().collect::<Vec<_>>()))
                        .is_some_and(|later| {
                            later.iter().any(|other| {
                                id != *other
                                    && self.s4_installation_order(context, id, other)
                                        == Some(Ordering::Less)
                            })
                        })
                {
                    continue;
                }
                let group = (group_depth > 0).then(|| defined.generic.clone());
                let mut certain = defined.complete && signature.complete;
                let direct_dot = dots
                    && group_depth == 0
                    && certain
                    && matches!(defined.arguments.as_slice(), [arg]
                        if signature.classes().any(|class| *class == arg.class));
                let mut distances = Vec::new();
                let mut possible = true;
                if !defined.complete || !signature.complete {
                    result
                        .uncertainty
                        .insert(S4SelectionUncertainty::DynamicSignature);
                    distances.resize(
                        if dots { 1 } else { signature.arguments.len() },
                        S4Distance::Unknown,
                    );
                } else if defined.arguments.len()
                    != if dots { 1 } else { signature.arguments.len() }
                {
                    possible = false;
                } else {
                    for (index, actual) in signature.arguments.iter().enumerate() {
                        let required = &defined.arguments[if dots { 0 } else { index }];
                        let (distance, known) = self.s4_argument_distance(
                            context,
                            method,
                            actual,
                            &required.class,
                            &loaded,
                            ancestry[index].as_ref(),
                        );
                        certain &= known;
                        match distance {
                            Some(distance)
                                if inherits_at(index, actual)
                                    || distance == S4Distance::Class(0)
                                    || distance == S4Distance::Unknown =>
                            {
                                distances.push(distance)
                            }
                            _ => {
                                possible = false;
                                break;
                            }
                        }
                    }
                    if dots && possible {
                        // A dots method covers every argument class. R ranks
                        // such methods by their closest match across the dots.
                        let distance = if distances.contains(&S4Distance::Unknown) {
                            S4Distance::Unknown
                        } else {
                            distances.iter().copied().min().unwrap_or(S4Distance::Any)
                        };
                        distances.clear();
                        distances.push(distance);
                    }
                }
                if !possible && !direct_dot {
                    continue;
                }
                if activity == Availability::Unknown {
                    certain = false;
                    result
                        .uncertainty
                        .insert(if method.owner.lexical.0.is_empty() {
                            S4SelectionUncertainty::IncompleteScope
                        } else {
                            S4SelectionUncertainty::RuntimeRegistration
                        });
                }
                match self.s4_registration_removed(context, method, defined, &removals) {
                    Availability::Present => continue,
                    Availability::Unknown => {
                        certain = false;
                        result
                            .uncertainty
                            .insert(S4SelectionUncertainty::RuntimeRegistration);
                    }
                    Availability::Absent => {}
                }
                if !self.class_file_complete(&method.owner.path) {
                    certain = false;
                    result
                        .uncertainty
                        .insert(S4SelectionUncertainty::IncompleteScope);
                }
                if !certain {
                    result
                        .uncertainty
                        .insert(S4SelectionUncertainty::IncompleteAncestry);
                }
                for issue in &method.uncertainty {
                    if let Some(issue) = registration_issue(*issue) {
                        certain = false;
                        result.uncertainty.insert(issue);
                    }
                }
                let candidate = S4MethodCandidate {
                    method: method.id.clone(),
                    signature: defined.clone(),
                    distances,
                    group,
                    group_depth,
                    certain,
                };
                if direct_dot {
                    direct_dots.push(S4MethodCandidate {
                        distances: vec![S4Distance::Class(0)],
                        ..candidate.clone()
                    });
                }
                if possible {
                    result.candidates.push(candidate);
                }
            }
        }
        if dots && let Some(first) = signature.classes().next() {
            let has_direct = |class: &DispatchClass, certain: bool| {
                direct_dots.iter().any(|candidate| {
                    (!certain || candidate.certain)
                        && candidate.signature.classes().next() == Some(class)
                })
            };
            if signature.classes().all(|class| has_direct(class, false)) {
                // When every queried class has a direct entry, selectMethod
                // uses the entry for the first class in the supplied vector.
                if signature.classes().all(|class| has_direct(class, true)) {
                    result.candidates.clear();
                }
                for candidate in direct_dots
                    .into_iter()
                    .filter(|candidate| candidate.signature.classes().next() == Some(first))
                {
                    if !result.candidates.contains(&candidate) {
                        result.candidates.push(candidate);
                    }
                }
            }
        }
        for candidate in &result.candidates {
            if !result
                .candidates
                .iter()
                .any(|other| dominates(other, candidate))
                && !result.best.contains(&candidate.method)
            {
                result.best.push(candidate.method.clone());
            }
        }
        if result.best.len() > 1 {
            result
                .uncertainty
                .insert(S4SelectionUncertainty::AmbiguousMethod);
        }
        if result.complete()
            && let [selected] = result.best.as_slice()
        {
            result.selected = Some(selected.clone());
        }
        result
    }
}
