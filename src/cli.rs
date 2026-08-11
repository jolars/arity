use std::path::PathBuf;

use clap::builder::Styles;
use clap::builder::styling::{AnsiColor, Effects};
use clap::{Parser, Subcommand};

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default());

#[derive(Parser)]
#[command(name = "arity")]
#[command(author, version)]
#[command(about = "Arity: a language server, formatter, and linter for R")]
#[command(styles = STYLES)]
#[command(arg_required_else_help = true)]
pub struct Cli {
    /// Path to an explicit `arity.toml` (skips discovery)
    #[arg(long, value_name = "PATH", global = true, conflicts_with = "no_config")]
    pub config: Option<PathBuf>,

    /// Ignore any discovered `arity.toml` and use built-in defaults
    #[arg(long, global = true)]
    pub no_config: bool,

    /// When to use color in output
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, global = true, value_name = "WHEN")]
    pub color: ColorChoice,

    /// Suppress informational output (errors are still shown); under
    /// `format --check` this drops the per-file diff, leaving the list of files
    /// that would be reformatted and the summary
    #[arg(long, short = 'q', global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Print extra informational output (e.g. per-command summaries)
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// When to colorize output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ColorChoice {
    /// Colorize when writing to a terminal and `NO_COLOR` is unset (default).
    #[default]
    Auto,
    /// Always colorize.
    Always,
    /// Never colorize.
    Never,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Parse and display the CST tree for debugging
    Parse {
        /// Input file. Pass `-` for stdin, also read when the path is omitted
        /// and stdin is not a terminal
        file: Option<PathBuf>,

        /// Suppress CST output to stdout
        #[arg(long)]
        quiet: bool,

        /// Verify parser losslessness (input must equal CST text)
        #[arg(long)]
        verify: bool,
    },
    /// Format .R files
    Format {
        /// Input file(s) or director(ies). Pass `-` for stdin, also read when
        /// paths are omitted and stdin is not a terminal
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Verify formatting idempotence for supported inputs (does not write files)
        #[arg(long)]
        verify: bool,

        /// Check formatting without writing changes; prints a diff for each file
        /// that would be reformatted and exits non-zero if any differ. Requires
        /// path arguments: there is no file on disk to report on when reading
        /// stdin
        #[arg(long)]
        check: bool,

        /// Override the configured line width
        #[arg(long, value_name = "N")]
        line_width: Option<u32>,

        /// Override the configured indent width
        #[arg(long, value_name = "N")]
        indent_width: Option<u32>,

        /// Additional gitignore-style exclude patterns (repeatable or
        /// comma-separated); augments the configured `exclude`/`extend-exclude`
        #[arg(long, value_name = "PATTERN", value_delimiter = ',')]
        exclude: Vec<String>,

        /// Apply exclude patterns to files named explicitly on the command line
        /// too (they are normally always processed); for runners like
        /// pre-commit that pass staged files as arguments
        #[arg(long)]
        force_exclude: bool,

        /// Disable the persistent already-formatted cache (read and write) for
        /// this run; only affects `--check`
        #[arg(long)]
        no_cache: bool,
    },
    /// Lint .R files
    ///
    /// Reads stdin when given `-`, or when paths are omitted and stdin is not
    /// a terminal. Exit codes: 0 = no findings,
    /// 1 = findings (or files blocked by parse errors), 2 = usage/IO error.
    Lint {
        /// Input file(s) or director(ies). Pass `-` for stdin, also read when
        /// paths are omitted and stdin is not a terminal
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Filename to report for stdin input (for diagnostics)
        #[arg(long, value_name = "PATH")]
        stdin_filename: Option<PathBuf>,

        /// Apply safe autofixes in place and report what remains
        #[arg(long)]
        fix: bool,

        /// Also apply fixes that may change behavior (requires --fix)
        #[arg(long)]
        unsafe_fixes: bool,

        /// Only run these rules (overrides config `select`); repeatable or comma-separated
        #[arg(long, value_name = "RULE_ID", value_delimiter = ',')]
        select: Vec<String>,

        /// Disable these rules (overrides config `ignore`); repeatable or comma-separated
        #[arg(long, value_name = "RULE_ID", value_delimiter = ',')]
        ignore: Vec<String>,

        /// Additional gitignore-style exclude patterns (repeatable or
        /// comma-separated); augments the configured `exclude`/`extend-exclude`
        #[arg(long, value_name = "PATTERN", value_delimiter = ',')]
        exclude: Vec<String>,

        /// Apply exclude patterns to files named explicitly on the command line
        /// too (they are normally always processed); for runners like
        /// pre-commit that pass staged files as arguments
        #[arg(long)]
        force_exclude: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t = LintOutput::Pretty)]
        output: LintOutput,
    },
    /// Build or refresh the installed-package introspection index
    Index {
        /// Project path(s) to scan for referenced packages (default: ".")
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Re-harvest even when the installed version is already indexed
        #[arg(long)]
        force: bool,

        /// Skip harvesting help (names only; faster)
        #[arg(long)]
        no_help: bool,

        /// Probe what each meta-package attaches by running `library()` in a
        /// fresh R session (spawns R and executes package attach hooks; also
        /// enabled by setting `ARITY_ATTACH_PROBE`)
        #[arg(long)]
        attach_probe: bool,

        /// Override the cache directory
        #[arg(long, value_name = "DIR")]
        cache_dir: Option<PathBuf>,

        /// Suppress per-package progress output
        #[arg(long)]
        quiet: bool,
    },
    /// Run the language server over stdio
    Lsp,
    /// Generate a shell completion script (write it to stdout)
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Write a starter `arity.toml` to the current directory
    Init {
        /// Overwrite an existing `arity.toml`
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LintOutput {
    /// Annotated multi-line snippets (default; matches jarl/rustc-style output).
    Pretty,
    /// One finding per line (`path:line:col: severity [rule] message`).
    Concise,
    /// JSON array of diagnostics, for editor integration.
    Json,
}
