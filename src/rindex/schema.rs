//! On-disk cache schema for the R-introspection index.
//!
//! One [`PackageIndex`] is serialized to `{package}@{version}.json` under the
//! cache directory; a single [`IndexMeta`] (`meta.json`) records which version
//! of each package is currently indexed. The schema is deliberately rich
//! (formals + help) even though early phases only populate names + help titles
//! — later phases fill the remaining fields without a schema change.
//!
//! The thin name-list view used by the [`SymbolProvider`](crate::semantic::symbols::SymbolProvider)
//! is derived in memory from [`PackageIndex::symbols`] at load time; there is
//! no separate names file.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// Bump when the on-disk shape changes incompatibly. Files (and the enclosing
/// `v{N}/` directory) carrying a different version are ignored and rebuilt.
pub const SCHEMA_VERSION: u32 = 1;

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
    /// R version that built the package (from `DESCRIPTION`'s `Built:` field),
    /// when available. Informational.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r_version: Option<SmolStr>,
    /// Unix epoch seconds when harvested. Informational + GC.
    pub harvested_at: u64,
    pub symbols: Vec<SymbolEntry>,
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
            schema_version: SCHEMA_VERSION,
            package: SmolStr::new("magrittr"),
            version: SmolStr::new("2.0.4"),
            lib_path: "/lib".to_string(),
            r_version: Some(SmolStr::new("4.5.3")),
            harvested_at: 0,
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
    fn meta_round_trips() {
        let mut meta = IndexMeta::new();
        meta.packages
            .insert(SmolStr::new("magrittr"), SmolStr::new("2.0.4"));
        let json = serde_json::to_string(&meta).unwrap();
        let back: IndexMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, back);
    }
}
