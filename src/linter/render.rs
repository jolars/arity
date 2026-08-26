//! Diagnostic rendering: pretty, concise, json output modes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

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
    render_findings_shared(diagnostics, mode, use_color, &|path| {
        source_for(path).map(Arc::from)
    })
}

/// Render diagnostics from shared source buffers without copying their text.
pub fn render_findings_shared(
    diagnostics: &[Diagnostic],
    mode: OutputMode,
    use_color: bool,
    source_for: &dyn Fn(&PathBuf) -> Option<Arc<str>>,
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
    source_for: &dyn Fn(&PathBuf) -> Option<Arc<str>>,
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
    source_for: &dyn Fn(&PathBuf) -> Option<Arc<str>>,
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
        let line_index = LineIndex::new(&source);
        for d in &diags {
            let level = severity_level(d.severity);
            let start = u32::from(d.range.start()) as usize;
            let end = u32::from(d.range.end()) as usize;
            // Hand annotate_snippets only the span's lines (plus one line of
            // padding each side, which its folding drops): it builds a source
            // map of whatever it's given, so passing the whole file would make
            // rendering O(file) per finding — quadratic over a file's worth of
            // findings. `line_start` anchors the gutter to absolute lines.
            let first_line = line_index.byte_to_lc(start).line;
            let last_line = line_index.byte_to_lc(end).line;
            let window_first = first_line.saturating_sub(2);
            let slice_start = line_index.line_start(window_first);
            let slice_end = line_index.line_start(last_line + 1);
            let snippet = Snippet::source(&source[slice_start..slice_end])
                .line_start(window_first + 1)
                .path(&origin)
                .annotation(
                    AnnotationKind::Primary
                        .span(start - slice_start..end - slice_start)
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

#[cfg(test)]
mod tests {
    use rowan::{TextRange, TextSize};

    use super::*;
    use crate::linter::diagnostic::ViolationData;

    fn diag(source_range: std::ops::Range<u32>, body: &str) -> Diagnostic {
        Diagnostic {
            rule: "test-rule",
            severity: Severity::Warning,
            path: PathBuf::from("lint.R"),
            range: TextRange::new(
                TextSize::from(source_range.start),
                TextSize::from(source_range.end),
            ),
            message: ViolationData::new("test-rule", body),
            fix: None,
        }
    }

    fn render(source: &str, diagnostics: &[Diagnostic]) -> String {
        let source = source.to_string();
        render_findings(diagnostics, OutputMode::Pretty, false, &|_| {
            Some(source.clone())
        })
    }

    #[test]
    fn pretty_reports_absolute_line_numbers_deep_into_a_file() {
        // 499 filler lines, then the flagged line: the gutter and --> header
        // must show line 500 even though only that line is rendered.
        let mut source = String::new();
        for i in 1..500 {
            source.push_str(&format!("filler_{i} <- {i}\n"));
        }
        let start = source.len() as u32;
        source.push_str("bad_symbol\n");
        let out = render(&source, &[diag(start..start + 10, "`bad_symbol` is bad")]);
        let expected = "\
warning: test-rule
   --> lint.R:500:1
    |
500 | bad_symbol
    | ^^^^^^^^^^ `bad_symbol` is bad
";
        assert_eq!(out, expected);
    }

    #[test]
    fn pretty_renders_a_multi_line_span() {
        let source = "before <- 1\ncall(\n  a,\n  b\n)\nafter <- 2\n";
        // Span covers the whole `call(...)` expression: lines 2-5.
        let start = source.find("call").unwrap() as u32;
        let end = (source.find(")\n").unwrap() + 1) as u32;
        let out = render(source, &[diag(start..end, "spans several lines")]);
        let expected = "\
warning: test-rule
 --> lint.R:2:1
  |
2 | / call(
3 | |   a,
4 | |   b
5 | | )
  | |_^ spans several lines
";
        assert_eq!(out, expected);
    }

    #[test]
    fn pretty_renders_an_empty_span_at_eof() {
        let source = "x <- 1\ny <- 2";
        let len = source.len() as u32;
        let out = render(source, &[diag(len..len, "something missing at eof")]);
        let expected = "\
warning: test-rule
 --> lint.R:2:7
  |
2 | y <- 2
  |       ^ something missing at eof
";
        assert_eq!(out, expected);
    }

    #[test]
    fn pretty_renders_findings_on_first_and_last_lines() {
        let source = "first <- 1\nmiddle <- 2\nlast <- 3";
        let out = render(
            source,
            &[diag(0..5, "first line"), diag(23..27, "last line")],
        );
        let expected = "\
warning: test-rule
 --> lint.R:1:1
  |
1 | first <- 1
  | ^^^^^ first line
warning: test-rule
 --> lint.R:3:1
  |
3 | last <- 3
  | ^^^^ last line
";
        assert_eq!(out, expected);
    }
}
