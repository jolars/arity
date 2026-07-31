use super::*;

/// Resolve `workspace/symbol`: the workspace's top-level definitions whose name
/// fuzzily matches `query`, as modern [`WorkspaceSymbol`]s carrying a full
/// [`Location`]. An empty query returns every top-level symbol; the client
/// re-filters and ranks, so the server only needs a lenient filter. Returns an
/// empty vec when no workspace is seeded (single-file mode). Snapshot reads are
/// wrapped in [`salsa::Cancelled::catch`] (as hover wraps its read).
///
/// Scope is **file-scope top-level definitions only** — that is what the
/// [`project_defs`](crate::project::project_defs) index holds and the right
/// granularity for a project-wide search, unlike `textDocument/documentSymbol`
/// (see [`compute_document_symbols`]) which also surfaces nested locals.
pub(crate) fn workspace_symbols_via_db(
    snapshot: &Analysis,
    query: &str,
    encoding: PositionEncoding,
) -> Vec<WorkspaceSymbol> {
    let needle = query.to_lowercase();
    let hits = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot.workspace_symbols(|name| fuzzy_subsequence(&needle, name))
    }))
    .unwrap_or_default();
    hits.into_iter()
        .filter_map(|(name, kind, path, range)| {
            let location = location_in(snapshot, &path, range, encoding)?;
            Some(WorkspaceSymbol {
                name,
                kind: match kind {
                    DefKind::Function => LspSymbolKind::FUNCTION,
                    DefKind::Value => LspSymbolKind::VARIABLE,
                },
                tags: None,
                container_name: None,
                location: OneOf::Left(location),
                data: None,
            })
        })
        .collect()
}

/// Whether the lowercased `query` is a subsequence of `name` (matched
/// case-insensitively): its characters appear in `name` in order, not
/// necessarily contiguously. An empty query matches everything. Dependency-free
/// and lenient by design — the client does the final ranking.
fn fuzzy_subsequence(query: &str, name: &str) -> bool {
    let mut q = query.chars().peekable();
    let mut name_chars = name.chars().flat_map(char::to_lowercase);
    while let Some(&target) = q.peek() {
        match name_chars.next() {
            // The query char is matched: advance to the next one.
            Some(ch) if ch == target => {
                q.next();
            }
            // This name char doesn't match the pending query char: skip it.
            Some(_) => {}
            // Name exhausted with query chars still pending: no match.
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed a two-file workspace and return a read snapshot. Mirrors the
    /// `navigation` tests' setup.
    fn workspace(a_src: &str, b_src: &str) -> Analysis {
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&ws_path("a.R"), a_src.to_string());
        let b = db.upsert_file(&ws_path("b.R"), b_src.to_string());
        db.set_workspace_members(vec![a, b], vec![ws_root()]);
        db.snapshot()
    }

    fn names(symbols: &[WorkspaceSymbol]) -> Vec<&str> {
        symbols.iter().map(|s| s.name.as_str()).collect()
    }

    fn location_of<'a>(symbols: &'a [WorkspaceSymbol], name: &str) -> &'a Location {
        let symbol = symbols
            .iter()
            .find(|s| s.name == name)
            .expect("symbol present");
        match &symbol.location {
            OneOf::Left(location) => location,
            OneOf::Right(_) => panic!("expected a full Location, not a uri-only WorkspaceLocation"),
        }
    }

    #[test]
    fn fuzzy_subsequence_matches_in_order() {
        assert!(fuzzy_subsequence("", "anything"));
        assert!(fuzzy_subsequence("foo", "foo"));
        assert!(fuzzy_subsequence("fb", "foobar")); // non-contiguous
        assert!(fuzzy_subsequence("foo", "FOObar")); // case-insensitive name
        assert!(!fuzzy_subsequence("bf", "foobar")); // wrong order
        assert!(!fuzzy_subsequence("xyz", "foobar"));
        assert!(!fuzzy_subsequence("fooo", "foo")); // query longer than match run
    }

    #[test]
    fn exact_hit_across_files() {
        let snapshot = workspace("foo <- function() 1\n", "bar <- 2\n");
        let symbols = workspace_symbols_via_db(&snapshot, "foo", PositionEncoding::Utf16);
        assert_eq!(names(&symbols), ["foo"]);
        assert_eq!(symbols[0].kind, LspSymbolKind::FUNCTION);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        assert_eq!(location_of(&symbols, "foo").uri, uri_a);
    }

    #[test]
    fn fuzzy_subsequence_query_hits_a_value() {
        let snapshot = workspace("foo <- function() 1\n", "bar <- 2\n");
        let symbols = workspace_symbols_via_db(&snapshot, "br", PositionEncoding::Utf16);
        assert_eq!(names(&symbols), ["bar"]);
        assert_eq!(symbols[0].kind, LspSymbolKind::VARIABLE);
    }

    #[test]
    fn empty_query_returns_all_top_level_defs() {
        let snapshot = workspace("foo <- function() 1\n", "bar <- 2\n");
        let symbols = workspace_symbols_via_db(&snapshot, "", PositionEncoding::Utf16);
        let mut got = names(&symbols);
        got.sort_unstable();
        assert_eq!(got, ["bar", "foo"]);
    }

    #[test]
    fn no_workspace_returns_empty() {
        // A pathless in-memory buffer with no seeded workspace.
        let mut db = IncrementalDatabase::default();
        db.upsert_file(&ws_path("a.R"), "foo <- function() 1\n".to_string());
        let snapshot = db.snapshot();
        assert!(workspace_symbols_via_db(&snapshot, "foo", PositionEncoding::Utf16).is_empty());
    }

    #[test]
    fn location_indexes_the_defining_identifier() {
        let a_src = "x <- 1\nfoo <- function() 1\n";
        let snapshot = workspace(a_src, "bar <- 2\n");
        let symbols = workspace_symbols_via_db(&snapshot, "foo", PositionEncoding::Utf16);
        let location = location_of(&symbols, "foo");
        // `foo` is defined on the second line, starting at column 0.
        assert_eq!(location.range.start.line, 1);
        assert_eq!(location.range.start.character, 0);
        assert_eq!(location.range.end.character, 3);
    }

    #[test]
    fn nested_locals_are_excluded() {
        // `inner` is a local inside `outer`'s body, not a file-scope binding, so
        // it must not surface as a workspace symbol.
        let snapshot = workspace(
            "outer <- function() {\n  inner <- 1\n  inner\n}\n",
            "z <- 2\n",
        );
        let symbols = workspace_symbols_via_db(&snapshot, "inner", PositionEncoding::Utf16);
        let got = names(&symbols);
        assert!(
            got.is_empty(),
            "nested local leaked into workspace symbols: {got:?}"
        );
    }
}
