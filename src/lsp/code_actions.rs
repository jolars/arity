use super::*;

/// Build quick-fix code actions for the fixes whose diagnostics overlap
/// `range`. Pure (no IO beyond the in-memory `text`) so it can be unit-tested.
pub fn compute_code_actions(
    text: &str,
    path: &std::path::Path,
    lint: &LintConfig,
    uri: &Uri,
    range: Range,
    encoding: PositionEncoding,
) -> CodeActionResponse {
    let diagnostics = crate::linter::check_document(path, text, lint).unwrap_or_default();
    code_actions_from_findings(&diagnostics, &TextBuffer::from(text), uri, range, encoding)
}

/// Build quick-fix code actions from already-computed lint findings, for the
/// fixes whose diagnostics overlap `range`. `buffer` must be the source the
/// `findings` were produced against (their ranges are byte offsets into it), so
/// the LSP only serves cached findings when the buffer version still matches.
pub(crate) fn code_actions_from_findings(
    findings: &[Diagnostic],
    buffer: &TextBuffer,
    uri: &Uri,
    range: Range,
    encoding: PositionEncoding,
) -> CodeActionResponse {
    let text = buffer.text();
    let line_index = buffer.line_index();

    let mut actions: CodeActionResponse = findings
        .iter()
        .filter_map(|d| {
            let fix = d.fix.as_ref()?;
            let diag_range = Range {
                start: line_index.byte_to_position(u32::from(d.range.start()) as usize, encoding),
                end: line_index.byte_to_position(u32::from(d.range.end()) as usize, encoding),
            };
            if !ranges_overlap(diag_range, range) {
                return None;
            }
            let edit = TextEdit {
                range: Range {
                    start: line_index.byte_to_position(fix.start, encoding),
                    end: line_index.byte_to_position(fix.end, encoding),
                },
                new_text: fix.content.clone(),
            };
            let mut changes = HashMap::new();
            changes.insert(uri.clone(), vec![edit]);
            Some(CodeActionOrCommand::CodeAction(CodeAction {
                title: fix.description.clone(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![to_lsp_diagnostic(d, line_index, encoding)]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        })
        .collect();

    // Cursor-context roxygen skeleton action (not diagnostic-backed).
    if let Some(action) = roxygen_code_action(text, uri, range, encoding) {
        actions.push(action);
    }

    actions
}

/// Inclusive overlap test for two LSP ranges (a zero-width cursor touching a
/// diagnostic's edge counts as overlapping, so the quick-fix still shows up).
pub(crate) fn ranges_overlap(a: Range, b: Range) -> bool {
    !(position_lt(a.end, b.start) || position_lt(b.end, a.start))
}

pub(crate) fn position_lt(a: Position, b: Position) -> bool {
    (a.line, a.character) < (b.line, b.character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_action_offers_quickfix_for_diagnostic_in_range() {
        let src = "if (x = 1) print(x)\n";
        let actions = compute_code_actions(
            src,
            test_path(),
            &LintConfig::default(),
            &test_uri(),
            full_line_0(),
            PositionEncoding::Utf16,
        );

        let CodeActionOrCommand::CodeAction(action) = actions
            .iter()
            .find(|a| matches!(a, CodeActionOrCommand::CodeAction(a) if a.title.contains("==")))
            .expect("an `=` → `==` quick-fix")
        else {
            unreachable!()
        };
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        let changes = action
            .edit
            .as_ref()
            .and_then(|e| e.changes.as_ref())
            .expect("workspace edit with changes");
        let edits = changes.get(&test_uri()).expect("edits for our uri");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "==");
        // The edit targets the `=` token on line 0.
        assert_eq!(edits[0].range.start.line, 0);
    }

    #[test]
    fn code_action_empty_when_range_misses_diagnostics() {
        let src = "if (x = 1) print(x)\n";
        let far = Range {
            start: pos(5, 0),
            end: pos(5, 0),
        };
        let actions = compute_code_actions(
            src,
            test_path(),
            &LintConfig::default(),
            &test_uri(),
            far,
            PositionEncoding::Utf16,
        );
        assert!(actions.is_empty(), "expected no actions, got {actions:?}");
    }
}
