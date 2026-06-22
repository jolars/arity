//! Rendering rule reference pages from rule metadata.
//!
//! [`render_rule_doc`] is the single source of truth shared by the snapshot
//! test (`tests/rule_docs.rs`) and the docs generator (`examples/docgen.rs`),
//! so the pinned docs and the generated files can never diverge. It runs the
//! *real* linter on each example, so the rendered diagnostics and autofix
//! before/after always reflect current behavior.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::config::LintConfig;
use crate::linter::check::check_document;
use crate::linter::diagnostic::Fix;
use crate::linter::fix::apply_fixes;
use crate::linter::render::{OutputMode, render_findings};
use crate::linter::rules::Rule;

/// The synthetic path used when linting an example snippet. The same value must
/// key both `check_document` and the `render_findings` source lookup: the
/// linter rewrites every diagnostic's `path` to the one passed here, and the
/// pretty renderer silently degrades to a one-liner if the source can't be
/// found for that exact path.
fn example_path() -> PathBuf {
    PathBuf::from("example.R")
}

/// Render the markdown reference page for a single rule.
pub fn render_rule_doc(rule: &dyn Rule) -> String {
    let mut out = String::new();
    let id = rule.id();
    let _ = writeln!(out, "# `{id}`");

    let description = rule.description().trim();
    if !description.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{description}");
    }

    // Restrict to this rule so an example can't trip a different one.
    let config = LintConfig {
        select: Some(vec![id.to_string()]),
        ..Default::default()
    };

    for example in rule.examples() {
        let _ = writeln!(out);
        if !example.caption.is_empty() {
            let _ = writeln!(out, "{}", example.caption);
            let _ = writeln!(out);
        }
        fenced(&mut out, "r", example.source);

        let diagnostics =
            check_document(&example_path(), example.source, &config).unwrap_or_default();
        let source = example.source.to_string();
        let rendered = render_findings(&diagnostics, OutputMode::Pretty, false, &|path| {
            (path == &example_path()).then(|| source.clone())
        });
        let _ = writeln!(out);
        fenced(&mut out, "text", &rendered);

        let fixes: Vec<Fix> = diagnostics.iter().filter_map(|d| d.fix.clone()).collect();
        let after = apply_fixes(example.source, &fixes, false);
        if after.applied > 0 {
            let _ = writeln!(out);
            let _ = writeln!(out, "After applying the fix:");
            let _ = writeln!(out);
            fenced(&mut out, "r", &after.output);
        }
    }

    out
}

/// Write a fenced code block, normalizing the body to end with exactly one
/// newline so the closing fence always sits on its own line (idempotence).
fn fenced(out: &mut String, lang: &str, body: &str) {
    let _ = writeln!(out, "```{lang}");
    let _ = out.write_str(body);
    if !body.ends_with('\n') {
        let _ = out.write_str("\n");
    }
    let _ = writeln!(out, "```");
}
