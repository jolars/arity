//! Pinned metadata from methods, generated offline by dump_s4_builtins.R.

use super::*;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CallableKind {
    Primitive,
    Generic,
}

pub(super) struct Callable {
    pub kind: CallableKind,
    pub package: &'static str,
    pub name: &'static str,
}

pub(super) fn callable(package: &str, name: &str) -> Option<&'static Callable> {
    static CALLABLES: OnceLock<BTreeMap<&str, BTreeMap<&str, Callable>>> = OnceLock::new();
    CALLABLES
        .get_or_init(|| {
            let mut packages = BTreeMap::<_, BTreeMap<_, _>>::new();
            for line in include_str!("callables.tsv").lines().skip(1) {
                let [package, name, kind, owner, generic] = line
                    .split('\t')
                    .collect::<Vec<_>>()
                    .try_into()
                    .expect("callable metadata row");
                let kind = match kind {
                    "primitive" => CallableKind::Primitive,
                    "generic" => CallableKind::Generic,
                    _ => unreachable!("callable kind in pinned metadata"),
                };
                packages.entry(package).or_default().insert(
                    name,
                    Callable {
                        kind,
                        package: owner,
                        name: generic,
                    },
                );
            }
            packages
        })
        .get(package)?
        .get(name)
}

pub(super) fn generics() -> &'static [S4Generic] {
    static GENERICS: OnceLock<Vec<S4Generic>> = OnceLock::new();
    GENERICS.get_or_init(|| {
        let mut signatures = BTreeMap::<(&str, &str), CallableSignature>::new();
        for line in include_str!("formals.tsv").lines().skip(1) {
            let columns = line.split('\t').collect::<Vec<_>>();
            if let [name, package, parameter, default] = columns.as_slice() {
                signatures
                    .entry((name, package))
                    .or_insert_with(|| CallableSignature {
                        parameters: Vec::new(),
                    })
                    .parameters
                    .push(CallableParameter {
                        name: (*parameter).into(),
                        default: (*default != "-").then(|| (*default).into()),
                    });
            }
        }
        include_str!("generics.tsv")
            .lines()
            .skip(1)
            .filter_map(|line| {
                let [name, package, arguments, groups, group, simple_only] =
                    line.split('\t').collect::<Vec<_>>().try_into().ok()?;
                let names = |text: &str| {
                    text.split(',')
                        .filter(|name| *name != "-")
                        .map(|name| ClassName::Known(name.into()))
                        .collect::<Vec<_>>()
                };
                let dispatch_arguments = names(arguments);
                Some(S4Generic {
                    id: S4GenericId::Named {
                        package: package.into(),
                        name: name.into(),
                    },
                    declaration: None,
                    object: None,
                    signature: signatures.get(&(name, package)).cloned(),
                    skeleton: None,
                    dispatch_arguments,
                    groups: names(groups),
                    group: group == "TRUE",
                    simple_inheritance_only: Some(simple_only == "TRUE"),
                    default: S4DefaultMethod {
                        kind: S4DefaultKind::Unknown,
                        signature: None,
                        implementation: None,
                    },
                    uncertainty: BTreeSet::new(),
                })
            })
            .collect()
    })
}

pub(super) fn classes() -> &'static BTreeMap<String, Vec<String>> {
    static CLASSES: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();
    CLASSES.get_or_init(|| {
        include_str!("classes.tsv")
            .lines()
            .skip(1)
            .filter_map(|line| {
                let (name, parents) = line.split_once('\t')?;
                Some((
                    name.into(),
                    parents
                        .split(',')
                        .filter(|parent| *parent != "-")
                        .map(str::to_owned)
                        .collect(),
                ))
            })
            .collect()
    })
}
