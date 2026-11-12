//! Generator mutations retain source order without putting offsets in the facts.

use super::*;

impl Projection<'_> {
    fn mutator_base(&self, value: SyntaxElement, depth: usize) -> Option<BinaryExpr> {
        if depth >= 64 {
            return None;
        }
        if let Some(binary) = value.as_node().cloned().and_then(BinaryExpr::cast) {
            return Some(binary);
        }
        let binding = value
            .as_token()
            .and_then(|token| read_binding(self.model, token))?;
        self.mutator_base(stable_value(self.model, self.root, binding)?, depth + 1)
    }
    pub(super) fn execution_events(&mut self) {
        for (class, location) in self
            .classes
            .facts
            .declarations
            .iter()
            .zip(&self.classes.locations)
        {
            if crate::semantic::builder::is_quoted(
                &self
                    .root
                    .covering_element(location.call_range)
                    .parent()
                    .unwrap_or_else(|| self.root.clone()),
            ) {
                continue;
            }
            self.events
                .push((location.call_range, OopEvent::Class(class.key.clone())));
        }
        for node in self.root.children() {
            if let Some(edge) = crate::project::source::top_level_source_edge_key(&node, None) {
                self.events
                    .push((node.text_range(), OopEvent::Source(edge)));
            }
        }
        for call in self.root.descendants().filter_map(CallExpr::cast) {
            if call
                .syntax()
                .ancestors()
                .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
                || crate::semantic::builder::is_quoted(call.syntax())
            {
                continue;
            }
            let Some(name) = call.callee_name() else {
                continue;
            };
            let name = if replacement_assignment(&call).is_some() {
                format!("{name}<-")
            } else {
                name.to_string()
            };
            if crate::semantic::calls::changes_bindings(&name) && name != "source" {
                self.events
                    .push((call.syntax().text_range(), OopEvent::InvalidateBindings));
            }
        }
        for binding in self.model.bindings() {
            if self.model.scope(binding.scope).kind != crate::semantic::ScopeKind::File {
                continue;
            }
            let Some(assignment) = binding_assignment(self.root, binding.def_range) else {
                continue;
            };
            let Some(value) = assignment.value_element() else {
                continue;
            };
            let name = unbacktick(&binding.name).to_string();
            if !unconditional(assignment.syntax()) {
                self.events.push((
                    value.text_range(),
                    OopEvent::UncertainGenerator {
                        name,
                        target: self.generator(&value),
                    },
                ));
                continue;
            }
            if let Some(signature) = signature(self.implementation_value(
                Some(value.clone()),
                &mut BTreeSet::new(),
                false,
            )) {
                let index = self.facts.callables.len();
                self.facts.callables.push(CallableBinding {
                    name: name.clone(),
                    signature,
                });
                self.events
                    .push((value.text_range(), OopEvent::Callable(index)));
                continue;
            }
            let mut target = self.generator(&value);
            if target == GeneratorRef::Unknown
                && let Some(token) = value
                    .as_token()
                    .filter(|token| token.kind() == SyntaxKind::IDENT)
            {
                target = GeneratorRef::Free(unbacktick(token.text()).into());
            }
            if target != GeneratorRef::Unknown {
                self.events
                    .push((value.text_range(), OopEvent::Generator { name, target }));
            } else {
                self.events
                    .push((value.text_range(), OopEvent::Binding(name)));
            }
        }
    }
    fn mutation_target(
        &self,
        value: &SyntaxElement,
        depth: usize,
    ) -> (ParentRef, Option<FunctionRef>) {
        if depth >= 64 {
            return (ParentRef::Unknown, None);
        }
        let generator = self.generator(value);
        if generator != GeneratorRef::Unknown {
            return (ParentRef::Generator(generator), None);
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast)
            && let Some((factory, name)) = factory(self.model, self.root, &call)
            && name == "getRefClass"
            && accepts(&factory, "methods")
            && let Some(args) = matched_args(&call, &["Class", "where"])
            && !args.contains_key("where")
            && let ClassName::Known(name) = string_name(argument(&args, "Class"))
        {
            return (ParentRef::Formal(name), Some(factory));
        }
        if let Some(binding) = value
            .as_token()
            .and_then(|token| read_binding(self.model, token))
            && let Some(value) = stable_value(self.model, self.root, binding)
        {
            return self.mutation_target(&value, depth + 1);
        }
        if let Some(binding) = value
            .as_token()
            .and_then(|token| read_binding(self.model, token))
            && self.model.scope(self.model.binding(binding).scope).kind
                == crate::semantic::ScopeKind::File
        {
            return (
                ParentRef::Generator(GeneratorRef::Free(
                    unbacktick(&self.model.binding(binding).name).into(),
                )),
                None,
            );
        }
        (ParentRef::Unknown, None)
    }

    pub(super) fn mutations(&mut self) {
        for call in self.root.descendants().filter_map(CallExpr::cast) {
            if crate::semantic::builder::is_quoted(call.syntax()) {
                continue;
            }
            if let Some((factory, name)) = factory(self.model, self.root, &call)
                && matches!(name.as_str(), "lockBinding" | "unlockBinding")
                && accepts(&factory, "base")
                && let Some(args) = matched_args(&call, &["sym", "env"])
                && let ClassName::Known(name_value) = string_name(argument(&args, "sym"))
                && let Some(class_name) = name_value.strip_prefix(".__C__")
            {
                let mut issues = BTreeSet::new();
                let deferred = call
                    .syntax()
                    .ancestors()
                    .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
                if !unconditional(call.syntax()) || deferred {
                    issues.insert(OopUncertainty::Conditional);
                }
                let environment = argument(&args, "env")
                    .and_then(SyntaxElement::into_node)
                    .and_then(CallExpr::cast)
                    .filter(|call| {
                        call.arg_list()
                            .is_some_and(|args| args.args().next().is_none())
                    })
                    .and_then(|call| call_function_ref(self.model, self.root, &call))
                    .filter(|factory| {
                        possible_functions(factory).iter().any(|identity| {
                            identity.package == "base"
                                && matches!(identity.name.as_str(), "environment" | "globalenv")
                        })
                    });
                if environment.is_none() {
                    issues.insert(OopUncertainty::CustomEnvironment);
                }
                self.push_mutation(
                    &call,
                    None,
                    MemberMutation {
                        owner: owner_at(call.syntax(), self.owners),
                        target: ParentRef::Formal(class_name.into()),
                        lookup: Some(factory),
                        kind: MutationKind::ClassBindingLock(name == "lockBinding"),
                        members: Vec::new(),
                        removals: Vec::new(),
                        factory: environment,
                        uncertainty: issues,
                        deferred,
                    },
                );
                continue;
            }
            let indirect = factory(self.model, self.root, &call)
                .is_some_and(|(factory, name)| name == "do.call" && accepts(&factory, "base"));
            let base = if indirect {
                matched_args(&call, &["what", "args", "quote", "envir"])
                    .and_then(|args| argument(&args, "what"))
            } else {
                call.base()
            };
            let Some(binary) = base.and_then(|base| self.mutator_base(base, 0)) else {
                continue;
            };
            if binary.op_kind() != Some(SyntaxKind::DOLLAR) {
                continue;
            }
            let (Some(receiver), Some(name)) =
                (binary.lhs(), binary.rhs().as_ref().and_then(literal))
            else {
                continue;
            };
            if !matches!(
                name.as_str(),
                "set" | "methods" | "lock" | "unlock" | "accessors"
            ) {
                continue;
            }
            let (target, lookup) = self.mutation_target(&receiver, 0);
            if target == ParentRef::Unknown {
                continue;
            }
            let acquisition = (!call
                .syntax()
                .text_range()
                .contains_range(binary.syntax().text_range()))
            .then(|| binary.syntax().text_range());
            let mut issues = BTreeSet::new();
            let owner = owner_at(call.syntax(), self.owners);
            let deferred = call
                .syntax()
                .ancestors()
                .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
            if !unconditional(call.syntax()) || deferred {
                issues.insert(OopUncertainty::Conditional);
            }
            let class = match &target {
                ParentRef::Generator(GeneratorRef::Local(key)) => Some(key.clone()),
                _ => None,
            };
            let mut mutation = MemberMutation {
                owner,
                target,
                lookup,
                kind: MutationKind::Dynamic,
                members: Vec::new(),
                removals: Vec::new(),
                factory: None,
                uncertainty: issues.clone(),
                deferred,
            };
            if indirect {
                mutation.uncertainty.insert(OopUncertainty::DynamicMembers);
                self.push_mutation(&call, acquisition, mutation);
                continue;
            }
            let system = if name == "methods" || name == "accessors" {
                ClassSystem::RefClass
            } else {
                class.as_ref().map_or(ClassSystem::R6, |key| key.system)
            };
            match name.as_str() {
                "set" => {
                    let Some(args) = matched_args(&call, &["which", "name", "value", "overwrite"])
                    else {
                        mutation.uncertainty.insert(OopUncertainty::DynamicMembers);
                        self.push_mutation(&call, acquisition, mutation);
                        continue;
                    };
                    let group = match string_name(argument(&args, "which")) {
                        ClassName::Known(name) => Some(name),
                        _ => None,
                    };
                    let overwrite = match argument(&args, "overwrite") {
                        None => Some(false),
                        Some(SyntaxElement::Token(token)) if token.text() == "TRUE" => Some(true),
                        Some(SyntaxElement::Token(token)) if token.text() == "FALSE" => Some(false),
                        _ => None,
                    };
                    mutation.kind = MutationKind::R6Set {
                        group: group.clone(),
                        overwrite,
                    };
                    let member_name = string_name(argument(&args, "name"));
                    if group.is_none() || member_name == ClassName::Unknown || overwrite.is_none() {
                        mutation.uncertainty.insert(OopUncertainty::DynamicMembers);
                    }
                    if let Some(value) = argument(&args, "value") {
                        let range = argument(&args, "name")
                            .map_or(value.text_range(), |name| name.text_range());
                        let entry = MemberEntry {
                            name: match member_name {
                                ClassName::Known(name) => Some(name),
                                _ => None,
                            },
                            value: Some(value),
                            range,
                            untyped: false,
                        };
                        let key = self.member_entry(
                            &call,
                            MemberInput {
                                system: ClassSystem::R6,
                                class: None,
                                visibility: if group.as_deref() == Some("private") {
                                    MemberVisibility::Private
                                } else {
                                    MemberVisibility::Public
                                },
                                kind: if group.as_deref() == Some("active") {
                                    MemberKind::ActiveBinding
                                } else {
                                    MemberKind::Field
                                },
                                reference_field: false,
                                issues,
                                entry,
                            },
                        );
                        mutation.members.push(key);
                    } else {
                        mutation
                            .uncertainty
                            .insert(OopUncertainty::InvalidArguments);
                    }
                }
                "methods" => {
                    mutation.kind = MutationKind::ReferenceMethods;
                    let Some(list) = call.arg_list() else {
                        continue;
                    };
                    let args: Vec<_> = list.args().collect();
                    if args.is_empty() {
                        continue;
                    }
                    if args.len() == 1
                        && args[0].name().is_none()
                        && string_name(args[0].value()) != ClassName::Unknown
                    {
                        continue;
                    }
                    let mut entries = Vec::new();
                    for arg in args {
                        if let Some(name) = arg.name() {
                            entries.push(MemberEntry {
                                name: Some(unbacktick(&name).into()),
                                value: arg.value(),
                                range: arg
                                    .name_token()
                                    .map_or(arg.syntax().text_range(), |token| token.text_range()),
                                untyped: false,
                            });
                        } else {
                            let value =
                                self.implementation_value(arg.value(), &mut BTreeSet::new(), false);
                            let constructor = value
                                .and_then(SyntaxElement::into_node)
                                .and_then(CallExpr::cast);
                            if let Some(constructor) = constructor
                                && let Some((factory, name)) =
                                    factory(self.model, self.root, &constructor)
                                && name == "list"
                                && accepts(&factory, "base")
                            {
                                mutation.factory = Some(factory);
                                if let Some(args) = constructor.arg_list() {
                                    entries.extend(args.args().map(|arg| {
                                        MemberEntry {
                                            name: arg.name().map(|name| unbacktick(&name).into()),
                                            value: arg.value(),
                                            range: arg
                                                .name_token()
                                                .map_or(arg.syntax().text_range(), |token| {
                                                    token.text_range()
                                                }),
                                            untyped: false,
                                        }
                                    }));
                                }
                            } else {
                                mutation.uncertainty.insert(OopUncertainty::DynamicMembers);
                            }
                        }
                    }
                    if !call
                        .syntax()
                        .children_with_tokens()
                        .any(|e| e.kind() == SyntaxKind::RPAREN)
                    {
                        mutation.uncertainty.insert(OopUncertainty::DynamicMembers);
                    }
                    for entry in entries {
                        if entry.value.as_ref().is_some_and(|value| matches!(value, SyntaxElement::Token(token) if token.text() == "NULL")) {
                            if let Some(name) = entry.name { mutation.removals.push(name); }
                            else { mutation.uncertainty.insert(OopUncertainty::DynamicMembers); }
                        } else {
                            mutation.members.push(self.member_entry(&call, MemberInput {
                                system: ClassSystem::RefClass, class: None, visibility: MemberVisibility::Public,
                                kind: MemberKind::Method, reference_field: false, issues: issues.clone(), entry,
                            }));
                        }
                    }
                }
                "lock" | "unlock" => {
                    let fields = call
                        .arg_list()
                        .map(|list| list.args().map(|arg| string_name(arg.value())).collect())
                        .unwrap_or_default();
                    mutation.kind = MutationKind::Lock {
                        locked: name == "lock",
                        fields,
                    };
                }
                _ => {
                    mutation.uncertainty.insert(OopUncertainty::DynamicMembers);
                }
            }
            // A known receiver of another system does not acquire this API.
            if class_system(&mutation.target).is_none_or(|known| known == system)
                || matches!(mutation.kind, MutationKind::Lock { .. })
            {
                self.push_mutation(&call, acquisition, mutation);
            }
        }
        for assignment in self.root.descendants().filter_map(AssignmentExpr::cast) {
            if crate::semantic::builder::is_quoted(assignment.syntax()) {
                continue;
            }
            let mut receiver = assignment.target_element();
            let mut target = ParentRef::Unknown;
            while let Some(binary) = receiver
                .and_then(SyntaxElement::into_node)
                .and_then(BinaryExpr::cast)
            {
                if binary.op_kind() != Some(SyntaxKind::DOLLAR) {
                    break;
                }
                let Some(lhs) = binary.lhs() else { break };
                target = self.mutation_target(&lhs, 0).0;
                if target != ParentRef::Unknown {
                    break;
                }
                receiver = Some(lhs);
            }
            if target == ParentRef::Unknown {
                continue;
            }
            let index = self.facts.mutations.len();
            self.facts.mutations.push(MemberMutation {
                owner: owner_at(assignment.syntax(), self.owners),
                target,
                lookup: None,
                kind: MutationKind::Dynamic,
                members: Vec::new(),
                removals: Vec::new(),
                factory: None,
                uncertainty: BTreeSet::from([OopUncertainty::DynamicMembers]),
                deferred: false,
            });
            self.events
                .push((assignment.syntax().text_range(), OopEvent::Mutation(index)));
        }
    }

    fn push_mutation(
        &mut self,
        call: &CallExpr,
        acquisition: Option<TextRange>,
        mutation: MemberMutation,
    ) {
        let index = self.facts.mutations.len();
        if let Some(acquisition) = acquisition {
            self.events.push((acquisition, OopEvent::Mutator(index)));
        }
        self.facts.mutations.push(mutation);
        self.events
            .push((call.syntax().text_range(), OopEvent::Mutation(index)));
    }
}

fn class_system(target: &ParentRef) -> Option<ClassSystem> {
    match target {
        ParentRef::Generator(GeneratorRef::Local(key)) => Some(key.system),
        _ => None,
    }
}
