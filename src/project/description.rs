//! Static discovery of roxygen2's package-wide options from `DESCRIPTION`.
//!
//! roxygen2 (7.3.3, `R/options.R`) resolves its options by evaluating the
//! `Roxygen` field of `DESCRIPTION` as an R expression (conventionally
//! `list(markdown = TRUE)`), overlaying the value of `man/roxygen/meta.R`
//! (sourced; its last expression's value) via `modifyList(desc, meta)`, and
//! falling back to defaults — `markdown = FALSE`.
//!
//! Arity's semantics stay **static** (no R evaluation), so this module
//! approximates that resolution by parsing the same texts with arity's own
//! parser and reading the `markdown` argument only when it is a literal
//! `TRUE`/`FALSE` in a plain `list(...)` call. Anything dynamic (a variable, a
//! computed list, an unparseable field) resolves to "unknown", which falls
//! through to the next layer exactly like an absent key would: an unknown
//! `meta.R` defers to the `DESCRIPTION` field, and an unknown field defers to
//! roxygen2's off default. The approximation therefore only misses packages
//! that compute their markdown flag at roxygenize time — it never *invents* a
//! markdown default.

use std::collections::BTreeSet;
use std::path::Path;

use crate::ast::{Arg, AstNode, CallExpr, HasArgList};
use crate::config::CompatVersion;
use crate::dcf::deps::dependency_entries;
use crate::parser::parse;
use crate::project::scope::package_root;
use crate::syntax::SyntaxKind;

/// Which field declared a dependency. The five carry different R semantics and
/// consumers must not conflate them — see [`DescriptionFacts::attached_packages`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyField {
    Depends,
    Imports,
    Suggests,
    LinkingTo,
    Enhances,
}

impl DependencyField {
    /// Every field, in R's canonical order.
    pub const ALL: [DependencyField; 5] = [
        DependencyField::Depends,
        DependencyField::Imports,
        DependencyField::Suggests,
        DependencyField::LinkingTo,
        DependencyField::Enhances,
    ];

    /// The DCF field name.
    pub fn name(self) -> &'static str {
        match self {
            DependencyField::Depends => "Depends",
            DependencyField::Imports => "Imports",
            DependencyField::Suggests => "Suggests",
            DependencyField::LinkingTo => "LinkingTo",
            DependencyField::Enhances => "Enhances",
        }
    }
}

/// One declared dependency: a package, and the version floor its entry states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub field: DependencyField,
    pub name: String,
    /// The lower bound (`>=`/`>`) the entry declares, verbatim. Other operators
    /// state no floor and are dropped, exactly as the R floor always was.
    pub version: Option<String>,
}

/// Everything a package's `DESCRIPTION` contributes to analysis, derived once.
///
/// **Range-free on purpose.** This rides in salsa, and the project layer's rule
/// is that a projection carrying spans cannot backdate — a `DESCRIPTION` save
/// would then cost a full project-graph rebuild. Spans live in the DCF CST,
/// where a consumer that needs them (a diagnostic, a hover) re-derives them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DescriptionFacts {
    /// The `Package` field, when non-empty.
    pub package: Option<String>,
    /// Every declared dependency, in field then source order. **`R` is not
    /// here**: it names the language, not a package, and its floor is
    /// [`compat`](Self::compat). Letting it reach an attach set would be
    /// catastrophic — no index can enumerate a package called `R`, so the
    /// conservative gate would silence every diagnostic in the package.
    pub dependencies: Vec<Dependency>,
    /// The tool-version floors the version-aware lint rules read.
    pub compat: DescriptionCompat,
    /// The `Roxygen` field's value, as R source.
    pub roxygen: Option<String>,
    /// Every `Collate*` field's entries.
    pub collate: BTreeSet<String>,
}

impl DescriptionFacts {
    /// Derive the facts from `DESCRIPTION` text. The parser's diagnostics are
    /// dropped here: this is the *analysis* reader, and reporting a malformed
    /// `DESCRIPTION` is a lint's job, not a side effect of asking for a floor.
    pub fn from_text(text: &str) -> Self {
        Self::from_document(&crate::dcf::parse(text).document())
    }

