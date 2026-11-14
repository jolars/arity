use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use arity::cli::{Cli, ColorChoice, Commands, LintOutput};
use arity::config::{
    Config, ConfigError, ConfigSource, DirectoryConfigs, LintConfig, ResolvedConfig,
};
use arity::file_discovery::ExcludeFilter;
use arity::formatter::{
    FormatCache, FormatStyle, Formatted, check_paths_with_style_cached,
    format_description_with_style, format_file, format_file_verified, format_verified_with_options,
    format_with_options,
};
use arity::incremental::IncrementalDatabase;
use arity::linter::{
    OutputMode, apply_fixes, check_document, check_document_in_project, render_findings_shared,
};
use arity::parser::ParseOptions;
use arity::parser::parse;
use arity::rindex::build::{BuildOptions, PackageOutcome, build_index};
use arity::rindex::cache::{Cache, resolve_cache_root};
use arity::rindex::discover::{referenced_packages, with_default_packages};
use arity::rindex::libpaths::LibrarySearch;
use arity::rindex::provider::{CompositeProvider, IndexedProvider};
use clap::Parser;
use std::io::IsTerminal;

/// Parsing, formatting, and the parallel lint passes are allocation-heavy;
/// glibc malloc serializes badly under rayon (30%+ of profile samples in
/// malloc/free on a project lint). mimalloc's per-thread heaps remove that
/// contention.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Autofix selection for `lint --fix`.
#[derive(Debug, Clone, Copy)]
struct FixOptions {
    fix: bool,
    unsafe_fixes: bool,
}

/// Cap on fixpoint iterations per file, guarding against a fix that fails to
/// clear its own diagnostic.
const MAX_FIX_ITERATIONS: usize = 10;

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Extraction recovery stays silent unless the user requests diagnostics.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("off"))
        .target(env_logger::Target::Stderr)
        .try_init();
    let config_source = ConfigOptions {
        explicit: cli.config.clone(),
        no_config: cli.no_config,
    };
    let out = OutputOptions {
        quiet: cli.quiet,
        verbose: cli.verbose,
        color: cli.color,
    };

    match cli.command {
        Commands::Parse {
            file,
            stdin_filename,
            quiet,
            verify,
        } => run_parse(file, stdin_filename, quiet, verify),
        Commands::Format {
            paths,
            stdin_filename,
            verify,
            check,
            line_width,
            indent_width,
            exclude,
            force_exclude,
            no_cache,
        } => run_format(
            paths,
            stdin_filename,
            FormatModes {
                verify,
                check,
                no_cache,
            },
            FormatOverrides {
                line_width,
                indent_width,
            },
            ExcludeOptions {
                patterns: exclude,
                force: force_exclude,
            },
            &config_source,
            out,
        ),
        Commands::Lint {
            paths,
            stdin_filename,
            fix,
            unsafe_fixes,
            select,
            extend_select,
            ignore,
            exclude,
            force_exclude,
            output,
        } => run_lint(
            LintInvocation {
                paths,
                stdin_filename,
                fix: FixOptions { fix, unsafe_fixes },
                overrides: LintOverrides {
                    select,
                    extend_select,
                    ignore,
                },
                excludes: ExcludeOptions {
                    patterns: exclude,
                    force: force_exclude,
                },
                output,
            },
            &config_source,
            out,
        ),
        Commands::Index {
            paths,
            force,
            no_help,
            attach_probe,
            cache_dir,
            quiet,
        } => run_index(
            paths,
            IndexCliOptions {
                force,
                no_help,
                attach_probe,
                cache_dir,
                quiet,
            },
            &config_source,
        ),
        Commands::Lsp => run_lsp(),
        Commands::Completions { shell } => run_completions(shell),
        Commands::Init { force } => run_init(force, out),
    }
}

/// Cross-cutting output preferences from the global flags.
#[derive(Debug, Clone, Copy)]
struct OutputOptions {
    quiet: bool,
    verbose: bool,
    color: ColorChoice,
}

/// Whether to emit ANSI color, given the `--color` choice and whether the target
/// stream is a terminal. `auto` also honors the `NO_COLOR` convention.
fn color_enabled(choice: ColorChoice, is_terminal: bool) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => std::env::var_os("NO_COLOR").is_none() && is_terminal,
    }
}

struct IndexCliOptions {
    force: bool,
    no_help: bool,
    attach_probe: bool,
    cache_dir: Option<PathBuf>,
    quiet: bool,
}

fn run_index(
    paths: Vec<PathBuf>,
    opts: IndexCliOptions,
    config_source: &ConfigOptions,
) -> ExitCode {
    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let (config, resolved_source) = match load_config_with_source(config_source, &anchor) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    let scan_paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };

    // Honor `exclude`/`extend-exclude` so `arity index` never harvests packages
    // referenced only from generated or vendored sources the user opted out of.
    let exclude = match build_exclude_filter(&config, &resolved_source, &anchor, &[]) {
        Ok(exclude) => exclude,
        Err(code) => return code,
    };

    // Always index R's default packages (base, stats, …) on top of the project's
    // explicit dependencies, so hover and signatures resolve for base-R symbols.
    let packages = match referenced_packages(&scan_paths, &exclude) {
        Ok(pkgs) => with_default_packages(pkgs),
        Err(err) => {
            eprintln!("error: {}", arity::linter::LintError::from(err));
            return ExitCode::from(2);
        }
    };

    let cache_root =
        match resolve_cache_root(opts.cache_dir.as_deref(), config.index.cache_dir.as_deref()) {
            Ok(root) => root,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(2);
            }
        };
    let cache = Cache::new(cache_root);
    let search = LibrarySearch::discover(Some(&anchor), &config.index.library_paths);

    let report = build_index(
        &packages,
        &cache,
        &search,
        BuildOptions {
            help: config.index.help && !opts.no_help,
            force: opts.force,
            // Consent is the flag or the env var, never a committed setting.
            attach_probe: opts.attach_probe || arity::rindex::attach_probe::enabled_by_env(),
        },
        now_unix_secs(),
    );

    let mut any_missing = false;
    for (pkg, outcome) in &report.packages {
        match outcome {
            PackageOutcome::Indexed { version, symbols } => {
                if !opts.quiet {
                    eprintln!("indexed {pkg}@{version} ({symbols} symbols)");
                }
            }
            PackageOutcome::UpToDate { version } => {
                if !opts.quiet {
                    eprintln!("up to date {pkg}@{version}");
                }
            }
            PackageOutcome::NotInstalled => {
                any_missing = true;
                eprintln!("warning: {pkg} is not installed in any known library");
            }
            PackageOutcome::Failed { reason } => {
                any_missing = true;
                eprintln!("warning: failed to index {pkg}: {reason}");
            }
        }
    }

    // A missing/failed package is a warning, not a hard error: you can index a
    // project before all its dependencies are installed.
    let _ = any_missing;
    ExitCode::SUCCESS
}

