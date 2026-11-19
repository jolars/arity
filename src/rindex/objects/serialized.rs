//! Object and environment shapes as stored in a lazy-load database.

use std::collections::BTreeMap;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::rindex::harvest::extract_formals;
use crate::rindex::lazyload::LazyLoadDb;
use crate::rindex::rds::{Rkind, Robj};
use crate::rindex::schema::{
    DispatchParameter, Formal, InstalledGeneric, ObjectName, ObjectOrigin, ObjectSystem, ObjectType,
};
use crate::semantic::checked_match_args_to_formals;

pub(super) struct EnvironmentRecords<'a> {
    db: Option<&'a LazyLoadDb>,
    references: BTreeMap<SmolStr, Option<Arc<Robj>>>,
}

impl<'a> EnvironmentRecords<'a> {
    pub(super) fn new(db: Option<&'a LazyLoadDb>) -> Self {
        Self {
            db,
            references: BTreeMap::new(),
        }
    }

    pub(super) fn resolve(&mut self, value: &Robj) -> Option<Environment> {
        match &value.kind {
            Rkind::Environment(_) => Some(Environment::Inline(value.clone())),
            Rkind::Persistent(keys) => {
                let [Some(key)] = keys.as_slice() else {
                    return None;
                };
                if !key
                    .strip_prefix("env::")
                    .is_some_and(|id| id.parse::<u32>().is_ok())
                {
                    return None;
                }
                let db = self.db?;
                let record = self
                    .references
                    .entry(key.as_str().into())
                    .or_insert_with(|| {
                        db.fetch_reference(key)
                            .inspect_err(|error| log::debug!("lazy-load reference {key}: {error}"))
                            .ok()
                            .map(Arc::new)
                    })
                    .as_ref()?;
                list_field(record, "bindings")?.as_list()?;
                Some(Environment::Persisted(Arc::clone(record)))
            }
            _ => None,
        }
    }

    fn shadows_use_method(
        &mut self,
        mut value: Robj,
        package: &str,
        namespace_shadows_use_method: bool,
    ) -> bool {
        // Persisted environments can refer to one another by key.
        let mut visited = std::collections::BTreeSet::new();
        loop {
            match &value.kind {
                Rkind::Namespace(_) => {
                    return namespace_shadows_use_method && value.as_str() == Some(package);
                }
                Rkind::Persistent(keys) if !visited.insert(keys.clone()) => return false,
                _ => {}
            }
            let Some(environment) = self.resolve(&value) else {
                return false;
            };
            if environment.get("UseMethod").is_some() {
                return true;
            }
            let Some(parent) = environment.parent() else {
                return false;
            };
            value = parent.clone();
        }
    }
}

pub(super) enum Environment {
    Inline(Robj),
    Persisted(Arc<Robj>),
}

impl Environment {
    pub(super) fn bindings(&self) -> Vec<(&str, &Robj)> {
        match self {
            Self::Inline(value) => {
                let Rkind::Environment(environment) = &value.kind else {
                    return Vec::new();
                };
                pairlist(&environment.frame)
                    .chain(
                        environment
                            .hash_table
                            .as_list()
                            .into_iter()
                            .flatten()
                            .flat_map(pairlist),
                    )
                    .collect()
            }
            Self::Persisted(record) => list_field(record, "bindings")
                .into_iter()
                .flat_map(named_list)
                .collect(),
        }
    }

    pub(super) fn get(&self, name: &str) -> Option<&Robj> {
        self.bindings()
            .into_iter()
            .find_map(|(key, value)| (key == name).then_some(value))
    }

    fn parent(&self) -> Option<&Robj> {
        match self {
            Self::Inline(value) => match &value.kind {
                Rkind::Environment(environment) => Some(&environment.parent),
                _ => None,
            },
            Self::Persisted(record) => list_field(record, "enclos"),
        }
    }

    pub(super) fn attribute(&self, name: &str) -> Option<&Robj> {
        match self {
            Self::Inline(value) => value.attr(name),
            Self::Persisted(record) => list_field(list_field(record, "attributes")?, name),
        }
    }

    pub(super) fn is_class(&self, name: &str) -> bool {
        has_class(self.attribute("class"), name)
    }
}

fn pairlist(value: &Robj) -> impl Iterator<Item = (&str, &Robj)> {
    let items = match &value.kind {
        Rkind::Pairlist(items) => items.as_slice(),
        _ => &[],
    };
    items
        .iter()
        .filter_map(|item| Some((item.tag.as_deref()?, &item.value)))
}

