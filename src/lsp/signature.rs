use super::*;

/// Resolve signature help off the snapshot's cached parse when the db's tracked
/// buffer for `path` still matches `text`; otherwise re-parse. Falls back on
/// cancellation. Mirrors [`hover_via_db`].
pub(crate) fn signature_help_via_db(
    snapshot: &Analysis,
    path: &Path,
    text: &str,
    position: Position,
) -> Option<SignatureHelp> {
    let line_index = LineIndex::new(text);
    let offset = line_index.position_to_byte(position).min(text.len());
    let index = snapshot.library_data().unwrap_or_default();
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if snapshot.file_text(file) != text {
            return None;
        }
        let root = snapshot.parsed_tree(file);
        Some(signature_help_from_node(&root, offset, &index))
    }));
    match cached {
        Ok(Some(help)) => help,
        Ok(None) | Err(_) => {
            let root = parse(text).cst;
            signature_help_from_node(&root, offset, &index)
        }
    }
}

/// Build signature help for the cursor at byte `offset`, if it sits inside a
/// call whose callee resolves to an indexed export. Pure (parses `text` itself)
/// so it is unit-testable.
pub fn compute_signature_help(
    text: &str,
    offset: usize,
    indexed: &IndexedProvider,
) -> Option<SignatureHelp> {
    let root = parse(text).cst;
    signature_help_from_node(&root, offset.min(text.len()), indexed)
}

/// Build signature help off an already-parsed CST, without re-parsing. The LSP
/// read path uses this against the cached parse tree; [`compute_signature_help`]
/// is the parse-from-text wrapper.
pub(crate) fn signature_help_from_node(
    root: &SyntaxNode,
    offset: usize,
    indexed: &IndexedProvider,
) -> Option<SignatureHelp> {
    let offset = TextSize::new(offset as u32);
    let call = enclosing_call(root, offset)?;
    let callee = call.callee_token()?;
    // Resolve the callee through the same index path hover uses: this reuses
    // bare-name origin resolution and `pkg::fn(` namespace handling alike.
    let query = symbol_query_at(root, callee.text_range().start())?;
    let (_package, entry, _range) = resolve_query(query, root, indexed)?;

    let (label, parameters) = build_signature(entry)?;
    let active = active_parameter(call.arg_list().as_ref(), offset, entry, parameters.len());
    let info = SignatureInformation {
        label,
        documentation: signature_documentation(entry),
        parameters: (!parameters.is_empty()).then_some(parameters),
        active_parameter: active,
    };
    Some(SignatureHelp {
        signatures: vec![info],
        active_signature: Some(0),
        active_parameter: active,
    })
}

/// The innermost `CALL_EXPR` whose argument list the cursor sits inside, or
/// `None` when the cursor isn't within a call's parentheses.
fn enclosing_call(root: &SyntaxNode, offset: TextSize) -> Option<CallExpr> {
    // Prefer the right token at a boundary, so a cursor just past a closing `)`
    // lands on what follows (outside the call) rather than back inside it, and a
    // cursor between two `)` of nested calls selects the still-open outer call.
    let token = match root.token_at_offset(offset) {
        TokenAtOffset::None => return None,
        TokenAtOffset::Single(t) => t,
        TokenAtOffset::Between(_left, right) => right,
    };
    let call = token.parent_ancestors().find_map(CallExpr::cast)?;
    // Reject a cursor past the closing paren: signature help is for *inside* the
    // call. An unclosed call (no `)`) has no such bound and always qualifies.
    if let Some(rparen) = call.arg_list().and_then(|al| {
        al.syntax()
            .children_with_tokens()
            .find(|el| el.kind() == SyntaxKind::RPAREN)
    }) && offset > rparen.text_range().start()
    {
        return None;
    }
    Some(call)
}

/// The signature label and its parameters. When formals are known the label is
/// built *from them* so every parameter's highlight offsets align with the
/// label (an Rd `\usage` string need not). Falls back to the raw `\usage` line
/// (no per-parameter highlight) when formals are absent.
fn build_signature(entry: &SymbolEntry) -> Option<(String, Vec<ParameterInformation>)> {
    if let Some(formals) = &entry.formals {
        let mut label = String::new();
        label.push_str(&entry.name);
        label.push('(');
        let mut parameters = Vec::with_capacity(formals.len());
        for (i, formal) in formals.iter().enumerate() {
            if i > 0 {
                label.push_str(", ");
            }
            // LSP label offsets are UTF-16 code units.
            let start = label.encode_utf16().count() as u32;
            label.push_str(&format_formal(formal));
            let end = label.encode_utf16().count() as u32;
            parameters.push(ParameterInformation {
                label: ParameterLabel::LabelOffsets([start, end]),
                documentation: parameter_documentation(entry, &formal.name),
            });
        }
        label.push(')');
        Some((label, parameters))
    } else {
        let usage = entry.help.as_ref().and_then(|h| h.usage.as_deref())?;
        Some((usage.to_string(), Vec::new()))
    }
}

/// The index of the parameter the cursor is positioned at: the formal named by
/// the enclosing `name = ` argument when present, else the count of top-level
/// commas before the cursor (clamped to the last parameter).
fn active_parameter(
    arg_list: Option<&ArgList>,
    offset: TextSize,
    entry: &SymbolEntry,
    param_count: usize,
) -> Option<u32> {
    if param_count == 0 {
        return None;
    }
    let Some(arg_list) = arg_list else {
        return Some(0);
    };
    // A named argument binds to its formal regardless of position.
    if let Some(name) = active_named_arg(arg_list, offset)
        && let Some(idx) = entry
            .formals
            .as_ref()
            .and_then(|formals| formals.iter().position(|f| f.name == name))
    {
        return Some(idx as u32);
    }
    // Positional: top-level commas before the cursor. Commas of nested calls
    // live under their own `ARG_LIST`, so this count never leaks across nesting.
    let positional = arg_list
        .syntax()
        .children_with_tokens()
        .filter(|el| el.kind() == SyntaxKind::COMMA && el.text_range().end() <= offset)
        .count();
    Some(positional.min(param_count - 1) as u32)
}

