use super::*;

impl Projection<'_> {
    pub(super) fn s4_call_sites(&mut self) -> Vec<S4CallSite> {
        let mut sites = Vec::new();
        for (call, callee, name) in factories(self.model, self.root) {
            let (kind, formals) = match name.as_str() {
                "standardGeneric" if accepts(&callee, "base") => {
                    (S4CallKind::StandardGeneric, &["f", "fdef"][..])
                }
                "callNextMethod" if accepts(&callee, "methods") => {
                    (S4CallKind::NextMethod, &["..."][..])
                }
                "selectMethod" if accepts(&callee, "methods") => (
                    S4CallKind::SelectMethod,
                    &[
                        "f",
                        "signature",
                        "optional",
                        "useInherited",
                        "mlist",
                        "fdef",
                        "verbose",
                        "doCache",
                    ][..],
                ),
                _ => continue,
            };
            let args = matched_args(&call, formals);
            let value = |name: &str| args.as_ref().and_then(|args| argument(args, name));
            let mut uncertainty = BTreeSet::new();
            // Dots in callNextMethod forward values, but never change selection.
            if args.is_none() && kind != S4CallKind::NextMethod {
                uncertainty.insert(S4CallUncertainty::InvalidArguments);
            }
            let owner = owner_at(call.syntax(), self.owners);
            let generic_value = value("f");
            let reference = (kind != S4CallKind::NextMethod).then(|| {
                self.generic_reference(generic_value.clone(), OopSystem::S4, &owner, false)
            });
            let function = call.syntax().ancestors().find_map(FunctionExpr::cast);
            let named_frame = function.as_ref().and_then(|function| {
                let assignment =
                    direct_assignment(&SyntaxElement::Node(function.syntax().clone()))?;
                let range = assignment.value_element()?.text_range();
                self.events
                    .iter()
                    .find_map(|(event_range, event)| match event {
                        OopEvent::Callable(index) if *event_range == range => Some(*index),
                        _ => None,
                    })
            });
            let mut frames: Vec<_> = self
                .method_implementations
                .iter()
                .filter(|site| {
                    function.as_ref().is_some_and(|function| {
                        function.syntax().text_range() == site.function_range
                    }) && site.method.system == OopSystem::S4
                })
                .map(|site| site.method.clone())
                .collect();
            frames.sort();
            frames.dedup();
            let default_frames = frames
                .iter()
                .filter(|key| {
                    self.s4_defaults.get(*key).is_some_and(|range| {
                        function
                            .as_ref()
                            .is_some_and(|function| function.syntax().text_range() == *range)
                    })
                })
                .cloned()
                .collect();
            let dispatch = if kind == S4CallKind::SelectMethod {
                if value("signature").is_none() {
                    uncertainty.insert(S4CallUncertainty::InvalidArguments);
                }
                self.dispatch(value("signature"), OopSystem::S4)
            } else {
                Dispatch {
                    arguments: Vec::new(),
                    factory: None,
                }
            };
            if value("mlist").is_some() || value("fdef").is_some() {
                uncertainty.insert(S4CallUncertainty::CustomMethodTable);
            }
            let inheritance = if kind == S4CallKind::SelectMethod {
                match value("useInherited")
                    .map(|value| value.to_string())
                    .as_deref()
                {
                    None => S4Inheritance::Default,
                    Some("TRUE") => S4Inheritance::Explicit(true),
                    Some("FALSE") => S4Inheritance::Explicit(false),
                    _ => S4Inheritance::Dynamic,
                }
            } else {
                S4Inheritance::Default
            };
            if let Some(value) = generic_value
                && let Some(reference) = &reference
            {
                self.site(
                    value.text_range(),
                    OopSiteRole::Use,
                    OopTarget::Generic(reference.clone()),
                );
            }
            sites.push(S4CallSite {
                range: call.syntax().text_range(),
                kind,
                callee,
                reference,
                frames,
                default_frames,
                named_frame,
                function: function.map(|function| function.syntax().text_range()),
                dispatch: dispatch.arguments.into_iter().map(|(arg, _)| arg).collect(),
                dispatch_factory: dispatch.factory,
                inheritance,
                uncertainty,
            });
        }
        sites
    }

    pub(super) fn s4_generic_definition(
        &self,
        args: &Option<MatchedArguments>,
        function: Option<SyntaxElement>,
        name: &ClassName,
    ) -> S4GenericDefinition {
        let value = |name: &str| args.as_ref().and_then(|args| argument(args, name));
        let definition = function
            .clone()
            .and_then(|value| self.supplied_implementation(value))
            .or_else(|| match name {
                ClassName::Known(name) => {
                    Some(MethodImplementationSource::Reference { name: name.clone() })
                }
                _ => None,
            });
        let previous = match name {
            ClassName::Known(name) => {
                Some(MethodImplementationSource::Reference { name: name.clone() })
            }
            _ => None,
        };
        let resolved = self
            .implementation_value(function.clone(), &mut BTreeSet::new(), false)
            .and_then(SyntaxElement::into_node)
            .and_then(FunctionExpr::cast);
        let standard_factory = resolved.as_ref().and_then(|function| {
            function
                .syntax()
                .descendants()
                .filter_map(CallExpr::cast)
                .filter(|call| {
                    call.syntax()
                        .ancestors()
                        .find_map(FunctionExpr::cast)
                        .as_ref()
                        == Some(function)
                })
                .find_map(|call| {
                    let reference = call_function_ref(self.model, self.root, &call)?;
                    possible_functions(&reference)
                        .iter()
                        .any(|identity| {
                            identity.package == "base" && identity.name == "standardGeneric"
                        })
                        .then_some(reference)
                })
        });
        // R's .NonstandardGenericTest checks the literal call head, independently
        // of its origin. Qualified calls and aliases can still become defaults.
        let standard_dispatch = resolved.as_ref().map(|function| {
            function
                .body()
                .and_then(SyntaxElement::into_node)
                .is_some_and(|body| {
                    body.descendants().filter_map(CallExpr::cast).any(|call| {
                        call.base()
                            .and_then(SyntaxElement::into_token)
                            .is_some_and(|token| {
                                token.kind() == SyntaxKind::IDENT
                                    && unbacktick(token.text()) == "standardGeneric"
                            })
                    })
                })
        });
        let supplied = value("useAsDefault");
        let no_default = supplied
            .as_ref()
            .is_some_and(|value| value.to_string() == "FALSE");
        let explicit =
            supplied.filter(|value| !matches!(value.to_string().as_str(), "NULL" | "FALSE"));
        let (kind, default_value) = if no_default {
            (S4DefaultKind::None, None)
        } else if let Some(value) = explicit {
            (S4DefaultKind::Explicit, Some(value))
        } else if standard_dispatch == Some(false) {
            (S4DefaultKind::Definition, function.clone())
        } else if function.is_none() {
            (S4DefaultKind::Existing, None)
        } else if standard_dispatch == Some(true) {
            (S4DefaultKind::None, None)
        } else {
            (S4DefaultKind::Unknown, None)
        };
        let default = S4DefaultMethod {
            kind,
            signature: self.implementation_signature(default_value.clone(), &mut BTreeSet::new()),
            implementation: default_value
                .and_then(|value| self.supplied_implementation(value))
                .or_else(|| {
                    (kind == S4DefaultKind::Existing)
                        .then(|| definition.clone())
                        .flatten()
                }),
        };
        let groups = value("group");
        let simple_inheritance_only = match value("simpleInheritanceOnly")
            .map(|value| value.to_string())
            .as_deref()
        {
            None | Some("NULL" | "FALSE") => Some(false),
            Some("TRUE") => Some(true),
            _ => None,
        };
        let vector = groups.as_ref().and_then(|value| {
            self.string_vector(value).or_else(|| {
                let call = value.as_node().cloned().and_then(CallExpr::cast)?;
                let (reference, name) = factory(self.model, self.root, &call)?;
                if name != "list" || !accepts(&reference, "base") {
                    return None;
                }
                Some(StringVector {
                    factory: Some(reference),
                    entries: call
                        .arg_list()?
                        .args()
                        .map(|arg| (None, string_name(arg.value()), arg.syntax().text_range()))
                        .collect(),
                })
            })
        });
        let skeleton = if value("genericFunction").is_some_and(|value| value.to_string() != "NULL")
            || value("valueClass").is_some()
            || simple_inheritance_only.is_none()
        {
            // Custom generic classes and body/signature transformations need
            // more than a callable signature to establish compatibility.
            None
        } else if value("def").is_none_or(|value| value.to_string() == "NULL")
            || standard_dispatch == Some(false)
        {
            Some(S4GenericSkeleton::Standard)
        } else {
            resolved.as_ref().and_then(|function| {
                let body = function.body()?;
                let standard = body
                    .as_node()
                    .cloned()
                    .and_then(CallExpr::cast)
                    .is_some_and(|call| {
                        call.base().is_some_and(|base| {
                            matches!(base, SyntaxElement::Token(token) if token.kind() == SyntaxKind::IDENT && unbacktick(token.text()) == "standardGeneric")
                        }) && call.arg_list().is_some_and(|args| {
                            let args: Vec<_> = args.args().collect();
                            args.len() == 1 && args[0].name().is_none()
                                && string_name(args[0].value()) == *name
                        })
                    });
                if standard {
                    Some(S4GenericSkeleton::Standard)
                } else {
                    S4GenericBody::from_syntax(body).map(S4GenericSkeleton::Nonstandard)
                }
            })
        };
        S4GenericDefinition {
            definition_supplied: value("def").is_some_and(|value| value.to_string() != "NULL"),
            simple_redeclaration: args.as_ref().is_some_and(|args| {
                args.iter()
                    .all(|(name, value)| name == "name" || value.is_none())
            }),
            signature_only: args.as_ref().is_some_and(|args| {
                args.get("signature").is_some_and(Option::is_some)
                    && args.iter().all(|(name, value)| {
                        matches!(name.as_str(), "name" | "signature") || value.is_none()
                    })
            }),
            simple_inheritance_only,
            lookup: value("name")
                .as_ref()
                .and_then(SyntaxElement::as_token)
                .and_then(|token| match name {
                    ClassName::Known(name) => free_function_ref(self.model, self.root, token, name),
                    _ => None,
                }),
            package: value("package")
                .filter(|value| value.to_string() != "NULL")
                .map(|value| string_name(Some(value))),
            groups: vector
                .as_ref()
                .map(|vector| {
                    vector
                        .entries
                        .iter()
                        .map(|(_, name, _)| name.clone())
                        .collect()
                })
                .unwrap_or_else(|| {
                    if groups.is_some() {
                        vec![ClassName::Unknown]
                    } else {
                        Vec::new()
                    }
                }),
            group_factory: vector.and_then(|vector| vector.factory),
            standard_factory,
            definition,
            previous,
            default,
            dispatch_explicit: value("signature").is_some_and(|value| value.to_string() != "NULL"),
            default_disabled: no_default,
            standard_dispatch,
            skeleton,
        }
    }
}
