use super::*;
use crate::semantic::Binding;

/// Build the outline from cached syntax and semantics when they match the live
/// buffer, falling back to fresh analysis on a cache miss or cancellation.
pub(crate) fn document_symbols_via_db(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    encoding: PositionEncoding,
) -> Vec<DocumentSymbol> {
    with_document_semantics(snapshot, path, buffer, |root, model| {
        document_symbols_from_model(root, model, buffer.line_index(), encoding)
    })
}

/// The document-symbol outline for `text`: every function and variable binding,
/// nested to mirror the source. Pure (parses `text` itself) and unit-testable;
/// single-file, so it never consults the workspace.
///
/// The set of names is authoritative from the [`SemanticModel`] — the file-scope
/// `Local`/`Implicit` predicate behind [`crate::project::file_exports`], lifted to
/// *every* scope so nested locals are included; parameters and `for`-vars are
/// deliberately excluded. The CST then supplies the tree shape and each symbol's
/// spans. Best-effort, with no clean-parse gate (an outline of partial input is
/// still useful).
pub fn compute_document_symbols(text: &str, encoding: PositionEncoding) -> Vec<DocumentSymbol> {
    compute_document_symbols_in(&TextBuffer::from(text), encoding)
}

/// [`compute_document_symbols`] against a live buffer, reusing its maintained
/// line index instead of rebuilding one per request.
pub(crate) fn compute_document_symbols_in(
    buffer: &TextBuffer,
    encoding: PositionEncoding,
) -> Vec<DocumentSymbol> {
    let text = buffer.text();
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    document_symbols_from_model(&root, &model, buffer.line_index(), encoding)
}

