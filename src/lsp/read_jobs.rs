use super::*;

/// Reuse the snapshot's syntax and semantics only while they describe the live
/// buffer. A missing file, stale text, or cancelled read falls back to fresh
/// analysis. The callback borrows the cached model without cloning it.
pub(crate) fn with_document_semantics<T>(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    s7_floor: Option<&crate::config::CompatVersion>,
    compute: impl Fn(&SyntaxNode, &SemanticModel) -> T,
) -> T {
    let cached = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let file = snapshot.lookup_file(path)?;
        if !snapshot.file_text_is(file, &buffer.text_arc()) {
            return None;
        }
        let root = snapshot.parsed_tree(file);
        let model = snapshot.runtime_semantic_model(file);
        if s7_floor.is_some_and(|floor| {
            model.s7_version != crate::semantic::s7::S7Version::from_floor(floor)
        }) {
            return None;
        }
        Some(compute(&root, model))
    }));
    if let Ok(Some(result)) = cached {
        return result;
    }
    let root = parse(buffer.text()).cst;
    let fresh = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        let model = if s7_floor.is_some() {
            snapshot.semantic_model_in_with_floor(path, &root, s7_floor)
        } else {
            snapshot.semantic_model_in(path, &root)
        };
        if !crate::project::classes::has_method_candidates(&model)
            && !model
                .scopes()
                .iter()
                .any(|scope| scope.kind == crate::semantic::ScopeKind::Function)
        {
            return compute(&root, &model);
        }
        let facts = crate::project::file_class_defs(&model, &root);
        let index = snapshot.class_index_in(path, &facts, &root, &model);
        let resolved = crate::project::classes::apply_method_environments(
            &model,
            &root,
            &index.method_environments(path),
        )
        .unwrap_or(model);
        let resolved = crate::project::classes::apply_dispatch_contexts(
            &resolved,
            &root,
            &index.dispatch_contexts(path),
        )
        .unwrap_or(resolved);
        compute(&root, &resolved)
    }));
    fresh.unwrap_or_else(|_| {
        let model = crate::project::classes::standalone_semantic_model_with_floor(&root, s7_floor);
        compute(&root, &model)
    })
}

