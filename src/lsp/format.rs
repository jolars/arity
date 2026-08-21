use similar::DiffTag;

use super::*;
use crate::text::line_diff::bounded_line_diff;

/// The [`ParseOptions`] for re-parsing `path`'s buffer outside the db: the
/// tracked flag when the file is known to `snapshot`, else resolved from disk
/// (the file may simply not be tracked yet). The cached-tree paths need none of
/// this — the salsa parse already ran under the tracked flag.
fn reparse_options(snapshot: &Analysis, path: &Path) -> ParseOptions {
    let markdown = snapshot
        .lookup_file(path)
        .map(|file| snapshot.roxygen_markdown(file))
        .unwrap_or_else(|| crate::project::description::roxygen_markdown_default_for_file(path));
    ParseOptions::default().with_roxygen_markdown_default(markdown)
}

/// Format `text` off the snapshot's cached parse when the db's tracked buffer
/// for `path` still matches it; otherwise re-parse. A write racing the read
/// trips [`salsa::Cancelled`], which also falls back to a fresh parse.
pub(crate) fn format_edits_via_db(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    style: FormatStyle,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    let text = buffer.text();

    // A `DESCRIPTION` is the other grammar and has no `SourceFile` in the db
    // whose parse could be reused, so it branches before the lookup — the same
    // shape `hover_via_db` and `completion_via_db` use. A red DCF tree is
    // neither `Send` nor `Eq` and has no business in salsa; the file is a few
    // kilobytes, so a fresh parse is not worth caching.
    //
    // `Err` covers both a refusal and a parse error, and `None` here means "no
    // edits", which is what leaves the client's file alone.
    if DocumentKind::from_path(path) == DocumentKind::Description {
        let formatted = format_description_with_style(text, style).ok()?;
        return Some(edits_for_formatted_in(buffer, formatted, encoding));
    }

    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if !snapshot.file_text_is(file, &buffer.text_arc()) {
            // The tracked input lags the live buffer; the cached tree is stale.
            return None;
        }
        if !snapshot.parse_diagnostics(file).is_empty() {
            // Parse errors: the formatter refuses, like `compute_format_edits`.
            return Some(None);
        }
        let root = snapshot.parsed_tree(file);
        let formatted = format_node(&root, style, text).ok();
        Some(formatted.map(|formatted| edits_for_formatted_in(buffer, formatted, encoding)))
    }));
    match cached {
        Ok(Some(edits)) => edits,
        // Cache miss (`Ok(None)`) or a racing write (`Err`): re-parse from text.
        Ok(None) | Err(_) => {
            compute_format_edits(text, style, encoding, &reparse_options(snapshot, path))
        }
    }
}

/// Range-format `text` off the snapshot's cached parse when the db's tracked
/// buffer for `path` still matches it; otherwise re-parse. Mirrors
/// [`format_edits_via_db`]'s cache/cancellation handling.
pub(crate) fn format_range_edits_via_db(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    range: Range,
    style: FormatStyle,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    let text = buffer.text();
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if !snapshot.file_text_is(file, &buffer.text_arc()) {
            // The tracked input lags the live buffer; the cached tree is stale.
            return None;
        }
        if !snapshot.parse_diagnostics(file).is_empty() {
            // Parse errors: the formatter refuses, like the whole-document path.
            return Some(None);
        }
        let root = snapshot.parsed_tree(file);
        let line_index = buffer.line_index();
        let text_range = lsp_range_to_text_range(line_index, range, encoding);
        let edits = match format_range(&root, text_range, style, text) {
            Ok(Some(formatted)) => Some(range_edits(line_index, text, formatted, encoding)),
            Ok(None) => Some(Vec::new()),
            Err(_) => None,
        };
        Some(edits)
    }));
    match cached {
        Ok(Some(edits)) => edits,
        // Cache miss (`Ok(None)`) or a racing write (`Err`): re-parse from text.
        Ok(None) | Err(_) => compute_format_range_edits(
            text,
            range,
            style,
            encoding,
            &reparse_options(snapshot, path),
        ),
    }
}

/// Compute the LSP `TextEdit`s to format `text` with `style`, re-parsing it
/// under `options` (the file's package-wide roxygen markdown default).
///
/// Returns `None` when the formatter rejects the input (e.g. parse error).
/// An empty `Vec` means the document is already formatted.
pub fn compute_format_edits(
    text: &str,
    style: FormatStyle,
    encoding: PositionEncoding,
    options: &ParseOptions,
) -> Option<Vec<TextEdit>> {
    let formatted = format_with_options(text, style, options).ok()?;
    Some(edits_for_formatted(text, formatted, encoding))
}

