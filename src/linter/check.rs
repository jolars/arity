//! `arity lint` driver: walks input paths, parses, builds a semantic model,
//! runs the configured rules, filters suppressed findings, and reports.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use rowan::{TextRange, TextSize};

use crate::config::LintConfig;
use crate::file_discovery::{ExcludeFilter, FileDiscoveryError, collect_r_files};
use crate::incremental::{
    Analysis, IncrementalDatabase, IncrementalDb, ParseDiagnosticData, SourceFile, file_exports,
    file_free_reads, file_qualified_reads, parsed_tree_root, semantic_model, source_edges,
    top_level_events,
};
use crate::project::{
    ExternalResolution, FileScope, PackageCollation, Project, ProjectMember, expected_r_sources,
    external_resolution, package_root, visible_symbols, workspace_project,
};
use crate::rindex::provider::IndexedProvider;
use crate::semantic::SymbolProvider;

use super::diagnostic::{Diagnostic, Severity, ViolationData};
use super::rules::{ResolvedRules, default_symbol_provider, run_rules};
use super::suppression::SuppressionMap;

/// Synthetic rule id carried by findings that originate from the parser's error
/// side channel rather than a lint rule. Shown as the `[code]` in CLI output and
/// as the LSP diagnostic code.
pub const SYNTAX_ERROR_RULE: &str = "syntax-error";

