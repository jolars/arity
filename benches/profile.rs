//! The profiler's target: one phase of arity's work, run in a loop, in one
//! process.
//!
//! This exists so `perf` samples the *library* rather than process startup,
//! dynamic linking, and config discovery. Sampling `arity format` directly on a
//! single file measures mostly exec overhead; sampling this binary for a few
//! hundred iterations measures the phase named on the command line.
//!
//! It is not a benchmark and prints no comparison: the wall-clock authority is
//! `hyperfine` on the real binary (`scripts/bench.sh`, `task bench`). The
//! per-iteration time printed here is a sanity check that the profile covers
//! the work it claims to, not a number to quote.
//!
//! ```sh
//! ./scripts/profile.sh                          # format, synthetic corpus
//! ./scripts/profile.sh --mode lint --path pkg/R/foo.R
//! ```
//!
//! Run standalone (no perf) with:
//!
//! ```sh
//! cargo bench --bench profile -- --mode parse --iterations 500
//! ```
//!
//! Modes:
//!
//! - `parse` — `parse()` alone, the cost every other phase pays first.
//! - `format` — `format_with_options()`: parse + lower + print.
//! - `format-warm` — `format_node()` on an already-parsed CST, which is what
//!   the language server's cached path actually runs. Use this one for LSP
//!   latency questions; `format` for `arity format` on a cold file.
//! - `lint` — `check_document()`: a one-shot database, semantic model, and the
//!   whole rule set over a single file.
//! - `format-dir` / `lint-dir` — the CLI drivers over a directory, so the
//!   rayon fan-out and file discovery are in the profile too. Needs `--path`.
//!
//! `--path` takes a file (all single-file modes) or a directory (the `-dir`
//! modes). Without it the input is the same synthetic corpus `scripts/bench.sh`
//! builds from the formatter fixtures, written to a temp file so the
//! path-taking modes have something real to read.

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use arity::config::LintConfig;
use arity::formatter::{FormatStyle, format_node, format_with_options};
use arity::linter;
use arity::parser::{ParseOptions, parse};

/// The shipping binary sets this too (`src/main.rs`). Allocator traffic is a
/// large share of every phase here, so profiling against the system allocator
/// would profile a program arity does not ship — measured at ~38% of a format
/// under glibc's malloc against ~10% under mimalloc.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str = "\
usage: profile [--mode MODE] [--path PATH] [--iterations N] [--repeat N]

  --mode        parse | format | format-warm | lint | format-dir | lint-dir
                (default: format)
  --path        input file, or directory for the -dir modes
                (default: the formatter fixtures, concatenated)
  --iterations  loop count (default: 300; ignored by the -dir modes, which
                default to 10)
  --repeat      repetitions of the synthetic corpus, for a larger input
                (default: 1; ignored when --path is given)
";

/// Every formatter fixture's `expected.R`, in sorted order — the same
/// deterministic base block `scripts/bench.sh` and `benches/format_edits.rs`
/// use, so a profile and the published numbers cover the same bytes.
fn corpus() -> String {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/arity-formatter/tests/fixtures/formatter");
    let mut files: Vec<PathBuf> = fs::read_dir(&root)
        .expect("formatter fixtures")
        .filter_map(|entry| {
            let path = entry.expect("fixture dir entry").path().join("expected.R");
            path.is_file().then_some(path)
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| fs::read_to_string(path).expect("fixture"))
        .collect()
}

struct Args {
    mode: String,
    path: Option<PathBuf>,
    iterations: Option<usize>,
    repeat: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        mode: "format".to_string(),
        path: None,
        iterations: None,
        repeat: 1,
    };
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let mut value = || argv.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--mode" => args.mode = value()?,
            "--path" => args.path = Some(PathBuf::from(value()?)),
            "--iterations" => {
                args.iterations = Some(value()?.parse().map_err(|_| "bad --iterations")?);
            }
            "--repeat" => args.repeat = value()?.parse().map_err(|_| "bad --repeat")?,
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            // `cargo bench` passes the harness a --bench flag; ignore it.
            "--bench" => {}
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let dir_mode = args.mode.ends_with("-dir");
    let iterations = args.iterations.unwrap_or(if dir_mode { 10 } else { 300 });

    // The synthetic corpus is written to disk rather than kept in memory: the
    // path-taking modes need a real file, and every mode should see the same
    // bytes whichever way it reads them.
    let scratch = tempfile::tempdir().expect("temp dir");
    let (path, text) = match &args.path {
        Some(path) => {
            let text = if dir_mode {
                String::new()
            } else {
                fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
            };
            (path.clone(), text)
        }
        None => {
            let text = corpus().repeat(args.repeat);
            let path = scratch.path().join("profile.R");
            fs::write(&path, &text).expect("write corpus");
            (path, text)
        }
    };

    let style = FormatStyle::default();
    let options = ParseOptions::default();
    let lint_config = LintConfig::default();

    let bytes = if dir_mode { 0 } else { text.len() };
    eprintln!(
        "profiling `{}` over {} x{iterations}",
        args.mode,
        if dir_mode {
            path.display().to_string()
        } else {
            format!("{} ({} KB)", path.display(), bytes / 1024)
        }
    );

    // Parsed once, outside the timed loop: `format-warm` is the language
    // server's path, where the CST comes back from salsa already built.
    let warm_root = (args.mode == "format-warm").then(|| parse(&text).cst);

    let start = Instant::now();
    for _ in 0..iterations {
        match args.mode.as_str() {
            "parse" => {
                black_box(parse(black_box(&text)));
            }
            "format" => {
                black_box(format_with_options(black_box(&text), style, &options))
                    .expect("input formats");
            }
            "format-warm" => {
                let root = warm_root.as_ref().expect("parsed once");
                black_box(format_node(black_box(root), style, &text)).expect("input formats");
            }
            "lint" => {
                black_box(linter::check_document(
                    black_box(&path),
                    black_box(&text),
                    &lint_config,
                ))
                .expect("lints");
            }
            "format-dir" => {
                if let Err(error) =
                    black_box(arity::formatter::check_paths(std::slice::from_ref(&path)))
                {
                    eprintln!(
                        "error: {} has nothing to format ({error:?})",
                        path.display()
                    );
                    return ExitCode::FAILURE;
                }
            }
            "lint-dir" => {
                if let Err(error) = black_box(linter::check_paths(std::slice::from_ref(&path))) {
                    eprintln!("error: {} has nothing to lint ({error:?})", path.display());
                    return ExitCode::FAILURE;
                }
            }
            other => {
                eprintln!("error: unknown mode: {other}\n\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    report(start, iterations, bytes);
    ExitCode::SUCCESS
}

/// A sanity line, not a benchmark result: enough to tell whether the profile
/// covered the work, and whether the iteration count is worth the samples.
fn report(start: Instant, iterations: usize, bytes: usize) {
    let elapsed = start.elapsed();
    let per_iter = elapsed.as_secs_f64() / iterations as f64;
    if bytes > 0 {
        eprintln!(
            "{:.2} ms/iter, {:.1} MB/s, {:.2} s total",
            per_iter * 1e3,
            bytes as f64 / per_iter / 1e6,
            elapsed.as_secs_f64(),
        );
    } else {
        eprintln!(
            "{:.2} ms/iter, {:.2} s total",
            per_iter * 1e3,
            elapsed.as_secs_f64(),
        );
    }
}