    /// [`from_text`](Self::from_text) over an already-parsed document.
    pub fn from_document(document: &crate::dcf::Document) -> Self {
        let get = |name: &str| document.field(name).map(|field| field.folded_value());

        let mut dependencies = Vec::new();
        let mut r_floor = None;
        let mut seen_r = false;
        for field in DependencyField::ALL {
            // First occurrence wins, matching `Document::field` and every
            // scalar reader here. R's `read.dcf` takes the last; closing that
            // divergence is its own deliberate change.
            let Some(node) = document.field(field.name()) else {
                continue;
            };
            for entry in dependency_entries(&node) {
                if entry.name == "R" {
                    // Only the first `R` entry of `Depends` states the floor —
                    // and an entry with no constraint states none, rather than
                    // deferring to a later one.
                    if field == DependencyField::Depends && !seen_r {
                        seen_r = true;
                        r_floor = entry
                            .lower_bound()
                            .and_then(|c| CompatVersion::parse(c.version.trim()));
                    }
                    continue;
                }
                dependencies.push(Dependency {
                    field,
                    name: entry.name.to_string(),
                    version: entry.lower_bound().map(|c| c.version.to_string()),
                });
            }
        }

        // `fields()` is record-blind on purpose: a stray blank line splits a
        // DESCRIPTION into two DCF records, and a `Collate` after that split
        // still names files R will load. R picks an OS-specific
        // `Collate@unix`/`Collate@windows` over plain `Collate`; we union every
        // `Collate*`, since over-including only tightens completeness.
        let mut collate = BTreeSet::new();
        for field in document.fields() {
            if !field.name().starts_with("Collate") {
                continue;
            }
            for entry in field.folded_value().split_whitespace() {
                let name = entry.trim_matches(['\'', '"']);
                if !name.is_empty() {
                    collate.insert(name.to_string());
                }
            }
        }

        DescriptionFacts {
            package: get("Package").filter(|name| !name.is_empty()),
            dependencies,
            compat: DescriptionCompat {
                r: r_floor,
                roxygen2: get("Config/roxygen2/version")
                    .or_else(|| get("RoxygenNote"))
                    .and_then(|v| CompatVersion::parse(v.trim())),
            },
            roxygen: get("Roxygen"),
            collate,
        }
    }

    /// The dependencies declared by one field.
    pub fn in_field(&self, field: DependencyField) -> impl Iterator<Item = &Dependency> {
        self.dependencies.iter().filter(move |d| d.field == field)
    }

    /// Packages **attached** to the search path when this package loads, so
    /// their exports resolve as bare names.
    ///
    /// `Depends` only. An `Imports` package is *not* attached — R reaches it
    /// solely through `pkg::` or a NAMESPACE `importFrom`/`import`, both of
    /// which the project layer already models. Adding `Imports` here would
    /// resolve names that fail under `R CMD check`.
    pub fn attached_packages(&self) -> BTreeSet<String> {
        self.in_field(DependencyField::Depends)
            .map(|d| d.name.clone())
            .collect()
    }

    /// Every declared package, whatever the field. *Referenced* — worth
    /// harvesting and fetching — but not attached.
    pub fn declared_packages(&self) -> BTreeSet<String> {
        self.dependencies.iter().map(|d| d.name.clone()).collect()
    }
}

/// The package-wide roxygen markdown default for the package at `root`
/// (a directory holding `DESCRIPTION`): `man/roxygen/meta.R` when statically
/// resolvable, else the `Roxygen` field of `DESCRIPTION`, else `false`
/// (roxygen2's default). Touches disk.
pub fn roxygen_markdown_default(root: &Path) -> bool {
    let desc = facts_at(root)
        .roxygen
        .as_deref()
        .and_then(markdown_from_r_text);
    let meta = std::fs::read_to_string(root.join("man/roxygen/meta.R"))
        .ok()
        .and_then(|text| markdown_from_r_text(&text));
    meta.or(desc).unwrap_or(false)
}

