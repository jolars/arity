//! Harvest a single installed package into a [`PackageIndex`] by reading its
//! on-disk metadata — no R runtime.
//!
//! What is read:
//! - `DESCRIPTION` → version (and the building R version, when present).
//! - `NAMESPACE` → exported names (explicit `export()` plus `exportPattern()`
//!   expanded against the lazy-load object names).
//! - `data/Rdata.rdx` → lazy-data (`LazyData`) object names — what R exposes
//!   via `.getNamespaceInfo(ns, "lazydata")`, classified `Data`. (`datasets`
//!   is entirely lazy-data.)
//! - `R/{pkg}.rdb` → function formals and symbol-kind refinement.
//! - `Meta/Rd.rds` → per-symbol help titles + the help-page key for each alias.
//! - `help/{pkg}.rdb` → full Rd bodies (description/usage/arguments), rendered
//!   to markdown via [`rd`](crate::rindex::rd).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smol_str::SmolStr;

use crate::rindex::deparse;
use crate::rindex::lazyload::{self, LazyLoadDb};
use crate::rindex::libpaths::LibrarySearch;
use crate::rindex::rd;
use crate::rindex::rds::{self, Rkind, Robj};
use crate::rindex::schema::{
    Formal, HelpDoc, PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind,
};

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
///
/// Attach-set capture is disabled here: it needs a [`LibrarySearch`] to
/// validate members as installed, so this wrapper always records an empty
/// `attaches`. Use [`harvest_package_in`] when a search path is available.
pub fn harvest_package(
    pkg_dir: &Path,
    opts: HarvestOptions,
    harvested_at: u64,
) -> Result<PackageIndex> {
    harvest_package_in(
        pkg_dir,
        opts,
        harvested_at,
        &LibrarySearch::from_dirs(Vec::new()),
    )
}

/// [`harvest_package`], plus attach-set capture against `search`: when the
/// package looks like a meta-package (it has an `.onAttach` hook and a
/// well-known attach-set variable such as tidyverse's `core`), the validated
/// member list is recorded in [`PackageIndex::attaches`].
pub fn harvest_package_in(
    pkg_dir: &Path,
    opts: HarvestOptions,
    harvested_at: u64,
    search: &LibrarySearch,
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
    let exports = resolve_package_exports(pkg_dir, &object_names);
    let lazydata = read_lazydata_names(pkg_dir);

    let help_index = if opts.help {
        read_help_index(pkg_dir)
    } else {
        AliasHelp::default()
    };

    // Open the lazy-load DB to fetch object values (formals + kind refinement).
    // `.ok()` is deliberate: a package may ship only a `.rdx` (no `.rdb`), in
    // which case we keep the cheap-tier defaults rather than failing the harvest.
    let db = LazyLoadDb::open(&pkg_dir.join("R").join(format!("{package}.rdx"))).ok();
    // The help DB (full Rd bodies) lives separately; many packages ship it, some
    // don't. `.ok()` keeps title-only behavior when it's absent.
    let help_db = if opts.help {
        LazyLoadDb::open(&pkg_dir.join("help").join(format!("{package}.rdx"))).ok()
    } else {
        None
    };

    let export_set: BTreeSet<&str> = exports.iter().map(String::as_str).collect();
    let mut symbols: Vec<SymbolEntry> = exports
        .iter()
        .map(|name| {
            let help = build_help(&help_index, help_db.as_ref(), name);
            let (kind, formals) = refine_symbol(db.as_ref(), name);
            SymbolEntry {
                name: SmolStr::new(name),
                kind,
                exported: true,
                formals,
                help,
            }
        })
        .collect();

    // Lazy-data objects are available as `pkg::name` (and after attach) even
    // though NAMESPACE exports nothing, so treat them as exported. They are
    // `Data` by definition — no need to decode the (potentially large)
    // `data/Rdata.rdb` to re-derive a kind we already know. A name that is both
    // a code export and a dataset keeps its richer export classification.
    for name in &lazydata {
        if export_set.contains(name.as_str()) {
            continue;
        }
        let help = build_help(&help_index, help_db.as_ref(), name);
        symbols.push(SymbolEntry {
            name: SmolStr::new(name),
            kind: SymbolKind::Data,
            exported: true,
            formals: None,
            help,
        });
    }
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
        attaches: detect_attaches(db.as_ref(), &package, search),
        symbols,
    })
}

/// Well-known namespace variables that hold a meta-package's attach set — the
/// convention tidyverse established (`core`) and tidymodels follows. Grown as
/// evidence for other conventions appears.
const ATTACH_SET_VARS: &[&str] = &["core"];

