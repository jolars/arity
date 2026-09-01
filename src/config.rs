//! `arity.toml` configuration: schema, file loading, and ancestor-walk discovery.
//!
//! The CLI is the primary consumer; the library API (`format_with_style`,
//! `check_paths_with_style`, ...) continues to take a fully-resolved
//! [`FormatStyle`]. The LSP and `arity index` also resolve config for the
//! [`Config::exclude_filter`] path so in-editor and index walks honor the same
//! `exclude`/`extend-exclude` as the CLI.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::file_discovery::{ExcludeError, ExcludeFilter};
use crate::formatter::{FormatStyle, LineEnding};

pub const CONFIG_FILE_NAME: &str = "arity.toml";

const MIN_WIDTH: u32 = 1;
const MAX_WIDTH: u32 = 1000;

const DEFAULT_LINE_WIDTH: u32 = 80;
const DEFAULT_INDENT_WIDTH: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    /// Gitignore-style patterns to exclude from file discovery, resolved
    /// relative to the directory containing this `arity.toml`. Applies to *both*
    /// `format` and `lint` (which share one file walk), so it is a top-level
    /// key, not nested under `[format]`.
    ///
    /// Setting this **replaces** the built-in [`DEFAULT_EXCLUDE`] set (it
    /// defaults to that set); use [`extend_exclude`](Self::extend_exclude) to
    /// add patterns without dropping the defaults.
    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,
    /// Gitignore-style patterns to exclude *in addition to*
    /// [`exclude`](Self::exclude). Unlike `exclude`, this never replaces the
    /// defaults, so it is the right key for project-specific additions.
    #[serde(default)]
    pub extend_exclude: Vec<String>,
    #[serde(default)]
    pub format: FormatConfig,
    #[serde(default)]
    pub lint: LintConfig,
    #[serde(default)]
    pub index: IndexConfig,
    /// Minimum supported tool versions the project targets. A top-level table
    /// (not under `[lint]`) because it states a project fact, not a lint
    /// option — the lint rules are merely its first consumer. When a key is
    /// absent it is derived from the package `DESCRIPTION` where possible;
    /// see [`CompatConfig`].
    #[serde(default)]
    pub compat: CompatConfig,
    /// Enable the persistent result cache (currently the `format --check`
    /// already-formatted cache). A top-level key because it will govern the
    /// lint cache too. The `--no-cache` CLI flag overrides this to `false`.
    #[serde(default = "default_true")]
    pub cache: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: default_exclude(),
            extend_exclude: Vec::new(),
            format: FormatConfig::default(),
            lint: LintConfig::default(),
            index: IndexConfig::default(),
            compat: CompatConfig::default(),
            cache: true,
        }
    }
}

/// Built-in exclude patterns, applied as the default value of `exclude` (and so
/// dropped when `exclude` is set explicitly). These are generated or vendored
/// files that should never be reformatted or linted. The set mirrors air's
/// defaults so the two tools agree on what to skip.
pub const DEFAULT_EXCLUDE: &[&str] = &[
    ".git/",
    "renv/",
    "revdep/",
    "cpp11.R",
    "RcppExports.R",
    "extendr-wrappers.R",
    "import-standalone-*.R",
];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct FormatConfig {
    #[serde(default = "default_line_width")]
    #[schemars(range(min = MIN_WIDTH, max = MAX_WIDTH))]
    pub line_width: u32,
    #[serde(default = "default_indent_width")]
    #[schemars(range(min = MIN_WIDTH, max = MAX_WIDTH))]
    pub indent_width: u32,
    /// The newline style the formatter emits. See [`LineEndingConfig`].
    #[serde(default)]
    pub line_ending: LineEndingConfig,
    /// Whether a package's `DESCRIPTION` is formatted. On by default.
    ///
    /// The off switch exists because arity is not the only tool that writes this
    /// file — `usethis` and `roxygen2` do too — and the honest answer to "arity
    /// and my package tooling disagree about my `DESCRIPTION`" has to be
    /// something other than "stop running `arity format`".
    ///
    /// It cannot be `extend-exclude`: excludes are shared with the linter, so
    /// excluding `DESCRIPTION` to stop formatting would also silence the
    /// packaging rules, and the language server applies no exclude filter to
    /// formatting at all.
    #[serde(default = "default_true")]
    pub description: bool,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            line_width: DEFAULT_LINE_WIDTH,
            indent_width: DEFAULT_INDENT_WIDTH,
            line_ending: LineEndingConfig::default(),
            description: true,
        }
    }
}

/// The `line-ending` key under `[format]`. A thin, serde-named mirror of
/// [`LineEnding`] (the formatter's own type), kept separate so the TOML spelling
/// (`kebab-case`) is a config concern, not baked into the formatter API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LineEndingConfig {
    /// Detect per file from the source; default `\n` when none is present.
    #[default]
    Auto,
    /// Always `\n` (Unix).
    Lf,
    /// Always `\r\n` (Windows).
    Crlf,
    /// `\n` on Unix, `\r\n` on Windows.
    Native,
}

impl From<LineEndingConfig> for LineEnding {
    fn from(value: LineEndingConfig) -> Self {
        match value {
            LineEndingConfig::Auto => LineEnding::Auto,
            LineEndingConfig::Lf => LineEnding::Lf,
            LineEndingConfig::Crlf => LineEnding::Crlf,
            LineEndingConfig::Native => LineEnding::Native,
        }
    }
}