pub(super) fn named_list(value: &Robj) -> impl Iterator<Item = (&str, &Robj)> {
    value
        .names()
        .unwrap_or_default()
        .into_iter()
        .zip(value.as_list().into_iter().flatten())
        .filter_map(|(name, value)| Some((name.filter(|name| !name.is_empty())?, value)))
}

pub(super) fn list_field<'a>(value: &'a Robj, field: &str) -> Option<&'a Robj> {
    named_list(value).find_map(|(name, value)| (name == field).then_some(value))
}

pub(super) fn is_class(value: &Robj, class: &str) -> bool {
    has_class(value.attr("class"), class)
}

fn has_class(classes: Option<&Robj>, class: &str) -> bool {
    classes
        .and_then(Robj::as_str_vec)
        .is_some_and(|classes| classes.iter().any(|name| name.as_deref() == Some(class)))
}

pub(super) fn scalar(value: &Robj) -> Option<&str> {
    let Rkind::Str(values) = &value.kind else {
        return None;
    };
    match values.as_slice() {
        [Some(text)] if !text.is_empty() => Some(text),
        _ => None,
    }
}

pub(super) fn object_name(value: &Robj) -> Option<ObjectName> {
    Some(ObjectName {
        name: scalar(value)?.into(),
        package: value.attr("package").and_then(scalar).map(Into::into),
    })
}

pub(super) fn s7_name(value: &Robj) -> Option<ObjectName> {
    if is_class(value, "S7_S3_class") {
        let classes = list_field(value, "class")?.as_str_vec()?;
        return Some(ObjectName {
            name: string_at(classes, 0)?,
            package: None,
        });
    }
    let (name, package) = if is_class(value, "S7_class") {
        (value.attr("name")?, value.attr("package").and_then(scalar))
    } else if is_class(value, "S7_base_class") {
        (list_field(value, "class")?, None)
    } else {
        return None;
    };
    Some(ObjectName {
        name: scalar(name)?.into(),
        package: package.map(Into::into),
    })
}

pub(super) fn s7_type(value: &Robj) -> Option<ObjectType> {
    s7_type_at(value, 0)
}

fn s7_type_at(value: &Robj, depth: usize) -> Option<ObjectType> {
    if depth >= 32 {
        return None;
    }
    if is_class(value, "S7_any") {
        Some(ObjectType::Any)
    } else if is_class(value, "S7_missing") {
        Some(ObjectType::Missing)
    } else if is_class(value, "S7_S3_class") {
        Some(ObjectType::S3Chain(strings(list_field(value, "class")?)?))
    } else if is_class(value, "S7_union") {
        let classes = list_field(value, "classes")?.as_list()?;
        Some(ObjectType::Union(
            classes
                .iter()
                .map(|item| s7_type_at(item, depth + 1))
                .collect::<Option<_>>()?,
        ))
    } else if matches!(value.kind, Rkind::S4) && is_class(value, "classRepresentation") {
        let mut identity = object_name(value.attr("className")?)?;
        if let Some(package) = value.attr("package").and_then(scalar) {
            identity.package = Some(package.into());
        }
        Some(ObjectType::Named(identity))
    } else {
        s7_name(value).map(ObjectType::Named)
    }
}

pub(super) fn s7_dispatch(
    signature: Option<&Robj>,
    args: Option<&Robj>,
) -> Option<Vec<DispatchParameter>> {
    let args = args.and_then(Robj::as_str_vec).unwrap_or_default();
    Some(
        signature?
            .as_list()?
            .iter()
            .enumerate()
            .map(|(index, class)| DispatchParameter {
                argument: string_at(args, index),
                class: s7_type(class),
            })
            .collect(),
    )
}

pub(super) fn binding_reference(value: &Robj) -> Option<ObjectName> {
    let symbol = |value: &Robj| match &value.kind {
        Rkind::Symbol(name) if !name.is_empty() => Some(name.clone()),
        _ => None,
    };
    if let Some(name) = symbol(value) {
        return Some(ObjectName {
            name,
            package: None,
        });
    }
    let Rkind::Pairlist(items) = &value.kind else {
        return None;
    };
    let [operator, package, name] = items.as_slice() else {
        return None;
    };
    if !symbol(&operator.value).is_some_and(|op| op == "::" || op == ":::") {
        return None;
    }
    Some(ObjectName {
        name: symbol(&name.value)?,
        package: Some(symbol(&package.value)?),
    })
}

