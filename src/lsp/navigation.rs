use super::*;

/// Resolve go-to-definition for the name at `position`. The current file is
/// always parsed from the live `text` (definition is a deliberate, infrequent
/// action, so the parse is cheap relative to the round-trip). An intra-file
/// binding wins and reports a `Location` back into `uri`. Otherwise a bare
/// top-level name falls back to the workspace index ([`Analysis::workspace_def_sites`]),
/// reporting the sibling file(s) it is defined in. Namespaced (`pkg::name`) and
/// base-R names have no in-tree location, so they resolve to nothing (hover still
/// documents them). Snapshot reads are wrapped in [`salsa::Cancelled::catch`].
pub(crate) fn definition_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    buffer: &TextBuffer,
    position: Position,
    encoding: PositionEncoding,
) -> Option<GotoDefinitionResponse> {
    let text = buffer.text();
    let line_index = buffer.line_index();
    let offset = TextSize::new(
        line_index
            .position_to_byte(position, encoding)
            .min(text.len()) as u32,
    );
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    // Intra-file: the cursor names a local binding (or sits on its definition).
    if let Some(def_range) = definition_local_range(&root, &model, offset) {
        let location = Location {
            uri: uri.clone(),
            range: text_range_to_lsp_range(line_index, def_range, encoding),
        };
        return Some(GotoDefinitionResponse::Scalar(location));
    }

    // Cross-file: a bare top-level name defined in a sibling workspace file. A
    // namespaced name is a package export with no in-tree source location.
    let token = pick_name_token(&root, offset)?;
    if token.kind() != SyntaxKind::IDENT
        || matches!(
            symbol_query_at(&root, offset),
            Some(SymbolQuery::Namespaced { .. })
        )
    {
        return None;
    }
    let name = SmolStr::new(token.text());
    let locations = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot
            .workspace_def_sites(&name)
            .into_iter()
            // The current file is handled intra-file above; skip it so a stale
            // tracked copy never shadows the live buffer's own definition.
            .filter(|(def_path, _)| def_path != path)
            .filter_map(|(def_path, range)| {
                let file = snapshot.lookup_file(&def_path)?;
                let target_uri = uri::from_path(&def_path)?;
                let target_index = snapshot.line_index(file);
                Some(Location {
                    uri: target_uri,
                    range: text_range_to_lsp_range(target_index, range, encoding),
                })
            })
            .collect::<Vec<_>>()
    }))
    .unwrap_or_default();

    match locations.len() {
        0 => None,
        1 => Some(GotoDefinitionResponse::Scalar(
            locations.into_iter().next()?,
        )),
        _ => Some(GotoDefinitionResponse::Array(locations)),
    }
}

/// Resolve `textDocument/references` against a db `snapshot`. The inverse of
/// [`definition_via_db`] and the read mirror of [`rename_via_db`], in the same
/// two phases, but **scope-aware**: cross-file reads come from the visibility
/// component ([`cross_file_reference_locations`]), not every workspace site of the
/// name. Intra-file: the cursor names a local binding (or its definition), and
/// every in-file read of it is reported (plus the definition when
/// `include_declaration`). When that binding is *file-scope*, the component adds
/// the cross-file reads (and sibling definitions, when `include_declaration`).
/// Otherwise the cursor sits on a bare free read, resolved to the definition(s)
/// it can see ([`Analysis::visible_def_files`]); a unique resolution reports its
/// component, and an ambiguous one reports the union.
///
/// Unlike rename, references is non-destructive, so it never refuses: it
/// over-reports (the whole component even on a conflict / with a dynamic
/// `source()`) rather than risk missing a reference. Namespaced (`pkg::name`) and
/// base-R names have no in-tree reads. Snapshot reads are wrapped in
/// [`salsa::Cancelled::catch`].
pub(crate) fn references_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    buffer: &TextBuffer,
    position: Position,
    include_declaration: bool,
    encoding: PositionEncoding,
) -> Option<Vec<Location>> {
    let text = buffer.text();
    let line_index = buffer.line_index();
    let offset = TextSize::new(
        line_index
            .position_to_byte(position, encoding)
            .min(text.len()) as u32,
    );
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    // Intra-file: the cursor names a local binding (or sits on its definition).
    if let Some((target, occ)) = local_occurrences(&root, &model, offset) {
        let mut locations: Vec<Location> = occ
            .reads
            .iter()
            .map(|range| Location {
                uri: uri.clone(),
                range: text_range_to_lsp_range(line_index, *range, encoding),
            })
            .collect();
        if include_declaration {
            locations.extend(occ.defs.iter().map(|range| Location {
                uri: uri.clone(),
                range: text_range_to_lsp_range(line_index, *range, encoding),
            }));
        }
        // Cross-file: a top-level binding can be read from files that can see
        // this one. Scope to that component; this file's own occurrences were
        // collected above, so skip it. Nested locals stay intra-file.
        if model.binding_is_file_scope(target.binding) {
            locations.extend(cross_file_reference_locations(
                snapshot,
                path,
                target.name.as_str(),
                include_declaration,
                Some(path),
                encoding,
            ));
        }
        return (!locations.is_empty()).then_some(locations);
    }

    // The cursor sits on a bare free read of a workspace name (no local binding).
    // A namespaced name is a package export with no in-tree reads to collect.
    let token = pick_name_token(&root, offset)?;
    if token.kind() != SyntaxKind::IDENT
        || matches!(
            symbol_query_at(&root, offset),
            Some(SymbolQuery::Namespaced { .. })
        )
    {
        return None;
    }
    let name = SmolStr::new(token.text());
    let visible_defs = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot.visible_def_files(path, name.as_str())
    }))
    .unwrap_or_default();
    if visible_defs.is_empty() {
        return None;
    }
    // A unique resolution reports its component; an ambiguous one reports the
    // union (references over-reports rather than refuse). Include this file's own
    // read (skip = None).
    let mut locations: Vec<Location> = visible_defs
        .iter()
        .flat_map(|def_file| {
            cross_file_reference_locations(
                snapshot,
                def_file,
                name.as_str(),
                include_declaration,
                None,
                encoding,
            )
        })
        .collect();
    dedup_locations(&mut locations);
    (!locations.is_empty()).then_some(locations)
}

