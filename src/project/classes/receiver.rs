//! Bounded receiver evidence shared by class systems, without executing accessors.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassAccess {
    Generator,
    Public,
    Private,
    Super,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassReceiver {
    pub class: ClassId,
    pub access: ClassAccess,
    /// A declared field, slot, property, or parameter type can admit subclasses.
    pub exact: bool,
    pub point: Option<OopPoint>,
    pub cached_members: BTreeMap<String, OopPoint>,
    pub uncertain_cache: bool,
}

impl ClassReceiver {
    fn new(class: ClassId, access: ClassAccess, point: Option<OopPoint>) -> Self {
        Self {
            class,
            access,
            exact: true,
            point,
            cached_members: BTreeMap::new(),
            uncertain_cache: false,
        }
    }
}

/// Possible receiver values. An unknown path may coexist with known candidates.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReceiverEvidence {
    pub candidates: Vec<ClassReceiver>,
    pub unknown: bool,
}

const MAX_RECEIVER_CANDIDATES: usize = 32;

impl ReceiverEvidence {
    fn unique(self) -> Option<ClassReceiver> {
        if self.unknown || self.candidates.len() != 1 {
            return None;
        }
        self.candidates.into_iter().next()
    }

    fn project(self, project: impl Fn(ClassReceiver) -> Option<ClassReceiver>) -> Self {
        let mut result = Self {
            candidates: Vec::new(),
            unknown: self.unknown,
        };
        for receiver in self.candidates {
            match project(receiver) {
                Some(candidate) if !result.candidates.contains(&candidate) => {
                    result.candidates.push(candidate);
                }
                Some(_) => {}
                None => result.unknown = true,
            }
        }
        result
    }

    fn project_many(self, project: impl Fn(ClassReceiver) -> Option<Self>) -> Self {
        let mut result = Self {
            candidates: Vec::new(),
            unknown: self.unknown,
        };
        for receiver in self.candidates {
            match project(receiver) {
                Some(evidence) => result.merge(evidence),
                None => result.unknown = true,
            }
        }
        result
    }

    fn instances(
        classes: impl IntoIterator<Item = ClassId>,
        unknown: bool,
        point: Option<OopPoint>,
    ) -> Self {
        let mut classes = classes.into_iter();
        let candidates = classes
            .by_ref()
            .take(MAX_RECEIVER_CANDIDATES)
            .map(|class| ClassReceiver::new(class, ClassAccess::Public, point.clone()))
            .collect();
        Self {
            candidates,
            unknown: unknown || classes.next().is_some(),
        }
    }

    fn merge(&mut self, other: Self) {
        self.unknown |= other.unknown;
        for candidate in other.candidates {
            if !self.candidates.contains(&candidate) {
                if self.candidates.len() == MAX_RECEIVER_CANDIDATES {
                    self.unknown = true;
                    break;
                }
                self.candidates.push(candidate);
            }
        }
    }
}