/// Build the harvested package index for linting, loading it from the cache
/// when one is configured/available. Installed into salsa as the
/// HIGH-durability library index; R's default + bundled CRAN lists are static.
/// Lint resolution only asks export-membership questions, and only about the
/// packages a file attaches, so this is the lazy load: `meta.json` up front and
/// a package file only once something asks about that package.
fn lint_index(config: &arity::config::Config) -> IndexedProvider {
    let Ok(cache_root) = resolve_cache_root(None, config.index.cache_dir.as_deref()) else {
        return IndexedProvider::empty();
    };
    let cache = Cache::new(cache_root);
    IndexedProvider::from_cache_lazy(&cache)
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn run_lsp() -> ExitCode {
    match arity::lsp::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: language server exited: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_completions(shell: clap_complete::Shell) -> ExitCode {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, bin_name, &mut io::stdout());
    ExitCode::SUCCESS
}

/// A commented starter `arity.toml` showing every key at its default.
const STARTER_CONFIG: &str = "\
# arity configuration. All keys are optional; values shown are the defaults.
# See https://arity.cc for the full reference.

# Load another config first; this file overrides its settings.
# extend = \"../shared/arity.toml\"

# Gitignore-style patterns to skip; applies to both `format` and `lint`.
# `exclude` replaces the built-in default set (shown below); use
# `extend-exclude` to add patterns while keeping the defaults.
# exclude = [\".git/\", \"renv/\", \"revdep/\", \"cpp11.R\", \"RcppExports.R\", \"extendr-wrappers.R\", \"import-standalone-*.R\"]
# extend-exclude = []

[format]
# line-width = 80
# indent-width = 2
# line-ending = \"auto\"  # auto | lf | crlf | native
# roxygen = true
# description = true

[lint]
# select = [\"...\"]  # replaces the default rule selection
# extend-select = [] # adds rules to the selection
# ignore = []        # rules to disable

# A few rules take options of their own, in a table named after the rule ID.
# `functions` replaces the rule's built-in set; `extend-functions` adds to it.
# [lint.rules.undesirable-function]
# extend-functions = { sapply = \"use `vapply()` for a stable return type\" }

# Minimum supported tool versions (plain strings; the key is the `>=` floor).
# When unset, derived from the package DESCRIPTION where possible.
# [compat]
# r = \"4.1\"
# roxygen2 = \"7.3.2\"
";

fn run_init(force: bool, out: OutputOptions) -> ExitCode {
    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let path = anchor.join(arity::config::CONFIG_FILE_NAME);
    if path.exists() && !force {
        eprintln!(
            "error: {} already exists; pass --force to overwrite",
            path.display()
        );
        return ExitCode::from(2);
    }
    match fs::write(&path, STARTER_CONFIG) {
        Ok(()) => {
            if !out.quiet {
                println!("Wrote {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: failed to write {}: {err}", path.display());
            ExitCode::from(2)
        }
    }
}

struct ConfigOptions {
    explicit: Option<PathBuf>,
    no_config: bool,
}

struct FormatOverrides {
    line_width: Option<u32>,
    indent_width: Option<u32>,
}

/// The boolean mode flags of the `format` command, bundled to keep
/// [`run_format`]'s arity in check.
#[derive(Debug, Clone, Copy)]
struct FormatModes {
    verify: bool,
    check: bool,
    no_cache: bool,
}

struct LintOverrides {
    select: Vec<String>,
    extend_select: Vec<String>,
    ignore: Vec<String>,
}

/// Resolve the config and its source so exclude patterns use the right anchor.
fn load_config_with_source(
    source: &ConfigOptions,
    anchor: &Path,
) -> Result<(Config, ConfigSource), ConfigError> {
    Config::resolve(source.explicit.as_deref(), source.no_config, anchor)
}

/// The exclusion-related CLI flags: extra `--exclude` patterns and whether
/// `--force-exclude` applies them to explicitly named files too.
struct ExcludeOptions {
    patterns: Vec<String>,
    force: bool,
}

/// Build the file-discovery exclude filter from the resolved config plus any
/// `--exclude` CLI patterns. Patterns resolve relative to the directory holding
/// a project config (or `anchor` for the user fallback and defaults).
fn build_exclude_filter(
    config: &Config,
    config_source: &ConfigSource,
    anchor: &Path,
    cli_excludes: &[String],
) -> Result<ExcludeFilter, ExitCode> {
    config
        .exclude_filter(config_source, anchor, cli_excludes)
        .map_err(|err| {
            eprintln!("error: {err}");
            ExitCode::from(2)
        })
}

/// Apply `--line-width`/`--indent-width` overrides over a loaded config and
/// validate, yielding the resolved [`FormatStyle`].
fn format_style_with_overrides(
    config: &Config,
    overrides: &FormatOverrides,
) -> Result<FormatStyle, ConfigError> {
    let mut format = config.format.clone();
    if let Some(width) = overrides.line_width {
        format.line_width = width;
    }
    if let Some(width) = overrides.indent_width {
        format.indent_width = width;
    }
    format.validate(None)?;
    Ok(FormatStyle::from(&format))
}

/// Everything the `format` command resolves from one config load.
struct FormatSetup {
    style: FormatStyle,
    /// The `[format] description` key: whether a package `DESCRIPTION` is one of
    /// the files this run is about.
    descriptions: bool,
}

/// Resolve the stdin formatter style and grammar scope from the cwd config. Prints and returns
/// an exit code on error.
fn resolve_format_setup(
    source: &ConfigOptions,
    overrides: &FormatOverrides,
    anchor: &Path,
) -> Result<FormatSetup, ExitCode> {
    let (config, _) = load_config_with_source(source, anchor).map_err(|err| {
        eprintln!("error: {err}");
        ExitCode::from(2)
    })?;
    let style = format_style_with_overrides(&config, overrides).map_err(|err| {
        eprintln!("error: {err}");
        ExitCode::from(2)
    })?;
    Ok(FormatSetup {
        style,
        descriptions: config.format.description,
    })
}

/// Where a subcommand's positional arguments say to read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inputs<'a> {
    /// Read the one buffer on stdin.
    Stdin,
    /// Read the named files and directories (never empty).
    Paths(&'a [PathBuf]),
}

/// A positional-argument mistake, rendered as a clap usage error by
/// [`inputs_or_exit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputError {
    /// `-` was mixed with file paths; there is no sane order to read them in.
    StdinWithPaths,
    /// Nothing to read: no paths, and stdin is an interactive terminal.
    NoInput,
}

/// `-` is the conventional spelling for "the stdin buffer" (black, ruff), and
/// is never a real file we would format.
fn is_stdin_path(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Decide what the positional paths point at.
///
/// Bare `arity format` reads stdin, matching rustfmt/gofmt/clang-format. That
/// default is right behind a pipe and a trap at a prompt: a new user runs it
/// expecting the current directory to be formatted and instead sees the process
/// hang on a terminal it is quietly reading. So no paths *and* an interactive
/// stdin is a usage error instead.
///
/// The terminal check is the whole gate, and it is deliberately narrow: it can
/// only fire where a human is typing, never behind a pipe, a redirect, a
/// heredoc, or a CI runner, so no scripted invocation changes behavior. Taking
/// `stdin_is_terminal` as an argument (as [`color_enabled`] does) keeps the
/// decision testable without a pty.
fn resolve_inputs(paths: &[PathBuf], stdin_is_terminal: bool) -> Result<Inputs<'_>, InputError> {
    if paths.iter().any(|path| is_stdin_path(path)) {
        if paths.len() > 1 {
            return Err(InputError::StdinWithPaths);
        }
        return Ok(Inputs::Stdin);
    }
    if paths.is_empty() {
        if stdin_is_terminal {
            return Err(InputError::NoInput);
        }
        return Ok(Inputs::Stdin);
    }
    Ok(Inputs::Paths(paths))
}

/// Exit with a clap-rendered usage error for `subcommand` (its own `Usage:`
/// line, the `--help` pointer, and clap's exit code 2), so an argument mistake
/// we diagnose ourselves is spelled exactly like one clap caught.
fn usage_error(subcommand: &str, kind: clap::error::ErrorKind, message: &str) -> ! {
    let mut cli = <Cli as clap::CommandFactory>::command();
    let Some(sub) = cli.find_subcommand_mut(subcommand) else {
        <Cli as clap::CommandFactory>::command()
            .error(kind, message)
            .exit()
    };
    // Fetched off a freshly built `Command`, the subcommand has no bin name yet,
    // so name it or the usage line reads `Usage: format …`.
    sub.clone()
        .bin_name(format!("arity {subcommand}"))
        .error(kind, message)
        .exit()
}

/// The "nothing to read" usage errors, one per subcommand. Each names `-` so the
/// way out of the trap rides in the message that reports it.
const FORMAT_MISSING_INPUT: &str =
    "no input paths; pass files or directories to format, or `-` to read from stdin";
const LINT_MISSING_INPUT: &str =
    "no input paths; pass files or directories to lint, or `-` to read from stdin";
const PARSE_MISSING_INPUT: &str = "no input path; pass a file to parse, or `-` to read from stdin";

/// [`resolve_inputs`] against the real stdin, exiting on a usage mistake.
/// `missing` spells the "nothing to read" case in the subcommand's own terms.
fn inputs_or_exit<'a>(paths: &'a [PathBuf], subcommand: &str, missing: &str) -> Inputs<'a> {
    match resolve_inputs(paths, io::stdin().is_terminal()) {
        Ok(inputs) => inputs,
        Err(InputError::StdinWithPaths) => usage_error(
            subcommand,
            clap::error::ErrorKind::ArgumentConflict,
            "`-` reads from stdin and cannot be combined with other paths",
        ),
        Err(InputError::NoInput) => usage_error(
            subcommand,
            clap::error::ErrorKind::MissingRequiredArgument,
            missing,
        ),
    }
}