/// Resolve `textDocument/rename` against a db `snapshot` — the write mirror of
/// [`references_via_db`], in the same two phases, emitting a multi-URI
/// [`WorkspaceEdit`] instead of `Location`s. Intra-file: the cursor names a local
/// binding (or its definition), and every in-file read plus the definition is
/// rewritten to `new_name` in `uri`. When that binding is *file-scope*, the
/// cross-file rewrite is **scope-aware** ([`cross_file_rename_edits`]): only the
/// visibility component is touched — package siblings / `source()`-closure
/// readers that can actually see the definition — not every workspace site of the
/// name. Otherwise the cursor sits on a bare free read, which is resolved to its
/// single visible definition ([`Analysis::visible_def_files`]) and then renamed
/// through the same component.
///
/// Rename is *sound in both directions* (never rewrite an unrelated binding,
/// never miss a read), so it **refuses** (`None`) when it cannot prove that:
/// an *incomplete* multi-def package cohort (a hidden alias/read could live in an
/// unanalyzed member), a dynamic `source()` whose reachable scope includes a
/// reader of the renamed name (a hidden reader could be missed), or a bare read
/// that resolves to zero or more than one visible definition. A *complete* multi-def package cohort renames all aliases
/// (one flat namespace). Namespaced (`pkg::name`) names and non-syntactic
/// `new_name`s are declined. Snapshot reads are wrapped in
/// [`salsa::Cancelled::catch`].
pub(crate) fn rename_via_db(
    snapshot: &Analysis,
    path: &Path,
    uri: &Uri,
    buffer: &TextBuffer,
    offset: usize,
    new_name: &str,
    encoding: PositionEncoding,
) -> Option<WorkspaceEdit> {
    let text = buffer.text();
    if !is_syntactic_r_name(new_name) {
        return None;
    }
    let line_index = buffer.line_index();
    let off = TextSize::new(offset.min(text.len()) as u32);
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);

    let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();

    // Intra-file: the cursor names a local binding (or sits on its definition).
    if let Some(target) = resolve_local_target(&root, &model, off) {
        let intra = rename_edits(&model, &target, new_name, line_index, encoding);
        // Cross-file: a top-level binding can be free-read from files that can
        // see this one. Scope to that component; refuse if it isn't safe. Nested
        // locals are file-private, so they stay intra-file.
        if model.binding_is_file_scope(target.binding) {
            // This file's own occurrences are `intra`; skip it cross-file.
            let cross = cross_file_rename_edits(
                snapshot,
                path,
                target.name.as_str(),
                new_name,
                Some(path),
                encoding,
            )?;
            changes.insert(uri.clone(), intra);
            for (edit_uri, edit) in cross {
                changes.entry(edit_uri).or_default().push(edit);
            }
        } else {
            changes.insert(uri.clone(), intra);
        }
        return finalize_rename(changes);
    }

    // The cursor sits on a bare free read of a workspace name (no local binding).
    // A namespaced name is a package export with no in-tree sites to rewrite.
    let token = pick_name_token(&root, off)?;
    if token.kind() != SyntaxKind::IDENT
        || matches!(
            symbol_query_at(&root, off),
            Some(SymbolQuery::Namespaced { .. })
        )
    {
        return None;
    }
    let name = SmolStr::new(token.text());
    // Resolve the bare read to the single definition it binds to. Zero (base R /
    // package export / not visible) or more than one (ambiguous) → refuse.
    let visible_defs = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot.visible_def_files(path, name.as_str())
    }))
    .unwrap_or_default();
    let [def_file] = visible_defs.as_slice() else {
        return None;
    };
    // Rename the whole component, the current file's read included (skip = None).
    let cross =
        cross_file_rename_edits(snapshot, def_file, name.as_str(), new_name, None, encoding)?;
    for (edit_uri, edit) in cross {
        changes.entry(edit_uri).or_default().push(edit);
    }
    finalize_rename(changes)
}

/// The cross-file text edits renaming the top-level `name` defined in `def_file`
/// to `new_name`, scoped to its visibility component via
/// [`Analysis::cross_file_binding`]. Rewrites each cohort member's own
/// definition and the reads bound to it ([`file_scope_occurrences_in`]) and, per
/// reader, the free reads that bind to the cohort under `source()` load order
/// ([`Analysis::reader_rename_ranges`] — a reader's pre-source / bound-elsewhere
/// reads are skipped, not refused). `skip` is the buffer whose occurrences the
/// caller already rewrote intra-file (skipped here to avoid double edits); pass
/// `None` to include every file.
///
/// Returns `None` to **refuse the whole rename** when soundness can't be
/// guaranteed: an *incomplete* multi-def package cohort (a hidden alias/read
/// could live in an unanalyzed member), any dynamic `source()` reaching a reader
/// of the name (`dynamic_source_risk`), or a reader with a top-level read whose
/// binding is undecidable (two static closure definers — `reader_rename_ranges`
/// yields `None`). A *complete* multi-def package cohort is **not** refused — its
/// members are aliases of one flat-namespace slot, so rewriting every cohort def
/// plus every cohort-bound reader read is a sound rename-all. (Cancellation also
/// yields `None`.)
pub(crate) fn cross_file_rename_edits(
    snapshot: &Analysis,
    def_file: &Path,
    name: &str,
    new_name: &str,
    skip: Option<&Path>,
    encoding: PositionEncoding,
) -> Option<Vec<(Uri, TextEdit)>> {
    let binding = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot.cross_file_binding(def_file, name)
    }))
    .ok()?;
    if binding.cohort_incomplete || binding.dynamic_source_risk {
        return None;
    }
    let mut edits = Vec::new();
    // Cohort members: their own definition + the reads that bind to it (these
    // resolve locally, so `read_ranges_in` — free reads only — would miss them).
    for member in &binding.cohort {
        if skip == Some(member.as_path()) {
            continue;
        }
        let Some(file) = snapshot.lookup_file(member) else {
            continue;
        };
        let member_model = snapshot.semantic_model(file);
        let Some((defs, reads)) = file_scope_occurrences_in(member_model, name) else {
            continue;
        };
        for range in defs.into_iter().chain(reads) {
            if let Some(edit) = text_edit_in(snapshot, member, range, new_name, encoding) {
                edits.push(edit);
            }
        }
    }
    // Readers: the free reads of the name that bind to the cohort under load
    // order. A reader's pre-source / bound-elsewhere reads are skipped; an
    // undecidable read makes `reader_rename_ranges` return `None` and refuses the
    // whole rename (`?`).
    for reader in &binding.readers {
        if skip == Some(reader.as_path()) {
            continue;
        }
        for range in snapshot.reader_rename_ranges(reader, name, &binding.cohort)? {
            if let Some(edit) = text_edit_in(snapshot, reader, range, new_name, encoding) {
                edits.push(edit);
            }
        }
    }
    Some(edits)
}

/// The cross-file reference [`Location`]s for the top-level `name` defined in
/// `def_file`, scoped to its visibility component via
/// [`Analysis::cross_file_binding`] — the read mirror of
/// [`cross_file_rename_edits`]. Reports each cohort member's reads (plus its
/// definition when `include_declaration`) and each reader's free reads. `skip`
/// is the buffer whose occurrences the caller already collected intra-file.
///
/// Unlike rename, references is *non-destructive*, so it never refuses: it
/// reports the whole component even on a name conflict or with a dynamic
/// `source()` present (over-reporting is acceptable, a missed/false edit is not).
pub(crate) fn cross_file_reference_locations(
    snapshot: &Analysis,
    def_file: &Path,
    name: &str,
    include_declaration: bool,
    skip: Option<&Path>,
    encoding: PositionEncoding,
) -> Vec<Location> {
    let Ok(binding) = salsa::Cancelled::catch(AssertUnwindSafe(|| {
        snapshot.cross_file_binding(def_file, name)
    })) else {
        return Vec::new();
    };
    let mut locations = Vec::new();
    for member in &binding.cohort {
        if skip == Some(member.as_path()) {
            continue;
        }
        let Some(file) = snapshot.lookup_file(member) else {
            continue;
        };
        let Some((defs, reads)) = file_scope_occurrences_in(snapshot.semantic_model(file), name)
        else {
            continue;
        };
        let decl_defs = include_declaration.then_some(defs).into_iter().flatten();
        for range in reads.into_iter().chain(decl_defs) {
            if let Some(loc) = location_in(snapshot, member, range, encoding) {
                locations.push(loc);
            }
        }
    }
    for reader in &binding.readers {
        if skip == Some(reader.as_path()) {
            continue;
        }
        let Some(file) = snapshot.lookup_file(reader) else {
            continue;
        };
        for range in snapshot.read_ranges_in(file, name) {
            if let Some(loc) = location_in(snapshot, reader, range, encoding) {
                locations.push(loc);
            }
        }
    }
    locations
}

