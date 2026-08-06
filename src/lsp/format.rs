use super::*;

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
    text: &str,
    style: FormatStyle,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            // The tracked input lags the live buffer; the cached tree is stale.
            return None;
        }
        if !snapshot.parse_diagnostics(file).is_empty() {
            // Parse errors: the formatter refuses, like `compute_format_edits`.
            return Some(None);
        }
        let root = snapshot.parsed_tree(file);
        let formatted = format_node(&root, style, text).ok();
        Some(formatted.map(|formatted| edits_for_formatted(text, formatted, encoding)))
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
    text: &str,
    range: Range,
    style: FormatStyle,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            // The tracked input lags the live buffer; the cached tree is stale.
            return None;
        }
        if !snapshot.parse_diagnostics(file).is_empty() {
            // Parse errors: the formatter refuses, like the whole-document path.
            return Some(None);
        }
        let root = snapshot.parsed_tree(file);
        let line_index = LineIndex::new(text);
        let text_range = lsp_range_to_text_range(&line_index, range, encoding);
        let edits = match format_range(&root, text_range, style, text) {
            Ok(Some(formatted)) => Some(range_edits(&line_index, text, formatted, encoding)),
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
pub(crate) fn range_edits(
    line_index: &LineIndex,
    text: &str,
    formatted: crate::formatter::RangeFormatted,
    encoding: PositionEncoding,
) -> Vec<TextEdit> {
    let start = usize::from(formatted.range.start());
    let end = usize::from(formatted.range.end());
    if text.get(start..end) == Some(formatted.text.as_str()) {
        return Vec::new();
    }
    vec![TextEdit {
        range: Range {
            start: line_index.byte_to_position(start, encoding),
            end: line_index.byte_to_position(end, encoding),
        },
        new_text: formatted.text,
    }]
}

/// The whole-document edit replacing `text` with its formatted form (empty when
/// already formatted). The single source of the edit geometry shared by the
/// re-parse path ([`compute_format_edits`]) and the cached-tree path.
pub(crate) fn edits_for_formatted(
    text: &str,
    formatted: String,
    encoding: PositionEncoding,
) -> Vec<TextEdit> {
    if formatted == text {
        return Vec::new();
    }
    let line_index = LineIndex::new(text);
    let end = line_index.byte_to_position(text.len(), encoding);
    vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end,
        },
        new_text: formatted,
    }]
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
    text: &str,
    encoding: PositionEncoding,
) -> Vec<LspDiagnostic> {
    let idx = LineIndex::new(text);
    findings
        .iter()
        .map(|d| to_lsp_diagnostic(d, &idx, encoding))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let edits = format_edits_via_db(&snapshot, &path, buffer, style, encoding)
            .expect("formatter accepts the buffer");
        assert!(
            edits.is_empty(),
            "markdown-canonical buffer is clean: {edits:?}"
        );

        // Re-parse fallback (path never tracked): resolves the flag from disk.
        let empty = IncrementalDatabase::default();
        let snapshot = empty.snapshot();
        let edits = format_edits_via_db(&snapshot, &path, buffer, style, encoding)
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
        let items = findings_to_items(&findings, text, PositionEncoding::Utf16);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.range.start, Position::new(1, 0));
        assert_eq!(item.range.end, Position::new(1, 4));
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.source.as_deref(), Some("arity"));
        assert_eq!(item.message, "a demo finding");
        assert!(matches!(&item.code, Some(NumberOrString::String(c)) if c == "demo-rule"));
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
            format_edits_via_db(&snapshot, path, buffer, style, encoding),
            expected,
            "cached-tree format must match the re-parse path"
        );

        // Stale db (tracked text lags the buffer) → fall back to a fresh parse.
        let mut stale = IncrementalDatabase::default();
        stale.upsert_file(path, "y <- 1\n".to_string());
        assert_eq!(
            format_edits_via_db(&stale.snapshot(), path, buffer, style, encoding),
            expected,
            "version skew must fall back to the buffer text"
        );

        // Untracked path → fall back as well.
        let empty = IncrementalDatabase::default();
        assert_eq!(
            format_edits_via_db(&empty.snapshot(), path, buffer, style, encoding),
            expected,
            "untracked path must fall back to the buffer text"
        );
    }
}
