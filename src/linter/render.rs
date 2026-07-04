//! Diagnostic rendering: pretty, concise, json output modes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use annotate_snippets::{AnnotationKind, Level, Renderer, Snippet};

use crate::text::LineIndex;

use super::diagnostic::{Diagnostic, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    #[default]
    Pretty,
    Concise,
    Json,
}

/// Render a collection of diagnostics in the requested mode.
///
/// `source_for` returns the file source for a given path. The renderer needs
/// it for `Pretty` mode (to draw the `-->` annotations) and falls back to
/// best-effort behavior if a file is missing.
pub fn render_findings(
    diagnostics: &[Diagnostic],
    mode: OutputMode,
    use_color: bool,
    source_for: &dyn Fn(&PathBuf) -> Option<String>,
) -> String {
    match mode {
        OutputMode::Json => render_json(diagnostics),
        OutputMode::Concise => render_concise(diagnostics, source_for),
        OutputMode::Pretty => render_pretty(diagnostics, use_color, source_for),
    }
}

fn render_json(diagnostics: &[Diagnostic]) -> String {
    let mut sorted: Vec<&Diagnostic> = diagnostics.iter().collect();
    sorted.sort_by_key(|d| (&d.path, d.range.start(), d.range.end()));
    serde_json::to_string_pretty(&sorted).unwrap_or_else(|_| "[]".to_string())
}

fn render_concise(
    diagnostics: &[Diagnostic],
    source_for: &dyn Fn(&PathBuf) -> Option<String>,
) -> String {
    // Group diagnostics by file so we can reuse the LineIndex.
    let mut by_path: BTreeMap<&PathBuf, Vec<&Diagnostic>> = BTreeMap::new();
    for d in diagnostics {
        by_path.entry(&d.path).or_default().push(d);
    }
    let mut out = String::new();
    for (path, mut diags) in by_path {
        diags.sort_by_key(|d| (d.range.start(), d.range.end()));
        let source = source_for(path);
        let line_index = source.as_deref().map(LineIndex::new);
        for d in diags {
            let (line, col) = match &line_index {
                Some(idx) => {
                    let lc = idx.byte_to_lc(u32::from(d.range.start()) as usize);
                    (lc.line, lc.column)
                }
                None => (1, 1),
            };
            let _ = writeln!(
                out,
                "{}:{line}:{col}: {} [{}] {}",
                path.display(),
                severity_word(d.severity),
                d.rule,
                d.message.body
            );
        }
    }
    out
}

fn render_pretty(
    diagnostics: &[Diagnostic],
    use_color: bool,
    source_for: &dyn Fn(&PathBuf) -> Option<String>,
) -> String {
    let renderer = if use_color {
        Renderer::styled()
    } else {
        Renderer::plain()
    };
    let mut by_path: BTreeMap<&PathBuf, Vec<&Diagnostic>> = BTreeMap::new();
    for d in diagnostics {
        by_path.entry(&d.path).or_default().push(d);
    }
    let mut out = String::new();
    for (path, mut diags) in by_path {
        diags.sort_by_key(|d| (d.range.start(), d.range.end()));
        let Some(source) = source_for(path) else {
            // Fall back to concise output for this file.
            for d in &diags {
                let _ = writeln!(
                    out,
                    "{}: {} [{}] {}",
                    path.display(),
                    severity_word(d.severity),
                    d.rule,
                    d.message.body
                );
            }
            continue;
        };
        let origin = path.display().to_string();
        for d in &diags {
            let level = severity_level(d.severity);
            let start = u32::from(d.range.start()) as usize;
            let end = u32::from(d.range.end()) as usize;
            let snippet = Snippet::source(&source).path(&origin).annotation(
                AnnotationKind::Primary
                    .span(start..end)
                    .label(&d.message.body),
            );
            let group = level.primary_title(d.rule).element(snippet);
            let rendered = renderer.render(&[group]);
            let _ = writeln!(out, "{rendered}");
            if let Some(s) = &d.message.suggestion {
                let _ = writeln!(out, "  = help: {s}");
            }
        }
    }
    out
}

fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    }
}

fn severity_level(s: Severity) -> Level<'static> {
    match s {
        Severity::Error => Level::ERROR,
        Severity::Warning => Level::WARNING,
        Severity::Info => Level::INFO,
        Severity::Hint => Level::HELP,
    }
}