/// The definition ranges and the ranges of the reads bound to them for the
/// file-scope variable named `name` in `model` (each sorted and deduped). `None`
/// when there is no such top-level binding. The sibling-file analogue of
/// [`local_occurrences`]: used to rewrite/report a cohort member's own
/// definitions (a file-scope name can be reassigned) and the reads that resolve
/// to them — which are *not* free reads, so [`Analysis::read_ranges_in`] would
/// miss them.
pub(crate) fn file_scope_occurrences_in(
    model: &SemanticModel,
    name: &str,
) -> Option<(Vec<TextRange>, Vec<TextRange>)> {
    let (idx, _) = model.bindings().iter().enumerate().find(|(i, b)| {
        matches!(b.kind, BindingKind::Local | BindingKind::Implicit)
            && b.name.as_str() == name
            && model.binding_is_file_scope(BindingId::from_index(*i))
    })?;
    Some(variable_occurrences(model, BindingId::from_index(idx)))
}

/// A cross-edit-stable anchor for an in-flight rename: a [`NodePtr`] to the
/// renamed identifier's enclosing node, the cursor's offset *within* that node,
/// the name, and the buffer the handle was taken against. Opaque to callers —
/// produced by [`compute_prepare_rename`] and consumed by
/// [`compute_rename_with_anchor`].
#[derive(Debug, Clone)]
pub struct RenameAnchor {
    node_ptr: NodePtr,
    offset_in_node: u32,
    text: String,
    /// The precise per-change edits transforming `text` into the current buffer,
    /// accumulated across every `didChange` since prepare (in application order).
    /// `Some` is a candidate for the precise [`map_range_through_edits`] path;
    /// `None` means the sequence was invalidated (a full-document replacement, or
    /// a change before the buffer existed) and re-anchoring must fall back to the
    /// whole-text [`diff_edit`]. Fed by [`RenameAnchor::record_edits`].
    edits: Option<Vec<Edit>>,
}

impl RenameAnchor {
    /// Fold one `didChange` batch into the anchor's accumulator. `precise` is the
    /// server's per-batch signal (false when the batch was a full-document
    /// replacement that no tight edit sequence can express): a non-precise batch,
    /// or an already-invalidated accumulator, latches the sequence to `None`.
    pub(crate) fn record_edits(&mut self, batch: &[Edit], precise: bool) {
        match &mut self.edits {
            Some(acc) if precise => acc.extend_from_slice(batch),
            _ => self.edits = None,
        }
    }
}

/// The result of [`compute_prepare_rename`]: the editable range + placeholder the
/// LSP returns, plus the [`RenameAnchor`] the server stashes for the follow-up
/// `rename`.
#[derive(Debug, Clone)]
pub struct PreparedRename {
    pub range: Range,
    pub placeholder: String,
    pub anchor: RenameAnchor,
}

/// The binding the cursor names, plus the token range and name.
pub(crate) struct LocalTarget {
    binding: BindingId,
    range: TextRange,
    name: SmolStr,
}

/// Resolve the cursor to the *local* binding it names, whether the cursor is on a
/// read site or the definition itself. Returns `None` for a non-identifier, or a
/// name that resolves to no local binding (a package export, a global, an
/// undefined name) — those are out of scope for intra-file rename and the
/// intra-file branch of go-to-definition (which falls back to the workspace
/// index for them).
pub(crate) fn resolve_local_target(
    root: &SyntaxNode,
    model: &SemanticModel,
    offset: TextSize,
) -> Option<LocalTarget> {
    let token = pick_name_token(root, offset)?;
    if token.kind() != SyntaxKind::IDENT {
        return None;
    }
    let range = token.text_range();
    let name = SmolStr::new(token.text());
    if let Some(ident) = model.idents().iter().find(|i| i.range == range) {
        let binding = model.resolve_local(ident)?;
        return Some(LocalTarget {
            binding,
            range,
            name,
        });
    }
    let idx = model.bindings().iter().position(|b| b.def_range == range)?;
    Some(LocalTarget {
        binding: BindingId::from_index(idx),
        range,
        name,
    })
}

/// `textDocument/prepareRename`: validate the cursor is on a renameable local and
/// return its range + placeholder, plus the cross-edit [`RenameAnchor`]. Pure
/// (parses `text` itself) so it is unit-testable. Refuses on parse errors so a
/// prepared rename never resolves against a malformed tree.
pub fn compute_prepare_rename(
    text: &str,
    offset: usize,
    encoding: PositionEncoding,
) -> Option<PreparedRename> {
    compute_prepare_rename_in(&TextBuffer::from(text), offset, encoding)
}

/// [`compute_prepare_rename`] against a live buffer, reusing its maintained
/// line index instead of rebuilding one per request.
pub(crate) fn compute_prepare_rename_in(
    buffer: &TextBuffer,
    offset: usize,
    encoding: PositionEncoding,
) -> Option<PreparedRename> {
    let text = buffer.text();
    let parsed = parse(text);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let root = parsed.cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let target = resolve_local_target(&root, &model, off)?;
    let token = pick_name_token(&root, off)?;
    let node = token.parent()?;
    let offset_in_node = u32::from(target.range.start()) - u32::from(node.text_range().start());
    let line_index = buffer.line_index();
    Some(PreparedRename {
        range: Range {
            start: line_index.byte_to_position(usize::from(target.range.start()), encoding),
            end: line_index.byte_to_position(usize::from(target.range.end()), encoding),
        },
        placeholder: target.name.to_string(),
        anchor: RenameAnchor {
            node_ptr: NodePtr::from_node(&node),
            offset_in_node,
            text: text.to_string(),
            edits: Some(Vec::new()),
        },
    })
}

/// `textDocument/rename` from a cursor offset: the text edits that rename the
/// binding under the cursor and all its in-file reads to `new_name`. Pure and
/// unit-testable. Returns `None` when `new_name` isn't a syntactic R identifier,
/// the file has parse errors, or the cursor names no renameable local.
pub fn compute_rename(
    text: &str,
    offset: usize,
    new_name: &str,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    if !is_syntactic_r_name(new_name) {
        return None;
    }
    let parsed = parse(text);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let root = parsed.cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let target = resolve_local_target(&root, &model, off)?;
    let line_index = LineIndex::new(text);
    let edits = rename_edits(&model, &target, new_name, &line_index, encoding);
    (!edits.is_empty()).then_some(edits)
}

/// The byte span of the definition of the local binding under the cursor, if the
/// name resolves to an in-file binding. Pure (parses `text` itself) and
/// unit-testable; the intra-file half of go-to-definition. Best-effort, with no
/// clean-parse gate (jumping is always safe). Returns `None` for a
/// non-identifier or a name that names no local binding — a package export, a
/// global, or a cross-file name (the last is resolved against the workspace index
/// by [`definition_via_db`], which this helper does not see).
pub fn compute_definition(text: &str, offset: usize) -> Option<TextRange> {
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    definition_local_range(&root, &model, off)
}