pub(super) fn s4_dispatch(signature: &Robj) -> Option<Vec<DispatchParameter>> {
    let classes = signature.as_str_vec()?;
    let args = signature
        .attr("names")
        .and_then(Robj::as_str_vec)
        .unwrap_or_default();
    let packages = signature
        .attr("package")
        .and_then(Robj::as_str_vec)
        .unwrap_or_default();
    Some(
        classes
            .iter()
            .enumerate()
            .map(|(index, _)| DispatchParameter {
                argument: string_at(args, index),
                class: string_at(classes, index).map(|name| match name.as_str() {
                    "ANY" => ObjectType::Any,
                    "missing" => ObjectType::Missing,
                    _ => ObjectType::Named(ObjectName {
                        name,
                        package: string_at(packages, index),
                    }),
                }),
            })
            .collect(),
    )
}

fn string_at(values: &[Option<String>], index: usize) -> Option<SmolStr> {
    values
        .get(index)?
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(Into::into)
}

pub(super) fn strings(value: &Robj) -> Option<Vec<SmolStr>> {
    value
        .as_str_vec()?
        .iter()
        .map(|value| value.as_deref().map(Into::into))
        .collect()
}

pub(super) fn origin(binding: &str, entry: Option<&str>) -> ObjectOrigin {
    ObjectOrigin {
        binding: binding.into(),
        entry: entry.map(Into::into),
    }
}

pub(super) fn formals(value: &Robj) -> Option<Vec<Formal>> {
    let Rkind::Closure { formals, .. } = &value.kind else {
        return None;
    };
    match &formals.kind {
        Rkind::Null => Some(Vec::new()),
        Rkind::Pairlist(items) if items.iter().all(|item| item.tag.is_some()) => {
            Some(extract_formals(formals))
        }
        _ => None,
    }
}

pub(super) fn closure_namespace(value: &Robj) -> Option<SmolStr> {
    match &value.kind {
        Rkind::Closure {
            environment: Some(environment),
            ..
        } if matches!(environment.kind, Rkind::Namespace(_)) => {
            environment.as_str().map(Into::into)
        }
        _ => None,
    }
}

/// A direct UseMethod call is static evidence of a generic surface. Other
/// bodies, including computed names and indirect dispatch, remain unknown.
pub(super) fn s3_generic(
    binding: &str,
    value: &Robj,
    environments: &mut EnvironmentRecords<'_>,
    package: &str,
    namespace_shadows_use_method: bool,
) -> Option<InstalledGeneric> {
    let Rkind::Closure {
        body,
        formals: parameters,
        environment,
    } = &value.kind
    else {
        return None;
    };
    let body = match &body.kind {
        Rkind::Bytecode(source) => source,
        _ => body,
    };
    let Rkind::Pairlist(items) = &body.kind else {
        return None;
    };
    let [callee, arguments @ ..] = items.as_slice() else {
        return None;
    };
    let direct = matches!(&callee.value.kind, Rkind::Symbol(name) if name == "UseMethod");
    let in_base = binding_reference(&callee.value)
        .is_some_and(|name| name.name == "UseMethod" && name.package.as_deref() == Some("base"));
    let unshadowed = direct
        && !pairlist(parameters).any(|(name, _)| name == "UseMethod")
        && !environment.as_deref().is_some_and(|environment| {
            environments.shadows_use_method(
                environment.clone(),
                package,
                namespace_shadows_use_method,
            )
        });
    if !(unshadowed || in_base) {
        return None;
    }
    let tags: Vec<_> = arguments
        .iter()
        .map(|argument| argument.tag.clone())
        .collect();
    let matched = checked_match_args_to_formals(&tags, &["generic", "object"]).ok()?;
    let mut name = None;
    let mut dispatch_arguments = None;
    for (argument, parameter) in arguments.iter().zip(matched) {
        match (parameter, &argument.value.kind) {
            (Some("generic"), _) => name = object_name(&argument.value),
            (Some("object"), Rkind::Symbol(name)) => {
                dispatch_arguments = Some(vec![name.clone()]);
            }
            _ => {}
        }
    }
    let mut name = name?;
    name.package = closure_namespace(value);
    Some(InstalledGeneric {
        system: ObjectSystem::S3,
        identity: name,
        origin: origin(binding, None),
        dispatch_arguments,
        formals: formals(value),
        help: None,
    })
}
