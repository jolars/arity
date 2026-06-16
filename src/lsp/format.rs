use super::*;

/// Format `text` off the snapshot's cached parse when the db's tracked buffer
/// for `path` still matches it; otherwise re-parse. A write racing the read
/// trips [`salsa::Cancelled`], which also falls back to a fresh parse.
pub(crate) fn format_edits_via_db(
    snapshot: &Analysis,
    path: &Path,
    text: &str,
    style: FormatStyle,
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
        let formatted = format_node(&root, style, text.ends_with('\n')).ok();
        Some(formatted.map(|formatted| edits_for_formatted(text, formatted)))
    }));
    match cached {
        Ok(Some(edits)) => edits,
        // Cache miss (`Ok(None)`) or a racing write (`Err`): re-parse from text.
        Ok(None) | Err(_) => compute_format_edits(text, style),
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
        let text_range = lsp_range_to_text_range(&line_index, range);
        let edits = match format_range(&root, text_range, style) {
            Ok(Some(formatted)) => Some(range_edits(&line_index, text, formatted)),
            Ok(None) => Some(Vec::new()),
            Err(_) => None,
        };
        Some(edits)
    }));
    match cached {
        Ok(Some(edits)) => edits,
        // Cache miss (`Ok(None)`) or a racing write (`Err`): re-parse from text.
        Ok(None) | Err(_) => compute_format_range_edits(text, range, style),
    }
}

/// Compute the LSP `TextEdit`s to format `text` with `style`, re-parsing it.
///
/// Returns `None` when the formatter rejects the input (e.g. parse error).
/// An empty `Vec` means the document is already formatted.
pub fn compute_format_edits(text: &str, style: FormatStyle) -> Option<Vec<TextEdit>> {
    let formatted = format_with_style(text, style).ok()?;
    Some(edits_for_formatted(text, formatted))
}

/// Compute the LSP `TextEdit`s to format the selection `range` of `text`,
/// re-parsing it.
///
/// Returns `None` when the formatter rejects the input (e.g. parse error). An
/// empty `Vec` means the selected region is already formatted or covers no
/// statement.
pub fn compute_format_range_edits(
    text: &str,
    range: Range,
    style: FormatStyle,
) -> Option<Vec<TextEdit>> {
    let parsed = parse(text);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let line_index = LineIndex::new(text);
    let text_range = lsp_range_to_text_range(&line_index, range);
    match format_range(&parsed.cst, text_range, style).ok()? {
        Some(formatted) => Some(range_edits(&line_index, text, formatted)),
        None => Some(Vec::new()),
    }
}

/// Convert a byte `TextRange` to an LSP `Range` via `line_index` (built over the
/// text the range indexes).
pub(crate) fn text_range_to_lsp_range(line_index: &LineIndex, range: TextRange) -> Range {
    Range {
        start: line_index.byte_to_position(u32::from(range.start()) as usize),
        end: line_index.byte_to_position(u32::from(range.end()) as usize),
    }
}

/// Convert an LSP `Range` to a byte `TextRange`. `position_to_byte` already
/// clamps to the text length; we only ensure `start <= end`.
pub(crate) fn lsp_range_to_text_range(line_index: &LineIndex, range: Range) -> TextRange {
    let start = line_index.position_to_byte(range.start);
    let end = line_index.position_to_byte(range.end);
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
) -> Vec<TextEdit> {
    let start = usize::from(formatted.range.start());
    let end = usize::from(formatted.range.end());
    if text.get(start..end) == Some(formatted.text.as_str()) {
        return Vec::new();
    }
    vec![TextEdit {
        range: Range {
            start: line_index.byte_to_position(start),
            end: line_index.byte_to_position(end),
        },
        new_text: formatted.text,
    }]
}

/// The whole-document edit replacing `text` with its formatted form (empty when
/// already formatted). The single source of the edit geometry shared by the
/// re-parse path ([`compute_format_edits`]) and the cached-tree path.
pub(crate) fn edits_for_formatted(text: &str, formatted: String) -> Vec<TextEdit> {
    if formatted == text {
        return Vec::new();
    }
    let line_index = LineIndex::new(text);
    let end = line_index.byte_to_position(text.len());
    vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end,
        },
        new_text: formatted,
    }]
}

pub(crate) fn to_lsp_diagnostic(d: &Diagnostic, idx: &LineIndex) -> LspDiagnostic {
    let start = idx.byte_to_position(u32::from(d.range.start()) as usize);
    let end = idx.byte_to_position(u32::from(d.range.end()) as usize);
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let expected = compute_format_edits(buffer, style);
        assert!(
            matches!(&expected, Some(edits) if !edits.is_empty()),
            "fixture must require reformatting"
        );

        // Cache hit: tracked text == buffer → format off the cached tree.
        let mut db = IncrementalDatabase::default();
        db.upsert_file(path, buffer.to_string());
        let snapshot = db.snapshot();
        assert_eq!(
            format_edits_via_db(&snapshot, path, buffer, style),
            expected,
            "cached-tree format must match the re-parse path"
        );

        // Stale db (tracked text lags the buffer) → fall back to a fresh parse.
        let mut stale = IncrementalDatabase::default();
        stale.upsert_file(path, "y <- 1\n".to_string());
        assert_eq!(
            format_edits_via_db(&stale.snapshot(), path, buffer, style),
            expected,
            "version skew must fall back to the buffer text"
        );

        // Untracked path → fall back as well.
        let empty = IncrementalDatabase::default();
        assert_eq!(
            format_edits_via_db(&empty.snapshot(), path, buffer, style),
            expected,
            "untracked path must fall back to the buffer text"
        );
    }
}
