use super::super::*;

struct StringVector {
    entries: Vec<(Option<String>, ClassName, TextRange)>,
    factory: Option<FunctionRef>,
}

struct Dispatch {
    arguments: Vec<(DispatchArgument, TextRange)>,
    factory: Option<FunctionRef>,
}

struct Projection<'a> {
    model: &'a SemanticModel,
    root: &'a SyntaxNode,
    owners: &'a HashMap<TextRange, ClassOwner>,
    classes: &'a Extracted,
    class_keys: HashMap<TextRange, ClassKey>,
    facts: OopFacts,
    sites: Vec<OopSite>,
    method_bodies: Vec<(TextRange, ClassKey)>,
    method_implementations: Vec<MethodImplementationSite>,
    generic_values: HashMap<TextRange, OopKey>,
    counts: BTreeMap<(ClassOwner, OopSystem, OopKind), u32>,
    include_sites: bool,
}

impl Projection<'_> {
    fn implementation(&mut self, key: &OopKey, value: Option<SyntaxElement>, keep_original: bool) {
        let Some(function) = self
            .implementation_value(
                value.clone(),
                &mut BTreeSet::new(),
                key.system == OopSystem::S3,
            )
            .and_then(SyntaxElement::into_node)
            .and_then(FunctionExpr::cast)
        else {
            if let Some(token) = value.as_ref().and_then(SyntaxElement::as_token)
                && (token.kind() == SyntaxKind::IDENT
                    || (key.system == OopSystem::S3 && token.kind() == SyntaxKind::STRING))
                && read_binding(self.model, token).is_none_or(|binding| {
                    self.model.scope(self.model.binding(binding).scope).kind
                        == crate::semantic::ScopeKind::File
                })
                && let Some(name) = literal(&SyntaxElement::Token(token.clone()))
            {
                self.facts
                    .method_implementations
                    .push(MethodImplementationDef {
                        method: key.clone(),
                        source: MethodImplementationSource::Reference { name },
                    });
            }
            return;
        };
        let source = direct_assignment(&SyntaxElement::Node(function.syntax().clone()))
            .and_then(|assignment| {
                Some(MethodImplementationSource::Binding {
                    owner: owner_at(assignment.syntax(), self.owners),
                    name: unbacktick(&assignment.target_name()?).to_string(),
                })
            })
            .unwrap_or(MethodImplementationSource::Inline);
        let keep_original = keep_original && !matches!(source, MethodImplementationSource::Inline);
        self.facts
            .method_implementations
            .push(MethodImplementationDef {
                method: key.clone(),
                source,
            });
        self.method_implementations.push(MethodImplementationSite {
            method: key.clone(),
            function_range: function.syntax().text_range(),
            owner_range: function
                .syntax()
                .ancestors()
                .skip(1)
                .find_map(FunctionExpr::cast)
                .map_or(self.root.text_range(), |owner| owner.syntax().text_range()),
            keep_original,
        });
    }

    fn key(&mut self, node: &SyntaxNode, system: OopSystem, kind: OopKind) -> OopKey {
        let owner = owner_at(node, self.owners);
        let count = self
            .counts
            .entry((owner.clone(), system, kind))
            .or_default();
        let key = OopKey {
            owner,
            system,
            kind,
            occurrence: *count,
        };
        *count += 1;
        key
    }

    fn site(&mut self, range: TextRange, role: OopSiteRole, target: OopTarget) {
        if !self.include_sites {
            return;
        }
        let node = self
            .root
            .covering_element(range)
            .parent()
            .unwrap_or_else(|| self.root.clone());
        let owner = owner_at(&node, self.owners);
        let in_function = node
            .ancestors()
            .any(|node| node.kind() == SyntaxKind::FUNCTION_EXPR);
        self.sites.push(OopSite {
            range,
            role,
            target,
            owner,
            in_function,
        });
    }

    fn generator(&self, value: &SyntaxElement) -> GeneratorRef {
        generator_ref(
            self.model,
            self.root,
            value,
            &self.class_keys,
            &mut BTreeSet::new(),
            false,
        )
    }
}

fn signature(value: Option<SyntaxElement>) -> Option<CallableSignature> {
    let function = value?.into_node().and_then(FunctionExpr::cast)?;
    function.rparen_index()?;
    Some(CallableSignature {
        parameters: function
            .formals()
            .into_iter()
            .map(|formal| CallableParameter {
                name: unbacktick(&formal.name()).to_string(),
                default: formal.default().map(|value| value.to_string()),
            })
            .collect(),
    })
}

fn s3_dispatch_argument(
    object: Option<SyntaxElement>,
    signature: Option<&CallableSignature>,
) -> Option<String> {
    let signature = signature?;
    let Some(mut object) = object else {
        return signature
            .parameters
            .first()
            .map(|parameter| parameter.name.clone());
    };
    while let Some(paren) = object
        .as_node()
        .cloned()
        .and_then(crate::ast::ParenExpr::cast)
    {
        object = paren.inner()?;
    }
    let token = object
        .as_token()
        .filter(|token| token.kind() == SyntaxKind::IDENT)?;
    signature
        .parameters
        .iter()
        .find(|parameter| parameter.name == unbacktick(token.text()))
        .map(|parameter| parameter.name.clone())
}

fn literal(value: &SyntaxElement) -> Option<String> {
    let token = value.as_token()?;
    match token.kind() {
        SyntaxKind::STRING => crate::semantic::decode_string_literal(token.text()),
        SyntaxKind::IDENT => Some(unbacktick(token.text()).to_string()),
        _ => None,
    }
}

fn string_name(value: Option<SyntaxElement>) -> ClassName {
    value
        .and_then(|value| {
            let token = value.as_token()?;
            (token.kind() == SyntaxKind::STRING)
                .then(|| literal(&value))
                .flatten()
        })
        .map_or(ClassName::Unknown, ClassName::Known)
}

