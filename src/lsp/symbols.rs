use super::*;

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
    // Name keyed by the defining identifier's span: an assignment is a symbol iff
    // its target token range is a key here. Using the model's name (not the raw
    // token text) yields the unquoted form for backtick/string targets.
    let bindings: HashMap<TextRange, SmolStr> = model
        .bindings()
        .iter()
        .filter(|b| matches!(b.kind, BindingKind::Local | BindingKind::Implicit))
        .map(|b| (b.def_range, b.name.clone()))
        .collect();
    let line_index = buffer.line_index();
    let mut symbols = Vec::new();
    collect_document_symbols(&root, &bindings, line_index, &mut symbols, encoding);
    symbols
}

/// Walk `node`'s child nodes, emitting a [`DocumentSymbol`] for each assignment
/// whose target is a known binding (recursing into its value for nested symbols)
/// and descending through every other node. Descending into non-binding nodes is
/// what lets a binding nested in an `if`/`for`/`{}` (none of which introduce a
/// symbol of their own) surface at the right level instead of being dropped.
pub(crate) fn collect_document_symbols(
    node: &SyntaxNode,
    bindings: &HashMap<TextRange, SmolStr>,
    line_index: &LineIndex,
    out: &mut Vec<DocumentSymbol>,
    encoding: PositionEncoding,
) {
    for child in node.children() {
        match document_symbol_for(&child, bindings, line_index, encoding) {
            Some(symbol) => out.push(symbol),
            None => collect_document_symbols(&child, bindings, line_index, out, encoding),
        }
    }
}

/// Build the [`DocumentSymbol`] for `node` when it is an assignment binding a
/// known name, else `None`. The full range is the whole assignment statement; the
/// selection range is the defining identifier; the kind is `FUNCTION` when the
/// value is a function/lambda, else `VARIABLE`. Children are the symbols nested in
/// the value side.
#[expect(deprecated, reason = "DocumentSymbol::deprecated is a required field")]
pub(crate) fn document_symbol_for(
    node: &SyntaxNode,
    bindings: &HashMap<TextRange, SmolStr>,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Option<DocumentSymbol> {
    let assign = AssignmentExpr::cast(node.clone())?;
    let name_token = assign.target_name_token()?;
    let name = bindings.get(&name_token.text_range())?;
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
        name: name.to_string(),
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
