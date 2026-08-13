use super::*;

/// Resolve signature help off the snapshot's cached parse when the db's tracked
/// buffer for `path` still matches `text`; otherwise re-parse. Falls back on
/// cancellation. Mirrors [`hover_via_db`].
pub(crate) fn signature_help_via_db(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    position: Position,
    encoding: PositionEncoding,
) -> Option<SignatureHelp> {
    let text = buffer.text();
    let line_index = buffer.line_index();
    let offset = line_index
        .position_to_byte(position, encoding)
        .min(text.len());
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

/// The index of the parameter the cursor is positioned at, following R's
/// argument matching (R Language Definition 4.3.2): a `name = ` argument binds
/// by exact tag, then by unique prefix among the formals *before* `...`; an
/// unmatched name falls into `...` when the function is variadic. A positional
/// argument takes the n-th formal not already bound by name, where n counts the
/// positional arguments before the cursor.
fn active_parameter(
    arg_list: Option<&ArgList>,
    offset: TextSize,
    entry: &SymbolEntry,
    param_count: usize,
) -> Option<u32> {
    if param_count == 0 {
        return None;
    }
    let formals = entry.formals.as_ref()?;
    let Some(arg_list) = arg_list else {
        return Some(0);
    };
    let active = arg_list
        .args()
        .find(|a| a.syntax().text_range().contains_inclusive(offset));
    let dots = formals.iter().position(|f| f.name == DOTS);

    // A named argument binds to its formal regardless of position.
    if let Some(name) = active.as_ref().and_then(Arg::name) {
        return match match_formal(&name, formals) {
            FormalMatch::Matched(idx) => Some(idx as u32),
            // Ambiguous is an error in R: highlight nothing rather than guess.
            FormalMatch::Ambiguous => None,
            FormalMatch::NoMatch => dots.map(|idx| idx as u32),
        };
    }

    // Positional. Arguments of nested calls live under their own `ARG_LIST`, so
    // walking this one's never leaks across nesting. Empty slots (`f(a, , b)`)
    // are zero-width `ARG` nodes and count, as they do in R.
    let mut bound = Vec::new();
    let mut preceding = 0usize;
    for arg in arg_list.args() {
        if active.as_ref().is_some_and(|a| a.syntax() == arg.syntax()) {
            continue;
        }
        match arg.name() {
            Some(name) => {
                if let FormalMatch::Matched(idx) = match_formal(&name, formals) {
                    bound.push(idx);
                }
            }
            None if arg.syntax().text_range().end() <= offset => preceding += 1,
            None => {}
        }
    }
    let mut remaining = preceding;
    let mut last = None;
    for (idx, formal) in formals.iter().enumerate() {
        // Positional matching stops at `...`: it and everything after it can
        // only be reached by name, and surplus positionals all land in `...`.
        if formal.name == DOTS {
            return Some(idx as u32);
        }
        if bound.contains(&idx) {
            continue;
        }
        if remaining == 0 {
            return Some(idx as u32);
        }
        remaining -= 1;
        last = Some(idx as u32);
    }
    // More positional arguments than formals: invalid R, so just stay on the
    // last formal rather than dropping the highlight mid-call.
    last
}

/// The variadic formal's name.
const DOTS: &str = "...";

/// How an argument tag binds to a formal.
enum FormalMatch {
    Matched(usize),
    /// A prefix of more than one formal: an error in R.
    Ambiguous,
    NoMatch,
}

/// Match an argument tag against `formals` the way R does: exact tag first,
/// then a unique prefix among the formals before `...` (formals from `...`
/// onward match only exactly).
fn match_formal(name: &str, formals: &[Formal]) -> FormalMatch {
    if let Some(idx) = formals.iter().position(|f| f.name == name) {
        return FormalMatch::Matched(idx);
    }
    let partial: Vec<usize> = formals
        .iter()
        .take_while(|f| f.name != DOTS)
        .enumerate()
        .filter(|(_, f)| f.name.starts_with(name))
        .map(|(idx, _)| idx)
        .collect();
    match partial.as_slice() {
        [idx] => FormalMatch::Matched(*idx),
        [] => FormalMatch::NoMatch,
        _ => FormalMatch::Ambiguous,
    }
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
    fn named_arg_active_right_after_equals() {
        // The `=` retrigger scenario: the cursor sits immediately after `=`,
        // closed call and unclosed call alike.
        let help = help_at("library(dplyr)\nacross(.fns =@)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(1));
        let help = help_at("library(dplyr)\nacross(.fns =@").expect("signature");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn partial_name_matches_like_r() {
        // `.f` is a unique prefix of `.fns`, so R binds it there.
        let help = help_at("library(dplyr)\nacross(.f = @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn unknown_name_has_no_active_parameter() {
        // `across` is not variadic in the fixture, so `zzz` binds nothing:
        // better no highlight than a wrong one.
        let help = help_at("library(dplyr)\nacross(zzz = @)\n").expect("signature");
        assert_eq!(help.active_parameter, None);
    }

    #[test]
    fn positional_skips_name_bound_formal() {
        // `.fns` is taken by name, so the positional slot is `.cols`.
        let help = help_at("library(dplyr)\nacross(.fns = 1, @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(0));
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
            title: None,
            r_version: None,
            harvested_at: 0,
            attaches: Vec::new(),
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

    /// A variadic, prefix-ambiguous fixture:
    /// `summarise(x, na.rm = FALSE, nan.rm = FALSE, ..., .keep = "all")`.
    fn variadic_provider() -> IndexedProvider {
        use crate::rindex::schema::{Formal, PackageIndex, SCHEMA_VERSION};
        let formal = |name: &str, default: Option<&str>| Formal {
            name: name.into(),
            default: default.map(str::to_string),
        };
        let idx = PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "dplyr".into(),
            version: "1.0".into(),
            lib_path: "/lib".into(),
            title: None,
            r_version: None,
            harvested_at: 0,
            attaches: Vec::new(),
            symbols: vec![SymbolEntry {
                name: "summarise".into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: Some(vec![
                    formal("x", None),
                    formal("na.rm", Some("FALSE")),
                    formal("nan.rm", Some("FALSE")),
                    formal("...", None),
                    formal(".keep", Some("\"all\"")),
                ]),
                help: None,
            }],
        };
        IndexedProvider::from_indices([idx])
    }

    /// Like [`help_at`], but against [`variadic_provider`].
    fn variadic_help_at(src: &str) -> Option<SignatureHelp> {
        let offset = src.find('@').expect("cursor marker");
        compute_signature_help(&src.replace('@', ""), offset, &variadic_provider())
    }

    #[test]
    fn extra_positional_lands_in_dots() {
        // `x`, `na.rm`, `nan.rm` are taken; everything after falls into `...`.
        let help = variadic_help_at("dplyr::summarise(a, b, c, @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(3));
    }

    #[test]
    fn unknown_name_lands_in_dots_when_variadic() {
        let help = variadic_help_at("dplyr::summarise(zzz = @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(3));
    }

    #[test]
    fn ambiguous_prefix_has_no_active_parameter() {
        // `n` prefixes both `na.rm` and `nan.rm`; R errors, so highlight nothing.
        let help = variadic_help_at("dplyr::summarise(n = @)\n").expect("signature");
        assert_eq!(help.active_parameter, None);
    }

    #[test]
    fn formal_after_dots_needs_an_exact_name() {
        // Partial matching stops at `...`, so `.k` goes to `...`, not `.keep`.
        let help = variadic_help_at("dplyr::summarise(.k = @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(3));
        let help = variadic_help_at("dplyr::summarise(.keep = @)\n").expect("signature");
        assert_eq!(help.active_parameter, Some(4));
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
        let help = signature_help_via_db(
            &db.snapshot(),
            path,
            &buf(src),
            position,
            PositionEncoding::Utf16,
        )
        .expect("signature via db");
        assert_eq!(help.signatures.len(), 1);

        // Untracked path still resolves, via the fresh-parse fallback.
        let mut empty = IncrementalDatabase::default();
        empty.set_library_index(documented_dplyr());
        assert!(
            signature_help_via_db(
                &empty.snapshot(),
                path,
                &buf(src),
                position,
                PositionEncoding::Utf16
            )
            .is_some(),
            "fallback signature help should resolve too"
        );
    }
}