fn document_symbols_from_model(
    root: &SyntaxNode,
    model: &SemanticModel,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Vec<DocumentSymbol> {
    // Source order lets the walk skip subtrees without eligible bindings. The
    // model visits assignment values before targets, so its order can differ.
    let mut bindings: Vec<_> = model
        .bindings()
        .iter()
        .filter(|b| matches!(b.kind, BindingKind::Local | BindingKind::Implicit))
        .collect();
    bindings.sort_unstable_by_key(|b| b.def_range.start());
    let mut symbols = Vec::new();
    collect_document_symbols(root, &bindings, line_index, &mut symbols, encoding);
    symbols
}

/// Walk `node`'s child nodes, emitting a [`DocumentSymbol`] for each assignment
/// whose target is a known binding (recursing into its value for nested symbols)
/// and descending through other nodes that contain eligible bindings. This lets
/// bindings nested in an `if`/`for`/`{}` (none of which introduce a symbol of their
/// own) surface at the right level instead of being dropped.
fn collect_document_symbols(
    node: &SyntaxNode,
    mut bindings: &[&Binding],
    line_index: &LineIndex,
    out: &mut Vec<DocumentSymbol>,
    encoding: PositionEncoding,
) {
    for child in node.children() {
        if bindings.is_empty() {
            break;
        }
        let range = child.text_range();
        let start = bindings.partition_point(|b| b.def_range.start() < range.start());
        bindings = &bindings[start..];
        let end = bindings.partition_point(|b| b.def_range.start() < range.end());
        let (within, remaining) = bindings.split_at(end);
        bindings = remaining;
        if within.is_empty() {
            continue;
        }
        match document_symbol_for(&child, within, line_index, encoding) {
            Some(symbol) => out.push(symbol),
            None => collect_document_symbols(&child, within, line_index, out, encoding),
        }
    }
}

/// Build the [`DocumentSymbol`] for `node` when it is an assignment binding a
/// known name, else `None`. The full range is the whole assignment statement; the
/// selection range is the defining identifier; the kind is `FUNCTION` when the
/// value is a function/lambda, else `VARIABLE`. Children are the symbols nested in
/// the value side.
#[expect(deprecated, reason = "DocumentSymbol::deprecated is a required field")]
fn document_symbol_for(
    node: &SyntaxNode,
    bindings: &[&Binding],
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Option<DocumentSymbol> {
    let assign = AssignmentExpr::cast(node.clone())?;
    let name_token = assign.target_name_token()?;
    let name_range = name_token.text_range();
    let binding_index = bindings
        .binary_search_by_key(&name_range.start(), |b| b.def_range.start())
        .ok()?;
    let binding = bindings[binding_index];
    if binding.def_range != name_range {
        return None;
    }
    let value = assign.value_element();
    let is_function =
        matches!(&value, Some(NodeOrToken::Node(n)) if FunctionExpr::can_cast(n.kind()));

    // Nested bindings live in the value side (a function body, or any expression
    // that itself contains assignments). The target side binds no further names.
    let mut children = Vec::new();
    if let Some(NodeOrToken::Node(value_node)) = &value {
        collect_document_symbols(value_node, bindings, line_index, &mut children, encoding);
    }

    Some(DocumentSymbol {
        name: binding.name.to_string(),
        detail: None,
        kind: if is_function {
            LspSymbolKind::FUNCTION
        } else {
            LspSymbolKind::VARIABLE
        },
        tags: None,
        deprecated: None,
        range: text_range_to_lsp_range(line_index, node.text_range(), encoding),
        selection_range: text_range_to_lsp_range(line_index, name_token.text_range(), encoding),
        children: (!children.is_empty()).then_some(children),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_symbols_preserve_source_order_nesting_and_spans() {
        let outer = "outer <- function(param = (default <- 1)) {\n\
            for (i in 1:2) { if (param) `café` <<- i }\n\
            x[idx <- 1] <- 2\n\
            { inner <- 3 } -> right\n\
            \"名\" <- 4\n\
        }";
        let text = format!("#' 😀 Documentation.\nquote(hidden <- 0)\n{outer}\nlast <- 5\n");
        let expected = [
            (0, "outer", "outer", outer, LspSymbolKind::FUNCTION),
            (
                1,
                "default",
                "default",
                "default <- 1",
                LspSymbolKind::VARIABLE,
            ),
            (
                1,
                "`café`",
                "`café`",
                "`café` <<- i",
                LspSymbolKind::VARIABLE,
            ),
            (1, "idx", "idx", "idx <- 1", LspSymbolKind::VARIABLE),
            (
                1,
                "right",
                "right",
                "{ inner <- 3 } -> right",
                LspSymbolKind::VARIABLE,
            ),
            (2, "inner", "inner", "inner <- 3", LspSymbolKind::VARIABLE),
            (1, "名", "\"名\"", "\"名\" <- 4", LspSymbolKind::VARIABLE),
            (0, "last", "last", "last <- 5", LspSymbolKind::VARIABLE),
        ];
        fn flatten<'a>(
            symbols: &'a [DocumentSymbol],
            depth: usize,
            out: &mut Vec<(usize, &'a DocumentSymbol)>,
        ) {
            for symbol in symbols {
                out.push((depth, symbol));
                if let Some(children) = &symbol.children {
                    flatten(children, depth + 1, out);
                }
            }
        }

        let index = LineIndex::new(&text);
        for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            let symbols = compute_document_symbols(&text, encoding);
            let mut actual = Vec::new();
            flatten(&symbols, 0, &mut actual);
            assert_eq!(actual.len(), expected.len(), "{actual:#?}");
            for ((depth, symbol), (want_depth, name, token, statement, kind)) in
                actual.into_iter().zip(expected)
            {
                assert_eq!(
                    (depth, symbol.name.as_str(), symbol.kind),
                    (want_depth, name, kind)
                );
                let start = text.find(statement).unwrap();
                let name_start = start + statement.rfind(token).unwrap();
                assert_eq!(
                    symbol.range,
                    Range::new(
                        index.byte_to_position(start, encoding),
                        index.byte_to_position(start + statement.len(), encoding),
                    ),
                );
                assert_eq!(
                    symbol.selection_range,
                    Range::new(
                        index.byte_to_position(name_start, encoding),
                        index.byte_to_position(name_start + token.len(), encoding),
                    ),
                );
            }
        }
    }

    #[test]
    fn cached_document_symbols_match_fresh_analysis_after_edits() {
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(test_path(), "old <- 0\n");
        let _ = db.semantic_model(file);
        // The cached tree may use the package's markdown mode while the cold
        // helper uses the loose-file default. Neither can change the outline.
        db.set_roxygen_markdown(file, true);
        for text in [
            "#' A **documented** function.\n`f f` <- function(x) {\n  y <- x\n  if (x) z <- y\n}\n",
            "label <- '😀'; 1 -> café\nf <- function(x) { x <<- 1; y <- x }\n",
            "f <- function(x) { y <- x;\n",
            "print(1)\n",
        ] {
            let buffer = buf(text);
            for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
                let expected = compute_document_symbols(text, encoding);
                assert_eq!(
                    document_symbols_via_db(&db.snapshot(), test_path(), &buffer, encoding),
                    expected,
                    "stale cache",
                );
                assert_eq!(
                    document_symbols_via_db(
                        &IncrementalDatabase::default().snapshot(),
                        test_path(),
                        &buffer,
                        encoding,
                    ),
                    expected,
                    "missing file",
                );
            }
            db.set_file_text(file, buffer.text_arc());
            let _ = db.semantic_model(file);
            db.clear_query_log();
            for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
                assert_eq!(
                    document_symbols_via_db(&db.snapshot(), test_path(), &buffer, encoding),
                    compute_document_symbols(text, encoding),
                    "warm cache",
                );
            }
            assert!(db.query_log().is_empty());
        }
    }
}