/// Compute the LSP `TextEdit`s to format the selection `range` of `text`,
/// re-parsing it under `options`.
///
/// Returns `None` when the formatter rejects the input (e.g. parse error). An
/// empty `Vec` means the selected region is already formatted or covers no
/// statement.
pub fn compute_format_range_edits(
    text: &str,
    range: Range,
    style: FormatStyle,
    encoding: PositionEncoding,
    options: &ParseOptions,
) -> Option<Vec<TextEdit>> {
    let parsed = parse_with_options(text, options);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let line_index = LineIndex::new(text);
    let text_range = lsp_range_to_text_range(&line_index, range, encoding);
    match format_range(&parsed.cst, text_range, style, text).ok()? {
        Some(formatted) => Some(range_edits(&line_index, text, formatted, encoding)),
        None => Some(Vec::new()),
    }
}

/// Convert a byte `TextRange` to an LSP `Range` via `line_index` (built over the
/// text the range indexes).
pub(crate) fn text_range_to_lsp_range(
    line_index: &LineIndex,
    range: TextRange,
    encoding: PositionEncoding,
) -> Range {
    Range {
        start: line_index.byte_to_position(u32::from(range.start()) as usize, encoding),
        end: line_index.byte_to_position(u32::from(range.end()) as usize, encoding),
    }
}

/// Convert an LSP `Range` to a byte `TextRange`. `position_to_byte` already
/// clamps to the text length; we only ensure `start <= end`.
pub(crate) fn lsp_range_to_text_range(
    line_index: &LineIndex,
    range: Range,
    encoding: PositionEncoding,
) -> TextRange {
    let start = line_index.position_to_byte(range.start, encoding);
    let end = line_index.position_to_byte(range.end, encoding);
    TextRange::new(
        TextSize::new(start as u32),
        TextSize::new(start.max(end) as u32),
    )
}

/// Turn a [`RangeFormatted`] region into the LSP edit list, dropping the edit
/// when it would not change the buffer.
///
/// Line-scoped within the widened span where the diff is small
/// ([`line_diff_edits`]), one replacement of the whole span otherwise.
pub(crate) fn range_edits(
    line_index: &LineIndex,
    text: &str,
    formatted: crate::formatter::RangeFormatted,
    encoding: PositionEncoding,
) -> Vec<TextEdit> {
    let start = usize::from(formatted.range.start());
    let end = usize::from(formatted.range.end());
    let old = text.get(start..end);
    if old == Some(formatted.text.as_str()) {
        return Vec::new();
    }
    if let Some(edits) =
        old.and_then(|old| line_diff_edits(line_index, start, old, &formatted.text, encoding))
    {
        return edits;
    }
    vec![TextEdit {
        range: Range {
            start: line_index.byte_to_position(start, encoding),
            end: line_index.byte_to_position(end, encoding),
        },
        new_text: formatted.text,
    }]
}

/// The edits turning `text` into its formatted form (empty when already
/// formatted). The single source of the edit geometry shared by the re-parse
/// path ([`compute_format_edits`]) and the cached-tree path.
///
/// Line-scoped where the diff is small ([`line_diff_edits`]), one
/// whole-document replacement otherwise.
pub(crate) fn edits_for_formatted(
    text: &str,
    formatted: String,
    encoding: PositionEncoding,
) -> Vec<TextEdit> {
    edits_for_formatted_in(&TextBuffer::from(text), formatted, encoding)
}

/// [`edits_for_formatted`] against a live buffer, reusing its maintained line
/// index instead of rebuilding one per request.
pub(crate) fn edits_for_formatted_in(
    buffer: &TextBuffer,
    formatted: String,
    encoding: PositionEncoding,
) -> Vec<TextEdit> {
    let text = buffer.text();
    if formatted == text {
        return Vec::new();
    }
    let line_index = buffer.line_index();
    if let Some(edits) = line_diff_edits(line_index, 0, text, &formatted, encoding) {
        return edits;
    }
    let end = line_index.byte_to_position(text.len(), encoding);
    vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end,
        },
        new_text: formatted,
    }]
}

/// A diff touching more than this fraction of the span it replaces is not worth
/// expressing as hunks: past it the client's cursor, folds, and markers are
/// disturbed either way, so one replacement is the cheaper equivalent.
const MAX_DIFF_COVERAGE: f64 = 0.5;