/// The name of the enclosing `name = value` argument at `offset`, if any.
fn active_named_arg(arg_list: &ArgList, offset: TextSize) -> Option<SmolStr> {
    let arg = arg_list
        .args()
        .find(|a| a.syntax().text_range().contains_inclusive(offset))?;
    let mut significant = arg
        .syntax()
        .children_with_tokens()
        .filter(|el| !is_trivia_or_comment(el.kind()));
    let name = significant.next()?.into_token()?;
    if name.kind() != SyntaxKind::IDENT {
        return None;
    }
    let eq = significant.next()?;
    (eq.kind() == SyntaxKind::ASSIGN_EQ).then(|| SmolStr::new(name.text()))
}

fn is_trivia_or_comment(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
    )
}

/// Per-parameter documentation drawn from the indexed `\arguments` block.
fn parameter_documentation(entry: &SymbolEntry, name: &str) -> Option<Documentation> {
    let help = entry.help.as_ref()?;
    let arg = help.arguments.iter().find(|a| a.name == name)?;
    Some(Documentation::String(arg.description.clone()))
}

/// Signature-level documentation: the indexed title and description.
fn signature_documentation(entry: &SymbolEntry) -> Option<Documentation> {
    let help = entry.help.as_ref()?;
    let mut out = String::new();
    if let Some(title) = &help.title {
        out.push_str(title);
    }
    if let Some(description) = &help.description {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(description);
    }
    (!out.is_empty()).then_some(Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: out,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signature help with the cursor at the `@` marker in `src`, resolved
    /// against the documented dplyr fixture (`across(.cols, .fns)`).
    fn help_at(src: &str) -> Option<SignatureHelp> {
        let offset = src.find('@').expect("cursor marker");
        let text = src.replace('@', "");
        compute_signature_help(&text, offset, &documented_dplyr())
    }

    #[test]
    fn first_argument_is_active() {
        let help = help_at("library(dplyr)\nacross(@)\n").expect("signature");
        assert_eq!(help.signatures.len(), 1);
        assert_eq!(help.active_parameter, Some(0));
        let info = &help.signatures[0];
        assert!(info.label.contains(".cols"), "label: {}", info.label);
        assert_eq!(info.parameters.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn second_argument_active_after_comma() {
        let help = help_at("library(dplyr)\nacross(a, @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn named_argument_overrides_position() {
        // `.fns` is the second formal, but it is written in the first position.
        let help = help_at("library(dplyr)\nacross(.fns = 1@)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn nested_call_commas_do_not_leak() {
        let help = help_at("library(dplyr)\nacross(foo(a, b), @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn namespaced_call_resolves_without_library() {
        let help = help_at("dplyr::across(@)\n").expect("signature");
        assert_eq!(help.signatures.len(), 1);
        assert_eq!(help.active_parameter, Some(0));
    }

    #[test]
    fn none_outside_a_call() {
        assert!(help_at("library(dplyr)\nx <- 1@\n").is_none());
    }

    #[test]
    fn none_after_closing_paren() {
        assert!(help_at("library(dplyr)\nacross(a)@\n").is_none());
    }

    #[test]
    fn none_for_computed_callee() {
        // `x$f(...)` has no simple callee name to resolve.
        assert!(help_at("x$f(@)\n").is_none());
    }

    #[test]
    fn usage_only_entry_has_label_without_parameters() {
        use crate::rindex::schema::{HelpDoc, PackageIndex, SCHEMA_VERSION};
        let idx = PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "base".into(),
            version: "4.5.3".into(),
            lib_path: "/lib".into(),
            r_version: None,
            harvested_at: 0,
            symbols: vec![SymbolEntry {
                name: "as.matrix".into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help: Some(HelpDoc {
                    title: Some("Matrices".into()),
                    description: None,
                    usage: Some("as.matrix(x, ...)".into()),
                    arguments: vec![],
                }),
            }],
        };
        let provider = IndexedProvider::from_indices([idx]);
        let src = "as.matrix(@x)\n";
        let offset = src.find('@').unwrap();
        let help = compute_signature_help(&src.replace('@', ""), offset, &provider)
            .expect("signature for as.matrix");
        assert_eq!(help.signatures[0].label, "as.matrix(x, ...)");
        assert!(help.signatures[0].parameters.is_none());
        assert_eq!(help.active_parameter, None);
    }

    #[test]
    fn signature_help_via_db_matches_compute() {
        use crate::incremental::IncrementalDatabase;
        let path = test_path();
        let src = "library(dplyr)\nacross(a, mean)\n";
        // Cursor inside the call, just after `across(`.
        let position = pos(1, 7);

        let mut db = IncrementalDatabase::default();
        db.set_library_index(documented_dplyr());
        db.upsert_file(path, src.to_string());
        let help =
            signature_help_via_db(&db.snapshot(), path, src, position).expect("signature via db");
        assert_eq!(help.signatures.len(), 1);

        // Untracked path still resolves, via the fresh-parse fallback.
        let mut empty = IncrementalDatabase::default();
        empty.set_library_index(documented_dplyr());
        assert!(
            signature_help_via_db(&empty.snapshot(), path, src, position).is_some(),
            "fallback signature help should resolve too"
        );
    }
}
