use super::*;
use crate::syntax::SyntaxElement;

/// Resolve hover off the snapshot's cached parse when the db's tracked buffer for
/// `path` still matches `text`; otherwise re-parse. Falls back on cancellation.
pub(crate) fn hover_via_db(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    position: Position,
    encoding: PositionEncoding,
    s7_floor: Option<&crate::config::CompatVersion>,
) -> Option<Hover> {
    let text = buffer.text();
    let line_index = buffer.line_index();
    let offset = line_index
        .position_to_byte(position, encoding)
        .min(text.len());
    // Read the harvested index from the same snapshot, so hover sees exactly the
    // index the lint thread last installed. An empty index (none installed yet)
    // still resolves base-R + bundled names via the static layers.
    let index = snapshot.library_data().unwrap_or_default();
    // A `DESCRIPTION` hovers packages, not symbols, and has no `SourceFile` in
    // the db whose parse could be reused — so it branches before the lookup.
    if DocumentKind::from_path(path) == DocumentKind::Description {
        return compute_description_hover(text, offset, &index, line_index, encoding);
    }
    let path = &crate::incremental::normalize_path(path);
    let context = Some(oop::ReadContext {
        snapshot,
        path,
        s7_floor,
    });
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if !snapshot.file_text_is(file, &buffer.text_arc()) {
            return None;
        }
        let root = snapshot.parsed_tree(file);
        Some(hover_from_node(
            &root, line_index, offset, &index, encoding, context,
        ))
    }));
    match cached {
        Ok(Some(hover)) => hover,
        Ok(None) | Err(_) => {
            let root = parse(text).cst;
            hover_from_node(&root, line_index, offset, &index, encoding, context)
        }
    }
}

/// The symbol referenced at a cursor position: either a namespaced access
/// (`pkg::name`) whose package is known directly, or a bare name whose package
/// must be resolved against the attached packages.
pub(crate) enum SymbolQuery {
    Namespaced {
        package: SmolStr,
        name: SmolStr,
        range: TextRange,
    },
    Bare {
        name: SmolStr,
        range: TextRange,
    },
}

/// Build hover contents for the symbol at byte `offset`, if it resolves to an
/// indexed package export. Pure (parses `text` itself) so it is unit-testable.
pub fn compute_hover(
    text: &str,
    offset: usize,
    indexed: &IndexedProvider,
    encoding: PositionEncoding,
) -> Option<Hover> {
    let root = parse(text).cst;
    let line_index = LineIndex::new(text);
    hover_from_node(
        &root,
        &line_index,
        offset.min(text.len()),
        indexed,
        encoding,
        None,
    )
}

/// Build hover contents off an already-parsed CST (and a matching line index),
/// without re-parsing. The LSP read path uses this against the cached parse tree
/// in its salsa database; [`compute_hover`] is the parse-from-text wrapper.
fn hover_from_node(
    root: &SyntaxNode,
    line_index: &LineIndex,
    offset: usize,
    indexed: &IndexedProvider,
    encoding: PositionEncoding,
    context: Option<oop::ReadContext<'_>>,
) -> Option<Hover> {
    let offset = TextSize::new(offset as u32);
    if let Some(hover) = oop_hover(root, offset, indexed, context) {
        let (range, value) = hover;
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: Some(text_range_to_lsp_range(line_index, range, encoding)),
        });
    }
    let query = symbol_query_at(root, offset)?;
    let (package, entry, range) = resolve_query(query, root, indexed)?;

    let lsp_range = Range {
        start: line_index.byte_to_position(u32::from(range.start()) as usize, encoding),
        end: line_index.byte_to_position(u32::from(range.end()) as usize, encoding),
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: render_hover_markdown(&package, entry),
        }),
        range: Some(lsp_range),
    })
}