/// Map the parser's side-channel diagnostics into lint [`Diagnostic`]s so they
/// surface through the normal finding pipeline (CLI render + LSP publish).
///
/// Parse errors *block* the lint rules for a file (Tenet 3: parsing is the
/// parser's job, and rules need a clean tree) — but blocking the rules must not
/// swallow the error. Each parser diagnostic becomes a [`Severity::Error`]
/// finding under the [`SYNTAX_ERROR_RULE`] id, preserving the message and span.
pub fn syntax_error_diagnostics(diags: &[ParseDiagnosticData], path: &Path) -> Vec<Diagnostic> {
    diags
        .iter()
        .map(|d| Diagnostic {
            rule: SYNTAX_ERROR_RULE,
            severity: Severity::Error,
            path: path.to_path_buf(),
            range: TextRange::new(TextSize::new(d.start as u32), TextSize::new(d.end as u32)),
            message: ViolationData::new(SYNTAX_ERROR_RULE, d.message.clone()),
            fix: None,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintStatus {
    Clean,
    Findings { count: usize },
    ParseDiagnostics { count: usize },
}

#[derive(Debug, Clone)]
pub struct LintFileReport {
    pub path: PathBuf,
    pub status: LintStatus,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct LintResult {
    pub checked_files: usize,
    pub total_findings: usize,
    pub reports: Vec<LintFileReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintError {
    MissingPaths,
    NoRFiles,
    NonRFilePath { path: PathBuf },
    WalkError { path: PathBuf, message: String },
    ReadError { path: PathBuf, source: String },
    UnknownRule { rule: String },
}

impl fmt::Display for LintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPaths => {
                write!(
                    f,
                    "lint requires at least one input path (file or directory)"
                )
            }
            Self::NoRFiles => write!(f, "no .R files found under the provided input paths"),
            Self::NonRFilePath { path } => write!(
                f,
                "input file {} is not an .R file; lint only supports .R files",
                path.display()
            ),
            Self::WalkError { path, message } => {
                write!(f, "failed while scanning {}: {message}", path.display())
            }
            Self::ReadError { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::UnknownRule { rule } => write!(f, "unknown lint rule: `{rule}`"),
        }
    }
}

impl std::error::Error for LintError {}

impl From<FileDiscoveryError> for LintError {
    fn from(value: FileDiscoveryError) -> Self {
        match value {
            FileDiscoveryError::NonRFilePath { path } => Self::NonRFilePath { path },
            FileDiscoveryError::WalkError { path, message } => Self::WalkError { path, message },
        }
    }
}

pub fn check_paths(paths: &[PathBuf]) -> Result<LintResult, LintError> {
    check_paths_with_config(paths, &LintConfig::default())
}

pub fn check_paths_with_config(
    paths: &[PathBuf],
    config: &LintConfig,
) -> Result<LintResult, LintError> {
    check_paths_with_index(
        paths,
        config,
        &ExcludeFilter::none(),
        IndexedProvider::empty(),
    )
}

/// Like [`check_paths_with_config`] but with a caller-supplied harvested package
/// index, installed into salsa as the HIGH-durability [`LibraryIndex`] and used
/// by the [`external_resolution`] query. R's default packages and the bundled
/// CRAN lists are static and need not be supplied.
pub fn check_paths_with_index(
    paths: &[PathBuf],
    config: &LintConfig,
    exclude: &ExcludeFilter,
    indexed: IndexedProvider,
) -> Result<LintResult, LintError> {
    if paths.is_empty() {
        return Err(LintError::MissingPaths);
    }

    let (rules, unknown) = ResolvedRules::resolve(config.select.as_deref(), &config.ignore);
    if let Some(rule) = unknown.into_iter().next() {
        return Err(LintError::UnknownRule { rule });
    }

    let files = collect_r_files(paths, exclude).map_err(LintError::from)?;
    if files.is_empty() {
        return Err(LintError::NoRFiles);
    }

    let mut db = IncrementalDatabase::default();
    let mut tracked: HashMap<PathBuf, SourceFile> = HashMap::new();

    // Pass 1: track every file. Parsing is deferred to the parallel warm-up
    // below, so this loop is disk reads and salsa input writes only. Membership
    // is derived from the workspace file-set below; files with parse diagnostics
    // are tracked but `workspace_project` drops them from the scope.
    for path in &files {
        let content = fs::read_to_string(path).map_err(|err| LintError::ReadError {
            path: path.clone(),
            source: err.to_string(),
        })?;
        let file = db.upsert_file(path, content);
        tracked.insert(path.clone(), file);
    }

    // Scope-only members: a package's generated R sources (`cpp11.R`,
    // `RcppExports.R`, `extendr-wrappers.R`, `import-standalone-*.R`) are in the
    // default exclude set, so they are never linted — but they *define* the
    // wrappers that hand-written siblings call. Track them so their top-level
    // bindings enter the package namespace; without this every caller is a false
    // `undefined-symbol`. They join the workspace below but are absent from
    // `files`, so pass 2 never lints them. Mirrors the LSP's exclude-free sibling
    // discovery in `seed_workspace_for`.
    let mut scope_only: Vec<SourceFile> = Vec::new();
    for path in excluded_package_sources(&files) {
        if let Ok(content) = fs::read_to_string(&path) {
            scope_only.push(db.upsert_file(&path, content));
        }
    }

    // Install the harvested index as the HIGH-durability library singleton;
    // `external_resolution` reads it in pass 2. This write must precede the
    // parallel warm-up: a HIGH write bumps every durability's revision, so
    // doing it after would strip the shallow-verify fast path from all the
    // freshly warmed memos.
    let manifest = db.set_library_index(indexed);

    // Seed the explicit workspace file-set and derive the interned project from
    // it. `workspace_project` filters to cleanly-parsing members, reads each
    // package's NAMESPACE, and interns — the same membership the inline build
    // produced, now keyed off the salsa `Workspace` input.
    let members: Vec<SourceFile> = tracked
        .values()
        .copied()
        .chain(scope_only.iter().copied())
        .collect();
    db.set_workspace_members(members, files.clone());

    // All salsa writes are done; everything below is reads. Warm every member's
    // per-file memos in parallel: the parse, and — for cleanly parsing files —
    // the firewall projections `project_graph` folds over (each of which forces
    // `semantic_model`). Without this the first worker to touch the project
    // graph would compute every member's model *sequentially inside one query*
    // while the rest block on it. Salsa db handles are `Send` but not `Sync`,
    // so each rayon worker gets its own clone (the LSP's read pattern); clones
    // share the memo storage and are all dropped when the parallel call
    // returns, before the owner handle is used again.
    let warm_file = |worker: &IncrementalDatabase, file: SourceFile| -> usize {
        let count = worker.parse_diagnostics(file).len();
        if count == 0 {
            file_exports(worker, file);
            file_free_reads(worker, file);
            file_qualified_reads(worker, file);
            source_edges(worker, file);
            top_level_events(worker, file);
        }
        count
    };
    let parse_errors: HashMap<PathBuf, usize> = files
        .par_iter()
        .map_with(db.clone(), |worker, path| {
            (path.clone(), warm_file(worker, tracked[path]))
        })
        .filter(|&(_, count)| count != 0)
        .collect();
    scope_only
        .par_iter()
        .map_with(db.clone(), |worker, &file| {
            warm_file(worker, file);
        })
        .collect::<Vec<()>>();

    // Derive the interned project and its graph once on the owner handle —
    // with the per-file memos warm this is a fold over cached values — so the
    // pass-2 workers' re-derives are memo hits rather than a
    // first-computation stampede.
    let _ = workspace_project(&db);

    // The cross-file path resolves undefined symbols through `external_resolution`
    // (which uses the salsa library index), so the provider passed to the rules is
    // only the fallback for rules that read static base-R facts (`is_base`).
    let fallback = default_symbol_provider();

    // Pass 2: lint each cleanly parsed file with its cross-file scope, in
    // parallel on db clones. `Project<'db>` is lifetime-bound to its handle, so
    // each worker re-derives it from its own clone (a memo hit after the force
    // above). The ordered collect keeps reports in `files` order.
    let reports: Vec<LintFileReport> = files
        .into_par_iter()
        .map_with(db.clone(), |worker, path| {
            let worker = &*worker;
            let file = tracked[&path];
            let (status, diagnostics) = if let Some(&count) = parse_errors.get(&path) {
                // Parse errors block the rules but are still reported (not
                // swallowed): surface the parser's side-channel diagnostics as
                // findings.
                let diagnostics = syntax_error_diagnostics(worker.parse_diagnostics(file), &path);
                (LintStatus::ParseDiagnostics { count }, diagnostics)
            } else {
                let project = workspace_project(worker);
                let visibility = visible_symbols(worker, project, file);
                let file_scope = visibility.scope();
                let resolution = external_resolution(worker, manifest, project, file);
                let kept = lint_parsed_file(
                    worker,
                    file,
                    &path,
                    &rules,
                    &fallback,
                    Some(&file_scope),
                    Some(resolution),
                );
                let status = if kept.is_empty() {
                    LintStatus::Clean
                } else {
                    LintStatus::Findings { count: kept.len() }
                };
                (status, kept)
            };
            LintFileReport {
                path,
                status,
                diagnostics,
            }
        })
        .collect();

    let total_findings = reports
        .iter()
        .map(|r| match r.status {
            LintStatus::Findings { count } => count,
            _ => 0,
        })
        .sum();

    Ok(LintResult {
        checked_files: tracked.len(),
        total_findings,
        reports,
    })
}

/// The R sources each analyzed package loads that the exclude filter dropped
/// from `lint_files` — the scope-only members [`check_paths_with_index`] adds so
/// generated wrappers still populate the package namespace.
///
/// For every package root represented in `lint_files`, this is the package's
/// expected `R/` source set (see [`expected_r_sources`]) minus the files already
/// being linted. Generated files (`cpp11.R`, `RcppExports.R`, …) are the usual
/// residents: excluded from linting, but their bindings are real package API.
fn excluded_package_sources(lint_files: &[PathBuf]) -> Vec<PathBuf> {
    let linted: HashSet<&PathBuf> = lint_files.iter().collect();
    let roots: BTreeSet<PathBuf> = lint_files.iter().filter_map(|p| package_root(p)).collect();
    let mut extra = Vec::new();
    for root in roots {
        let r_dir = root.join("R");
        for name in expected_r_sources(&root) {
            let path = r_dir.join(name);
            if !linted.contains(&path) && path.is_file() {
                extra.push(path);
            }
        }
    }
    extra
}

/// Intern a [`Project`] from a membership snapshot. Sorts `members` by path so
/// the interned key is deterministic — an unchanged set always yields the same
/// id, which is what keeps the project-graph memo alive across body edits.
fn intern_project<'db>(
    db: &'db dyn IncrementalDb,
    mut members: Vec<ProjectMember>,
    namespaces: Vec<(PathBuf, String)>,
    collations: Vec<PackageCollation>,
) -> Project<'db> {
    members.sort_by(|a, b| a.path.cmp(&b.path));
    Project::new(db, members, namespaces, collations)
}

/// Run the resolved rules against a cleanly-parsed file, using the cached parse
/// tree and semantic model, and drop suppressed findings. Callers must have
/// already confirmed the file parses without diagnostics.
fn lint_parsed_file(
    db: &dyn IncrementalDb,
    file: SourceFile,
    path: &Path,
    rules: &ResolvedRules,
    provider: &dyn SymbolProvider,
    project: Option<&FileScope<'_>>,
    resolution: Option<&ExternalResolution>,
) -> Vec<Diagnostic> {
    let root_node = parsed_tree_root(db, file);
    let model = semantic_model(db, file);
    let mut diagnostics = run_rules(
        rules, path, &root_node, model, provider, project, resolution,
    );
    let suppress = SuppressionMap::build(&root_node);
    diagnostics.retain(|d| !suppress.is_suppressed(d.rule, d.range));
    for d in &mut diagnostics {
        d.path = path.to_path_buf();
    }
    diagnostics
}

/// Lint a file already tracked in `db`, reusing its cached parse and model.
/// When the file has parse diagnostics, the lint rules can't run (they need a
/// clean tree); instead of dropping them, those parser diagnostics are surfaced
/// as [`SYNTAX_ERROR_RULE`] findings so callers still report the error.
pub fn check_tracked_file(
    db: &IncrementalDatabase,
    file: SourceFile,
    path: &Path,
    config: &LintConfig,
    provider: &dyn SymbolProvider,
) -> Result<Vec<Diagnostic>, LintError> {
    let (rules, unknown) = ResolvedRules::resolve(config.select.as_deref(), &config.ignore);
    if let Some(rule) = unknown.into_iter().next() {
        return Err(LintError::UnknownRule { rule });
    }
    let parse_diagnostics = db.parse_diagnostics(file);
    if !parse_diagnostics.is_empty() {
        return Ok(syntax_error_diagnostics(parse_diagnostics, path));
    }
    Ok(lint_parsed_file(
        db, file, path, &rules, provider, None, None,
    ))
}

/// The write-phase output of cross-file linting: everything [`analyze_prepared`]
/// needs, all derivable with read-only `&db` access afterward. Produced by
/// [`prepare_document_in_project`].
///
/// Splitting the lint into a write-phase ([`prepare_document_in_project`], needs
/// `&mut db`) and a read-phase ([`analyze_prepared`], `&db` only) lets the LSP
/// run the expensive read-phase off its lint thread on a short-lived db clone,
/// where it can be canceled by a fresher edit (see `src/lsp.rs`).
pub struct PreparedProject {
    active: SourceFile,
    /// The resolved rule set, shared (not rebuilt) across lints. The LSP lint
    /// worker caches one `Arc` per lint config and hands a clone to each
    /// keystroke's prepare; the read-phase borrows through it.
    rules: Arc<ResolvedRules>,
    /// Cleanly-parsing project members (incl. `active`), with their tracked
    /// inputs and package roots; files with parse diagnostics are dropped, as
    /// before. Plain owned data — *not* an interned [`Project`] — because the
    /// LSP moves this across a thread boundary onto a different db handle and
    /// interns inside the read-phase ([`analyze_prepared`]).
    members: Vec<ProjectMember>,
    /// `(package_root, NAMESPACE text)` pairs, sorted by root.
    namespaces: Vec<(PathBuf, String)>,
    /// Per package root, its completeness verdict, sorted by root. Snapshotted
    /// from the interned [`Project`] (disk-derived in [`workspace_project`]) so
    /// the read-phase re-interns it without touching disk.
    collations: Vec<PackageCollation>,
}

/// Write-phase of cross-file linting (needs `&mut db`). Discovers the enclosing
/// project — the R package root, else the file's directory — loads its sibling
/// files into `db` (cached across calls, so unchanged siblings aren't re-parsed),
/// and reads the relevant `NAMESPACE` files. `active` must already be tracked in
/// `db` carrying the live editor buffer.
///
/// Returns `None` when the active file has parse diagnostics (the caller
/// publishes no findings, as the old early-return did). The `rules` are resolved
/// by the caller (so the LSP can cache them across keystrokes) and shared into
/// the returned [`PreparedProject`]. All `db` *writes* (`upsert_file`) happen
/// here; the `PreparedProject` is then consumed by the read-only
/// [`analyze_prepared`].
pub fn prepare_document_in_project(
    db: &mut IncrementalDatabase,
    _path: &Path,
    active: SourceFile,
    rules: Arc<ResolvedRules>,
) -> Option<PreparedProject> {
    if !db.parse_diagnostics(active).is_empty() {
        return None;
    }

    // Membership comes from the explicit `Workspace` file-set (seeded by the
    // caller — the LSP's lazy seed or `seed_workspace_for`), not a per-call disk
    // walk. `workspace_project` filters to cleanly-parsing members and reads each
    // package's NAMESPACE; we snapshot its owned membership for the read-phase,
    // which re-interns it on a db clone (so the `Project<'db>` never crosses the
    // thread boundary).
    let project = workspace_project(&*db);
    let members = project.members(&*db).clone();
    let namespaces = project.namespaces(&*db).clone();
    let collations = project.collations(&*db).clone();

    Some(PreparedProject {
        active,
        rules,
        members,
        namespaces,
        collations,
    })
}

/// Fold the project enclosing `path` — its R package root, else its directory —
/// plus `active` into the salsa [`Workspace`](crate::incremental::Workspace)
/// file-set, so [`prepare_document_in_project`] can derive membership from it.
///
/// Walks disk once to discover siblings and unions them into the existing
/// file-set; the conditional setter
/// ([`set_workspace_members`](IncrementalDatabase::set_workspace_members)) makes
/// a repeat call with an unchanged set a no-op. The LSP calls this lazily (only
/// when the active file isn't yet a member), so the walk leaves the per-keystroke
/// path; one-shot callers ([`check_document_in_project`]) call it each time.
pub fn seed_workspace_for(
    db: &mut IncrementalDatabase,
    path: &Path,
    active: SourceFile,
    exclude: &ExcludeFilter,
) {
    let (mut files, mut roots) = match db.workspace() {
        Some(ws) => (ws.members(&*db).to_vec(), ws.roots(&*db).to_vec()),
        None => (Vec::new(), Vec::new()),
    };
    files.push(active);

    let search_dir =
        package_root(path).or_else(|| path.parent().filter(|p| p.is_dir()).map(Path::to_path_buf));
    if let Some(dir) = search_dir {
        for sibling in scope_members(std::slice::from_ref(&dir), exclude) {
            if sibling == path {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&sibling) {
                files.push(db.upsert_file(&sibling, text));
            }
        }
        if !roots.contains(&dir) {
            roots.push(dir);
        }
    }
    db.set_workspace_members(files, roots);
}

/// Resolve the [`ExcludeFilter`] governing files under `anchor`, discovering the
/// `arity.toml` upward from it exactly as the CLI does. Falls back to an
/// exclude-nothing filter when config resolution or pattern compilation fails,
/// so seeding never hard-errors on a malformed workspace config. Used by the
/// single-document seed paths (the LSP and [`check_document_in_project`]).
pub fn resolve_exclude_at(anchor: &Path) -> ExcludeFilter {
    match crate::config::Config::resolve(None, false, anchor) {
        Ok((config, source)) => config
            .exclude_filter(source.as_deref(), anchor, &[])
            .unwrap_or_else(|_| ExcludeFilter::none()),
        Err(_) => ExcludeFilter::none(),
    }
}

/// Discover the R files under `dirs` that belong in the workspace *scope*: every
/// file `exclude` keeps, plus the generated package sources it drops (so their
/// wrappers still populate the package namespace). Mirrors the scope-only
/// handling in [`check_paths_with_index`] — without the re-add, excluding
/// `cpp11.R`/`RcppExports.R` would make every caller a false `undefined-symbol`.
pub fn scope_members(dirs: &[PathBuf], exclude: &ExcludeFilter) -> Vec<PathBuf> {
    let mut files = collect_r_files(dirs, exclude).unwrap_or_default();
    files.extend(excluded_package_sources(&files));
    files.sort();
    files.dedup();
    files
}

/// Read-phase of cross-file linting (`&db` only — no disk, no writes). Builds the
/// per-file facts from cached models/trees, assembles the project scope, and
/// lints the active file against it. Safe to run on a db clone; salsa aborts it
/// with [`salsa::Cancelled`] (at the next tracked-query entry) if a write races.
pub fn analyze_prepared(
    analysis: &Analysis,
    prepared: &PreparedProject,
    provider: &dyn SymbolProvider,
) -> Vec<Diagnostic> {
    // One `&dyn IncrementalDb` borrow for the read-phase: a single `'db`
    // lifetime keeps the interned `Project<'db>` and `visible_symbols` aligned.
    let db = analysis.as_db();
    // Intern the project here (read-phase): the membership snapshot is plain
    // owned data in `prepared`, so this is safe on a db clone, and an unchanged
    // set re-interns to the same id — keeping the project-graph memo warm.
    let project = intern_project(
        db,
        prepared.members.clone(),
        prepared.namespaces.clone(),
        prepared.collations.clone(),
    );
    let active_path = analysis
        .file_path(prepared.active)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let visibility = visible_symbols(db, project, prepared.active);
    let file_scope = visibility.scope();
    // Resolve undefined symbols through the salsa library index when one is
    // installed (HIGH-durability, so it survives keystrokes); else fall back to
    // the threaded `provider`. The query memoizes and backdates across body edits.
    let resolution = analysis
        .library_index()
        .map(|manifest| external_resolution(db, manifest, project, prepared.active));
    lint_parsed_file(
        db,
        prepared.active,
        &active_path,
        &prepared.rules,
        provider,
        Some(&file_scope),
        resolution,
    )
}

/// Lint `path` (already tracked in `db` as `active`, carrying the live editor
/// buffer) with cross-file resolution. Thin wrapper over the write-phase
/// ([`prepare_document_in_project`]) and read-phase ([`analyze_prepared`]); used
/// by the CLI and tests. The LSP drives the two phases separately so the
/// read-phase can run cancellably off its lint thread.
pub fn check_document_in_project(
    db: &mut IncrementalDatabase,
    path: &Path,
    active: SourceFile,
    config: &LintConfig,
    provider: &dyn SymbolProvider,
) -> Result<Vec<Diagnostic>, LintError> {
    let (rules, unknown) = ResolvedRules::resolve(config.select.as_deref(), &config.ignore);
    if let Some(rule) = unknown.into_iter().next() {
        return Err(LintError::UnknownRule { rule });
    }
    let exclude = resolve_exclude_at(path.parent().unwrap_or(path));
    seed_workspace_for(db, path, active, &exclude);
    match prepare_document_in_project(db, path, active, Arc::new(rules)) {
        Some(prepared) => {
            let analysis = db.snapshot();
            Ok(analyze_prepared(&analysis, &prepared, provider))
        }
        None => Ok(Vec::new()),
    }
}

/// Convenience: lint a single in-memory document by path + text (used by quick
/// fixes and tests). Builds a one-shot database; the LSP's hot lint path uses
/// [`check_tracked_file`] against its persistent database instead.
pub fn check_document(
    path: &Path,
    content: &str,
    config: &LintConfig,
) -> Result<Vec<Diagnostic>, LintError> {
    check_document_with_provider(path, content, config, &default_symbol_provider())
}

/// Like [`check_document`] but with a caller-supplied symbol provider.
pub fn check_document_with_provider(
    path: &Path,
    content: &str,
    config: &LintConfig,
    provider: &dyn SymbolProvider,
) -> Result<Vec<Diagnostic>, LintError> {
    let db = IncrementalDatabase::default();
    let file = db.add_file(content.to_string());
    check_tracked_file(&db, file, path, config, provider)
}