impl FormatConfig {
    /// Validate values, returning an [`ConfigError::InvalidValue`] with the
    /// originating file path (when known) for diagnostics.
    pub fn validate(&self, path: Option<&Path>) -> Result<(), ConfigError> {
        validate_width("line-width", self.line_width, path)?;
        validate_width("indent-width", self.indent_width, path)?;
        Ok(())
    }
}

fn default_line_width() -> u32 {
    DEFAULT_LINE_WIDTH
}

fn default_indent_width() -> u32 {
    DEFAULT_INDENT_WIDTH
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct LintConfig {
    /// Explicit allowlist of rule IDs. When `Some`, only these rules run.
    /// Unknown rule IDs are reported at lint-time, not at config parse-time.
    #[serde(default)]
    pub select: Option<Vec<String>>,
    /// Rule IDs to disable. Applied on top of either `select` (subtracts) or
    /// the default rule set.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Per-rule option tables (`[lint.rules.<id>]`).
    #[serde(default)]
    pub rules: RulesConfig,
    /// The resolved top-level `[compat]` table, copied here when the config is
    /// parsed (`Config::parse_str`) so every consumer that ships a
    /// [`LintConfig`] — the CLI, the LSP's lint thread — carries the compat
    /// floors without a parallel plumbing path. `#[serde(skip)]` (like
    /// `IndexConfig::remote_url`): the authored key is the *top-level*
    /// `[compat]`, never `[lint.compat]`, which `deny_unknown_fields` rejects.
    #[serde(skip)]
    pub compat: CompatConfig,
}

/// `[lint.rules]` — per-rule option tables, one field per *configurable* rule.
///
/// Deliberately typed rather than a `Map<String, Value>` keyed by rule ID: the
/// `deny_unknown_fields` convention then makes a mistyped rule ID a parse error
/// instead of a silently-ignored table. Note the asymmetry this creates with
/// `select`/`ignore`, where an unknown ID is reported at *lint* time — there the
/// IDs are free-form data, here they are schema.
///
/// Most rules take no options and so have no field here.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct RulesConfig {
    /// `[lint.rules.undesirable-function]`
    #[serde(default)]
    pub undesirable_function: UndesirableFunctionConfig,
}

/// `[lint.rules.undesirable-function]` — the function-name policy for the
/// `undesirable-function` rule.
///
/// The `functions`/`extend-functions` pair mirrors the top-level
/// `exclude`/`extend-exclude` idiom: the base key **replaces** the built-in set
/// ([`default_undesirable_functions`]), the `extend-` key **adds** to whichever
/// set that resolved to. Use [`resolved`] rather than reading either field
/// directly.
///
/// [`resolved`]: UndesirableFunctionConfig::resolved
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct UndesirableFunctionConfig {
    /// Function name -> suggested alternative. An empty string means "no
    /// alternative, just don't call this". Replaces the built-in set; `{}`
    /// therefore silences the rule entirely.
    #[serde(default = "default_undesirable_functions")]
    pub functions: BTreeMap<String, String>,
    /// Entries added on top of `functions`, overriding same-named ones. The
    /// usual way to extend the built-in set without restating it.
    #[serde(default)]
    pub extend_functions: BTreeMap<String, String>,
}

impl Default for UndesirableFunctionConfig {
    fn default() -> Self {
        Self {
            functions: default_undesirable_functions(),
            extend_functions: BTreeMap::new(),
        }
    }
}

impl UndesirableFunctionConfig {
    /// The suggestion configured for `name`, or `None` if it is not flagged.
    ///
    /// Equivalent to `self.resolved().get(name)` but allocation-free, which
    /// matters because the rule asks this once per call expression in the file:
    /// `extend-functions` wins, so it is consulted first.
    pub fn lookup(&self, name: &str) -> Option<&str> {
        self.extend_functions
            .get(name)
            .or_else(|| self.functions.get(name))
            .map(String::as_str)
    }