/// The in-file read spans of the local binding under the cursor, plus its
/// definition span when `include_declaration`. Pure (parses `text` itself) and
/// unit-testable: the intra-file core of go-to-references. The cross-file reads
/// of a top-level name are added by [`references_via_db`], which this helper does
/// not see (mirroring how [`compute_definition`] handles only the intra-file
/// half). `None` for a non-identifier or a name that names no local binding.
pub fn compute_references(
    text: &str,
    offset: usize,
    include_declaration: bool,
) -> Option<Vec<TextRange>> {
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let (_, occ) = local_occurrences(&root, &model, off)?;
    let mut ranges = occ.reads;
    if include_declaration {
        ranges.extend(occ.defs);
    }
    ranges.sort_by_key(|range| range.start());
    ranges.dedup();
    Some(ranges)
}

/// The document highlights for the local binding under the cursor: its definition
/// as [`DocumentHighlightKind::WRITE`] and each in-file read as
/// [`DocumentHighlightKind::READ`], sorted by position. Pure and unit-testable;
/// always same-file (no workspace lookup). `None` when the cursor names no local
/// binding.
pub fn compute_document_highlights(
    text: &str,
    offset: usize,
) -> Option<Vec<(TextRange, DocumentHighlightKind)>> {
    let root = parse(text).cst;
    let model = SemanticModel::build(&root);
    let off = TextSize::new(offset.min(text.len()) as u32);
    let (_, occ) = local_occurrences(&root, &model, off)?;
    let mut highlights: Vec<(TextRange, DocumentHighlightKind)> =
        Vec::with_capacity(occ.reads.len() + occ.defs.len());
    highlights.extend(
        occ.defs
            .into_iter()
            .map(|range| (range, DocumentHighlightKind::WRITE)),
    );
    highlights.extend(
        occ.reads
            .into_iter()
            .map(|range| (range, DocumentHighlightKind::READ)),
    );
    highlights.sort_by_key(|(range, _)| range.start());
    Some(highlights)
}

/// The def span the cursor's local binding resolves to, off an already-parsed CST
/// and model. The shared core of [`compute_definition`] and the intra-file branch
/// of [`definition_via_db`].
pub(crate) fn definition_local_range(
    root: &SyntaxNode,
    model: &SemanticModel,
    offset: TextSize,
) -> Option<TextRange> {
    let target = resolve_local_target(root, model, offset)?;
    Some(model.binding(target.binding).def_range)
}

/// The definition spans and every in-file read span of the *variable* under the
/// cursor, each sorted and deduped. The shared intra-file core of find-references
/// and document highlight: [`resolve_local_target`] picks the binding, then its
/// frame cohort (all same-name reassignments, [`SemanticModel::variable_cohort`])
/// contributes every definition, and the def-use reverse index
/// ([`SemanticModel::read_sites`]) contributes every read bound to the cohort.
/// A reassignment is one variable, so all its writes and reads are gathered
/// together (the read-gathering half of [`rename_edits`]). `None` when the cursor
/// names no local binding.
pub(crate) struct LocalOccurrences {
    defs: Vec<TextRange>,
    reads: Vec<TextRange>,
}

pub(crate) fn local_occurrences(
    root: &SyntaxNode,
    model: &SemanticModel,
    offset: TextSize,
) -> Option<(LocalTarget, LocalOccurrences)> {
    let target = resolve_local_target(root, model, offset)?;
    let (defs, reads) = variable_occurrences(model, target.binding);
    Some((target, LocalOccurrences { defs, reads }))
}

/// The sorted, deduped definition and read spans of the whole variable that
/// `binding` belongs to: the frame cohort's [`Binding::def_range`]s and the union
/// of each cohort member's [`SemanticModel::read_sites`]. Cohort members share
/// conservatively-resolved reads, so the read set is deduped. The shared gather
/// behind [`local_occurrences`], [`rename_edits`], [`file_scope_occurrences_in`],
/// and call hierarchy's nested-item incoming edges
/// ([`crate::lsp::call_hierarchy`]).
pub(crate) fn variable_occurrences(
    model: &SemanticModel,
    binding: BindingId,
) -> (Vec<TextRange>, Vec<TextRange>) {
    let cohort = model.variable_cohort(binding);
    let mut defs: Vec<TextRange> = cohort
        .iter()
        .map(|id| model.binding(*id).def_range)
        .collect();
    defs.sort_by_key(|range| range.start());
    defs.dedup();
    let mut reads: Vec<TextRange> = cohort
        .iter()
        .flat_map(|id| model.read_sites(*id).map(|ident| ident.range))
        .collect();
    reads.sort_by_key(|range| range.start());
    reads.dedup();
    (defs, reads)
}

/// `textDocument/rename` driven by a [`RenameAnchor`] instead of a fresh
/// position: re-locate the cursor in `current_text` via the anchor (mapping its
/// node range across any edit since prepare), then rename. This mirrors
/// [`Analysis::resolve_ptr`](crate::incremental::Analysis::resolve_ptr) but
/// resolves against the live buffer, which is authoritative for the in-flight
/// edit. Returns `None` if the anchor's node was edited away (caller falls back
/// to the request position).
pub fn compute_rename_with_anchor(
    current_text: &str,
    anchor: &RenameAnchor,
    new_name: &str,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    let offset = rename_cursor_offset(current_text, anchor)?;
    compute_rename(current_text, offset, new_name, encoding)
}

/// Re-derive the cursor's byte offset in `current_text` from a [`RenameAnchor`]:
/// resolve the anchor's node (directly when the text is unchanged, else by
/// mapping its range forward) and add the stored intra-node offset.
///
/// When the anchor accumulated the precise per-change edits since prepare and
/// they reconstruct `current_text` exactly (apply-and-verify), the node range
/// folds through them with [`map_range_through_edits`] — disjoint edits stay
/// disjoint, so a node between two of them survives where a single coalesced
/// [`diff_edit`] would straddle it. A missing, empty, or misaligned sequence
/// falls back to that whole-text `diff_edit`.
pub(crate) fn rename_cursor_offset(current_text: &str, anchor: &RenameAnchor) -> Option<usize> {
    let root = parse(current_text).cst;
    let node = if current_text == anchor.text {
        anchor.node_ptr.try_to_node(&root)?
    } else {
        let mapped = match &anchor.edits {
            Some(edits)
                if !edits.is_empty() && apply_edits(&anchor.text, edits) == current_text =>
            {
                map_range_through_edits(anchor.node_ptr.text_range(), edits)?
            }
            _ => {
                let edit = diff_edit(&anchor.text, current_text);
                map_range_through_edit(anchor.node_ptr.text_range(), &edit)?
            }
        };
        anchor.node_ptr.with_range(mapped).try_to_node(&root)?
    };
    Some(usize::from(node.text_range().start()) + anchor.offset_in_node as usize)
}

