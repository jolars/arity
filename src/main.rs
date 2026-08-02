use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use arity::cli::{Cli, ColorChoice, Commands, LintOutput};
use arity::config::{Config, ConfigError, LintConfig};
use arity::file_discovery::{ExcludeFilter, collect_r_files};
use arity::formatter::{
    ChangedFile, FormatCache, FormatStyle, check_paths_with_style_cached, format_with_style,
};
use arity::linter::{OutputMode, apply_fixes, check_document, render_findings};
use arity::parser::{parse, reconstruct};
use arity::rindex::build::{BuildOptions, PackageOutcome, build_index};
use arity::rindex::cache::{Cache, resolve_cache_root};
use arity::rindex::discover::{referenced_packages, with_default_packages};
use arity::rindex::libpaths::LibrarySearch;
use arity::rindex::provider::IndexedProvider;
use clap::Parser;
use similar::{ChangeTag, TextDiff};
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
            verify,
            check,
            line_width,
            indent_width,
            exclude,
            force_exclude,
            no_cache,
        } => run_format(
            paths,
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
            cache_dir,
            quiet,
        } => run_index(
            paths,
            IndexCliOptions {
                force,
                no_help,
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
/// Lint resolution only asks export-membership questions, so this is the lean
/// names-only load — the rich formals/help payload stays on disk.
fn lint_index(config: &arity::config::Config) -> IndexedProvider {
    let Ok(cache_root) = resolve_cache_root(None, config.index.cache_dir.as_deref()) else {
        return IndexedProvider::empty();
    };
    let cache = Cache::new(cache_root);
    IndexedProvider::from_cache_exports(&cache)
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

/// Resolve the formatter style, exclude filter, and cache settings for the
/// `format` command from a single config load. Prints and returns an exit code
/// on error.
fn resolve_format_setup(
    source: &ConfigSource,
    overrides: &FormatOverrides,
    cli_excludes: &[String],
    anchor: &Path,
) -> Result<(FormatStyle, ExcludeFilter, FormatCacheSetup), ExitCode> {
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
    Ok((style, exclude, cache))
}

fn run_parse(file: Option<PathBuf>, quiet: bool, verify: bool) -> ExitCode {
    let input = match read_input(file.as_deref()) {
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
    let (style, exclude, cache_setup) =
        match resolve_format_setup(config_source, &overrides, &excludes.patterns, &anchor) {
            Ok(setup) => setup,
            Err(code) => return code,
        };
    let exclude = exclude.with_force_exclude(excludes.force);

    if modes.check {
        if modes.verify {
            eprintln!("error: --verify cannot be combined with --check");
            return ExitCode::from(2);
        }
        let cache_enabled = cache_setup.enabled && !modes.no_cache && CACHE_SUPPORTED;
        return run_format_check(&paths, style, &exclude, cache_enabled, cache_setup.dir, out);
    }

    if paths.is_empty() {
        let input = match read_input(None) {
            Ok(input) => input,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(2);
            }
        };

        let formatted = match format_with_style(&input, style) {
            Ok(formatted) => formatted,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(1);
            }
        };

        if modes.verify {
            let reformatted = match format_with_style(&formatted, style) {
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
    }

    run_format_write_paths(&paths, modes.verify, style, &exclude, out)
}

fn run_format_check(
    paths: &[PathBuf],
    style: FormatStyle,
    exclude: &ExcludeFilter,
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

    match check_paths_with_style_cached(paths, style, exclude, cache.as_mut()) {
        Ok(result) => {
            if result.changed_files.is_empty() {
                if out.verbose {
                    eprintln!("{} file(s) already formatted", result.checked_files);
                }
                ExitCode::SUCCESS
            } else {
                let use_color = color_enabled(out.color, io::stdout().is_terminal());
                for (idx, file) in result.changed_files.iter().enumerate() {
                    if idx > 0 {
                        println!();
                    }
                    print_diff(file, use_color);
                }
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(2)
        }
    }
}

/// Print a unified-style, per-file diff of the formatting change (rustfmt-like:
/// a `Diff in <path>:<line>:` header followed by context-grouped hunks).
fn print_diff(file: &ChangedFile, use_color: bool) {
    const RED: &str = "\x1b[31m";
    const GREEN: &str = "\x1b[32m";
    const RESET: &str = "\x1b[0m";

    let diff = TextDiff::from_lines(&file.original, &file.formatted);
    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if idx > 0 {
            println!("---");
        }
        let start = group[0].old_range().start + 1;
        println!("Diff in {}:{}:", file.path.display(), start);
        for op in group {
            for change in diff.iter_changes(op) {
                let (sign, color) = match change.tag() {
                    ChangeTag::Delete => ("-", RED),
                    ChangeTag::Insert => ("+", GREEN),
                    ChangeTag::Equal => (" ", ""),
                };
                let value = change.value();
                let newline = value.ends_with('\n');
                let line = value.strip_suffix('\n').unwrap_or(value);
                if use_color && !color.is_empty() {
                    print!("{color}{sign}{line}{RESET}");
                } else {
                    print!("{sign}{line}");
                }
                if newline {
                    println!();
                }
            }
        }
    }
}

fn run_format_write_paths(
    paths: &[PathBuf],
    verify: bool,
    style: FormatStyle,
    exclude: &ExcludeFilter,
    out: OutputOptions,
) -> ExitCode {
    let files = match arity::file_discovery::collect_r_files(paths, exclude) {
        Ok(files) => files,
        Err(arity::file_discovery::FileDiscoveryError::NonRFilePath { path }) => {
            eprintln!(
                "error: input file {} is not an .R file; format only supports .R files",
                path.display()
            );
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
        eprintln!("error: no .R files found under the provided input paths");
        return ExitCode::from(2);
    }

    let total = files.len();
    let mut reformatted_count = 0usize;
    for path in files {
        let input = match fs::read_to_string(&path) {
            Ok(input) => input,
            Err(err) => {
                eprintln!("error: failed to read {}: {err}", path.display());
                return ExitCode::from(2);
            }
        };
        let formatted = match format_with_style(&input, style) {
            Ok(formatted) => formatted,
            Err(err) => {
                eprintln!("error: failed to format {}: {err}", path.display());
                return ExitCode::from(1);
            }
        };
        if verify {
            let reformatted = match format_with_style(&formatted, style) {
                Ok(reformatted) => reformatted,
                Err(err) => {
                    eprintln!(
                        "error: formatted output failed verification for {}: {err}",
                        path.display()
                    );
                    return ExitCode::from(1);
                }
            };
            if reformatted != formatted {
                eprintln!(
                    "error: formatter verification failed for {} (non-idempotent output)",
                    path.display()
                );
                return ExitCode::from(1);
            }
            continue;
        }
        if formatted != input {
            if let Err(err) = fs::write(&path, formatted) {
                eprintln!("error: failed to write {}: {err}", path.display());
                return ExitCode::from(2);
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

    // No paths: lint a single document read from stdin.
    if paths.is_empty() {
        return run_lint_stdin(&config.lint, fix_opts, stdin_filename, output, out);
    }

    let exclude =
        match build_exclude_filter(&config, config_path.as_deref(), &anchor, &excludes.patterns) {
            Ok(exclude) => exclude.with_force_exclude(excludes.force),
            Err(code) => return code,
        };

    // Apply fixes in place first; the reporting pass below then re-reads from
    // disk and shows whatever findings remain.
    if fix_opts.fix
        && let Some(code) =
            apply_fixes_to_paths(&paths, &config.lint, fix_opts.unsafe_fixes, &exclude, out)
    {
        return code;
    }

    let index = lint_index(&config);
    match arity::linter::check_paths_with_index(&paths, &config.lint, &exclude, index) {
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
                let source_for = |path: &PathBuf| fs::read_to_string(path).ok();
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
fn apply_fixes_to_paths(
    paths: &[PathBuf],
    config: &LintConfig,
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
    for path in files {
        match fix_file(&path, config, include_unsafe) {
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
fn fix_file(path: &Path, config: &LintConfig, include_unsafe: bool) -> io::Result<usize> {
    let content = fs::read_to_string(path)?;
    let (fixed, total) = fix_source(path, &content, config, include_unsafe);
    if total > 0 {
        fs::write(path, &fixed)?;
    }
    Ok(total)
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
    source_for: &dyn Fn(&PathBuf) -> Option<String>,
) {
    let mode = match output {
        LintOutput::Pretty => OutputMode::Pretty,
        LintOutput::Concise => OutputMode::Concise,
        LintOutput::Json => OutputMode::Json,
    };
    // Human output goes to stderr; JSON to stdout.
    let use_color = color_enabled(color, io::stderr().is_terminal());
    let rendered = render_findings(findings, mode, use_color, source_for);
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
