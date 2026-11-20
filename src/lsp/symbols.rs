use super::*;
use crate::semantic::Binding;
use crate::syntax::SyntaxElement;
use lsp_types::SymbolInformation;

/// Adapt the cached outline without cloning its tree for hierarchical clients.
/// Both response shapes serialize on the read pool.
pub(crate) fn document_symbol_response<'a>(
    symbols: &'a [DocumentSymbol],
    uri: &Uri,
    hierarchical: bool,
) -> impl Serialize + 'a {
    #[derive(Serialize)]
    #[serde(untagged)]
    enum SymbolResponse<'a> {
        Nested(&'a [DocumentSymbol]),
        Flat(Vec<SymbolInformation>),
    }

    if hierarchical {
        SymbolResponse::Nested(symbols)
    } else {
        let mut flat = Vec::new();
        collect_flat_symbols(symbols, uri, None, &mut flat);
        SymbolResponse::Flat(flat)
    }
}

#[expect(
    deprecated,
    reason = "SymbolInformation::deprecated is a required field"
)]
fn collect_flat_symbols(
    symbols: &[DocumentSymbol],
    uri: &Uri,
    container: Option<&str>,
    out: &mut Vec<SymbolInformation>,
) {
    for symbol in symbols {
        out.push(SymbolInformation {
            name: symbol.name.clone(),
            kind: symbol.kind,
            tags: symbol.tags.clone(),
            deprecated: symbol.deprecated,
            location: Location::new(uri.clone(), symbol.selection_range),
            container_name: container.map(str::to_owned),
        });
        if let Some(children) = &symbol.children {
            collect_flat_symbols(children, uri, Some(&symbol.name), out);
        }
    }
}