/// The smallest set of line-granular edits turning `old` into `new`, where
/// `old` is the slice of the document at byte offset `base` — the whole
/// document when `base` is 0, a widened range-format span otherwise.
///
/// Returns `None` when the diff degenerates, covering more than
/// [`MAX_DIFF_COVERAGE`] of `old`, leaving the caller to emit its single
/// replacement instead. That is what keeps a `line_width` change or a
/// line-ending normalization, which rewrite every line, from becoming a hunk
/// per line. `None` rather than the edit itself, so neither caller has to clone
/// the formatted string it already owns.
///
/// Why lines and not something finer: a hunk boundary has to be a position the
/// client can reason about, the formatter's unit of change *is* the line, and
/// character-level diffing would cost more than the format it follows to spare
/// edits nobody can see. The edits come back ascending and non-overlapping, as
/// `textDocument/formatting` requires, and every range indexes the *original*
/// document.
fn line_diff_edits(
    line_index: &LineIndex,
    base: usize,
    old: &str,
    new: &str,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    let diff = bounded_line_diff(old, new);

    let mut hunks: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> = Vec::new();
    for op in diff.ops() {
        let (tag, old_lines, new_lines) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        match hunks.last_mut() {
            // Consecutive non-equal ops (a delete abutting an insert) are one
            // hunk: the client gets a replacement rather than a pair of edits
            // meeting at a point.
            Some((old_prev, new_prev))
                if old_prev.end == old_lines.start && new_prev.end == new_lines.start =>
            {
                old_prev.end = old_lines.end;
                new_prev.end = new_lines.end;
            }
            _ => hunks.push((old_lines, new_lines)),
        }
    }

    let covered: usize = hunks
        .iter()
        .map(|(old_lines, _)| diff.old_byte_range(old_lines.clone()).len())
        .sum();
    if hunks.len() > 1 && covered as f64 > old.len() as f64 * MAX_DIFF_COVERAGE {
        return None;
    }

    Some(
        hunks
            .into_iter()
            .map(|(old_lines, new_lines)| TextEdit {
                range: Range {
                    start: line_index.byte_to_position(
                        base + diff.old_byte_range(old_lines.clone()).start,
                        encoding,
                    ),
                    end: line_index
                        .byte_to_position(base + diff.old_byte_range(old_lines).end, encoding),
                },
                new_text: new[diff.new_byte_range(new_lines)].to_string(),
            })
            .collect(),
    )
}

pub(crate) fn to_lsp_diagnostic(
    d: &Diagnostic,
    idx: &LineIndex,
    encoding: PositionEncoding,
) -> LspDiagnostic {
    let start = idx.byte_to_position(u32::from(d.range.start()) as usize, encoding);
    let end = idx.byte_to_position(u32::from(d.range.end()) as usize, encoding);
    let severity = match d.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
        Severity::Hint => DiagnosticSeverity::HINT,
    };
    LspDiagnostic {
        range: Range { start, end },
        severity: Some(severity),
        code: Some(NumberOrString::String(d.rule.to_string())),
        source: Some("arity".to_string()),
        message: d.message.body.clone(),
        ..Default::default()
    }
}

