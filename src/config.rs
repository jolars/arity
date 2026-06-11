//! `arity.toml` configuration: schema, file loading, and ancestor-walk discovery.
//!
//! The CLI is the only consumer; the library API (`format_with_style`,
//! `check_paths_with_style`, ...) continues to take a fully-resolved
//! [`FormatStyle`].

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::formatter::FormatStyle;

pub const CONFIG_FILE_NAME: &str = "arity.toml";

const MIN_WIDTH: u32 = 1;
const MAX_WIDTH: u32 = 1000;

const DEFAULT_LINE_WIDTH: u32 = 80;
const DEFAULT_INDENT_WIDTH: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    #[serde(default)]
    pub format: FormatConfig,
    #[serde(default)]
    pub lint: LintConfig,
    #[serde(default)]
    pub index: IndexConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct FormatConfig {
    #[serde(default = "default_line_width")]
    pub line_width: u32,
    #[serde(default = "default_indent_width")]
    pub indent_width: u32,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            line_width: DEFAULT_LINE_WIDTH,
            indent_width: DEFAULT_INDENT_WIDTH,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
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
}

/// `[index]` — the R-introspection sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            library_paths: Vec::new(),
            cache_dir: None,
            auto_build: true,
            help: true,
        }
    }
}

fn default_true() -> bool {
    true
}

impl From<&FormatConfig> for FormatStyle {
    fn from(config: &FormatConfig) -> Self {
        FormatStyle {
            line_width: config.line_width as usize,
            indent_width: config.indent_width as usize,
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
        let config: Self = toml::from_str(text).map_err(|err| {
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
        Ok(config)
    }

    fn validate(&self, path: Option<&Path>) -> Result<(), ConfigError> {
        self.format.validate(path)
    }

    /// Walk `start` and its ancestors looking for a `arity.toml`. Stops at the
    /// first match or at a directory that contains a `.git` entry (repo root),
    /// whichever comes first. Returns `None` if neither is found before the
    /// filesystem root.
    pub fn discover(start: &Path) -> Result<Option<(PathBuf, Self)>, ConfigError> {
        let canonical = start.canonicalize().map_err(|source| ConfigError::Io {
            path: start.to_path_buf(),
            source,
        })?;
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
    fn default_config_matches_format_style_default() {
        let config = Config::default();
        let style = FormatStyle::from(&config.format);
        assert_eq!(style, FormatStyle::default());
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
    fn parses_lint_ignore() {
        let config = parse("[lint]\nignore = [\"undefined-symbol\"]\n").expect("parse");
        assert_eq!(config.lint.ignore, vec!["undefined-symbol".to_string()]);
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