/// [`roxygen_markdown_default`] resolved for a single file: walk up to the
/// enclosing package root (`DESCRIPTION` + `R/`). A loose file outside any
/// package keeps roxygen2's off default. Touches disk.
pub fn roxygen_markdown_default_for_file(path: &Path) -> bool {
    package_root(path).is_some_and(|root| roxygen_markdown_default(&root))
}

/// [`roxygen_markdown_default_for_file`] anchored at a directory instead of a
/// file — for stdin input, where the only location is the working directory.
/// The walk starts at `dir` itself (a `package_root` walk starts at the
/// argument's parent). Touches disk.
pub fn roxygen_markdown_default_for_dir(dir: &Path) -> bool {
    roxygen_markdown_default_for_file(&dir.join("_stdin_.R"))
}

/// A per-directory-memoized [`roxygen_markdown_default_for_file`], for batch
/// walks (format/lint over a package) where every file in `R/` would otherwise
/// re-walk to the root and re-read `DESCRIPTION`. Two files sharing a parent
/// directory always share a package root, so the memo key is the parent.
#[derive(Debug, Default)]
pub struct MarkdownDefaultResolver {
    by_dir: std::collections::HashMap<std::path::PathBuf, bool>,
}

impl MarkdownDefaultResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// The markdown default for `file`, resolved once per parent directory.
    pub fn resolve(&mut self, file: &Path) -> bool {
        match file.parent() {
            Some(dir) => *self
                .by_dir
                .entry(dir.to_path_buf())
                .or_insert_with(|| roxygen_markdown_default_for_file(file)),
            None => false,
        }
    }
}

/// The name declared by the `Package` field of the DESCRIPTION at `root`
/// (a directory holding one). `None` when there is no readable DESCRIPTION or
/// it declares no `Package`. Touches disk.
pub fn package_name(root: &Path) -> Option<String> {
    facts_at(root).package
}

/// The [`DescriptionFacts`] of the package at `root` (a directory holding
/// `DESCRIPTION`). All-default when there is no readable one. Touches disk.
///
/// This is the single disk-reading entry point for DESCRIPTION facts. Callers
/// walking a whole package should go through [`DescriptionCache`] instead, so
/// the file is read and parsed once per root rather than once per question.
pub fn facts_at(root: &Path) -> DescriptionFacts {
    match std::fs::read_to_string(root.join("DESCRIPTION")) {
        Ok(text) => DescriptionFacts::from_text(&text),
        Err(_) => DescriptionFacts::default(),
    }
}

/// [`package_name`] resolved for a single file: walk up to the enclosing
/// package root (`DESCRIPTION` + `R/`) and read its `Package` field. `None` for
/// a loose file outside any package. Touches disk.
///
/// Note the walk anchors on [`package_root`], which requires an `R/` directory,
/// so a file under `tests/testthat/` of a package resolves to that package —
/// which is exactly the case that matters for `internal-function`.
pub fn package_name_for_file(path: &Path) -> Option<String> {
    package_root(path).and_then(|root| package_name(&root))
}

/// The tool-version facts a package's `DESCRIPTION` declares, for the
/// version-aware lint rules' compat floors (see `config::CompatConfig`, whose
/// explicit keys override these).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DescriptionCompat {
    /// The R floor from `Depends: R (>= x.y)` (a `>` constraint counts too —
    /// close enough for a floor).
    pub r: Option<CompatVersion>,
    /// The roxygen2 version the package documents with:
    /// `Config/roxygen2/version` (written by roxygen2 >= 8.0.0), then the
    /// legacy `RoxygenNote`.
    pub roxygen2: Option<CompatVersion>,
}

/// Read the [`DescriptionCompat`] facts of the package at `root` (a directory
/// holding `DESCRIPTION`). One disk read for both fields; all-`None` when
/// there is no readable `DESCRIPTION`. Touches disk.
pub fn description_compat(root: &Path) -> DescriptionCompat {
    facts_at(root).compat
}