impl ClassIndex {
    pub fn class_receiver(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<ClassReceiver> {
        self.class_receiver_evidence(path, root, model, value)
            .unique()
    }

    pub fn class_receiver_evidence(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> ReceiverEvidence {
        let extracted = extract(model, root, false);
        self.receiver_for(path, root, model, value, &extracted)
    }

    pub(super) fn dispatch_receiver(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
        extracted: &Extracted,
    ) -> Option<ClassReceiver> {
        self.receiver_for(path, root, model, value, extracted)
            .unique()
            .filter(|receiver| receiver.exact)
    }

    fn receiver_for(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
        extracted: &Extracted,
    ) -> ReceiverEvidence {
        let keys = extracted
            .facts
            .declarations
            .iter()
            .zip(&extracted.locations)
            .map(|(class, location)| (location.call_range, class.key.clone()))
            .collect();
        ReceiverLookup {
            index: self,
            path,
            root,
            model,
            keys: &keys,
            extracted,
            factory: None,
        }
        .evidence(value, &mut 32)
    }

    /// Callable class surface, including implicit Reference Class method calls and
    /// callSuper's lexical method owner. Accessor reads are never executed.
    pub fn class_call_signature(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Option<(String, CallableSignature)> {
        let base = call.base()?;
        if let Some(binary) = base.as_node().cloned().and_then(BinaryExpr::cast)
            && binary.op_kind() == Some(SyntaxKind::DOLLAR)
            && let Some(SyntaxElement::Token(name)) = binary.rhs()
        {
            let receiver = self
                .class_receiver(path, root, model, &binary.lhs()?)
                .filter(|receiver| receiver.exact)?;
            let name = unbacktick(name.text());
            if receiver.class.key.system == ClassSystem::R6 {
                let api = self.r6_api_at(&receiver.class, receiver.point.as_ref())?;
                let surface = api.members(receiver.access);
                if !surface.complete {
                    return None;
                }
                let signature = if receiver.access == ClassAccess::Generator && name == "new" {
                    api.initializer?
                } else {
                    let [member] = surface.entries.get(name)?.as_slice() else {
                        return None;
                    };
                    if member.kind != MemberKind::Method {
                        return None;
                    }
                    member.signature.clone()?
                };
                return Some((name.into(), signature));
            }
            let api = self.reference_receiver_api(&receiver)?;
            let signature = if receiver.access == ClassAccess::Generator && name == "new" {
                api.initializer?
            } else {
                let surface = if receiver.access == ClassAccess::Generator {
                    &api.generator
                } else {
                    &api.instance
                };
                if !surface.complete {
                    return None;
                }
                let [member] = surface.entries.get(name)?.as_slice() else {
                    return None;
                };
                if member.kind != MemberKind::Method {
                    return None;
                }
                member.signature.clone()?
            };
            return Some((name.into(), signature));
        }
        if let Some(receiver) = self.class_receiver(path, root, model, &base)
            && receiver.exact
            && receiver.access == ClassAccess::Generator
        {
            if receiver.class.key.system == ClassSystem::RefClass {
                return Some((
                    base.to_string(),
                    self.reference_receiver_api(&receiver)?.initializer?,
                ));
            }
            if self.s7_constructible(&receiver.class) {
                return Some((base.to_string(), self.s7_api(&receiver.class)?.constructor?));
            }
        }
        if let Some(signature) = self.s7_generic_signature(path, root, model, &base) {
            return Some((base.to_string(), signature));
        }
        let token = base.as_token()?;
        let runtime = apply_method_environments(model, root, &self.method_environments(path))?;
        let ident = runtime
            .idents()
            .iter()
            .find(|ident| ident.range == token.text_range())?;
        let resolution = runtime.identifier_resolution(ident);
        if !resolution.lexical.is_empty()
            || resolution.generated.is_empty()
            || resolution.uncertain
            || resolution.external
        {
            return None;
        }
        let name = unbacktick(token.text());
        let sites = method_implementation_sites(model, root);
        let innermost = sites
            .iter()
            .filter(|site| site.function_range.contains_range(token.text_range()))
            .map(|site| site.function_range.len())
            .min()?;
        let mut candidates = sites
            .iter()
            .filter(|site| {
                site.function_range.contains_range(token.text_range())
                    && site.function_range.len() == innermost
            })
            .filter_map(|site| {
                self.file(path)?
                    .facts
                    .oop
                    .methods
                    .iter()
                    .find(|method| method.key == site.method)
            })
            .filter(|method| method.system == OopSystem::RefClass)
            .collect::<Vec<_>>();
        candidates.dedup_by_key(|method| method.key.clone());
        let [method] = candidates.as_slice() else {
            return None;
        };
        let class = self.method_class(path, &method.key)?;
        let member = if name == "callSuper" {
            let ClassName::Known(method_name) = &method.name else {
                return None;
            };
            self.reference_super_method(&class, method_name)?
        } else {
            let api = self.reference_api(&class)?;
            if !api.instance.complete {
                return None;
            }
            let [member] = api.instance.entries.get(name)?.as_slice() else {
                return None;
            };
            member.clone()
        };
        if member.kind != MemberKind::Method {
            return None;
        }
        Some((name.into(), member.signature?))
    }
}

struct ReceiverLookup<'a> {
    index: &'a ClassIndex,
    path: &'a std::path::Path,
    root: &'a SyntaxNode,
    model: &'a SemanticModel,
    keys: &'a HashMap<TextRange, ClassKey>,
    extracted: &'a Extracted,
    factory: Option<&'a FactoryFrame<'a>>,
}

struct FactoryFrame<'a> {
    function: TextRange,
    point: Option<OopPoint>,
    call: SyntaxElement,
    caller: Option<&'a FactoryFrame<'a>>,
}

impl ReceiverLookup<'_> {
    fn s7_constructor_returns_class(&self, id: &ClassId) -> bool {
        use super::s7::S7Function;

        let Some(class) = self.index.s7_class(id) else {
            return false;
        };
        match &class.constructor {
            S7Function::Absent => true,
            S7Function::Known { key: Some(key), .. } if id.path == self.path => self
                .extracted
                .method_implementations
                .iter()
                .filter(|site| site.method == *key)
                .any(|site| {
                    let body = self
                        .root
                        .covering_element(site.function_range)
                        .into_node()
                        .filter(|node| node.text_range() == site.function_range)
                        .and_then(FunctionExpr::cast)
                        .and_then(|function| function.body());
                    let Some(call) = body
                        .and_then(SyntaxElement::into_node)
                        .and_then(CallExpr::cast)
                    else {
                        return false;
                    };
                    self.index
                        .s7_new_object_in(
                            self.path,
                            self.root,
                            self.model,
                            &call,
                            &self.extracted.method_implementations,
                        )
                        .as_ref()
                        == Some(id)
                }),
            _ => false,
        }
    }