/// The text edits renaming every definition and in-file read of the variable
/// `target` names — its whole frame cohort, not just one binding record.
pub(crate) fn rename_edits(
    model: &SemanticModel,
    target: &LocalTarget,
    new_name: &str,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Vec<TextEdit> {
    let (defs, reads) = variable_occurrences(model, target.binding);
    let mut ranges: Vec<TextRange> = defs;
    ranges.extend(reads);
    ranges.sort_by_key(|range| range.start());
    ranges.dedup();
    ranges
        .into_iter()
        .map(|range| TextEdit {
            range: Range {
                start: line_index.byte_to_position(usize::from(range.start()), encoding),
                end: line_index.byte_to_position(usize::from(range.end()), encoding),
            },
            new_text: new_name.to_string(),
        })
        .collect()
}

/// Whether `name` is a syntactic R identifier usable without backtick-quoting:
/// starts with a letter or `.` (and a leading `.` is not followed by a digit),
/// contains only letters, digits, `.`, and `_`, and isn't a reserved word.
/// Backtick-quoted non-syntactic names are out of scope (the rename withholds).
pub(crate) fn is_syntactic_r_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '.') {
        return false;
    }
    if first == '.' && matches!(name.as_bytes().get(1), Some(b) if b.is_ascii_digit()) {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return false;
    }
    !is_reserved_word(name)
}