/// [`description_compat`] resolved for a single file: walk up to the enclosing
/// package root (`DESCRIPTION` + `R/`). All-`None` for a loose file outside
/// any package — the version-aware rules then stay silent unless the user
/// configures a floor. Touches disk.
pub fn description_compat_for_file(path: &Path) -> DescriptionCompat {
    package_root(path)
        .map(|root| description_compat(&root))
        .unwrap_or_default()
}

/// A per-package-root memo over [`facts_at`], for the batch walks (format,
/// lint, `arity index`) that have no salsa database and would otherwise read
/// and parse one `DESCRIPTION` per question per file.
///
/// The salsa paths do not use this — they read the tracked `DESCRIPTION` input
/// instead, so an edit invalidates rather than going stale.
#[derive(Debug, Default)]
pub struct DescriptionCache {
    by_root: std::collections::HashMap<std::path::PathBuf, std::sync::Arc<DescriptionFacts>>,
}

impl DescriptionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The facts for the package at `root`, read once per root.
    pub fn at(&mut self, root: &Path) -> std::sync::Arc<DescriptionFacts> {
        self.by_root
            .entry(root.to_path_buf())
            .or_insert_with(|| std::sync::Arc::new(facts_at(root)))
            .clone()
    }

    /// The facts for the package enclosing `file`, or all-default when it is
    /// outside any package.
    pub fn for_file(&mut self, file: &Path) -> std::sync::Arc<DescriptionFacts> {
        match package_root(file) {
            Some(root) => self.at(&root),
            None => std::sync::Arc::new(DescriptionFacts::default()),
        }
    }
}

/// Statically resolve the `markdown` element of the R text's value: the text's
/// **last** top-level expression (both `eval(parse(text = field))` and
/// `source(meta.R)$value` yield the last expression's value) must be a plain
/// `list(...)` call carrying a literal `markdown = TRUE`/`FALSE`. `None` when
/// the value is absent or not statically resolvable.
fn markdown_from_r_text(text: &str) -> Option<bool> {
    let output = parse(text);
    if !output.diagnostics.is_empty() {
        return None;
    }
    // The last top-level expression must itself be the `list(...)` call. Walk
    // elements, not nodes: an atom statement (`x` after the list) is a bare
    // token in this CST and a node-level `last_child` would skip right past it.
    let last = output
        .cst
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE
                    | SyntaxKind::NEWLINE
                    | SyntaxKind::COMMENT
                    | SyntaxKind::SEMICOLON
            )
        })
        .last()?;
    let last = CallExpr::cast(last.into_node()?)?;
    if last.callee_name().as_deref() != Some("list") {
        return None;
    }
    last.args()
        .filter(|arg| arg.name().as_deref() == Some("markdown"))
        .last()
        .and_then(|arg| literal_logical(&arg))
}

