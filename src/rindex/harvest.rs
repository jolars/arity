//! Harvest a single installed package into a [`PackageIndex`] by reading its
//! on-disk metadata — no R runtime.
//!
//! Cheap tier (this phase):
//! - `DESCRIPTION` → version (and the building R version, when present).
//! - `NAMESPACE` → exported names (explicit `export()` plus `exportPattern()`
//!   expanded against the lazy-load object names).
//! - `Meta/Rd.rds` → per-symbol help titles via the alias → title map.
//!
//! Formals and full help bodies are filled in by later phases.

use std::path::{Path, PathBuf};

use smol_str::SmolStr;

use crate::rindex::lazyload;
use crate::rindex::rds::{self, Rkind, Robj};
use crate::rindex::schema::{HelpDoc, PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};

#[derive(Debug)]
pub enum HarvestError {
    NotAPackage(PathBuf),
    Io(String),
    BadDescription(String),
}

impl std::fmt::Display for HarvestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarvestError::NotAPackage(p) => {
                write!(f, "{} is not an installed R package", p.display())
            }
            HarvestError::Io(s) => write!(f, "harvest I/O error: {s}"),
            HarvestError::BadDescription(s) => write!(f, "malformed DESCRIPTION: {s}"),
        }
    }
}

impl std::error::Error for HarvestError {}

type Result<T> = std::result::Result<T, HarvestError>;

#[derive(Debug, Clone, Copy)]
pub struct HarvestOptions {
    /// Harvest help (titles in this phase). When false, `help` is left `None`.
    pub help: bool,
}

impl Default for HarvestOptions {
    fn default() -> Self {
        HarvestOptions { help: true }
    }
}

/// Harvest the package installed at `pkg_dir` (the directory named after the
/// package inside a library). `harvested_at` is supplied by the caller so this
/// stays a pure function of its inputs (callers stamp the wall clock).
pub fn harvest_package(
    pkg_dir: &Path,
    opts: HarvestOptions,
    harvested_at: u64,
) -> Result<PackageIndex> {
    let desc_path = pkg_dir.join("DESCRIPTION");
    if !desc_path.is_file() {
        return Err(HarvestError::NotAPackage(pkg_dir.to_path_buf()));
    }
    let desc = read_dcf(&desc_path)?;
    let package = desc
        .field("Package")
        .ok_or_else(|| HarvestError::BadDescription("no Package field".into()))?
        .to_string();
    let version = desc
        .field("Version")
        .ok_or_else(|| HarvestError::BadDescription("no Version field".into()))?
        .to_string();
    let r_version = desc.field("Built").and_then(parse_built_r_version);

    let object_names = read_object_names(pkg_dir, &package);
    let namespace = std::fs::read_to_string(pkg_dir.join("NAMESPACE")).unwrap_or_default();
    let exports = resolve_exports(&namespace, &object_names);

    let titles = if opts.help {
        read_help_titles(pkg_dir)
    } else {
        AliasTitles::default()
    };

    let mut symbols: Vec<SymbolEntry> = exports
        .into_iter()
        .map(|name| {
            let help = titles.title_for(&name).map(|t| HelpDoc {
                title: Some(t.to_string()),
                ..Default::default()
            });
            SymbolEntry {
                name: SmolStr::new(&name),
                // Phase 1 cannot cheaply distinguish functions from data
                // without fetching objects; default to Function and refine in
                // a later phase.
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help,
            }
        })
        .collect();
    symbols.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(PackageIndex {
        schema_version: SCHEMA_VERSION,
        package: SmolStr::new(&package),
        version: SmolStr::new(&version),
        lib_path: pkg_dir
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        r_version,
        harvested_at,
        symbols,
    })
}

