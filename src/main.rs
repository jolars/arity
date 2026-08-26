use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use arity::cli::{Cli, ColorChoice, Commands, LintOutput};
use arity::config::{Config, ConfigError, LintConfig};
use arity::file_discovery::{ExcludeFilter, collect_r_files};
use arity::formatter::{
    FormatCache, FormatStyle, Formatted, check_paths_with_style_cached,
    format_description_with_style, format_file, format_with_options,
};
use arity::incremental::IncrementalDatabase;
use arity::linter::{
    OutputMode, apply_fixes, check_document, check_document_in_project, render_findings_shared,
};
use arity::parser::ParseOptions;
use arity::parser::{parse, reconstruct};
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
    let config_source = ConfigSource {
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
            quiet,
            verify,
        } => run_parse(file, quiet, verify),
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
            ignore,
            exclude,
            force_exclude,
            output,
        } => run_lint(
            LintInvocation {
                paths,
                stdin_filename,
                fix: FixOptions { fix, unsafe_fixes },
                overrides: LintOverrides { select, ignore },
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

fn run_index(paths: Vec<PathBuf>, opts: IndexCliOptions, config_source: &ConfigSource) -> ExitCode {
    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let (config, config_path) = match load_config_with_source(config_source, &anchor) {
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
    let exclude = match build_exclude_filter(&config, config_path.as_deref(), &anchor, &[]) {
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

# Gitignore-style patterns to skip; applies to both `format` and `lint`.
# `exclude` replaces the built-in default set (shown below); use
# `extend-exclude` to add patterns while keeping the defaults.
# exclude = [\".git/\", \"renv/\", \"revdep/\", \"cpp11.R\", \"RcppExports.R\", \"extendr-wrappers.R\", \"import-standalone-*.R\"]
# extend-exclude = []

[format]
# line-width = 80
# indent-width = 2
# line-ending = \"auto\"  # auto | lf | crlf | native

[lint]
# select = [\"...\"]  # if set, only these rules run
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

struct ConfigSource {
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
    ignore: Vec<String>,
}

/// Resolve the config and return it alongside the loaded file's path (if any),
/// needed to root exclude patterns relative to the directory containing
/// `arity.toml`.
fn load_config_with_source(
    source: &ConfigSource,
    anchor: &Path,
) -> Result<(Config, Option<PathBuf>), ConfigError> {
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
/// `arity.toml` (or `anchor` when there is no config file).
fn build_exclude_filter(
    config: &Config,
    config_path: Option<&Path>,
    anchor: &Path,
    cli_excludes: &[String],
) -> Result<ExcludeFilter, ExitCode> {
    config
        .exclude_filter(config_path, anchor, cli_excludes)
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

/// The persistent-cache settings resolved from config for the `format` command.
/// `enabled` reflects the config `cache` key (before the `--no-cache` override);
/// `dir` is the optional `[index] cache-dir` override.
struct FormatCacheSetup {
    enabled: bool,
    dir: Option<PathBuf>,
}

/// Everything the `format` command resolves from one config load.
struct FormatSetup {
    style: FormatStyle,
    exclude: ExcludeFilter,
    cache: FormatCacheSetup,
    /// The `[format] description` key: whether a package `DESCRIPTION` is one of
    /// the files this run is about.
    descriptions: bool,
}

/// Resolve the formatter style, exclude filter, cache settings, and grammar
/// scope for the `format` command from a single config load. Prints and returns
/// an exit code on error.
fn resolve_format_setup(
    source: &ConfigSource,
    overrides: &FormatOverrides,
    cli_excludes: &[String],
    anchor: &Path,
) -> Result<FormatSetup, ExitCode> {
    let (config, config_path) = load_config_with_source(source, anchor).map_err(|err| {
        eprintln!("error: {err}");
        ExitCode::from(2)
    })?;
    let style = format_style_with_overrides(&config, overrides).map_err(|err| {
        eprintln!("error: {err}");
        ExitCode::from(2)
    })?;
    let exclude = build_exclude_filter(&config, config_path.as_deref(), anchor, cli_excludes)?;
    let cache = FormatCacheSetup {
        enabled: config.cache,
        dir: config.index.cache_dir.clone(),
    };
    Ok(FormatSetup {
        style,
        exclude,
        cache,
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

fn run_parse(file: Option<PathBuf>, quiet: bool, verify: bool) -> ExitCode {
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

    let parse_output = parse(&input);

    if !quiet {
        println!("{:#?}", parse_output.cst);
    }

    if !parse_output.diagnostics.is_empty() {
        for diag in &parse_output.diagnostics {
            eprintln!("error[{}..{}]: {}", diag.start, diag.end, diag.message);
        }
        return ExitCode::from(1);
    }

    if verify {
        let reconstructed = reconstruct(&input);
        if reconstructed != input {
            eprintln!("error: parser losslessness check failed");
            return ExitCode::from(1);
        }
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

fn run_format(
    paths: Vec<PathBuf>,
    stdin_filename: Option<PathBuf>,
    modes: FormatModes,
    overrides: FormatOverrides,
    excludes: ExcludeOptions,
    config_source: &ConfigSource,
    out: OutputOptions,
) -> ExitCode {
    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let setup = match resolve_format_setup(config_source, &overrides, &excludes.patterns, &anchor) {
        Ok(setup) => setup,
        Err(code) => return code,
    };
    let FormatSetup {
        style,
        exclude,
        cache: cache_setup,
        descriptions,
    } = setup;
    let exclude = exclude.with_force_exclude(excludes.force);

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
        let cache_enabled = cache_setup.enabled && !modes.no_cache && CACHE_SUPPORTED;
        return run_format_check(
            &paths,
            style,
            &exclude,
            descriptions,
            cache_enabled,
            cache_setup.dir,
            out,
        );
    }

    let paths = match inputs_or_exit(&paths, "format", FORMAT_MISSING_INPUT) {
        Inputs::Stdin => None,
        Inputs::Paths(paths) => Some(paths),
    };
    let Some(paths) = paths else {
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
        let formatted = match format_with_options(&input, style, &options) {
            Ok(formatted) => formatted,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(1);
            }
        };

        if modes.verify {
            let reformatted = match format_with_options(&formatted, style, &options) {
                Ok(reformatted) => reformatted,
                Err(err) => {
                    eprintln!("error: formatted output failed verification: {err}");
                    return ExitCode::from(1);
                }
            };
            if reformatted != formatted {
                eprintln!("error: formatter verification failed (non-idempotent output)");
                return ExitCode::from(1);
            }
        }

        print!("{formatted}");
        return ExitCode::SUCCESS;
    };

    run_format_write_paths(paths, modes.verify, style, &exclude, descriptions, out)
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

#[allow(clippy::too_many_arguments)]
fn run_format_check(
    paths: &[PathBuf],
    style: FormatStyle,
    exclude: &ExcludeFilter,
    descriptions: bool,
    cache_enabled: bool,
    cache_dir: Option<PathBuf>,
    out: OutputOptions,
) -> ExitCode {
    // Load the persistent already-formatted cache when enabled and a cache root
    // is resolvable; an unresolvable root just skips caching (never an error).
    let mut cache = cache_enabled
        .then(|| resolve_cache_root(None, cache_dir.as_deref()).ok())
        .flatten()
        .map(|root| FormatCache::load(&root, &style));

    match check_paths_with_style_cached(paths, style, exclude, descriptions, cache.as_mut()) {
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
    paths: &[PathBuf],
    verify: bool,
    style: FormatStyle,
    exclude: &ExcludeFilter,
    descriptions: bool,
    out: OutputOptions,
) -> ExitCode {
    let discovered = if descriptions {
        arity::file_discovery::collect_source_files(paths, exclude)
    } else {
        arity::file_discovery::collect_r_files(paths, exclude).map(|r| {
            arity::file_discovery::DiscoveredFiles {
                r,
                description: Vec::new(),
            }
        })
    };
    let files = match discovered {
        Ok(files) => arity::formatter::merge(files),
        Err(arity::file_discovery::FileDiscoveryError::UnsupportedFilePath { path }) => {
            eprint!(
                "error: input file {} is not formattable; format supports .R files and DESCRIPTION",
                path.display()
            );
            if !descriptions && arity::file_discovery::is_description_file(&path) {
                eprint!(" (DESCRIPTION formatting is off: `[format] description = false`)");
            }
            eprintln!();
            return ExitCode::from(2);
        }
        Err(arity::file_discovery::FileDiscoveryError::WalkError { path, message }) => {
            eprintln!("error: failed while scanning {}: {message}", path.display());
            return ExitCode::from(2);
        }
    };
    if files.is_empty() {
        // Under --force-exclude every named file may be excluded; that is an
        // expected clean no-op, not a usage error.
        if exclude.force() {
            return ExitCode::SUCCESS;
        }
        eprintln!("error: no .R files or DESCRIPTION files found under the provided input paths");
        return ExitCode::from(2);
    }

    let total = files.len();
    let mut reformatted_count = 0usize;
    // A per-file problem is reported and the walk carries on. Returning here
    // would let one file decide whether the rest of the tree gets formatted at
    // all, and `merge` sorts by path, so a package's `DESCRIPTION` sorts before
    // its `R/` and would reliably preempt everything the user asked about.
    let mut failed = 0usize;
    let mut markdown = arity::project::description::MarkdownDefaultResolver::new();
    for path in files {
        let input = match fs::read_to_string(&path) {
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
        let formatted = match format_file(&path, &input, style, &mut markdown) {
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
            let reformatted = match format_file(&path, &formatted, style, &mut markdown) {
                Ok(Formatted::Text(reformatted)) => reformatted,
                Ok(Formatted::Declined(reason)) => {
                    eprintln!(
                        "error: formatted output of {} would now be declined: {reason}",
                        path.display()
                    );
                    failed += 1;
                    continue;
                }
                Err(err) => {
                    eprintln!(
                        "error: formatted output failed verification for {}: {err}",
                        path.display()
                    );
                    failed += 1;
                    continue;
                }
            };
            if reformatted != formatted {
                eprintln!(
                    "error: formatter verification failed for {} (non-idempotent output)",
                    path.display()
                );
                failed += 1;
            }
            continue;
        }
        if formatted != input {
            if let Err(err) = fs::write(&path, formatted) {
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
    config_source: &ConfigSource,
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
    let (mut config, config_path) = match load_config_with_source(config_source, &anchor) {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    // CLI flags override the configured rule selection when provided.
    if !overrides.select.is_empty() {
        config.lint.select = Some(overrides.select);
    }
    if !overrides.ignore.is_empty() {
        config.lint.ignore = overrides.ignore;
    }

    // No paths (or `-`): lint a single document read from stdin.
    let paths = match inputs_or_exit(&paths, "lint", LINT_MISSING_INPUT) {
        Inputs::Stdin => {
            return run_lint_stdin(&config.lint, fix_opts, stdin_filename, output, out);
        }
        Inputs::Paths(paths) => paths,
    };

    let exclude =
        match build_exclude_filter(&config, config_path.as_deref(), &anchor, &excludes.patterns) {
            Ok(exclude) => exclude.with_force_exclude(excludes.force),
            Err(code) => return code,
        };

    if fix_opts.fix
        && let Some(code) =
            apply_fixes_to_paths(paths, &config, fix_opts.unsafe_fixes, &exclude, out)
    {
        return code;
    }

    let index = lint_index(&config);
    match arity::linter::check_paths_with_index(paths, &config.lint, &exclude, index) {
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

/// Discover `.R` files under `paths` and apply autofixes in place. Returns
/// `Some(exit_code)` only on a hard error (discovery / IO); on success returns
/// `None` so the caller falls through to the normal reporting pass.
///
/// Deliberately `collect_r_files`, not `collect_source_files`: no `DESCRIPTION`
/// rule ships a fix, and reading one here would only invite feeding it to the R
/// parser. The day a DCF fix lands, this has to widen.
fn apply_fixes_to_paths(
    paths: &[PathBuf],
    config: &Config,
    include_unsafe: bool,
    exclude: &ExcludeFilter,
    out: OutputOptions,
) -> Option<ExitCode> {
    let files = match collect_r_files(paths, exclude) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: {}", arity::linter::LintError::from(err));
            return Some(ExitCode::from(2));
        }
    };
    // The fix loop must see the same cross-file scope the reporting pass does.
    // On the single-file path a NAMESPACE export or a sibling's call is
    // invisible, so every top-level binding reads as unused and `--unsafe-fixes`
    // deletes the package.
    let provider = CompositeProvider::with_index(lint_index(config));
    let mut db = IncrementalDatabase::default();
    for path in files {
        match fix_file(&mut db, &provider, &path, &config.lint, include_unsafe) {
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