/// Capture what `package` attaches at `library()` time, without running R.
///
/// `.onAttach` bodies are byte-compiled, so the hook itself cannot be read;
/// instead the well-known attach-set variables ([`ATTACH_SET_VARS`]) are
/// fetched from the namespace lazy-load DB and validated. Empty means "found
/// nothing"; resolution then falls back to the static curated table.
fn detect_attaches(db: Option<&LazyLoadDb>, package: &str, search: &LibrarySearch) -> Vec<SmolStr> {
    let Some(db) = db else {
        return Vec::new();
    };
    // A package without an `.onAttach` hook cannot attach anything, whatever
    // its internal variables happen to be named.
    if !db.contains(".onAttach") {
        return Vec::new();
    }
    let installed = |member: &str| search.find_package(member).is_some();
    for var in ATTACH_SET_VARS {
        if let Ok(obj) = db.fetch(var)
            && let Some(members) = validate_attach_set(&obj, package, &installed)
        {
            return members;
        }
    }
    Vec::new()
}

/// Validate a candidate attach-set object, all-or-nothing: `Some` only when
/// `obj` is a plain character vector of syntactically valid, *installed*
/// package names (self and duplicates dropped). Any NA, malformed name, or
/// uninstalled member rejects the whole set — a partial set would override the
/// static fallback table while being wrong, and an unverifiable member would
/// permanently trip the conservative undefined-symbol gates.
fn validate_attach_set(
    obj: &Robj,
    package: &str,
    installed: &dyn Fn(&str) -> bool,
) -> Option<Vec<SmolStr>> {
    let strs = obj.as_str_vec()?;
    let mut members: Vec<SmolStr> = Vec::new();
    for s in strs {
        let name = s.as_deref()?;
        if name == package {
            continue;
        }
        if !is_valid_package_name(name) {
            return None;
        }
        if !members.iter().any(|m| m == name) {
            members.push(SmolStr::new(name));
        }
    }
    if members.is_empty() || !members.iter().all(|m| installed(m)) {
        return None;
    }
    Some(members)
}

/// R's package-name rules: at least two characters of ASCII letters, digits,
/// and dots, starting with a letter and not ending with a dot.
fn is_valid_package_name(name: &str) -> bool {
    name.len() >= 2
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
        && !name.ends_with('.')
}

/// The exported names of the package at `pkg_dir`. A package with a `NAMESPACE`
/// file exports exactly what it declares (explicit exports plus `exportPattern`
/// matches). A package *without* a `NAMESPACE` — most notably `base`, whose
/// namespace is the implicit base environment — exports its whole object set,
/// matching R's pre-namespace "export everything" rule. Over-exporting is
/// harmless: name resolution still gates which names reach a `base` lookup.
fn resolve_package_exports(pkg_dir: &Path, object_names: &[String]) -> Vec<String> {
    let ns_path = pkg_dir.join("NAMESPACE");
    if ns_path.is_file() {
        let namespace = std::fs::read_to_string(&ns_path).unwrap_or_default();
        resolve_exports(&namespace, object_names)
    } else {
        object_names.to_vec()
    }
}

/// Read the lazy-load object names from `R/{pkg}.rdx`, if present.
fn read_object_names(pkg_dir: &Path, package: &str) -> Vec<String> {
    let rdx = pkg_dir.join("R").join(format!("{package}.rdx"));
    lazyload::read_index_names(&rdx).unwrap_or_default()
}

/// Read lazy-data (`LazyData`) object names from `data/Rdata.rdx`, if present.
/// These are the names R exposes via `.getNamespaceInfo(ns, "lazydata")`;
/// `datasets` is entirely lazy-data. The `.rdx` index alone is read (cheap — no
/// `.rdb` decode). The file uses the fixed `Rdata` stem, not `{pkg}`.
fn read_lazydata_names(pkg_dir: &Path) -> Vec<String> {
    let rdx = pkg_dir.join("data").join("Rdata.rdx");
    lazyload::read_index_names(&rdx).unwrap_or_default()
}