fn oop_hover(
    root: &SyntaxNode,
    offset: TextSize,
    indexed: &IndexedProvider,
    context: Option<oop::ReadContext<'_>>,
) -> Option<(TextRange, String)> {
    use crate::project::classes::{EditorRead, OopTarget, class_source_sites};
    use oop::presentation::{declarations, member_kind, signature, source_role};

    let token = pick_name_token(root, offset).or_else(|| {
        root.token_at_offset(offset)
            .find(|token| token.kind() == SyntaxKind::STRING)
    })?;
    if let Some(namespace) = token
        .parent()
        .and_then(BinaryExpr::cast)
        .and_then(|binary| binary.namespace_access())
        && namespace.name_token != token
    {
        return None;
    }
    let range = token.text_range();
    let remote = context
        .and_then(|context| context.snapshot.remote_exports())
        .unwrap_or_default();
    oop::with_live_model(root, context, |snapshot, model, path| {
        let (facts, sites) = class_source_sites(model, root);
        let index = snapshot.class_index_for_presentation(path, &facts, root, model);
        for decl in declarations(root, model, &facts, &sites, &index, path) {
            if decl.selection == range || decl.binding == Some(range) {
                return Some((range, format!("{} · `{}`", decl.detail, decl.name)));
            }
        }
        if let Some((_, name, detail)) = source_role(&index, path, &sites, range) {
            return Some((range, format!("{detail} · `{name}`")));
        }

        let read = EditorRead::new(&index, root, model, path, indexed, &remote);
        let value = SyntaxElement::Token(token.clone());
        let binary = token.parent().and_then(BinaryExpr::cast);
        let (label, sig, help) = if let Some(binary) = &binary
            && binary.rhs().as_ref() == Some(&value)
            && matches!(binary.op_kind(), Some(SyntaxKind::DOLLAR | SyntaxKind::AT))
        {
            let name = if token.kind() == SyntaxKind::STRING {
                crate::semantic::decode_string_literal(token.text())?
            } else {
                crate::semantic::symbols::unbacktick(token.text()).to_owned()
            };
            let member = read
                .members(&binary.lhs()?, binary.op_kind() == Some(SyntaxKind::AT))?
                .members
                .remove(&name)?;
            let role = match member_kind(member.kind) {
                LspSymbolKind::METHOD => "method",
                LspSymbolKind::FIELD => "field",
                _ => "property",
            };
            let mut label = format!("{role} `{name}`");
            if let Some(ty) = member.declared_type {
                label.push_str(&format!(" · {ty}"));
            }
            (label, member.signature, member.help)
        } else {
            let value = if let Some(binary) = &binary
                && binary.namespace_access().is_some()
                && binary.rhs().as_ref() == Some(&value)
            {
                binary.syntax().clone().into()
            } else {
                value
            };
            let class_argument = sites.oop.iter().any(|source| {
                source.site.range == range
                    && matches!(source.site.target, OopTarget::ClassReference(_))
            });
            let class = || {
                if class_argument && token.kind() == SyntaxKind::STRING {
                    let call = token
                        .parent_ancestors()
                        .find(|node| CallExpr::can_cast(node.kind()))?;
                    read.class_hover(&call.into())
                } else {
                    read.class_hover(&value)
                }
            };
            if token.kind() == SyntaxKind::STRING && !class_argument {
                return None;
            }
            class()
                .or_else(|| read.generic_hover(&value))
                .or_else(|| read.method_hover(&value))
                .or_else(|| {
                    let call = value.parent().and_then(CallExpr::cast)?;
                    if call.base().as_ref() != Some(&value) {
                        return None;
                    }
                    let callable = read.callable(&call)?;
                    if callable.help.is_none()
                        && let Some(entry) = indexed.lookup("methods", "new")
                        && entry.help.is_some()
                        && read.is_formal_constructor(&call)
                    {
                        let mut markdown = render_hover_markdown("methods", entry);
                        if signature_of(entry).is_none() {
                            markdown.insert_str(
                                0,
                                &format!("`{}`\n\n", signature(&callable.signature)),
                            );
                        }
                        return Some((markdown, None, None));
                    }
                    Some((callable.name, Some(callable.signature), callable.help))
                })?
        };
        let mut markdown = label;
        if let Some(sig) = sig {
            markdown.push_str(&format!("\n\n`{}`", signature(&sig)));
        }
        if let Some(doc) = help.as_ref().and_then(|help| help.resolve(indexed))
            && let Some(Documentation::MarkupContent(doc)) = signature::help_documentation(doc, &[])
        {
            markdown.push_str("\n\n");
            markdown.push_str(&doc.value);
        }
        Some((range, markdown))
    })
}