    fn local_return(&self, base: &SyntaxElement) -> Option<(TextRange, SyntaxElement)> {
        let token = base.as_token()?;
        let binding = read_binding(self.model, token)?;
        if self.model.binding(binding).def_range.start() >= token.text_range().start() {
            return None;
        }
        let function = stable_value(self.model, self.root, binding)?
            .into_node()
            .and_then(FunctionExpr::cast)?;
        let mut body = function.body()?;
        let mut local = None;
        if let Some(block) = body
            .as_node()
            .cloned()
            .and_then(crate::ast::BlockExpr::cast)
        {
            let mut statements = block.statements();
            body = statements.next()?;
            if let Some(result) = statements.next() {
                if statements.next().is_some() {
                    return None;
                }
                local = body
                    .into_node()
                    .and_then(AssignmentExpr::cast)
                    .and_then(|assignment| assignment.target_name_token());
                local.as_ref()?;
                body = result;
            }
        }
        if let Some(call) = body.as_node().cloned().and_then(CallExpr::cast)
            && call_function_ref(self.model, self.root, &call)
                .and_then(|reference| self.index.callee_identity(self.path, &reference))
                .is_some_and(|identity| identity.package == "base" && identity.name == "return")
        {
            let list = call.arg_list()?;
            let mut args = list.args();
            body = args.next()?.value()?;
            if args.next().is_some() {
                return None;
            }
        }
        if let Some(local) = local {
            let token = body
                .as_token()
                .filter(|token| token.kind() == SyntaxKind::IDENT)?;
            let binding = read_binding(self.model, token)?;
            if self.model.binding(binding).def_range != local.text_range() {
                return None;
            }
            body = stable_value(self.model, self.root, binding)?;
        }
        Some((function.syntax().text_range(), body))
    }