/// Classify an exported object and, for closures, read its formals. Falls back
/// to the cheap-tier default (`Function`, no formals) when there is no DB to
/// fetch from, or the object isn't in it, or it decodes to a type we don't
/// model — a single object that won't decode never aborts the package.
fn refine_symbol(db: Option<&LazyLoadDb>, name: &str) -> (SymbolKind, Option<Vec<Formal>>) {
    let Some(db) = db else {
        return (SymbolKind::Function, None);
    };
    let Ok(obj) = db.fetch(name) else {
        return (SymbolKind::Function, None);
    };
    match &obj.kind {
        Rkind::Closure { formals, .. } => (SymbolKind::Function, Some(extract_formals(formals))),
        // A primitive aliased into the package (e.g. `add <- `+``): callable,
        // but no R-level formals.
        Rkind::Builtin => (SymbolKind::Function, None),
        Rkind::Logical(_) | Rkind::Int(_) | Rkind::Real(_) | Rkind::Str(_) | Rkind::List(_) => {
            (SymbolKind::Data, None)
        }
        // Environments, S4 objects, external pointers, symbols, …
        _ => (SymbolKind::Other, None),
    }
}

/// Map a closure's formals pairlist (tag = parameter name, value = default
/// expression or the empty-arg sentinel) to [`Formal`]s. A zero-parameter
/// function yields an empty list (distinct from `None` = "not read").
fn extract_formals(formals: &Robj) -> Vec<Formal> {
    match &formals.kind {
        Rkind::Pairlist(items) => items
            .iter()
            .map(|it| Formal {
                name: it.tag.clone().unwrap_or_default(),
                default: deparse::deparse(&it.value),
            })
            .collect(),
        _ => Vec::new(),
    }
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

/// Parse DCF (Debian Control File) text into its `(field, value)` pairs, in
/// file order. A continuation line (one starting with whitespace) appends to the
/// previous field's value, joined with `\n`. Shared by the DESCRIPTION readers
/// (`harvest` for package metadata, `project::graph` for `Collate:`); kept
/// IO-free so callers do their own `read_to_string`.
pub(crate) fn parse_dcf(text: &str) -> Vec<(String, String)> {
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
    fields
}

fn read_dcf(path: &Path) -> Result<Dcf> {
    let text = std::fs::read_to_string(path).map_err(|e| HarvestError::Io(e.to_string()))?;
    Ok(Dcf {
        fields: parse_dcf(&text),
    })
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

/// Everything a NAMESPACE file declares that name resolution cares about.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NamespaceInfo {
    /// Names made public via `export()` / `exportMethods()` / `exportClasses()`,
    /// plus `exportPattern()` matches against the package's object names.
    pub exports: BTreeSet<String>,
    /// Names imported individually via `importFrom(pkg, name, ...)`.
    pub imported_names: BTreeSet<String>,
    /// Packages imported wholesale via `import(pkg)`. Every export of such a
    /// package is in scope, so without that package's index any unresolved name
    /// could come from it.
    pub imported_packages: BTreeSet<String>,
}

/// Parse a NAMESPACE file into the exports and imports it declares, expanding
/// `exportPattern` directives against `object_names` (the package's top-level
/// object names).
pub fn parse_namespace(namespace: &str, object_names: &[String]) -> NamespaceInfo {
    let mut info = NamespaceInfo::default();
    let mut patterns: Vec<regex::Regex> = Vec::new();

    for directive in NamespaceDirectives::new(namespace) {
        match directive.name {
            "export" | "exportMethods" | "exportClasses" => {
                info.exports.extend(directive.args);
            }
            "S3method" => {
                // `S3method(generic, class)` registers the method `generic.class`;
                // the three-arg form `S3method(generic, class, method)` binds the
                // explicitly named `method` instead.
                match directive.args.as_slice() {
                    [_, _, method] => {
                        info.exports.insert(method.clone());
                    }
                    [generic, class] => {
                        info.exports.insert(format!("{generic}.{class}"));
                    }
                    _ => {}
                }
            }
            "exportPattern" | "exportClassPattern" => {
                for arg in directive.args {
                    if let Some(re) = compile_r_pattern(&arg) {
                        patterns.push(re);
                    }
                }
            }
            "importFrom" => {
                // `importFrom(pkg, a, b, ...)`: the first arg is the package, the
                // rest are the imported names.
                let mut args = directive.args.into_iter();
                if args.next().is_some() {
                    info.imported_names.extend(args);
                }
            }
            "import" => {
                info.imported_packages.extend(directive.args);
            }
            _ => {}
        }
    }

    if !patterns.is_empty() {
        for name in object_names {
            if patterns.iter().any(|re| re.is_match(name)) {
                info.exports.insert(name.clone());
            }
        }
    }

    info
}

/// Resolve the set of exported names from a NAMESPACE file, expanding
/// `exportPattern` directives against the package's object names.
pub fn resolve_exports(namespace: &str, object_names: &[String]) -> Vec<String> {
    parse_namespace(namespace, object_names)
        .exports
        .into_iter()
        .collect()
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
    "importFrom",
    "import",
    "S3method",
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
// Help index (Meta/Rd.rds) — alias → (title, help-page key)
// ---------------------------------------------------------------------------

/// What `Meta/Rd.rds` tells us about an alias: its page title and the help-DB
/// key (the `File` column minus `.Rd`) under which the full Rd body is stored.
#[derive(Clone, Default)]
struct AliasEntry {
    title: Option<String>,
    page: Option<String>,
}

#[derive(Default)]
struct AliasHelp {
    /// alias → entry.
    map: std::collections::HashMap<String, AliasEntry>,
}

impl AliasHelp {
    fn entry_for(&self, name: &str) -> Option<&AliasEntry> {
        self.map.get(name)
    }
}

fn read_help_index(pkg_dir: &Path) -> AliasHelp {
    let path = pkg_dir.join("Meta").join("Rd.rds");
    let Ok(bytes) = std::fs::read(&path) else {
        return AliasHelp::default();
    };
    let Ok(rd) = rds::read_rds(&bytes) else {
        return AliasHelp::default();
    };
    parse_rd_index(&rd).unwrap_or_default()
}

/// Parse the `Meta/Rd.rds` data frame into an alias → entry map. `Aliases` (the
/// keying column) is required; `Title` and `File` are best-effort. The help-DB
/// key is the `File` value with its `.Rd` suffix stripped.
fn parse_rd_index(rd: &Robj) -> Option<AliasHelp> {
    let names = rd.names()?;
    let cols = rd.as_list()?;
    let col = |label: &str| {
        names
            .iter()
            .position(|c| *c == Some(label))
            .and_then(|i| cols.get(i))
    };
    let alias_idx = names.iter().position(|c| *c == Some("Aliases"))?;
    let aliases = cols.get(alias_idx)?.as_list()?; // list column
    let titles = col("Title").and_then(|c| c.as_str_vec());
    let files = col("File").and_then(|c| c.as_str_vec());

    let mut map = std::collections::HashMap::new();
    for (i, alias_cell) in aliases.iter().enumerate() {
        let title = titles
            .and_then(|t| t.get(i))
            .and_then(|t| t.as_deref())
            .map(str::to_string);
        let page = files
            .and_then(|f| f.get(i))
            .and_then(|f| f.as_deref())
            .map(|f| f.strip_suffix(".Rd").unwrap_or(f).to_string());
        if let Rkind::Str(alias_vec) = &alias_cell.kind {
            for a in alias_vec.iter().flatten() {
                map.entry(a.clone()).or_insert_with(|| AliasEntry {
                    title: title.clone(),
                    page: page.clone(),
                });
            }
        }
    }
    Some(AliasHelp { map })
}

/// Assemble a symbol's [`HelpDoc`]: the title from `Meta/Rd.rds`, the body
/// (description/usage/arguments) from the help lazy-load DB page, if any. A page
/// that fails to decode degrades to title-only; a symbol no Rd documents yields
/// `None`.
fn build_help(index: &AliasHelp, db: Option<&LazyLoadDb>, name: &str) -> Option<HelpDoc> {
    let entry = index.entry_for(name)?;
    let sections = entry
        .page
        .as_deref()
        .zip(db)
        .and_then(|(page, db)| db.fetch(page).ok())
        .map(|page_obj| rd::render_page(&page_obj))
        .unwrap_or_default();
    let doc = rd::into_help_doc(entry.title.clone(), sections);
    (doc != HelpDoc::default()).then_some(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_vec(names: &[Option<&str>]) -> Robj {
        Robj {
            kind: Rkind::Str(names.iter().map(|n| n.map(String::from)).collect()),
            attr: Vec::new(),
        }
    }

    #[test]
    fn attach_set_accepts_installed_package_names() {
        let obj = str_vec(&[Some("dplyr"), Some("ggplot2")]);
        let members = validate_attach_set(&obj, "tidyverse", &|_| true).unwrap();
        assert_eq!(members, [SmolStr::new("dplyr"), SmolStr::new("ggplot2")]);
    }

    #[test]
    fn attach_set_rejects_non_character_objects() {
        let obj = Robj {
            kind: Rkind::Opaque,
            attr: Vec::new(),
        };
        assert!(validate_attach_set(&obj, "tidyverse", &|_| true).is_none());
    }

    #[test]
    fn attach_set_rejects_na_and_invalid_names() {
        // An NA element poisons the whole set.
        let obj = str_vec(&[Some("dplyr"), None]);
        assert!(validate_attach_set(&obj, "tidyverse", &|_| true).is_none());
        // Not a syntactically valid package name (a path, a sentence, …).
        for bad in ["with space", "path/pkg", "x", "1pkg", "pkg.", ""] {
            let obj = str_vec(&[Some(bad)]);
            assert!(
                validate_attach_set(&obj, "tidyverse", &|_| true).is_none(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn attach_set_rejects_any_uninstalled_member() {
        // All-or-nothing: one unverifiable member kills the whole set — a
        // partial set would beat the static fallback while being wrong.
        let obj = str_vec(&[Some("dplyr"), Some("notinstalled")]);
        let installed = |m: &str| m == "dplyr";
        assert!(validate_attach_set(&obj, "tidyverse", &installed).is_none());
    }

    #[test]
    fn attach_set_strips_self_and_duplicates() {
        let obj = str_vec(&[Some("tidyverse"), Some("dplyr"), Some("dplyr")]);
        let members = validate_attach_set(&obj, "tidyverse", &|_| true).unwrap();
        assert_eq!(members, [SmolStr::new("dplyr")]);
    }

    #[test]
    fn attach_set_rejects_empty_and_self_only_sets() {
        let obj = str_vec(&[]);
        assert!(validate_attach_set(&obj, "tidyverse", &|_| true).is_none());
        let obj = str_vec(&[Some("tidyverse")]);
        assert!(validate_attach_set(&obj, "tidyverse", &|_| true).is_none());
    }

    #[test]
    fn parse_dcf_folds_continuation_lines() {
        let text = "Package: testpkg\nCollate:\n    a.R\n    b.R\nVersion: 1.0\n";
        let fields = parse_dcf(text);
        assert_eq!(fields[0], ("Package".to_string(), "testpkg".to_string()));
        // The leading-whitespace lines append to `Collate`, joined with `\n`.
        assert_eq!(fields[1], ("Collate".to_string(), "\na.R\nb.R".to_string()));
        assert_eq!(fields[2], ("Version".to_string(), "1.0".to_string()));
    }

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
        // `S3method(generic, class)` registers the method `generic.class`, not
        // the generic or class names on their own.
        assert!(exports.contains(&"print.baz".to_string()));
        assert!(!exports.contains(&"print".to_string()));
        assert!(!exports.contains(&"baz".to_string()));
    }

    #[test]
    fn s3method_registers_dotted_method_name() {
        let ns = r#"
            S3method(coef, SLOPE)
            S3method(predict, "GaussianSLOPE")
        "#;
        let exports = resolve_exports(ns, &[]);
        assert!(exports.contains(&"coef.SLOPE".to_string()));
        assert!(exports.contains(&"predict.GaussianSLOPE".to_string()));
    }

    #[test]
    fn s3method_three_arg_form_uses_explicit_method() {
        // `S3method(generic, class, method)` binds `method`, not `generic.class`.
        let ns = "S3method(print, foo, print_foo_impl)\n";
        let exports = resolve_exports(ns, &[]);
        assert!(exports.contains(&"print_foo_impl".to_string()));
        assert!(!exports.contains(&"print.foo".to_string()));
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
    fn parses_import_directives() {
        let ns = "import(rlang)\nimportFrom(dplyr, filter, select)\nexport(foo)\n";
        let info = parse_namespace(ns, &[]);
        assert!(info.exports.contains("foo"));
        assert!(info.imported_names.contains("filter"));
        assert!(info.imported_names.contains("select"));
        // The package name is not itself an imported name.
        assert!(!info.imported_names.contains("dplyr"));
        assert!(info.imported_packages.contains("rlang"));
    }

    #[test]
    fn no_namespace_exports_every_object() {
        // `base` ships no NAMESPACE; its whole object set is exported.
        let tmp = tempfile::tempdir().unwrap();
        let objs = vec!["as.matrix".to_string(), "cbind".to_string()];
        let exports = resolve_package_exports(tmp.path(), &objs);
        assert_eq!(exports, objs);
    }

    #[test]
    fn namespace_present_restricts_to_declared_exports() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("NAMESPACE"), "export(foo)\n").unwrap();
        let exports = resolve_package_exports(tmp.path(), &["foo".to_string(), "bar".to_string()]);
        assert_eq!(exports, vec!["foo".to_string()]);
    }

    #[test]
    fn parses_built_r_version() {
        assert_eq!(
            parse_built_r_version("R 4.5.3; ; 2025-01-01 00:00:00 UTC; unix").as_deref(),
            Some("4.5.3")
        );
    }
}