/// Classify the name token under the cursor, distinguishing `pkg::name` from a
/// bare reference. Returns `None` when the cursor isn't on a name.
pub(crate) fn symbol_query_at(root: &SyntaxNode, offset: TextSize) -> Option<SymbolQuery> {
    let token = pick_name_token(root, offset)?;
    for ancestor in token.parent_ancestors() {
        if let Some(access) = BinaryExpr::cast(ancestor).and_then(|b| b.namespace_access())
            && access.name_token == token
        {
            return Some(SymbolQuery::Namespaced {
                package: access.package,
                name: access.name,
                range: token.text_range(),
            });
        }
    }
    Some(SymbolQuery::Bare {
        name: SmolStr::new(token.text()),
        range: token.text_range(),
    })
}

/// The `IDENT`/`USER_OP` token at `offset`, preferring the right side when the
/// cursor sits exactly between two tokens.
pub(crate) fn pick_name_token(
    root: &SyntaxNode,
    offset: TextSize,
) -> Option<SyntaxToken<RLanguage>> {
    let is_name = |k: SyntaxKind| matches!(k, SyntaxKind::IDENT | SyntaxKind::USER_OP);
    match root.token_at_offset(offset) {
        TokenAtOffset::None => None,
        TokenAtOffset::Single(t) => is_name(t.kind()).then_some(t),
        TokenAtOffset::Between(left, right) => {
            if is_name(right.kind()) {
                Some(right)
            } else if is_name(left.kind()) {
                Some(left)
            } else {
                None
            }
        }
    }
}

/// Resolve a [`SymbolQuery`] to the indexed entry that documents it.
pub(crate) fn resolve_query<'p>(
    query: SymbolQuery,
    root: &SyntaxNode,
    indexed: &'p IndexedProvider,
) -> Option<(SmolStr, &'p SymbolEntry, TextRange)> {
    match query {
        SymbolQuery::Namespaced {
            package,
            name,
            range,
        } => {
            let entry = indexed.lookup(&package, &name)?;
            Some((package, entry, range))
        }
        SymbolQuery::Bare { name, range } => {
            let model = SemanticModel::build(root);
            let remote = RemoteExports::new();
            let package = match resolve_origin(indexed, &remote, &name, model.loaded_packages()) {
                PackageOrigin::Resolved(p) => p,
                // The last attacher masks the rest under R's lookup rules.
                PackageOrigin::Ambiguous(mut v) => v.pop()?,
                PackageOrigin::Unknown => return None,
            };
            let entry = indexed.lookup(&package, &name)?;
            Some((package, entry, range))
        }
    }
}

/// Render a symbol's signature + help into hover markdown.
pub(crate) fn render_hover_markdown(package: &str, entry: &SymbolEntry) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // Signature: the `\usage` block if present, else a formals-derived call.
    if let Some(signature) = signature_of(entry) {
        let _ = write!(out, "```r\n{signature}\n```\n");
    }

    let kind = match entry.kind {
        SymbolKind::Function => "function",
        SymbolKind::Data => "data",
        SymbolKind::Other => "object",
    };
    let _ = write!(out, "`{package}::{}` · {kind}", entry.name);

    if let Some(help) = &entry.help {
        if let Some(title) = &help.title {
            let _ = write!(out, "\n\n**{title}**");
        }
        if let Some(description) = &help.description {
            let _ = write!(out, "\n\n{description}");
        }
        if !help.arguments.is_empty() {
            out.push_str("\n\n**Arguments**\n");
            for arg in &help.arguments {
                let _ = write!(out, "\n- `{}` — {}", arg.name, arg.description);
            }
        }
    }
    out
}