    fn factory_frames(&self) -> impl Iterator<Item = &FactoryFrame<'_>> {
        std::iter::successors(self.factory, |frame| frame.caller)
    }

    fn enter_factory(&self, function: TextRange, call: &SyntaxElement) -> Option<FactoryFrame<'_>> {
        if self
            .factory_frames()
            .any(|frame| frame.function == function)
        {
            return None;
        }
        Some(FactoryFrame {
            function,
            point: self.point(call),
            call: call.clone(),
            caller: self.factory,
        })
    }

    fn binding_read_offset(&self, value: &SyntaxElement, definition: TextRange) -> TextSize {
        if self.factory.is_none() {
            return value.text_range().start();
        }
        let owner = enclosing_function(&self.root.covering_element(definition));
        // Follow call sites until the read reaches the binding's lexical frame.
        let mut read = value;
        let mut frames = self.factory_frames();
        while let Some(function) = enclosing_function(read) {
            if owner == Some(function) {
                break;
            }
            let Some(frame) = frames.find(|frame| frame.function == function) else {
                break;
            };
            read = &frame.call;
        }
        read.text_range().start()
    }

    fn point(&self, value: &SyntaxElement) -> Option<OopPoint> {
        if let Some(function) = enclosing_function(value) {
            // Only expressions evaluated in an active factory use its call site.
            // Captured values trace back to the frame that created them.
            return self
                .factory_frames()
                .find(|frame| frame.function == function)?
                .point
                .clone();
        }
        Some(
            self.extracted
                .oop_point_at(self.path, value.text_range().start()),
        )
    }

    fn object_origin(&self, value: &SyntaxElement, depth: usize) -> Option<TextRange> {
        if depth >= 32 {
            return None;
        }
        if let Some(binding) = value
            .as_token()
            .and_then(|token| read_binding(self.model, token))
        {
            return self.object_origin(&stable_value(self.model, self.root, binding)?, depth + 1);
        }
        Some(value.text_range())
    }

    fn reference_cache(&self, receiver: &mut ClassReceiver, value: &SyntaxElement) {
        let Some(origin) = self.object_origin(value, 0) else {
            return;
        };
        for binary in self.root.descendants().filter_map(BinaryExpr::cast) {
            if binary.op_kind() != Some(SyntaxKind::DOLLAR)
                || binary.syntax().text_range().start() >= value.text_range().start()
            {
                continue;
            }
            let (Some(lhs), Some(rhs)) = (binary.lhs(), binary.rhs()) else {
                continue;
            };
            if self.object_origin(&lhs, 0) != Some(origin) {
                continue;
            }
            if binary
                .syntax()
                .ancestors()
                .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
            {
                continue;
            }
            let Some(name) = rhs
                .as_token()
                .map(|token| unbacktick(token.text()).to_string())
            else {
                continue;
            };
            if binary
                .syntax()
                .parent()
                .and_then(AssignmentExpr::cast)
                .is_some_and(|assignment| {
                    assignment
                        .target_element()
                        .is_some_and(|target| target.text_range() == binary.syntax().text_range())
                })
            {
                continue;
            }
            let eager = unconditional(binary.syntax())
                || binary
                    .syntax()
                    .parent()
                    .and_then(CallExpr::cast)
                    .is_some_and(|call| unconditional(call.syntax()));
            if !eager {
                let previous = self
                    .index
                    .reference_api_at(&receiver.class, self.point(&lhs).as_ref());
                let current = self
                    .index
                    .reference_api_at(&receiver.class, receiver.point.as_ref());
                if previous
                    .as_ref()
                    .and_then(|api| api.instance.entries.get(&name))
                    != current
                        .as_ref()
                        .and_then(|api| api.instance.entries.get(&name))
                {
                    receiver.uncertain_cache = true;
                }
                continue;
            }
            if let Some(point) = self.point(&lhs) {
                receiver.cached_members.entry(name).or_insert(point);
            }
        }
    }

    fn declared_instances(
        &self,
        classes: impl IntoIterator<Item = ClassId>,
        mut unknown: bool,
        point: Option<OopPoint>,
    ) -> ReceiverEvidence {
        let mut classes = classes.into_iter();
        let mut seen = classes
            .by_ref()
            .take(MAX_RECEIVER_CANDIDATES)
            .collect::<BTreeSet<_>>();
        unknown |= classes.next().is_some();
        let mut pending = seen.iter().cloned().collect::<Vec<_>>();
        let mut candidates = BTreeSet::new();
        while let Some(class) = pending.pop() {
            let Some(union) = self
                .index
                .declarations
                .get(&class)
                .and_then(|class| class.declaration.s4.as_ref())
                .filter(|definition| definition.definition == S4Definition::Union)
            else {
                candidates.insert(class);
                continue;
            };
            let mut enqueue = |member: ClassId| {
                if seen.contains(&member) {
                    return true;
                }
                if seen.len() >= MAX_RECEIVER_CANDIDATES {
                    return false;
                }
                seen.insert(member.clone());
                pending.push(member);
                true
            };
            // The graph includes registered extensions as well as explicit members.
            if let Some(extensions) = self.index.subtypes.get(&class) {
                unknown |= !extensions.iter().cloned().all(&mut enqueue);
            }
            if union.validity
                || !union.parents_complete
                || union.union_members.is_empty()
                || union.schema_helpers.iter().any(|helper| {
                    self.index
                        .callee_identity(&class.path, helper)
                        .is_none_or(|identity| identity.package != "base")
                })
            {
                unknown = true;
                continue;
            }
            unknown |= union.union_members.len() > MAX_RECEIVER_CANDIDATES;
            for name in union.union_members.iter().take(MAX_RECEIVER_CANDIDATES) {
                let ClassName::Known(name) = name else {
                    unknown = true;
                    continue;
                };
                let members = self.index.resolve_reference(
                    &class.path,
                    &class.key.owner,
                    &ParentRef::Formal(name.clone()),
                    true,
                );
                unknown |= members.is_empty() || !members.into_iter().all(&mut enqueue);
            }
        }
        unknown |= candidates.is_empty();
        let mut evidence = ReceiverEvidence::instances(candidates, unknown, point);
        for receiver in &mut evidence.candidates {
            receiver.exact = false;
        }
        evidence
    }

    fn member_receiver(
        &self,
        binary: &BinaryExpr,
        receiver: ClassReceiver,
        value: &SyntaxElement,
    ) -> Option<ReceiverEvidence> {
        let token = binary.rhs()?.into_token()?;
        let name = if token.kind() == SyntaxKind::STRING {
            crate::semantic::decode_string_literal(token.text())?
        } else {
            unbacktick(token.text()).to_owned()
        };
        let class = match binary.op_kind()? {
            SyntaxKind::AT if receiver.class.key.system != ClassSystem::S7 => {
                self.index.s4_slot_class(&receiver, &name)?
            }
            SyntaxKind::AT => {
                let (classes, unknown) = self.index.s7_property_classes(&receiver, &name)?;
                return Some(self.declared_instances(classes, unknown, self.point(value)));
            }
            SyntaxKind::DOLLAR => self.index.reference_field_class(&receiver, &name)?,
            _ => return None,
        };
        Some(self.declared_instances([class], false, self.point(value)))
    }

    fn property_receiver(
        &self,
        property: &s7::PropertyCall,
        mut receiver: ClassReceiver,
    ) -> Option<ReceiverEvidence> {
        let parent = self.index.s7_s4_parents.get(&receiver.class);
        if receiver.access == ClassAccess::Generator
            || (receiver.class.key.system != ClassSystem::S7 && parent.is_none())
        {
            return None;
        }
        if property.write {
            return Some(ReceiverEvidence {
                candidates: vec![receiver],
                unknown: false,
            });
        }
        if !property.single || !property.complete {
            return None;
        }
        let [name] = property.names.as_deref()? else {
            return None;
        };
        if let Some(parent) = parent {
            receiver.class = parent.clone();
        }
        let (classes, unknown) = self.index.s7_property_classes(&receiver, name)?;
        Some(self.declared_instances(classes, unknown, receiver.point))
    }

    fn construction(
        &self,
        receiver: ClassReceiver,
        value: &SyntaxElement,
        member_new: bool,
    ) -> Option<ClassReceiver> {
        if receiver.access != ClassAccess::Generator {
            return None;
        }
        let constructible = match (receiver.class.key.system, member_new) {
            (ClassSystem::R6, true) | (ClassSystem::RefClass, _) => true,
            (ClassSystem::S4, false) => self.index.s4_constructible(&receiver.class),
            (ClassSystem::S7, false) => {
                self.index.s7_constructible(&receiver.class)
                    && self.s7_constructor_returns_class(&receiver.class)
            }
            _ => false,
        };
        constructible.then(|| ClassReceiver {
            exact: receiver.exact,
            ..ClassReceiver::new(receiver.class, ClassAccess::Public, self.point(value))
        })
    }

    fn parameter_evidence(&self, binding: BindingId) -> ReceiverEvidence {
        if self.model.binding(binding).kind != BindingKind::Param
            || self.model.variable_cohort(binding).len() != 1
        {
            return ReceiverEvidence {
                candidates: Vec::new(),
                unknown: true,
            };
        }
        let binding = self.model.binding(binding);
        let (mut classes, unknown) =
            self.index
                .s4_parameter_classes(self.path, self.root, binding, self.extracted);
        let (s7_classes, s7_unknown) = self.index.s7_parameter_classes(
            self.path,
            self.root,
            binding,
            &self.extracted.method_implementations,
        );
        classes.extend(s7_classes);
        let unknown = unknown || s7_unknown || classes.is_empty();
        self.declared_instances(classes, unknown, None)
    }

    fn evidence(&self, value: &SyntaxElement, budget: &mut usize) -> ReceiverEvidence {
        if *budget == 0 {
            return ReceiverEvidence {
                candidates: Vec::new(),
                unknown: true,
            };
        }
        *budget -= 1;
        if let Some(candidate) = self.resolve(value, &mut BTreeSet::new(), 0) {
            return ReceiverEvidence {
                candidates: vec![candidate],
                unknown: false,
            };
        }
        let unknown = || ReceiverEvidence {
            candidates: Vec::new(),
            unknown: true,
        };
        if let Some(branch) = value.as_node().cloned().and_then(crate::ast::IfExpr::cast) {
            let mut evidence = ReceiverEvidence::default();
            for elements in [branch.then_elements(), branch.else_elements()] {
                let candidate = elements.into_iter().flatten().find(|element| {
                    !crate::ast::kinds::is_trivia(element.kind())
                        && element.kind() != SyntaxKind::COMMENT
                });
                evidence
                    .merge(candidate.map_or_else(unknown, |value| self.evidence(&value, budget)));
            }
            return evidence;
        }
        if let Some(paren) = value
            .as_node()
            .cloned()
            .and_then(crate::ast::ParenExpr::cast)
        {
            return paren
                .inner()
                .map_or_else(unknown, |inner| self.evidence(&inner, budget));
        }
        if let Some(block) = value
            .as_node()
            .cloned()
            .and_then(crate::ast::BlockExpr::cast)
        {
            let mut statements = block.statements();
            if let Some(statement) = statements.next()
                && statements.next().is_none()
            {
                return self.evidence(&statement, budget);
            }
        }
        if let Some(binary) = value.as_node().cloned().and_then(BinaryExpr::cast)
            && matches!(binary.op_kind(), Some(SyntaxKind::AT | SyntaxKind::DOLLAR))
            && let Some(lhs) = binary.lhs()
        {
            return self
                .evidence(&lhs, budget)
                .project_many(|receiver| self.member_receiver(&binary, receiver, value));
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast)
            && let Some(base) = call.base()
        {
            if let Some(property) = self
                .index
                .s7_property_call(self.path, self.root, self.model, &call)
            {
                return self
                    .evidence(&property.object, budget)
                    .project_many(|receiver| self.property_receiver(&property, receiver));
            }
            if let Some((object, name, replacement)) =
                s4::slot_call(self.index, self.path, self.root, self.model, &call)
            {
                let receivers = self.evidence(&object, budget);
                if replacement {
                    return receivers;
                }
                return receivers.project_many(|receiver| {
                    let class = self.index.s4_slot_class(&receiver, &name)?;
                    Some(self.declared_instances([class], false, self.point(value)))
                });
            }
            if let Some((function, result)) = self.local_return(&base) {
                let Some(frame) = self.enter_factory(function, value) else {
                    return unknown();
                };
                return ReceiverLookup {
                    factory: Some(&frame),
                    ..*self
                }
                .evidence(&result, budget);
            }
            if let Some(generator) = member_new_generator(&base) {
                return self
                    .evidence(&generator, budget)
                    .project(|receiver| self.construction(receiver, value, true));
            }
            return self
                .evidence(&base, budget)
                .project(|receiver| self.construction(receiver, value, false));
        }
        if let Some(token) = value.as_token()
            && let Some(evidence) = self.contextual_receiver(value, token)
        {
            return evidence;
        }
        if let Some(token) = value
            .as_token()
            .filter(|token| token.kind() == SyntaxKind::IDENT)
            && let Some(binding) = read_binding(self.model, token)
        {
            if self.model.binding(binding).kind == BindingKind::Param {
                return self.parameter_evidence(binding);
            }
            let definition = self.model.binding(binding).def_range;
            let read_offset = self.binding_read_offset(value, definition);
            if definition.start() >= read_offset {
                return unknown();
            }
            let Some(rhs) = stable_value(self.model, self.root, binding) else {
                return unknown();
            };
            let mut evidence = self.evidence(&rhs, budget);
            for receiver in &mut evidence.candidates {
                if receiver.class.key.system == ClassSystem::RefClass
                    && read_offset == token.text_range().start()
                {
                    receiver.point = self.point(value);
                    self.reference_cache(receiver, value);
                }
            }
            return evidence;
        }
        unknown()
    }

    fn contextual_receiver(
        &self,
        value: &SyntaxElement,
        token: &SyntaxToken,
    ) -> Option<ReceiverEvidence> {
        let name = unbacktick(token.text());
        let (system, access) = match name {
            "self" => (ClassSystem::R6, ClassAccess::Public),
            "private" => (ClassSystem::R6, ClassAccess::Private),
            "super" => (ClassSystem::R6, ClassAccess::Super),
            ".self" => (ClassSystem::RefClass, ClassAccess::Public),
            _ => return None,
        };
        let Self {
            index,
            path,
            root,
            model,
            extracted,
            ..
        } = *self;
        let implementations = &extracted.method_implementations;
        let contextual = apply_method_environments(model, root, &index.method_environments(path));
        let runtime = contextual.as_ref().unwrap_or(model);
        let ident = runtime
            .idents()
            .iter()
            .find(|ident| ident.range == token.text_range())?;
        let resolution = runtime.identifier_resolution(ident);
        let external_self = name == ".self"
            && matches!(resolution.lexical.as_slice(), [binding]
            if runtime.binding(*binding).kind == BindingKind::Param
            && runtime.variable_cohort(*binding).len() == 1
            && implementations.iter().any(|site| {
                site.function_range.contains_range(token.text_range())
                    && index.file(path).is_some_and(|file| file.facts.oop.members.iter().any(|member| member.kind == MemberKind::Method && member.method.as_ref() == Some(&site.method)))
                    && root.covering_element(site.function_range).into_node().and_then(FunctionExpr::cast)
                        .and_then(|function| function.formals().first().map(|formal| formal.name_token().text_range()))
                        == Some(runtime.binding(*binding).def_range)
            }));
        if !external_self
            && (!resolution.lexical.is_empty()
                || resolution.generated.is_empty()
                || resolution.external
                || resolution.uncertain)
        {
            return None;
        }
        let sites = implementations
            .iter()
            .filter(|site| site.function_range.contains_range(token.text_range()));
        let innermost = sites.clone().map(|site| site.function_range.len()).min()?;
        let mut classes = BTreeSet::new();
        let mut unknown = false;
        for site in sites.filter(|site| site.function_range.len() == innermost) {
            match index.method_class(path, &site.method) {
                Some(class) if class.key.system == system => {
                    classes.insert(class);
                }
                _ => unknown = true,
            }
        }
        if classes.is_empty() {
            return None;
        }
        let point = (system == ClassSystem::RefClass)
            .then(|| self.point(value))
            .flatten();
        let mut evidence = ReceiverEvidence::instances(classes, unknown, point);
        for candidate in &mut evidence.candidates {
            candidate.access = access;
        }
        Some(evidence)
    }

    fn resolve(
        &self,
        value: &SyntaxElement,
        visiting: &mut BTreeSet<u32>,
        depth: u32,
    ) -> Option<ClassReceiver> {
        if depth >= 64 {
            return None;
        }
        let Self {
            index,
            path,
            root,
            model,
            keys,
            extracted,
            ..
        } = *self;
        let implementations = &extracted.method_implementations;
        if let Some(paren) = value
            .as_node()
            .cloned()
            .and_then(crate::ast::ParenExpr::cast)
        {
            return self.resolve(&paren.inner()?, visiting, depth + 1);
        }
        if let Some(binary) = value.as_node().cloned().and_then(BinaryExpr::cast)
            && matches!(binary.op_kind(), Some(SyntaxKind::AT | SyntaxKind::DOLLAR))
        {
            let receiver = self.resolve(&binary.lhs()?, visiting, depth + 1)?;
            return self.member_receiver(&binary, receiver, value)?.unique();
        }
        // Enclosing frames and deferred promises can supply later bindings.
        // Reads in the current execution frame still follow source order.
        let deferred = model
            .idents()
            .iter()
            .find(|ident| ident.range == value.text_range())
            .is_some_and(|ident| {
                ident.deferred
                    || model.resolve_local(ident).is_some_and(|binding| {
                        model.frame_scope(ident.scope)
                            != model.frame_scope(model.binding(binding).scope)
                    })
            });
        let reference = generator_ref(model, root, value, keys, &mut BTreeSet::new(), deferred);
        if reference != GeneratorRef::Unknown {
            let owner = ClassOwner::default();
            let candidates = index.resolve_reference(
                path,
                &owner,
                &ParentRef::Generator(reference),
                value.parent().is_some_and(|node| {
                    node.ancestors()
                        .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
                }),
            );
            if let [id] = candidates.as_slice()
                && index.declarations.get(id).is_some_and(|class| {
                    matches!(
                        class.declaration.system(),
                        ClassSystem::R6 | ClassSystem::RefClass | ClassSystem::S4 | ClassSystem::S7
                    ) && class.declaration.binding_known()
                        && class.declaration.returns_generator()
                })
            {
                return Some(ClassReceiver::new(
                    id.clone(),
                    ClassAccess::Generator,
                    self.point(value),
                ));
            }
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast)
            && let Some(generator) = call.base().as_ref().and_then(member_new_generator)
        {
            let receiver = self.resolve(&generator, visiting, depth + 1)?;
            return self.construction(receiver, value, true);
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast) {
            if let Some(class) = index.s7_new_object_in(path, root, model, &call, implementations) {
                return Some(ClassReceiver::new(
                    class,
                    ClassAccess::Public,
                    self.point(value),
                ));
            }
            if let Some(property) = index.s7_property_call(path, root, model, &call) {
                let receiver = self.resolve(&property.object, visiting, depth + 1)?;
                return self.property_receiver(&property, receiver)?.unique();
            }
            if let Some((object, name, replacement)) =
                s4::slot_call(index, path, root, model, &call)
            {
                let receiver = self.resolve(&object, visiting, depth + 1)?;
                if replacement {
                    return Some(receiver);
                }
                let class = index.s4_slot_class(&receiver, &name)?;
                return self
                    .declared_instances([class], false, self.point(value))
                    .unique();
            }
            if let Some(reference) = call_function_ref(model, root, &call)
                && let Some(identity) = index.callee_identity(path, &reference)
                && identity.package == "methods"
            {
                let class = if identity.name == "new" {
                    let args = matched_args(&call, &["Class", "..."])?;
                    Some(index.s4_named_class(path, root, model, &argument(&args, "Class")?)?)
                } else if identity.name == "as" {
                    let args = matched_args(&call, &["object", "Class", "strict", "ext"])?;
                    Some(index.s4_named_class(path, root, model, &argument(&args, "Class")?)?)
                } else if identity.name == "as<-" {
                    let args = matched_args(&call, &["object", "Class", "value"])?;
                    return self.resolve(&argument(&args, "object")?, visiting, depth + 1);
                } else {
                    None
                };
                if let Some(class) = class {
                    if identity.name == "new"
                        && class.key.system == ClassSystem::S4
                        && !index.s4_constructible(&class)
                    {
                        return None;
                    }
                    return Some(ClassReceiver::new(
                        class,
                        ClassAccess::Public,
                        self.point(value),
                    ));
                }
            }
            if let Some(class) = index.reference_lookup(path, root, model, &call) {
                return Some(ClassReceiver::new(
                    class,
                    ClassAccess::Generator,
                    self.point(value),
                ));
            }
            if let Some(binary) = call
                .base()
                .and_then(SyntaxElement::into_node)
                .and_then(BinaryExpr::cast)
                && binary.op_kind() == Some(SyntaxKind::DOLLAR)
                && let Some(SyntaxElement::Token(name)) = binary.rhs()
                && matches!(unbacktick(name.text()), "copy" | "getRefClass")
            {
                let receiver = self.resolve(&binary.lhs()?, visiting, depth + 1)?;
                if receiver.class.key.system == ClassSystem::RefClass
                    && receiver.access == ClassAccess::Public
                {
                    let name = unbacktick(name.text());
                    let api = index.reference_receiver_api(&receiver)?;
                    let [member] = api.instance.entries.get(name)?.as_slice() else {
                        return None;
                    };
                    if member.declaration.is_none()
                        && (name == "copy" || call.arg_list()?.args().next().is_none())
                    {
                        return Some(ClassReceiver {
                            exact: receiver.exact,
                            ..ClassReceiver::new(
                                receiver.class,
                                if name == "copy" {
                                    ClassAccess::Public
                                } else {
                                    ClassAccess::Generator
                                },
                                self.point(value),
                            )
                        });
                    }
                }
                return None;
            }
            let base = call.base()?;
            if let Some((function, result)) = self.local_return(&base) {
                let frame = self.enter_factory(function, value)?;
                return ReceiverLookup {
                    factory: Some(&frame),
                    ..*self
                }
                .resolve(&result, visiting, depth + 1);
            }
            let receiver = self.resolve(&base, visiting, depth + 1)?;
            return self.construction(receiver, value, false);
        }
        let token = value
            .as_token()
            .filter(|token| token.kind() == SyntaxKind::IDENT)?;
        if let Some(evidence) = self.contextual_receiver(value, token) {
            return evidence.unique();
        }
        let binding = read_binding(model, token)?;
        if model.binding(binding).kind == BindingKind::Param {
            return self.parameter_evidence(binding).unique();
        }
        let definition = model.binding(binding).def_range;
        let read_offset = self.binding_read_offset(value, definition);
        if visiting.len() >= 32 || !visiting.insert(binding.0) || definition.start() >= read_offset
        {
            return None;
        }
        let mut receiver =
            self.resolve(&stable_value(model, root, binding)?, visiting, depth + 1)?;
        if receiver.class.key.system == ClassSystem::RefClass
            && read_offset == token.text_range().start()
        {
            receiver.point = self.point(value);
            self.reference_cache(&mut receiver, value);
        }
        Some(receiver)
    }
}

fn enclosing_function(value: &SyntaxElement) -> Option<TextRange> {
    value
        .parent()?
        .ancestors()
        .find(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
        .map(|node| node.text_range())
}

fn member_new_generator(base: &SyntaxElement) -> Option<SyntaxElement> {
    let member = base.as_node().cloned().and_then(BinaryExpr::cast)?;
    let name = member.rhs()?.into_token()?;
    if member.op_kind() != Some(SyntaxKind::DOLLAR) || unbacktick(name.text()) != "new" {
        return None;
    }
    member.lhs()
}
