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
    pub point: Option<OopPoint>,
    pub cached_members: BTreeMap<String, OopPoint>,
    pub uncertain_cache: bool,
}

impl ClassIndex {
    pub fn class_receiver(
        &self,
        path: &std::path::Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<ClassReceiver> {
        let extracted = extract(model, root, false);
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
            implementations: &extracted.method_implementations,
        }
        .resolve(value, &mut BTreeSet::new(), 0)
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
            let receiver = self.class_receiver(path, root, model, &binary.lhs()?)?;
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
        if let Some(receiver) = self.reference_receiver(path, root, model, &base)
            && receiver.access == ClassAccess::Generator
        {
            return Some((
                base.to_string(),
                self.reference_receiver_api(&receiver)?.initializer?,
            ));
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
    implementations: &'a [MethodImplementationSite],
}

impl ReceiverLookup<'_> {
    fn point(&self, value: &SyntaxElement) -> Option<OopPoint> {
        if value.parent().is_some_and(|node| {
            node.ancestors()
                .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
        }) {
            return None;
        }
        Some(oop_point(
            self.model,
            self.root,
            self.path,
            value.text_range().start(),
        ))
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
            implementations,
        } = *self;
        if let Some(paren) = value
            .as_node()
            .cloned()
            .and_then(crate::ast::ParenExpr::cast)
        {
            return self.resolve(&paren.inner()?, visiting, depth + 1);
        }
        if let Some(binary) = value.as_node().cloned().and_then(BinaryExpr::cast)
            && binary.op_kind() == Some(SyntaxKind::AT)
        {
            let receiver = self.resolve(&binary.lhs()?, visiting, depth + 1)?;
            let rhs = binary.rhs()?;
            let name = rhs.as_token().and_then(|token| {
                if token.kind() == SyntaxKind::STRING {
                    crate::semantic::decode_string_literal(token.text())
                } else {
                    Some(unbacktick(token.text()).to_string())
                }
            })?;
            return Some(ClassReceiver {
                class: index.s4_slot_class(&receiver, &name)?,
                access: ClassAccess::Public,
                point: self.point(value),
                cached_members: BTreeMap::new(),
                uncertain_cache: false,
            });
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
                        ClassSystem::R6 | ClassSystem::RefClass | ClassSystem::S4
                    ) && class.declaration.binding_known()
                        && class.declaration.returns_generator()
                })
            {
                return Some(ClassReceiver {
                    point: self.point(value),
                    cached_members: BTreeMap::new(),
                    uncertain_cache: false,
                    class: id.clone(),
                    access: ClassAccess::Generator,
                });
            }
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast)
            && let Some(binary) = call
                .base()
                .and_then(SyntaxElement::into_node)
                .and_then(BinaryExpr::cast)
            && binary.op_kind() == Some(SyntaxKind::DOLLAR)
            && let Some(SyntaxElement::Token(name)) = binary.rhs()
            && unbacktick(name.text()) == "new"
        {
            let receiver = self.resolve(&binary.lhs()?, visiting, depth + 1)?;
            return (receiver.access == ClassAccess::Generator
                && matches!(
                    receiver.class.key.system,
                    ClassSystem::R6 | ClassSystem::RefClass
                ))
            .then_some(ClassReceiver {
                point: self.point(value),
                cached_members: BTreeMap::new(),
                uncertain_cache: false,
                class: receiver.class,
                access: ClassAccess::Public,
            });
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast) {
            if let Some((object, name, replacement)) =
                s4::slot_call(index, path, root, model, &call)
            {
                let receiver = self.resolve(&object, visiting, depth + 1)?;
                if replacement {
                    return Some(receiver);
                }
                return Some(ClassReceiver {
                    class: index.s4_slot_class(&receiver, &name)?,
                    access: ClassAccess::Public,
                    point: self.point(value),
                    cached_members: BTreeMap::new(),
                    uncertain_cache: false,
                });
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
                    return Some(ClassReceiver {
                        class,
                        access: ClassAccess::Public,
                        point: self.point(value),
                        cached_members: BTreeMap::new(),
                        uncertain_cache: false,
                    });
                }
            }
            if let Some(class) = index.reference_lookup(path, root, model, &call) {
                return Some(ClassReceiver {
                    point: self.point(value),
                    cached_members: BTreeMap::new(),
                    uncertain_cache: false,
                    class,
                    access: ClassAccess::Generator,
                });
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
                            point: self.point(value),
                            cached_members: BTreeMap::new(),
                            uncertain_cache: false,
                            class: receiver.class,
                            access: if name == "copy" {
                                ClassAccess::Public
                            } else {
                                ClassAccess::Generator
                            },
                        });
                    }
                }
                return None;
            }
            let receiver = self.resolve(&call.base()?, visiting, depth + 1)?;
            return ((receiver.class.key.system == ClassSystem::RefClass
                || (receiver.class.key.system == ClassSystem::S4
                    && index.s4_constructible(&receiver.class)))
                && receiver.access == ClassAccess::Generator)
                .then_some(ClassReceiver {
                    point: self.point(value),
                    cached_members: BTreeMap::new(),
                    uncertain_cache: false,
                    class: receiver.class,
                    access: ClassAccess::Public,
                });
        }
        let token = value
            .as_token()
            .filter(|token| token.kind() == SyntaxKind::IDENT)?;
        let name = unbacktick(token.text());
        if matches!(name, "self" | "private" | "super" | ".self") {
            let contextual =
                apply_method_environments(model, root, &index.method_environments(path));
            let runtime = contextual.as_ref().unwrap_or(model);
            if let Some(ident) = runtime
                .idents()
                .iter()
                .find(|ident| ident.range == token.text_range())
            {
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
                if (resolution.lexical.is_empty()
                    && !resolution.generated.is_empty()
                    && !resolution.uncertain
                    && !resolution.external)
                    || external_self
                {
                    let innermost = implementations
                        .iter()
                        .filter(|site| site.function_range.contains_range(token.text_range()))
                        .map(|site| site.function_range.len())
                        .min();
                    let mut candidates = implementations
                        .iter()
                        .filter(|site| site.function_range.contains_range(token.text_range()))
                        .filter(|site| Some(site.function_range.len()) == innermost)
                        .filter_map(|site| index.method_class(path, &site.method))
                        .collect::<BTreeSet<_>>();
                    candidates.retain(|id| {
                        id.key.system
                            == if name == ".self" {
                                ClassSystem::RefClass
                            } else {
                                ClassSystem::R6
                            }
                    });
                    if candidates.len() == 1 {
                        return Some(ClassReceiver {
                            point: self.point(value),
                            cached_members: BTreeMap::new(),
                            uncertain_cache: false,
                            class: candidates.pop_first()?,
                            access: match name {
                                "private" => ClassAccess::Private,
                                "super" => ClassAccess::Super,
                                _ => ClassAccess::Public,
                            },
                        });
                    }
                }
            }
        }
        let binding = read_binding(model, token)?;
        if visiting.len() >= 32
            || !visiting.insert(binding.0)
            || model.binding(binding).def_range.start() >= token.text_range().start()
        {
            return None;
        }
        let mut receiver =
            self.resolve(&stable_value(model, root, binding)?, visiting, depth + 1)?;
        if receiver.class.key.system == ClassSystem::RefClass {
            receiver.point = self.point(value);
            self.reference_cache(&mut receiver, value);
        }
        Some(receiver)
    }
}