    /// The effective name -> suggestion map: `functions` with `extend-functions`
    /// layered on top. Materializes the map; prefer [`lookup`] on a hot path.
    ///
    /// [`lookup`]: UndesirableFunctionConfig::lookup
    pub fn resolved(&self) -> BTreeMap<String, String> {
        let mut out = self.functions.clone();
        out.extend(
            self.extend_functions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        out
    }
}

/// The built-in `undesirable-function` set: base-R functions that reach outside
/// the current evaluation and mutate global state, plus the debugging entry
/// points that should never survive into committed code.
///
/// Deliberately conservative, and deliberately disjoint from the rules that
/// already own a name: `browser` has its own rule, and `ifelse` overlaps
/// `redundant-ifelse`. Style-contentious names (`sapply`, `mapply`,
/// `library`/`require`) are left out — a user who wants them adds them via
/// `extend-functions`.
fn default_undesirable_functions() -> BTreeMap<String, String> {
    [
        ("attach", "use `with()` or refer to columns explicitly"),
        ("detach", "avoid modifying the search path"),
        (".libPaths", "set `R_LIBS` outside the script"),
        (
            "install.packages",
            "declare dependencies in DESCRIPTION or renv",
        ),
        ("setwd", "use paths relative to the project root"),
        ("sink", "use `capture.output()` or an explicit connection"),
        ("source", "make the code a package or use `box::use()`"),
        ("options", "set options in the session, not in library code"),
        ("par", "restore graphical parameters with `on.exit()`"),
        ("Sys.setenv", "set the environment outside the script"),
        ("Sys.setlocale", "set the locale outside the script"),
        ("debug", "remove the debugging call"),
        ("debugonce", "remove the debugging call"),
        ("undebug", "remove the debugging call"),
        ("trace", "remove the debugging call"),
        ("untrace", "remove the debugging call"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// `[compat]` — the minimum tool versions the project supports, MSRV-style
/// (clippy's `msrv`, ruff's `target-version`). Consumed by the version-aware
/// lint rules (`r-compat`, `roxygen2-compat`): syntax or documentation
/// constructs needing a *newer* version than the declared floor are flagged.
///
/// Resolution order, per file: an explicit key here wins; an absent key is
/// derived from the enclosing package's `DESCRIPTION` (`Depends: R (>= …)` for
/// `r`; `Config/roxygen2/version`, then the legacy `RoxygenNote`, for
/// `roxygen2` — see `project::description`); with neither, the version-aware
/// rules stay silent (no floor, nothing to flag), so loose scripts see no
/// false positives.
///
/// Values are plain version strings (`"4.1"`, `"7.3.2"`), not requirement
/// specs — the key *is* the `>=` floor.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct CompatConfig {
    /// Minimum supported R version, e.g. `"4.1"`.
    #[serde(default)]
    #[schemars(regex(pattern = r"^[0-9]+(?:[.-][0-9]+)*$"))]
    pub r: Option<String>,
    /// The roxygen2 version the project documents with, e.g. `"7.3.2"`.
    #[serde(default)]
    #[schemars(regex(pattern = r"^[0-9]+(?:[.-][0-9]+)*$"))]
    pub roxygen2: Option<String>,
}

impl CompatConfig {
    /// Validate that both keys, when present, parse as version strings.
    fn validate(&self, path: Option<&Path>) -> Result<(), ConfigError> {
        for (field, value) in [("compat.r", &self.r), ("compat.roxygen2", &self.roxygen2)] {
            if let Some(text) = value
                && CompatVersion::parse(text).is_none()
            {
                return Err(ConfigError::InvalidValue {
                    path: path.map(Path::to_path_buf),
                    field,
                    message: format!(
                        "`{text}` is not a version string (expected dot- or \
                         dash-separated numbers, e.g. \"4.1\" or \"7.3.2\")"
                    ),
                });
            }
        }
        Ok(())
    }

    /// The parsed `r` floor, when configured. Infallible after [`validate`].
    ///
    /// [`validate`]: CompatConfig::validate
    pub fn r_version(&self) -> Option<CompatVersion> {
        self.r.as_deref().and_then(CompatVersion::parse)
    }

    /// The parsed `roxygen2` floor, when configured. Infallible after
    /// [`validate`].
    ///
    /// [`validate`]: CompatConfig::validate
    pub fn roxygen2_version(&self) -> Option<CompatVersion> {
        self.roxygen2.as_deref().and_then(CompatVersion::parse)
    }
}

/// A parsed tool version for floor comparisons: numeric components in order,
/// compared componentwise with a missing component reading as zero
/// (`4.1 == 4.1.0 < 4.1.1`). The zero-padding deliberately diverges from R's
/// `utils::compareVersion` (where the shorter version *loses* on a shared
/// prefix): a declared `4.0` floor must satisfy a construct introduced "in
/// 4.0.0" — these are `>=` floors, not release identities. R version strings
/// separate components with `.` or `-` (`"1.2-3"`), and so does this parser.
#[derive(Debug, Clone, Eq)]
pub struct CompatVersion(Vec<u32>);

impl PartialEq for CompatVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Ord for CompatVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let len = self.0.len().max(other.0.len());
        for i in 0..len {
            let a = self.0.get(i).copied().unwrap_or(0);
            let b = other.0.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                std::cmp::Ordering::Equal => continue,
                order => return order,
            }
        }
        std::cmp::Ordering::Equal
    }
}

impl PartialOrd for CompatVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl CompatVersion {
    /// Parse a version string: one or more non-empty numeric components
    /// separated by `.` or `-`. `None` on anything else (empty string, empty
    /// or non-numeric components).
    pub fn parse(text: &str) -> Option<Self> {
        let components: Option<Vec<u32>> = text
            .split(['.', '-'])
            .map(|c| {
                (!c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| c.parse().ok())
                    .flatten()
            })
            .collect();
        components.filter(|c| !c.is_empty()).map(CompatVersion)
    }
}

impl fmt::Display for CompatVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for c in &self.0 {
            if !first {
                write!(f, ".")?;
            }
            write!(f, "{c}")?;
            first = false;
        }
        Ok(())
    }
}

