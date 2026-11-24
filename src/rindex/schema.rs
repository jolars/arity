//! On-disk cache schema for the R-introspection index.
//!
//! Each package file stores independent JSON sections for export membership,
//! symbol offsets, package metadata, and symbol records. A single [`IndexMeta`]
//! (`meta.json`) records which version of each package is currently indexed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// Bump when the on-disk shape changes incompatibly or cached metadata needs
/// to be reharvested. Older versions are ignored and rebuilt.
pub const SCHEMA_VERSION: u32 = 8;

/// Everything harvested for a single installed package at a single version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageIndex {
    /// Must equal [`SCHEMA_VERSION`]; a mismatch means "treat as absent".
    pub schema_version: u32,
    pub package: SmolStr,
    /// Installed version this index was harvested from, e.g. `"2.0.4"`.
    pub version: SmolStr,
    /// Absolute path of the library directory the package was found in at
    /// harvest time (used for staleness checks).
    pub lib_path: String,
    /// The package's own one-line `Title`, from its `DESCRIPTION` — not a
    /// symbol's Rd help title (that is [`HelpDoc::title`]). Harvested so hover
    /// and `completionItem/resolve` can label a package without reading its
    /// installed `DESCRIPTION`. `None` for a `DESCRIPTION` with no `Title`
    /// field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<SmolStr>,
    /// R version that built the package (from `DESCRIPTION`'s `Built:` field),
    /// when available. Informational.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r_version: Option<SmolStr>,
    /// Unix epoch seconds when harvested. Informational + GC.
    pub harvested_at: u64,
    /// Packages this package attaches beyond itself when `library()`d — a
    /// meta-package's core set (e.g. tidyverse → dplyr, ggplot2, …), captured
    /// at harvest time. Empty for ordinary packages, and when capture found
    /// nothing (resolution then falls back to the static curated table).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attaches: Vec<SmolStr>,
    pub symbols: Vec<SymbolEntry>,
    #[serde(default, skip_serializing_if = "ObjectMetadata::is_empty")]
    pub objects: ObjectMetadata,
}

/// Positive evidence from installed records. Empty collections carry no
/// completeness guarantee, including for registrations performed at load time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub classes: Vec<InstalledClass>,
    pub generics: Vec<InstalledGeneric>,
    pub methods: Vec<InstalledMethod>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub help: BTreeMap<String, HelpDoc>,
}

impl ObjectMetadata {
    fn is_empty(&self) -> bool {
        self.classes.is_empty() && self.generics.is_empty() && self.methods.is_empty()
    }