fn assigned_name(node: &SyntaxNode) -> Option<String> {
    direct_assignment(&SyntaxElement::Node(node.clone()))
        .and_then(|assignment| assignment.target_name())
        .map(|name| unbacktick(&name).to_string())
}

fn factory(
    model: &SemanticModel,
    root: &SyntaxNode,
    call: &CallExpr,
) -> Option<(FunctionRef, String)> {
    let reference = call_function_ref(model, root, call)?;
    let name = match &reference {
        FunctionRef::Qualified { name, .. } | FunctionRef::Bare { name, .. } => name.clone(),
        FunctionRef::Unresolved(_) => return None,
    };
    Some((reference, name))
}

fn accepts(reference: &FunctionRef, package: &str) -> bool {
    match reference {
        FunctionRef::Qualified {
            package: origin, ..
        } => origin == package,
        FunctionRef::Bare { .. } => true,
        FunctionRef::Unresolved(candidates) => candidates
            .iter()
            .any(|candidate| candidate.package == package),
    }
}

fn uncertainty(node: &SyntaxNode, reference: &FunctionRef) -> BTreeSet<OopUncertainty> {
    let mut issues = BTreeSet::new();
    if !unconditional(node) {
        issues.insert(OopUncertainty::Conditional);
    }
    // Bare calls retain their origin proof obligation for the project resolver.
    if !matches!(reference, FunctionRef::Qualified { .. }) {
        issues.insert(OopUncertainty::UnresolvedCallee);
    }
    issues
}

/// Declaration roots also determine lexical owners. Ordinary closures that do
/// not contain declarations cannot renumber the shared owner keys.
pub(in crate::project::classes) fn declaration_roots(
    model: &SemanticModel,
    root: &SyntaxNode,
) -> Vec<SyntaxNode> {
    factories(model, root)
        .into_iter()
        .filter_map(|(call, reference, name)| {
            let package = match name.as_str() {
                "setGeneric" | "setMethod" | "setReplaceMethod" => "methods",
                "new_generic" | "method<-" => "S7",
                "registerS3method" | "UseMethod" => "base",
                _ => return None,
            };
            if !accepts(&reference, package) {
                return None;
            }
            if name == "UseMethod" {
                let function = call.syntax().ancestors().find_map(FunctionExpr::cast)?;
                return direct_assignment(&SyntaxElement::Node(function.syntax().clone()))
                    .map(|assignment| assignment.syntax().clone());
            }
            Some(call.syntax().clone())
        })
        .chain(
            root.descendants()
                .filter_map(AssignmentExpr::cast)
                .filter(|assignment| {
                    assignment.target_name().is_some_and(|name| {
                        let name = unbacktick(&name);
                        name.char_indices()
                            .any(|(i, ch)| ch == '.' && i > 0 && i + 1 < name.len())
                    }) && assignment
                        .value_element()
                        .is_some_and(|value| signature(Some(value)).is_some())
                })
                .map(|assignment| assignment.syntax().clone()),
        )
        .collect()
}

pub(in crate::project::classes) fn project(
    model: &SemanticModel,
    root: &SyntaxNode,
    owners: &HashMap<TextRange, ClassOwner>,
    classes: &Extracted,
    include_sites: bool,
) -> (OopFacts, Vec<OopSite>, Vec<MethodImplementationSite>) {
    let mut projection = Projection {
        model,
        root,
        owners,
        classes,
        class_keys: classes
            .facts
            .declarations
            .iter()
            .zip(&classes.locations)
            .filter(|(def, _)| !matches!(def.factory, FunctionRef::Unresolved(_)))
            .map(|(def, loc)| (loc.call_range, def.key.clone()))
            .collect(),
        facts: OopFacts::default(),
        sites: Vec::new(),
        method_bodies: Vec::new(),
        method_implementations: Vec::new(),
        generic_values: HashMap::new(),
        counts: BTreeMap::new(),
        include_sites,
    };
    projection.members();
    projection.generics();
    projection.generic_bindings();
    projection.registered_methods();
    projection.s3_methods();
    if include_sites {
        projection.use_sites();
    }
    projection
        .sites
        .sort_by_key(|site| (site.range.start(), site.range.end()));
    (
        projection.facts,
        projection.sites,
        projection.method_implementations,
    )
}

/// The declaration projection is range-free. Call this against the current CST
/// to obtain declaration, alias, registration-reference, and member-use spans.
pub fn oop_sites(model: &SemanticModel, root: &SyntaxNode) -> Vec<OopSite> {
    extract(model, root, true).oop_sites
}

pub fn method_implementation_sites(
    model: &SemanticModel,
    root: &SyntaxNode,
) -> Vec<MethodImplementationSite> {
    extract(model, root, false).method_implementations
}