/// `[index]` — the R-introspection sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct IndexConfig {
    /// Explicit R library directories, used when automatic `.libPaths()`
    /// discovery misses (highest-priority source).
    #[serde(default)]
    pub library_paths: Vec<PathBuf>,
    /// Override the cache directory (otherwise XDG / `$ARITY_CACHE_DIR`).
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
    /// Let the LSP lazily build indices for referenced-but-unindexed packages.
    #[serde(default = "default_true")]
    pub auto_build: bool,
    /// Harvest help (titles in this phase). When false, only names are stored.
    #[serde(default = "default_true")]
    pub help: bool,
    /// Base URL of a downloadable CRAN symbol sidecar. When set, the LSP fetches
    /// names-only export lists for referenced-but-uninstalled packages over the
    /// network (cached on disk); `None` keeps arity fully offline.
    ///
    /// Deliberately **not** read from `arity.toml` (`#[serde(skip)]`): enabling
    /// network egress is a per-user, per-machine consent decision, not a shared,
    /// committed project setting. The LSP populates it from the `ARITY_REMOTE_URL`
    /// environment variable instead (see `resolve_settings` in `src/lsp/state.rs`).
    #[serde(skip)]
    pub remote_url: Option<String>,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            library_paths: Vec::new(),
            cache_dir: None,
            auto_build: true,
            help: true,
            remote_url: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// The default value of `exclude`: the built-in [`DEFAULT_EXCLUDE`] set as owned
/// strings. Setting `exclude` in the config replaces this wholesale.
fn default_exclude() -> Vec<String> {
    DEFAULT_EXCLUDE.iter().map(|p| p.to_string()).collect()
}

