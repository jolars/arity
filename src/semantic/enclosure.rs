//! Runtime method enclosures supplied to the single-file lexical resolver.
//!
//! Project analysis proves which environments a function can acquire. This
//! module resolves reads and nonlocal writes through those environments; it
//! neither recognizes class declarations nor performs cross-file lookup.

use std::collections::{BTreeMap, HashMap, HashSet};

use rowan::{TextRange, TextSize};
use smol_str::SmolStr;

use super::{BindingId, ScopeId, ScopeKind, SemanticModel, binding_name_key};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum Availability {
    Present,
    Absent,
    Unknown,
}

/// An identity within this semantic model, deliberately without a source span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeneratedBinding {
    pub origin: GeneratedOrigin,
    pub name: SmolStr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GeneratedOrigin {
    Method(u32),
    Scope(u32),
}

/// A registration can introduce a name without an editable identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeBinding {
    pub scope: ScopeId,
    pub name: SmolStr,
    pub available_from: TextSize,
    pub availability: Availability,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentifierResolution {
    pub lexical: Vec<BindingId>,
    pub generated: Vec<GeneratedBinding>,
    pub uncertain: bool,
    /// At least one possible enclosure reaches the file's external scope.
    pub external: bool,
}

impl IdentifierResolution {
    pub fn is_free(&self) -> bool {
        self.lexical.is_empty() && self.generated.is_empty() && !self.uncertain
    }