fn run_parse(
    file: Option<PathBuf>,
    stdin_filename: Option<PathBuf>,
    quiet: bool,
    verify: bool,
) -> ExitCode {
    // Clap caps `parse` at one positional, so the slice holds at most one path.
    let file = match inputs_or_exit(file.as_slice(), "parse", PARSE_MISSING_INPUT) {
        Inputs::Stdin => None,
        Inputs::Paths(paths) => Some(paths[0].as_path()),
    };
    let input = match read_input(file) {
        Ok(input) => input,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    if file
        .or(stdin_filename.as_deref())
        .is_some_and(arity::file_discovery::is_rd_file)
    {
        let output = arity::rd::parse(&input);
        let lossless = !verify || output.cst.text().to_string() == input;
        return report_parse(&output.cst, &output.diagnostics, quiet, lossless);
    }

    let output = parse(&input);
    let lossless = !verify || output.cst.text().to_string() == input;
    report_parse(&output.cst, &output.diagnostics, quiet, lossless)
}

fn report_parse(
    cst: &impl std::fmt::Debug,
    diagnostics: &[arity::parser::ParseDiagnostic],
    quiet: bool,
    lossless: bool,
) -> ExitCode {
    if !quiet {
        println!("{cst:#?}");
    }

    if !lossless {
        eprintln!("error: parser losslessness check failed");
    }
    if !diagnostics.is_empty() {
        for diag in diagnostics {
            eprintln!("error[{}..{}]: {}", diag.start, diag.end, diag.message);
        }
        return ExitCode::from(1);
    }

    if !lossless {
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

/// Whether the persistent format cache is compiled in. It is disabled in debug
/// builds: the on-disk cache key is the crate version, which is frozen across a
/// whole dev cycle, so a stale fixed-point hit would mask formatter behavior
/// changes while iterating. Release users get the cache, where formatting is
/// stable within a published version. `--no-cache`/`cache = false` still apply.
#[cfg(debug_assertions)]
const CACHE_SUPPORTED: bool = false;
#[cfg(not(debug_assertions))]
const CACHE_SUPPORTED: bool = true;

type FileConfigs = HashMap<PathBuf, Arc<ResolvedConfig>>;

/// Discover once, then retain the exact selected config for both processing
/// passes. The walker prunes exclusions before it ever loads child configs.
fn discover_configured(
    paths: &[PathBuf],
    source: &ConfigOptions,
    excludes: &ExcludeOptions,
    anchor: &Path,
) -> Result<
    (arity::file_discovery::DiscoveredFiles, FileConfigs),
    arity::file_discovery::FileDiscoveryError,
> {
    let resolver = Arc::new(Mutex::new(DirectoryConfigs::new(
        anchor.to_path_buf(),
        source.explicit.clone(),
        source.no_config,
        excludes.patterns.clone(),
        excludes.force,
    )));
    let walk_resolver = resolver.clone();
    let cwd = anchor.to_path_buf();
    let files = arity::file_discovery::collect_source_files_filtered(
        paths,
        move |path, is_dir, explicit| {
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            walk_resolver
                .lock()
                .unwrap()
                .includes(&absolute, is_dir, explicit)
        },
    )?;
    let mut configs = HashMap::new();
    let mut resolver = resolver.lock().unwrap();
    for path in files.r.iter().chain(&files.description) {
        let directory = path.parent().unwrap_or(anchor);
        let resolved = resolver.resolve(directory).map_err(|message| {
            arity::file_discovery::FileDiscoveryError::WalkError {
                path: directory.to_path_buf(),
                message,
            }
        })?;
        configs.insert(path.clone(), resolved);
    }
    Ok((files, configs))
}

#[allow(clippy::too_many_arguments)]
fn run_format_files(
    paths: &[PathBuf],
    modes: FormatModes,
    overrides: &FormatOverrides,
    excludes: &ExcludeOptions,
    source: &ConfigOptions,
    anchor: &Path,
    out: OutputOptions,
) -> ExitCode {
    if modes.check && modes.verify {
        eprintln!("error: --verify cannot be combined with --check");
        return ExitCode::from(2);
    }
    // Invalid flags remain errors even when exclusions leave no files.
    if let Err(err) = format_style_with_overrides(&Config::default(), overrides) {
        eprintln!("error: {err}");
        return ExitCode::from(2);
    }
    let (mut discovered, configs) = match discover_configured(paths, source, excludes, anchor) {
        Ok(files) => files,
        Err(arity::file_discovery::FileDiscoveryError::UnsupportedFilePath { path })
            if !modes.check =>
        {
            eprintln!(
                "error: input file {} is not formattable; format supports .R files and DESCRIPTION",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(err) => return run_format_check(Err(err.into()), out),
    };
    // Explicit DESCRIPTION requests retain the existing refusal when formatting
    // is disabled. Walked descriptions are simply outside the formatter scope.
    for path in &discovered.description {
        if !configs[path].config.format.description && paths.contains(path) {
            eprintln!(
                "error: input file {} is not formattable (DESCRIPTION formatting is off: `[format] description = false`)",
                path.display()
            );
            return ExitCode::from(2);
        }
    }
    discovered
        .description
        .retain(|path| configs[path].config.format.description);
    let files = arity::formatter::merge(discovered);
    if files.is_empty() {
        if excludes.force {
            return ExitCode::SUCCESS;
        }
        eprintln!("error: no .R files or DESCRIPTION files found under the provided input paths");
        return ExitCode::from(2);
    }
    let mut styles = HashMap::new();
    let mut groups: HashMap<usize, (Arc<ResolvedConfig>, Vec<PathBuf>)> = HashMap::new();
    for path in &files {
        let resolved = &configs[path];
        let style = match format_style_with_overrides(&resolved.config, overrides) {
            Ok(style) => style,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(2);
            }
        };
        styles.insert(path.clone(), style);
        groups
            .entry(Arc::as_ptr(resolved) as usize)
            .or_insert_with(|| (resolved.clone(), Vec::new()))
            .1
            .push(path.clone());
    }
    if !modes.check {
        return run_format_write_paths(&files, modes.verify, &styles, out);
    }
    let mut combined = arity::formatter::CheckResult {
        checked_files: 0,
        changed_files: Vec::new(),
        outdated_directives: Vec::new(),
        failed_files: Vec::new(),
        skipped: Vec::new(),
    };
    for (resolved, group) in groups.into_values() {
        let style = styles[&group[0]];
        // Each group loads the cache for its resolved style, including CLI
        // overrides, so identical bytes under different styles cannot collide.
        let mut cache = (resolved.config.cache && !modes.no_cache && CACHE_SUPPORTED)
            .then(|| resolve_cache_root(None, resolved.config.index.cache_dir.as_deref()).ok())
            .flatten()
            .map(|root| FormatCache::load(&root, &style));
        match check_paths_with_style_cached(
            &group,
            style,
            &ExcludeFilter::none(),
            true,
            cache.as_mut(),
        ) {
            Ok(result) => {
                combined.checked_files += result.checked_files;
                combined.changed_files.extend(result.changed_files);
                combined
                    .outdated_directives
                    .extend(result.outdated_directives);
                combined.failed_files.extend(result.failed_files);
                combined.skipped.extend(result.skipped);
            }
            Err(err) => return run_format_check(Err(err), out),
        }
    }
    combined.changed_files.sort_by(|a, b| a.path.cmp(&b.path));
    combined
        .outdated_directives
        .sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
    combined.failed_files.sort_by(|a, b| a.path.cmp(&b.path));
    combined.skipped.sort();
    run_format_check(Ok(combined), out)
}

fn run_format(
    paths: Vec<PathBuf>,
    stdin_filename: Option<PathBuf>,
    modes: FormatModes,
    overrides: FormatOverrides,
    excludes: ExcludeOptions,
    config_source: &ConfigOptions,
    out: OutputOptions,
) -> ExitCode {
    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    if !paths.is_empty() && !paths.iter().any(|path| is_stdin_path(path)) {
        return run_format_files(
            &paths,
            modes,
            &overrides,
            &excludes,
            config_source,
            &anchor,
            out,
        );
    }
    let setup = match resolve_format_setup(config_source, &overrides, &anchor) {
        Ok(setup) => setup,
        Err(code) => return code,
    };
    let FormatSetup {
        style,
        descriptions,
    } = setup;

    if modes.check {
        if modes.verify {
            eprintln!("error: --verify cannot be combined with --check");
            return ExitCode::from(2);
        }
        // `--check` reports on files it leaves on disk, so stdin is not an input
        // here; `run_format_check` rejects the empty path list on its own.
        if paths.iter().any(|path| is_stdin_path(path)) {
            usage_error(
                "format",
                clap::error::ErrorKind::ArgumentConflict,
                "`--check` reports on files and cannot read from stdin",
            );
        }
        return run_format_check(Err(arity::formatter::CheckError::MissingPaths), out);
    }

    inputs_or_exit(&paths, "format", FORMAT_MISSING_INPUT);
    let input = match read_input(None) {
        Ok(input) => input,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    // A buffer with no path is R unless `--stdin-filename` says otherwise.
    // Without that flag, `cat DESCRIPTION | arity format -` would reflow DCF
    // as R — the one door into the formatter with no path to classify on.
    if stdin_filename
        .as_deref()
        .is_some_and(arity::file_discovery::is_description_file)
    {
        // `[format] description = false` reaches stdin too. Falling through
        // would reflow DCF as R, which is the corruption the key exists to
        // prevent, and stdin is the shape an editor or pre-commit hook uses
        // — the integrations most likely to need the off switch.
        if !descriptions {
            print!("{input}");
            return ExitCode::SUCCESS;
        }
        return format_description_stdin(&input, style, modes.verify);
    }

    // Stdin carries no path; resolve the package-wide roxygen markdown
    // default from the working directory, the same anchor config
    // discovery uses.
    let options = ParseOptions::default().with_roxygen_markdown_default(
        arity::project::description::roxygen_markdown_default_for_dir(&anchor),
    );
    let formatted = match if modes.verify {
        format_verified_with_options(&input, style, &options).map_err(|err| err.to_string())
    } else {
        format_with_options(&input, style, &options).map_err(|err| err.to_string())
    } {
        Ok(formatted) => formatted,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(1);
        }
    };

    print!("{formatted}");
    ExitCode::SUCCESS
}

/// The stdin path for a buffer `--stdin-filename` identified as a
/// `DESCRIPTION`. A decline prints the buffer back unchanged: stdin formatting
/// is a filter, and a filter that swallows its input is worse than one that
/// passes it through.
fn format_description_stdin(input: &str, style: FormatStyle, verify: bool) -> ExitCode {
    let formatted = match format_description_with_style(input, style) {
        Ok(formatted) => formatted,
        // `err` already reads "left unformatted: <reason>".
        Err(err) if err.is_decline() => {
            eprintln!("warning: {err}");
            print!("{input}");
            return ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(1);
        }
    };

    if verify {
        match format_description_with_style(&formatted, style) {
            Ok(reformatted) if reformatted == formatted => {}
            Ok(_) => {
                eprintln!("error: formatter verification failed (non-idempotent output)");
                return ExitCode::from(1);
            }
            Err(err) => {
                eprintln!("error: formatted output failed verification: {err}");
                return ExitCode::from(1);
            }
        }
    }

    print!("{formatted}");
    ExitCode::SUCCESS
}

fn run_format_check(
    result: Result<arity::formatter::CheckResult, arity::formatter::CheckError>,
    out: OutputOptions,
) -> ExitCode {
    match result {
        Ok(result) => {
            report_unchecked(&result.failed_files, &result.skipped, out.quiet);
            if result.changed_files.is_empty() && result.outdated_directives.is_empty() {
                if out.verbose {
                    eprintln!("{} file(s) already formatted", result.checked_files);
                }
            } else if out.quiet {
                // `--check` writes nothing, so the diff is normally the only
                // account of what would change; `--quiet` trades it for the
                // file list plus a summary, for callers (a CI step over a
                // wholly unformatted project) that would drown in hunks.
                if !result.changed_files.is_empty() {
                    for file in &result.changed_files {
                        println!("would reformat {}", file.path.display());
                    }
                    println!(
                        "{} of {} file(s) would be reformatted",
                        result.changed_files.len(),
                        result.checked_files
                    );
                }
            } else {
                let use_color = color_enabled(out.color, io::stdout().is_terminal());
                for (idx, file) in result.changed_files.iter().enumerate() {
                    if idx > 0 {
                        println!();
                    }
                    file.write_diff(&mut io::stdout().lock(), use_color)
                        .expect("failed to write formatting diff");
                }
            }
            for directive in &result.outdated_directives {
                directive
                    .write_diagnostic(&mut io::stdout().lock())
                    .expect("failed to write format directive diagnostic");
            }
            // Reported *after* the diff, so a file that could not be checked
            // never costs the user the account of the files that could. A
            // failure outranks a mere reformat: the run has no verdict for it.
            if !result.failed_files.is_empty() {
                ExitCode::from(2)
            } else if result.changed_files.is_empty() && result.outdated_directives.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(2)
        }
    }
}

/// Report the files a `format` run could not check, so neither bucket is silent.
///
/// A failure is an error even under `--quiet` — it is why the run is about to
/// exit 2. A skip is a warning, matching how `arity lint` reports the same file.
fn report_unchecked(failed: &[arity::formatter::FailedFile], skipped: &[PathBuf], quiet: bool) {
    for failure in failed {
        eprintln!("error: {failure}");
    }
    if quiet {
        return;
    }
    for path in skipped {
        eprintln!(
            "warning: skipped {}: stream did not contain valid UTF-8",
            path.display()
        );
    }
}

fn run_format_write_paths(
    files: &[PathBuf],
    verify: bool,
    styles: &HashMap<PathBuf, FormatStyle>,
    out: OutputOptions,
) -> ExitCode {
    let total = files.len();
    let mut reformatted_count = 0usize;
    // A per-file problem is reported and the walk carries on. Returning here
    // would let one file decide whether the rest of the tree gets formatted at
    // all, and `merge` sorts by path, so a package's `DESCRIPTION` sorts before
    // its `R/` and would reliably preempt everything the user asked about.
    let mut failed = 0usize;
    let mut markdown = arity::project::description::MarkdownDefaultResolver::new();
    for path in files {
        let style = styles[path];
        let input = match fs::read_to_string(path) {
            Ok(input) => input,
            // Not UTF-8: skipped, the same answer `arity lint` gives.
            Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                if !out.quiet {
                    eprintln!(
                        "warning: skipped {}: stream did not contain valid UTF-8",
                        path.display()
                    );
                }
                continue;
            }
            Err(err) => {
                eprintln!("error: failed to read {}: {err}", path.display());
                failed += 1;
                continue;
            }
        };
        let formatted = match if verify {
            format_file_verified(path, &input, style, &mut markdown)
        } else {
            format_file(path, &input, style, &mut markdown)
        } {
            Ok(Formatted::Text(formatted)) => formatted,
            Ok(Formatted::Declined(reason)) => {
                if out.verbose {
                    eprintln!("Skipped {}: {reason}", path.display());
                }
                continue;
            }
            Err(err) => {
                eprintln!("error: failed to format {}: {err}", path.display());
                failed += 1;
                continue;
            }
        };
        if verify {
            continue;
        }
        if formatted != input {
            if let Err(err) = fs::write(path, formatted) {
                eprintln!("error: failed to write {}: {err}", path.display());
                failed += 1;
                continue;
            }
            reformatted_count += 1;
            if out.verbose {
                eprintln!("Formatted {}", path.display());
            }
        }
    }

    if out.verbose && !verify {
        eprintln!("{reformatted_count} of {total} file(s) reformatted");
    }

    if failed > 0 {
        eprintln!("error: {failed} of {total} file(s) could not be formatted");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// The `lint` command's options, as parsed from the CLI.
struct LintInvocation {
    paths: Vec<PathBuf>,
    stdin_filename: Option<PathBuf>,
    fix: FixOptions,
    overrides: LintOverrides,
    excludes: ExcludeOptions,
    output: LintOutput,
}

fn run_lint(
    invocation: LintInvocation,
    config_source: &ConfigOptions,
    out: OutputOptions,
) -> ExitCode {
    let LintInvocation {
        paths,
        stdin_filename,
        fix: fix_opts,
        overrides,
        excludes,
        output,
    } = invocation;
    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let paths = match inputs_or_exit(&paths, "lint", LINT_MISSING_INPUT) {
        Inputs::Stdin => {
            let (mut config, _) = match load_config_with_source(config_source, &anchor) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::from(2);
                }
            };
            apply_lint_overrides(&mut config.lint, &overrides);
            return run_lint_stdin(&config.lint, fix_opts, stdin_filename, output, out);
        }
        Inputs::Paths(paths) => paths,
    };
    let mut cli_config = LintConfig::default();
    apply_lint_overrides(&mut cli_config, &overrides);
    let (_, unknown) = arity::linter::rules::ResolvedRules::resolve(&cli_config);
    if let Some(rule) = unknown.into_iter().next() {
        eprintln!("error: {}", arity::linter::LintError::UnknownRule { rule });
        return ExitCode::from(2);
    }
    let (discovered, configs) = match discover_configured(paths, config_source, &excludes, &anchor)
    {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: {}", arity::linter::LintError::from(err));
            return ExitCode::from(2);
        }
    };
    if discovered.r.is_empty() && discovered.description.is_empty() && !excludes.force {
        eprintln!("error: {}", arity::linter::LintError::NoFiles);
        return ExitCode::from(2);
    }
    let mut lint_configs = HashMap::new();
    for resolved in configs.values() {
        let key = Arc::as_ptr(resolved) as usize;
        if lint_configs.contains_key(&key) {
            continue;
        }
        let mut config = resolved.config.lint.clone();
        apply_lint_overrides(&mut config, &overrides);
        let (_, unknown) = arity::linter::rules::ResolvedRules::resolve(&config);
        if let Some(rule) = unknown.first() {
            eprintln!(
                "error: {}",
                arity::linter::LintError::UnknownRule { rule: rule.clone() }
            );
            return ExitCode::from(2);
        }
        lint_configs.insert(key, config);
    }
    if fix_opts.fix
        && let Some(code) = apply_fixes_to_paths(
            &discovered.r,
            &configs,
            &lint_configs,
            fix_opts.unsafe_fixes,
            out,
        )
    {
        return code;
    }
    let mut indexes = HashMap::new();
    for resolved in configs.values() {
        indexes
            .entry(resolved.config.index.cache_dir.clone())
            .or_insert_with(|| Arc::new(lint_index(&resolved.config)));
    }
    match arity::linter::check::check_files_with_configs(
        discovered,
        &|path| &lint_configs[&(Arc::as_ptr(&configs[path]) as usize)],
        &|path| &indexes[&configs[path].config.index.cache_dir],
    ) {
        Ok(result) => {
            // Files that couldn't be decoded as UTF-8 are skipped rather than
            // aborting the run; warn about each so they aren't silently ignored.
            if !out.quiet {
                for path in &result.skipped {
                    eprintln!(
                        "warning: skipped {}: stream did not contain valid UTF-8",
                        path.display()
                    );
                }
            }

            // Both lint findings and parse-error diagnostics render the same way;
            // parse errors block the rules but are reported as `syntax-error`
            // findings rather than swallowed behind a bare count.
            let mut all_findings = Vec::new();
            for report in &result.reports {
                match report.status {
                    arity::linter::LintStatus::Clean => {}
                    arity::linter::LintStatus::Findings { .. }
                    | arity::linter::LintStatus::ParseDiagnostics { .. } => {
                        all_findings.extend(report.diagnostics.iter().cloned());
                    }
                }
            }

            if !all_findings.is_empty() {
                let source_for = |path: &PathBuf| {
                    result
                        .reports
                        .iter()
                        .find(|report| &report.path == path)
                        .and_then(|report| report.source.clone())
                };
                emit_findings(&all_findings, output, out.color, &source_for);
            } else if out.verbose {
                eprintln!("{} file(s) checked, no findings", result.reports.len());
            }

            if all_findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(2)
        }
    }
}

fn apply_lint_overrides(config: &mut LintConfig, overrides: &LintOverrides) {
    if !overrides.select.is_empty() {
        config.select = Some(overrides.select.clone());
    }
    config
        .extend_select
        .extend(overrides.extend_select.iter().cloned());
    if !overrides.ignore.is_empty() {
        config.ignore = overrides.ignore.clone();
    }
}

/// Apply fixes to the same discovered set used by the reporting pass.
fn apply_fixes_to_paths(
    files: &[PathBuf],
    configs: &FileConfigs,
    lint_configs: &HashMap<usize, LintConfig>,
    include_unsafe: bool,
    out: OutputOptions,
) -> Option<ExitCode> {
    let mut db = IncrementalDatabase::default();
    for path in files {
        let provider = CompositeProvider::with_index(lint_index(&configs[path].config));
        match fix_file(
            &mut db,
            &provider,
            path,
            &lint_configs[&(Arc::as_ptr(&configs[path]) as usize)],
            include_unsafe,
        ) {
            Ok(0) => {}
            Ok(n) => {
                if !out.quiet {
                    eprintln!("{}: {n} fix{} applied", path.display(), plural(n));
                }
            }
            Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                // Non-UTF-8 source: skip-and-warn, matching the lint pass, rather
                // than aborting the whole fix run on one undecodable file.
                if !out.quiet {
                    eprintln!(
                        "warning: skipped {}: stream did not contain valid UTF-8",
                        path.display()
                    );
                }
            }
            Err(err) => {
                eprintln!("error: failed to fix {}: {err}", path.display());
                return Some(ExitCode::from(2));
            }
        }
    }
    None
}

/// Run the fixpoint loop on a single file and write it back if anything changed.
/// Returns the number of individual fixes applied.
fn fix_file(
    db: &mut IncrementalDatabase,
    provider: &CompositeProvider,
    path: &Path,
    config: &LintConfig,
    include_unsafe: bool,
) -> io::Result<usize> {
    let content = fs::read_to_string(path)?;
    let (fixed, total) =
        fix_source_in_project(db, provider, path, &content, config, include_unsafe);
    if total > 0 {
        fs::write(path, &fixed)?;
    }
    Ok(total)
}

/// [`fix_source`] with cross-file resolution: the active file's text is the
/// in-memory buffer while its siblings are seeded from disk, so project-aware
/// rules judge the same way they do under `arity lint`.
fn fix_source_in_project(
    db: &mut IncrementalDatabase,
    provider: &CompositeProvider,
    path: &Path,
    content: &str,
    config: &LintConfig,
    include_unsafe: bool,
) -> (String, usize) {
    let mut content = content.to_string();
    let mut total = 0usize;
    for _ in 0..MAX_FIX_ITERATIONS {
        let active = db.upsert_file(path, content.clone());
        let Ok(diagnostics) = check_document_in_project(db, path, active, config, provider) else {
            break;
        };
        let fixes: Vec<_> = diagnostics.into_iter().filter_map(|d| d.fix).collect();
        if fixes.is_empty() {
            break;
        }
        let outcome = apply_fixes(&content, &fixes, include_unsafe);
        if outcome.applied == 0 {
            break;
        }
        total += outcome.applied;
        content = outcome.output;
    }
    // Leave the tracked buffer agreeing with what we are about to write.
    db.upsert_file(path, content.clone());
    (content, total)
}

/// Apply safe (and optionally unsafe) autofixes to `content` to a fixpoint,
/// returning the rewritten source and the number of fixes applied. `path` is a
/// label only — no disk access.
fn fix_source(
    path: &Path,
    content: &str,
    config: &LintConfig,
    include_unsafe: bool,
) -> (String, usize) {
    let mut content = content.to_string();
    let mut total = 0usize;
    for _ in 0..MAX_FIX_ITERATIONS {
        let Ok(diagnostics) = check_document(path, &content, config) else {
            break;
        };
        let fixes: Vec<_> = diagnostics.into_iter().filter_map(|d| d.fix).collect();
        if fixes.is_empty() {
            break;
        }
        let outcome = apply_fixes(&content, &fixes, include_unsafe);
        if outcome.applied == 0 {
            break;
        }
        total += outcome.applied;
        content = outcome.output;
    }
    (content, total)
}

/// Map the CLI output choice to the renderer's [`OutputMode`] and emit the
/// findings: JSON goes to stdout (machine-readable), human formats to stderr.
fn emit_findings(
    findings: &[arity::linter::Diagnostic],
    output: LintOutput,
    color: ColorChoice,
    source_for: &dyn Fn(&PathBuf) -> Option<Arc<str>>,
) {
    let mode = match output {
        LintOutput::Pretty => OutputMode::Pretty,
        LintOutput::Concise => OutputMode::Concise,
        LintOutput::Json => OutputMode::Json,
    };
    // Human output goes to stderr; JSON to stdout.
    let use_color = color_enabled(color, io::stderr().is_terminal());
    let rendered = render_findings_shared(findings, mode, use_color, source_for);
    if matches!(mode, OutputMode::Json) {
        println!("{rendered}");
    } else {
        eprint!("{rendered}");
    }
}

/// Lint a single document read from stdin. With `--fix`, the fixed source is
/// written to stdout (mirroring `format`'s stdin behavior) and remaining
/// findings are reported; otherwise findings are reported and the source is not
/// echoed. Returns exit 1 when findings remain.
fn run_lint_stdin(
    config: &LintConfig,
    fix_opts: FixOptions,
    stdin_filename: Option<PathBuf>,
    output: LintOutput,
    out: OutputOptions,
) -> ExitCode {
    let input = match read_input(None) {
        Ok(input) => input,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    let path = stdin_filename.unwrap_or_else(|| PathBuf::from("-"));

    let content = if fix_opts.fix {
        let (fixed, _) = fix_source(&path, &input, config, fix_opts.unsafe_fixes);
        print!("{fixed}");
        fixed
    } else {
        input
    };

    let findings = match check_document(&path, &content, config) {
        Ok(findings) => findings,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    if findings.is_empty() {
        return ExitCode::SUCCESS;
    }
    let content = Arc::<str>::from(content);
    let source_for = |p: &PathBuf| (p == &path).then(|| content.clone());
    emit_findings(&findings, output, out.color, &source_for);
    ExitCode::from(1)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "es" }
}

fn cwd_anchor() -> Result<PathBuf, ExitCode> {
    std::env::current_dir().map_err(|err| {
        eprintln!("error: failed to determine current directory: {err}");
        ExitCode::from(2)
    })
}

fn read_input(path: Option<&Path>) -> io::Result<String> {
    match path {
        Some(path) => fs::read_to_string(path),
        None => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;
            Ok(input)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(entries: &[&str]) -> Vec<PathBuf> {
        entries.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn no_paths_reads_stdin_unless_it_is_a_terminal() {
        // Behind a pipe, a redirect, or a CI runner, bare `arity format` keeps
        // reading stdin exactly as rustfmt/gofmt do — the gate cannot reach any
        // scripted invocation.
        assert_eq!(resolve_inputs(&[], false), Ok(Inputs::Stdin));
        // At a prompt there is nothing to read, so it is a usage error instead of
        // a silent wait on the terminal.
        assert_eq!(resolve_inputs(&[], true), Err(InputError::NoInput));
    }

    #[test]
    fn dash_names_stdin_even_at_a_terminal() {
        let dash = paths(&["-"]);
        assert_eq!(resolve_inputs(&dash, true), Ok(Inputs::Stdin));
        assert_eq!(resolve_inputs(&dash, false), Ok(Inputs::Stdin));
    }

    #[test]
    fn dash_cannot_be_mixed_with_file_paths() {
        assert_eq!(
            resolve_inputs(&paths(&["-", "a.R"]), false),
            Err(InputError::StdinWithPaths)
        );
        assert_eq!(
            resolve_inputs(&paths(&["a.R", "-"]), false),
            Err(InputError::StdinWithPaths)
        );
    }

    #[test]
    fn named_paths_are_passed_through() {
        let named = paths(&["a.R", "R"]);
        assert_eq!(resolve_inputs(&named, true), Ok(Inputs::Paths(&named)));
    }
}