impl From<&FormatConfig> for FormatStyle {
    fn from(config: &FormatConfig) -> Self {
        FormatStyle {
            line_width: config.line_width as usize,
            indent_width: config.indent_width as usize,
            line_ending: config.line_ending.into(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        line: usize,
        column: usize,
        message: String,
    },
    InvalidValue {
        path: Option<PathBuf>,
        field: &'static str,
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::Parse {
                path,
                line,
                column,
                message,
            } => write!(f, "{}:{line}:{column}: {message}", path.display()),
            Self::InvalidValue {
                path,
                field,
                message,
            } => match path {
                Some(path) => write!(f, "{}: invalid `{field}`: {message}", path.display()),
                None => write!(f, "invalid `{field}`: {message}"),
            },
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl Config {
    /// Parse a `arity.toml` from disk and validate it.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse_str(&text, path)
    }

    fn parse_str(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let mut config: Self = toml::from_str(text).map_err(|err| {
            let (line, column) = match err.span() {
                Some(span) => byte_offset_to_line_col(text, span.start),
                None => (1, 1),
            };
            ConfigError::Parse {
                path: path.to_path_buf(),
                line,
                column,
                message: err.message().to_string(),
            }
        })?;
        config.validate(Some(path))?;
        // Mirror the top-level `[compat]` onto the lint section (see
        // `LintConfig::compat`).
        config.lint.compat = config.compat.clone();
        Ok(config)
    }

    fn validate(&self, path: Option<&Path>) -> Result<(), ConfigError> {
        self.format.validate(path)?;
        self.compat.validate(path)
    }

    /// Walk `start` and its ancestors looking for a `arity.toml`. Stops at the
    /// first match or at a directory that contains a `.git` entry (repo root),
    /// whichever comes first. Returns `None` if neither is found before the
    /// filesystem root.
    ///
    /// `start` need not exist on disk — see [`deepest_existing_ancestor`].
    pub fn discover(start: &Path) -> Result<Option<(PathBuf, Self)>, ConfigError> {
        let Some(canonical) = deepest_existing_ancestor(start) else {
            return Ok(None);
        };
        for dir in canonical.ancestors() {
            let candidate = dir.join(CONFIG_FILE_NAME);
            if candidate.is_file() {
                let config = Self::load_from(&candidate)?;
                return Ok(Some((candidate, config)));
            }
            if dir.join(".git").exists() {
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// CLI resolution. Returns the final config plus the source path of the
    /// loaded file (for diagnostics), if any. CLI flag overrides for the
    /// formatter knobs are applied by the caller after this returns.
    pub fn resolve(
        explicit: Option<&Path>,
        no_config: bool,
        anchor: &Path,
    ) -> Result<(Self, Option<PathBuf>), ConfigError> {
        if no_config {
            return Ok((Self::default(), None));
        }
        if let Some(path) = explicit {
            let config = Self::load_from(path)?;
            return Ok((config, Some(path.to_path_buf())));
        }
        match Self::discover(anchor)? {
            Some((path, config)) => Ok((config, Some(path))),
            None => Ok((Self::default(), None)),
        }
    }

    /// Build the file-discovery [`ExcludeFilter`] from this config's `exclude` +
    /// `extend-exclude` (plus any `extra` patterns, e.g. CLI `--exclude`).
    /// Patterns are rooted at the directory containing the loaded config file
    /// (`source`), or at `anchor` when there is no config file. This is the single
    /// source of truth for turning a resolved config into an exclude filter, shared
    /// by the CLI walks, the LSP workspace seed, and `arity index` discovery.
    pub fn exclude_filter(
        &self,
        source: Option<&Path>,
        anchor: &Path,
        extra: &[String],
    ) -> Result<ExcludeFilter, ExcludeError> {
        let root = source.and_then(Path::parent).unwrap_or(anchor);
        let mut patterns = self.exclude.clone();
        patterns.extend(self.extend_exclude.iter().cloned());
        patterns.extend(extra.iter().cloned());
        ExcludeFilter::new(root, &patterns)
    }
}

/// The deepest ancestor of `start` (itself included) that exists on disk, in
/// canonical form, or `None` when none is reachable.
///
/// Discovery must tolerate an anchor that isn't on disk: the LSP anchors on the
/// parent directory of the buffer being edited, which may never have been
/// created (an unsaved buffer, a directory removed while a file in it is open,
/// or a path like `C:\tmp` that simply isn't there on Windows). Skipping the
/// missing tail loses nothing, because discovery only ever reads *from*
/// directories and a directory that doesn't exist can hold neither an
/// `arity.toml` nor a `.git`. An unreachable tree is likewise not an error —
/// there is no config to read, which is exactly what `None` reports.
fn deepest_existing_ancestor(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| dir.canonicalize().ok())
}

fn validate_width(field: &'static str, value: u32, path: Option<&Path>) -> Result<(), ConfigError> {
    if !(MIN_WIDTH..=MAX_WIDTH).contains(&value) {
        return Err(ConfigError::InvalidValue {
            path: path.map(Path::to_path_buf),
            field,
            message: format!("must be between {MIN_WIDTH} and {MAX_WIDTH}, got {value}"),
        });
    }
    Ok(())
}

fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;
    let clamped = offset.min(source.len());
    for ch in source[..clamped].chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse_str(text, Path::new("arity.toml"))
    }

    #[test]
    fn compat_table_parses_and_resolves() {
        let config = parse("[compat]\nr = \"4.1\"\nroxygen2 = \"7.3.2\"\n").unwrap();
        assert_eq!(config.compat.r_version(), CompatVersion::parse("4.1"));
        assert_eq!(
            config.compat.roxygen2_version(),
            CompatVersion::parse("7.3.2")
        );
        // Absent keys resolve to no floor.
        let config = parse("").unwrap();
        assert_eq!(config.compat.r_version(), None);
        assert_eq!(config.compat.roxygen2_version(), None);
    }

    #[test]
    fn compat_invalid_version_is_a_config_error() {
        let err = parse("[compat]\nr = \"latest\"\n").unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::InvalidValue {
                    field: "compat.r",
                    ..
                }
            ),
            "{err}"
        );
        // A mistyped key is a parse error (`deny_unknown_fields`).
        assert!(parse("[compat]\nroxygen = \"7.0\"\n").is_err());
    }

    #[test]
    fn compat_version_ordering_zero_pads() {
        let v = |s: &str| CompatVersion::parse(s).unwrap();
        // Componentwise with missing components reading as zero (floor
        // semantics: a declared `4.1` floor satisfies a 4.1.0 requirement),
        // and `-` accepted as a separator.
        assert!(v("4.1") == v("4.1.0"));
        assert!(v("4.1") < v("4.1.1"));
        assert!(v("4.1.0") < v("4.2"));
        assert!(v("4.10") > v("4.9"));
        assert!(v("1.2-3") == v("1.2.3"));
        assert!(v("7.3.2") < v("7.3.2.9000"));
        assert_eq!(CompatVersion::parse(""), None);
        assert_eq!(CompatVersion::parse("4."), None);
        assert_eq!(CompatVersion::parse("v4.1"), None);
    }

    #[test]
    fn exclude_filter_from_config_applies_patterns() {
        use std::fs;
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("keep.R"), "x <- 1\n").unwrap();
        fs::create_dir(root.join("vendor")).unwrap();
        fs::write(root.join("vendor").join("skip.R"), "y <- 2\n").unwrap();

        let config = Config {
            exclude: vec!["vendor/".to_string()],
            ..Config::default()
        };
        let filter = config.exclude_filter(None, root, &[]).unwrap();
        let files = crate::file_discovery::collect_r_files(&[root.to_path_buf()], &filter).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["keep.R".to_string()]);
    }

    #[test]
    fn exclude_filter_extra_and_extend_apply_together() {
        use std::fs;
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("keep.R"), "x <- 1\n").unwrap();
        fs::create_dir(root.join("gen")).unwrap();
        fs::write(root.join("gen").join("a.R"), "y <- 2\n").unwrap();
        fs::create_dir(root.join("cli")).unwrap();
        fs::write(root.join("cli").join("b.R"), "z <- 3\n").unwrap();

        let config = Config {
            exclude: Vec::new(),
            extend_exclude: vec!["gen/".to_string()],
            ..Config::default()
        };
        let filter = config
            .exclude_filter(None, root, &["cli/".to_string()])
            .unwrap();
        let files = crate::file_discovery::collect_r_files(&[root.to_path_buf()], &filter).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["keep.R".to_string()]);
    }

    #[test]
    fn default_config_matches_format_style_default() {
        let config = Config::default();
        let style = FormatStyle::from(&config.format);
        assert_eq!(style, FormatStyle::default());
    }

    #[test]
    fn cache_defaults_true_and_parses_false() {
        assert!(parse("").expect("parse").cache);
        assert!(!parse("cache = false\n").expect("parse").cache);
    }

    #[test]
    fn parses_minimal_format_section() {
        let config = parse("[format]\nline-width = 100\n").expect("parse");
        let style = FormatStyle::from(&config.format);
        assert_eq!(style.line_width, 100);
        assert_eq!(style.indent_width, 2);
    }

    #[test]
    fn parses_indent_width() {
        let config = parse("[format]\nindent-width = 4\n").expect("parse");
        let style = FormatStyle::from(&config.format);
        assert_eq!(style.indent_width, 4);
        assert_eq!(style.line_width, 80);
    }

    #[test]
    fn line_ending_defaults_to_auto() {
        let config = parse("[format]\n").expect("parse");
        assert_eq!(config.format.line_ending, LineEndingConfig::Auto);
        let style = FormatStyle::from(&config.format);
        assert_eq!(style.line_ending, LineEnding::Auto);
    }

    #[test]
    fn parses_line_ending_variants() {
        for (key, expected) in [
            ("auto", LineEndingConfig::Auto),
            ("lf", LineEndingConfig::Lf),
            ("crlf", LineEndingConfig::Crlf),
            ("native", LineEndingConfig::Native),
        ] {
            let text = format!("[format]\nline-ending = \"{key}\"\n");
            let config = parse(&text).unwrap_or_else(|e| panic!("parse {key}: {e}"));
            assert_eq!(config.format.line_ending, expected, "for {key}");
        }
    }

    #[test]
    fn rejects_unknown_line_ending() {
        let err = parse("[format]\nline-ending = \"mac\"\n").expect_err("unknown variant");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn empty_file_yields_defaults() {
        let config = parse("").expect("parse");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn rejects_unknown_top_level_table() {
        let err = parse("[formatt]\nline-width = 80\n").expect_err("unknown table");
        match err {
            ConfigError::Parse { message, .. } => {
                assert!(message.contains("formatt"), "got: {message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_field_in_format() {
        let err = parse("[format]\nline-widht = 80\n").expect_err("unknown field");
        match err {
            ConfigError::Parse { message, .. } => {
                assert!(message.contains("line-widht"), "got: {message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_snake_case_keys() {
        // We use kebab-case in the schema; snake_case must be rejected so users
        // get a clear error instead of silent fallthrough to defaults.
        let err = parse("[format]\nline_width = 80\n").expect_err("snake_case");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn rejects_zero_line_width() {
        let err = parse("[format]\nline-width = 0\n").expect_err("zero width");
        match err {
            ConfigError::InvalidValue { field, message, .. } => {
                assert_eq!(field, "line-width");
                assert!(message.contains('0'));
            }
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn rejects_huge_line_width() {
        let err = parse("[format]\nline-width = 10000\n").expect_err("too big");
        assert!(matches!(
            err,
            ConfigError::InvalidValue {
                field: "line-width",
                ..
            }
        ));
    }

    #[test]
    fn rejects_negative_width_as_parse_error() {
        // u32 deserialization rejects negatives at the type layer.
        let err = parse("[format]\nline-width = -1\n").expect_err("negative");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn exclude_defaults_to_builtin_set_and_extend_is_empty() {
        let config = Config::default();
        assert_eq!(config.exclude, default_exclude());
        assert!(config.extend_exclude.is_empty());
    }

    #[test]
    fn parses_top_level_exclude_and_extend_exclude() {
        let config =
            parse("exclude = [\"vendor/\", \"*.gen.R\"]\nextend-exclude = [\"generated/\"]\n")
                .expect("parse");
        // `exclude` replaces the built-in defaults wholesale.
        assert_eq!(
            config.exclude,
            vec!["vendor/".to_string(), "*.gen.R".to_string()]
        );
        assert_eq!(config.extend_exclude, vec!["generated/".to_string()]);
    }

    #[test]
    fn extend_exclude_keeps_defaults() {
        // Setting only `extend-exclude` leaves `exclude` at the default set.
        let config = parse("extend-exclude = [\"generated/\"]\n").expect("parse");
        assert_eq!(config.exclude, default_exclude());
        assert_eq!(config.extend_exclude, vec!["generated/".to_string()]);
    }

    #[test]
    fn rejects_exclude_under_format() {
        // `exclude` is a top-level key (it governs both format and lint), never
        // nested under `[format]`.
        let err = parse("[format]\nexclude = [\"x\"]\n").expect_err("exclude is top-level");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn accepts_empty_lint_section() {
        let config = parse("[lint]\n").expect("parse");
        assert_eq!(config.lint, LintConfig::default());
    }

    #[test]
    fn rejects_unknown_field_in_lint() {
        let err = parse("[lint]\nstyle = \"strict\"\n").expect_err("unknown field");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn parses_lint_select() {
        let config = parse("[lint]\nselect = [\"unused-binding\"]\n").expect("parse");
        assert_eq!(
            config.lint.select.as_deref(),
            Some(&["unused-binding".to_string()][..])
        );
    }

    #[test]
    fn parses_index_section() {
        let config = parse(concat!(
            "[index]\n",
            "library-paths = [\"/opt/R/lib\", \"~/rlibs\"]\n",
            "cache-dir = \"/tmp/arity-cache\"\n",
            "auto-build = false\n",
            "help = false\n",
        ))
        .expect("parse");
        assert_eq!(
            config.index.library_paths,
            vec![PathBuf::from("/opt/R/lib"), PathBuf::from("~/rlibs")]
        );
        assert_eq!(
            config.index.cache_dir.as_deref(),
            Some(Path::new("/tmp/arity-cache"))
        );
        assert!(!config.index.auto_build);
        assert!(!config.index.help);
    }

    #[test]
    fn index_section_defaults() {
        let config = parse("[index]\n").expect("parse");
        assert_eq!(config.index, IndexConfig::default());
        assert!(config.index.auto_build);
        assert!(config.index.help);
        assert!(config.index.library_paths.is_empty());
        assert_eq!(config.index.cache_dir, None);
    }

    #[test]
    fn rejects_unknown_field_in_index() {
        let err = parse("[index]\nlibrary-path = [\"/x\"]\n").expect_err("unknown field");
        match err {
            ConfigError::Parse { message, .. } => {
                assert!(message.contains("library-path"), "got: {message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn index_remote_url_defaults_to_none() {
        let config = parse("[index]\n").expect("parse");
        assert_eq!(config.index.remote_url, None);
        assert_eq!(config.index, IndexConfig::default());
    }

    #[test]
    fn rejects_remote_url_in_config() {
        // Enabling network egress is a per-user/per-machine consent decision, not a
        // shared project setting: it lives in `ARITY_REMOTE_URL`, never arity.toml.
        // A `remote-url` key in the shared config is rejected as unknown.
        let err = parse("[index]\nremote-url = \"https://sidecar.example/cran\"\n")
            .expect_err("remote-url is not a config key");
        match err {
            ConfigError::Parse { message, .. } => {
                assert!(message.contains("remote-url"), "got: {message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn parses_lint_ignore() {
        let config = parse("[lint]\nignore = [\"undefined-symbol\"]\n").expect("parse");
        assert_eq!(config.lint.ignore, vec!["undefined-symbol".to_string()]);
    }

    #[test]
    fn accepts_empty_lint_rules_section() {
        let config = parse("[lint.rules]\n").expect("parse");
        assert_eq!(config.lint.rules, RulesConfig::default());
    }

    #[test]
    fn accepts_empty_undesirable_function_table() {
        let config = parse("[lint.rules.undesirable-function]\n").expect("parse");
        assert_eq!(
            config.lint.rules.undesirable_function,
            UndesirableFunctionConfig::default()
        );
        // The built-in set is the default; an empty table changes nothing.
        assert_eq!(
            config.lint.rules.undesirable_function.resolved(),
            default_undesirable_functions()
        );
    }

    #[test]
    fn undesirable_function_functions_replaces_the_builtin_set() {
        // Mirrors `exclude` vs `DEFAULT_EXCLUDE`: the base key *replaces*.
        let config = parse(concat!(
            "[lint.rules.undesirable-function]\n",
            "functions = { sapply = \"use `vapply()`\" }\n",
        ))
        .expect("parse");
        let resolved = config.lint.rules.undesirable_function.resolved();
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved.get("sapply").map(String::as_str),
            Some("use `vapply()`")
        );
        assert!(
            !resolved.contains_key("attach"),
            "`functions` must replace, not extend: {resolved:?}"
        );
    }

    #[test]
    fn undesirable_function_extend_functions_adds_to_the_builtin_set() {
        let config = parse(concat!(
            "[lint.rules.undesirable-function]\n",
            "extend-functions = { sapply = \"use `vapply()`\" }\n",
        ))
        .expect("parse");
        let resolved = config.lint.rules.undesirable_function.resolved();
        assert_eq!(
            resolved.get("sapply").map(String::as_str),
            Some("use `vapply()`")
        );
        assert!(
            resolved.contains_key("attach"),
            "`extend-functions` must keep the defaults: {resolved:?}"
        );
    }

    #[test]
    fn undesirable_function_extend_overrides_a_default_entry() {
        let config = parse(concat!(
            "[lint.rules.undesirable-function]\n",
            "extend-functions = { attach = \"custom advice\" }\n",
        ))
        .expect("parse");
        let resolved = config.lint.rules.undesirable_function.resolved();
        assert_eq!(
            resolved.get("attach").map(String::as_str),
            Some("custom advice")
        );
    }

    #[test]
    fn undesirable_function_empty_functions_table_disables_the_rule() {
        let config = parse(concat!(
            "[lint.rules.undesirable-function]\n",
            "functions = {}\n",
        ))
        .expect("parse");
        assert!(config.lint.rules.undesirable_function.resolved().is_empty());
    }

    #[test]
    fn rejects_unknown_rule_id_table() {
        // Unlike `select`/`ignore` (validated at lint time), an unknown rule ID
        // under `[lint.rules]` is a typed field and so a parse error.
        let err = parse("[lint.rules.undesirabl-function]\nfunctions = {}\n")
            .expect_err("unknown rule table");
        match err {
            ConfigError::Parse { message, .. } => {
                assert!(message.contains("undesirabl-function"), "got: {message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_field_in_undesirable_function() {
        let err =
            parse("[lint.rules.undesirable-function]\nfunction = {}\n").expect_err("unknown field");
        match err {
            ConfigError::Parse { message, .. } => {
                assert!(message.contains("function"), "got: {message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_snake_case_in_undesirable_function() {
        let err = parse("[lint.rules.undesirable-function]\nextend_functions = {}\n")
            .expect_err("keys are kebab-case");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn undesirable_function_lookup_agrees_with_resolved() {
        // `lookup` is the allocation-free hot path; it must not drift from the
        // materialized map that documents the semantics.
        let config = parse(concat!(
            "[lint.rules.undesirable-function]\n",
            "functions = { attach = \"a\", sapply = \"b\" }\n",
            "extend-functions = { attach = \"override\", setwd = \"c\" }\n",
        ))
        .expect("parse")
        .lint
        .rules
        .undesirable_function;

        let resolved = config.resolved();
        for name in ["attach", "sapply", "setwd", "absent"] {
            assert_eq!(
                config.lookup(name),
                resolved.get(name).map(String::as_str),
                "lookup/resolved disagree on {name:?}"
            );
        }
        // And the override actually took effect, in both.
        assert_eq!(config.lookup("attach"), Some("override"));
    }

    #[test]
    fn default_undesirable_functions_excludes_rules_with_their_own_id() {
        // `browser` has a dedicated rule; including it here would double-report.
        let defaults = default_undesirable_functions();
        assert!(!defaults.contains_key("browser"), "{defaults:?}");
        assert!(defaults.contains_key("attach"), "{defaults:?}");
    }

    #[test]
    fn parse_error_reports_file_path_and_line() {
        let path = Path::new("/tmp/oops.toml");
        let err = Config::parse_str("[format]\nbogus = 1\n", path).expect_err("bad field");
        let rendered = err.to_string();
        assert!(rendered.starts_with("/tmp/oops.toml:"));
    }

    #[test]
    fn load_from_missing_file_returns_io_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.toml");
        let err = Config::load_from(&path).expect_err("missing file");
        assert!(matches!(err, ConfigError::Io { .. }));
    }

    #[test]
    fn discover_finds_arity_toml_in_parent() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[format]\nline-width = 70\n",
        )
        .unwrap();
        let nested = dir.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();

        let (path, config) = Config::discover(&nested).expect("discover").expect("found");
        assert_eq!(
            path,
            dir.path().canonicalize().unwrap().join(CONFIG_FILE_NAME)
        );
        assert_eq!(config.format.line_width, 70);
    }

    #[test]
    fn discover_stops_at_git_boundary() {
        let dir = tempdir().unwrap();
        // Ancestor sets a config we must NOT pick up.
        fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[format]\nline-width = 70\n",
        )
        .unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let nested = repo.join("src");
        fs::create_dir_all(&nested).unwrap();

        let found = Config::discover(&nested).expect("discover");
        assert!(
            found.is_none(),
            "should stop at .git boundary, got {found:?}"
        );
    }

    #[test]
    fn discover_prefers_config_at_repo_root() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(CONFIG_FILE_NAME), "[format]\nline-width = 70\n").unwrap();
        let nested = repo.join("src");
        fs::create_dir_all(&nested).unwrap();

        let (path, config) = Config::discover(&nested).expect("discover").expect("found");
        assert_eq!(path, repo.canonicalize().unwrap().join(CONFIG_FILE_NAME));
        assert_eq!(config.format.line_width, 70);
    }

    /// A missing anchor is not an error: the LSP anchors discovery on the
    /// directory of the buffer being edited, which need not exist on disk (an
    /// unsaved buffer, a directory deleted while open, or simply `C:\tmp`).
    /// Discovery still walks the ancestors that *do* exist.
    #[test]
    fn discover_tolerates_a_missing_anchor_directory() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[format]\nline-width = 70\n",
        )
        .unwrap();
        let missing = dir.path().join("no").join("such").join("dir");
        assert!(!missing.exists(), "fixture directory must not exist");

        let (path, config) = Config::discover(&missing)
            .expect("discovery must not fail on a missing anchor")
            .expect("an existing ancestor still supplies the config");
        assert_eq!(
            path,
            dir.path().canonicalize().unwrap().join(CONFIG_FILE_NAME)
        );
        assert_eq!(config.format.line_width, 70);
    }

    /// The same, with nothing to find: a missing anchor under no config at all
    /// reports "no config" rather than an IO error.
    #[test]
    fn discover_on_a_missing_anchor_without_config_returns_none() {
        let dir = tempdir().unwrap();
        // A `.git` at the temp root bounds the walk so an arity.toml in a real
        // ancestor of the temp dir can't leak into the assertion.
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        let missing = dir.path().join("no").join("such").join("dir");

        assert!(
            Config::discover(&missing)
                .expect("discovery must not fail on a missing anchor")
                .is_none()
        );
    }

    #[test]
    fn resolve_no_config_returns_defaults() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[format]\nline-width = 20\n",
        )
        .unwrap();
        let (config, source) = Config::resolve(None, true, dir.path()).expect("resolve");
        assert_eq!(config, Config::default());
        assert!(source.is_none());
    }

    #[test]
    fn resolve_explicit_overrides_discovery() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[format]\nline-width = 20\n",
        )
        .unwrap();
        let explicit = dir.path().join("custom.toml");
        fs::write(&explicit, "[format]\nline-width = 40\n").unwrap();

        let (config, source) =
            Config::resolve(Some(&explicit), false, dir.path()).expect("resolve");
        assert_eq!(config.format.line_width, 40);
        assert_eq!(source.as_deref(), Some(explicit.as_path()));
    }

    #[test]
    fn resolve_discovers_when_no_explicit_and_not_disabled() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            "[format]\nline-width = 50\n",
        )
        .unwrap();
        let (config, source) = Config::resolve(None, false, dir.path()).expect("resolve");
        assert_eq!(config.format.line_width, 50);
        assert!(source.is_some());
    }
}