/// A symbol's call signature: the `\usage` block when help carries one, else a
/// formals-derived `name(args)`. Shared by hover and completion's `resolve`.
pub(crate) fn signature_of(entry: &SymbolEntry) -> Option<String> {
    let usage = entry.help.as_ref().and_then(|h| h.usage.as_deref());
    usage.map(str::to_string).or_else(|| {
        entry.formals.as_ref().map(|formals| {
            let args = formals
                .iter()
                .map(format_formal)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", entry.name, args)
        })
    })
}

pub(crate) fn format_formal(formal: &Formal) -> String {
    match &formal.default {
        Some(default) => format!("{} = {}", formal.name, default),
        None => formal.name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_describes_source_objects_and_uses_installed_object_help() {
        let src = "G <- R6::R6Class('Display', public = list(read = function(n = 1) n))\nobj <- G$new()\nobj$read()\nf <- function(x) UseMethod('f')\nf.Display <- function(x) x\nlabel <- 'Display'";
        let empty = IndexedProvider::empty();
        for (needle, expected) in [
            ("'Display'", "R6 class"),
            ("read =", "R6 method (n = 1)"),
            ("read()", "(n = 1)"),
            ("'f'", "S3 generic (x)"),
            ("f.Display", "S3 method for Display"),
        ] {
            let offset = src.find(needle).unwrap() + 1;
            let hover = compute_hover(src, offset, &empty, PositionEncoding::Utf16).unwrap();
            let HoverContents::Markup(markup) = hover.contents else {
                panic!("expected markdown")
            };
            assert!(
                markup.value.contains(expected),
                "{needle}: {}",
                markup.value
            );
        }
        assert!(
            compute_hover(
                src,
                src.rfind("Display").unwrap(),
                &empty,
                PositionEncoding::Utf16
            )
            .is_none()
        );

        use crate::rindex::harvest::{HarvestOptions, harvest_package};
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/oopfixture");
        let package = harvest_package(&fixture, HarvestOptions::default(), 0).unwrap();
        let indexed = IndexedProvider::from_indices([package]);
        let src = "obj <- oopfixture::Counter$new()\nobj$base_add()\noopfixture::area";
        for (needle, expected) in [
            ("Counter", "Documentation for fixture objects."),
            ("base_add", "Documentation for fixture objects."),
            ("area", "S4 generic"),
        ] {
            let markdown = hover_markdown(src, needle, &indexed).unwrap();
            assert!(markdown.contains(expected), "{needle}: {markdown}");
        }
    }

    #[test]
    fn hover_resolves_bare_name_via_attached_package() {
        let provider = documented_dplyr();
        let src = "library(dplyr)\nacross(a, mean)\n";
        let md = hover_markdown(src, "across(a", &provider).expect("hover for across");
        assert!(md.contains("across(.cols, .fns)"), "signature: {md}");
        assert!(md.contains("dplyr::across"), "origin: {md}");
        assert!(
            md.contains("Apply a function across columns"),
            "title: {md}"
        );
        assert!(md.contains("`.cols`"), "arguments: {md}");
    }

    #[test]
    fn hover_resolves_base_r_bare_name() {
        // Regression: base-R symbols resolve to package `base` via the static
        // name list, but hover also needs the harvested rich entry. Once `base`
        // is harvested, a bare `as.matrix` (no `library()`) hovers.
        use crate::rindex::schema::{HelpDoc, PackageIndex, SCHEMA_VERSION};
        let idx = PackageIndex {
            objects: Default::default(),
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
        let src = "x <- cbind(1:5, 6:10)\nas.matrix(x)\n";
        let md = hover_markdown(src, "as.matrix(x)", &provider).expect("hover for as.matrix");
        assert!(md.contains("as.matrix(x, ...)"), "signature: {md}");
        assert!(md.contains("base::as.matrix"), "origin: {md}");
        assert!(md.contains("Matrices"), "title: {md}");
    }

    #[test]
    fn s4_generic_result_alias_is_not_callable() {
        let src = "registration <- methods::setGeneric('say', function(x) standardGeneric('say'))
alias <- registration
alias()
installed <- say
installed(1)
";
        let indexed = IndexedProvider::empty();
        assert!(hover_markdown(src, "alias()", &indexed).is_none());
        assert!(hover_markdown(src, "installed(1)", &indexed).is_some());
    }

    #[test]
    fn constructor_hover_preserves_indexed_function_help() {
        use crate::rindex::schema::{HelpDoc, PackageIndex, SCHEMA_VERSION};
        let indexed = IndexedProvider::from_indices([PackageIndex {
            objects: Default::default(),
            schema_version: SCHEMA_VERSION,
            package: "methods".into(),
            version: "4.5.3".into(),
            lib_path: "/lib".into(),
            title: None,
            r_version: None,
            harvested_at: 0,
            attaches: Vec::new(),
            symbols: vec![SymbolEntry {
                name: "new".into(),
                kind: SymbolKind::Function,
                exported: true,
                formals: None,
                help: Some(HelpDoc {
                    title: Some("Generate an Object from a Class".into()),
                    description: None,
                    usage: Some("new(Class, ...)".into()),
                    arguments: vec![],
                }),
            }],
        }]);
        let markdown = hover_markdown("methods::new('environment')", "new", &indexed).unwrap();
        assert!(
            markdown.contains("Generate an Object from a Class"),
            "{markdown}"
        );
    }

    #[test]
    fn hover_resolves_namespaced_without_library() {
        let provider = documented_dplyr();
        // No `library(dplyr)`: the `pkg::name` form resolves directly.
        let src = "dplyr::across(a)\n";
        let md = hover_markdown(src, "across", &provider).expect("hover for dplyr::across");
        assert!(md.contains("dplyr::across"));
    }

    #[test]
    fn hover_none_for_unknown_and_non_name() {
        let provider = documented_dplyr();
        // `bogus` is not indexed by any attached package.
        assert!(compute_hover("bogus()\n", 1, &provider, PositionEncoding::Utf16).is_none());
        // Cursor on whitespace yields nothing.
        let src = "across (a)\n";
        assert!(
            compute_hover(
                src,
                offset_of(src, " (a"),
                &provider,
                PositionEncoding::Utf16
            )
            .is_none()
        );
    }

    #[test]
    fn hover_via_db_matches_compute() {
        use crate::incremental::IncrementalDatabase;
        let path = test_path();
        let src = "library(dplyr)\nacross(a, mean)\n";
        // Cursor on `across` (line 1, character 0).
        let position = pos(1, 0);

        // Hover reads the index from the snapshot, so it must be installed first.
        let mut db = IncrementalDatabase::default();
        db.set_library_index(documented_dplyr());
        db.upsert_file(path, src.to_string());
        let hover = hover_via_db(
            &db.snapshot(),
            path,
            &buf(src),
            position,
            PositionEncoding::Utf16,
            None,
        )
        .expect("hover for across via db");
        let md = match hover.contents {
            HoverContents::Markup(m) => m.value,
            other => panic!("expected markup, got {other:?}"),
        };
        assert!(md.contains("dplyr::across"), "origin: {md}");

        // Untracked path still resolves, via the fresh-parse fallback.
        let mut empty = IncrementalDatabase::default();
        empty.set_library_index(documented_dplyr());
        assert!(
            hover_via_db(
                &empty.snapshot(),
                path,
                &buf(src),
                position,
                PositionEncoding::Utf16,
                None,
            )
            .is_some(),
            "fallback hover should resolve too"
        );
    }
}