/// Read the lazy-load object names from `R/{pkg}.rdx`, if present.
fn read_object_names(pkg_dir: &Path, package: &str) -> Vec<String> {
    let rdx = pkg_dir.join("R").join(format!("{package}.rdx"));
    lazyload::read_index_names(&rdx).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// DESCRIPTION (DCF)
// ---------------------------------------------------------------------------

struct Dcf {
    fields: Vec<(String, String)>,
}

impl Dcf {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn read_dcf(path: &Path) -> Result<Dcf> {
    let text = std::fs::read_to_string(path).map_err(|e| HarvestError::Io(e.to_string()))?;
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            // Continuation of the previous field.
            if let Some(last) = fields.last_mut() {
                last.1.push('\n');
                last.1.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            fields.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok(Dcf { fields })
}

/// `Built:` looks like `R 4.5.3; ; 2025-...; unix` — pull the R version.
fn parse_built_r_version(built: &str) -> Option<SmolStr> {
    let first = built.split(';').next()?.trim();
    let ver = first.strip_prefix("R ").unwrap_or(first).trim();
    if ver.is_empty() {
        None
    } else {
        Some(SmolStr::new(ver))
    }
}

// ---------------------------------------------------------------------------
// NAMESPACE
// ---------------------------------------------------------------------------

/// Resolve the set of exported names from a NAMESPACE file, expanding
/// `exportPattern` directives against the package's object names.
pub fn resolve_exports(namespace: &str, object_names: &[String]) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut patterns: Vec<regex::Regex> = Vec::new();

    for directive in NamespaceDirectives::new(namespace) {
        match directive.name {
            "export" | "exportMethods" | "exportClasses" => {
                for arg in directive.args {
                    out.insert(arg);
                }
            }
            "exportPattern" | "exportClassPattern" => {
                for arg in directive.args {
                    if let Some(re) = compile_r_pattern(&arg) {
                        patterns.push(re);
                    }
                }
            }
            _ => {}
        }
    }

    if !patterns.is_empty() {
        for name in object_names {
            if patterns.iter().any(|re| re.is_match(name)) {
                out.insert(name.clone());
            }
        }
    }

    out.into_iter().collect()
}

/// Translate an R regular expression (as found in `exportPattern`) into a Rust
/// `regex`. R uses POSIX ERE here; for the patterns that appear in NAMESPACE
/// files these are directly compatible. Returns `None` (caller skips the
/// pattern) if it fails to compile.
fn compile_r_pattern(pattern: &str) -> Option<regex::Regex> {
    regex::Regex::new(pattern).ok()
}

struct NamespaceDirective {
    name: &'static str,
    args: Vec<String>,
}

/// Iterator over the recognized top-level NAMESPACE directives. Tolerant of
/// comments, conditionals, and whitespace; only the directives we care about
/// are surfaced.
struct NamespaceDirectives<'a> {
    rest: &'a str,
}

const RECOGNIZED: &[&str] = &[
    "exportPattern",
    "exportClassPattern",
    "exportClasses",
    "exportMethods",
    "export",
];

impl<'a> NamespaceDirectives<'a> {
    fn new(text: &'a str) -> Self {
        NamespaceDirectives { rest: text }
    }
}

impl Iterator for NamespaceDirectives<'_> {
    type Item = NamespaceDirective;

    fn next(&mut self) -> Option<Self::Item> {
        // Find the next recognized keyword followed by '('.
        let mut best: Option<(usize, &'static str)> = None;
        for &kw in RECOGNIZED {
            if let Some(idx) = find_call(self.rest, kw)
                && best.is_none_or(|(b, _)| idx < b)
            {
                best = Some((idx, kw));
            }
        }
        let (idx, kw) = best?;
        let after_kw = idx + kw.len();
        // Position of '(' (skip spaces).
        let paren_rel = self.rest[after_kw..].find('(')?;
        let open = after_kw + paren_rel;
        let close = matching_paren(self.rest, open)?;
        let inner = &self.rest[open + 1..close];
        self.rest = &self.rest[close + 1..];
        Some(NamespaceDirective {
            name: kw,
            args: parse_args(inner),
        })
    }
}

/// Find `keyword` occurring as a call head (followed, after optional spaces, by
/// `(`) and not as a substring of a longer identifier.
fn find_call(text: &str, keyword: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = text[from..].find(keyword) {
        let idx = from + rel;
        let before_ok = idx == 0
            || !text[..idx]
                .chars()
                .next_back()
                .map(is_ident_char)
                .unwrap_or(false);
        let after = &text[idx + keyword.len()..];
        let after_ok = after.trim_start().starts_with('(');
        // Reject if the char right after is an identifier char (e.g. matching
        // `export` inside `exportPattern`).
        let next_is_ident = after.chars().next().map(is_ident_char).unwrap_or(false);
        if before_ok && after_ok && !next_is_ident {
            return Some(idx);
        }
        from = idx + keyword.len();
    }
    None
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '.' || c == '_'
}

fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    let mut i = open;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                in_str = None;
            }
        } else {
            match c {
                b'"' | b'\'' | b'`' => in_str = Some(c),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Parse the comma-separated arguments of a directive, unquoting string
/// literals and interpreting R string escapes. Keyword arguments
/// (`name = value`) are reduced to their value.
fn parse_args(inner: &str) -> Vec<String> {
    let mut args = Vec::new();
    for raw in split_top_level_commas(inner) {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        // Drop a leading `name =` for things like `pattern = "..."`.
        let value = match raw.split_once('=') {
            Some((lhs, rhs)) if !lhs.trim_end().ends_with(['<', '>', '!']) => rhs.trim(),
            _ => raw,
        };
        if let Some(s) = unquote(value) {
            args.push(s);
        }
    }
    args
}

fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let bytes = inner.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                in_str = None;
            }
        } else {
            match c {
                b'"' | b'\'' | b'`' => in_str = Some(c),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(&inner[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    parts.push(&inner[start..]);
    parts
}

/// Unquote an R string literal, applying `\\` → `\` and `\"`/`\'` unescaping.
/// A bare (unquoted) identifier is returned as-is.
fn unquote(value: &str) -> Option<String> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'' || bytes[0] == b'`')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        Some(out)
    } else if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

// ---------------------------------------------------------------------------
// Help titles (Meta/Rd.rds)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct AliasTitles {
    /// alias → title.
    map: std::collections::HashMap<String, String>,
}

