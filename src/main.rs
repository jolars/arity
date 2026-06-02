use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use ravel::cli::{Cli, Commands, LintOutput};
use ravel::config::{Config, ConfigError};
use ravel::formatter::{FormatStyle, check_paths_with_style, format_with_style};
use ravel::linter::{OutputMode, render_findings};
use ravel::parser::{parse, reconstruct};

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
            output,
        } => run_lint(paths, check, output, &config_source),
        Commands::Lsp => run_lsp(),
    }
}

fn run_lsp() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: failed to start LSP runtime: {err}");
            return ExitCode::from(2);
        }
    };
    runtime.block_on(ravel::lsp::run());
    ExitCode::SUCCESS
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
                for path in result.changed_files {
                    eprintln!("would reformat: {}", path.display());
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

fn run_format_write_paths(paths: &[PathBuf], verify: bool, style: FormatStyle) -> ExitCode {
    let files = match ravel::file_discovery::collect_r_files(paths) {
        Ok(files) => files,
        Err(ravel::file_discovery::FileDiscoveryError::NonRFilePath { path }) => {
            eprintln!(
                "error: input file {} is not an .R file; format only supports .R files",
                path.display()
            );
            return ExitCode::from(2);
        }
        Err(ravel::file_discovery::FileDiscoveryError::WalkError { path, message }) => {
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

    match ravel::linter::check_paths_with_config(&paths, &config.lint) {
        Ok(result) => {
            let mut has_parse_blockers = false;
            let mut all_findings = Vec::new();
            for report in &result.reports {
                match report.status {
                    ravel::linter::LintStatus::Clean => {}
                    ravel::linter::LintStatus::Findings { .. } => {
                        all_findings.extend(report.diagnostics.iter().cloned());
                    }
                    ravel::linter::LintStatus::ParseDiagnostics { count } => {
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
