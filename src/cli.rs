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
#[command(name = "ravel")]
#[command(author, version)]
#[command(about = "Ravel: a language server, formatter, and linter for R")]
#[command(styles = STYLES)]
#[command(arg_required_else_help = true)]
pub struct Cli {
    /// Path to an explicit `ravel.toml` (skips discovery)
    #[arg(long, value_name = "PATH", global = true, conflicts_with = "no_config")]
    pub config: Option<PathBuf>,

    /// Ignore any discovered `ravel.toml` and use built-in defaults
    #[arg(long, global = true)]
    pub no_config: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Parse and display the CST tree for debugging
    Parse {
        /// Input file (stdin if not provided)
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
        /// Input file(s) or path(s) (stdin if omitted)
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Verify formatting idempotence for supported inputs (does not write files)
        #[arg(long)]
        verify: bool,

        /// Check formatting of .R files under the provided paths without writing changes
        #[arg(long)]
        check: bool,

        /// Override the configured line width
        #[arg(long, value_name = "N")]
        line_width: Option<u32>,

        /// Override the configured indent width
        #[arg(long, value_name = "N")]
        indent_width: Option<u32>,
    },
    /// Lint .R files
    Lint {
        /// Input file or path
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Exit non-zero when any findings are reported (no effect on output)
        #[arg(long)]
        check: bool,

        /// Apply safe autofixes in place and report what remains
        #[arg(long)]
        fix: bool,

        /// Also apply fixes that may change behavior (requires --fix)
        #[arg(long)]
        unsafe_fixes: bool,

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

        /// Override the cache directory
        #[arg(long, value_name = "DIR")]
        cache_dir: Option<PathBuf>,

        /// Suppress per-package progress output
        #[arg(long)]
        quiet: bool,
    },
    /// Run the language server over stdio (formatting only)
    Lsp,
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
