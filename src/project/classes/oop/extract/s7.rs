use super::*;

impl Projection<'_> {
    pub(super) fn s7_function(
        &mut self,
        node: &SyntaxNode,
        value: Option<SyntaxElement>,
    ) -> S7Function {
        let Some(value) = value.filter(|value| !constant_atom(value, "NULL")) else {
            return S7Function::Absent;
        };
        let key = self.key(node, OopSystem::S7, OopKind::S7Function);
        let signature = self.implementation_signature(Some(value.clone()), &mut BTreeSet::new());
        self.implementation(&key, Some(value), true);
        match signature {
            Some(signature) => S7Function::Known {
                key: Some(key),
                signature,
            },
            None => S7Function::Unknown,
        }
    }

    pub(super) fn s7_class(&mut self, def: &ClassDef, call: &CallExpr, args: &MatchedArguments) {
        let package = match argument(args, "package") {
            None => S7Package::Enclosing,
            Some(value) if constant_atom(&value, "NULL") => S7Package::None,
            value => match string_name(value) {
                ClassName::Known(package) => S7Package::Named(package),
                _ => S7Package::Unknown,
            },
        };
        let parent = argument(args, "parent").map(|parent| self.s7_spec(&parent, 0));
        let abstract_class = match argument(args, "abstract") {
            None => Some(false),
            Some(value) if constant_atom(&value, "FALSE") => Some(false),
            Some(value) if constant_atom(&value, "TRUE") => Some(true),
            _ => None,
        };
        let constructor = self.s7_function(call.syntax(), argument(args, "constructor"));
        let validator = self.s7_function(call.syntax(), argument(args, "validator"));
        self.facts.s7.classes.insert(
            def.key.clone(),
            S7ClassDef {
                package,
                parent,
                abstract_class,
                constructor,
                validator,
            },
        );
    }

    pub(super) fn s7_property(
        &mut self,
        node: &SyntaxNode,
        entry: &mut MemberEntry,
    ) -> S7PropertyDef {
        let value = entry.value.clone().and_then(|value| {
            self.implementation_value(Some(value.clone()), &mut BTreeSet::new(), false)
                .or(Some(value))
        });
        let property = value.as_ref().and_then(|value| {
            let call = CallExpr::cast(value.as_node()?.clone())?;
            let (reference, name) = factory(self.model, self.root, &call)?;
            (name == "new_property" && accepts(&reference, "S7")).then_some((call, reference))
        });
        let Some((property, reference)) = property else {
            return S7PropertyDef {
                class: value.as_ref().map(|value| self.s7_spec(value, 0)),
                default: None,
                getter: S7Function::Absent,
                setter: S7Function::Absent,
                validator: S7Function::Absent,
                factory: None,
                complete: value.is_some(),
            };
        };
        self.facts.s7.helpers.insert(reference.clone());
        let args = matched_args(
            &property,
            &["class", "getter", "setter", "validator", "default", "name"],
        );
        let get = |name| args.as_ref().and_then(|args| argument(args, name));
        if entry.name.is_none() && args.is_some() {
            if let Some(value) = get("name") {
                if let ClassName::Known(name) = string_name(Some(value.clone())) {
                    entry.name = Some(name);
                    entry.range = value.text_range();
                }
            } else if let Some(target) = s7_named_target(self.model, property.syntax()) {
                entry.name = Some(unbacktick(target.text()).to_string());
                entry.range = target.text_range();
            }
        }
        let class = get("class").map(|class| self.s7_spec(&class, 0));
        let default = get("default")
            .filter(|value| !constant_atom(value, "NULL"))
            .map(|value| value.to_string());
        let getter = self.s7_function(node, get("getter"));
        let setter = self.s7_function(node, get("setter"));
        let validator = self.s7_function(node, get("validator"));
        S7PropertyDef {
            class,
            default,
            getter,
            setter,
            validator,
            factory: Some(reference),
            complete: args.is_some(),
        }
    }

    pub(super) fn s7_spec(&mut self, value: &SyntaxElement, depth: usize) -> S7ClassSpec {
        // Repeated aliases can expand the same union exponentially even
        // within the depth limit. Give the entire specification one budget.
        let mut budget = 256;
        let spec = self.s7_spec_bounded(value, depth, &mut budget);
        if budget == 0 {
            S7ClassSpec::Unknown
        } else {
            spec
        }
    }

    fn s7_spec_bounded(
        &mut self,
        value: &SyntaxElement,
        depth: usize,
        budget: &mut usize,
    ) -> S7ClassSpec {
        if *budget == 0 {
            return S7ClassSpec::Unknown;
        }
        *budget -= 1;
        if depth >= 32 {
            return S7ClassSpec::Unknown;
        }
        let resolved = self.implementation_value(Some(value.clone()), &mut BTreeSet::new(), false);
        let expr = resolved.as_ref().unwrap_or(value);
        if constant_atom(expr, "NULL") {
            return S7ClassSpec::Null;
        }
        if let Some(reference) = function_ref(self.model, self.root, expr, &mut BTreeSet::new())
            && possible_functions(&reference)
                .iter()
                .any(|identity| identity.package == "S7" && is_builtin(&identity.name))
        {
            self.facts.s7.helpers.insert(reference.clone());
            return S7ClassSpec::Builtin(reference);
        }
        if let Some(binary) = expr.as_node().cloned().and_then(BinaryExpr::cast)
            && binary.op_kind() == Some(SyntaxKind::OR)
            && let Some(operator) = binary.syntax().children_with_tokens().find_map(|element| {
                element
                    .into_token()
                    .filter(|token| token.kind() == SyntaxKind::OR)
            })
            && let Some(reference) =
                named_function_ref(self.model, self.root, &operator, "|", &mut BTreeSet::new())
            && accepts(&reference, "base")
            && let (Some(lhs), Some(rhs)) = (binary.lhs(), binary.rhs())
        {
            self.facts.s7.helpers.insert(reference.clone());
            return S7ClassSpec::Union {
                factory: reference,
                classes: vec![
                    self.s7_spec_bounded(&lhs, depth + 1, budget),
                    self.s7_spec_bounded(&rhs, depth + 1, budget),
                ],
            };
        }
        if let Some(call) = expr.as_node().cloned().and_then(CallExpr::cast)
            && let Some((reference, name)) = factory(self.model, self.root, &call)
        {
            let result = self.s7_call_spec(&call, &reference, &name, depth, budget);
            if let Some(spec) = result {
                self.facts.s7.helpers.insert(reference);
                return spec;
            }
        }
        match self.generator(value) {
            GeneratorRef::Unknown => S7ClassSpec::Unknown,
            reference => S7ClassSpec::Generator(reference),
        }
    }

    fn s7_call_spec(
        &mut self,
        call: &CallExpr,
        reference: &FunctionRef,
        name: &str,
        depth: usize,
        budget: &mut usize,
    ) -> Option<S7ClassSpec> {
        let strings = |args: &MatchedArguments, name| match string_name(argument(args, name)) {
            ClassName::Known(value) => Some(value),
            _ => None,
        };
        Some(match name {
            "new_union" if accepts(reference, "S7") => S7ClassSpec::Union {
                factory: reference.clone(),
                classes: {
                    let mut classes = Vec::new();
                    for arg in call.arg_list()?.args() {
                        if *budget == 0 {
                            return Some(S7ClassSpec::Unknown);
                        }
                        classes.push(match arg.value() {
                            Some(value) => self.s7_spec_bounded(&value, depth + 1, budget),
                            None => {
                                *budget -= 1;
                                S7ClassSpec::Unknown
                            }
                        });
                    }
                    classes
                },
            },
            "new_S3_class" if accepts(reference, "S7") => {
                let formals = if self.model.s7_version == S7Version::Development {
                    &["class", "constructor", "validator", "default", "attributes"][..]
                } else {
                    &["class", "constructor", "validator"][..]
                };
                let args = matched_args(call, formals)?;
                let vector = self.string_vector(&argument(&args, "class")?)?;
                let classes = vector
                    .entries
                    .into_iter()
                    .map(|(_, name, _)| match name {
                        ClassName::Known(name) => Some(name),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                if let Some(helper) = &vector.factory {
                    self.facts.s7.helpers.insert(helper.clone());
                }
                S7ClassSpec::S3 {
                    factory: reference.clone(),
                    classes,
                    constructor: self.s7_function(call.syntax(), argument(&args, "constructor")),
                    default: argument(&args, "default").map(|expr| expr.to_string()),
                    vector: vector.factory,
                }
            }
            "new_external_class" if accepts(reference, "S7") => {
                let args = matched_args(call, &["package", "name", "version"])?;
                if argument(&args, "version").is_some_and(|value| {
                    !constant_atom(&value, "NULL") && strings(&args, "version").is_none()
                }) {
                    return Some(S7ClassSpec::Unknown);
                }
                S7ClassSpec::External {
                    factory: reference.clone(),
                    package: strings(&args, "package")?,
                    name: strings(&args, "name")?,
                    version: strings(&args, "version"),
                }
            }
            "getClass" | "getClassDef" if accepts(reference, "methods") => {
                let formals = if name == "getClass" {
                    &["Class", ".Force", "where", "package"][..]
                } else {
                    &["Class", "where", "package", "inherits"][..]
                };
                let args = matched_args(call, formals)?;
                if args.contains_key("where") {
                    return Some(S7ClassSpec::Unknown);
                }
                S7ClassSpec::S4 {
                    factory: reference.clone(),
                    name: strings(&args, "Class")?,
                    package: strings(&args, "package"),
                }
            }
            _ => return None,
        })
    }
}

impl Projection<'_> {
    pub(super) fn s7_generic_shape(
        &mut self,
        key: &OopKey,
        value: Option<SyntaxElement>,
    ) -> S7GenericDef {
        let Some(value) = value else {
            return S7GenericDef::Generated;
        };
        self.implementation(key, Some(value.clone()), true);
        let dispatch = self
            .implementation_value(Some(value), &mut BTreeSet::new(), false)
            .and_then(SyntaxElement::into_node)
            .and_then(FunctionExpr::cast)
            .and_then(|function| function.body())
            .and_then(SyntaxElement::into_node)
            .and_then(|body| {
                if body.kind() != SyntaxKind::BLOCK_EXPR {
                    return CallExpr::cast(body);
                }
                let mut expressions = body.children_with_tokens().filter(|element| {
                    !matches!(
                        element.kind(),
                        SyntaxKind::LBRACE | SyntaxKind::RBRACE | SyntaxKind::COMMENT
                    ) && !crate::ast::kinds::is_trivia(element.kind())
                });
                let call = CallExpr::cast(expressions.next()?.into_node()?)?;
                expressions.next().is_none().then_some(call)
            })
            .filter(|call| {
                call.arg_list()
                    .is_some_and(|args| args.args().next().is_none())
            })
            .and_then(|call| call_function_ref(self.model, self.root, &call))
            .filter(|reference| {
                possible_functions(reference)
                    .iter()
                    .any(|origin| origin.package == "S7" && origin.name == "S7_dispatch")
            });
        if let Some(dispatch) = dispatch {
            self.facts.s7.helpers.insert(dispatch.clone());
            S7GenericDef::Dispatch(dispatch)
        } else {
            S7GenericDef::Custom
        }
    }

    pub(super) fn s7_external_generic(
        &mut self,
        value: &SyntaxElement,
    ) -> Option<S7ExternalGeneric> {
        let call = CallExpr::cast(
            self.implementation_value(Some(value.clone()), &mut BTreeSet::new(), false)?
                .into_node()?,
        )?;
        let (reference, name) = factory(self.model, self.root, &call)?;
        if !accepts(&reference, "S7") || name != "new_external_generic" {
            return None;
        }
        let args = matched_args(&call, &["package", "name", "dispatch_args", "version"])?;
        let literal = |name| match string_name(argument(&args, name)) {
            ClassName::Known(name) => Some(name),
            _ => None,
        };
        if argument(&args, "version")
            .is_some_and(|value| literal("version").is_none() && !constant_atom(&value, "NULL"))
        {
            return None;
        }
        let vector = self.string_vector(&argument(&args, "dispatch_args")?)?;
        let dispatch_arguments = vector
            .entries
            .into_iter()
            .map(|(_, name, _)| match name {
                ClassName::Known(name) => Some(name),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        self.facts.s7.helpers.insert(reference.clone());
        self.facts.s7.helpers.extend(vector.factory.clone());
        Some(S7ExternalGeneric {
            factory: reference,
            name: literal("name")?,
            package: literal("package")?,
            version: literal("version"),
            dispatch_arguments,
            dispatch_factory: vector.factory,
        })
    }

    pub(super) fn s7_hooks(&mut self) {
        for (call, reference, name) in factories(self.model, self.root) {
            let (kind, required_frame) = match name.as_str() {
                "methods_register" | "S7_on_load" => (S7HookKind::Load, Some(".onLoad")),
                "S7_on_unload" => (S7HookKind::Unload, Some(".onUnload")),
                "S7_on_build" => (S7HookKind::Build, None),
                _ => continue,
            };
            if !accepts(&reference, "S7")
                || (S7Version::development_export(&name)
                    && self.model.s7_version == S7Version::Release)
                || call
                    .arg_list()
                    .is_none_or(|args| args.args().next().is_some())
            {
                continue;
            }
            let function = call.syntax().ancestors().find_map(FunctionExpr::cast);
            let owner = match &function {
                Some(function) => {
                    let Some(target) = assigned_binding(self.model, function.syntax()) else {
                        continue;
                    };
                    if !function
                        .syntax()
                        .ancestors()
                        .skip(1)
                        .all(|node| node.kind() != SyntaxKind::FUNCTION_EXPR)
                        || !declaration_eager(self.model, function.syntax())
                    {
                        continue;
                    }
                    Some(unbacktick(target.text()).to_string())
                }
                None => None,
            };
            if owner.as_deref() != required_frame {
                continue;
            }
            self.facts.s7.helpers.insert(reference.clone());
            let ordinal = self.facts.s7.hooks.len();
            self.facts.s7.hooks.push(S7Hook {
                kind,
                factory: reference,
                unconditional: unconditional(call.syntax()),
            });
            if kind == S7HookKind::Build {
                self.events
                    .push((call.syntax().text_range(), OopEvent::S7Hook(ordinal)));
            }
        }
    }
}

impl Projection<'_> {
    pub(super) fn s7_s4_contains(&mut self, def: &ClassDef, declaration: &CallExpr) {
        let Some(args) = matched_args(declaration, ClassSystem::S4.formals()) else {
            return;
        };
        let value = argument(&args, "contains")
            .and_then(|value| self.implementation_value(Some(value), &mut BTreeSet::new(), false));
        let Some(call) = value
            .and_then(SyntaxElement::into_node)
            .and_then(CallExpr::cast)
        else {
            return;
        };
        let Some((reference, name)) = factory(self.model, self.root, &call) else {
            return;
        };
        if !accepts(&reference, "S7") || name != "S4_contains" {
            return;
        }
        let Some(args) = matched_args(&call, &["class", "env"]) else {
            return;
        };
        let Some(class) = argument(&args, "class") else {
            return;
        };
        if args.contains_key("env") {
            return;
        }
        let class = self.s7_spec(&class, 0);
        self.facts.s7.helpers.insert(reference.clone());
        self.facts.s7.s4_contains.insert(
            def.key.clone(),
            S7S4Bridge {
                factory: reference,
                class,
                unconditional: unconditional(declaration.syntax()),
            },
        );
    }

    pub(super) fn s7_s4_registrations(&mut self) {
        for (call, reference, name) in factories(self.model, self.root) {
            if !accepts(&reference, "S7") || name != "S4_register" {
                continue;
            }
            let Some(args) = matched_args(&call, &["class", "env"]) else {
                continue;
            };
            let Some(class) = argument(&args, "class") else {
                continue;
            };
            let class = self.s7_spec(&class, 0);
            self.facts.s7.helpers.insert(reference.clone());
            let index = self.facts.s7.s4_registrations.len();
            self.facts.s7.s4_registrations.push(S7S4Bridge {
                factory: reference,
                class,
                unconditional: unconditional(call.syntax())
                    && !args.contains_key("env")
                    && owner_at(call.syntax(), self.owners).0.is_empty()
                    && !call
                        .syntax()
                        .ancestors()
                        .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR),
            });
            self.events
                .push((call.syntax().text_range(), OopEvent::S7S4Bridge(index)));
        }
    }
}
