//! S7's source-level class, property, and dispatch model.

use super::*;
use std::path::Path;

mod bridges;
mod dispatch;
mod properties;

pub use dispatch::*;
pub use properties::{S7Api, S7Property, S7PropertyUse};

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S7ClassSpec {
    Builtin(FunctionRef),
    Generator(GeneratorRef),
    S3 {
        factory: FunctionRef,
        vector: Option<FunctionRef>,
        classes: Vec<String>,
        constructor: S7Function,
        default: Option<String>,
    },
    S4 {
        factory: FunctionRef,
        name: String,
        package: Option<String>,
    },
    External {
        factory: FunctionRef,
        package: String,
        name: String,
        version: Option<String>,
    },
    Union {
        factory: FunctionRef,
        classes: Vec<S7ClassSpec>,
    },
    Null,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S7Function {
    Absent,
    Known {
        key: Option<OopKey>,
        signature: CallableSignature,
    },
    Unknown,
}

impl S7Function {
    fn signature(&self) -> Option<&CallableSignature> {
        match self {
            Self::Known { signature, .. } => Some(signature),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S7Package {
    Enclosing,
    Named(String),
    None,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S7ClassDef {
    pub package: S7Package,
    pub parent: Option<S7ClassSpec>,
    pub abstract_class: Option<bool>,
    pub constructor: S7Function,
    pub validator: S7Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S7PropertyDef {
    pub class: Option<S7ClassSpec>,
    pub default: Option<String>,
    pub getter: S7Function,
    pub setter: S7Function,
    pub validator: S7Function,
    pub factory: Option<FunctionRef>,
    pub complete: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S7Facts {
    pub version: S7Version,
    pub classes: BTreeMap<ClassKey, S7ClassDef>,
    pub properties: BTreeMap<OopKey, S7PropertyDef>,
    pub helpers: BTreeSet<FunctionRef>,
    pub generics: BTreeMap<OopKey, S7GenericDef>,
    pub methods: BTreeMap<OopKey, S7MethodDef>,
    pub hooks: Vec<S7Hook>,
    pub s4_registrations: Vec<S7S4Bridge>,
    pub s4_contains: BTreeMap<ClassKey, S7S4Bridge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S7S4Bridge {
    pub factory: FunctionRef,
    pub class: S7ClassSpec,
    pub unconditional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S7GenericDef {
    Generated,
    Dispatch(FunctionRef),
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S7ExternalGeneric {
    pub factory: FunctionRef,
    pub package: String,
    pub name: String,
    pub version: Option<String>,
    pub dispatch_arguments: Vec<String>,
    pub dispatch_factory: Option<FunctionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S7MethodDef {
    pub list_signature: bool,
    pub removal: bool,
    pub external: Option<S7ExternalGeneric>,
    pub generic_origin: Option<FunctionRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum S7HookKind {
    Load,
    Unload,
    Build,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct S7Hook {
    pub kind: S7HookKind,
    pub factory: FunctionRef,
    pub unconditional: bool,
}

pub(super) fn is_builtin(name: &str) -> bool {
    name == "S7_object"
        || builtin_default(name).is_some()
        || matches!(
            name,
            "class_missing"
                | "class_data.frame"
                | "class_Date"
                | "class_factor"
                | "class_formula"
                | "class_POSIXct"
                | "class_POSIXlt"
                | "class_POSIXt"
        )
}

fn builtin_default(name: &str) -> Option<&'static str> {
    Some(match name {
        "class_any" => "NULL",
        "class_call" => "quote({ })",
        "class_character" => "character(0)",
        "class_complex" => "complex(0)",
        "class_double" => "numeric(0)",
        "class_environment" => "new.env(parent = emptyenv())",
        "class_expression" => "expression()",
        "class_function" => "function() NULL",
        "class_integer" | "class_numeric" => "integer(0)",
        "class_list" => "list()",
        "class_logical" | "class_atomic" | "class_vector" => "logical(0)",
        "class_raw" => "raw(0)",
        "class_language" | "class_name" => "quote(x)",
        _ => return None,
    })
}

// The release defaults inline the S3 constructors. Display equivalent calls
// through the exported classes to keep the signatures readable.
fn builtin_s3_property_default(name: &str, version: S7Version) -> Option<String> {
    let default = match name {
        "class_Date" => ".Date(numeric())",
        "class_POSIXct" => ".POSIXct(numeric(), tz = \"\")",
        "class_POSIXlt" => "as.POSIXlt(NULL, tz = \"\")",
        // Both editions generate a call to the stub for this abstract class.
        "class_POSIXt" => return Some("S7::class_POSIXt$constructor()".into()),
        "class_data.frame" => "data.frame()",
        "class_factor" => "factor()",
        "class_formula" => "stats::formula(NULL)",
        _ => return None,
    };
    Some(match version {
        S7Version::Release => format!("S7::{name}$constructor()"),
        S7Version::Development => default.into(),
    })
}

impl ClassIndex {
    pub(super) fn s7_constructible(&self, id: &ClassId) -> bool {
        self.s7_class(id)
            .is_some_and(|class| class.abstract_class == Some(false))
    }

    pub fn s7_class(&self, id: &ClassId) -> Option<&S7ClassDef> {
        self.declarations.get(id)?;
        self.file(&id.path)?.facts.oop.s7.classes.get(&id.key)
    }

    pub fn s7_version(&self, path: &Path) -> S7Version {
        self.file(path)
            .map_or(S7Version::Release, |file| file.facts.oop.s7.version)
    }

    /// The R class tag combines the declared S7 name and package. Neither the
    /// local constructor binding nor the exporting namespace supplies its name.
    pub fn s7_class_tag(&self, id: &ClassId) -> Option<String> {
        let class = self.s7_class(id)?;
        let declaration = self.declarations.get(id)?;
        let ClassName::Known(name) = &declaration.declaration.name else {
            return None;
        };
        if class.package == S7Package::Unknown
            || (class.package == S7Package::Enclosing
                && declaration.package_root.is_some()
                && self.s7_package(id).is_none())
        {
            return None;
        }
        let package = self.s7_package(id);
        Some(package.map_or_else(|| name.clone(), |package| format!("{package}::{name}")))
    }

    fn workspace_package_name(&self, root: &Path) -> Option<&str> {
        self.s3_origins.package_names.get(root).map(String::as_str)
    }

    fn s7_package(&self, id: &ClassId) -> Option<&str> {
        match &self.s7_class(id)?.package {
            S7Package::Enclosing => self.declarations[id]
                .package_root
                .as_ref()
                .and_then(|root| self.workspace_package_name(root)),
            S7Package::Named(package) => Some(package),
            _ => None,
        }
    }

    fn s7_external_version(&self, package: &str, floor: Option<&str>) -> bool {
        use crate::config::CompatVersion;
        let Some(floor) = floor else {
            return true;
        };
        self.s4_workspace_root(package)
            .and_then(|root| self.package_versions.get(root))
            .and_then(|version| CompatVersion::parse(version))
            .zip(CompatVersion::parse(floor))
            .is_some_and(|(version, floor)| version >= floor)
    }

    fn s7_union_origin(&self, path: &Path, factory: &FunctionRef) -> bool {
        self.s7_helper(path, factory, "S7").as_deref() == Some("new_union")
            || self.s7_helper(path, factory, "base").as_deref() == Some("|")
    }

    fn s7_helper(&self, path: &Path, reference: &FunctionRef, package: &str) -> Option<String> {
        let (origin, name) = match reference {
            FunctionRef::Qualified { package, name } => (package, name),
            _ => {
                let identity = self.callee_identity(path, reference)?;
                (&identity.package, &identity.name)
            }
        };
        (origin == package
            && (origin != "S7"
                || !S7Version::development_export(name)
                || self.s7_version(path) == S7Version::Development))
            .then(|| name.clone())
    }

    fn s7_spec_class(
        &self,
        path: &Path,
        owner: &ClassOwner,
        spec: &S7ClassSpec,
    ) -> Option<ClassId> {
        let (reference, package) = match spec {
            S7ClassSpec::Generator(reference) => (ParentRef::Generator(reference.clone()), None),
            S7ClassSpec::External {
                factory,
                package,
                name,
                version,
            } => {
                self.s7_helper(path, factory, "S7")?;
                if version.is_some() && !self.s7_external_version(package, version.as_deref()) {
                    return None;
                }
                (
                    ParentRef::Generator(GeneratorRef::Qualified {
                        package: package.clone(),
                        name: name.clone(),
                    }),
                    None,
                )
            }
            S7ClassSpec::S4 {
                factory,
                name,
                package,
            } => {
                self.s7_helper(path, factory, "methods")?;
                (ParentRef::Formal(name.clone()), package.as_deref())
            }
            _ => return None,
        };
        let candidates = self.resolve_reference(path, owner, &reference, false);
        let mut candidates = candidates.into_iter().filter(|id| {
            self.declarations.get(id).is_some_and(|declaration| {
                declaration.declaration.binding_known()
                    && package.is_none_or(|package| {
                        declaration
                            .package_root
                            .as_ref()
                            .and_then(|root| self.workspace_package_name(root))
                            == Some(package)
                    })
            })
        });
        let candidate = candidates.next()?;
        candidates.next().is_none().then_some(candidate)
    }
}
