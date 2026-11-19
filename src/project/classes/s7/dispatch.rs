use super::*;
use crate::semantic::enclosure::Availability::{self, Absent, Present, Unknown};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S7GenericTarget {
    S7(OopId),
    S3(S3GenericId),
    S4(S4GenericId),
    External { package: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7Registration {
    pub source: OopId,
    pub owner_package: Option<String>,
    pub targets: Vec<S7GenericTarget>,
    pub implementation: Option<MethodImplementationSource>,
    pub delayed: bool,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7Dispatch {
    pub generic: OopId,
    pub candidates: Vec<OopId>,
    pub selected: Option<OopId>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SignatureClass {
    Tag(String),
    Any,
}

impl ClassIndex {
    pub fn s7_registration(&self, id: &OopId) -> Option<S7Registration> {
        let file = self.file(&id.path)?;
        let registration = file.facts.oop.s7.methods.get(&id.key)?;
        let method = self.method(id)?;
        self.s7_helper(&id.path, method.registration.as_ref()?, "S7")?;
        let reference = method.generic.as_ref()?;
        let deferred = !reference.owner.0.is_empty();
        let mut targets: Vec<_> = if reference.system == OopSystem::S7 {
            self.generic_candidates(&id.path, reference, deferred)
                .into_iter()
                .map(S7GenericTarget::S7)
                .collect()
        } else {
            Vec::new()
        };
        if targets.is_empty() {
            targets = self.s7_bridge_targets(id, method, registration);
        }
        if targets.is_empty()
            && let Some(package) = &reference.package
            && let ClassName::Known(name) = &reference.name
        {
            targets.push(S7GenericTarget::External {
                package: package.clone(),
                name: name.clone(),
            });
        }
        let owner_package = file
            .package_root
            .as_ref()
            .and_then(|root| self.workspace_package_name(root))
            .map(str::to_string);
        let external_signature = method.dispatch.iter().any(|arg| match &arg.class {
            DispatchClass::S7(spec) => spec_has_external(spec),
            _ => false,
        });
        let foreign = owner_package.is_some()
            && targets.iter().any(|target| match target {
                S7GenericTarget::S7(target) => {
                    self.file(&target.path)
                        .and_then(|file| file.package_root.as_ref())
                        != file.package_root.as_ref()
                }
                S7GenericTarget::S3(S3GenericId::Declaration(target))
                | S7GenericTarget::S4(S4GenericId::Declaration(target)) => {
                    self.file(&target.path)
                        .and_then(|file| file.package_root.as_ref())
                        != file.package_root.as_ref()
                }
                S7GenericTarget::S3(S3GenericId::Known { package, .. })
                | S7GenericTarget::S4(S4GenericId::Named { package, .. })
                | S7GenericTarget::External { package, .. } => {
                    Some(package) != owner_package.as_ref()
                }
            });
        let delayed = registration.external.is_some() || external_signature || foreign;
        let implementation = file
            .facts
            .oop
            .method_implementations
            .iter()
            .find(|fact| fact.method == id.key)
            .map(|fact| fact.source.clone());
        let complete = targets.len() == 1
            && !matches!(targets[0], S7GenericTarget::External { .. })
            && (implementation.is_some() || registration.removal)
            && self.s7_signature_list_is_base(&id.path, method)
            && method.uncertainty.iter().all(|issue| {
                matches!(
                    issue,
                    OopUncertainty::UnresolvedCallee | OopUncertainty::UnresolvedImplementation
                )
            })
            && registration.external.as_ref().is_none_or(|external| {
                self.s7_helper(&id.path, &external.factory, "S7").is_some()
                    && self.s7_external_version(&external.package, external.version.as_deref())
                    && external
                        .dispatch_factory
                        .as_ref()
                        .is_none_or(|helper| self.s7_helper(&id.path, helper, "base").is_some())
            });
        Some(S7Registration {
            source: id.clone(),
            owner_package,
            targets,
            implementation,
            delayed,
            complete,
        })
    }

    fn s7_signature_list_is_base(&self, path: &Path, method: &MethodDef) -> bool {
        method
            .dispatch_factory
            .as_ref()
            .is_none_or(|factory| self.s7_helper(path, factory, "base").as_deref() == Some("list"))
    }

    fn s7_bridge_targets(
        &self,
        id: &OopId,
        method: &MethodDef,
        registration: &S7MethodDef,
    ) -> Vec<S7GenericTarget> {
        let Some(reference) = method.generic.as_ref() else {
            return Vec::new();
        };
        if method
            .uncertainty
            .contains(&OopUncertainty::UnresolvedAlias)
        {
            return Vec::new();
        }
        let mut reference = reference.clone();
        if let Some(FunctionRef::Qualified { package, name }) = &registration.generic_origin {
            reference.package = Some(package.clone());
            reference.name = ClassName::Known(name.clone());
        }
        let known_s7_class = method.dispatch.iter().any(|argument| {
            let spec = match &argument.class {
                DispatchClass::Generator(reference) => S7ClassSpec::Generator(reference.clone()),
                DispatchClass::S7(spec) => spec.clone(),
                _ => return false,
            };
            self.s7_spec_class(&id.path, &id.key.owner, &spec)
                .is_some_and(|class| class.key.system == ClassSystem::S7)
        });
        reference.system = OopSystem::S4;
        let s4 = self
            .s4_generic_candidates(&id.path, &reference, !id.key.owner.0.is_empty())
            .into_iter()
            .filter(|generic| {
                generic.object.is_some() || reference.package.as_deref() == Some("methods")
            })
            .map(|generic| S7GenericTarget::S4(generic.id.clone()))
            .collect::<Vec<_>>();
        if !s4.is_empty() {
            return s4;
        }
        if self.s7_version(&id.path) == S7Version::Release
            && (method.dispatch.len() != 1 || !known_s7_class)
        {
            return Vec::new();
        }
        reference.system = OopSystem::S3;
        let candidates = self.generic_candidates(&id.path, &reference, !id.key.owner.0.is_empty());
        if !candidates.is_empty() {
            return candidates
                .into_iter()
                .map(|id| S7GenericTarget::S3(S3GenericId::Declaration(id)))
                .collect();
        }
        let ClassName::Known(name) = &reference.name else {
            return Vec::new();
        };
        let package = reference
            .package
            .as_deref()
            .or_else(|| {
                (!self
                    .definition_files(&id.path, name, !id.key.owner.0.is_empty())
                    .is_empty())
                .then_some("")
            })
            .unwrap_or("base");
        self.s3
            .generics
            .iter()
            .filter_map(|generic| match &generic.id {
                S3GenericId::Known {
                    package: origin,
                    name: known,
                } if origin == package && known == name => {
                    Some(S7GenericTarget::S3(generic.id.clone()))
                }
                _ => None,
            })
            .collect()
    }

    pub(in crate::project::classes) fn s7_live_generic(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        extracted: &Extracted,
        value: &SyntaxElement,
    ) -> Option<OopId> {
        let namespace = value
            .as_node()
            .cloned()
            .and_then(BinaryExpr::cast)
            .and_then(|binary| binary.namespace_access());
        let (name, package, local) = if let Some(namespace) = namespace {
            (
                namespace.name.to_string(),
                Some(namespace.package.to_string()),
                None,
            )
        } else {
            let token = value.as_token()?;
            let name = resolve::namespace_name(value.clone())?;
            let local = if let Some(binding) = read_binding(model, token) {
                let declaration = model.binding(binding);
                let value = stable_value(model, root, binding)?;
                if declaration.available_from > token.text_range().start()
                    && model.innermost_scope_at(token.text_range().start()) == declaration.scope
                {
                    return None;
                }
                if let Some(namespace) = value
                    .as_node()
                    .cloned()
                    .and_then(BinaryExpr::cast)
                    .and_then(|binary| binary.namespace_access())
                {
                    let reference = GenericRef {
                        system: OopSystem::S7,
                        owner: ClassOwner::default(),
                        name: ClassName::Known(namespace.name.to_string()),
                        package: Some(namespace.package.to_string()),
                        by_binding: true,
                        local: None,
                        lookup: None,
                    };
                    let candidates = self.generic_candidates(path, &reference, false);
                    return match candidates.as_slice() {
                        [candidate] => Some(candidate.clone()),
                        _ => None,
                    };
                }
                let node = root.covering_element(declaration.def_range).parent()?;
                let owner = owner_at(&node, &extracted.owners);
                Some(
                    extracted
                        .facts
                        .oop
                        .generic_bindings
                        .iter()
                        .find(|binding| {
                            binding.name == name
                                && binding.owner == owner
                                && binding.target.system == OopSystem::S7
                        })?
                        .target
                        .clone(),
                )
            } else {
                None
            };
            (name, None, local)
        };
        let node = value.parent()?;
        let reference = GenericRef {
            system: OopSystem::S7,
            owner: owner_at(&node, &extracted.owners),
            name: ClassName::Known(name),
            package,
            by_binding: true,
            local,
            lookup: None,
        };
        let in_function = node
            .ancestors()
            .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
        match self
            .generic_candidates(path, &reference, in_function)
            .as_slice()
        {
            [candidate] => Some(candidate.clone()),
            _ => None,
        }
    }

    pub fn s7_dispatch_at(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Option<S7Dispatch> {
        let extracted = extract(model, root, true);
        let dispatcher = call_function_ref(model, root, call)
            .and_then(|reference| self.s7_helper(path, &reference, "S7"))
            .as_deref()
            == Some("S7_dispatch");
        let generic_id = if dispatcher {
            if call.arg_list()?.args().next().is_some() {
                return None;
            }
            let owner = call.syntax().ancestors().find_map(FunctionExpr::cast)?;
            let sites: Vec<_> = extracted
                .method_implementations
                .iter()
                .filter(|site| {
                    site.method.system == OopSystem::S7
                        && site.method.kind == OopKind::Generic
                        && site.function_range == owner.syntax().text_range()
                })
                .collect();
            let [site] = sites.as_slice() else {
                return None;
            };
            OopId {
                path: path.to_path_buf(),
                key: site.method.clone(),
            }
        } else {
            self.s7_live_generic(path, root, model, &extracted, &call.base()?)?
        };
        let generic = self.generic(&generic_id)?;
        if !self.generic_confirmed(&generic_id.path, generic) {
            return None;
        }
        let shape = self
            .file(&generic_id.path)?
            .facts
            .oop
            .s7
            .generics
            .get(&generic_id.key)?;
        let proven_dispatch = match shape {
            S7GenericDef::Generated => true,
            S7GenericDef::Dispatch(helper) => {
                self.s7_helper(&generic_id.path, helper, "S7").is_some()
            }
            S7GenericDef::Custom => false,
        };
        let dispatch_count = generic.dispatch_arguments.len();
        let parsed_formals = generic.signature.as_ref().map(|signature| {
            signature
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>()
        });
        let args = (!dispatcher)
            .then(|| matched_args(call, parsed_formals.as_deref()?))
            .flatten();
        let mut eager_packages = BTreeSet::new();
        eager_packages.extend(call.base().and_then(namespace_package));
        let evidence: Vec<_> = generic
            .dispatch_arguments
            .iter()
            .map(|formal| {
                let ClassName::Known(formal) = formal else {
                    return None;
                };
                let args = args.as_ref()?;
                if let Some(value) = argument(args, formal) {
                    if proven_dispatch {
                        eager_packages.extend(
                            value
                                .as_node()
                                .cloned()
                                .and_then(CallExpr::cast)
                                .and_then(|call| call.base())
                                .and_then(namespace_package),
                        );
                    }
                    return self.s7_value_dispatch(path, root, model, &extracted, &value, 0);
                }
                let default = generic
                    .signature
                    .as_ref()?
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == *formal)?
                    .default
                    .as_deref();
                match default {
                    None => Some(vec!["MISSING".into()]),
                    Some(default) => {
                        let parsed = crate::parser::parse(default);
                        let value = parsed
                            .cst
                            .children_with_tokens()
                            .find(|element| !crate::ast::kinds::is_trivia(element.kind()))?;
                        literal_class(&value).map(|class| vec![class.into()])
                    }
                }
            })
            .collect();
        let in_function = call
            .syntax()
            .ancestors()
            .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
        let boundary = extracted
            .oop_event_ranges
            .iter()
            .filter(|range| range.end() <= call.syntax().text_range().start())
            .count();
        let execution = self.execution(path);
        let mut complete = proven_dispatch
            && !dispatcher
            && !in_function
            && execution.complete
            && !generic
                .uncertainty
                .contains(&OopUncertainty::DynamicSignature)
            && !evidence.is_empty()
            && evidence.iter().all(Option::is_some);
        let mut candidates = Vec::new();
        let mut package_activity = BTreeMap::new();
        let mut definite = BTreeMap::<Vec<usize>, (OopId, bool, Option<usize>)>::new();
        for file in self.oop_files() {
            for (key, registration) in &file.facts.oop.s7.methods {
                if !key.owner.0.is_empty() && (!in_function || file.path != path) {
                    continue;
                }
                let source = OopId {
                    path: file.path.clone(),
                    key: key.clone(),
                };
                let Some(method) = self.method(&source) else {
                    continue;
                };
                let Some(resolved) = self.s7_registration(&source) else {
                    continue;
                };
                if !resolved
                    .targets
                    .contains(&S7GenericTarget::S7(generic_id.clone()))
                {
                    if resolved.targets.is_empty()
                        && (file.path == path || self.sees_file(path, &file.path))
                    {
                        complete = false;
                    }
                    continue;
                }
                let Some(event) =
                    file.facts.oop.events.iter().position(
                        |event| matches!(event, OopEvent::S7Method(target) if target == key),
                    )
                else {
                    continue;
                };
                let order = execution
                    .events
                    .iter()
                    .enumerate()
                    .take_while(|(_, (file, event))| {
                        in_function || file != path || *event < boundary
                    })
                    .filter(|(_, (origin, index))| origin == &file.path && *index == event)
                    .map(|(index, _)| index)
                    .last();
                let package = file.package_root.as_ref().map_or(Absent, |package| {
                    *package_activity.entry(package).or_insert_with(|| {
                        self.s7_runtime_package_activity(
                            path,
                            package,
                            call,
                            &extracted,
                            &eager_packages,
                            (root, model),
                        )
                    })
                });
                if order.is_none() && package == Absent {
                    continue;
                }
                if !in_function && file.path == path && event >= boundary {
                    continue;
                }
                if !self.s7_signature_list_is_base(&file.path, method) {
                    if !registration.removal {
                        candidates.push(source);
                    }
                    complete = false;
                    continue;
                }
                if method.dispatch.len() != dispatch_count
                    || (dispatch_count > 1 && !registration.list_signature)
                    || (dispatch_count == 1
                        && registration.list_signature
                        && self.s7_version(&file.path) == S7Version::Release)
                {
                    continue;
                }
                let ranks = self.s7_signature_ranks(&source, method, &evidence);
                let Some(ranks) = ranks else {
                    continue;
                };
                let hooks_present = !resolved.delayed
                    || file
                        .package_root
                        .as_ref()
                        .is_none_or(|package| self.s7_has_hook(package, S7HookKind::Load));
                let external_shape = registration.external.as_ref().is_none_or(|external| {
                    generic
                        .dispatch_arguments
                        .iter()
                        .map(|name| match name {
                            ClassName::Known(name) => Some(name),
                            _ => None,
                        })
                        .eq(external.dispatch_arguments.iter().map(Some))
                });
                let known = resolved.complete
                    && hooks_present
                    && external_shape
                    && ranks.is_some()
                    && (order.is_some() || package == Present)
                    && (registration.removal
                        || self
                            .s7_implementation_signature(&source, &resolved)
                            .is_some_and(|method| {
                                method_matches_generic(
                                    generic,
                                    &method,
                                    self.s7_version(&file.path),
                                )
                            }));
                if !registration.removal {
                    candidates.push(source.clone());
                }
                if !known {
                    complete = false;
                    continue;
                }
                for rank in ranks.expect("known ranks") {
                    match definite.get_mut(&rank) {
                        Some(slot) => match (slot.2, order) {
                            (Some(old), Some(new)) if new > old => {
                                *slot = (source.clone(), registration.removal, order)
                            }
                            (Some(_), Some(_)) => {}
                            _ => complete = false,
                        },
                        None => {
                            definite.insert(rank, (source.clone(), registration.removal, order));
                        }
                    }
                }
            }
        }
        let selected = complete
            .then(|| {
                definite
                    .into_values()
                    .find(|(_, removal, _)| !removal)
                    .map(|(id, _, _)| id)
            })
            .flatten();
        candidates.sort();
        candidates.dedup();
        Some(S7Dispatch {
            generic: generic_id,
            candidates,
            selected,
            complete,
        })
    }

    fn s7_signature_ranks(
        &self,
        id: &OopId,
        method: &MethodDef,
        evidence: &[Option<Vec<String>>],
    ) -> Option<Option<Vec<Vec<usize>>>> {
        let mut complete = true;
        let mut ranks = vec![Vec::new()];
        for (arg, classes) in method.dispatch.iter().zip(evidence) {
            let Some(classes) = classes else {
                complete = false;
                continue;
            };
            let Some(registered) = self.s7_signature_classes(id, &arg.class) else {
                complete = false;
                continue;
            };
            let choices: BTreeSet<_> = registered
                .iter()
                .filter_map(|class| match class {
                    SignatureClass::Any => Some(usize::MAX),
                    SignatureClass::Tag(class) => {
                        classes.iter().position(|candidate| candidate == class)
                    }
                })
                .collect();
            if choices.is_empty() {
                return None;
            }
            if ranks.len().saturating_mul(choices.len()) > 4096 {
                complete = false;
                continue;
            }
            ranks = ranks
                .into_iter()
                .flat_map(|rank| {
                    choices.iter().map(move |choice| {
                        let mut rank = rank.clone();
                        rank.push(*choice);
                        rank
                    })
                })
                .collect();
        }
        Some(complete.then_some(ranks))
    }

    pub(super) fn s7_signature_classes(
        &self,
        id: &OopId,
        class: &DispatchClass,
    ) -> Option<Vec<SignatureClass>> {
        match class {
            DispatchClass::Generator(reference) => self.s7_spec_tags(
                &id.path,
                &id.key.owner,
                &S7ClassSpec::Generator(reference.clone()),
            ),
            DispatchClass::S7(spec) => self.s7_spec_tags(&id.path, &id.key.owner, spec),
            DispatchClass::Unknown => None,
            _ => None,
        }
    }

    pub(super) fn s7_spec_tags(
        &self,
        path: &Path,
        owner: &ClassOwner,
        spec: &S7ClassSpec,
    ) -> Option<Vec<SignatureClass>> {
        match spec {
            S7ClassSpec::Builtin(reference) => {
                let name = self.s7_helper(path, reference, "S7")?;
                Some(
                    builtin_dispatch_tags(&name)
                        .into_iter()
                        .map(|tag| {
                            if tag == "ANY" {
                                SignatureClass::Any
                            } else {
                                SignatureClass::Tag(tag)
                            }
                        })
                        .collect(),
                )
            }
            S7ClassSpec::S3 {
                factory,
                vector,
                classes,
                ..
            } if self.s7_helper(path, factory, "S7").is_some()
                && vector
                    .as_ref()
                    .is_none_or(|factory| self.s7_helper(path, factory, "base").is_some()) =>
            {
                Some(vec![SignatureClass::Tag(classes.first()?.clone())])
            }
            S7ClassSpec::Union { factory, classes } if self.s7_union_origin(path, factory) => {
                if classes.is_empty() {
                    return None;
                }
                let nested = classes
                    .iter()
                    .map(|class| self.s7_spec_tags(path, owner, class))
                    .collect::<Option<Vec<_>>>()?;
                Some(nested.into_iter().flatten().collect())
            }
            S7ClassSpec::Null => Some(vec![SignatureClass::Tag("NULL".into())]),
            _ => {
                let id = self.s7_spec_class(path, owner, spec)?;
                Some(vec![SignatureClass::Tag(self.s7_dispatch_tag(&id)?)])
            }
        }
    }

    fn s7_dispatch_tag(&self, class: &ClassId) -> Option<String> {
        if class.key.system == ClassSystem::S7 {
            return self.s7_class_tag(class);
        }
        let declaration = self.declarations.get(class)?;
        let ClassName::Known(name) = &declaration.declaration.name else {
            return None;
        };
        let s4 = declaration.declaration.s4.as_ref()?;
        if s4.definition == S4Definition::OldClass {
            return Some(name.clone());
        }
        let package = declaration
            .package_root
            .as_ref()
            .and_then(|root| self.workspace_package_name(root));
        Some(package.map_or_else(
            || format!("S4/{name}"),
            |package| format!("S4/{package}::{name}"),
        ))
    }

    pub(in crate::project::classes) fn s7_lineage(
        &self,
        class: &ClassId,
        visiting: &mut BTreeSet<ClassId>,
    ) -> Option<Vec<String>> {
        if visiting.len() >= 64 || !visiting.insert(class.clone()) {
            return None;
        }
        let mut classes = vec![self.s7_dispatch_tag(class)?];
        if let Some(definition) = self.s7_class(class) {
            match &definition.parent {
                None => classes.push("S7_object".into()),
                Some(S7ClassSpec::Builtin(reference)) => {
                    let name = self.s7_helper(&class.path, reference, "S7")?;
                    classes.extend(builtin_lineage(&name)?);
                }
                Some(S7ClassSpec::S3 {
                    factory,
                    classes: parent,
                    vector,
                    ..
                }) => {
                    self.s7_helper(&class.path, factory, "S7")?;
                    if let Some(vector) = vector {
                        self.s7_helper(&class.path, vector, "base")?;
                    }
                    classes.extend(parent.iter().cloned());
                    classes.push("S7_object".into());
                }
                Some(spec) => {
                    let parent = self.s7_spec_class(&class.path, &class.key.owner, spec)?;
                    let formal =
                        matches!(parent.key.system, ClassSystem::S4 | ClassSystem::RefClass);
                    if formal && self.s7_version(&class.path) == S7Version::Release {
                        return None;
                    }
                    classes.extend(self.s7_lineage(&parent, visiting)?);
                    if formal {
                        classes.push("S7_object".into());
                    }
                }
            }
        } else {
            let supers = self.supertypes.get(class);
            let declaration = self.declarations.get(class)?;
            if declaration.declaration.parents.len() > 1
                || !declaration.declaration.parents.is_empty()
                    && supers.is_none_or(BTreeSet::is_empty)
            {
                return None;
            }
            for parent in supers.into_iter().flatten() {
                classes.extend(self.s7_lineage(parent, visiting)?);
            }
        }
        visiting.remove(class);
        Some(classes)
    }

    fn s7_value_dispatch(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        extracted: &Extracted,
        value: &SyntaxElement,
        depth: usize,
    ) -> Option<Vec<String>> {
        if depth >= 32 {
            return None;
        }
        if let Some(class) = literal_class(value) {
            return Some(vec![class.into()]);
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast)
            && call_function_ref(model, root, &call)
                .and_then(|reference| self.s7_helper(path, &reference, "S7"))
                .as_deref()
                == Some("super")
        {
            let args = matched_args(&call, &["from", "to"])?;
            let from = argument(&args, "from")?;
            if self.s7_version(path) == S7Version::Release
                && !self
                    .class_receiver(path, root, model, &from)
                    .is_some_and(|receiver| {
                        receiver.class.key.system == ClassSystem::S7
                            && receiver.access != ClassAccess::Generator
                    })
            {
                return None;
            }
            let classes = self.s7_value_dispatch(path, root, model, extracted, &from, depth + 1)?;
            let to = argument(&args, "to")?;
            let spec = oop::live_s7_spec(model, root, extracted, &to);
            let owner = owner_at(call.syntax(), &extracted.owners);
            return self.s7_super_dispatch(path, &owner, &spec, &classes);
        }
        if let Some(receiver) = self.dispatch_receiver(path, root, model, value, extracted)
            && receiver.access != ClassAccess::Generator
            && matches!(
                receiver.class.key.system,
                ClassSystem::S7 | ClassSystem::S4 | ClassSystem::RefClass
            )
        {
            return self.s7_lineage(&receiver.class, &mut BTreeSet::new());
        }
        if let Some(token) = value.as_token()
            && let Some(binding) = read_binding(model, token)
            && model.binding(binding).available_from <= token.text_range().start()
        {
            let value = stable_value(model, root, binding)?;
            return self.s7_value_dispatch(path, root, model, extracted, &value, depth + 1);
        }
        match self.s3_class_evidence(path, root, model, value) {
            S3ClassEvidence::Explicit(class) => Some(class),
            _ => None,
        }
    }

    fn s7_super_dispatch(
        &self,
        path: &Path,
        owner: &ClassOwner,
        target: &S7ClassSpec,
        from: &[String],
    ) -> Option<Vec<String>> {
        let dispatch = match target {
            S7ClassSpec::S3 { classes, .. } => {
                // Casts check the full S3 class vector. Confirm its provenance
                // with the same check used during method registration.
                self.s7_spec_tags(path, owner, target)?;
                if from.iter().any(|tag| tag.starts_with("S4/")) {
                    return None;
                }
                let inherits = match self.s7_version(path) {
                    S7Version::Release => classes.iter().all(|tag| from.contains(tag)),
                    S7Version::Development => {
                        let from = if from.last().is_some_and(|tag| tag == "S7_object") {
                            &from[..from.len() - 1]
                        } else {
                            from
                        };
                        from.windows(classes.len()).any(|window| window == classes)
                    }
                };
                return inherits.then(|| {
                    let mut dispatch = classes.clone();
                    dispatch.push("S7_object".into());
                    dispatch
                });
            }
            S7ClassSpec::Builtin(reference) => {
                let name = self.s7_helper(path, reference, "S7")?;
                builtin_lineage(&name)?
            }
            S7ClassSpec::Generator(_) | S7ClassSpec::S4 { .. } => {
                let class = self.s7_spec_class(path, owner, target)?;
                self.s7_lineage(&class, &mut BTreeSet::new())?
            }
            _ => return None,
        };
        from.contains(dispatch.first()?).then_some(dispatch)
    }

    fn s7_runtime_package_activity(
        &self,
        path: &Path,
        package_root: &Path,
        call: &CallExpr,
        extracted: &Extracted,
        eager_packages: &BTreeSet<String>,
        live: (&SyntaxNode, &SemanticModel),
    ) -> Availability {
        let Some(package) = self.workspace_package_name(package_root) else {
            return Unknown;
        };
        let loads_package = |target: &str, attaches: bool| {
            target == package
                || self.s4_workspace_root(target).is_some_and(|root| {
                    let attachments = if attaches {
                        BTreeSet::from([root.clone()])
                    } else {
                        BTreeSet::new()
                    };
                    let (loaded, _) =
                        self.namespace_load_closure(Some(root), &BTreeSet::new(), &attachments);
                    loaded.contains(package_root)
                })
        };
        if eager_packages
            .iter()
            .any(|target| loads_package(target, false))
        {
            return Present;
        }
        let owner = self.file(path).and_then(|file| file.package_root.as_ref());
        if owner.is_some_and(|owner| owner == package_root) {
            return Present;
        }
        let initially_loaded = owner.is_some_and(|owner| {
            let (loaded, _) = self.namespace_load_closure(
                Some(owner),
                &BTreeSet::new(),
                &BTreeSet::from([owner.clone()]),
            );
            loaded.contains(package_root)
        });
        let mut activity = if initially_loaded { Present } else { Absent };
        let function = call
            .syntax()
            .ancestors()
            .find_map(FunctionExpr::cast)
            .map(|function| function.syntax().text_range());
        let boundary = call.syntax().text_range().start();
        let mut events: Vec<_> = extracted
            .s4_package_load_sites
            .iter()
            .filter(|site| {
                site.range.end() <= boundary
                    && (site.function.is_none() || site.function == function)
            })
            .filter_map(|site| {
                let load = self.s4_loader_activity(path, &site.load);
                let target = site.load.package.as_deref();
                let attaches = site.load.attach_depends();
                let reachable =
                    target.is_some_and(|target| loads_package(target, attaches == Some(true)));
                let possible = target.is_none()
                    || reachable
                    || (attaches.is_none()
                        && target.is_some_and(|target| loads_package(target, true)));
                (load != Absent && possible).then_some((
                    site.range.end(),
                    true,
                    load == Present && site.load.unconditional && reachable,
                ))
            })
            .collect();
        if self.s7_has_hook(package_root, S7HookKind::Unload) {
            let (root, model) = live;
            events.extend(
                root.descendants()
                    .filter_map(CallExpr::cast)
                    .filter_map(|call| {
                        let range = call.syntax().text_range();
                        if range.end() > boundary
                            || call
                                .syntax()
                                .ancestors()
                                .find_map(FunctionExpr::cast)
                                .map(|function| function.syntax().text_range())
                                .is_some_and(|owner| Some(owner) != function)
                            || self
                                .s7_helper(path, &call_function_ref(model, root, &call)?, "base")
                                .as_deref()
                                != Some("unloadNamespace")
                        {
                            return None;
                        }
                        let target = argument(&matched_args(&call, &["ns"])?, "ns")
                            .and_then(super::properties::string_literal);
                        target
                            .as_deref()
                            .is_none_or(|target| target == package)
                            .then_some((
                                range.end(),
                                false,
                                target.is_some() && unconditional(call.syntax()),
                            ))
                    }),
            );
        }
        events.sort_by_key(|(point, _, _)| *point);
        for (_, load, definite) in events {
            let result = if load { Present } else { Absent };
            if definite {
                activity = result;
            } else if activity != result {
                activity = Unknown;
            }
        }
        activity
    }

    pub fn s7_has_hook(&self, root: &Path, kind: S7HookKind) -> bool {
        self.oop_files().any(|file| {
            file.package_root.as_deref() == Some(root)
                && file.facts.oop.s7.hooks.iter().any(|hook| {
                    hook.kind == kind
                        && hook.unconditional
                        && self.s7_helper(&file.path, &hook.factory, "S7").is_some()
                })
        })
    }

    fn s7_implementation_signature(
        &self,
        id: &OopId,
        registration: &S7Registration,
    ) -> Option<CallableSignature> {
        if let Some(signature) = self.method(id)?.signature.clone() {
            return Some(signature);
        }
        let (name, package) = match registration.implementation.as_ref()? {
            MethodImplementationSource::Reference { name } => (name, None),
            MethodImplementationSource::Qualified { package, name } => {
                (name, Some(package.as_str()))
            }
            _ => return None,
        };
        let mut signatures = self
            .implementation_files(&id.path, name, package)
            .into_iter()
            .filter_map(|path| self.file(&path))
            .flat_map(|file| {
                file.facts
                    .oop
                    .callables
                    .iter()
                    .filter(|callable| callable.name == *name)
            });
        let signature = signatures.next()?.signature.clone();
        signatures.next().is_none().then_some(signature)
    }

    pub(in crate::project::classes) fn s7_parameter_classes(
        &self,
        path: &Path,
        root: &SyntaxNode,
        binding: &crate::semantic::Binding,
        implementations: &[MethodImplementationSite],
    ) -> (BTreeSet<ClassId>, bool) {
        let Some(file) = self.file(path) else {
            return (BTreeSet::new(), false);
        };
        let mut classes = BTreeSet::new();
        let mut unknown = false;
        for site in implementations.iter().filter(|site| {
            site.method.system == OopSystem::S7
                && site.function_range.contains_range(binding.def_range)
        }) {
            let Some(function) = root
                .covering_element(site.function_range)
                .into_node()
                .and_then(FunctionExpr::cast)
            else {
                continue;
            };
            if !function
                .formals()
                .iter()
                .any(|formal| formal.name_token().text_range() == binding.def_range)
            {
                continue;
            }
            if site.method.kind == OopKind::S7Function {
                if function
                    .formals()
                    .first()
                    .is_none_or(|formal| formal.name_token().text_range() != binding.def_range)
                {
                    continue;
                }
                let is_binding = |function: &S7Function| matches!(function, S7Function::Known { key: Some(key), .. } if *key == site.method);
                for (key, class) in &file.facts.oop.s7.classes {
                    if is_binding(&class.validator) {
                        classes.insert(ClassId {
                            path: path.to_path_buf(),
                            key: key.clone(),
                        });
                    }
                }
                for member in &file.facts.oop.members {
                    let Some(property) = file.facts.oop.s7.properties.get(&member.key) else {
                        continue;
                    };
                    let Some(key) = &member.class else {
                        continue;
                    };
                    if is_binding(&property.getter) || is_binding(&property.setter) {
                        classes.insert(ClassId {
                            path: path.to_path_buf(),
                            key: key.clone(),
                        });
                    }
                    if is_binding(&property.validator) {
                        let value_class = property.class.as_ref().filter(|_| {
                            property.complete
                                && property.factory.as_ref().is_some_and(|factory| {
                                    self.s7_helper(path, factory, "S7").is_some()
                                })
                        });
                        if let Some(spec) = value_class {
                            let (candidates, incomplete) =
                                self.s7_spec_candidates(path, &key.owner, spec);
                            classes.extend(candidates);
                            unknown |= incomplete;
                        } else {
                            unknown = true;
                        }
                    }
                }
            } else if site.method.kind == OopKind::Method {
                let id = OopId {
                    path: path.to_path_buf(),
                    key: site.method.clone(),
                };
                let Some(resolved) = self.s7_registration(&id) else {
                    unknown = true;
                    continue;
                };
                let [S7GenericTarget::S7(generic)] = resolved.targets.as_slice() else {
                    unknown = true;
                    continue;
                };
                let name = unbacktick(&binding.name);
                let Some(position) = self.generic(generic).and_then(|generic| {
                    generic
                        .dispatch_arguments
                        .iter()
                        .position(|arg| matches!(arg, ClassName::Known(arg) if arg == name))
                }) else {
                    unknown = true;
                    continue;
                };
                let Some(method) = self.method(&id) else {
                    unknown = true;
                    continue;
                };
                if !self.s7_signature_list_is_base(path, method) {
                    unknown = true;
                    continue;
                }
                let Some(arg) = method.dispatch.get(position) else {
                    unknown = true;
                    continue;
                };
                let spec = match &arg.class {
                    DispatchClass::Generator(reference) => {
                        S7ClassSpec::Generator(reference.clone())
                    }
                    DispatchClass::S7(spec) => spec.clone(),
                    _ => {
                        unknown = true;
                        continue;
                    }
                };
                let (candidates, incomplete) = self.s7_spec_candidates(path, &id.key.owner, &spec);
                classes.extend(candidates);
                unknown |= incomplete;
            }
        }
        (classes, unknown)
    }
}

fn namespace_package(element: SyntaxElement) -> Option<String> {
    let access = BinaryExpr::cast(element.into_node()?)?.namespace_access()?;
    Some(access.package.to_string())
}

fn method_matches_generic(
    generic: &GenericDef,
    method: &CallableSignature,
    version: S7Version,
) -> bool {
    let Some(signature) = &generic.signature else {
        return false;
    };
    if method.parameters.len() < generic.dispatch_arguments.len()
        || !method
            .parameters
            .iter()
            .zip(&generic.dispatch_arguments)
            .all(|(formal, arg)| {
                formal.default.is_none()
                    && matches!(arg, ClassName::Known(name) if *name == formal.name)
            })
    {
        return false;
    }
    if signature
        .parameters
        .iter()
        .any(|formal| formal.name == "...")
    {
        return true;
    }
    method.parameters.len() == signature.parameters.len()
        && method.parameters.iter().zip(&signature.parameters).all(|(method, generic_parameter)| {
            method.name == generic_parameter.name && (method.default == generic_parameter.default
                || version == S7Version::Development && generic.dispatch_arguments.iter().any(|name| {
                    matches!(name, ClassName::Known(name) if *name == method.name)
                }))
        })
}

fn spec_has_external(spec: &S7ClassSpec) -> bool {
    match spec {
        S7ClassSpec::External { .. } => true,
        S7ClassSpec::Union { classes, .. } => classes.iter().any(spec_has_external),
        _ => false,
    }
}

fn builtin_dispatch_tags(name: &str) -> Vec<String> {
    let tags: &[&str] = match name {
        "class_any" => &["ANY"],
        "class_missing" => &["MISSING"],
        "class_numeric" => &["integer", "double"],
        "class_language" => &["name", "call"],
        "class_atomic" => &[
            "logical",
            "integer",
            "double",
            "complex",
            "character",
            "raw",
        ],
        "class_vector" => &[
            "logical",
            "integer",
            "double",
            "complex",
            "character",
            "raw",
            "expression",
            "list",
        ],
        _ => &[name.trim_start_matches("class_")],
    };
    tags.iter().map(|tag| (*tag).into()).collect()
}

fn builtin_lineage(name: &str) -> Option<Vec<String>> {
    let mut tags = builtin_dispatch_tags(name);
    if tags.len() != 1 || matches!(name, "class_any" | "class_missing") {
        return None;
    }
    if matches!(name, "class_POSIXct" | "class_POSIXlt") {
        tags.push("POSIXt".into());
    }
    if name != "S7_object" {
        tags.push("S7_object".into());
    }
    Some(tags)
}

pub(in crate::project::classes) fn literal_class(value: &SyntaxElement) -> Option<&'static str> {
    let token = value.as_token()?;
    Some(match token.kind() {
        SyntaxKind::STRING => "character",
        _ => match token.text() {
            "NULL" => "NULL",
            "TRUE" | "FALSE" | "NA" => "logical",
            "NA_integer_" => "integer",
            "NA_real_" | "NaN" | "Inf" => "double",
            "NA_character_" => "character",
            "NA_complex_" => "complex",
            _ => match crate::ast::Expr::cast(value.clone())? {
                crate::ast::Expr::IntLiteral(_) => {
                    // R keeps an L-suffixed value as a double when it cannot
                    // represent it in its integer range.
                    let text = token.text().strip_suffix('L')?;
                    let number: f64 =
                        match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                            Some(hex) => i64::from_str_radix(hex, 16).ok()? as f64,
                            None => text.parse().ok()?,
                        };
                    if number.fract() == 0.0 && (0.0..=f64::from(i32::MAX)).contains(&number) {
                        "integer"
                    } else {
                        "double"
                    }
                }
                crate::ast::Expr::FloatLiteral(_) => "double",
                crate::ast::Expr::ComplexLiteral(_) => "complex",
                _ => return None,
            },
        },
    })
}