impl Projection<'_> {
    fn members(&mut self) {
        for (def, location) in self
            .classes
            .facts
            .declarations
            .iter()
            .zip(&self.classes.locations)
        {
            let Some(call) = self
                .root
                .covering_element(location.call_range)
                .into_node()
                .and_then(CallExpr::cast)
            else {
                continue;
            };
            let Some(args) = matched_args(&call, def.system().formals()) else {
                continue;
            };
            let groups: &[(&str, MemberVisibility, MemberKind)] = match def.system() {
                ClassSystem::R6 => &[
                    ("public", MemberVisibility::Public, MemberKind::Field),
                    ("private", MemberVisibility::Private, MemberKind::Field),
                    (
                        "active",
                        MemberVisibility::Public,
                        MemberKind::ActiveBinding,
                    ),
                ],
                ClassSystem::RefClass => &[
                    ("fields", MemberVisibility::Public, MemberKind::Field),
                    ("methods", MemberVisibility::Public, MemberKind::Method),
                ],
                ClassSystem::S4 => &[("slots", MemberVisibility::Public, MemberKind::Slot)],
                ClassSystem::S7 => {
                    &[("properties", MemberVisibility::Public, MemberKind::Property)]
                }
            };
            for &(group, visibility, kind) in groups {
                let Some(value) = argument(&args, group) else {
                    continue;
                };
                if matches!(&value, SyntaxElement::Token(token) if token.text() == "NULL") {
                    continue;
                }
                let constructor = value.as_node().cloned().and_then(CallExpr::cast);
                let factory = constructor
                    .as_ref()
                    .and_then(|call| factory(self.model, self.root, call));
                let mut issues: BTreeSet<_> = def
                    .uncertainty
                    .iter()
                    .filter_map(|issue| {
                        Some(match issue {
                            ClassUncertainty::Conditional => OopUncertainty::Conditional,
                            ClassUncertainty::CustomEnvironment => {
                                OopUncertainty::CustomEnvironment
                            }
                            ClassUncertainty::InvalidArguments => OopUncertainty::InvalidArguments,
                            ClassUncertainty::DynamicName => OopUncertainty::DynamicName,
                            ClassUncertainty::UnresolvedCallee => OopUncertainty::UnresolvedCallee,
                            ClassUncertainty::DynamicParent => return None,
                        })
                    })
                    .collect();
                let entries = match (constructor, &factory) {
                    (Some(call), Some((reference, name)))
                        if accepts(reference, "base")
                            && (name == "list"
                                || (name == "c"
                                    && matches!(kind, MemberKind::Field | MemberKind::Slot))) =>
                    {
                        if !matches!(reference, FunctionRef::Qualified { .. }) {
                            issues.insert(OopUncertainty::UnresolvedCallee);
                        }
                        if call
                            .syntax()
                            .children_with_tokens()
                            .any(|e| e.kind() == SyntaxKind::RPAREN)
                        {
                            call.arg_list().map(|list| list.args().collect::<Vec<_>>())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if entries.is_none() {
                    issues.insert(OopUncertainty::DynamicMembers);
                }
                self.facts.member_groups.push(MemberGroup {
                    class: def.key.clone(),
                    name: group.into(),
                    factory: factory.map(|(reference, _)| reference),
                    uncertainty: issues.clone(),
                });
                for arg in entries.into_iter().flatten() {
                    let name = arg.name().map(|name| unbacktick(&name).to_string());
                    let mut member_issues = issues.clone();
                    if name.is_none() {
                        member_issues.insert(OopUncertainty::DynamicName);
                    }
                    let implementation =
                        self.implementation_value(arg.value(), &mut BTreeSet::new(), false);
                    let callable = signature(implementation.clone());
                    let member_kind =
                        if kind == MemberKind::Field && def.system() == ClassSystem::R6 {
                            if callable.is_some() {
                                MemberKind::Method
                            } else if implementation
                                .clone()
                                .and_then(crate::ast::Expr::cast)
                                .is_some_and(|value| match value {
                                    crate::ast::Expr::Name(name) => name.is_reserved_constant(),
                                    crate::ast::Expr::IntLiteral(_)
                                    | crate::ast::Expr::FloatLiteral(_)
                                    | crate::ast::Expr::StringLiteral(_)
                                    | crate::ast::Expr::ComplexLiteral(_) => true,
                                    _ => false,
                                })
                            {
                                MemberKind::Field
                            } else {
                                member_issues.insert(OopUncertainty::UnresolvedImplementation);
                                MemberKind::Unknown
                            }
                        } else {
                            kind
                        };
                    if callable.is_none()
                        && matches!(member_kind, MemberKind::Method | MemberKind::ActiveBinding)
                    {
                        member_issues.insert(OopUncertainty::UnresolvedImplementation);
                    }
                    let key = self.key(call.syntax(), def.system().into(), OopKind::Member);
                    let method = if callable.is_some()
                        && matches!(member_kind, MemberKind::Method | MemberKind::ActiveBinding)
                    {
                        let method_key =
                            self.key(call.syntax(), def.system().into(), OopKind::Method);
                        self.facts.methods.push(MethodDef {
                            key: method_key.clone(),
                            system: def.system().into(),
                            name: name.clone().map_or(ClassName::Unknown, ClassName::Known),
                            class: Some(def.key.clone()),
                            generic: None,
                            dispatch: Vec::new(),
                            dispatch_factory: None,
                            signature: callable,
                            registration: None,
                            uncertainty: member_issues.clone(),
                        });
                        if matches!(def.system(), ClassSystem::R6 | ClassSystem::RefClass)
                            && let Some(function) = implementation
                                .clone()
                                .and_then(SyntaxElement::into_node)
                                .and_then(FunctionExpr::cast)
                        {
                            let assignment =
                                direct_assignment(&SyntaxElement::Node(function.syntax().clone()));
                            let source = assignment
                                .as_ref()
                                .and_then(|assignment| {
                                    Some(MethodImplementationSource::Binding {
                                        owner: owner_at(assignment.syntax(), self.owners),
                                        name: unbacktick(&assignment.target_name()?).to_string(),
                                    })
                                })
                                .unwrap_or(MethodImplementationSource::Inline);
                            let keep_original =
                                !matches!(source, MethodImplementationSource::Inline);
                            self.facts
                                .method_implementations
                                .push(MethodImplementationDef {
                                    method: method_key.clone(),
                                    source,
                                });
                            self.method_implementations.push(MethodImplementationSite {
                                method: method_key.clone(),
                                function_range: function.syntax().text_range(),
                                owner_range: call
                                    .syntax()
                                    .ancestors()
                                    .find_map(FunctionExpr::cast)
                                    .map_or(self.root.text_range(), |owner| {
                                        owner.syntax().text_range()
                                    }),
                                keep_original,
                            });
                        }
                        if let Some(body) = arg.value() {
                            self.method_bodies
                                .push((body.text_range(), def.key.clone()));
                        }
                        Some(method_key)
                    } else {
                        None
                    };
                    let declared_type = match member_kind {
                        MemberKind::Slot => match string_name(arg.value()) {
                            ClassName::Known(name) => Some(DispatchClass::Formal(name)),
                            _ => Some(DispatchClass::Unknown),
                        },
                        MemberKind::Field if def.system() == ClassSystem::RefClass => {
                            match string_name(arg.value()) {
                                ClassName::Known(name) => Some(DispatchClass::Formal(name)),
                                _ => Some(DispatchClass::Unknown),
                            }
                        }
                        MemberKind::Property => arg
                            .value()
                            .map(|value| DispatchClass::Generator(self.generator(&value))),
                        _ => None,
                    };
                    self.facts.members.push(MemberDef {
                        key: key.clone(),
                        class: def.key.clone(),
                        name: name.map_or(ClassName::Unknown, ClassName::Known),
                        visibility,
                        kind: member_kind,
                        method,
                        declared_type,
                        uncertainty: member_issues,
                    });
                    let range = arg
                        .name_token()
                        .map_or_else(|| arg.syntax().text_range(), |token| token.text_range());
                    self.site(range, OopSiteRole::Declaration, OopTarget::Declaration(key));
                }
            }
        }
    }

    fn generics(&mut self) {
        for (call, reference, name) in factories(self.model, self.root) {
            let (system, node, formals) = match name.as_str() {
                "setGeneric" if accepts(&reference, "methods") => (
                    OopSystem::S4,
                    call.syntax().clone(),
                    &[
                        "name",
                        "def",
                        "group",
                        "valueClass",
                        "where",
                        "package",
                        "signature",
                        "useAsDefault",
                        "genericFunction",
                        "simpleInheritanceOnly",
                    ][..],
                ),
                "new_generic" if accepts(&reference, "S7") => (
                    OopSystem::S7,
                    call.syntax().clone(),
                    &["name", "dispatch_args", "fun"][..],
                ),
                "UseMethod" if accepts(&reference, "base") => {
                    let Some(function) = call.syntax().ancestors().find_map(FunctionExpr::cast)
                    else {
                        continue;
                    };
                    let Some(assignment) =
                        direct_assignment(&SyntaxElement::Node(function.syntax().clone()))
                    else {
                        continue;
                    };
                    (
                        OopSystem::S3,
                        assignment.syntax().clone(),
                        &["generic", "object"][..],
                    )
                }
                _ => continue,
            };
            let key = self.key(&node, system, OopKind::Generic);
            let mut issues = uncertainty(&node, &reference);
            if !unconditional(call.syntax()) {
                issues.insert(OopUncertainty::Conditional);
            }
            let args = matched_args(&call, formals);
            if args.is_none() {
                issues.insert(OopUncertainty::InvalidArguments);
            }
            let value = |name: &str| args.as_ref().and_then(|args| argument(args, name));
            let name_value = value(if system == OopSystem::S3 {
                "generic"
            } else {
                "name"
            });
            let generic_name = string_name(name_value.clone());
            if generic_name == ClassName::Unknown {
                issues.insert(OopUncertainty::DynamicName);
            }
            if value("where").is_some() {
                issues.insert(OopUncertainty::CustomEnvironment);
            }
            let function = if system == OopSystem::S3 {
                AssignmentExpr::cast(node.clone()).and_then(|assignment| assignment.value_element())
            } else {
                value(if system == OopSystem::S7 {
                    "fun"
                } else {
                    "def"
                })
            };
            let signature = signature(function.clone());
            let dispatch_value = value(if system == OopSystem::S7 {
                "dispatch_args"
            } else {
                "signature"
            });
            let mut dispatch_factory = None;
            let dispatch_arguments = if let Some(value) = dispatch_value {
                match self.string_vector(&value) {
                    Some(vector) => {
                        dispatch_factory = vector.factory;
                        let values = vector.entries;
                        if values
                            .iter()
                            .any(|(_, name, _)| *name == ClassName::Unknown)
                        {
                            issues.insert(OopUncertainty::DynamicSignature);
                        }
                        values.into_iter().map(|(_, name, _)| name).collect()
                    }
                    None => {
                        issues.insert(OopUncertainty::DynamicSignature);
                        Vec::new()
                    }
                }
            } else if system == OopSystem::S3 {
                let argument = s3_dispatch_argument(value("object"), signature.as_ref());
                if argument.is_none() {
                    issues.insert(OopUncertainty::DynamicSignature);
                }
                vec![argument.map_or(ClassName::Unknown, ClassName::Known)]
            } else if system == OopSystem::S4 {
                signature
                    .as_ref()
                    .map(|sig| {
                        sig.parameters
                            .iter()
                            .filter(|p| p.name != "...")
                            .map(|p| ClassName::Known(p.name.clone()))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                issues.insert(OopUncertainty::DynamicSignature);
                Vec::new()
            };
            let binding = if system == OopSystem::S3 {
                AssignmentExpr::cast(node.clone())
                    .and_then(|a| a.target_name())
                    .map(|name| unbacktick(&name).to_string())
            } else {
                assigned_name(&node)
            };
            let def = GenericDef {
                key: key.clone(),
                name: generic_name,
                binding,
                factory: reference,
                signature,
                dispatch_arguments,
                dispatch_factory,
                uncertainty: issues,
            };
            let binding_possible = def.binding_possible();
            if binding_possible
                && system == OopSystem::S4
                && let ClassName::Known(name) = &def.name
                && name_value
                    .as_ref()
                    .and_then(SyntaxElement::as_token)
                    .is_some_and(|token| read_named_binding(self.model, token, name).is_none())
            {
                self.facts.generic_bindings.push(GenericBinding {
                    owner: key.owner.clone(),
                    name: name.clone(),
                    target: key.clone(),
                });
            }
            self.facts.generics.push(def);
            if system == OopSystem::S4 {
                self.implementation(&key, function.clone(), true);
            }
            let generic_value = if system == OopSystem::S3 {
                function
            } else {
                Some(SyntaxElement::Node(node))
            };
            if binding_possible && let Some(value) = generic_value {
                self.generic_values.insert(value.text_range(), key.clone());
            }
            self.site(
                name_value
                    .as_ref()
                    .map_or(call.syntax().text_range(), SyntaxElement::text_range),
                OopSiteRole::Declaration,
                OopTarget::Declaration(key),
            );
        }
    }

    fn string_vector(&self, value: &SyntaxElement) -> Option<StringVector> {
        if value
            .as_token()
            .is_some_and(|token| token.kind() == SyntaxKind::STRING)
        {
            return Some(StringVector {
                entries: vec![(None, string_name(Some(value.clone())), value.text_range())],
                factory: None,
            });
        }
        let call = value.as_node().cloned().and_then(CallExpr::cast)?;
        let (reference, name) = factory(self.model, self.root, &call)?;
        if !accepts(&reference, "base") || name != "c" {
            return None;
        }
        Some(StringVector {
            factory: Some(reference),
            entries: call
                .arg_list()?
                .args()
                .map(|arg| {
                    (
                        arg.name().map(|name| unbacktick(&name).to_string()),
                        string_name(arg.value()),
                        arg.value()
                            .map_or(arg.syntax().text_range(), |value| value.text_range()),
                    )
                })
                .collect(),
        })
    }

    fn generic_value(&self, value: &SyntaxElement, visiting: &mut BTreeSet<u32>) -> Option<OopKey> {
        if let Some(key) = self.generic_values.get(&value.text_range()) {
            return Some(key.clone());
        }
        let token = value.as_token()?;
        if token.kind() != SyntaxKind::IDENT {
            return None;
        }
        let Some(binding) = read_binding(self.model, token) else {
            let owner = owner_at(&token.parent()?, self.owners);
            for depth in (0..=owner.0.len()).rev() {
                let candidates: BTreeSet<_> = self
                    .facts
                    .generic_bindings
                    .iter()
                    .filter(|binding| {
                        binding.name == unbacktick(token.text())
                            && binding.owner.0 == owner.0[..depth]
                            && self.generic_values.iter().any(|(range, key)| {
                                key == &binding.target && range.end() <= token.text_range().start()
                            })
                    })
                    .map(|binding| binding.target.clone())
                    .collect();
                if !candidates.is_empty() {
                    return (candidates.len() == 1)
                        .then(|| candidates.into_iter().next())
                        .flatten();
                }
            }
            return None;
        };
        if visiting.len() >= 64
            || !visiting.insert(binding.0)
            || self.model.binding(binding).def_range.start() >= token.text_range().start()
        {
            return None;
        }
        self.generic_value(&stable_value(self.model, self.root, binding)?, visiting)
    }

    fn generic_bindings(&mut self) {
        for (index, binding) in self.model.bindings().iter().enumerate() {
            let frame = self
                .root
                .covering_element(binding.def_range)
                .parent()
                .and_then(|node| {
                    node.ancestors()
                        .find(|node| node.kind() == SyntaxKind::FUNCTION_EXPR)
                });
            if frame
                .as_ref()
                .is_some_and(|frame| !self.owners.contains_key(&frame.text_range()))
            {
                continue;
            }
            let Some(value) = stable_value(self.model, self.root, BindingId::from_index(index))
            else {
                continue;
            };
            let Some(target) = self.generic_value(&value, &mut BTreeSet::new()) else {
                continue;
            };
            let owner = owner_at(
                &self
                    .root
                    .covering_element(binding.def_range)
                    .parent()
                    .unwrap_or_else(|| self.root.clone()),
                self.owners,
            );
            let fact = GenericBinding {
                owner,
                name: unbacktick(&binding.name).to_string(),
                target: target.clone(),
            };
            if !self.facts.generic_bindings.contains(&fact) {
                self.facts.generic_bindings.push(fact);
            }
            self.site(
                binding.def_range,
                OopSiteRole::Alias,
                OopTarget::Declaration(target),
            );
        }
    }

    fn generic_reference(
        &self,
        value: Option<SyntaxElement>,
        system: OopSystem,
        owner: &ClassOwner,
        by_binding: bool,
    ) -> GenericRef {
        let mut result = GenericRef {
            system,
            owner: owner.clone(),
            name: ClassName::Unknown,
            package: None,
            by_binding,
            local: None,
        };
        let Some(value) = value else {
            return result;
        };
        if let Some(key) = self.generic_value(&value, &mut BTreeSet::new())
            && let Some(def) = self
                .facts
                .generics
                .iter()
                .find(|def| def.key == key && def.key.system == system)
        {
            result.name = def.name.clone();
            result.owner = def.key.owner.clone();
            result.by_binding = false;
            result.local = Some(key);
            return result;
        }
        if by_binding {
            if let Some(namespace) = value
                .as_node()
                .cloned()
                .and_then(BinaryExpr::cast)
                .and_then(|binary| binary.namespace_access())
            {
                result.name = ClassName::Known(namespace.name.to_string());
                result.package = Some(namespace.package.to_string());
                return result;
            }
            // A lexical binding that is not a proven generic must not fall
            // through to a same-named generic in a sourced file or package.
            if value
                .as_token()
                .is_some_and(|token| read_binding(self.model, token).is_some())
            {
                return result;
            }
            result.name = literal(&value).map_or(ClassName::Unknown, ClassName::Known);
        } else {
            result.name = string_name(Some(value));
        }
        result
    }

    fn implementation_signature(
        &self,
        value: Option<SyntaxElement>,
        visiting: &mut BTreeSet<u32>,
    ) -> Option<CallableSignature> {
        signature(self.implementation_value(value, visiting, true))
    }

    fn implementation_value(
        &self,
        value: Option<SyntaxElement>,
        visiting: &mut BTreeSet<u32>,
        quoted_names: bool,
    ) -> Option<SyntaxElement> {
        let value = value?;
        if let Some(paren) = value
            .as_node()
            .cloned()
            .and_then(crate::ast::ParenExpr::cast)
        {
            return self.implementation_value(paren.inner(), visiting, quoted_names);
        }
        let Some(token) = value.as_token().filter(|token| {
            token.kind() == SyntaxKind::IDENT
                || (quoted_names && token.kind() == SyntaxKind::STRING)
        }) else {
            return Some(value);
        };
        if crate::ast::Expr::cast(value.clone()).is_some_and(
            |value| matches!(value, crate::ast::Expr::Name(name) if name.is_reserved_constant()),
        ) {
            return Some(value);
        }
        let binding = read_binding(self.model, token)?;
        if visiting.len() >= 64
            || !visiting.insert(binding.0)
            || self.model.binding(binding).def_range.start() >= token.text_range().start()
        {
            return None;
        }
        self.implementation_value(
            stable_value(self.model, self.root, binding),
            visiting,
            quoted_names,
        )
    }

    fn dispatch(&self, value: Option<SyntaxElement>, system: OopSystem) -> Dispatch {
        let Some(value) = value else {
            return Dispatch {
                arguments: Vec::new(),
                factory: None,
            };
        };
        if system == OopSystem::S7 {
            let list = value
                .as_node()
                .cloned()
                .and_then(CallExpr::cast)
                .filter(|call| {
                    factory(self.model, self.root, call).is_some_and(|(reference, name)| {
                        accepts(&reference, "base") && name == "list"
                    })
                });
            let dispatch_factory = list
                .as_ref()
                .and_then(|call| call_function_ref(self.model, self.root, call));
            let entries: Vec<_> = list
                .and_then(|call| call.arg_list())
                .map(|list| {
                    list.args()
                        .map(|arg| {
                            (
                                arg.name().map(|name| unbacktick(&name).to_string()),
                                arg.value(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_else(|| vec![(None, Some(value.clone()))]);
            return Dispatch {
                factory: dispatch_factory,
                arguments: entries
                    .into_iter()
                    .map(|(argument, entry)| {
                        let range = entry
                            .as_ref()
                            .map_or(value.text_range(), SyntaxElement::text_range);
                        (
                            DispatchArgument {
                                argument,
                                class: entry.as_ref().map_or(DispatchClass::Unknown, |entry| {
                                    DispatchClass::Generator(self.generator(entry))
                                }),
                            },
                            range,
                        )
                    })
                    .collect(),
            };
        }
        let vector = self
            .string_vector(&value)
            .or_else(|| {
                let call = value.as_node().cloned().and_then(CallExpr::cast)?;
                let (reference, name) = factory(self.model, self.root, &call)?;
                if name != "signature" || !accepts(&reference, "methods") {
                    return None;
                }
                Some(StringVector {
                    factory: Some(reference),
                    entries: call
                        .arg_list()?
                        .args()
                        .map(|arg| {
                            (
                                arg.name().map(|name| unbacktick(&name).to_string()),
                                string_name(arg.value()),
                                arg.value()
                                    .map_or(arg.syntax().text_range(), |entry| entry.text_range()),
                            )
                        })
                        .collect(),
                })
            })
            .unwrap_or_else(|| StringVector {
                entries: vec![(None, ClassName::Unknown, value.text_range())],
                factory: None,
            });
        Dispatch {
            factory: vector.factory,
            arguments: vector
                .entries
                .into_iter()
                .map(|(argument, name, range)| {
                    (
                        DispatchArgument {
                            argument,
                            class: match name {
                                ClassName::Known(name) if system == OopSystem::S3 => {
                                    DispatchClass::S3(name)
                                }
                                ClassName::Known(name) if name == "ANY" => DispatchClass::Any,
                                ClassName::Known(name) if name == "missing" => {
                                    DispatchClass::Missing
                                }
                                ClassName::Known(name) => DispatchClass::Formal(name),
                                _ => DispatchClass::Unknown,
                            },
                        },
                        range,
                    )
                })
                .collect(),
        }
    }

    fn registered_methods(&mut self) {
        for (call, reference, name) in factories(self.model, self.root) {
            let (system, formals) = match name.as_str() {
                "setMethod" | "setReplaceMethod" if accepts(&reference, "methods") => (
                    OopSystem::S4,
                    &[
                        "f",
                        "signature",
                        "definition",
                        "where",
                        "valueClass",
                        "sealed",
                    ][..],
                ),
                "method<-" if accepts(&reference, "S7") => {
                    (OopSystem::S7, &["generic", "signature", "value"][..])
                }
                "registerS3method" if accepts(&reference, "base") => {
                    (OopSystem::S3, &["genname", "class", "method", "envir"][..])
                }
                _ => continue,
            };
            let assignment = replacement_assignment(&call);
            let node = assignment
                .as_ref()
                .filter(|_| system == OopSystem::S7)
                .map_or(call.syntax(), |assignment| assignment.syntax());
            let key = self.key(node, system, OopKind::Method);
            let mut issues = uncertainty(node, &reference);
            let args = registration_args(&call, &name, formals);
            if args.is_none() {
                issues.insert(OopUncertainty::InvalidArguments);
            }
            let value = |name: &str| args.as_ref().and_then(|args| argument(args, name));
            let generic_value = value(match system {
                OopSystem::S4 => "f",
                OopSystem::S7 => "generic",
                _ => "genname",
            });
            let mut generic = self.generic_reference(
                generic_value.clone(),
                system,
                &key.owner,
                system == OopSystem::S7,
            );
            if name == "setReplaceMethod"
                && let ClassName::Known(name) = &mut generic.name
            {
                name.push_str("<-");
                // The getter's object proves its name, not the setter's identity.
                generic.local = None;
                generic.owner = key.owner.clone();
            }
            if generic.name == ClassName::Unknown {
                issues.insert(OopUncertainty::DynamicName);
            }
            if generic.by_binding
                && generic.local.is_none()
                && generic_value
                    .as_ref()
                    .and_then(SyntaxElement::as_token)
                    .is_some_and(|token| read_binding(self.model, token).is_some())
            {
                issues.insert(OopUncertainty::UnresolvedAlias);
            }
            if value(if system == OopSystem::S4 {
                "where"
            } else if system == OopSystem::S7 {
                "env"
            } else {
                "envir"
            })
            .is_some()
            {
                issues.insert(OopUncertainty::CustomEnvironment);
            }
            let dispatch = self.dispatch(
                value(if matches!(system, OopSystem::S4 | OopSystem::S7) {
                    "signature"
                } else {
                    "class"
                }),
                system,
            );
            if dispatch.arguments.is_empty()
                || dispatch.arguments.iter().any(|(arg, _)| {
                    matches!(
                        arg.class,
                        DispatchClass::Unknown | DispatchClass::Generator(GeneratorRef::Unknown)
                    )
                })
            {
                issues.insert(OopUncertainty::DynamicSignature);
            }
            let implementation = value(if system == OopSystem::S4 {
                "definition"
            } else if system == OopSystem::S7 {
                "value"
            } else {
                "method"
            });
            let signature =
                self.implementation_signature(implementation.clone(), &mut BTreeSet::new());
            if signature.is_none() {
                issues.insert(OopUncertainty::UnresolvedImplementation);
            }
            if matches!(system, OopSystem::S3 | OopSystem::S4) {
                self.implementation(&key, implementation, true);
            }
            self.facts.methods.push(MethodDef {
                key: key.clone(),
                system,
                name: generic.name.clone(),
                class: None,
                generic: Some(generic.clone()),
                dispatch: dispatch
                    .arguments
                    .iter()
                    .map(|(arg, _)| arg.clone())
                    .collect(),
                dispatch_factory: dispatch.factory,
                signature,
                registration: Some(reference),
                uncertainty: issues,
            });
            self.site(
                call.callee_token()
                    .map_or(call.syntax().text_range(), |token| token.text_range()),
                OopSiteRole::Declaration,
                OopTarget::Declaration(key),
            );
            if let Some(value) = generic_value {
                self.site(
                    value.text_range(),
                    OopSiteRole::Use,
                    OopTarget::Generic(generic),
                );
            }
            for (arg, range) in dispatch.arguments {
                let reference = match arg.class {
                    DispatchClass::Formal(name) => ParentRef::Formal(name),
                    DispatchClass::Generator(generator) => ParentRef::Generator(generator),
                    class
                    @ (DispatchClass::S3(_) | DispatchClass::Any | DispatchClass::Missing) => {
                        self.site(
                            range,
                            OopSiteRole::Use,
                            OopTarget::DispatchClass { system, class },
                        );
                        continue;
                    }
                    _ => continue,
                };
                self.site(
                    range,
                    OopSiteRole::Use,
                    OopTarget::ClassReference(reference),
                );
            }
        }
    }

    fn s3_methods(&mut self) {
        for assignment in self.root.descendants().filter_map(AssignmentExpr::cast) {
            let Some(binding) = assignment.target_name() else {
                continue;
            };
            let binding = unbacktick(&binding);
            let owner = owner_at(assignment.syntax(), self.owners);
            let mut references = BTreeMap::new();
            for generic in &self.facts.generics {
                if generic.key.system != OopSystem::S3 || generic.key.owner != owner {
                    continue;
                }
                let ClassName::Known(name) = &generic.name else {
                    continue;
                };
                if let Some(class) = binding.strip_prefix(&format!("{name}."))
                    && !class.is_empty()
                {
                    references.insert(
                        (name.clone(), class.to_string()),
                        GenericRef {
                            system: OopSystem::S3,
                            owner: owner.clone(),
                            name: generic.name.clone(),
                            package: None,
                            by_binding: false,
                            local: None,
                        },
                    );
                }
            }
            let Some(signature) =
                self.implementation_signature(assignment.value_element(), &mut BTreeSet::new())
            else {
                continue;
            };
            if references.is_empty() {
                // An external generic or NAMESPACE registration may establish
                // dispatch. A name alone proves neither dispatch nor its absence.
                if let Some((name, class)) = binding.rsplit_once('.')
                    && !name.is_empty()
                    && !class.is_empty()
                {
                    references.insert(
                        (name.to_string(), class.to_string()),
                        GenericRef {
                            system: OopSystem::S3,
                            owner: owner.clone(),
                            name: ClassName::Known(name.to_string()),
                            package: None,
                            by_binding: false,
                            local: None,
                        },
                    );
                }
            }
            for ((_, class), generic) in references {
                let implementation = self.implementation_value(
                    assignment.value_element(),
                    &mut BTreeSet::new(),
                    false,
                );
                if self.facts.methods.iter().any(|method| {
                    method.system == OopSystem::S3
                        && method.registration.is_some()
                        && method
                            .generic
                            .as_ref()
                            .is_some_and(|reference| reference.name == generic.name)
                        && method
                            .dispatch
                            .iter()
                            .any(|argument| argument.class == DispatchClass::S3(class.clone()))
                        && self.method_implementations.iter().any(|site| {
                            site.method == method.key
                                && implementation
                                    .as_ref()
                                    .is_some_and(|value| site.function_range == value.text_range())
                        })
                }) {
                    continue;
                }
                let key = self.key(assignment.syntax(), OopSystem::S3, OopKind::Method);
                let mut issues: BTreeSet<_> = self
                    .facts
                    .generics
                    .iter()
                    .filter(|def| {
                        def.key.system == OopSystem::S3
                            && def.key.owner == owner
                            && def.name == generic.name
                    })
                    .flat_map(|def| def.uncertainty.iter().copied())
                    .collect();
                if !self.facts.generics.iter().any(|def| {
                    def.key.system == OopSystem::S3
                        && def.key.owner == owner
                        && def.name == generic.name
                }) {
                    issues.insert(OopUncertainty::UnprovenMethod);
                }
                if !unconditional(assignment.syntax()) {
                    issues.insert(OopUncertainty::Conditional);
                }
                self.facts.methods.push(MethodDef {
                    key: key.clone(),
                    system: OopSystem::S3,
                    name: ClassName::Known(binding.to_string()),
                    class: None,
                    generic: Some(generic),
                    dispatch: vec![DispatchArgument {
                        argument: signature.parameters.first().map(|p| p.name.clone()),
                        class: DispatchClass::S3(class),
                    }],
                    dispatch_factory: None,
                    signature: Some(signature.clone()),
                    registration: None,
                    uncertainty: issues,
                });
                self.implementation(&key, assignment.value_element(), false);
                if let Some(token) = assignment.target_name_token() {
                    self.site(
                        token.text_range(),
                        OopSiteRole::Declaration,
                        OopTarget::Declaration(key),
                    );
                }
            }
        }
    }

    fn use_sites(&mut self) {
        for (range, occurrence) in &self.classes.occurrences {
            match occurrence {
                ClassOccurrence::Declaration(key) => self.site(
                    *range,
                    OopSiteRole::Declaration,
                    OopTarget::Class(key.clone()),
                ),
                ClassOccurrence::Reference { reference, .. } => self.site(
                    *range,
                    OopSiteRole::Use,
                    OopTarget::ClassReference(reference.clone()),
                ),
            }
        }
        for (index, binding) in self.model.bindings().iter().enumerate() {
            let Some(value) = stable_value(self.model, self.root, BindingId::from_index(index))
            else {
                continue;
            };
            if let GeneratorRef::Local(key) = self.generator(&value)
                && !self.sites.iter().any(|site| {
                    site.range == binding.def_range && site.role == OopSiteRole::Declaration
                })
            {
                self.site(binding.def_range, OopSiteRole::Alias, OopTarget::Class(key));
            }
        }
        for ident in self.model.idents() {
            if self.sites.iter().any(|site| site.range == ident.range) {
                continue;
            }
            let Some(token) = self.root.covering_element(ident.range).into_token() else {
                continue;
            };
            let value = SyntaxElement::Token(token);
            if let GeneratorRef::Local(key) = self.generator(&value) {
                self.site(
                    ident.range,
                    OopSiteRole::Use,
                    OopTarget::ClassReference(ParentRef::Generator(GeneratorRef::Local(key))),
                );
            } else if let Some(key) = self.generic_value(&value, &mut BTreeSet::new()) {
                self.site(ident.range, OopSiteRole::Use, OopTarget::Declaration(key));
            }
        }
        for binary in self.root.descendants().filter_map(BinaryExpr::cast) {
            let slot = match binary.op_kind() {
                Some(SyntaxKind::DOLLAR) => false,
                Some(SyntaxKind::AT) => true,
                _ => continue,
            };
            let Some(rhs) = binary.rhs() else {
                continue;
            };
            let Some(name) = literal(&rhs) else {
                continue;
            };
            let Some(lhs) = binary.lhs() else {
                continue;
            };
            let implicit = literal(&lhs)
                .filter(|name| matches!(name.as_str(), "self" | "private" | "super" | ".self"))
                .filter(|_| {
                    lhs.as_token()
                        .is_some_and(|token| read_binding(self.model, token).is_none())
                })
                .and_then(|name| {
                    self.method_bodies
                        .iter()
                        .filter(|(range, _)| range.contains_range(binary.syntax().text_range()))
                        .min_by_key(|(range, _)| range.len())
                        .map(|(_, class)| MemberReceiver::Implicit {
                            class: class.clone(),
                            name,
                        })
                });
            let receiver = implicit.unwrap_or_else(|| match self.generator(&lhs) {
                GeneratorRef::Unknown => MemberReceiver::Unknown,
                GeneratorRef::Free(name) => MemberReceiver::Binding(name),
                generator => MemberReceiver::Generator(generator),
            });
            self.site(
                rhs.text_range(),
                OopSiteRole::Use,
                OopTarget::Member {
                    receiver,
                    name,
                    slot,
                },
            );
        }
        for call in self.root.descendants().filter_map(CallExpr::cast) {
            let Some((reference, name)) = factory(self.model, self.root, &call) else {
                continue;
            };
            if name == "new" && accepts(&reference, "methods") {
                let value = matched_args(&call, &["Class", "..."])
                    .and_then(|args| argument(&args, "Class"));
                if let Some(value) = value
                    && let ClassName::Known(name) = string_name(Some(value.clone()))
                {
                    self.site(
                        value.text_range(),
                        OopSiteRole::Use,
                        OopTarget::ClassReference(ParentRef::Formal(name)),
                    );
                }
            } else if accepts(&reference, "base") {
                let value = if name == "structure" {
                    call.arg_list()
                        .and_then(|list| {
                            list.args()
                                .find(|arg| arg.name().as_deref() == Some("class"))
                        })
                        .and_then(|arg| arg.value())
                } else if name == "class<-" {
                    replacement_assignment(&call)
                        .and_then(|assignment| assignment.value_element())
                        .or_else(|| {
                            matched_args(&call, &["x", "value"])
                                .and_then(|args| argument(&args, "value"))
                        })
                } else {
                    None
                };
                if let Some(value) = value
                    && let Some(classes) = self.string_vector(&value)
                {
                    for (_, class, range) in classes.entries {
                        if let ClassName::Known(name) = class {
                            // S3 tags are uses of a dispatch name, not declarations
                            // of a unique class object or a constructor binding.
                            self.site(
                                range,
                                OopSiteRole::Use,
                                OopTarget::DispatchClass {
                                    system: OopSystem::S3,
                                    class: DispatchClass::S3(name),
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}
