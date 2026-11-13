//! Promise provenance across the two S3 forwarding mechanisms.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S3DispatchObject {
    OriginalFirst {
        helpers: Vec<FunctionRef>,
    },
    Formal(String),
    Classes {
        classes: S3ClassEvidence,
        helpers: Vec<FunctionRef>,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S3GenericDispatch {
    pub object: S3DispatchObject,
    pub locals: BTreeMap<String, Availability>,
}

/// An exact runtime version policy, not an assertion that a compatibility
/// minimum below 4.4 prevents the program from running on a newer R.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3RVersion {
    Before44,
    Since44,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3ArgumentSource {
    Original { argument: usize },
    CurrentFormal { name: String },
    NextArgument { argument: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3ForwardedArgument {
    pub name: Option<String>,
    pub source: S3ArgumentSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Forwarding {
    pub arguments: Vec<S3ForwardedArgument>,
    pub generic_locals: BTreeMap<String, Availability>,
    pub nonreturning: bool,
    pub uncertain: bool,
}

impl S3Forwarding {
    /// UseMethod forwards the incoming promises, with names expanded by the
    /// generic's matching. Generic assignments and defaults are not actuals.
    pub fn use_method(
        signature: &CallableSignature,
        supplied_names: &[Option<SmolStr>],
        locals: &BTreeMap<String, Availability>,
        version: S3RVersion,
    ) -> Self {
        let formals: Vec<_> = signature
            .parameters
            .iter()
            .map(|formal| formal.name.as_str())
            .collect();
        let matched = crate::semantic::checked_match_args_to_formals(supplied_names, &formals);
        let mut arguments = Vec::new();
        if let Ok(matched) = &matched {
            for formal in &formals {
                for (index, name) in matched.iter().enumerate() {
                    if name == &Some(*formal) || *formal == "..." && name.is_none() {
                        arguments.push(S3ForwardedArgument {
                            name: name.map(str::to_string).or_else(|| {
                                supplied_names[index].as_ref().map(ToString::to_string)
                            }),
                            source: S3ArgumentSource::Original { argument: index },
                        });
                    }
                }
            }
        }
        Self {
            arguments,
            generic_locals: locals
                .iter()
                .filter(|(name, _)| {
                    !formals.contains(&name.as_str())
                        && !supplied_names
                            .iter()
                            .flatten()
                            .any(|actual| unbacktick(actual) == name.as_str())
                })
                .map(|(name, availability)| {
                    (
                        name.clone(),
                        match version {
                            S3RVersion::Before44 => *availability,
                            S3RVersion::Since44 => Availability::Absent,
                            S3RVersion::Unknown => Availability::Unknown,
                        },
                    )
                })
                .collect(),
            nonreturning: true,
            uncertain: matched.is_err(),
        }
    }

    /// Match only recovered promise names. Dispatch never makes a method's
    /// signature safe for call diagnostics; the existing closure gates own that.
    pub fn matched_formals(&self, signature: &CallableSignature) -> Option<Vec<Option<String>>> {
        if self.uncertain {
            return None;
        }
        let names: Vec<_> = self
            .arguments
            .iter()
            .map(|argument| argument.name.as_deref().map(SmolStr::new))
            .collect();
        let formals: Vec<_> = signature
            .parameters
            .iter()
            .map(|formal| formal.name.as_str())
            .collect();
        Some(
            crate::semantic::checked_match_args_to_formals(&names, &formals)
                .ok()?
                .into_iter()
                .map(|formal| formal.map(str::to_string))
                .collect(),
        )
    }

    /// NextMethod forwards supplied method formals through the current frame.
    /// Dots preserve their incoming promises. Exact named extras replace an
    /// existing argument; other extras append, including all unnamed extras.
    pub fn next_method(
        &self,
        signature: &CallableSignature,
        extra_names: &[Option<String>],
    ) -> Self {
        let matched = self.matched_formals(signature);
        let mut arguments = self.arguments.clone();
        if let Some(matched) = &matched {
            for (argument, formal) in arguments.iter_mut().zip(matched) {
                if let Some(name) = formal {
                    argument.source = S3ArgumentSource::CurrentFormal { name: name.clone() };
                }
            }
        }
        for (index, name) in extra_names.iter().enumerate() {
            let extra = S3ForwardedArgument {
                name: name.clone(),
                source: S3ArgumentSource::NextArgument { argument: index },
            };
            if let Some(existing) = name.as_ref().and_then(|name| {
                arguments
                    .iter_mut()
                    .find(|argument| argument.name.as_ref() == Some(name))
            }) {
                *existing = extra;
            } else {
                arguments.push(extra);
            }
        }
        Self {
            arguments,
            generic_locals: BTreeMap::new(),
            nonreturning: false,
            uncertain: self.uncertain || matched.is_none(),
        }
    }
}

pub(in crate::project::classes) fn generic_dispatch(
    model: &SemanticModel,
    call: &CallExpr,
    function: Option<&SyntaxElement>,
) -> S3GenericDispatch {
    let function = function
        .and_then(SyntaxElement::as_node)
        .cloned()
        .and_then(FunctionExpr::cast);
    let scope = function.as_ref().and_then(|function| {
        model
            .scopes()
            .iter()
            .find(|scope| scope.range == function.syntax().text_range())
    });
    let formals: BTreeSet<_> = scope
        .into_iter()
        .flat_map(|scope| &scope.bindings)
        .map(|id| model.binding(*id))
        .filter(|binding| binding.kind == BindingKind::Param)
        .map(|binding| unbacktick(&binding.name).to_string())
        .collect();
    let root = call.syntax().ancestors().last().expect("a call has a root");
    let locals = scope
        .into_iter()
        .flat_map(|scope| &scope.bindings)
        .map(|id| model.binding(*id))
        .filter(|binding| {
            binding.kind == BindingKind::Local
                && binding.def_range.start() < call.syntax().text_range().start()
        })
        .filter(|binding| !formals.contains(unbacktick(&binding.name)))
        .fold(BTreeMap::new(), |mut locals, binding| {
            let availability = if !opaque_binding_frame(&root, binding.def_range)
                && binding_assignment(&root, binding.def_range)
                    .is_some_and(|assignment| unconditional(assignment.syntax()))
            {
                Availability::Present
            } else {
                Availability::Unknown
            };
            locals
                .entry(unbacktick(&binding.name).to_string())
                .and_modify(|known| {
                    if availability == Availability::Present {
                        *known = availability;
                    }
                })
                .or_insert(availability);
            locals
        });
    let args = matched_args(call, &["generic", "object"]);
    let mut object = match args.as_ref() {
        None => S3DispatchObject::Unknown,
        Some(args) if !args.contains_key("object") => {
            super::super::s3::original_object_preservation(model, call, &formals)
                .map(|helpers| S3DispatchObject::OriginalFirst { helpers })
                .unwrap_or(S3DispatchObject::Unknown)
        }
        Some(args) => argument(args, "object")
            .and_then(|value| value.into_token())
            .filter(|token| formals.contains(unbacktick(token.text())))
            .filter(|token| {
                !scope
                    .into_iter()
                    .flat_map(|scope| &scope.bindings)
                    .map(|id| model.binding(*id))
                    .any(|binding| {
                        binding.kind != BindingKind::Param
                            && unbacktick(&binding.name) == unbacktick(token.text())
                            && binding.def_range.start() < call.syntax().text_range().start()
                    })
            })
            .map(|token| S3DispatchObject::Formal(unbacktick(token.text()).into()))
            .unwrap_or(S3DispatchObject::Unknown),
    };
    if object == S3DispatchObject::Unknown
        && let Some((classes, helpers)) = super::super::s3::generic_object_evidence(model, call)
    {
        object = S3DispatchObject::Classes { classes, helpers };
    }
    S3GenericDispatch { object, locals }
}

impl ClassIndex {
    pub(super) fn s3_call_arguments(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
        call: &CallShape,
        generic: &S3GenericId,
    ) -> (Vec<S3ClassEvidence>, Option<S3Forwarding>) {
        let evidence = |index: usize| {
            call.arguments
                .get(index)
                .and_then(Option::as_ref)
                .map(|argument| self.s3_argument_evidence(path, root, model, value, argument))
                .unwrap_or_default()
        };
        let S3GenericId::Declaration(id) = generic else {
            if matches!(call.name.as_str(), "cbind" | "rbind") {
                let classes = self.s3_arguments_evidence(path, root, model, value, &call.arguments);
                let classes = call
                    .names
                    .iter()
                    .zip(classes)
                    .filter(|(name, _)| name.as_deref() != Some("deparse.level"))
                    .map(|(_, class)| class)
                    .collect();
                return (classes, None);
            }
            let both = matches!(group(&call.name), Some("Ops" | "matrixOps"));
            if both {
                let classes = self.s3_arguments_evidence(path, root, model, value, &call.arguments);
                return (classes.into_iter().take(2).collect(), None);
            }
            let formals: &[&str] = match call.name.as_str() {
                "print" => &["x", "..."],
                "round" | "signif" => &["x", "digits"],
                _ => {
                    // Library signatures are not part of this projection.
                    // Named actuals cannot safely establish source-order dispatch.
                    return (
                        vec![if call.positional
                            || matches!(
                                call.name.as_str(),
                                "[" | "[[" | "[<-" | "[[<-" | "$" | "$<-" | "@" | "@<-"
                            )
                            || call.names.iter().all(Option::is_none)
                        {
                            evidence(0)
                        } else {
                            S3ClassEvidence::Unknown
                        }],
                        None,
                    );
                }
            };
            let dispatch = crate::semantic::checked_match_args_to_formals(&call.names, formals)
                .ok()
                .and_then(|matched| matched.iter().position(|formal| *formal == Some("x")));
            if matches!(call.name.as_str(), "round" | "signif") {
                let classes = self.s3_arguments_evidence(path, root, model, value, &call.arguments);
                return (
                    vec![
                        dispatch
                            .and_then(|index| classes.get(index).cloned())
                            .unwrap_or_default(),
                    ],
                    None,
                );
            }
            return (vec![dispatch.map(evidence).unwrap_or_default()], None);
        };
        let Some(def) = self.generic(id) else {
            return (vec![S3ClassEvidence::Unknown], None);
        };
        let Some(signature) = &def.signature else {
            return (vec![S3ClassEvidence::Unknown], None);
        };
        let forwarding = S3Forwarding::use_method(
            signature,
            &call.names,
            &def.s3_dispatch
                .as_ref()
                .map(|body| body.locals.clone())
                .unwrap_or_default(),
            S3RVersion::Unknown,
        );
        let formals: Vec<_> = signature
            .parameters
            .iter()
            .map(|formal| formal.name.as_str())
            .collect();
        let matched = crate::semantic::checked_match_args_to_formals(&call.names, &formals);
        let class = if let Ok(matched) = matched {
            let object = def.s3_dispatch.as_ref().map(|body| &body.object);
            let formal = match object {
                Some(S3DispatchObject::OriginalFirst { .. }) => formals.first().copied(),
                Some(S3DispatchObject::Formal(name)) => Some(name.as_str()),
                _ => None,
            };
            if let Some(S3DispatchObject::OriginalFirst { helpers }) = object {
                if helpers.iter().all(|helper| {
                    self.callee_identity(&id.path, helper)
                        .is_some_and(|origin| origin.package == "base")
                }) {
                    forwarding
                        .arguments
                        .first()
                        .and_then(|argument| match argument.source {
                            S3ArgumentSource::Original { argument } => Some(evidence(argument)),
                            _ => None,
                        })
                        .unwrap_or(S3ClassEvidence::NoExplicitClass)
                } else {
                    S3ClassEvidence::Unknown
                }
            } else if id.path == path {
                let dispatch = oop_sites(model, root)
                    .into_iter()
                    .find(|site| {
                        site.role == OopSiteRole::Declaration
                            && site.target == OopTarget::Declaration(id.key.clone())
                    })
                    .and_then(|site| root.covering_element(site.range).parent())
                    .and_then(|node| node.ancestors().find_map(CallExpr::cast));
                if let Some(dispatch) = dispatch {
                    let arguments = matched
                        .iter()
                        .enumerate()
                        .filter_map(|(index, formal)| {
                            formal.map(|name| (name.to_string(), evidence(index)))
                        })
                        .collect();
                    self.s3_generic_object(path, root, model, &dispatch, &arguments)
                } else {
                    S3ClassEvidence::Unknown
                }
            } else if let Some(S3DispatchObject::Classes { classes, helpers }) = object {
                if helpers.iter().all(|helper| {
                    self.callee_identity(&id.path, helper)
                        .is_some_and(|origin| origin.package == "base")
                }) {
                    classes.clone()
                } else {
                    S3ClassEvidence::Unknown
                }
            } else {
                formal
                    .and_then(|formal| matched.iter().position(|matched| *matched == Some(formal)))
                    .map(evidence)
                    .unwrap_or_default()
            }
        } else {
            S3ClassEvidence::Unknown
        };
        (vec![class], Some(forwarding))
    }
}
