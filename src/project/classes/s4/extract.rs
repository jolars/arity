use super::*;

pub(in super::super) fn formals(name: &str) -> &'static [&'static str] {
    match name {
        "setClassUnion" => &["name", "members", "where"],
        "setOldClass" => &["Classes", "prototype", "where", "test", "S4Class"],
        _ => ClassSystem::S4.formals(),
    }
}

fn string(value: Option<SyntaxElement>) -> ClassName {
    value
        .and_then(SyntaxElement::into_token)
        .filter(|token| token.kind() == SyntaxKind::STRING)
        .and_then(|token| crate::semantic::decode_string_literal(token.text()))
        .map_or(ClassName::Unknown, ClassName::Known)
}

fn non_null(value: Option<SyntaxElement>) -> bool {
    value.is_some_and(
        |value| !matches!(value, SyntaxElement::Token(token) if token.text() == "NULL"),
    )
}

fn flag(value: Option<SyntaxElement>, default: bool) -> Option<bool> {
    match value {
        None => Some(default),
        Some(SyntaxElement::Token(token)) if token.text() == "TRUE" => Some(true),
        Some(SyntaxElement::Token(token)) if token.text() == "FALSE" => Some(false),
        _ => None,
    }
}

struct Entries {
    values: Vec<SlotEntry>,
    helpers: Vec<FunctionRef>,
}

struct ClassVector {
    entries: Vec<(ClassName, TextRange)>,
    helpers: Vec<FunctionRef>,
}