pub(crate) fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "repeat"
            | "while"
            | "function"
            | "for"
            | "in"
            | "next"
            | "break"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "Inf"
            | "NaN"
            | "NA"
            | "NA_integer_"
            | "NA_real_"
            | "NA_character_"
            | "NA_complex_"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_cursor_offset_precise_slice_survives_disjoint_edits() {
        // Prepare a rename of `value`, then two disjoint edits straddle its
        // `value <- 1` assignment: a comment above and a statement below.
        // Coalesced into one `diff_edit` they span the node interior and
        // invalidate the handle; kept disjoint via the accumulated slice, the
        // node survives and the cursor re-anchors at the shifted `value`.
        let text_a = "value <- 1\nprint(value)\n";
        let mut anchor = compute_prepare_rename(text_a, 0, PositionEncoding::Utf16)
            .expect("offers rename")
            .anchor;

        let text_b = "# note\nvalue <- 1\nprint(value)\nz <- 2\n";
        anchor.record_edits(
            &[
                Edit {
                    range: 0..0,
                    insert: "# note\n".to_string(),
                },
                Edit {
                    range: 31..31,
                    insert: "z <- 2\n".to_string(),
                },
            ],
            true,
        );

        // Precise path: `value` sits at offset 7 in text_b (shifted by "# note\n").
        assert_eq!(rename_cursor_offset(text_b, &anchor), Some(7));
    }

    #[test]
    fn rename_cursor_offset_without_slice_falls_back_and_invalidates() {
        // Same disjoint edits, but no slice was recorded (the empty accumulator a
        // fresh anchor carries). The whole-text `diff_edit` fallback spans the
        // node interior, so re-anchoring fails — `on_rename` then falls back to
        // the request position.
        let text_a = "value <- 1\nprint(value)\n";
        let anchor = compute_prepare_rename(text_a, 0, PositionEncoding::Utf16)
            .expect("offers rename")
            .anchor;
        let text_b = "# note\nvalue <- 1\nprint(value)\nz <- 2\n";
        assert_eq!(rename_cursor_offset(text_b, &anchor), None);
    }

    #[test]
    fn record_edits_latches_to_none_on_a_non_precise_batch() {
        // A full-document replacement (precise=false) can't be expressed as a
        // tight sequence, so it invalidates the accumulator and a later precise
        // batch cannot revive it.
        let anchor = compute_prepare_rename("x <- 1\n", 0, PositionEncoding::Utf16)
            .expect("offers rename")
            .anchor;
        let mut anchor = anchor;
        anchor.record_edits(&[], false);
        assert!(anchor.edits.is_none());
        anchor.record_edits(
            &[Edit {
                range: 0..0,
                insert: "y".to_string(),
            }],
            true,
        );
        assert!(anchor.edits.is_none(), "a latched accumulator stays None");
    }

    #[test]
    fn rename_via_db_rewrites_a_definition_and_its_cross_file_reads() {
        // a.R defines `foo`; b.R sources a.R and reads it. Renaming from the
        // definition must edit both files in one WorkspaceEdit.
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("rename is available on a file-scope definition");
        let changes = edit.changes.expect("changes present");

        let a_edits = changes
            .get(&uri_a)
            .expect("the definition in a.R is edited");
        assert_eq!(a_edits.len(), 1);
        assert_eq!(a_edits[0].new_text, "renamed");

        let b_edits = changes
            .get(&uri_b)
            .expect("the cross-file read in b.R is edited");
        assert_eq!(b_edits.len(), 1);
        assert_eq!(b_edits[0].new_text, "renamed");
    }

    #[test]
    fn rename_via_db_rewrites_a_reassigned_file_scope_def_across_files() {
        // a.R defines `foo` then reassigns it; b.R sources a.R and reads foo.
        // The reassignment is one variable, so renaming rewrites *both* defs in
        // a.R (via the frame cohort) plus the cross-file read in b.R.
        let a_src = "foo <- function() 1\nfoo <- function() 2\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("rename is available on a file-scope definition");
        let changes = edit.changes.expect("changes present");

        let a_edits = changes.get(&uri_a).expect("a.R is edited");
        assert_eq!(
            a_edits.len(),
            2,
            "both reassignment defs of foo in a.R are rewritten"
        );
        let b_edits = changes.get(&uri_b).expect("b.R read is edited");
        assert_eq!(b_edits.len(), 1);
    }

    #[test]
    fn rename_via_db_from_a_cross_file_read_rewrites_the_definition() {
        // Cursor on the `foo()` read in b.R (which sources a.R), binding to no
        // local: rename resolves the read to a.R's definition and touches both.
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = b_src.find("foo()").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("b.R"),
            &uri_b,
            &buf(b_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("rename is available on a workspace free read");
        let changes = edit.changes.expect("changes present");

        assert!(
            changes.contains_key(&uri_a),
            "the definition in a.R is edited"
        );
        assert!(changes.contains_key(&uri_b), "the read in b.R is edited");
    }

    #[test]
    fn rename_via_db_does_not_corename_disjoint_same_name_defs() {
        // Two unconnected flat scripts each define their own top-level `foo`.
        // Renaming a.R's `foo` must NOT touch b.R's unrelated `foo`.
        let a_src = "foo <- function() 1\n";
        let b_src = "foo <- function() 2\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("renaming a local file-scope def is fine");
        let changes = edit.changes.expect("changes present");

        assert!(changes.contains_key(&uri_a), "a.R's own foo is renamed");
        assert!(
            !changes.contains_key(&uri_b),
            "b.R's unrelated foo must be left alone"
        );
    }

    #[test]
    fn rename_via_db_corenames_package_siblings() {
        // a.R defines `foo`; b.R (a package sibling, shared flat namespace) reads
        // it. Both must be rewritten.
        let (_dir, snapshot, a_path, b_path) =
            rename_package("foo <- function() 1\n", "bar <- function() foo()\n");
        let uri_a = uri::from_path(&a_path).unwrap();
        let uri_b = uri::from_path(&b_path).unwrap();
        let a_src = "foo <- function() 1\n";
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &a_path,
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("rename available on a package-scope definition");
        let changes = edit.changes.expect("changes present");

        assert!(changes.contains_key(&uri_a), "a.R's definition is renamed");
        assert!(
            changes.contains_key(&uri_b),
            "the package sibling's read is renamed"
        );
    }

    #[test]
    fn rename_via_db_skips_reader_that_shadows_with_own_def() {
        // b.R sources a.R but also defines its own top-level `foo`, so its `foo`
        // read binds to its own def, not a.R's. Renaming a.R's foo must not touch
        // b.R. (a.R alone defines foo within a.R's component, so no conflict.)
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nfoo <- function() 2\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("a.R's foo is the only def in a.R's component");
        let changes = edit.changes.expect("changes present");

        assert!(changes.contains_key(&uri_a), "a.R's definition is renamed");
        assert!(
            !changes.contains_key(&uri_b),
            "b.R shadows foo with its own def, so it is untouched"
        );
    }

    #[test]
    fn rename_via_db_corenames_complete_package_multidef() {
        // Package siblings both define top-level `foo`: one flat namespace, so
        // they are aliases of one slot. The package is complete (both R files
        // analyzed), so a rename-all is sound — rewrite *both* definitions.
        let (_dir, snapshot, a_path, b_path) =
            rename_package("foo <- function() 1\n", "foo <- function() 2\n");
        let uri_a = uri::from_path(&a_path).unwrap();
        let uri_b = uri::from_path(&b_path).unwrap();
        let a_src = "foo <- function() 1\n";
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &a_path,
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("complete package: multi-def rename is a sound rename-all");
        let changes = edit.changes.expect("changes present");
        assert!(
            changes.contains_key(&uri_a),
            "a.R's definition of the alias is renamed"
        );
        assert!(
            changes.contains_key(&uri_b),
            "the sibling's definition of the same flat slot is renamed too"
        );
    }

    #[test]
    fn rename_via_db_refuses_multidef_with_unseeded_sibling() {
        // a.R + b.R both define foo (seeded); c.R on disk also defines foo but is
        // not analyzed. The package's R/ set isn't covered, so a rename-all could
        // miss c.R's alias/reads — refuse.
        assert!(
            !package_multidef_rename_offered(
                "Package: testpkg\n",
                &[
                    ("a.R", "foo <- function() 1\n", true),
                    ("b.R", "foo <- function() 2\n", true),
                    ("c.R", "foo <- function() 3\n", false),
                ],
            ),
            "an unanalyzed R/ sibling makes the package incomplete: rename refuses"
        );
    }

    #[test]
    fn rename_via_db_refuses_multidef_with_parse_error_sibling() {
        // c.R is seeded but has a parse error, so `workspace_project` drops it —
        // the analyzed set no longer covers R/, so the multi-def rename refuses.
        assert!(
            !package_multidef_rename_offered(
                "Package: testpkg\n",
                &[
                    ("a.R", "foo <- function() 1\n", true),
                    ("b.R", "foo <- function() 2\n", true),
                    ("c.R", "foo <- function() {\n", true),
                ],
            ),
            "a dropped parse-error member makes the package incomplete: rename refuses"
        );
    }

    #[test]
    fn rename_via_db_refuses_multidef_when_collate_names_absent_file() {
        // `Collate:` lists c.R, which is neither on disk nor analyzed. The
        // expected source set (dir glob ∪ Collate) exceeds the analyzed set, so
        // the package is incomplete and the multi-def rename refuses.
        assert!(
            !package_multidef_rename_offered(
                "Package: testpkg\nCollate: a.R b.R c.R\n",
                &[
                    ("a.R", "foo <- function() 1\n", true),
                    ("b.R", "foo <- function() 2\n", true),
                ],
            ),
            "a Collate entry outside the analyzed set makes the package incomplete: rename refuses"
        );
    }

    #[test]
    fn rename_via_db_refuses_when_a_dynamic_sourcer_reads_the_name() {
        // b.R has a dynamic source() *and* free-reads `foo`: it could resolve to
        // a.R at runtime, binding its `foo` read to the renamed definition. That
        // is a hidden reader we cannot rewrite, so the rename refuses (the `r == d`
        // arm of the name-keyed dynamic-source check).
        let a_src = "foo <- function() 1\n";
        let b_src = "p <- \"x.R\"\nsource(p)\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        assert!(
            rename_via_db(
                &snapshot,
                &ws_path("a.R"),
                &uri_a,
                &buf(a_src),
                offset,
                "renamed",
                PositionEncoding::Utf16
            )
            .is_none(),
            "a dynamic sourcer that reads the name could hide a reader of it: refuse"
        );
    }

    #[test]
    fn rename_via_db_refuses_when_a_name_reader_reaches_a_dynamic_sourcer() {
        // c.R free-reads `foo` and statically sources b.R, which has a dynamic
        // source(). b.R could load a.R at runtime, so c.R would transitively see
        // a.R's `foo` — a hidden reader. The dynamic sourcer itself never reads
        // `foo`, so this exercises the `seen_by(d)` arm, not just `r == d`.
        let a_src = "foo <- function() 1\n";
        let b_src = "p <- \"x.R\"\nsource(p)\n";
        let c_src = "source(\"b.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace3(a_src, b_src, c_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        assert!(
            rename_via_db(
                &snapshot,
                &ws_path("a.R"),
                &uri_a,
                &buf(a_src),
                offset,
                "renamed",
                PositionEncoding::Utf16
            )
            .is_none(),
            "a name-reader that can reach a dynamic sourcer could hide a read: refuse"
        );
    }

    #[test]
    fn rename_via_db_allows_rename_with_an_unrelated_dynamic_source() {
        // a.R defines `foo`; b.R sources a.R and reads it (a normal reader). c.R
        // has a dynamic source() but never reads `foo`, and nothing sources c.R,
        // so no reader of `foo` is in its reach. The dynamic source is irrelevant
        // to renaming `foo`: it no longer blocks the rename project-wide.
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let c_src = "p <- \"x.R\"\nsource(p)\n";
        let snapshot = rename_workspace3(a_src, b_src, c_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("an unrelated dynamic source must not block the rename");
        let changes = edit.changes.expect("changes present");
        assert_eq!(changes.get(&uri_a).expect("a.R definition edited").len(), 1);
        assert_eq!(changes.get(&uri_b).expect("b.R read edited").len(), 1);
    }

    #[test]
    fn rename_via_db_skips_a_pre_source_read() {
        // b.R reads `foo` at top level *before* sourcing a.R, so that read binds
        // to nothing (not a.R's foo). Rename rewrites a.R's definition and leaves
        // the pre-source read alone — b.R gets no edit (precise per-read
        // filtering, where the coarse model refused the whole rename).
        let a_src = "foo <- function() 1\n";
        let b_src = "before <- foo\nsource(\"a.R\")\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("the pre-source read is skipped, not refused");
        let changes = edit.changes.expect("changes present");
        assert_eq!(changes.get(&uri_a).expect("a.R definition edited").len(), 1);
        assert!(
            !changes.contains_key(&uri_b),
            "the pre-source read binds to nothing, so b.R is untouched"
        );
    }

    #[test]
    fn rename_via_db_mixed_reader_renames_only_the_post_source_read() {
        // b.R reads `foo` before *and* after sourcing a.R: the pre-source read
        // binds to nothing (skip), the post-source read binds to a.R's foo
        // (rename). Exactly one edit in b.R, on the second occurrence.
        let a_src = "foo <- function() 1\n";
        let b_src = "before <- foo\nsource(\"a.R\")\nafter <- foo\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("the post-source read is renamable even though a pre-source one isn't");
        let changes = edit.changes.expect("changes present");
        assert_eq!(changes.get(&uri_a).expect("a.R definition edited").len(), 1);
        let b_edits = changes.get(&uri_b).expect("b.R post-source read edited");
        assert_eq!(b_edits.len(), 1, "only the post-source read is renamed");
        // The edit lands on the *second* `foo` occurrence (after the source).
        let post_offset = b_src.rfind("foo").unwrap();
        let post_pos = pos_at(b_src, post_offset);
        assert_eq!(b_edits[0].range.start, post_pos);
    }

    #[test]
    fn rename_via_db_renames_body_read_skips_pre_source_read() {
        // b.R reads `foo` at top level before sourcing a.R (skip) and again inside
        // a function body (run at call time against the final scope → binds to
        // a.R's foo, rename). The body read is renamed; the pre-source one isn't.
        let a_src = "foo <- function() 1\n";
        let b_src = "before <- foo\nsource(\"a.R\")\ng <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("the body read binds to the final scope and renames");
        let changes = edit.changes.expect("changes present");
        let b_edits = changes.get(&uri_b).expect("b.R body read edited");
        assert_eq!(b_edits.len(), 1, "only the body read is renamed");
        let body_offset = b_src.rfind("foo").unwrap();
        assert_eq!(b_edits[0].range.start, pos_at(b_src, body_offset));
    }

    #[test]
    fn rename_via_db_skips_body_read_bound_to_source_shadow() {
        // b.R sources a.R then z.R, both defining `foo`; its function-body read
        // of foo runs against the final scope, which binds foo to z.R (last
        // sourced), not a.R. Renaming a.R's foo must not touch b.R's body read.
        let a_src = "foo <- function() 1\n";
        let z_src = "foo <- function() 2\n";
        let b_src = "source(\"a.R\")\nsource(\"z.R\")\ng <- function() foo()\n";
        let snapshot = rename_workspace_files(&[("a.R", a_src), ("z.R", z_src), ("b.R", b_src)]);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let uri_z = uri::from_path(&ws_path("z.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("a.R's foo is the only def in its cohort");
        let changes = edit.changes.expect("changes present");
        assert!(changes.contains_key(&uri_a), "a.R's definition is renamed");
        assert!(
            !changes.contains_key(&uri_b),
            "the body read binds to z.R's shadow, not a.R"
        );
        assert!(!changes.contains_key(&uri_z), "z.R's own foo is untouched");
    }

    #[test]
    fn rename_via_db_refuses_on_ambiguous_closure_definers() {
        // b.R sources d.R, which sources both a.R and c.R — both define `foo`.
        // Which one is live at b.R's read is order-dependent inside the closure,
        // so the read's binding is undecidable: the whole rename refuses.
        let a_src = "foo <- function() 1\n";
        let c_src = "foo <- function() 2\n";
        let d_src = "source(\"a.R\")\nsource(\"c.R\")\n";
        let b_src = "source(\"d.R\")\nx <- foo\n";
        let snapshot = rename_workspace_files(&[
            ("a.R", a_src),
            ("c.R", c_src),
            ("d.R", d_src),
            ("b.R", b_src),
        ]);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        assert!(
            rename_via_db(
                &snapshot,
                &ws_path("a.R"),
                &uri_a,
                &buf(a_src),
                offset,
                "renamed",
                PositionEncoding::Utf16
            )
            .is_none(),
            "an ambiguous (two-definer) closure read makes the rename refuse"
        );
    }

    #[test]
    fn rename_via_db_skips_a_clicked_pre_source_read() {
        // The cursor sits *on* the pre-source read in b.R (a bare free read that
        // resolves position-blind to a.R's foo). Rename rewrites the genuine
        // references and leaves the clicked token — which binds to nothing — alone.
        let a_src = "foo <- function() 1\nuse <- foo\n";
        let b_src = "before <- foo\nsource(\"a.R\")\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = b_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("b.R"),
            &uri_b,
            &buf(b_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("clicking a pre-source read still offers the rename for real references");
        let changes = edit.changes.expect("changes present");
        // a.R's definition and its own post-def read are renamed.
        assert_eq!(changes.get(&uri_a).expect("a.R edited").len(), 2);
        // b.R's clicked pre-source read is not a reference, so it is untouched.
        assert!(
            !changes.contains_key(&uri_b),
            "the clicked pre-source token binds to nothing and is skipped"
        );
    }

    #[test]
    fn rename_via_db_renames_a_top_level_read_after_the_source() {
        // b.R reads `foo` at top level *after* sourcing a.R, so it binds to a.R's
        // foo under load order: rename rewrites the definition and that read.
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nx <- foo\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("a post-source top-level read binds cleanly: rename is offered");
        let changes = edit.changes.expect("changes present");
        assert!(changes.contains_key(&uri_a), "the definition is renamed");
        assert!(
            changes.contains_key(&uri_b),
            "the post-source top-level read is renamed"
        );
    }

    #[test]
    fn references_via_db_overreports_a_pre_source_read() {
        // References is non-destructive, so where rename refuses on the
        // order-ambiguous pre-source read, references still reports it.
        let a_src = "foo <- function() 1\n";
        let b_src = "before <- foo\nsource(\"a.R\")\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let position = pos_at(a_src, a_src.find("foo").unwrap());

        let locations = references_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            position,
            true,
            PositionEncoding::Utf16,
        )
        .expect("references present");
        let uris = ref_uris(&locations);
        assert!(uris.contains(&uri_a));
        assert!(
            uris.contains(&uri_b),
            "references over-reports the pre-source read rename refuses"
        );
    }

    #[test]
    fn rename_via_db_bare_read_resolves_to_visible_def_only() {
        // c.R sources a.R (which defines foo); a separate disjoint file also
        // defines foo but c.R can't see it. A bare read in c.R resolves only to
        // a.R, and rename touches a.R + c.R, not the disjoint file.
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&ws_path("a.R"), "foo <- function() 1\n".to_string());
        let c = db.upsert_file(
            &ws_path("c.R"),
            "source(\"a.R\")\nbaz <- function() foo()\n".to_string(),
        );
        let d = db.upsert_file(&ws_path("d.R"), "foo <- function() 99\n".to_string());
        db.set_workspace_members(vec![a, c, d], vec![ws_root()]);
        let snapshot = db.snapshot();

        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_c = uri::from_path(&ws_path("c.R")).unwrap();
        let uri_d = uri::from_path(&ws_path("d.R")).unwrap();
        let c_src = "source(\"a.R\")\nbaz <- function() foo()\n";
        let offset = c_src.find("foo()").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("c.R"),
            &uri_c,
            &buf(c_src),
            offset,
            "renamed",
            PositionEncoding::Utf16,
        )
        .expect("the bare read resolves to a.R's foo");
        let changes = edit.changes.expect("changes present");

        assert!(changes.contains_key(&uri_a), "a.R's def is renamed");
        assert!(changes.contains_key(&uri_c), "c.R's read is renamed");
        assert!(
            !changes.contains_key(&uri_d),
            "d.R's disjoint foo is invisible to c.R and untouched"
        );
    }

    #[test]
    fn rename_via_db_bare_read_unresolved_returns_none() {
        // b.R reads `foo` but can't see any definition of it (no edge to a.R).
        // The bare read resolves to nothing, so rename refuses.
        let a_src = "foo <- function() 1\n";
        let b_src = "bar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = b_src.find("foo").unwrap();

        assert!(
            rename_via_db(
                &snapshot,
                &ws_path("b.R"),
                &uri_b,
                &buf(b_src),
                offset,
                "renamed",
                PositionEncoding::Utf16
            )
            .is_none(),
            "a bare read with no visible definition can't be renamed"
        );
    }

    #[test]
    fn rename_via_db_keeps_a_nested_local_intra_file() {
        // `x` is a local inside f's body, not a file-scope binding, so a same-named
        // free read in another file is unrelated and must not be touched.
        let a_src = "f <- function() {\n  x <- 1\n  x + 1\n}\n";
        let b_src = "g <- function() x\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let offset = a_src.find("x").unwrap();

        let edit = rename_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            offset,
            "y",
            PositionEncoding::Utf16,
        )
        .expect("rename is available on the local");
        let changes = edit.changes.expect("changes present");

        assert_eq!(changes.len(), 1, "only a.R is touched");
        let a_edits = changes.get(&uri_a).expect("the local def and read in a.R");
        assert_eq!(a_edits.len(), 2, "definition plus the one read");
        assert!(
            !changes.contains_key(&uri_b),
            "the sibling free read is unrelated"
        );
    }

    #[test]
    fn rename_via_db_declines_a_non_syntactic_new_name() {
        let a_src = "foo <- function() 1\n";
        let b_src = "bar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let offset = a_src.find("foo").unwrap();

        assert!(
            rename_via_db(
                &snapshot,
                &ws_path("a.R"),
                &uri_a,
                &buf(a_src),
                offset,
                "new name",
                PositionEncoding::Utf16,
            )
            .is_none(),
            "a non-syntactic new name is withheld (backtick-quoting is out of scope)"
        );
    }

    #[test]
    fn references_via_db_excludes_disjoint_same_name_defs() {
        // Two unconnected scripts each define `foo`. References to a.R's foo must
        // not include b.R's unrelated foo.
        let a_src = "foo <- function() 1\n";
        let b_src = "foo <- function() 2\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let position = pos_at(a_src, a_src.find("foo").unwrap());

        let locations = references_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            position,
            true,
            PositionEncoding::Utf16,
        )
        .expect("the definition has at least itself");
        let uris = ref_uris(&locations);
        assert!(uris.contains(&uri_a));
        assert!(!uris.contains(&uri_b), "b.R's foo is unrelated");
    }

    #[test]
    fn references_via_db_includes_source_connected_reader() {
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let position = pos_at(a_src, a_src.find("foo").unwrap());

        let locations = references_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            position,
            true,
            PositionEncoding::Utf16,
        )
        .expect("references present");
        let uris = ref_uris(&locations);
        assert!(uris.contains(&uri_a));
        assert!(uris.contains(&uri_b), "the source-connected read is found");
    }

    #[test]
    fn references_via_db_includes_package_siblings() {
        let (_dir, snapshot, a_path, b_path) =
            rename_package("foo <- function() 1\n", "bar <- function() foo()\n");
        let uri_a = uri::from_path(&a_path).unwrap();
        let uri_b = uri::from_path(&b_path).unwrap();
        let a_src = "foo <- function() 1\n";
        let position = pos_at(a_src, a_src.find("foo").unwrap());

        let locations = references_via_db(
            &snapshot,
            &a_path,
            &uri_a,
            &buf(a_src),
            position,
            true,
            PositionEncoding::Utf16,
        )
        .expect("references present");
        let uris = ref_uris(&locations);
        assert!(uris.contains(&uri_a));
        assert!(uris.contains(&uri_b), "the package sibling's read is found");
    }

    #[test]
    fn references_via_db_excludes_shadowing_reader() {
        // b.R sources a.R but defines its own foo, so its read binds to its own
        // def. References to a.R's foo must not include b.R.
        let a_src = "foo <- function() 1\n";
        let b_src = "source(\"a.R\")\nfoo <- function() 2\nbar <- function() foo()\n";
        let snapshot = rename_workspace(a_src, b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();
        let position = pos_at(a_src, a_src.find("foo").unwrap());

        let locations = references_via_db(
            &snapshot,
            &ws_path("a.R"),
            &uri_a,
            &buf(a_src),
            position,
            true,
            PositionEncoding::Utf16,
        )
        .expect("references present");
        let uris = ref_uris(&locations);
        assert!(uris.contains(&uri_a));
        assert!(
            !uris.contains(&uri_b),
            "b.R shadows foo, so it is not a reference to a.R's foo"
        );
    }

    #[test]
    fn references_via_db_reports_cohort_for_package_multidef() {
        // Package siblings both define foo: one flat slot. References reports the
        // whole cohort (both defs) — the same answer rename now rewrites.
        let (_dir, snapshot, a_path, b_path) =
            rename_package("foo <- function() 1\n", "foo <- function() 2\n");
        let uri_a = uri::from_path(&a_path).unwrap();
        let uri_b = uri::from_path(&b_path).unwrap();
        let a_src = "foo <- function() 1\n";
        let position = pos_at(a_src, a_src.find("foo").unwrap());

        let locations = references_via_db(
            &snapshot,
            &a_path,
            &uri_a,
            &buf(a_src),
            position,
            true,
            PositionEncoding::Utf16,
        )
        .expect("references present");
        let uris = ref_uris(&locations);
        assert!(uris.contains(&uri_a));
        assert!(
            uris.contains(&uri_b),
            "references reports the sibling def of the same flat slot"
        );
    }

    #[test]
    fn references_via_db_bare_read_resolves_to_visible_def_only() {
        // c.R sources a.R (defines foo); d.R defines a disjoint foo c.R can't see.
        // A bare read in c.R resolves to a.R only.
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&ws_path("a.R"), "foo <- function() 1\n".to_string());
        let c = db.upsert_file(
            &ws_path("c.R"),
            "source(\"a.R\")\nbaz <- function() foo()\n".to_string(),
        );
        let d = db.upsert_file(&ws_path("d.R"), "foo <- function() 99\n".to_string());
        db.set_workspace_members(vec![a, c, d], vec![ws_root()]);
        let snapshot = db.snapshot();

        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_c = uri::from_path(&ws_path("c.R")).unwrap();
        let uri_d = uri::from_path(&ws_path("d.R")).unwrap();
        let c_src = "source(\"a.R\")\nbaz <- function() foo()\n";
        let position = pos_at(c_src, c_src.find("foo()").unwrap());

        let locations = references_via_db(
            &snapshot,
            &ws_path("c.R"),
            &uri_c,
            &buf(c_src),
            position,
            true,
            PositionEncoding::Utf16,
        )
        .expect("the bare read resolves to a.R");
        let uris = ref_uris(&locations);
        assert!(uris.contains(&uri_a), "a.R's def is reported");
        assert!(uris.contains(&uri_c), "c.R's read is reported");
        assert!(
            !uris.contains(&uri_d),
            "d.R's disjoint foo is invisible to c.R"
        );
    }
}