/// The literal logical value of a named argument, when its value is exactly
/// the `TRUE` or `FALSE` constant (an identifier token — R's special constants
/// are bare tokens in the CST). `None` for anything else, including `T`/`F`
/// (reassignable in R) and computed values.
fn literal_logical(arg: &Arg) -> Option<bool> {
    let value = arg.value()?;
    let token = value.into_token()?;
    match token.text() {
        "TRUE" => Some(true),
        "FALSE" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(description: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("R")).expect("R/");
        std::fs::write(dir.path().join("R/a.R"), "NULL\n").expect("a.R");
        std::fs::write(dir.path().join("DESCRIPTION"), description).expect("DESCRIPTION");
        dir
    }

    fn write_meta(dir: &tempfile::TempDir, source: &str) {
        let meta_dir = dir.path().join("man/roxygen");
        std::fs::create_dir_all(&meta_dir).expect("man/roxygen/");
        std::fs::write(meta_dir.join("meta.R"), source).expect("meta.R");
    }

    #[test]
    fn description_compat_reads_r_floor_and_roxygen2_version() {
        let dir = package(
            "Package: mypkg\n\
             Depends: methods, R (>= 4.1.0), stats\n\
             RoxygenNote: 7.3.2\n",
        );
        let compat = description_compat(dir.path());
        assert_eq!(compat.r, CompatVersion::parse("4.1.0"));
        assert_eq!(compat.roxygen2, CompatVersion::parse("7.3.2"));
        // Per-file resolution walks to the package root.
        assert_eq!(
            description_compat_for_file(&dir.path().join("R/a.R")),
            compat
        );
        // A loose file outside any package resolves to no floors.
        let loose = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            description_compat_for_file(&loose.path().join("script.R")),
            DescriptionCompat::default()
        );
    }

    #[test]
    fn description_compat_prefers_the_modern_roxygen2_field() {
        // roxygen2 8.0.0 records its version in `Config/roxygen2/version`;
        // the legacy `RoxygenNote` remains as the fallback.
        let dir = package(
            "Package: mypkg\n\
             Config/roxygen2/version: 8.0.0\n\
             RoxygenNote: 7.3.2\n",
        );
        assert_eq!(
            description_compat(dir.path()).roxygen2,
            CompatVersion::parse("8.0.0")
        );
    }

    #[test]
    fn r_depends_floor_parses_constraint_shapes() {
        // Unchanged from when this was a bespoke string splitter: the floor is
        // now a projection of `dcf::deps`, and these cases are what prove the
        // generalization changed no behavior.
        let floor = |s: &str| {
            DescriptionFacts::from_text(&format!("Package: p\nDepends: {s}\n"))
                .compat
                .r
        };
        assert_eq!(floor("R (>= 4.1)"), CompatVersion::parse("4.1"));
        assert_eq!(floor("R (> 4.0.5)"), CompatVersion::parse("4.0.5"));
        assert_eq!(
            floor("stats, R(>=3.5.0), utils"),
            CompatVersion::parse("3.5.0")
        );
        // No constraint, a non-floor operator, or no R entry: no floor.
        assert_eq!(floor("R"), None);
        assert_eq!(floor("R (== 4.1)"), None);
        assert_eq!(floor("Rcpp (>= 1.0)"), None);
        assert_eq!(floor(""), None);
    }

    #[test]
    fn r_is_never_a_dependency() {
        // `R` names the language. If it reached an attach set, `package_indexed`
        // would fail for it and suppress every diagnostic in the package.
        let facts = DescriptionFacts::from_text("Package: p\nDepends: R (>= 4.1), stats\n");
        assert_eq!(facts.attached_packages(), ["stats".to_string()].into());
        assert_eq!(facts.declared_packages(), ["stats".to_string()].into());
        assert_eq!(facts.compat.r, CompatVersion::parse("4.1"));
    }

    #[test]
    fn every_dependency_field_is_parsed() {
        let facts = DescriptionFacts::from_text(
            "Package: p\n\
             Depends: R (>= 4.1), stats\n\
             Imports: dplyr (>= 1.0.0), rlang\n\
             Suggests: testthat\n\
             LinkingTo: cpp11\n\
             Enhances: data.table\n",
        );
        let named = |field: DependencyField| {
            facts
                .in_field(field)
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(named(DependencyField::Depends), ["stats"]);
        assert_eq!(named(DependencyField::Imports), ["dplyr", "rlang"]);
        assert_eq!(named(DependencyField::Suggests), ["testthat"]);
        assert_eq!(named(DependencyField::LinkingTo), ["cpp11"]);
        assert_eq!(named(DependencyField::Enhances), ["data.table"]);

        // Only `Depends` attaches; everything is declared.
        assert_eq!(facts.attached_packages(), ["stats".to_string()].into());
        assert_eq!(facts.declared_packages().len(), 6);

        let dplyr = facts
            .in_field(DependencyField::Imports)
            .next()
            .expect("dplyr");
        assert_eq!(dplyr.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn facts_collect_collate_and_roxygen() {
        let facts = DescriptionFacts::from_text(
            "Package: p\n\
             Roxygen: list(markdown = TRUE)\n\
             Collate: 'a.R' b.R\n\
             Collate@windows: c.R\n",
        );
        assert_eq!(facts.package.as_deref(), Some("p"));
        assert_eq!(facts.roxygen.as_deref(), Some("list(markdown = TRUE)"));
        assert_eq!(
            facts.collate,
            ["a.R".to_string(), "b.R".to_string(), "c.R".to_string()].into()
        );
    }

    #[test]
    fn facts_are_all_default_without_a_description() {
        let loose = tempfile::tempdir().expect("tempdir");
        assert_eq!(facts_at(loose.path()), DescriptionFacts::default());
    }

    #[test]
    fn description_cache_reads_once_per_root() {
        let dir = package("Package: mypkg\nImports: dplyr\n");
        let mut cache = DescriptionCache::new();
        let first = cache.for_file(&dir.path().join("R/a.R"));
        // Delete the file: a second lookup that re-read disk would come back
        // empty, so an unchanged answer is what proves the memo.
        std::fs::remove_file(dir.path().join("DESCRIPTION")).expect("remove");
        let second = cache.at(dir.path());
        assert_eq!(first.package.as_deref(), Some("mypkg"));
        assert_eq!(first, second);
    }

    #[test]
    fn package_name_from_description() {
        let dir = package("Package: mypkg\nVersion: 1.0\n");
        assert_eq!(package_name(dir.path()).as_deref(), Some("mypkg"));
        assert_eq!(
            package_name_for_file(&dir.path().join("R/a.R")).as_deref(),
            Some("mypkg")
        );
        assert_eq!(
            package_name_for_file(&dir.path().join("tests/testthat/test-a.R")).as_deref(),
            Some("mypkg")
        );
    }

    #[test]
    fn package_name_is_none_without_a_package() {
        // No DESCRIPTION at all.
        let loose = tempfile::tempdir().expect("tempdir");
        assert_eq!(package_name(loose.path()), None);
        assert_eq!(package_name_for_file(&loose.path().join("a.R")), None);
        // A DESCRIPTION that declares no `Package`.
        let dir = package("Version: 1.0\n");
        assert_eq!(package_name(dir.path()), None);
    }

    #[test]
    fn markdown_true_from_roxygen_field() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn markdown_defaults_off_without_field() {
        let dir = package("Package: p\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn markdown_false_is_explicit_off() {
        let dir = package("Package: p\nRoxygen: list(markdown = FALSE)\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn field_with_more_options_and_continuation() {
        let dir = package(
            "Package: p\nRoxygen: list(load = \"installed\",\n    markdown = TRUE)\nDepends: R\n",
        );
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn non_literal_markdown_value_is_unknown() {
        let dir = package("Package: p\nRoxygen: list(markdown = flag)\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn non_list_field_is_unknown() {
        let dir = package("Package: p\nRoxygen: make_options()\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_overrides_description_on() {
        let dir = package("Package: p\nRoxygen: list(markdown = FALSE)\n");
        write_meta(&dir, "list(markdown = TRUE)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_overrides_description_off() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        write_meta(&dir, "list(markdown = FALSE)\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_last_expression_wins_after_other_statements() {
        let dir = package("Package: p\n");
        write_meta(&dir, "x <- 1\nlist(markdown = TRUE)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_trailing_atom_is_not_the_list() {
        // `source()`'s value is `x`, not the list — statically unknown, so the
        // DESCRIPTION field wins.
        let dir = package("Package: p\nRoxygen: list(markdown = FALSE)\n");
        write_meta(&dir, "list(markdown = TRUE)\nx\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn unresolvable_meta_defers_to_description() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        write_meta(&dir, "build_meta()\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_list_without_markdown_defers_to_description() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        write_meta(&dir, "list(knitr_chunk_options = NULL)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn for_file_resolves_via_package_root() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        assert!(roxygen_markdown_default_for_file(&dir.path().join("R/a.R")));
    }

    #[test]
    fn for_file_outside_a_package_is_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loose = dir.path().join("loose.R");
        std::fs::write(&loose, "NULL\n").expect("loose.R");
        assert!(!roxygen_markdown_default_for_file(&loose));
    }
}