fn entries(model: &SemanticModel, root: &SyntaxNode, value: &SyntaxElement) -> Option<Entries> {
    if value
        .as_token()
        .is_some_and(|token| token.kind() == SyntaxKind::STRING)
    {
        return Some(Entries {
            values: vec![SlotEntry {
                name: None,
                value: Some(value.clone()),
                range: value.text_range(),
                untyped: false,
            }],
            helpers: Vec::new(),
        });
    }
    let call = value.as_node().cloned().and_then(CallExpr::cast)?;
    let reference = call_function_ref(model, root, &call)?;
    let identities = possible_functions(&reference);
    let [identity] = identities.as_slice() else {
        return None;
    };
    let empty = identity.package == "base"
        && identity.name == "character"
        && call.arg_list()?.args().next().is_none();
    if !empty
        && !matches!(
            (identity.package.as_str(), identity.name.as_str()),
            ("base", "c" | "list") | ("methods", "representation" | "prototype")
        )
    {
        return None;
    }
    if !call
        .syntax()
        .children_with_tokens()
        .any(|token| token.kind() == SyntaxKind::RPAREN)
    {
        return None;
    }
    let values = call
        .arg_list()?
        .args()
        .map(|arg| {
            let value = arg.value()?;
            if value.as_token().is_some_and(|token| token.text() == "...") {
                return None;
            }
            Some(SlotEntry {
                name: arg.name().map(|name| unbacktick(&name).to_string()),
                range: arg
                    .name_token()
                    .map_or(value.text_range(), |token| token.text_range()),
                value: Some(value),
                untyped: false,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Entries {
        values,
        helpers: vec![reference],
    })
}

fn vector(model: &SemanticModel, root: &SyntaxNode, value: &SyntaxElement) -> Option<ClassVector> {
    let entries = entries(model, root, value)?;
    if entries.helpers.iter().any(|reference| {
        possible_functions(reference).iter().any(|identity| {
            identity.package != "base" || !matches!(identity.name.as_str(), "c" | "character")
        })
    }) {
        return None;
    }
    Some(ClassVector {
        entries: entries
            .values
            .into_iter()
            .map(|entry| {
                let range = entry
                    .value
                    .as_ref()
                    .map_or(entry.range, SyntaxElement::text_range);
                (string(entry.value), range)
            })
            .collect(),
        helpers: entries.helpers,
    })
}

pub(in super::super) fn declaration_count(
    model: &SemanticModel,
    root: &SyntaxNode,
    call: &CallExpr,
    name: &str,
) -> usize {
    if name != "setOldClass" {
        return 1;
    }
    matched_args(call, formals(name))
        .and_then(|args| argument(&args, "Classes"))
        .and_then(|value| vector(model, root, &value))
        .map_or(1, |classes| classes.entries.len().max(1))
}

fn default_value(value: Option<SyntaxElement>) -> S4Default {
    let Some(SyntaxElement::Token(token)) = value else {
        return S4Default::Unknown;
    };
    if token.kind() == SyntaxKind::STRING {
        return crate::semantic::decode_string_literal(token.text())
            .map_or(S4Default::Unknown, S4Default::String);
    }
    if matches!(
        token.kind(),
        SyntaxKind::INT | SyntaxKind::FLOAT | SyntaxKind::COMPLEX
    ) || matches!(
        token.text(),
        "TRUE"
            | "FALSE"
            | "NULL"
            | "NA"
            | "NA_real_"
            | "NA_integer_"
            | "NA_character_"
            | "NA_complex_"
            | "Inf"
            | "NaN"
    ) {
        S4Default::Literal(token.text().into())
    } else {
        S4Default::Unknown
    }
}

fn prototype(
    model: &SemanticModel,
    root: &SyntaxNode,
    value: Option<SyntaxElement>,
) -> S4Prototype {
    let Some(value) = value else {
        return S4Prototype {
            complete: true,
            ..Default::default()
        };
    };
    let mut result = S4Prototype {
        provided: true,
        complete: true,
        ..Default::default()
    };
    if let Some(entries) = entries(model, root, &value)
        && entries.helpers.iter().all(|helper| {
            possible_functions(helper).iter().all(|identity| {
                matches!(
                    (identity.package.as_str(), identity.name.as_str()),
                    ("base", "list") | ("methods", "prototype")
                )
            })
        })
        && !entries.helpers.is_empty()
    {
        result.helpers = entries.helpers;
        for entry in entries.values {
            let value = default_value(entry.value);
            result.complete &= value != S4Default::Unknown;
            if let Some(name) = entry.name {
                if result.slots.insert(name, value).is_some() {
                    result.complete = false;
                }
            } else if result.data.replace(value).is_some() {
                result.complete = false;
            }
        }
    } else {
        result.data = Some(default_value(Some(value)));
        result.complete = result.data != Some(S4Default::Unknown);
    }
    result
}

fn old_class_virtuality(
    model: &SemanticModel,
    root: &SyntaxNode,
    value: Option<SyntaxElement>,
    classes: &[(ClassName, TextRange)],
    helpers: &mut Vec<FunctionRef>,
    depth: usize,
) -> Option<bool> {
    if depth >= 64 {
        return None;
    }
    let Some(value) = value else {
        return Some(true);
    };
    if value.as_token().is_some() {
        return match default_value(Some(value)) {
            S4Default::Literal(value) if value == "NULL" => Some(true),
            S4Default::Unknown => None,
            _ => Some(false),
        };
    }
    let call = value.into_node().and_then(CallExpr::cast)?;
    let reference = call_function_ref(model, root, &call)?;
    let identities = possible_functions(&reference);
    let [identity] = identities.as_slice() else {
        return None;
    };
    if identity.package != "base" {
        return None;
    }
    match identity.name.as_str() {
        "list" => {
            matched_args(&call, &["..."])?;
            helpers.push(reference);
            Some(false)
        }
        "structure" => {
            let args = matched_args(&call, &[".Data", "..."])?;
            let data = argument(&args, ".Data")?;
            // Only recognize a literal class vector matching this registration.
            // Computed attributes can change whether the prototype is valid.
            for arg in call.arg_list()?.args() {
                if arg.name().is_some_and(|name| name == "class") {
                    let names = vector(model, root, &arg.value()?)?;
                    if names
                        .entries
                        .iter()
                        .map(|(name, _)| name)
                        .ne(classes.iter().map(|(name, _)| name))
                        || names
                            .entries
                            .iter()
                            .any(|(name, _)| *name == ClassName::Unknown)
                    {
                        return None;
                    }
                    helpers.extend(names.helpers);
                }
            }
            // structure(NULL, ...) has special coercion rules. Keep that case
            // uncertain instead of assuming it remains NULL.
            if old_class_virtuality(model, root, Some(data), classes, helpers, depth + 1)? {
                return None;
            }
            helpers.push(reference);
            Some(false)
        }
        _ => None,
    }
}

pub(in super::super) fn class_shape(
    model: &SemanticModel,
    root: &SyntaxNode,
    call: &CallExpr,
    factory: &str,
    ordinal: usize,
) -> Option<ClassShape> {
    let args = matched_args(call, formals(factory))?;
    let name_value = argument(
        &args,
        match factory {
            "setClassUnion" => "name",
            "setOldClass" => "Classes",
            _ => "Class",
        },
    );
    let mut shape = ClassShape {
        name: string(name_value.clone()),
        name_range: name_value.as_ref().map(SyntaxElement::text_range),
        parents: Vec::new(),
        slots: Vec::new(),
        metadata: S4Class {
            definition: match factory {
                "setClassUnion" => S4Definition::Union,
                "setOldClass" => S4Definition::OldClass,
                _ => S4Definition::Class,
            },
            virtual_class: Some(false),
            sealed: flag(argument(&args, "sealed"), false),
            package: argument(&args, "package")
                .map_or(ClassName::Absent, |value| string(Some(value))),
            prototype: prototype(model, root, argument(&args, "prototype")),
            validity: non_null(argument(&args, "validity")),
            schema_helpers: Vec::new(),
            slot_helpers: Vec::new(),
            slots_complete: true,
            parents_complete: true,
            union_members: Vec::new(),
            old_class: None,
        },
    };
    if factory == "setClassUnion" {
        shape.metadata.virtual_class = Some(true);
        if let Some(members) =
            argument(&args, "members").and_then(|value| vector(model, root, &value))
        {
            shape.metadata.union_members =
                members.entries.into_iter().map(|(name, _)| name).collect();
            shape.metadata.schema_helpers = members.helpers;
        } else {
            shape.metadata.union_members.push(ClassName::Unknown);
            shape.metadata.parents_complete = false;
        }
        return Some(shape);
    }
    if factory == "setOldClass" {
        let classes = name_value
            .as_ref()
            .and_then(|value| vector(model, root, value))
            .unwrap_or_else(|| ClassVector {
                entries: vec![(ClassName::Unknown, call.syntax().text_range())],
                helpers: Vec::new(),
            });
        let (name, range) = classes.entries.get(ordinal)?;
        shape.name = name.clone();
        shape.name_range = Some(*range);
        shape.metadata.schema_helpers = classes.helpers;
        shape.metadata.virtual_class = if ordinal == 0 {
            old_class_virtuality(
                model,
                root,
                argument(&args, "prototype"),
                &classes.entries,
                &mut shape.metadata.prototype.helpers,
                0,
            )
        } else {
            shape.metadata.prototype = prototype(model, root, None);
            Some(true)
        };
        shape.metadata.old_class = Some(S4OldClass {
            classes: classes
                .entries
                .iter()
                .map(|(name, _)| name.clone())
                .collect(),
            index: ordinal,
            test: flag(argument(&args, "test"), false),
            s4_class: (ordinal == 0)
                .then(|| argument(&args, "S4Class"))
                .flatten()
                .map(|value| match string(Some(value)) {
                    ClassName::Known(name) => ParentRef::Formal(name),
                    _ => ParentRef::Unknown,
                }),
            parent_start: 0,
            factory: call_function_ref(model, root, call)?,
            helpers: shape.metadata.schema_helpers.clone(),
        });
        if let Some((ClassName::Known(parent), range)) = classes.entries.get(ordinal + 1) {
            shape.parents.push((parent.clone(), *range));
        }
        shape.metadata.parents_complete = classes
            .entries
            .iter()
            .all(|(name, _)| matches!(name, ClassName::Known(_)));
        if let Some(old) = &shape.metadata.old_class {
            if old.test != Some(false) {
                shape.metadata.parents_complete = false;
            }
            if let Some(ParentRef::Formal(_)) = &old.s4_class {
                shape.metadata.virtual_class = None;
            } else if old.s4_class.is_some() {
                shape.metadata.slots_complete = false;
            }
        }
        return Some(shape);
    }
    let mut virtual_marker = false;
    if let Some(value) = argument(&args, "contains") {
        if let Some(parents) = vector(model, root, &value) {
            shape.metadata.schema_helpers.extend(parents.helpers);
            for (name, range) in parents.entries {
                match name {
                    ClassName::Known(name) if name == "VIRTUAL" => virtual_marker = true,
                    ClassName::Known(name) => shape.parents.push((name, range)),
                    _ => shape.metadata.parents_complete = false,
                }
            }
        } else {
            shape.metadata.parents_complete = false;
        }
    }
    for group in ["representation", "slots"] {
        let Some(value) = argument(&args, group) else {
            continue;
        };
        let Some(mut entries) = entries(model, root, &value) else {
            shape.metadata.slots_complete = false;
            if group == "representation" {
                shape.metadata.parents_complete = false;
            }
            continue;
        };
        if entries.helpers.iter().any(|reference| {
            possible_functions(reference).iter().any(|identity| {
                !matches!(
                    (identity.package.as_str(), identity.name.as_str()),
                    ("base", "c" | "list" | "character") | ("methods", "representation")
                )
            })
        }) {
            shape.metadata.slots_complete = false;
            continue;
        }
        shape.metadata.slot_helpers.extend(entries.helpers.clone());
        shape.metadata.schema_helpers.extend(entries.helpers);
        let unnamed = entries.values.iter().all(|entry| entry.name.is_none());
        for mut entry in entries.values.drain(..) {
            if group == "representation" && entry.name.is_none() {
                match string(entry.value) {
                    ClassName::Known(name) if name == "VIRTUAL" => virtual_marker = true,
                    ClassName::Known(name) => shape.parents.push((name, entry.range)),
                    _ => shape.metadata.parents_complete = false,
                }
            } else {
                if group == "slots" && unnamed {
                    entry.name = match string(entry.value.clone()) {
                        ClassName::Known(name) => Some(name),
                        _ => None,
                    };
                    entry.untyped = true;
                }
                if entry.name.is_none() {
                    shape.metadata.slots_complete = false;
                }
                shape.slots.push(entry);
            }
        }
    }
    shape.metadata.virtual_class = if virtual_marker {
        Some(true)
    } else if !shape.metadata.parents_complete || !shape.metadata.slots_complete {
        None
    } else {
        Some(
            shape.parents.is_empty()
                && shape.slots.is_empty()
                && !non_null(argument(&args, "prototype")),
        )
    };
    Some(shape)
}

pub(in super::super) fn project(
    model: &SemanticModel,
    root: &SyntaxNode,
    owners: &HashMap<TextRange, ClassOwner>,
    classes: &[ClassDef],
) -> S4Facts {
    let mut result = S4Facts::default();
    for class in classes {
        let Some(s4) = &class.s4 else {
            continue;
        };
        for member in &s4.union_members {
            result.relations.push(S4Relation {
                owner: class.key.owner.clone(),
                subclass: member.clone(),
                superclass: class.name.clone(),
                kind: S4RelationKind::UnionMember,
                factory: class.factory.clone(),
                conditional: false,
                coerce: false,
                replace: false,
                by: Vec::new(),
                helpers: s4.schema_helpers.clone(),
                uncertainty: class
                    .uncertainty
                    .iter()
                    .filter_map(|issue| match issue {
                        ClassUncertainty::Conditional => Some(OopUncertainty::Conditional),
                        ClassUncertainty::CustomEnvironment => {
                            Some(OopUncertainty::CustomEnvironment)
                        }
                        ClassUncertainty::InvalidArguments => {
                            Some(OopUncertainty::InvalidArguments)
                        }
                        _ => None,
                    })
                    .chain((!class.key.owner.0.is_empty()).then_some(OopUncertainty::Conditional))
                    .collect(),
            });
        }
    }
    for (call, factory, name) in factories(model, root) {
        if !matches!(name.as_str(), "setIs" | "setAs") {
            continue;
        }
        let formals: &[&str] = if name == "setIs" {
            &[
                "class1",
                "class2",
                "test",
                "coerce",
                "replace",
                "by",
                "where",
                "classDef",
                "extensionObject",
                "doComplete",
            ]
        } else {
            &["from", "to", "def", "replace", "where"]
        };
        let args = matched_args(&call, formals);
        let mut uncertainty = BTreeSet::new();
        if args.is_none() {
            uncertainty.insert(OopUncertainty::InvalidArguments);
        }
        if !unconditional(call.syntax()) || !owner_at(call.syntax(), owners).0.is_empty() {
            uncertainty.insert(OopUncertainty::Conditional);
        }
        let args = args.unwrap_or_default();
        if args.contains_key("where")
            || args.contains_key("extensionObject")
            || args.contains_key("classDef")
        {
            uncertainty.insert(OopUncertainty::CustomEnvironment);
        }
        let from = string(argument(
            &args,
            if name == "setIs" { "class1" } else { "from" },
        ));
        let to = string(argument(
            &args,
            if name == "setIs" { "class2" } else { "to" },
        ));
        if !matches!(from, ClassName::Known(_)) || !matches!(to, ClassName::Known(_)) {
            uncertainty.insert(OopUncertainty::DynamicName);
        }
        let owner = owner_at(call.syntax(), owners);
        if name == "setIs" {
            let by_value = argument(&args, "by");
            let by_known = by_value.is_none()
                || by_value
                    .as_ref()
                    .is_some_and(|value| vector(model, root, value).is_some());
            if !by_known {
                uncertainty.insert(OopUncertainty::DynamicSignature);
            }
            let (by, helpers) = by_value
                .and_then(|value| vector(model, root, &value))
                .map(|vector| {
                    (
                        vector.entries.into_iter().map(|(name, _)| name).collect(),
                        vector.helpers,
                    )
                })
                .unwrap_or_else(|| {
                    if by_known {
                        (Vec::new(), Vec::new())
                    } else {
                        (vec![ClassName::Unknown], Vec::new())
                    }
                });
            result.relations.push(S4Relation {
                owner,
                subclass: from,
                superclass: to,
                kind: S4RelationKind::Is,
                factory,
                conditional: non_null(argument(&args, "test")),
                coerce: non_null(argument(&args, "coerce")),
                replace: non_null(argument(&args, "replace")),
                by,
                helpers,
                uncertainty,
            });
        } else {
            result.coercions.push(S4Coercion {
                owner,
                from,
                to,
                factory,
                coerce: non_null(argument(&args, "def")),
                replace: non_null(argument(&args, "replace")),
                uncertainty,
            });
        }
    }
    result
}