/// Build the outline from cached syntax and semantics when they match the live
/// buffer, falling back to fresh analysis on a cache miss or cancellation.
pub(crate) fn document_symbols_via_db(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    encoding: PositionEncoding,
    s7_floor: Option<&crate::config::CompatVersion>,
) -> (Vec<DocumentSymbol>, bool) {
    let path = &crate::incremental::normalize_path(path);
    with_document_semantics(snapshot, path, buffer, s7_floor, |root, model| {
        // A binding candidate can appear or disappear after a config,
        // DESCRIPTION, or project scope change with the same buffer text.
        let (facts, sites) = crate::project::classes::class_source_sites(model, root);
        let has_objects = sites.has_declarations();
        let cache_by_text = !has_objects
            && !root
                .descendants()
                .any(|node| crate::semantic::s7::binding_parts(&node).is_some());
        let symbols = document_symbols_from_model(root, model, buffer.line_index(), encoding);
        let declarations = if has_objects {
            let index = salsa::Cancelled::catch(AssertUnwindSafe(|| {
                snapshot.class_index_in(path, &facts, root, model)
            }))
            .unwrap_or_else(|_| {
                IncrementalDatabase::default()
                    .snapshot()
                    .class_index_in(path, &facts, root, model)
            });
            oop::presentation::declarations(root, model, &facts, &sites, &index, path)
        } else {
            Vec::new()
        };
        (
            add_oop_symbols(symbols, declarations, buffer.line_index(), encoding),
            cache_by_text,
        )
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
    let model = crate::project::classes::standalone_semantic_model(&root);
    let symbols = document_symbols_from_model(&root, &model, buffer.line_index(), encoding);
    let (facts, sites) = crate::project::classes::class_source_sites(&model, &root);
    let index = IncrementalDatabase::default().snapshot().class_index_in(
        Path::new("buffer.R"),
        &facts,
        &root,
        &model,
    );
    let declarations = oop::presentation::declarations(
        &root,
        &model,
        &facts,
        &sites,
        &index,
        Path::new("buffer.R"),
    );
    add_oop_symbols(symbols, declarations, buffer.line_index(), encoding)
}

#[expect(deprecated, reason = "DocumentSymbol::deprecated is a required field")]
fn add_oop_symbols(
    mut symbols: Vec<DocumentSymbol>,
    mut declarations: Vec<oop::presentation::Declaration>,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Vec<DocumentSymbol> {
    declarations.sort_by_key(|decl| (decl.range.start(), std::cmp::Reverse(decl.range.end())));
    for decl in declarations {
        let range = text_range_to_lsp_range(line_index, decl.range, encoding);
        let selection_range = text_range_to_lsp_range(line_index, decl.selection, encoding);
        let binding = decl
            .binding
            .map(|range| text_range_to_lsp_range(line_index, range, encoding));
        insert_oop_symbol(
            &mut symbols,
            DocumentSymbol {
                name: decl.name,
                detail: Some(decl.detail),
                kind: decl.kind,
                tags: None,
                deprecated: None,
                range,
                selection_range,
                children: None,
            },
            binding,
        );
    }
    symbols
}

fn insert_oop_symbol(
    symbols: &mut Vec<DocumentSymbol>,
    mut symbol: DocumentSymbol,
    binding: Option<Range>,
) {
    if let Some(existing) = symbols
        .iter_mut()
        .find(|existing| Some(existing.selection_range) == binding)
    {
        symbol.children = existing.children.take();
        *existing = symbol;
        return;
    }
    let contains = |outer: Range, inner: Range| {
        outer != inner && outer.start <= inner.start && outer.end >= inner.end
    };
    if let Some(parent) = symbols
        .iter_mut()
        .find(|parent| contains(parent.range, symbol.range))
    {
        insert_oop_symbol(
            parent.children.get_or_insert_with(Vec::new),
            symbol,
            binding,
        );
        return;
    }
    let mut children = Vec::new();
    symbols.retain_mut(|existing| {
        if contains(symbol.range, existing.range) {
            children.push(existing.clone());
            false
        } else {
            true
        }
    });
    symbol.children = (!children.is_empty()).then_some(children);
    let index = symbols.partition_point(|existing| existing.range.start <= symbol.range.start);
    symbols.insert(index, symbol);
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
    collect_document_symbols(root, &bindings, line_index, &mut symbols, encoding, model);
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
    model: &SemanticModel,
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
        match document_symbol_for(&child, within, line_index, encoding, model) {
            Some(symbol) => out.push(symbol),
            None => collect_document_symbols(&child, within, line_index, out, encoding, model),
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
    model: &SemanticModel,
) -> Option<DocumentSymbol> {
    let (name_token, value) = if let Some(assign) = AssignmentExpr::cast(node.clone()) {
        (assign.target_name_token()?, assign.value_element())
    } else if model.s7_bindings.contains(&node.text_range()) {
        let (target, call) = crate::semantic::s7::binding_parts(node)?;
        (target, Some(call.syntax().clone().into()))
    } else {
        return None;
    };
    let name_range = name_token.text_range();
    let binding_index = bindings
        .binary_search_by_key(&name_range.start(), |b| b.def_range.start())
        .ok()?;
    let binding = bindings[binding_index];
    if binding.def_range != name_range {
        return None;
    }
    let function = value
        .as_ref()
        .and_then(|value| value.as_node())
        .and_then(|node| FunctionExpr::cast(node.clone()));

    // Nested bindings live in the value side (a function body, or any expression
    // that itself contains assignments). The target side binds no further names.
    let mut children = Vec::new();
    if let Some(NodeOrToken::Node(value_node)) = &value {
        collect_document_symbols(
            value_node,
            bindings,
            line_index,
            &mut children,
            encoding,
            model,
        );
    }

    Some(DocumentSymbol {
        name: binding.name.to_string(),
        detail: function.as_ref().and_then(function_detail),
        kind: if function.is_some() {
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

fn function_detail(function: &FunctionExpr) -> Option<String> {
    // An unfinished parameter list must not look like a known empty signature.
    function.rparen_index()?;
    let args = function
        .formals()
        .into_iter()
        .map(|formal| match formal.default() {
            Some(default) => format!("{} = {}", formal.name(), default_preview(default)),
            None => formal.name().to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("({args})"))
}

fn default_preview(default: SyntaxElement) -> String {
    let mut out = String::new();
    let mut space = false;
    let mut push_token = |token: SyntaxToken<RLanguage>| {
        match token.kind() {
            SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT => space = true,
            kind if kind.is_roxygen_token() => space = true,
            _ => {
                if space && !out.is_empty() {
                    out.push(' ');
                }
                // Only trivia is compacted: whitespace inside a string or a
                // backtick name is part of its value and must survive intact.
                out.push_str(token.text());
                space = false;
            }
        }
    };
    match default {
        NodeOrToken::Token(token) => push_token(token),
        NodeOrToken::Node(node) => {
            for token in node
                .descendants_with_tokens()
                .filter_map(NodeOrToken::into_token)
            {
                push_token(token);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oop_outline_includes_members_and_unassigned_registrations() {
        let text = "G <- R6::R6Class('Display', public = list(value = 1, read = function() self$value))\nmethods::setGeneric('inspect', function(x) standardGeneric('inspect'))\nmethods::setMethod('inspect', 'Display', function(x) x)";
        let symbols = compute_document_symbols(text, PositionEncoding::Utf16);
        assert_eq!(
            symbols.iter().map(|s| s.kind).collect::<Vec<_>>(),
            [
                LspSymbolKind::CLASS,
                LspSymbolKind::FUNCTION,
                LspSymbolKind::METHOD,
            ]
        );
        let members = symbols[0].children.as_ref().unwrap();
        assert_eq!(
            members
                .iter()
                .map(|s| (s.name.as_str(), s.kind))
                .collect::<Vec<_>>(),
            [
                ("value", LspSymbolKind::FIELD),
                ("read", LspSymbolKind::METHOD),
            ]
        );
    }

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
            "#' A **documented** function.\n`f f` <- function(x = 1) {\n  y <- x\n  if (x) z <- y\n}\n",
            "#' A **documented** function.\n`f f` <- function(x = c(1, 2)) {\n  y <- x\n  if (x) z <- y\n}\n",
            "label <- '😀'; 1 -> café\nf <- function(x) { x <<- 1; y <- x }\n",
            "f <- function(x) { y <- x;\n",
            "print(1)\n",
        ] {
            let buffer = buf(text);
            for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
                let expected = compute_document_symbols(text, encoding);
                assert_eq!(
                    document_symbols_via_db(&db.snapshot(), test_path(), &buffer, encoding, None).0,
                    expected,
                    "stale cache",
                );
                assert_eq!(
                    document_symbols_via_db(
                        &IncrementalDatabase::default().snapshot(),
                        test_path(),
                        &buffer,
                        encoding,
                        None,
                    )
                    .0,
                    expected,
                    "missing file",
                );
            }
            db.set_file_text(file, buffer.text_arc());
            let _ = db.semantic_model(file);
            db.clear_query_log();
            for encoding in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
                assert_eq!(
                    document_symbols_via_db(&db.snapshot(), test_path(), &buffer, encoding, None).0,
                    compute_document_symbols(text, encoding),
                    "warm cache",
                );
            }
            assert!(db.query_log().is_empty());
        }
    }
}