/// A read-only request the lint thread services by cloning its salsa db and
/// running the work off-thread on the read pool. Each variant carries a shared
/// handle to the live buffer and the main loop's `out` channel, so the worker replies with an
/// [`Outbound::ReadReply`] that the loop gates (cancellation, stale version)
/// before it reaches the client; the lint thread only adds the db snapshot. See
/// [`run_read`].
pub(crate) enum ReadJob {
    Format {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        style: FormatStyle,
        out: Sender<Outbound>,
    },
    FormatRange {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        range: Range,
        style: FormatStyle,
        out: Sender<Outbound>,
    },
    SemanticTokens {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    Hover {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        position: Position,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    InlayHints {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        /// The client's visible range, honored verbatim.
        range: Range,
        out: Sender<Outbound>,
    },
    Completion {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        position: Position,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    SignatureHelp {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        position: Position,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    ResolveCompletion {
        id: RequestId,
        // Boxed: `CompletionItem` is large and would bloat every `ReadJob`.
        item: Box<CompletionItem>,
        out: Sender<Outbound>,
    },
    Definition {
        id: RequestId,
        path: PathBuf,
        /// The current document's URI — an intra-file hit reports a `Location`
        /// back into it, so unlike the other jobs this one needs the URI too.
        uri: Uri,
        buffer: Arc<TextBuffer>,
        position: Position,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    Implementation {
        id: RequestId,
        path: PathBuf,
        /// The current document's URI — an intra-file hit reports a `Location`
        /// back into it, so unlike the other jobs this one needs the URI too.
        uri: Uri,
        buffer: Arc<TextBuffer>,
        position: Position,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    References {
        id: RequestId,
        path: PathBuf,
        /// In-file reads report `Location`s back into this URI; cross-file reads
        /// carry their own.
        uri: Uri,
        buffer: Arc<TextBuffer>,
        position: Position,
        include_declaration: bool,
        open_namespaces: Vec<(PathBuf, Arc<TextBuffer>)>,
        out: Sender<Outbound>,
    },
    PrepareRename {
        id: RequestId,
        path: PathBuf,
        uri: Uri,
        buffer: Arc<TextBuffer>,
        offset: usize,
        peers: Vec<(PathBuf, Arc<TextBuffer>)>,
        out: Sender<Outbound>,
    },
    Rename {
        id: RequestId,
        path: PathBuf,
        /// In-file edits land in this URI; cross-file edits carry their own.
        uri: Uri,
        buffer: Arc<TextBuffer>,
        /// The cursor's byte offset, already resolved on the main thread (via the
        /// `prepareRename` anchor when present, else the request position).
        offset: usize,
        new_name: String,
        target: Option<ObjectRenameAnchor>,
        peers: Vec<(PathBuf, Arc<TextBuffer>)>,
        out: Sender<Outbound>,
    },
    WillRenameFiles {
        id: RequestId,
        /// `(old, new)` filesystem path pairs for the files being renamed.
        renames: Vec<(PathBuf, PathBuf)>,
        out: Sender<Outbound>,
    },
    DocumentSymbol {
        id: RequestId,
        path: PathBuf,
        uri: Uri,
        buffer: Arc<TextBuffer>,
        cached: Arc<OnceLock<Vec<DocumentSymbol>>>,
        hierarchical: bool,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    WorkspaceSymbol {
        id: RequestId,
        /// The fuzzy name filter; an empty string requests every symbol.
        query: String,
        out: Sender<Outbound>,
    },
    PrepareCallHierarchy {
        id: RequestId,
        path: PathBuf,
        /// An intra-file item reports back into this URI; cross-file items carry
        /// their own.
        uri: Uri,
        buffer: Arc<TextBuffer>,
        position: Position,
        s7_floor: Option<crate::config::CompatVersion>,
        out: Sender<Outbound>,
    },
    IncomingCalls {
        id: RequestId,
        /// The prepared item carries a source URI and a callable identity.
        item: Box<CallHierarchyItem>,
        out: Sender<Outbound>,
    },
    OutgoingCalls {
        id: RequestId,
        item: Box<CallHierarchyItem>,
        out: Sender<Outbound>,
    },
    PrepareTypeHierarchy {
        id: RequestId,
        path: PathBuf,
        /// An intra-file item reports back into this URI; cross-file items carry
        /// their own.
        uri: Uri,
        buffer: Arc<TextBuffer>,
        position: Position,
        out: Sender<Outbound>,
    },
    Supertypes {
        id: RequestId,
        /// The prepared item, round-tripped from the client; the target class is
        /// recovered from its `name`.
        item: Box<TypeHierarchyItem>,
        out: Sender<Outbound>,
    },
    Subtypes {
        id: RequestId,
        item: Box<TypeHierarchyItem>,
        out: Sender<Outbound>,
    },
}

/// Service a read-only job against a db `snapshot`, replying to the client.
/// Runs on a read-pool worker; the `snapshot` is dropped on return so it never
/// blocks the lint thread's next write longer than the job itself.
pub(crate) fn run_read(snapshot: Analysis, encoding: PositionEncoding, job: ReadJob) {
    match job {
        ReadJob::Format {
            id,
            path,
            buffer,
            style,
            out,
        } => {
            let result = format_edits_via_db(&snapshot, &path, &buffer, style, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::FormatRange {
            id,
            path,
            buffer,
            range,
            style,
            out,
        } => {
            let result =
                format_range_edits_via_db(&snapshot, &path, &buffer, range, style, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::SemanticTokens {
            id,
            path,
            buffer,
            s7_floor,
            out,
        } => {
            let tokens =
                semantic_tokens_via_db(&snapshot, &path, &buffer, encoding, s7_floor.as_ref());
            let result = SemanticTokensResult::Tokens(tokens);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Hover {
            id,
            path,
            buffer,
            position,
            s7_floor,
            out,
        } => {
            let result = hover_via_db(
                &snapshot,
                &path,
                &buffer,
                position,
                encoding,
                s7_floor.as_ref(),
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::InlayHints {
            id,
            path,
            buffer,
            range,
            out,
        } => {
            let result = inlay_hints_via_db(&snapshot, &path, &buffer, range, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Completion {
            id,
            path,
            buffer,
            position,
            s7_floor,
            out,
        } => {
            let result = completion_via_db(
                &snapshot,
                &path,
                &buffer,
                position,
                encoding,
                s7_floor.as_ref(),
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::SignatureHelp {
            id,
            path,
            buffer,
            position,
            s7_floor,
            out,
        } => {
            let result = signature_help_via_db(
                &snapshot,
                &path,
                &buffer,
                position,
                encoding,
                s7_floor.as_ref(),
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::ResolveCompletion { id, item, out } => {
            let result = resolve_completion(*item, &snapshot.library_data().unwrap_or_default());
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Definition {
            id,
            path,
            uri,
            buffer,
            position,
            s7_floor,
            out,
        } => {
            let result = definition_via_db(
                &snapshot,
                &path,
                &uri,
                &buffer,
                position,
                encoding,
                s7_floor.as_ref(),
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Implementation {
            id,
            path,
            uri,
            buffer,
            position,
            s7_floor,
            out,
        } => {
            let result = implementation_via_db(
                &snapshot,
                &path,
                &uri,
                &buffer,
                position,
                encoding,
                s7_floor.as_ref(),
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::References {
            id,
            path,
            uri,
            buffer,
            position,
            include_declaration,
            open_namespaces,
            out,
        } => {
            let result = references_via_db(
                &snapshot,
                &path,
                &uri,
                &buffer,
                position,
                include_declaration,
                encoding,
                &open_namespaces,
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::PrepareRename {
            id,
            path,
            uri,
            buffer,
            offset,
            peers,
            out,
        } => {
            let prepared = prepare_rename_via_db(&snapshot, &path, &uri, &buffer, offset, encoding)
                .filter(|prepared| {
                    prepared.anchor.local || rename_peers_current(&snapshot, &peers)
                });
            let _ = out.send(Outbound::PreparedRename {
                id,
                uri,
                prepared: prepared.map(Box::new),
            });
        }
        ReadJob::Rename {
            id,
            path,
            uri,
            buffer,
            offset,
            new_name,
            target,
            peers,
            out,
        } => {
            let result = if let Some(target) = target {
                if is_syntactic_r_name(&new_name) && rename_peers_current(&snapshot, &peers) {
                    oop::object_rename(
                        &snapshot,
                        &path,
                        &uri,
                        &buffer,
                        TextSize::new(offset as u32),
                        Some(&new_name),
                        encoding,
                    )
                    .flatten()
                    .filter(|result| {
                        result.identity == target.identity
                            && result.spelling == target.spelling
                            && result.declaration == target.declaration
                            && (target.declaration_is_local
                                || result.declaration_text == target.declaration_text)
                    })
                    .map(|result| (result.edit, true))
                } else {
                    None
                }
            } else {
                rename_with_scope(&snapshot, &path, &uri, &buffer, offset, &new_name, encoding)
                    .filter(|(_, workspace)| !workspace || rename_peers_current(&snapshot, &peers))
            };
            let workspace = result.as_ref().is_none_or(|(_, workspace)| *workspace);
            let _ = out.send(Outbound::RenameReply {
                response: Response::new_ok(id, result.map(|(edit, _)| edit)),
                workspace,
            });
        }
        ReadJob::WillRenameFiles { id, renames, out } => {
            let result = will_rename_via_db(&snapshot, &renames, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::DocumentSymbol {
            id,
            path,
            uri,
            buffer,
            cached,
            hierarchical,
            s7_floor,
            out,
        } => {
            let fresh;
            let symbols = if let Some(symbols) = cached.get() {
                symbols
            } else {
                let (symbols, cache_by_text) =
                    document_symbols_via_db(&snapshot, &path, &buffer, encoding, s7_floor.as_ref());
                if cache_by_text {
                    cached.get_or_init(|| symbols)
                } else {
                    fresh = symbols;
                    &fresh
                }
            };
            let response = document_symbol_response(symbols, &uri, hierarchical);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, response)));
        }
        ReadJob::WorkspaceSymbol { id, query, out } => {
            let symbols = workspace_symbols_via_db(&snapshot, &query, encoding);
            let response = WorkspaceSymbolResponse::Nested(symbols);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, response)));
        }
        ReadJob::PrepareCallHierarchy {
            id,
            path,
            uri,
            buffer,
            position,
            s7_floor,
            out,
        } => {
            let result = prepare_call_hierarchy_via_db(
                &snapshot,
                &path,
                &uri,
                &buffer,
                position,
                encoding,
                s7_floor.as_ref(),
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::IncomingCalls { id, item, out } => {
            let result = incoming_calls_via_db(&snapshot, &item, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::OutgoingCalls { id, item, out } => {
            let result = outgoing_calls_via_db(&snapshot, &item, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::PrepareTypeHierarchy {
            id,
            path,
            uri,
            buffer,
            position,
            out,
        } => {
            let result =
                prepare_type_hierarchy_via_db(&snapshot, &path, &uri, &buffer, position, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Supertypes { id, item, out } => {
            let result = supertypes_via_db(&snapshot, &item, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Subtypes { id, item, out } => {
            let result = subtypes_via_db(&snapshot, &item, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
    }
}

/// Sort and dedup `locations` (by URI then range) so a union of overlapping
/// components doesn't report the same site twice.
pub(crate) fn dedup_locations(locations: &mut Vec<Location>) {
    locations.sort_by(|a, b| {
        (a.uri.as_str(), pos_key(a.range.start), pos_key(a.range.end)).cmp(&(
            b.uri.as_str(),
            pos_key(b.range.start),
            pos_key(b.range.end),
        ))
    });
    locations.dedup();
}

/// A totally-ordered key for an LSP [`Position`].
pub(crate) fn pos_key(position: Position) -> (u32, u32) {
    (position.line, position.character)
}

/// A `Location` for `range` in the workspace file at `path`, mapping the byte
/// span through that file's *current* text. `None` if the file isn't tracked or
/// its path has no URI.
pub(crate) fn location_in(
    snapshot: &Analysis,
    path: &Path,
    range: TextRange,
    encoding: PositionEncoding,
) -> Option<Location> {
    let file = snapshot.lookup_file(path)?;
    let target_uri = uri::from_path(path)?;
    let target_index = snapshot.line_index(file);
    Some(Location {
        uri: target_uri,
        range: text_range_to_lsp_range(target_index, range, encoding),
    })
}

/// The [`TextEdit`] rewriting `range` to `new_name` in the workspace file at
/// `path`, paired with that file's URI. The write mirror of [`location_in`]: the
/// byte span is mapped through the file's *current* text via its own line index.
/// `None` if the file isn't tracked or its path has no URI.
pub(crate) fn text_edit_in(
    snapshot: &Analysis,
    path: &Path,
    range: TextRange,
    new_name: &str,
    encoding: PositionEncoding,
) -> Option<(Uri, TextEdit)> {
    let file = snapshot.lookup_file(path)?;
    let target_uri = uri::from_path(path)?;
    let target_index = snapshot.line_index(file);
    Some((
        target_uri,
        TextEdit {
            range: text_range_to_lsp_range(target_index, range, encoding),
            new_text: new_name.to_string(),
        },
    ))
}

/// Sort and dedup each file's edits, dropping empties, and wrap them in a
/// [`WorkspaceEdit`]. `None` when nothing is left to rewrite.
pub(crate) fn finalize_rename(mut changes: HashMap<Uri, Vec<TextEdit>>) -> Option<WorkspaceEdit> {
    changes.retain(|_, edits| {
        edits.sort_by_key(|a| (a.range.start, a.range.end));
        edits.dedup();
        !edits.is_empty()
    });
    (!changes.is_empty()).then(|| WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

fn rename_peers_current(snapshot: &Analysis, peers: &[(PathBuf, Arc<TextBuffer>)]) -> bool {
    salsa::Cancelled::catch(AssertUnwindSafe(|| {
        peers.iter().all(|(path, buffer)| {
            if path.file_name().is_some_and(|name| name == "NAMESPACE") {
                return match snapshot
                    .namespace_sources()
                    .iter()
                    .find(|(root, _)| Some(root.as_path()) == path.parent())
                {
                    Some((_, text)) => text == buffer.text(),
                    None => !snapshot.class_index_ref().oop_files().any(|file| {
                        file.package_root
                            .as_deref()
                            .is_some_and(|root| Some(root) == path.parent())
                    }),
                };
            }
            match snapshot.lookup_file(path) {
                Some(file) => snapshot.file_text_is(file, &buffer.text_arc()),
                None => !snapshot.class_index_ref().oop_files().any(|file| {
                    file.package_root
                        .as_ref()
                        .is_some_and(|root| path.starts_with(root.join("R")))
                }),
            }
        })
    }))
    .unwrap_or(false)
}

#[cfg(test)]
mod document_semantics_tests {
    use super::*;

    #[test]
    fn rename_checks_peer_buffer_against_the_snapshot() {
        let path = test_path().to_path_buf();
        let mut db = IncrementalDatabase::default();
        db.upsert_file(&path, "A$new()$value");
        let snapshot = db.snapshot();
        let peer = |text| [(path.clone(), Arc::new(TextBuffer::from(text)))];
        assert!(rename_peers_current(&snapshot, &peer("A$new()$value")));
        assert!(!rename_peers_current(
            &snapshot,
            &peer("# shifted\nA$new()$value")
        ));
    }

    #[test]
    fn r6_cold_warm_and_live_overlays_share_project_environment_resolution() {
        use crate::project::PackageInfo;
        let path = Path::new("/pkg/R/child.R");
        let source = "Child <- R6Class(inherit = Parent, public = list(read = function() list(self, private, super)))\n";
        let mut db = IncrementalDatabase::default();
        let parent = db.upsert_file(
            Path::new("/pkg/R/parent.R"),
            "Parent <- R6::R6Class(private = list(value = 1))\n",
        );
        let child = db.upsert_file(path, source);
        db.set_workspace_members(vec![parent, child], vec![PathBuf::from("/pkg")]);
        db.set_package_metadata(vec![PackageInfo {
            root: PathBuf::from("/pkg"),
            namespace: Some("importFrom(R6, R6Class)\n".into()),
            dir_sources: Default::default(),
            native_routines: Default::default(),
        }]);
        let buffer = buf(source);
        let cold = with_document_semantics(&db.snapshot(), path, &buffer, None, |_, model| {
            model.clone()
        });
        let warm = with_document_semantics(&db.snapshot(), path, &buffer, None, |_, model| {
            model.clone()
        });
        assert_eq!(cold, warm);
        for name in ["self", "private", "super"] {
            assert!(
                !cold.is_free_read(
                    cold.idents()
                        .iter()
                        .find(|ident| ident.name == name)
                        .unwrap()
                )
            );
        }
        let live = buf(&format!(
            "# Shifted.\n{}",
            source.replace(
                "list(self, private, super)",
                "{ nested <- function() list(self, private, super); nested() }"
            )
        ));
        let overlay = with_document_semantics(&db.snapshot(), path, &live, None, |root, model| {
            assert_eq!(root.text().to_string(), live.text());
            model.clone()
        });
        db.set_file_text(child, live.text_arc());
        let fresh =
            with_document_semantics(&db.snapshot(), path, &live, None, |_, model| model.clone());
        assert_eq!(overlay, fresh);
    }

    #[test]
    fn reference_cold_warm_and_live_overlays_share_project_environment_resolution() {
        use crate::project::PackageInfo;
        let path = Path::new("/pkg/R/child.R");
        let source = "Child <- setRefClass('Child', contains = 'Parent', methods = list(read = function() { value <<- value + 1; list(value, .self, .refClassDef) }))\n";
        let mut db = IncrementalDatabase::default();
        let parent = db.upsert_file(
            Path::new("/pkg/R/parent.R"),
            "Parent <- methods::setRefClass('Parent', fields = list(value = 'numeric'))\n",
        );
        let child = db.upsert_file(path, source);
        db.set_workspace_members(vec![parent, child], vec![PathBuf::from("/pkg")]);
        db.set_package_metadata(vec![PackageInfo {
            root: PathBuf::from("/pkg"),
            namespace: Some("importFrom(methods, setRefClass)\n".into()),
            dir_sources: Default::default(),
            native_routines: Default::default(),
        }]);
        let buffer = buf(source);
        let cold = with_document_semantics(&db.snapshot(), path, &buffer, None, |_, model| {
            model.clone()
        });
        let warm = with_document_semantics(&db.snapshot(), path, &buffer, None, |_, model| {
            model.clone()
        });
        assert_eq!(cold, warm);
        for name in ["value", ".self", ".refClassDef"] {
            assert!(
                !cold.is_free_read(
                    cold.idents()
                        .iter()
                        .find(|ident| ident.name == name)
                        .unwrap()
                )
            );
        }
        let live = buf(&format!(
            "# Shifted.\n{}",
            source.replace(
                "list(value, .self, .refClassDef)",
                "{ nested <- function() list(value, .self, .refClassDef); nested() }"
            )
        ));
        let overlay = with_document_semantics(&db.snapshot(), path, &live, None, |root, model| {
            assert_eq!(root.text().to_string(), live.text());
            model.clone()
        });
        db.set_file_text(child, live.text_arc());
        let fresh =
            with_document_semantics(&db.snapshot(), path, &live, None, |_, model| model.clone());
        assert_eq!(overlay, fresh);
    }

    #[test]
    fn matching_buffers_reuse_the_semantic_model() {
        let buffer = buf("f <- function(x) { y <- x; y }\nf(1)\n");
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(test_path(), buffer.text_arc());
        let snapshot = db.snapshot();
        let cached = snapshot.semantic_model(file);
        db.clear_query_log();

        // Equal text in a separate allocation must take the same path as the
        // shared live buffer; pointer identity alone cannot establish staleness.
        for live in [&buffer, &buf(buffer.text())] {
            with_document_semantics(&snapshot, test_path(), live, None, |root, model| {
                assert_eq!(root.text().to_string(), live.text());
                assert!(std::ptr::eq(model, cached), "reuse the cached model");
            });
        }
        assert!(db.query_log().is_empty());
    }

    #[test]
    fn stale_and_missing_models_use_the_live_buffer() {
        let buffer = buf("# shifted 😀\nf <- function(x) { y <- x; y }\nf(1)\n");
        let expected = SemanticModel::build(&parse(buffer.text()).cst);
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(test_path(), "old <- 1\n");
        let _ = db.semantic_model(file);
        let empty = IncrementalDatabase::default();
        for snapshot in [db.snapshot(), empty.snapshot()] {
            with_document_semantics(&snapshot, test_path(), &buffer, None, |root, model| {
                assert_eq!(root.text().to_string(), buffer.text());
                assert_eq!(model, &expected);
            });
        }
    }

    #[test]
    fn cancelled_document_read_retries_with_the_resolved_s7_floor() {
        let buffer = buf("library(S7)\nPoint := S7::new_class()\nPoint()\n");
        let floor = crate::config::CompatVersion::parse("0.2.2.9000");
        let path = test_path();
        let mut db = IncrementalDatabase::default();
        let file = db.upsert_file(path, buffer.text_arc());
        db.set_s7_floor(file, floor.clone());
        let snapshot = db.snapshot();
        let cached = snapshot.runtime_semantic_model(file);
        let calls = std::cell::Cell::new(0);

        let result =
            with_document_semantics(&snapshot, path, &buffer, floor.as_ref(), |root, model| {
                let attempt = calls.get();
                calls.set(attempt + 1);
                assert_eq!(std::ptr::eq(model, cached), attempt == 0);
                if attempt < 2 {
                    // Simulate a pending write canceling both snapshot-backed reads.
                    std::panic::resume_unwind(Box::new(salsa::Cancelled::PendingWrite));
                }
                assert!(
                    model
                        .bindings()
                        .iter()
                        .any(|binding| binding.name == "Point")
                );
                root.text().to_string()
            });
        assert_eq!(result, buffer.text());
        assert_eq!(calls.get(), 3);
    }
}
