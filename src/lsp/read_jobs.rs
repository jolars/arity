use super::*;

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
    Hover {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        position: Position,
        out: Sender<Outbound>,
    },
    Completion {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        position: Position,
        out: Sender<Outbound>,
    },
    SignatureHelp {
        id: RequestId,
        path: PathBuf,
        buffer: Arc<TextBuffer>,
        position: Position,
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
        out: Sender<Outbound>,
    },
    Rename {
        id: RequestId,
        path: PathBuf,
        /// In-file edits land in this URI; cross-file edits carry their own.
        uri: Uri,
        buffer: Arc<TextBuffer>,
        /// The cursor's byte offset, already resolved on the main thread (via the
        /// `prepareRename` anchor when present, else the request position) so the
        /// anchor state never crosses the thread boundary.
        offset: usize,
        new_name: String,
        out: Sender<Outbound>,
    },
    WillRenameFiles {
        id: RequestId,
        /// `(old, new)` filesystem path pairs for the files being renamed.
        renames: Vec<(PathBuf, PathBuf)>,
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
        out: Sender<Outbound>,
    },
    IncomingCalls {
        id: RequestId,
        /// The prepared item, round-tripped from the client; the target function
        /// is recovered from its `uri` + `name`.
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
        ReadJob::Hover {
            id,
            path,
            buffer,
            position,
            out,
        } => {
            let result = hover_via_db(&snapshot, &path, &buffer, position, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Completion {
            id,
            path,
            buffer,
            position,
            out,
        } => {
            let result = completion_via_db(&snapshot, &path, &buffer, position, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::SignatureHelp {
            id,
            path,
            buffer,
            position,
            out,
        } => {
            let result = signature_help_via_db(&snapshot, &path, &buffer, position, encoding);
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
            out,
        } => {
            let result = definition_via_db(&snapshot, &path, &uri, &buffer, position, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::References {
            id,
            path,
            uri,
            buffer,
            position,
            include_declaration,
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
            );
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::Rename {
            id,
            path,
            uri,
            buffer,
            offset,
            new_name,
            out,
        } => {
            let result =
                rename_via_db(&snapshot, &path, &uri, &buffer, offset, &new_name, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
        }
        ReadJob::WillRenameFiles { id, renames, out } => {
            let result = will_rename_via_db(&snapshot, &renames, encoding);
            let _ = out.send(Outbound::ReadReply(Response::new_ok(id, result)));
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
            out,
        } => {
            let result =
                prepare_call_hierarchy_via_db(&snapshot, &path, &uri, &buffer, position, encoding);
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