    pub(crate) fn render_help(&self, association: &HelpAssociation) -> Option<&HelpDoc> {
        self.help.get(&association.cache_key())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectSystem {
    S3,
    S4,
    R6,
    Reference,
    S7,
}

/// A name observed in serialized metadata. Package is present only when the
/// object or registration identifies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectName {
    pub name: SmolStr,
    pub package: Option<SmolStr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledClass {
    pub system: ObjectSystem,
    pub identity: ObjectName,
    pub origin: ObjectOrigin,
    pub parents: Vec<ObjectName>,
    /// An R6 inheritance expression recoverable as a binding reference.
    pub parent_generator: Option<ObjectName>,
    /// A missing R6 parent may be either a known NULL or an undecodable expression.
    #[serde(default)]
    pub parent_known: bool,
    #[serde(default)]
    pub portable: Option<bool>,
    #[serde(default)]
    pub cloneable: Option<bool>,
    #[serde(default)]
    pub virtual_class: Option<bool>,
    #[serde(default)]
    pub exported_class: bool,
    /// Reference Class generator bindings associated with this representation.
    pub generators: Vec<ObjectOrigin>,
    pub constructor: Option<Vec<Formal>>,
    #[serde(default)]
    pub constructor_returns_instance: bool,
    pub members: Vec<InstalledMember>,
    pub help: Option<HelpAssociation>,
}

/// Binding or table entry within the package containing this index. The
/// referenced class or generic can be defined in a different package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectOrigin {
    pub binding: SmolStr,
    pub entry: Option<SmolStr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledMemberKind {
    Field,
    Slot,
    Property,
    Method,
    ActiveBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledMember {
    pub name: SmolStr,
    pub kind: InstalledMemberKind,
    pub private: bool,
    pub declared_type: Option<ObjectType>,
    pub formals: Option<Vec<Formal>>,
    pub help: Option<HelpAssociation>,
    /// The member is a stored value with no known read accessor.
    #[serde(default)]
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledGeneric {
    pub system: ObjectSystem,
    pub identity: ObjectName,
    pub origin: ObjectOrigin,
    pub dispatch_arguments: Option<Vec<SmolStr>>,
    pub formals: Option<Vec<Formal>>,
    pub help: Option<HelpAssociation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledMethod {
    pub system: ObjectSystem,
    pub generic: ObjectName,
    pub origin: ObjectOrigin,
    /// Unknown signature positions are retained so later positions stay aligned.
    pub dispatch: Option<Vec<DispatchParameter>>,
    /// The S4 table's target signature, when recorded separately from the
    /// definition's signature (for example, for an inherited method).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dispatch: Option<Vec<DispatchParameter>>,
    pub conditional: bool,
    pub formals: Option<Vec<Formal>>,
    pub help: Option<HelpAssociation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchParameter {
    pub argument: Option<SmolStr>,
    pub class: Option<ObjectType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectType {
    Named(ObjectName),
    S3Chain(Vec<SmolStr>),
    Union(Vec<ObjectType>),
    Any,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpAssociation {
    pub alias: SmolStr,
    pub topic: Option<SmolStr>,
}

impl HelpAssociation {
    pub(super) fn cache_key(&self) -> String {
        self.topic.as_ref().map_or_else(
            || format!("alias:{}", self.alias),
            |topic| format!("topic:{topic}"),
        )
    }
}

/// A single exported (or, under a future `--all`, internal) name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolEntry {
    pub name: SmolStr,
    pub kind: SymbolKind,
    /// True for namespace exports. A future `--all` build may add internals
    /// with `exported: false`; the thin provider view filters on this.
    pub exported: bool,
    /// Present only for functions whose formals we could read. `None` for
    /// data/other, primitives, or not-yet-harvested phases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formals: Option<Vec<Formal>>,
    /// Rendered help. Title-only in the cheap tier; full body later. `None`
    /// when no Rd documents this symbol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<HelpDoc>,
}

/// Coarse classification of an exported object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Data,
    /// S4 generics/classes, environments, and anything not a plain function or
    /// data set.
    Other,
}

/// One formal argument of a function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Formal {
    pub name: SmolStr,
    /// Default expression as source text (e.g. `"TRUE"`, `"c(1, 2)"`). `None`
    /// for a required argument; the `...` argument has name `"..."` and no
    /// default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Names and attachment information stored in the first section of a package file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageExports {
    /// Must equal [`SCHEMA_VERSION`]; a mismatch means "treat as absent".
    pub schema_version: u32,
    pub package: SmolStr,
    pub version: SmolStr,
    /// See [`PackageIndex::attaches`]. Resolution needs this even in the
    /// membership-only load, so the projection carries it too.
    #[serde(default)]
    pub attaches: Vec<SmolStr>,
    pub symbols: Vec<ExportEntry>,
}

/// The membership fields of a [`SymbolEntry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportEntry {
    pub name: SmolStr,
    pub exported: bool,
}

/// Rendered help for a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HelpDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The `\usage` block, rendered to text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    /// Per-argument documentation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<HelpArg>,
}

/// One `\item` from an Rd `\arguments` block. `name` may name several arguments
/// (e.g. `"x, y"`) when the Rd groups them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpArg {
    pub name: String,
    pub description: String,
}

/// Global index metadata (`meta.json`): which version of each package is
/// currently the authoritative cache entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IndexMeta {
    pub schema_version: u32,
    /// `package -> indexed version`. Lets us find the current entry for a
    /// package without statting every file, and detect a stale (old-version)
    /// entry cheaply.
    pub packages: BTreeMap<SmolStr, SmolStr>,
}

impl IndexMeta {
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            packages: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PackageIndex {
        PackageIndex {
            objects: Default::default(),
            schema_version: SCHEMA_VERSION,
            package: SmolStr::new("magrittr"),
            version: SmolStr::new("2.0.4"),
            lib_path: "/lib".to_string(),
            title: None,
            r_version: Some(SmolStr::new("4.5.3")),
            harvested_at: 0,
            attaches: Vec::new(),
            symbols: vec![
                SymbolEntry {
                    name: SmolStr::new("%>%"),
                    kind: SymbolKind::Function,
                    exported: true,
                    formals: Some(vec![
                        Formal {
                            name: SmolStr::new("lhs"),
                            default: None,
                        },
                        Formal {
                            name: SmolStr::new("rhs"),
                            default: None,
                        },
                    ]),
                    help: Some(HelpDoc {
                        title: Some("Pipe operator".to_string()),
                        ..Default::default()
                    }),
                },
                SymbolEntry {
                    name: SmolStr::new("debug_pipe"),
                    kind: SymbolKind::Function,
                    exported: true,
                    formals: None,
                    help: None,
                },
            ],
        }
    }

    #[test]
    fn round_trips_through_json() {
        let idx = sample();
        let json = serde_json::to_string(&idx).unwrap();
        let back: PackageIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(idx, back);
    }

    #[test]
    fn omits_empty_optional_fields() {
        let idx = sample();
        let json = serde_json::to_string(&idx).unwrap();
        // The second symbol has no formals/help; those keys must be absent.
        assert!(json.contains("debug_pipe"));
        assert!(!json.contains("\"arguments\""));
        // r_version present here, so it should appear.
        assert!(json.contains("r_version"));
    }

    #[test]
    fn attaches_round_trips_and_is_omitted_when_empty() {
        let mut idx = sample();
        // Empty attach set: the key must not be serialized at all.
        let json = serde_json::to_string(&idx).unwrap();
        assert!(!json.contains("\"attaches\""));

        idx.attaches = vec![SmolStr::new("dplyr"), SmolStr::new("ggplot2")];
        let json = serde_json::to_string(&idx).unwrap();
        let back: PackageIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(idx, back);
    }

    #[test]
    fn exports_view_carries_attaches() {
        let mut idx = sample();
        idx.attaches = vec![SmolStr::new("dplyr")];
        let json = serde_json::to_string(&idx).unwrap();
        let exports: PackageExports = serde_json::from_str(&json).unwrap();
        assert_eq!(exports.attaches, vec![SmolStr::new("dplyr")]);

        // A v1-era file without the key still deserializes (serde default).
        let idx = sample();
        let json = serde_json::to_string(&idx).unwrap();
        let exports: PackageExports = serde_json::from_str(&json).unwrap();
        assert!(exports.attaches.is_empty());
    }

    #[test]
    fn meta_round_trips() {
        let mut meta = IndexMeta::new();
        meta.packages
            .insert(SmolStr::new("magrittr"), SmolStr::new("2.0.4"));
        let json = serde_json::to_string(&meta).unwrap();
        let back: IndexMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, back);
    }
}