/// Convert a lint's findings into LSP diagnostics against `text` (the source the
/// findings' byte ranges index). Used by the pull-diagnostic path; the push path
/// maps the same way inline in the lint thread.
pub(crate) fn findings_to_items(
    findings: &[Diagnostic],
    buffer: &TextBuffer,
    encoding: PositionEncoding,
) -> Vec<LspDiagnostic> {
    let idx = buffer.line_index();
    findings
        .iter()
        .map(|d| to_lsp_diagnostic(d, idx, encoding))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `DESCRIPTION` is served by the DCF grammar off the live buffer, never
    /// through salsa: there is no `SourceFile` for it, and a red DCF tree is
    /// neither `Send` nor `Eq`.
    #[test]
    fn format_via_db_routes_a_description_to_dcf() {
        use crate::incremental::IncrementalDatabase;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("DESCRIPTION");
        // Valid as R *and* as DCF, so only the routing decides the answer.
        let buffer = "Package: p\nImports: b, a\n";
        let db = IncrementalDatabase::default();
        let snapshot = db.snapshot();

        let edits = format_edits_via_db(
            &snapshot,
            &path,
            &buf(buffer),
            FormatStyle::default(),
            PositionEncoding::Utf16,
        )
        .expect("formatter accepts the buffer");
        // What the edits produce, not their geometry (which
        // `description_edits_are_scoped_to_the_field_that_changes` pins):
        // `Imports` reflowed one-per-line is the DCF answer, not the R one.
        assert_eq!(
            apply(buffer, &edits, PositionEncoding::Utf16),
            "Package: p\nImports:\n    a,\n    b\n"
        );
    }

    /// A `DESCRIPTION` the formatter refuses produces no edits at all, which is
    /// what leaves the client's file untouched.
    #[test]
    fn format_via_db_declines_a_description_it_cannot_restyle() {
        use crate::incremental::IncrementalDatabase;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("DESCRIPTION");
        let db = IncrementalDatabase::default();
        let snapshot = db.snapshot();

        assert!(
            format_edits_via_db(
                &snapshot,
                &path,
                &buf("Package: p\n\nPackage: q\n"),
                FormatStyle::default(),
                PositionEncoding::Utf16,
            )
            .is_none()
        );
    }

    /// LSP document formatting honors the file's package-wide markdown default,
    /// on both the cached-tree path (the salsa parse ran under the tracked
    /// flag) and the re-parse fallback (which resolves the flag itself).
    #[test]
    fn format_via_db_honors_package_markdown_default() {
        use crate::incremental::IncrementalDatabase;

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("R")).expect("R/");
        std::fs::write(
            dir.path().join("DESCRIPTION"),
            "Package: p\nRoxygen: list(markdown = TRUE)\n",
        )
        .expect("DESCRIPTION");
        let path = dir.path().join("R/doc.R");
        // Markdown-canonical: the indented code block survives only in md mode.
        let buffer = "#' Title\n#'\n#' @details\n#' Some prose before the code.\n#'\n#'     code_looking <- \"indented\"\nNULL\n";
        std::fs::write(&path, buffer).expect("doc.R");
        let style = FormatStyle::default();
        let encoding = PositionEncoding::Utf16;

        // Cached-tree path: the tracked file parsed under the resolved flag.
        let mut db = IncrementalDatabase::default();
        db.upsert_file(&path, buffer.to_string());
        let snapshot = db.snapshot();
        let edits = format_edits_via_db(&snapshot, &path, &buf(buffer), style, encoding)
            .expect("formatter accepts the buffer");
        assert!(
            edits.is_empty(),
            "markdown-canonical buffer is clean: {edits:?}"
        );

        // Re-parse fallback (path never tracked): resolves the flag from disk.
        let empty = IncrementalDatabase::default();
        let snapshot = empty.snapshot();
        let edits = format_edits_via_db(&snapshot, &path, &buf(buffer), style, encoding)
            .expect("formatter accepts the buffer");
        assert!(
            edits.is_empty(),
            "fallback resolves the flag too: {edits:?}"
        );
    }

    #[test]
    fn findings_to_items_maps_range_severity_and_code() {
        use crate::linter::ViolationData;
        // "line0\nWARN\n": the finding spans bytes 6..10 (the second line).
        let text = "line0\nWARN\n";
        let findings = vec![Diagnostic {
            rule: "demo-rule",
            severity: Severity::Warning,
            path: test_path().to_path_buf(),
            range: TextRange::new(TextSize::from(6), TextSize::from(10)),
            message: ViolationData::new("demo-rule", "a demo finding"),
            fix: None,
        }];
        let items = findings_to_items(&findings, &buf(text), PositionEncoding::Utf16);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.range.start, Position::new(1, 0));
        assert_eq!(item.range.end, Position::new(1, 4));
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.source.as_deref(), Some("arity"));
        assert_eq!(item.message, "a demo finding");
        assert!(matches!(&item.code, Some(NumberOrString::String(c)) if c == "demo-rule"));
    }

    // --- edit geometry ----------------------------------------------------

    /// Apply `edits` the way a client does: every range indexes the *original*
    /// document, so splicing from the end keeps the earlier offsets valid.
    /// Asserts the LSP requirement that the edits do not overlap along the way.
    fn apply(text: &str, edits: &[TextEdit], encoding: PositionEncoding) -> String {
        let line_index = LineIndex::new(text);
        let mut spans: Vec<(usize, usize, &str)> = edits
            .iter()
            .map(|edit| {
                (
                    line_index.position_to_byte(edit.range.start, encoding),
                    line_index.position_to_byte(edit.range.end, encoding),
                    edit.new_text.as_str(),
                )
            })
            .collect();
        spans.sort_by_key(|&(start, ..)| start);
        for pair in spans.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "edits must not overlap: {spans:?}");
        }
        let mut out = text.to_string();
        for &(start, end, new_text) in spans.iter().rev() {
            out.replace_range(start..end, new_text);
        }
        out
    }

    fn format_edits(text: &str, encoding: PositionEncoding) -> Vec<TextEdit> {
        compute_format_edits(text, FormatStyle::default(), encoding, &Default::default())
            .expect("formatter accepts the fixture")
    }

    /// A one-line change in a longer document is one edit covering that line,
    /// not a whole-document replacement. This is what leaves the client's
    /// cursor, folds, and markers on every other line alone.
    #[test]
    fn format_edits_are_scoped_to_the_lines_that_change() {
        let text = "a <- 1\nb <- 2\nc<-3\nd <- 4\ne <- 5\n";
        let edits = format_edits(text, PositionEncoding::Utf16);
        assert_eq!(edits.len(), 1, "one changed line is one edit: {edits:?}");
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(2, 0), Position::new(3, 0)),
            "the edit must cover exactly the third line"
        );
        assert_eq!(edits[0].new_text, "c <- 3\n");
    }

    /// Changes separated by untouched lines come back as separate edits.
    #[test]
    fn separated_changes_are_separate_edits() {
        let text = "a<-1\nb <- 2\nc <- 3\nd <- 4\ne<-5\n";
        let edits = format_edits(text, PositionEncoding::Utf16);
        assert_eq!(edits.len(), 2, "two changed lines are two edits: {edits:?}");
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(0, 0), Position::new(1, 0))
        );
        assert_eq!(edits[0].new_text, "a <- 1\n");
        assert_eq!(
            edits[1].range,
            Range::new(Position::new(4, 0), Position::new(5, 0))
        );
        assert_eq!(edits[1].new_text, "e <- 5\n");
    }

    /// A diff scattered across most of the document collapses back to the
    /// single whole-document replacement: past that point the client's anchors
    /// are disturbed either way, and one edit is the cheaper equivalent.
    #[test]
    fn scattered_wholesale_change_falls_back_to_one_edit() {
        let text = "a<-1\nb <- 2\nc<-3\nd <- 4\ne<-5\n";
        let edits = format_edits(text, PositionEncoding::Utf16);
        assert_eq!(
            edits.len(),
            1,
            "a majority-of-the-file change must collapse: {edits:?}"
        );
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(0, 0), Position::new(5, 0)),
            "the fallback edit must span the whole document"
        );
    }

    /// The property the whole scheme rests on: whatever set of edits comes
    /// back, applying it to the original must reproduce `format` byte for byte.
    #[test]
    fn format_edits_reproduce_the_formatted_document() {
        let style = FormatStyle::default();
        let cases = [
            "",
            "x <- 1\n",
            "x<-1\n",
            "x <- 1",
            "x<-1",
            "a <- 1\nb <- 2\nc<-3\nd <- 4\ne <- 5\n",
            "a<-1\nb<-2\nc<-3\nd<-4\n",
            "a<-1\nb <- 2\nc <- 3\nd <- 4\ne<-5\n",
            "f <- function(x) {\n  y <- 1\n    z<-2\n  y+z\n}\n",
            "s <- \"\u{1F600}\"\nt<-\"\u{00E9}\"\nu <- 3\n",
            "a <- 1\r\nb<-2\r\nc <- 3\r\n",
            "# comment\n\n\n\nx<-1\n",
            "f(a,b,c)\ng( 1 )\n",
            "#' Title\n#'\n#' @param x A value.\nf<-function(x) x\n",
        ];
        for text in cases {
            let formatted =
                format_with_options(text, style, &Default::default()).expect("fixture must format");
            for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
                let edits = format_edits(text, encoding);
                assert_eq!(
                    apply(text, &edits, encoding),
                    formatted,
                    "edits must reproduce the formatted text for {text:?} ({encoding:?})"
                );
            }
        }
    }

    /// A repetitive rewrite beyond the diff work bound still produces valid
    /// LSP edits rather than making formatting latency depend on diff search.
    #[test]
    fn bounded_repetitive_edits_reproduce_the_formatted_document() {
        let style = FormatStyle::default();
        let text = "x<-1\n".repeat(crate::text::line_diff::MAX_UNANCHORED_LINE_PAIRS.isqrt() + 1);
        let formatted =
            format_with_options(&text, style, &Default::default()).expect("fixture must format");

        for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            let edits = format_edits(&text, encoding);
            assert_eq!(
                apply(&text, &edits, encoding),
                formatted,
                "bounded edits must reproduce the formatter output ({encoding:?})"
            );
        }
    }

    /// A `DESCRIPTION` narrows the same way, through the same choke point: the
    /// clean `Package:` line is left alone and only the reflowed field moves.
    #[test]
    fn description_edits_are_scoped_to_the_field_that_changes() {
        use crate::incremental::IncrementalDatabase;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("DESCRIPTION");
        let text = "Package: p\nImports: b, a\n";
        let db = IncrementalDatabase::default();

        let edits = format_edits_via_db(
            &db.snapshot(),
            &path,
            &buf(text),
            FormatStyle::default(),
            PositionEncoding::Utf16,
        )
        .expect("formatter accepts the buffer");
        assert_eq!(edits.len(), 1, "one reflowed field is one edit: {edits:?}");
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(1, 0), Position::new(2, 0)),
            "the `Package:` line must be left alone"
        );
        assert_eq!(edits[0].new_text, "Imports:\n    a,\n    b\n");
    }

    /// Range formatting narrows within the widened span the same way: the
    /// formatter's unit of work is the enclosing statements, but the edits
    /// cover only the lines that actually changed.
    #[test]
    fn range_edits_are_scoped_to_the_lines_that_change() {
        let text = "a <- 1\nb <- 2\nc<-3\nd <- 4\n";
        // A selection spanning all four statements widens to all four.
        let range = Range::new(Position::new(0, 0), Position::new(3, 6));
        let edits = compute_format_range_edits(
            text,
            range,
            FormatStyle::default(),
            PositionEncoding::Utf16,
            &Default::default(),
        )
        .expect("formats");
        assert_eq!(edits.len(), 1, "one changed line is one edit: {edits:?}");
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(2, 0), Position::new(3, 0)),
            "the edit must cover only the changed line, not the widened span"
        );
        assert_eq!(edits[0].new_text, "c <- 3\n");
    }

    /// [`format_edits_reproduce_the_formatted_document`]'s property for the
    /// range path: the edits reproduce what the formatter produced for the
    /// widened span, and leave the rest of the document alone.
    #[test]
    fn range_edits_reproduce_the_formatted_span() {
        let style = FormatStyle::default();
        let text = "a<-1\nb <- 2\nc<-3\nd <- 4\ne<-5\n";
        for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            // Widens to the middle three statements, leaving the outer two.
            let range = Range::new(Position::new(1, 0), Position::new(3, 6));
            let edits =
                compute_format_range_edits(text, range, style, encoding, &Default::default())
                    .expect("formats");
            assert_eq!(
                apply(text, &edits, encoding),
                "a<-1\nb <- 2\nc <- 3\nd <- 4\ne<-5\n",
                "only the widened span may change ({encoding:?})"
            );
        }
    }

    // --- db read path -----------------------------------------------------

    /// The cached-tree format path matches the re-parse path when the db's
    /// tracked buffer is the live text, and falls back (still correctly) when the
    /// db lags the buffer or has never seen the path.
    #[test]
    fn format_via_db_matches_compute_and_falls_back() {
        use crate::incremental::IncrementalDatabase;
        let style = FormatStyle::default();
        let path = test_path();
        let buffer = "x<-f(1 )\n";
        let encoding = PositionEncoding::Utf16;
        let expected = compute_format_edits(buffer, style, encoding, &Default::default());
        assert!(
            matches!(&expected, Some(edits) if !edits.is_empty()),
            "fixture must require reformatting"
        );

        // Cache hit: tracked text == buffer → format off the cached tree.
        let mut db = IncrementalDatabase::default();
        db.upsert_file(path, buffer.to_string());
        let snapshot = db.snapshot();
        assert_eq!(
            format_edits_via_db(&snapshot, path, &buf(buffer), style, encoding),
            expected,
            "cached-tree format must match the re-parse path"
        );

        // Stale db (tracked text lags the buffer) → fall back to a fresh parse.
        let mut stale = IncrementalDatabase::default();
        stale.upsert_file(path, "y <- 1\n".to_string());
        assert_eq!(
            format_edits_via_db(&stale.snapshot(), path, &buf(buffer), style, encoding),
            expected,
            "version skew must fall back to the buffer text"
        );

        // Untracked path → fall back as well.
        let empty = IncrementalDatabase::default();
        assert_eq!(
            format_edits_via_db(&empty.snapshot(), path, &buf(buffer), style, encoding),
            expected,
            "untracked path must fall back to the buffer text"
        );
    }
}
