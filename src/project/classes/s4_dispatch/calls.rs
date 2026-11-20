use super::execution::source_sites;
use super::*;
use crate::semantic::enclosure::Availability::{Absent, Present, Unknown};

/// Unlike registrations, selectMethod consumes the class vector positionally,
/// even when signature() or c() supplies names.
fn selection_signature(generic: &S4Generic, supplied: &[DispatchArgument]) -> S4MethodSignature {
    if generic.dispatches_on_dots() && !supplied.is_empty() {
        let mut query = s4_method_signature(generic, &[]);
        query.complete &= supplied.iter().all(|arg| {
            !matches!(
                arg.class,
                DispatchClass::Unknown | DispatchClass::Generator(_)
            )
        });
        query.arguments = supplied
            .iter()
            .map(|arg| DispatchArgument {
                argument: Some("...".into()),
                class: arg.class.clone(),
            })
            .collect();
        return query;
    }
    let positional: Vec<_> = supplied
        .iter()
        .map(|arg| DispatchArgument {
            argument: None,
            class: arg.class.clone(),
        })
        .collect();
    s4_method_signature(generic, &positional)
}

impl ClassIndex {
    /// Resolve the callable binding independently of whether the actual dispatch
    /// arguments are complete enough to select a method.
    pub(in crate::project::classes) fn s4_ordinary_call(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Option<(&S4Generic, Option<String>)> {
        if self
            .s4
            .generics
            .iter()
            .all(|generic| generic.declaration.is_none())
        {
            return None;
        }
        let extracted = extract(model, root, true);
        let (name, context, _, generic) =
            self.s4_call_generic(path, root, model, call, &extracted)?;
        let selected = self
            .s4_called_method(&context, generic, path, root, model, call, &extracted)
            .map(|arguments| super::super::editor::method_note(&name, &arguments));
        Some((generic, selected))
    }

    pub(crate) fn s4_external_implementations(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Vec<&S4Method> {
        let extracted = extract(model, root, true);
        let Some((_, context, loaded, generic)) =
            self.s4_call_generic(path, root, model, call, &extracted)
        else {
            return Vec::new();
        };
        if generic.declaration.is_some() || generic.object.is_some() {
            return Vec::new();
        }
        let active: Vec<_> = self
            .s4
            .methods
            .iter()
            .filter(|method| {
                method.unique_table() == Some(generic.table())
                    && self.s4_method_activity(&context, method, &loaded) == Present
                    && !method
                        .uncertainty
                        .contains(&OopUncertainty::InvalidArguments)
            })
            .collect();
        let removals: Vec<_> = self
            .s4
            .removals
            .iter()
            .filter(|removal| {
                self.s4_generic_activity(&context, &removal.source, &loaded) == Present
                    && removal.uncertainty.is_empty()
                    && matches!(removal.dispatch.as_slice(), [signature]
                        if signature.complete && signature.valid && signature.table() == generic.table())
            })
            .map(|removal| (removal, true))
            .collect();
        active
            .iter()
            .copied()
            .filter(|method| {
                method.dispatch.iter().any(|signature| {
                    signature.valid
                        && self.s4_registration_removed(&context, method, signature, &removals)
                            != Present
                        && !active.iter().any(|later| {
                            let S4MethodId::Registration(id) = &later.id else {
                                return false;
                            };
                            self.class_file_complete(&later.owner.path)
                                && later.uncertainty.iter().copied().all(|issue| {
                                    super::selection::registration_issue(issue).is_none()
                                })
                                && matches!(later.dispatch.as_slice(), [replacement]
                                    if replacement.valid && signature.same_classes(replacement))
                                && self.s4_installation_order(&context, method.id.source(), id)
                                    == Some(std::cmp::Ordering::Less)
                        })
                })
            })
            .collect()
    }

    fn s4_call_generic(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
        extracted: &Extracted,
    ) -> Option<(String, S4DispatchContext, BTreeSet<PathBuf>, &S4Generic)> {
        let base = call.base()?;
        let token = call.callee_token()?;
        let function = call.syntax().ancestors().find_map(FunctionExpr::cast);
        let owner = owner_at(call.syntax(), &extracted.owners);
        let lookup = function_ref(model, root, &base, &mut BTreeSet::new());
        let (package, name) = match &lookup {
            Some(FunctionRef::Qualified { package, name }) => (Some(package.clone()), name.clone()),
            _ => (None, unbacktick(token.text()).into()),
        };
        let mut reference = GenericRef {
            system: OopSystem::S4,
            owner: owner.clone(),
            name: ClassName::Known(name.clone()),
            package,
            by_binding: true,
            local: None,
            lookup: lookup.clone(),
        };
        if reference.package.is_none() {
            reference.local = s4_local_call_target(model, root, extracted, &token, &owner)?;
        }
        let site = S4CallSite {
            range: call.syntax().text_range(),
            kind: S4CallKind::StandardGeneric,
            callee: lookup.unwrap_or_else(|| FunctionRef::Unresolved(Vec::new())),
            reference: None,
            frames: Vec::new(),
            default_frames: Vec::new(),
            named_frame: None,
            function: function
                .as_ref()
                .map(|function| function.syntax().text_range()),
            dispatch: Vec::new(),
            dispatch_factory: None,
            inheritance: S4Inheritance::Default,
            uncertainty: BTreeSet::new(),
        };
        let mut context = S4DispatchContext::new(path);
        context.owner = owner;
        context.deferred = function.is_some();
        self.s4_package_loads_at(&mut context, &extracted.s4_package_load_sites, &site);
        if let Some(package) = &reference.package {
            self.s4_add_loaded_package(&mut context, Some(package));
        }
        context.execution =
            Some(self.s4_execution_at(path, &site, extracted, &source_sites(root, model)));
        let (loaded, _) = self.s4_dispatch_packages(&context, &context.loaded_packages);
        let candidates = self.s4_generic_candidates_at(&context, &reference, &loaded);
        let [generic] = candidates.as_slice() else {
            return None;
        };
        Some((name, context, loaded, generic))
    }

    #[allow(clippy::too_many_arguments)]
    fn s4_called_method(
        &self,
        context: &S4DispatchContext,
        generic: &S4Generic,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
        extracted: &Extracted,
    ) -> Option<Vec<DispatchArgument>> {
        if generic.skeleton != Some(S4GenericSkeleton::Standard)
            || generic.dispatches_on_dots()
            || generic.dispatch_arguments.is_empty()
            || call
                .syntax()
                .parent()
                .and_then(BinaryExpr::cast)
                .is_some_and(|binary| binary.op().is_some_and(|op| op.text() == "|>"))
        {
            return None;
        }
        let signature = generic.signature.as_ref()?;
        let formals = signature
            .parameters
            .iter()
            .map(|formal| formal.name.as_str())
            .collect::<Vec<_>>();
        let args = matched_args(call, &formals)?;
        let supplied = generic
            .dispatch_arguments
            .iter()
            .map(|name| {
                let ClassName::Known(name) = name else {
                    return None;
                };
                let class = if let Some(value) = argument(&args, name) {
                    if let Some(receiver) =
                        self.dispatch_receiver(path, root, model, &value, extracted)
                        && receiver.access == ClassAccess::Public
                        && matches!(
                            receiver.class.key.system,
                            ClassSystem::S4 | ClassSystem::RefClass
                        )
                        && let ClassName::Known(class) =
                            &self.declarations.get(&receiver.class)?.declaration.name
                    {
                        DispatchClass::Formal(class.clone())
                    } else {
                        let class = s7::dispatch::literal_class(&value)?;
                        DispatchClass::Formal(
                            if class == "double" { "numeric" } else { class }.into(),
                        )
                    }
                } else {
                    DispatchClass::Missing
                };
                Some(DispatchArgument {
                    argument: Some(name.clone()),
                    class,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let query = s4_method_signature(generic, &supplied);
        let resolved = self.s4_resolve_methods_with(context, &query, S4Inheritance::Default);
        if !resolved.complete() {
            return None;
        }
        let selected = resolved.selected?;
        resolved
            .candidates
            .into_iter()
            .find(|candidate| candidate.method == selected && candidate.certain)
            .map(|candidate| candidate.signature.arguments)
    }

    /// Link live operations to declaration identities, resolve literal
    /// selectMethod signatures, and recover declared callNextMethod targets.
    pub fn s4_dispatch_calls(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
    ) -> Vec<S4DispatchCall> {
        let extracted = extract(model, root, true);
        let sources = source_sites(root, model);
        extracted
            .s4_calls
            .iter()
            .map(|site| {
                let mut result = S4DispatchCall {
                    range: site.range,
                    kind: site.kind,
                    reference: site.reference.clone(),
                    generics: Vec::new(),
                    methods: Vec::new(),
                    exact_methods: Vec::new(),
                    resolutions: Vec::new(),
                    dispatch: Vec::new(),
                    use_inherited: match site.inheritance {
                        S4Inheritance::Explicit(value) => Some(value),
                        _ => None,
                    },
                    uncertainty: site.uncertainty.clone(),
                };
                if !self.s4_call_callee(path, &site.callee) {
                    result
                        .uncertainty
                        .insert(S4CallUncertainty::UnresolvedCallee);
                    return result;
                }
                if site.kind == S4CallKind::NextMethod {
                    for frame in &site.frames {
                        let source = OopId {
                            path: path.into(),
                            key: frame.clone(),
                        };
                        let id = if frame.kind == OopKind::Generic {
                            if !site.default_frames.contains(frame) {
                                continue;
                            }
                            S4MethodId::Default(source)
                        } else {
                            S4MethodId::Registration(source)
                        };
                        if let Some(method) = self.s4.methods.iter().find(|method| method.id == id)
                        {
                            result.methods.push(id);
                            result.generics.extend(method.generics.clone());
                            result.dispatch.extend(method.dispatch.clone());
                        }
                    }
                    if let Some(frame) = site.named_frame {
                        for method in &self.s4.methods {
                            let registration = match &method.id {
                                S4MethodId::Registration(source) | S4MethodId::Default(source) => {
                                    source
                                }
                            };
                            let matches = method.implementation.as_ref().is_some_and(|source| {
                                self.s4_named_frame(path, frame, registration, source)
                            });
                            if matches && !result.methods.contains(&method.id) {
                                result.methods.push(method.id.clone());
                                result.generics.extend(method.generics.clone());
                                result.dispatch.extend(method.dispatch.clone());
                            }
                        }
                    }
                    if result.methods.is_empty() {
                        result.uncertainty.insert(S4CallUncertainty::InvalidFrame);
                    }
                    if result.methods.len() > 1 {
                        result
                            .uncertainty
                            .insert(S4CallUncertainty::AmbiguousMethodFrame);
                    }
                    if result.dispatch.iter().any(|dispatch| !dispatch.complete) {
                        result
                            .uncertainty
                            .insert(S4CallUncertainty::DynamicSignature);
                    }
                    let mut group_frame = false;
                    for signature in &result.dispatch {
                        let mut context = S4DispatchContext::new(path);
                        context.continuation = true;
                        self.s4_package_loads_at(
                            &mut context,
                            &extracted.s4_package_load_sites,
                            site,
                        );
                        if let Some(method) = self.s4.methods.iter().find(|method| {
                            result.methods.contains(&method.id)
                                && method.dispatch.contains(signature)
                        }) {
                            context.path = method.owner.path.clone();
                            context.owner = method.owner.lexical.clone();
                            if let Some(package) = &method.generic.package {
                                self.s4_add_loaded_package(&mut context, Some(package));
                            }
                            if matches!(method.id, S4MethodId::Default(_)) {
                                context.excluded.insert(method.id.clone());
                            }
                        }
                        let (origins, group) = self.s4_next_signatures(&context, signature);
                        group_frame |= group;
                        result.resolutions.extend(
                            origins
                                .iter()
                                .map(|origin| self.s4_resolve_methods(&context, origin, true)),
                        );
                    }
                    if group_frame {
                        result
                            .uncertainty
                            .insert(S4CallUncertainty::AmbiguousGeneric);
                    }
                } else if let Some(reference) = &site.reference {
                    let mut context = S4DispatchContext::new(path);
                    context.owner = reference.owner.clone();
                    context.deferred = site.function.is_some();
                    if site.kind == S4CallKind::SelectMethod {
                        self.s4_package_loads_at(
                            &mut context,
                            &extracted.s4_package_load_sites,
                            site,
                        );
                        if let Some(package) = &reference.package {
                            self.s4_add_loaded_package(&mut context, Some(package));
                        }
                        context.execution =
                            Some(self.s4_execution_at(path, site, &extracted, &sources));
                    }
                    let generics = if site.kind == S4CallKind::SelectMethod {
                        let (loaded, _) =
                            self.s4_dispatch_packages(&context, &context.loaded_packages);
                        self.s4_generic_candidates_at(&context, reference, &loaded)
                    } else {
                        self.s4_generic_candidates(path, reference, true)
                    };
                    result.generics = generics.iter().map(|generic| generic.id.clone()).collect();
                    if site.kind == S4CallKind::SelectMethod {
                        result.dispatch = generics
                            .iter()
                            .map(|generic| selection_signature(generic, &site.dispatch))
                            .collect();
                        if site
                            .dispatch_factory
                            .as_ref()
                            .is_some_and(|callee| !self.s4_call_callee(path, callee))
                        {
                            for dispatch in &mut result.dispatch {
                                dispatch.complete = false;
                            }
                            result
                                .uncertainty
                                .insert(S4CallUncertainty::DynamicSignature);
                        }
                        if result.dispatch.iter().any(|dispatch| !dispatch.complete) {
                            result
                                .uncertainty
                                .insert(S4CallUncertainty::DynamicSignature);
                        }
                        if !result
                            .uncertainty
                            .contains(&S4CallUncertainty::CustomMethodTable)
                        {
                            result.exact_methods = self
                                .s4
                                .methods
                                .iter()
                                .filter(|method| {
                                    result.dispatch.iter().any(|query| {
                                        query.complete
                                            && method.dispatch.iter().any(|defined| {
                                                defined.complete
                                                    && defined.generic == query.generic
                                                    && defined.object == query.object
                                                    && defined.arguments == query.arguments
                                            })
                                    }) && !method
                                        .uncertainty
                                        .contains(&OopUncertainty::InvalidArguments)
                                })
                                .map(|method| method.id.clone())
                                .collect();
                            result.resolutions = result
                                .dispatch
                                .iter()
                                .map(|signature| {
                                    self.s4_resolve_methods_with(
                                        &context,
                                        signature,
                                        site.inheritance,
                                    )
                                })
                                .collect();
                            if matches!(site.inheritance, S4Inheritance::Dynamic) {
                                result
                                    .uncertainty
                                    .insert(S4CallUncertainty::DynamicInheritance);
                            }
                        }
                    } else {
                        let frame_generics: Vec<_> = generics
                            .iter()
                            .filter(|generic| {
                                generic.declaration.as_ref().is_some_and(|id| {
                                    id.path == path && site.frames.contains(&id.key)
                                        || site.named_frame.is_some_and(|frame| {
                                            self.generic(id)
                                                .and_then(|def| def.s4.as_ref())
                                                .and_then(|s4| s4.definition.as_ref())
                                                .is_some_and(|source| {
                                                    self.s4_named_frame(path, frame, id, source)
                                                })
                                        })
                                })
                            })
                            .collect();
                        if frame_generics.is_empty() {
                            result.uncertainty.insert(S4CallUncertainty::InvalidFrame);
                        }
                    }
                    if generics.first().is_some_and(|first| {
                        generics.iter().any(|generic| {
                            generic.id != first.id
                                || generic.signature != first.signature
                                || generic.dispatch_arguments != first.dispatch_arguments
                                || generic.groups != first.groups
                                || generic.default != first.default
                                || generic.uncertainty != first.uncertainty
                        })
                    }) {
                        result
                            .uncertainty
                            .insert(S4CallUncertainty::AmbiguousGeneric);
                    }
                }
                result.generics.sort();
                result.generics.dedup();
                if result.generics.is_empty() {
                    result
                        .uncertainty
                        .insert(S4CallUncertainty::UnresolvedGeneric);
                }
                result
            })
            .collect()
    }

    pub(super) fn s4_named_frame(
        &self,
        path: &Path,
        frame: usize,
        registration: &OopId,
        source: &MethodImplementationSource,
    ) -> bool {
        self.s4_callable_at(&registration.path, source, Some(&registration.key))
            .is_some_and(|callable| {
                callable.path == path
                    && self.file(path).is_some_and(|file| {
                        file.facts.oop.callables[callable.binding].origin == frame
                    })
            })
    }

    /// An installed group method may be entered from any of its member tables.
    /// Replay the continuation against each known origin with the same classes.
    fn s4_next_signatures(
        &self,
        context: &S4DispatchContext,
        signature: &S4MethodSignature,
    ) -> (Vec<S4MethodSignature>, bool) {
        let mut origins = vec![signature.clone()];
        if !self.s4.generics.iter().any(|generic| {
            generic.group && generic.id == signature.generic && generic.object == signature.object
        }) {
            return (origins, false);
        }
        let (loaded, _) = self.s4_dispatch_packages(context, &context.loaded_packages);
        let group = signature.table();
        let mut seen = BTreeSet::from([group]);
        for generic in &self.s4.generics {
            if generic.groups.is_empty()
                || generic
                    .declaration
                    .as_ref()
                    .is_some_and(|id| self.s4_generic_activity(context, id, &loaded) == Absent)
                || seen.contains(&generic.table())
            {
                continue;
            }
            let (groups, _) = self.s4_group_depths(context, &[generic], &loaded);
            if groups.contains_key(&group) {
                let origin = selection_signature(generic, &signature.arguments);
                if origin.valid {
                    seen.insert(generic.table());
                    origins.push(origin);
                }
            }
        }
        (origins, true)
    }

    fn s4_package_loads_at(
        &self,
        context: &mut S4DispatchContext,
        loads: &[S4PackageLoadSite],
        query: &S4CallSite,
    ) {
        for load in loads {
            let precedes = if load.function == query.function {
                load.range.end() <= query.range.start()
            } else {
                load.function.is_none()
            };
            if !precedes {
                continue;
            }
            match self.s4_loader_activity(&context.path, &load.load) {
                Absent => {}
                Present => self.s4_record_load(context, &load.load),
                Unknown => context.runtime_registration = true,
            }
        }
    }

    fn s4_call_callee(&self, path: &Path, reference: &FunctionRef) -> bool {
        if let FunctionRef::Qualified { package, name } = reference {
            return expected_package(name) == Some(package.as_str());
        }
        self.callee_identity(path, reference).is_some()
    }
}

/// Match the local read to the last visible installation event. Lexical
/// ordinary writes can retire a generic even when registrations remain.
fn s4_local_call_target(
    model: &SemanticModel,
    root: &SyntaxNode,
    extracted: &Extracted,
    token: &SyntaxToken,
    owner: &ClassOwner,
) -> Option<Option<OopKey>> {
    let name = unbacktick(token.text());
    let offset = token.text_range().start();
    let binding = read_binding(model, token).map(|id| model.binding(id));
    for (event, range) in extracted
        .facts
        .oop
        .events
        .iter()
        .zip(&extracted.oop_event_ranges)
        .rev()
    {
        if range.end() > offset {
            continue;
        }
        match event {
            OopEvent::Generic {
                name: ClassName::Known(binding_name),
                target,
            } if name == binding_name && owner.0.starts_with(&target.owner.0) => {
                if binding.is_none_or(|binding| {
                    binding.def_range.start() < range.start()
                        && matches!(binding.kind, BindingKind::Local | BindingKind::Implicit)
                        && owner_at(
                            &root
                                .covering_element(binding.def_range)
                                .parent()
                                .unwrap_or_else(|| root.clone()),
                            &extracted.owners,
                        ) == target.owner
                }) {
                    return Some(Some(target.clone()));
                }
            }
            OopEvent::InvalidateBindings if owner.0.is_empty() => return None,
            _ => {}
        }
    }
    if let Some(binding) = binding {
        if binding.available_from > offset {
            return None;
        }
        let mut aliases = extracted.facts.oop.generic_bindings.iter().filter(|alias| {
            alias.name == name
                && alias.target.system == OopSystem::S4
                && owner.0.starts_with(&alias.owner.0)
                // Registration accepts a name string, but only a callable
                // alias can serve as the target of an ordinary call.
                && extracted.oop_sites.iter().any(|source| {
                    let site = &source.site;
                    site.range == binding.def_range
                        && site.role == OopSiteRole::Alias
                        && matches!(&site.target, OopTarget::Declaration(key) if key == &alias.target)
                })
                && extracted
                    .facts
                    .oop
                    .events
                    .iter()
                    .zip(&extracted.oop_event_ranges)
                    .any(|(event, range)| {
                        matches!(event, OopEvent::Generic { target, .. } if target == &alias.target)
                            && range.end() <= binding.def_range.start()
                    })
        });
        let alias = aliases.next()?;
        return aliases.next().is_none().then(|| Some(alias.target.clone()));
    }
    Some(None)
}