    pub fn is_source_only(&self) -> bool {
        self.generated.is_empty() && !self.uncertain
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodEnclosure {
    /// The class creation frame, rather than the function's original enclosure.
    /// An unknown custom environment retains this frame as a possible source
    /// of reads, without proving that navigation can target its bindings.
    pub parent: Option<ScopeId>,
    pub parent_is_unknown: bool,
    pub bindings: BTreeMap<SmolStr, Availability>,
    /// Computed member lists or unresolved ancestors can supply additional names.
    pub unknown_members: bool,
    /// Dispatch metadata lives in the call frame. A direct `<<-` skips it,
    /// whereas a write from a nested closure can reach it.
    pub call_frame: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodImplementation {
    pub scope: ScopeId,
    pub enclosure: u32,
    /// An aliased function also keeps its original, ordinary closure value.
    pub keep_original: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MethodEnclosures {
    environments: Vec<MethodEnclosure>,
    functions: HashMap<ScopeId, Vec<Option<u32>>>,
    scopes: HashSet<ScopeId>,
    method_scopes: HashSet<ScopeId>,
    writes: HashSet<TextRange>,
    bindings: Vec<ScopeBinding>,
    nonlocal_writes: HashMap<TextRange, NonlocalWrite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NonlocalWrite {
    name: SmolStr,
    origin: ScopeId,
}

impl SemanticModel {
    /// Resolve a copy of the lexical model under supplied method environments.
    /// No synthetic source bindings or definition ranges are manufactured.
    pub fn with_method_enclosures(
        &self,
        environments: Vec<MethodEnclosure>,
        implementations: &[MethodImplementation],
    ) -> Self {
        let mut model = self.clone();
        let offset = model.method_enclosures.environments.len() as u32;
        model.method_enclosures.environments.extend(environments);
        for implementation in implementations {
            let candidates = model
                .method_enclosures
                .functions
                .entry(implementation.scope)
                .or_default();
            if implementation.keep_original && !candidates.contains(&None) {
                candidates.push(None);
            }
            if !candidates.contains(&Some(offset + implementation.enclosure)) {
                candidates.push(Some(offset + implementation.enclosure));
            }
        }
        model.refresh_contextual_resolution();
        model
    }

    pub fn with_scope_bindings(&self, bindings: Vec<ScopeBinding>) -> Self {
        let mut model = self.clone();
        // Converting an ordinary function into a generic consumes its old
        // value. The literal registration name is a use, not a rename site.
        model
            .use_only_reads
            .extend(bindings.iter().filter_map(|binding| {
                Some(super::UseOnlyRead {
                    name: binding.name.clone(),
                    scope: binding.scope,
                    range: TextRange::empty(binding.available_from.checked_sub(TextSize::new(1))?),
                    deferred: false,
                    gate: None,
                })
            }));
        model.method_enclosures.bindings.extend(bindings);
        model
            .method_enclosures
            .bindings
            .sort_by_key(|binding| binding.available_from);
        model.refresh_contextual_resolution();
        model
    }

    pub fn generated_file_names(&self) -> impl Iterator<Item = &str> {
        self.method_enclosures
            .bindings
            .iter()
            .filter(|binding| {
                self.scope(binding.scope).kind == ScopeKind::File
                    && binding.availability != Availability::Absent
            })
            .map(|binding| binding.name.as_str())
    }

    fn refresh_contextual_resolution(&mut self) {
        let mut scopes = HashSet::new();
        let mut method_scopes = HashSet::new();
        for index in 0..self.scopes.len() {
            let id = ScopeId::from_index(index);
            let mut current = Some(id);
            while let Some(scope) = current {
                if self.method_enclosures.functions.contains_key(&scope) {
                    scopes.insert(id);
                    method_scopes.insert(id);
                    break;
                }
                if self
                    .method_enclosures
                    .bindings
                    .iter()
                    .any(|binding| binding.scope == scope)
                {
                    scopes.insert(id);
                }
                current = self.scope(scope).parent;
            }
        }
        self.method_enclosures.scopes = scopes;
        self.method_enclosures.method_scopes = method_scopes;
        let writes = self.resolve_method_writes();
        self.method_enclosures.writes = writes.iter().map(|(range, _)| *range).collect();
        // NextMethod() synthesizes one read per formal at the same call span.
        // Cache only unambiguous spans; those synthetic reads resolve by name.
        let mut seen = HashSet::new();
        let repeated: HashSet<_> = self
            .idents()
            .iter()
            .filter(|ident| !seen.insert(ident.range))
            .map(|ident| ident.range)
            .collect();
        self.contextual_resolutions = self
            .idents()
            .iter()
            .filter(|ident| !repeated.contains(&ident.range))
            .filter_map(|ident| {
                self.contextual_resolution(&ident.name, ident.range, ident.scope, ident.deferred)
                    .map(|resolution| (ident.range, resolution))
            })
            .chain(writes)
            .collect();
        super::builder::refresh_resolution(self);
    }

    // The lexical builder cannot know the target of a nonlocal write. Replace
    // its provisional binding once an enclosing local or runtime environment
    // supplies that target, so a field update cannot define a file variable.
    fn resolve_method_writes(&mut self) -> Vec<(TextRange, IdentifierResolution)> {
        let mut removed = HashSet::new();
        let mut writes = Vec::new();
        let mut binding_ids = HashMap::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            let Some(origin) = binding.super_assignment_scope else {
                continue;
            };
            binding_ids.insert(binding.def_range, BindingId::from_index(index));
            // Keep the source operation after removing its provisional binding.
            // Another method context can add a different target for this write.
            self.method_enclosures
                .nonlocal_writes
                .entry(binding.def_range)
                .or_insert_with(|| NonlocalWrite {
                    name: binding_name_key(&binding.name),
                    origin,
                });
        }
        for (range, write) in &self.method_enclosures.nonlocal_writes {
            if !self.method_enclosures.scopes.contains(&write.origin) {
                continue;
            }
            let mut frame = write.origin;
            while !matches!(
                self.scope(frame).kind,
                ScopeKind::Function | ScopeKind::File
            ) {
                let Some(parent) = self.scope(frame).parent else {
                    break;
                };
                frame = parent;
            }
            let mut lookup = Lookup {
                model: self,
                name: write.name.clone(),
                range: *range,
                deferred: false,
                path: HashSet::new(),
                budget: 512,
                result: IdentifierResolution::default(),
                nonlocal_write: true,
            };
            lookup.enclosing(frame, false, true);
            if !lookup.result.is_source_only() || !lookup.result.lexical.is_empty() {
                if let Some(id) = binding_ids.get(range) {
                    removed.insert(*id);
                }
                writes.push((*range, lookup.result));
            }
        }
        if removed.is_empty() {
            return writes;
        }
        let mut remap = HashMap::new();
        let mut next = 0;
        for index in 0..self.bindings.len() {
            let old = BindingId::from_index(index);
            if !removed.contains(&old) {
                remap.insert(old, BindingId::from_index(next));
                next += 1;
            }
        }
        let mut index = 0;
        self.bindings.retain(|_| {
            let keep = !removed.contains(&BindingId::from_index(index));
            index += 1;
            keep
        });
        for ids in self.bindings_by_name.values_mut() {
            *ids = ids.iter().filter_map(|id| remap.get(id).copied()).collect();
        }
        for scope in &mut self.scopes {
            scope.bindings = scope
                .bindings
                .iter()
                .filter_map(|id| remap.get(id).copied())
                .collect();
        }
        for (_, resolution) in &mut writes {
            resolution.lexical = resolution
                .lexical
                .iter()
                .filter_map(|id| remap.get(id).copied())
                .collect();
        }
        writes
    }

    pub(crate) fn contextual_source_target(&self, range: TextRange) -> Option<BindingId> {
        self.source_resolution_target(self.contextual_resolutions.get(&range)?)
    }

    pub(crate) fn contextual_write_ranges(&self, binding: BindingId) -> Vec<TextRange> {
        self.method_enclosures
            .writes
            .iter()
            .filter(|range| {
                self.contextual_resolutions
                    .get(range)
                    .is_some_and(|resolution| resolution.lexical.contains(&binding))
            })
            .copied()
            .collect()
    }

    pub(crate) fn is_contextual_write(&self, range: TextRange) -> bool {
        self.method_enclosures.writes.contains(&range)
    }

    pub fn identifier_resolution(&self, ident: &super::IdentRef) -> IdentifierResolution {
        self.contextual_resolutions
            .get(&ident.range)
            .cloned()
            .unwrap_or_else(|| IdentifierResolution {
                lexical: self.resolve_local(ident).into_iter().collect(),
                external: self.resolve_local(ident).is_none(),
                ..Default::default()
            })
    }

    pub fn is_free_read(&self, ident: &super::IdentRef) -> bool {
        self.contextual_resolutions.get(&ident.range).map_or_else(
            || self.resolve_local(ident).is_none(),
            IdentifierResolution::is_free,
        )
    }

    pub fn may_read_externally(&self, ident: &super::IdentRef) -> bool {
        self.contextual_resolutions.get(&ident.range).map_or_else(
            || self.resolve_local(ident).is_none(),
            |resolution| resolution.external,
        )
    }

    /// Non-source identities cannot fall through to workspace-name navigation.
    pub fn blocks_source_navigation(&self, range: TextRange) -> bool {
        self.contextual_resolutions
            .get(&range)
            .is_some_and(|resolution| {
                !resolution.is_source_only()
                    || (!resolution.lexical.is_empty()
                        && self.source_resolution_target(resolution).is_none())
            })
    }

    pub(super) fn source_resolution_target(
        &self,
        resolution: &IdentifierResolution,
    ) -> Option<BindingId> {
        if !resolution.is_source_only() || resolution.external {
            return None;
        }
        let first = *resolution.lexical.first()?;
        let frame = self.frame_scope(self.binding(first).scope);
        resolution
            .lexical
            .iter()
            .all(|id| self.frame_scope(self.binding(*id).scope) == frame)
            .then_some(first)
    }

    /// References include every read sharing an injected environment identity.
    pub fn generated_read_ranges(&self, range: TextRange) -> Option<Vec<TextRange>> {
        let resolution = self.contextual_resolutions.get(&range)?;
        if resolution.generated.is_empty() {
            return None;
        }
        Some({
            let mut ranges: Vec<_> = self
                .contextual_resolutions
                .iter()
                .filter(|(_, other)| {
                    other
                        .generated
                        .iter()
                        .any(|id| resolution.generated.contains(id))
                })
                .map(|(range, _)| *range)
                .collect();
            ranges.sort_by_key(|range| range.start());
            ranges
        })
    }

    /// One source occurrence cannot be rewritten differently in method copies
    /// and in the original closure, so a mixed identity makes rename unsafe.
    pub fn binding_rename_is_safe(&self, id: BindingId) -> bool {
        let binding = self.binding(id);
        if self.method_enclosures.bindings.iter().any(|effect| {
            effect.scope == binding.scope && effect.name == binding_name_key(&binding.name)
        }) {
            return false;
        }
        self.variable_cohort(id).into_iter().all(|binding| {
            self.read_sites(binding)
                .all(|ident| !self.blocks_source_navigation(ident.range))
                && self
                    .contextual_write_ranges(binding)
                    .into_iter()
                    .all(|range| !self.blocks_source_navigation(range))
        })
    }

    pub(super) fn contextual_resolution(
        &self,
        name: &SmolStr,
        range: TextRange,
        scope: ScopeId,
        deferred: bool,
    ) -> Option<IdentifierResolution> {
        if !self.method_enclosures.scopes.contains(&scope) {
            return None;
        }
        let mut lookup = Lookup {
            model: self,
            name: binding_name_key(name),
            range,
            deferred,
            path: HashSet::new(),
            budget: 512,
            result: IdentifierResolution::default(),
            nonlocal_write: false,
        };
        lookup.scope(scope, true);
        lookup.result.lexical.sort_by_key(|id| id.0);
        lookup.result.lexical.dedup();
        lookup
            .result
            .generated
            .sort_by(|a, b| (a.origin, &a.name).cmp(&(b.origin, &b.name)));
        lookup.result.generated.dedup();
        Some(lookup.result)
    }
}

struct Lookup<'a> {
    model: &'a SemanticModel,
    name: SmolStr,
    range: TextRange,
    deferred: bool,
    path: HashSet<ScopeId>,
    budget: usize,
    result: IdentifierResolution,
    nonlocal_write: bool,
}

impl Lookup<'_> {
    fn scope(&mut self, id: ScopeId, in_frame: bool) {
        if self.budget == 0 || !self.path.insert(id) {
            self.result.uncertain = true;
            return;
        }
        self.budget -= 1;
        let scope = self.model.scope(id);
        let candidates: Vec<_> = self
            .model
            .scope_bindings_named(id, &self.name)
            .iter()
            .copied()
            .filter(|binding| {
                let binding = self.model.binding(*binding);
                binding.def_range != self.range
                    && !binding.super_assignment_scope.is_some_and(|origin| {
                        self.model.method_enclosures.method_scopes.contains(&origin)
                            || (self.nonlocal_write
                                && self.model.method_enclosures.scopes.contains(&origin))
                    })
                    && (!in_frame
                        || self.deferred
                        || binding.available_from <= self.range.start()
                        || binding
                            .loop_range
                            .is_some_and(|range| range.contains_range(self.range)))
            })
            .collect();
        // A promise or nested closure may run before an enclosing local write.
        // Keep the injected binding as an alternative until a preceding,
        // unconditional write (or a formal) establishes the local variable.
        let needs_parent = !candidates.iter().any(|id| {
            let binding = self.model.binding(*id);
            !binding.conditional
                && (binding.kind == super::BindingKind::Param
                    || (!in_frame && scope.kind == ScopeKind::File)
                    || (in_frame && !self.deferred)
                    || binding.available_from <= self.range.start())
        });
        let effect = self
            .model
            .method_enclosures
            .bindings
            .iter()
            .enumerate()
            .rev()
            .find(|(_, binding)| {
                binding.scope == id
                    && binding.name == self.name
                    && (!in_frame || self.deferred || binding.available_from <= self.range.start())
            });
        let mut needs_parent = needs_parent;
        if let Some((index, effect)) = effect {
            let later_local = candidates.iter().any(|id| {
                let binding = self.model.binding(*id);
                !binding.conditional && binding.available_from >= effect.available_from
            });
            if !later_local {
                self.result.generated.push(GeneratedBinding {
                    origin: GeneratedOrigin::Scope(index as u32),
                    name: self.name.clone(),
                });
                if effect.availability == Availability::Present {
                    needs_parent = false;
                } else {
                    self.result.uncertain = true;
                }
            }
        }
        self.result
            .lexical
            .extend(candidates.into_iter().filter(|binding| {
                effect.is_none_or(|(_, effect)| {
                    effect.availability != Availability::Present
                        || self.model.binding(*binding).available_from >= effect.available_from
                })
            }));
        if needs_parent {
            self.enclosing(id, in_frame, false);
        }
        self.path.remove(&id);
    }

    fn enclosing(&mut self, id: ScopeId, in_frame: bool, nonlocal_write: bool) {
        let scope = self.model.scope(id);
        if let Some(enclosures) = self.model.method_enclosures.functions.get(&id) {
            for enclosure in enclosures {
                if let Some(enclosure) = enclosure {
                    let environment =
                        &self.model.method_enclosures.environments[*enclosure as usize];
                    if nonlocal_write && environment.call_frame {
                        if let Some(parent) = scope.parent {
                            self.scope(parent, false);
                        } else {
                            self.result.external = true;
                        }
                        continue;
                    }
                    match environment.bindings.get(&self.name) {
                        Some(Availability::Present) => {
                            self.result.generated.push(GeneratedBinding {
                                origin: GeneratedOrigin::Method(*enclosure),
                                name: self.name.clone(),
                            });
                            continue;
                        }
                        Some(Availability::Unknown) => {
                            self.result.generated.push(GeneratedBinding {
                                origin: GeneratedOrigin::Method(*enclosure),
                                name: self.name.clone(),
                            });
                            self.result.uncertain = true;
                        }
                        _ => self.result.uncertain |= environment.unknown_members,
                    }
                    self.result.uncertain |= environment.parent_is_unknown;
                    if let Some(parent) = environment.parent {
                        self.scope(parent, false);
                    } else {
                        self.result.uncertain = true;
                    }
                } else if let Some(parent) = scope.parent {
                    self.scope(parent, false);
                } else {
                    self.result.external = true;
                }
            }
        } else if let Some(parent) = scope.parent {
            self.scope(
                parent,
                in_frame && !matches!(scope.kind, ScopeKind::Function | ScopeKind::File),
            );
        } else {
            self.result.external = true;
        }
    }
}
