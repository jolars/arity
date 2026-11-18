use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7Property {
    pub name: String,
    pub declaration: Option<OopId>,
    pub owner: ClassId,
    pub definition: S7PropertyDef,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S7Api {
    /// R preserves declaration order when overlaying inherited properties.
    pub properties: Vec<S7Property>,
    pub complete: bool,
    pub constructor: Option<CallableSignature>,
    inlineable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S7PropertyUse {
    pub receiver: ClassReceiver,
    pub names: Vec<String>,
    pub write: bool,
    pub complete: bool,
}

pub(in crate::project::classes) struct PropertyCall {
    pub object: SyntaxElement,
    pub names: Option<Vec<String>>,
    pub write: bool,
    pub single: bool,
    pub complete: bool,
}

impl S7Property {
    fn writable(&self) -> Option<bool> {
        match (&self.definition.getter, &self.definition.setter) {
            (_, S7Function::Known { .. }) | (S7Function::Absent, S7Function::Absent) => Some(true),
            (S7Function::Known { .. }, S7Function::Absent) => Some(false),
            _ => None,
        }
    }

    fn parameter(&self, index: &ClassIndex) -> Option<CallableParameter> {
        if !self.writable()? {
            return None;
        }
        let default = match (&self.definition.default, &self.definition.class) {
            (Some(default), _) => Some(default.clone()),
            (_, None) => Some("NULL".into()),
            (_, Some(spec)) => index.s7_class_default(&self.owner, spec)?,
        };
        Some(CallableParameter {
            name: self.name.clone(),
            default,
        })
    }

    pub fn declared_type(&self, index: &ClassIndex) -> Option<String> {
        index.s7_type_label(&self.owner, self.definition.class.as_ref()?)
    }

    pub(in crate::project::classes) fn access(&self) -> MemberAccess {
        let accessor = |function: &S7Function, argument| match function {
            S7Function::Absent => MemberEffect::Stored,
            S7Function::Known { key, .. } => MemberEffect::InvokeAccessor {
                implementation: key.clone().map(|key| OopId {
                    path: self.owner.path.clone(),
                    key,
                }),
                argument,
            },
            S7Function::Unknown => MemberEffect::Unknown,
        };
        MemberAccess {
            read: accessor(&self.definition.getter, AccessorArgument::Missing),
            write: if self.writable() == Some(false) {
                MemberEffect::ReadOnly
            } else {
                accessor(&self.definition.setter, AccessorArgument::AssignedValue)
            },
        }
    }
}

fn overlay<T>(values: &mut Vec<T>, new: impl IntoIterator<Item = T>, name: impl Fn(&T) -> &str) {
    for value in new {
        if let Some(existing) = values
            .iter_mut()
            .find(|existing| name(existing) == name(&value))
        {
            *existing = value;
        } else {
            values.push(value);
        }
    }
}

impl ClassIndex {
    pub fn s7_new_object_class(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Option<ClassId> {
        self.s7_new_object_in(
            path,
            root,
            model,
            call,
            &method_implementation_sites(model, root),
        )
    }

    pub(in crate::project::classes) fn s7_new_object_in(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
        implementations: &[MethodImplementationSite],
    ) -> Option<ClassId> {
        let file = self.file(path)?;
        if file.facts.oop.s7.classes.is_empty()
            || self
                .s7_helper(path, &call_function_ref(model, root, call)?, "S7")
                .as_deref()
                != Some("new_object")
        {
            return None;
        }
        let function = call.syntax().ancestors().find_map(FunctionExpr::cast)?;
        let parent = if self.s7_version(path) == S7Version::Development {
            "_parent"
        } else {
            ".parent"
        };
        argument(&matched_args(call, &[parent, "..."])?, parent)?;
        let mut candidates = file.facts.oop.s7.classes.iter().filter(|(_, class)| {
            class.abstract_class == Some(false)
                && matches!(&class.constructor,
                    S7Function::Known { key: Some(key), .. } if implementations.iter().any(|site| {
                        site.method == *key && site.function_range == function.syntax().text_range()
                    })
                )
        });
        let (key, _) = candidates.next()?;
        (candidates.next().is_none()).then(|| ClassId {
            path: path.to_path_buf(),
            key: key.clone(),
        })
    }

    pub fn s7_api(&self, id: &ClassId) -> Option<S7Api> {
        self.s7_api_inner(id, &mut BTreeSet::new())
    }

    fn s7_api_inner(&self, id: &ClassId, visiting: &mut BTreeSet<ClassId>) -> Option<S7Api> {
        let class = self.s7_class(id)?;
        if visiting.len() >= 64 || !visiting.insert(id.clone()) {
            return None;
        }
        let file = self.file(&id.path)?;
        let version = file.facts.oop.s7.version;
        let parent_id = class
            .parent
            .as_ref()
            .and_then(|spec| self.s7_spec_class(&id.path, &id.key.owner, spec));
        let (mut parent, abstract_parent, formal_parent) = match (&class.parent, &parent_id) {
            (None, _) => (Self::s7_root_api(), false, false),
            (_, Some(parent)) if self.s7_class(parent).is_some() => (
                self.s7_api_inner(parent, visiting).unwrap_or_default(),
                self.s7_class(parent)
                    .and_then(|class| class.abstract_class)
                    .unwrap_or(false),
                false,
            ),
            (_, Some(parent))
                if version == S7Version::Development
                    && matches!(parent.key.system, ClassSystem::S4 | ClassSystem::RefClass) =>
            {
                (
                    self.supertypes
                        .get(id)
                        .filter(|parents| parents.contains(parent))
                        .and_then(|_| self.s7_from_s4_slots(parent))
                        .unwrap_or_default(),
                    false,
                    true,
                )
            }
            (Some(S7ClassSpec::Builtin(reference)), _) => (
                self.s7_helper(&id.path, reference, "S7")
                    .and_then(|name| self.s7_base_api(&name, version))
                    .unwrap_or_default(),
                false,
                false,
            ),
            (
                Some(S7ClassSpec::S3 {
                    factory,
                    vector,
                    constructor,
                    ..
                }),
                _,
            ) if self.s7_helper(&id.path, factory, "S7").is_some()
                && vector.as_ref().is_none_or(|reference| {
                    self.s7_helper(&id.path, reference, "base").is_some()
                }) =>
            {
                (
                    S7Api {
                        properties: Vec::new(),
                        complete: true,
                        constructor: constructor.signature().cloned(),
                        inlineable: false,
                    },
                    false,
                    false,
                )
            }
            _ => (S7Api::default(), false, false),
        };
        visiting.remove(id);
        let mut own = Vec::new();
        let mut names = BTreeSet::new();
        let mut complete = parent.complete && self.declarations[id].declaration.binding_known();
        for group in file
            .facts
            .oop
            .member_groups
            .iter()
            .filter(|group| group.class == id.key && group.name == "properties")
        {
            if group.uncertainty.contains(&OopUncertainty::DynamicMembers)
                || group
                    .factory
                    .as_ref()
                    .is_some_and(|factory| self.s7_helper(&id.path, factory, "base").is_none())
            {
                complete = false;
                // The apparent entries came from a computed list.
                own.clear();
                break;
            }
            for member in file.facts.oop.members.iter().filter(|member| {
                member.class.as_ref() == Some(&id.key) && member.kind == MemberKind::Property
            }) {
                let Some(definition) = file.facts.oop.s7.properties.get(&member.key) else {
                    complete = false;
                    continue;
                };
                let ClassName::Known(name) = &member.name else {
                    complete = false;
                    continue;
                };
                let reserved = name == "..."
                    || match version {
                        S7Version::Development => name.starts_with('_'),
                        S7Version::Release => matches!(
                            name.as_str(),
                            "names"
                                | "dim"
                                | "dimnames"
                                | "class"
                                | "tsp"
                                | "comment"
                                | "row.names"
                        ),
                    };
                if reserved || !names.insert(name.clone()) {
                    complete = false;
                    continue;
                }
                let mut definition = definition.clone();
                let known_shorthand = definition.factory.is_some()
                    || definition.class.as_ref().is_some_and(|class| {
                        self.s7_spec_tags(&id.path, &id.key.owner, class).is_some()
                    });
                if !definition.complete
                    || !known_shorthand
                    || definition
                        .factory
                        .as_ref()
                        .is_some_and(|factory| self.s7_helper(&id.path, factory, "S7").is_none())
                {
                    complete = false;
                    definition.class = Some(S7ClassSpec::Unknown);
                    definition.getter = S7Function::Unknown;
                    definition.setter = S7Function::Unknown;
                }
                own.push(S7Property {
                    name: name.clone(),
                    declaration: Some(OopId {
                        path: id.path.clone(),
                        key: member.key.clone(),
                    }),
                    owner: id.clone(),
                    definition,
                });
            }
        }
        let constructor = match &class.constructor {
            S7Function::Known { signature, .. } => Some(signature.clone()),
            S7Function::Absent if complete => {
                let external_parent = parent_id
                    .as_ref()
                    .and_then(|parent| self.s7_package(id).zip(self.s7_package(parent)))
                    .is_some_and(|(child, parent)| child != parent);
                let forwards = version == S7Version::Development
                    && !abstract_parent
                    && !formal_parent
                    && (!parent.inlineable
                        || external_parent
                        || matches!(class.parent, Some(S7ClassSpec::External { .. })));
                if abstract_parent || formal_parent {
                    let mut effective = parent.properties.clone();
                    overlay(&mut effective, own.iter().cloned(), |p| &p.name);
                    self.s7_constructor_parameters(effective.iter())
                        .map(|parameters| CallableSignature { parameters })
                } else if forwards {
                    self.s7_constructor_parameters(own.iter()).map(|own| {
                        let mut parameters = vec![CallableParameter {
                            name: "...".into(),
                            default: None,
                        }];
                        parameters.extend(own);
                        CallableSignature { parameters }
                    })
                } else {
                    let inherited = parent.constructor.take();
                    let eligible = own.iter().filter(|property| {
                        version == S7Version::Development
                            || !parent
                                .properties
                                .iter()
                                .any(|inherited| inherited.name == property.name)
                    });
                    inherited.zip(self.s7_constructor_parameters(eligible)).map(
                        |(mut signature, own)| {
                            overlay(&mut signature.parameters, own, |parameter| &parameter.name);
                            signature
                        },
                    )
                }
            }
            _ => None,
        };
        overlay(&mut parent.properties, own, |property| &property.name);
        Some(S7Api {
            properties: parent.properties,
            complete,
            constructor,
            inlineable: class.constructor == S7Function::Absent,
        })
    }

    fn s7_constructor_parameters<'a>(
        &self,
        properties: impl Iterator<Item = &'a S7Property>,
    ) -> Option<Vec<CallableParameter>> {
        let mut parameters = Vec::new();
        for property in properties {
            if property.writable()? {
                parameters.push(property.parameter(self)?);
            }
        }
        Some(parameters)
    }

    fn s7_root_api() -> S7Api {
        S7Api {
            properties: Vec::new(),
            complete: true,
            constructor: Some(CallableSignature {
                parameters: Vec::new(),
            }),
            inlineable: true,
        }
    }

    fn s7_base_api(&self, name: &str, version: S7Version) -> Option<S7Api> {
        if name == "S7_object" {
            return Some(Self::s7_root_api());
        }
        if matches!(
            name,
            "class_any" | "class_numeric" | "class_vector" | "class_atomic" | "class_missing"
        ) || name == "class_environment" && version == S7Version::Release
        {
            return None;
        }
        let formals = match name {
            "class_Date" => vec![(".data", Some("double()"))],
            "class_data.frame" => vec![(".data", Some("list()")), ("row.names", Some("NULL"))],
            "class_factor" => vec![(".data", Some("integer()")), ("levels", Some("NULL"))],
            "class_formula" => vec![(".data", Some("NULL")), ("env", Some("parent.frame()"))],
            "class_POSIXct" => vec![(".data", Some("double()")), ("tz", Some("\"\""))],
            "class_POSIXlt" => vec![(".data", Some("NULL")), ("tz", Some("\"\""))],
            "class_POSIXt" => vec![(".data", None)],
            _ => vec![(".data", Some(builtin_default(name)?))],
        };
        Some(S7Api {
            constructor: Some(CallableSignature {
                parameters: formals
                    .into_iter()
                    .map(|(name, default)| CallableParameter {
                        name: name.into(),
                        default: default.map(str::to_owned),
                    })
                    .collect(),
            }),
            ..Self::s7_root_api()
        })
    }

    fn s7_from_s4_slots(&self, parent: &ClassId) -> Option<S7Api> {
        let slots = self.s4_slots(parent)?;
        let mut complete = slots.complete;
        let mut properties = Vec::new();
        for (name, entries) in slots.into_ordered_entries() {
            let [slot] = entries.as_slice() else {
                complete = false;
                continue;
            };
            let class = match &slot.declared_type {
                DispatchClass::Formal(name) => S7ClassSpec::Builtin(FunctionRef::Qualified {
                    package: "S7".into(),
                    name: format!("class_{name}"),
                }),
                DispatchClass::Any => S7ClassSpec::Builtin(FunctionRef::Qualified {
                    package: "S7".into(),
                    name: "class_any".into(),
                }),
                _ => S7ClassSpec::Unknown,
            };
            properties.push(S7Property {
                name,
                declaration: slot.declaration.clone(),
                owner: parent.clone(),
                definition: S7PropertyDef {
                    class: Some(class),
                    default: None,
                    getter: S7Function::Absent,
                    setter: S7Function::Absent,
                    validator: S7Function::Absent,
                    factory: None,
                    complete: true,
                },
            });
        }
        Some(S7Api {
            properties,
            complete,
            ..Self::s7_root_api()
        })
    }

    fn s7_class_default(&self, owner: &ClassId, spec: &S7ClassSpec) -> Option<Option<String>> {
        Some(match spec {
            S7ClassSpec::Null => Some("NULL".into()),
            S7ClassSpec::Builtin(reference) => {
                match self.s7_helper(&owner.path, reference, "S7")?.as_str() {
                    "class_missing" => None,
                    "S7_object" => Some("S7::S7_object()".into()),
                    name => Some(builtin_default(name).map(str::to_owned).or_else(|| {
                        builtin_s3_property_default(name, self.s7_version(&owner.path))
                    })?),
                }
            }
            S7ClassSpec::Union { factory, classes }
                if self.s7_union_origin(&owner.path, factory) =>
            {
                return self.s7_class_default(owner, classes.first()?);
            }
            S7ClassSpec::S3 {
                factory,
                default: Some(default),
                ..
            } if self.s7_helper(&owner.path, factory, "S7").is_some() => Some(default.clone()),
            S7ClassSpec::Generator(_) | S7ClassSpec::External { .. } => {
                let target = self.s7_spec_class(&owner.path, &owner.key.owner, spec)?;
                Some(format!("{}()", self.s7_class_tag(&target)?))
            }
            _ => return None,
        })
    }

    fn s7_type_label(&self, owner: &ClassId, spec: &S7ClassSpec) -> Option<String> {
        match spec {
            S7ClassSpec::Builtin(reference) => Some(
                self.s7_helper(&owner.path, reference, "S7")?
                    .trim_start_matches("class_")
                    .to_string(),
            ),
            S7ClassSpec::Union { factory, classes }
                if self.s7_union_origin(&owner.path, factory) =>
            {
                Some(
                    classes
                        .iter()
                        .map(|spec| self.s7_type_label(owner, spec))
                        .collect::<Option<Vec<_>>>()?
                        .join(" | "),
                )
            }
            S7ClassSpec::S3 { classes, .. } => classes.first().cloned(),
            S7ClassSpec::Null => Some("NULL".into()),
            _ => {
                let class = self.s7_spec_class(&owner.path, &owner.key.owner, spec)?;
                self.s7_class_tag(&class).or_else(|| {
                    Some(
                        self.declarations
                            .get(&class)?
                            .declaration
                            .display_name()
                            .to_string(),
                    )
                })
            }
        }
    }

    pub(in crate::project::classes) fn s7_property_classes(
        &self,
        receiver: &ClassReceiver,
        name: &str,
    ) -> Option<(BTreeSet<ClassId>, bool)> {
        let api = self.s7_api(&receiver.class)?;
        if !api.complete || receiver.access == ClassAccess::Generator {
            return None;
        }
        let property = api
            .properties
            .iter()
            .find(|property| property.name == name)?;
        if property.definition.getter != S7Function::Absent {
            return None;
        }
        Some(self.s7_spec_candidates(
            &property.owner.path,
            &property.owner.key.owner,
            property.definition.class.as_ref()?,
        ))
    }

    pub fn s7_property_use(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: &SyntaxElement,
    ) -> Option<S7PropertyUse> {
        let node = value.as_node()?.clone();
        if let Some(binary) = BinaryExpr::cast(node.clone())
            && binary.op_kind() == Some(SyntaxKind::AT)
        {
            let receiver = self.class_receiver(path, root, model, &binary.lhs()?)?;
            let name = resolve::namespace_name(binary.rhs()?)?;
            let write = node
                .parent()
                .and_then(AssignmentExpr::cast)
                .and_then(|assignment| assignment.target_element())
                == Some(SyntaxElement::Node(node));
            return (receiver.class.key.system == ClassSystem::S7
                && receiver.access != ClassAccess::Generator)
                .then_some(S7PropertyUse {
                    receiver,
                    names: vec![name],
                    write,
                    complete: true,
                });
        }
        let call = CallExpr::cast(node)?;
        let property_call = self.s7_property_call(path, root, model, &call)?;
        let mut receiver = self.class_receiver(path, root, model, &property_call.object)?;
        if receiver.access == ClassAccess::Generator {
            return None;
        }
        if receiver.class.key.system != ClassSystem::S7 {
            receiver.class = self.s7_s4_parents.get(&receiver.class)?.clone();
        }
        let names = match property_call.names {
            Some(names) => names,
            None => self
                .s7_api(&receiver.class)?
                .properties
                .into_iter()
                .map(|property| property.name)
                .collect(),
        };
        Some(S7PropertyUse {
            receiver,
            names,
            write: property_call.write,
            complete: property_call.complete,
        })
    }

    pub(in crate::project::classes) fn s7_property_call(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        call: &CallExpr,
    ) -> Option<PropertyCall> {
        let reference = call_function_ref(model, root, call)?;
        let name = self.s7_helper(path, &reference, "S7")?;
        let version = self.s7_version(path);
        let object = if name == "set_props" && version == S7Version::Development {
            "_object"
        } else {
            "object"
        };
        let formals = match name.as_str() {
            "prop" => &["object", "name"][..],
            "prop<-" => &["object", "name", "check", "value"][..],
            "props" => &["object", "names"][..],
            "props<-" => &["object", "check", "value"][..],
            "set_props" => &[object, "...", ".check"][..],
            _ => return None,
        };
        let args = registration_args(call, &name, formals)?;
        let receiver = argument(&args, object)?;
        let write = matches!(name.as_str(), "prop<-" | "props<-" | "set_props");
        let (names, complete) = match name.as_str() {
            "prop" | "prop<-" => match argument(&args, "name").and_then(string_literal) {
                Some(name) => (Some(vec![name]), true),
                None => (Some(Vec::new()), false),
            },
            "props" => match argument(&args, "names") {
                None => (None, true),
                Some(value) => match self.s7_literal_vector(path, root, model, value) {
                    Some(names) => (Some(names), true),
                    None => (Some(Vec::new()), false),
                },
            },
            "props<-" => {
                let names = argument(&args, "value")
                    .and_then(SyntaxElement::into_node)
                    .and_then(CallExpr::cast)
                    .filter(|value| {
                        call_function_ref(model, root, value).is_some_and(|reference| {
                            self.s7_helper(path, &reference, "base").as_deref() == Some("list")
                        })
                    })
                    .and_then(|call| {
                        call.arg_list()?
                            .args()
                            .map(|arg| arg.name().map(|name| unbacktick(&name).to_string()))
                            .collect::<Option<Vec<_>>>()
                    });
                let complete = names.is_some();
                (Some(names.unwrap_or_default()), complete)
            }
            _ => {
                let names = call
                    .arg_list()?
                    .args()
                    .filter_map(|arg| arg.name().map(|name| name.to_string()))
                    .filter(|name| name != object && name != ".check")
                    .collect();
                (Some(names), false)
            }
        };
        Some(PropertyCall {
            object: receiver,
            names,
            write,
            single: name == "prop",
            complete,
        })
    }

    fn s7_literal_vector(
        &self,
        path: &Path,
        root: &SyntaxNode,
        model: &SemanticModel,
        value: SyntaxElement,
    ) -> Option<Vec<String>> {
        if constant_atom(&value, "NULL") {
            return Some(Vec::new());
        }
        if let Some(name) = string_literal(value.clone()) {
            return Some(vec![name]);
        }
        let call = CallExpr::cast(value.into_node()?)?;
        let helper = self.s7_helper(path, &call_function_ref(model, root, &call)?, "base")?;
        if helper == "c" {
            call.arg_list()?
                .args()
                .map(|arg| string_literal(arg.value()?))
                .collect()
        } else if helper == "character" && call.arg_list()?.args().next().is_none() {
            Some(Vec::new())
        } else {
            None
        }
    }
}

pub(super) fn string_literal(value: SyntaxElement) -> Option<String> {
    let token = value.into_token()?;
    (token.kind() == SyntaxKind::STRING)
        .then(|| crate::semantic::decode_string_literal(token.text()))
        .flatten()
}
