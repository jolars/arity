//! Bounded receiver evidence shared by R6 and Reference Classes.

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
                let api = self.r6_api(&receiver.class)?;
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
            let api = self.reference_api(&receiver.class)?;
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
                self.reference_api(&receiver.class)?.initializer?,
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
        let class = ClassId {
            path: path.into(),
            key: method.class.clone()?,
        };
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
                        ClassSystem::R6 | ClassSystem::RefClass
                    ) && class.declaration.binding_known()
                })
            {
                return Some(ClassReceiver {
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
            return (receiver.access == ClassAccess::Generator).then_some(ClassReceiver {
                class: receiver.class,
                access: ClassAccess::Public,
            });
        }
        if let Some(call) = value.as_node().cloned().and_then(CallExpr::cast) {
            if let Some(class) = index.reference_lookup(path, root, model, &call) {
                return Some(ClassReceiver {
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
                    let api = index.reference_api(&receiver.class)?;
                    let [member] = api.instance.entries.get(name)?.as_slice() else {
                        return None;
                    };
                    if member.declaration.is_none()
                        && (name == "copy" || call.arg_list()?.args().next().is_none())
                    {
                        return Some(ClassReceiver {
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
            return (receiver.class.key.system == ClassSystem::RefClass
                && receiver.access == ClassAccess::Generator)
                .then_some(ClassReceiver {
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
                        .filter_map(|site| {
                            index
                                .file(path)?
                                .facts
                                .oop
                                .methods
                                .iter()
                                .find(|method| method.key == site.method)?
                                .class
                                .clone()
                        })
                        .collect::<BTreeSet<_>>();
                    candidates.retain(|key| {
                        key.system
                            == if name == ".self" {
                                ClassSystem::RefClass
                            } else {
                                ClassSystem::R6
                            }
                    });
                    if candidates.len() == 1 {
                        return Some(ClassReceiver {
                            class: ClassId {
                                path: path.into(),
                                key: candidates.pop_first()?,
                            },
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
        self.resolve(&stable_value(model, root, binding)?, visiting, depth + 1)
    }
}