impl AliasTitles {
    fn title_for(&self, name: &str) -> Option<&str> {
        self.map.get(name).map(|s| s.as_str())
    }
}

fn read_help_titles(pkg_dir: &Path) -> AliasTitles {
    let path = pkg_dir.join("Meta").join("Rd.rds");
    let Ok(bytes) = std::fs::read(&path) else {
        return AliasTitles::default();
    };
    let Ok(rd) = rds::read_rds(&bytes) else {
        return AliasTitles::default();
    };
    parse_rd_titles(&rd).unwrap_or_default()
}

fn parse_rd_titles(rd: &Robj) -> Option<AliasTitles> {
    let names = rd.names()?;
    let cols = rd.as_list()?;
    let title_idx = names.iter().position(|c| *c == Some("Title"))?;
    let alias_idx = names.iter().position(|c| *c == Some("Aliases"))?;
    let titles = cols.get(title_idx)?.as_str_vec()?;
    let aliases = cols.get(alias_idx)?.as_list()?; // list column

    let mut map = std::collections::HashMap::new();
    for (i, alias_cell) in aliases.iter().enumerate() {
        let Some(title) = titles.get(i).and_then(|t| t.as_deref()) else {
            continue;
        };
        if let Rkind::Str(alias_vec) = &alias_cell.kind {
            for a in alias_vec.iter().flatten() {
                map.entry(a.clone()).or_insert_with(|| title.to_string());
            }
        }
    }
    Some(AliasTitles { map })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_explicit_exports() {
        let ns = r#"
            export(foo)
            export("bar")
            S3method(print, baz)
            exportMethods(show)
        "#;
        let exports = resolve_exports(ns, &[]);
        assert!(exports.contains(&"foo".to_string()));
        assert!(exports.contains(&"bar".to_string()));
        assert!(exports.contains(&"show".to_string()));
        // S3method is not a bare export.
        assert!(!exports.contains(&"print".to_string()));
        assert!(!exports.contains(&"baz".to_string()));
    }

    #[test]
    fn expands_export_pattern_excluding_dotted() {
        let ns = r#"exportPattern("^[^\\.]")"#;
        let objs = vec![
            "alpha".to_string(),
            "beta".to_string(),
            ".hidden".to_string(),
            ".__NAMESPACE__.".to_string(),
        ];
        let exports = resolve_exports(ns, &objs);
        assert_eq!(exports, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn unquotes_operator_exports() {
        let ns = r#"export("%>%")
                    export("n'est pas")"#;
        let exports = resolve_exports(ns, &[]);
        assert!(exports.contains(&"%>%".to_string()));
        assert!(exports.contains(&"n'est pas".to_string()));
    }

    #[test]
    fn ignores_export_inside_export_pattern_keyword() {
        // `find_call` must not treat the `export` in `exportPattern` as an
        // `export(...)` directive.
        let ns = r#"exportPattern("^x")"#;
        let exports = resolve_exports(ns, &["xa".to_string(), "yb".to_string()]);
        assert_eq!(exports, vec!["xa".to_string()]);
    }

    #[test]
    fn parses_built_r_version() {
        assert_eq!(
            parse_built_r_version("R 4.5.3; ; 2025-01-01 00:00:00 UTC; unix").as_deref(),
            Some("4.5.3")
        );
    }
}
