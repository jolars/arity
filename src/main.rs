use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use arity::cli::{Cli, Commands, LintOutput};
use arity::config::{Config, ConfigError, LintConfig};
use arity::file_discovery::collect_r_files;
use arity::formatter::{ChangedFile, FormatStyle, check_paths_with_style, format_with_style};
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
        } => run_format(
            paths,
            verify,
            check,
            FormatOverrides {
                line_width,
                indent_width,
            },
            &config_source,
        ),
        Commands::Lint {
            paths,
            check,
            fix,
            unsafe_fixes,
            output,
        } => run_lint(
            paths,
            check,
            FixOptions { fix, unsafe_fixes },
            output,
            &config_source,
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
    let config = match load_config(config_source, &anchor) {
        Ok(config) => config,
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

    // Always index R's default packages (base, stats, …) on top of the project's
    // explicit dependencies, so hover and signatures resolve for base-R symbols.
    let packages = match referenced_packages(&scan_paths) {
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
fn lint_index(config: &arity::config::Config) -> IndexedProvider {
    let Ok(cache_root) = resolve_cache_root(None, config.index.cache_dir.as_deref()) else {
        return IndexedProvider::empty();
    };
    let cache = Cache::new(cache_root);
    IndexedProvider::from_cache(&cache)
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

struct ConfigSource {
    explicit: Option<PathBuf>,
    no_config: bool,
}

struct FormatOverrides {
    line_width: Option<u32>,
    indent_width: Option<u32>,
}

fn load_config(source: &ConfigSource, anchor: &Path) -> Result<Config, ConfigError> {
    let (config, _path) = Config::resolve(source.explicit.as_deref(), source.no_config, anchor)?;
    Ok(config)
}

fn resolve_format_style(
    source: &ConfigSource,
    overrides: &FormatOverrides,
    anchor: &Path,
) -> Result<FormatStyle, ConfigError> {
    let mut config = load_config(source, anchor)?;
    if let Some(width) = overrides.line_width {
        config.format.line_width = width;
    }
    if let Some(width) = overrides.indent_width {
        config.format.indent_width = width;
    }
    config.format.validate(None)?;
    Ok(FormatStyle::from(&config.format))
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

fn run_format(
    paths: Vec<PathBuf>,
    verify: bool,
    check: bool,
    overrides: FormatOverrides,
    config_source: &ConfigSource,
) -> ExitCode {
    if check {
        if verify {
            eprintln!("error: --verify cannot be combined with --check");
            return ExitCode::from(2);
        }
        let anchor = match cwd_anchor() {
            Ok(anchor) => anchor,
            Err(code) => return code,
        };
        let style = match resolve_format_style(config_source, &overrides, &anchor) {
            Ok(style) => style,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(2);
            }
        };
        return run_format_check(&paths, style);
    }

    if paths.is_empty() {
        let anchor = match cwd_anchor() {
            Ok(anchor) => anchor,
            Err(code) => return code,
        };
        let style = match resolve_format_style(config_source, &overrides, &anchor) {
            Ok(style) => style,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::from(2);
            }
        };

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

        if verify {
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

    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let style = match resolve_format_style(config_source, &overrides, &anchor) {
        Ok(style) => style,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    run_format_write_paths(&paths, verify, style)
}

fn run_format_check(paths: &[PathBuf], style: FormatStyle) -> ExitCode {
    match check_paths_with_style(paths, style) {
        Ok(result) => {
            if result.changed_files.is_empty() {
                ExitCode::SUCCESS
            } else {
                let use_color = should_colorize();
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

/// Whether to emit ANSI color into the diff. Honors the `NO_COLOR` convention
/// and only colorizes when stdout is a terminal.
fn should_colorize() -> bool {
    std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
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

fn run_format_write_paths(paths: &[PathBuf], verify: bool, style: FormatStyle) -> ExitCode {
    let files = match arity::file_discovery::collect_r_files(paths) {
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
        eprintln!("error: no .R files found under the provided input paths");
        return ExitCode::from(2);
    }

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
        if formatted != input
            && let Err(err) = fs::write(&path, formatted)
        {
            eprintln!("error: failed to write {}: {err}", path.display());
            return ExitCode::from(2);
        }
    }

    ExitCode::SUCCESS
}

fn run_lint(
    paths: Vec<PathBuf>,
    check: bool,
    fix_opts: FixOptions,
    output: LintOutput,
    config_source: &ConfigSource,
) -> ExitCode {
    let anchor = match cwd_anchor() {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let config = match load_config(config_source, &anchor) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    // Apply fixes in place first; the reporting pass below then re-reads from
    // disk and shows whatever findings remain.
    if fix_opts.fix
        && let Some(code) = apply_fixes_to_paths(&paths, &config.lint, fix_opts.unsafe_fixes)
    {
        return code;
    }

    let index = lint_index(&config);
    match arity::linter::check_paths_with_index(&paths, &config.lint, index) {
        Ok(result) => {
            let mut has_parse_blockers = false;
            let mut all_findings = Vec::new();
            for report in &result.reports {
                match report.status {
                    arity::linter::LintStatus::Clean => {}
                    arity::linter::LintStatus::Findings { .. } => {
                        all_findings.extend(report.diagnostics.iter().cloned());
                    }
                    arity::linter::LintStatus::ParseDiagnostics { count } => {
                        has_parse_blockers = true;
                        eprintln!(
                            "lint blocked by parse diagnostics: {} ({} diagnostic{})",
                            report.path.display(),
                            count,
                            if count == 1 { "" } else { "s" }
                        );
                    }
                }
            }

            if !all_findings.is_empty() {
                let mode = match output {
                    LintOutput::Pretty => OutputMode::Pretty,
                    LintOutput::Concise => OutputMode::Concise,
                    LintOutput::Json => OutputMode::Json,
                };
                let source_for = |path: &PathBuf| fs::read_to_string(path).ok();
                let rendered = render_findings(&all_findings, mode, &source_for);
                if matches!(mode, OutputMode::Json) {
                    println!("{rendered}");
                } else {
                    eprint!("{rendered}");
                }
            }

            let _ = check;
            let has_findings = !all_findings.is_empty();
            if has_parse_blockers || has_findings {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
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
) -> Option<ExitCode> {
    let files = match collect_r_files(paths) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: {}", arity::linter::LintError::from(err));
            return Some(ExitCode::from(2));
        }
    };
    for path in files {
        match fix_file(&path, config, include_unsafe) {
            Ok(0) => {}
            Ok(n) => eprintln!("{}: {n} fix{} applied", path.display(), plural(n)),
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
    let mut content = fs::read_to_string(path)?;
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
    if total > 0 {
        fs::write(path, &content)?;
    }
    Ok(total)
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
