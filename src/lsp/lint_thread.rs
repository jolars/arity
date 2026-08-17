use super::*;

/// A lint request handed to the dedicated lint thread.
pub(crate) struct LintRequest {
    pub(crate) uri: Uri,
    pub(crate) path: PathBuf,
    pub(crate) buffer: Arc<TextBuffer>,
    /// Precise per-change edits transforming the *previously sent* buffer into
    /// `text` (Stage B), in application order. Threaded to `parsed_document` for
    /// a precise multi-edit reparse; empty means "no hint, use `diff_edit`". The
    /// coalescing in [`LintWorker::enqueue`] concatenates these across superseded
    /// requests so none are lost.
    pub(crate) edits: Vec<Edit>,
    pub(crate) version: i32,
    /// Which grammar `buffer` is in, decided at `didOpen`. Picks the pipeline in
    /// [`LintWorker::start`]; the two share the queue, the in-flight slot, and
    /// the coalescing, but nothing below them.
    pub(crate) kind: DocumentKind,
    pub(crate) lint_config: LintConfig,
    pub(crate) index_config: IndexConfig,
}

pub(crate) enum LintMsg {
    // Boxed: `LintRequest` is much larger than the other variant, so boxing keeps
    // the enum (and every channel slot) small.
    Request(Box<LintRequest>),
    /// Seed the explicit workspace file-set from the discovered roots (sent once
    /// at startup). Handled on the lint thread, the sole db writer.
    SeedWorkspace {
        roots: Vec<PathBuf>,
    },
    /// Files were renamed on disk (`workspace/didRenameFiles`). Refresh the db's
    /// membership so cross-file analysis tracks the new paths. Handled on the lint
    /// thread, the sole db writer.
    RenameFiles {
        renames: Vec<(PathBuf, PathBuf)>,
    },
    /// On-disk files changed outside the editor (`workspace/didChangeWatchedFiles`):
    /// `.R` create/delete/change and `DESCRIPTION`/`NAMESPACE` edits. Refreshes db
    /// membership and package metadata so cross-file analysis tracks the new state.
    /// Handled on the lint thread, the sole db writer. (`arity.toml` changes are
    /// handled on the main loop, which owns the config cache.)
    WatchedFiles {
        batch: WatchedFilesBatch,
    },
}

/// Spawn the dedicated lint thread that owns the persistent salsa database.
pub(crate) fn spawn_lint_thread(
    lint_rx: Receiver<LintMsg>,
    read_rx: Receiver<ReadJob>,
    out_tx: Sender<Outbound>,
    read_spawner: Spawner,
    position_encoding: PositionEncoding,
) -> JoinHandle<()> {
    let (build_tx, build_rx) = crossbeam_channel::unbounded::<IndexedProvider>();
    let (remote_tx, remote_rx) = crossbeam_channel::unbounded::<RemoteExports>();
    let (done_tx, done_rx) = crossbeam_channel::unbounded::<AnalyzeDone>();
    std::thread::Builder::new()
        .name("arity-lint".to_string())
        .spawn(move || {
            // The single-thread index pool isolates the one unbounded-duration
            // job (background package harvesting) from the read pool, so a long
            // build can never starve a latency-sensitive read. Owned by the
            // worker, so its thread lives exactly as long as the lint thread.
            let mut worker = LintWorker {
                db: IncrementalDatabase::default(),
                index_loaded: HashSet::new(),
                index_attempts: HashSet::new(),
                remote_loaded: HashSet::new(),
                remote_attempts: HashSet::new(),
                out_tx,
                build_tx,
                remote_tx,
                done_tx,
                inflight: None,
                pending: HashMap::new(),
                read_spawner,
                index_pool: TaskPool::new("arity-index", 1),
                resolved_rules: None,
                position_encoding,
            };
            worker.run(&lint_rx, &read_rx, &build_rx, &remote_rx, &done_rx);
        })
        .expect("spawn lint thread")
}

/// Signal from a finished read-phase ([`LintWorker::spawn_analyze`]) back to the
/// lint thread: the analyze for `uri`@`version` has completed (or unwound on
/// cancellation) and dropped its db clone, so the in-flight slot is free.
pub(crate) struct AnalyzeDone {
    uri: Uri,
    version: i32,
}

/// The single in-flight read-phase analyze, if any.
pub(crate) struct InflightAnalyze {
    uri: Uri,
    version: i32,
}

/// What [`LintWorker::try_dispatch`] should do given the in-flight analyze and
/// the pending queue. Pure decision (see [`decide`]) so it can be unit-tested.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DispatchAction {
    /// Idle with nothing queued, or busy with no newer edit for the in-flight
    /// URI: leave the in-flight analyze running and wait for its `done`.
    Wait,
    /// The slot is free; start a fresh analyze for this URI.
    Start(Uri),
    /// A strictly-newer edit for the *in-flight* URI arrived; cancel the running
    /// analyze and start this URI. Only ever the in-flight URI — a different
    /// pending URI must never cancel the in-flight one (it would drop that
    /// file's diagnostics under `RelintAll`).
    SupersedeAndStart(Uri),
}

/// Decide the next dispatch action. `inflight` is the running analyze's
/// `(uri, version)`, if any; `pending` maps each queued URI to its latest
/// version. Cancel only on a strictly-newer edit of the *same* URI.
pub(crate) fn decide(inflight: Option<(&Uri, i32)>, pending: &HashMap<Uri, i32>) -> DispatchAction {
    match inflight {
        None => match pending.keys().next() {
            Some(uri) => DispatchAction::Start(uri.clone()),
            None => DispatchAction::Wait,
        },
        Some((uri, version)) => {
            if pending.get(uri).is_some_and(|&v| v > version) {
                DispatchAction::SupersedeAndStart(uri.clone())
            } else {
                DispatchAction::Wait
            }
        }
    }
}

/// Run `f` on the lint thread, catching any panic so a single malformed request
/// can't take down the sole salsa-db writer and, with it, the whole server. This
/// mirrors the read pool's per-job `catch_unwind` (see [`task_pool`]); the lint
/// thread and main loop were the two places a panic still meant process death.
/// Returns `true` if `f` ran to completion, `false` if it panicked (logged). The
/// db's internal mutexes recover from poisoning (see `IncrementalDatabase`), so a
/// panic mid-write leaves the db usable for the next request rather than bricked.
/// Also used by the main loop (`server::main_loop`) to isolate request handlers.
pub(crate) fn guard(label: &str, f: impl FnOnce()) -> bool {
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => true,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&'static str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<non-string panic payload>");
            log::error!("lint thread caught panic in {label}: {msg}");
            false
        }
    }
}

pub(crate) struct LintWorker {
    db: IncrementalDatabase,
    /// Workspace anchors whose index cache has already been loaded into the salsa
    /// [`LibraryIndex`] singleton.
    index_loaded: HashSet<PathBuf>,
    /// Packages a background harvest has already been scheduled for this session
    /// — never retried, so a not-installed package doesn't loop.
    index_attempts: HashSet<SmolStr>,
    /// Workspace anchors whose remote-sidecar disk cache has already been warmed
    /// into the salsa [`LibraryIndex`]'s `remote` field.
    remote_loaded: HashSet<PathBuf>,
    /// Packages a remote-sidecar fetch has already been attempted for this session
    /// — never retried, so a package absent from the sidecar doesn't loop.
    remote_attempts: HashSet<SmolStr>,
    out_tx: Sender<Outbound>,
    /// A finished background harvest sends its freshly-loaded index here; the
    /// lint thread (sole writer) installs it into salsa at HIGH durability.
    build_tx: Sender<IndexedProvider>,
    /// A finished sidecar fetch sends its freshly-fetched names-only batch here;
    /// the lint thread (sole writer) merges and reinstalls it at HIGH durability.
    remote_tx: Sender<RemoteExports>,
    /// Read-phase workers signal completion here so the lint thread can free the
    /// in-flight slot and dispatch the next pending lint.
    done_tx: Sender<AnalyzeDone>,
    /// The single in-flight read-phase analyze, if any. At most one runs at a
    /// time: the write-phase needs exclusive `&mut db`, and salsa cancellation is
    /// global, so a second concurrent analyze couldn't be canceled selectively.
    inflight: Option<InflightAnalyze>,
    /// Coalesced lint queue: the latest pending request per URI. Persists across
    /// `select!` iterations (it used to be a per-iteration local).
    pending: HashMap<Uri, LintRequest>,
    /// Submit-side handle onto the read pool, shared with the main loop. Used for
    /// read jobs (formatting, hover) and the analyze read-phase.
    read_spawner: Spawner,
    /// Single-thread pool that isolates background package indexing — the one
    /// unbounded-duration job — from the read pool.
    index_pool: TaskPool,
    /// The resolved rule set, cached across keystrokes and rebuilt only when the
    /// lint config changes. Resolving instantiates the rule registry and derives
    /// the dispatch table + severity map ([`ResolvedRules`]); doing it once per
    /// config keeps that off the per-keystroke path. Shared into each
    /// [`prepare_document_in_project`](crate::linter::check::prepare_document_in_project)
    /// as an `Arc`. `None` until the first lint; the tuple's first element is the
    /// config it was resolved for.
    resolved_rules: Option<(LintConfig, Arc<ResolvedRules>)>,
    /// The session's negotiated position encoding, used when rendering
    /// diagnostics and passed to each read job so its LSP positions match.
    position_encoding: PositionEncoding,
}

impl LintWorker {
    fn run(
        &mut self,
        lint_rx: &Receiver<LintMsg>,
        read_rx: &Receiver<ReadJob>,
        build_rx: &Receiver<IndexedProvider>,
        remote_rx: &Receiver<RemoteExports>,
        done_rx: &Receiver<AnalyzeDone>,
    ) {
        loop {
            select! {
                recv(lint_rx) -> msg => {
                    let Ok(msg) = msg else { break };
                    // Coalesce: keep only the latest version per URI, so a fast
                    // typist's stale edits are dropped before they're ever linted.
                    // A `SeedWorkspace` is applied inline (it's the db writer).
                    // Guarded: a panic in one request must not kill the thread.
                    guard("lint message", || {
                        self.handle_lint_msg(msg);
                        while let Ok(m) = lint_rx.try_recv() {
                            self.handle_lint_msg(m);
                        }
                        self.try_dispatch();
                    });
                }
                recv(done_rx) -> done => {
                    let Ok(done) = done else { continue };
                    guard("analyze done", || {
                        // Free the slot only if this `done` is for the *current*
                        // in-flight analyze — a late `done` from a superseded one
                        // (different version) must not clear the new analyze.
                        if matches!(&self.inflight, Some(f) if f.uri == done.uri && f.version == done.version) {
                            self.inflight = None;
                        }
                        self.try_dispatch();
                    });
                }
                recv(read_rx) -> job => {
                    let Ok(job) = job else { continue };
                    // Mint a short-lived read-only snapshot and run the job off the
                    // lint thread. The clone is dropped inside `run_read`, so the
                    // next write isn't blocked once the read finishes (or a racing
                    // write trips `salsa::Cancelled`, handled by the fallback).
                    guard("read job dispatch", || {
                        let snapshot = self.db.snapshot();
                        let encoding = self.position_encoding;
                        self.read_spawner
                            .spawn(move || run_read(snapshot, encoding, job));
                    });
                }
                recv(build_rx) -> built => {
                    let Ok(indexed) = built else { continue };
                    guard("index install", || {
                        // Sole writer installs the freshly-harvested index at HIGH
                        // durability, then re-lints every open document against it.
                        self.db.set_library_index(indexed);
                        let _ = self.out_tx.send(Outbound::RelintAll);
                    });
                }
                recv(remote_rx) -> fetched => {
                    let Ok(fetched) = fetched else { continue };
                    guard("sidecar install", || {
                        // Merge the freshly-fetched names into the live sidecar and
                        // reinstall it (HIGH durability), then re-lint every document.
                        let mut merged = self
                            .db
                            .remote_exports()
                            .map(|a| (*a).clone())
                            .unwrap_or_default();
                        merged.merge_from(fetched);
                        self.db.set_remote_exports(merged);
                        let _ = self.out_tx.send(Outbound::RelintAll);
                    });
                }
            }
        }
    }

    /// Dispatch a lint-channel message: queue a request, or apply a workspace
    /// seed inline (the lint thread is the sole db writer).
    fn handle_lint_msg(&mut self, msg: LintMsg) {
        match msg {
            LintMsg::Request(req) => self.enqueue(*req),
            LintMsg::SeedWorkspace { roots } => self.seed_workspace(roots),
            LintMsg::RenameFiles { renames } => self.rename_files(renames),
            LintMsg::WatchedFiles { batch } => self.on_watched_files(batch),
        }
    }

    /// Walk the workspace roots once and install the discovered `.R` files as the
    /// explicit [`Workspace`](crate::incremental::Workspace) file-set, unioned with
    /// anything already tracked. Pre-warms cross-file membership so later edits
    /// don't re-walk (see [`seed_workspace_for`](crate::linter::check::seed_workspace_for)).
    fn seed_workspace(&mut self, roots: Vec<PathBuf>) {
        let mut files: Vec<SourceFile> = self
            .db
            .workspace()
            .map(|ws| ws.members(&self.db).to_vec())
            .unwrap_or_default();
        for root in &roots {
            for path in crate::linter::check::scope_members_at(root) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    files.push(self.db.upsert_file(&path, text));
                }
            }
        }
        self.db.set_workspace_members(files, roots);
    }

    /// Refresh db membership after a `workspace/didRenameFiles`, then re-lint
    /// every open document so `source()` edges re-resolve against the new graph.
    /// The db mutation is [`apply_file_renames`].
    fn rename_files(&mut self, renames: Vec<(PathBuf, PathBuf)>) {
        if apply_file_renames(&mut self.db, &renames) {
            let _ = self.out_tx.send(Outbound::RelintAll);
        }
    }

    /// Apply a `workspace/didChangeWatchedFiles` batch to the db, then re-lint if
    /// anything changed. `.R` content changes to tracked-but-unopened files refresh
    /// their text; `.R` create/delete adjusts membership (which cascades a package
    /// graph refresh); a bare `NAMESPACE` edit, or a `DESCRIPTION` event that moves
    /// package-ness, refreshes the graph on its own, while an edit to a tracked
    /// root's `DESCRIPTION` re-reads only that file. See [`WatchedFilesBatch`].
    fn on_watched_files(&mut self, batch: WatchedFilesBatch) {
        let mut relint = false;

        // Content changed on disk for a member that isn't open in the editor (open
        // buffers are authoritative and were filtered out by the classifier).
        for path in &batch.r_changed {
            if let Ok(text) = std::fs::read_to_string(path) {
                self.db.upsert_file(path, text);
                relint = true;
            }
        }

        // Create/delete reshapes membership; that already refreshes the package
        // graph (see `set_workspace_members`), so a bare metadata edit only needs a
        // standalone refresh when membership didn't move.
        let member_changed = if batch.r_created.is_empty() && batch.r_deleted.is_empty() {
            false
        } else {
            apply_r_membership(&mut self.db, &batch.r_created, &batch.r_deleted)
        };
        relint |= member_changed;

        if !member_changed {
            // Refresh only what actually moved. A `NAMESPACE` change reshapes
            // the package graph, and so does a `DESCRIPTION` whose arrival or
            // departure changes which directories are packages
            // ([`description_edit_is_local`]). An edit to a known root's
            // `DESCRIPTION` needs nothing but its own text re-read.
            let mut refresh_graph = false;
            for (path, kind) in &batch.meta_changed {
                match kind {
                    WatchedKind::Namespace => refresh_graph = true,
                    WatchedKind::Description => match path.parent() {
                        Some(root)
                            if description_edit_is_local(
                                path,
                                self.db.lookup_description(root).is_some(),
                            ) =>
                        {
                            let root = root.to_path_buf();
                            relint |= self.db.refresh_description(&root);
                        }
                        _ => refresh_graph = true,
                    },
                    _ => {}
                }
            }
            if refresh_graph {
                relint |= self.db.refresh_package_graph();
            }
        }

        // A `DESCRIPTION` (or `man/roxygen/meta.R`) edit may flip the
        // package-wide roxygen markdown default; re-resolve it for every
        // tracked file. Unchanged resolutions write nothing, so the common
        // case invalidates no parse. A `NAMESPACE` change cannot flip it.
        if batch.touches_roxygen_options() {
            relint |= self.db.refresh_roxygen_markdown();
        }

        if relint {
            let _ = self.out_tx.send(Outbound::RelintAll);
        }
    }

    /// Add `req` to the pending queue, keeping the highest version per URI (guards
    /// against an out-of-order lower version clobbering a newer one). Superseding
    /// a pending request **prepends** its still-unconsumed edits to `req`'s, so
    /// the precise Stage-B edit sequence spanning the last-parsed buffer to `req`
    /// stays intact across coalescing (an incomplete sequence merely fails the
    /// `reparse_edits` guard and falls back to `diff_edit`, never miscomputes).
    fn enqueue(&mut self, mut req: LintRequest) {
        match self.pending.remove(&req.uri) {
            Some(existing) if existing.version >= req.version => {
                // Out-of-order stale request: keep the newer pending one as-is.
                self.pending.insert(req.uri.clone(), existing);
            }
            Some(mut existing) => {
                existing.edits.append(&mut req.edits);
                req.edits = existing.edits;
                self.pending.insert(req.uri.clone(), req);
            }
            None => {
                self.pending.insert(req.uri.clone(), req);
            }
        }
    }

    /// Start lints until the slot is occupied or the queue is exhausted (see
    /// [`decide`]). Cancels the in-flight analyze only when superseded by a newer
    /// edit of the *same* URI. Loops because a [`start`](Self::start) that hits a
    /// parse error spawns no worker (and thus no `done`), so the next pending URI
    /// must be picked up here rather than stalling until the next event — this is
    /// what keeps a multi-URI `RelintAll` draining.
    fn try_dispatch(&mut self) {
        loop {
            let versions: HashMap<Uri, i32> = self
                .pending
                .iter()
                .map(|(uri, req)| (uri.clone(), req.version))
                .collect();
            let inflight = self.inflight.as_ref().map(|f| (&f.uri, f.version));
            let uri = match decide(inflight, &versions) {
                DispatchAction::Wait => return,
                DispatchAction::Start(uri) => uri,
                DispatchAction::SupersedeAndStart(uri) => {
                    // Explicit cancellation: the write-phase may be a no-op (an
                    // unchanged `upsert_file` doesn't bump the revision), so we
                    // can't rely on it to unwind the running analyze. Blocks until
                    // the old clone drops; safe — this thread holds no clone.
                    self.db.trigger_cancellation();
                    self.inflight = None;
                    uri
                }
            };
            let Some(req) = self.pending.remove(&uri) else {
                return;
            };
            // A spawned worker occupies the slot; stop. Otherwise (parse error /
            // bad config) the slot is still free, so loop to the next pending URI.
            if self.start(req) {
                return;
            }
        }
    }

    /// Resolve the rule set for `config`, reusing the cached `Arc` when the config
    /// is unchanged (the common keystroke case). On the first lint or a config
    /// change it re-resolves and re-caches; an unknown rule ID surfaces as `Err`
    /// and is *not* cached, so a corrected config on the next lint re-resolves.
    fn resolved_rules(&mut self, config: &LintConfig) -> Result<Arc<ResolvedRules>, LintError> {
        if let Some((cfg, rules)) = &self.resolved_rules
            && cfg == config
        {
            return Ok(Arc::clone(rules));
        }
        let (rules, unknown) = ResolvedRules::resolve(config);
        if let Some(rule) = unknown.into_iter().next() {
            return Err(LintError::UnknownRule { rule });
        }
        let rules = Arc::new(rules);
        self.resolved_rules = Some((config.clone(), Arc::clone(&rules)));
        Ok(rules)
    }

    /// Run one lint: the write-phase (`&mut db`, on this thread) then the
    /// read-phase analyze on the read pool holding a db clone. Returning to
    /// `select!` right after spawning keeps reads responsive (problem 2) and lets
    /// a fresher edit cancel the analyze (problem 1).
    ///
    /// Returns `true` if a worker was spawned (the in-flight slot is now busy),
    /// `false` if the buffer couldn't be linted (no worker, slot still free).
    fn start(&mut self, req: LintRequest) -> bool {
        match req.kind {
            DocumentKind::R => self.start_r(req),
            DocumentKind::Description => self.start_description(req),
        }
    }

    /// The R pipeline: parse + model + cross-file rules over a `SourceFile`.
    fn start_r(&mut self, mut req: LintRequest) -> bool {
        let anchor = req
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        self.ensure_index(&anchor, &req.index_config);
        self.ensure_remote(&anchor, &req.index_config);

        // Write-phase: push the live buffer + sibling files into the persistent
        // db. Cheap — the parse/model are lazy salsa queries deferred to analyze.
        let active = self.db.upsert_file(&req.path, req.buffer.text_arc());
        // Stage the precise per-change edits (Stage B) for the parse this upsert
        // will force below (via `prepare_document_in_project`). Overwrites any
        // unconsumed sequence; `parsed_document` verifies they reconstruct the
        // buffer before use, so a stale/empty sequence simply falls back to
        // `diff_edit`.
        self.db.stage_edits(active, std::mem::take(&mut req.edits));
        // Ensure the active file's project is in the workspace file-set. Lazy:
        // only walks disk when the file isn't already a member (the initialize
        // seed covers the common case), so discovery leaves the keystroke path.
        let already_member = self
            .db
            .workspace()
            .is_some_and(|ws| ws.members(&self.db).contains(&active));
        if !already_member {
            let exclude = crate::linter::check::resolve_exclude_at(&anchor);
            crate::linter::check::seed_workspace_for(&mut self.db, &req.path, active, &exclude);
        }
        // Resolve the rule set (cached across keystrokes; rebuilt only on a config
        // change). An unknown-rule config error clears stale diagnostics and runs
        // no worker, leaving the slot free.
        let rules = match self.resolved_rules(&req.lint_config) {
            Ok(rules) => rules,
            Err(_) => {
                self.publish_empty(&req);
                return false;
            }
        };
        let prepared = match crate::linter::check::prepare_document_in_project(
            &mut self.db,
            &req.path,
            active,
            rules,
        ) {
            Some(prepared) => prepared,
            // Parse errors (None): the rules can't run, but the parser's
            // diagnostics are still published so the editor shows the error.
            None => {
                self.publish_parse_diagnostics(&req, active);
                return false;
            }
        };

        // `auto_build` reads the buffer + the current salsa index and mutates
        // `index_attempts`, so it stays on the lint thread; it spawns its own
        // background build, whose result is installed back here on `build_rx`.
        // The enclosing package's declared dependencies count as referenced even
        // when no `.R` file mentions them: they are exactly what the file is
        // entitled to use, and the undefined-symbol gates need them enumerable.
        let declared = self.declared_packages_for(&req.path);
        if req.index_config.auto_build {
            self.maybe_build(&anchor, &req.index_config, req.buffer.text(), &declared);
        }
        // Fetch names-only exports for referenced packages the offline tiers don't
        // cover, when a sidecar URL is configured. Background, like `maybe_build`.
        self.maybe_fetch_remote(&anchor, &req.index_config, req.buffer.text(), &declared);

        // The snapshot carries the salsa library index, so `analyze_prepared`
        // resolves undefined symbols through it; this provider is only the
        // fallback for rules that read static base-R facts (`is_base`).
        let fallback = CompositeProvider::base_only();
        let buffer = Arc::clone(&req.buffer);
        self.spawn_analyze(&req, buffer, move |analysis| {
            crate::linter::check::analyze_prepared(analysis, &prepared, &fallback)
        })
    }

    /// The `DESCRIPTION` pipeline: DCF parse + the packaging rules.
    ///
    /// Gated on the file sitting at its own package root, which mirrors the
    /// CLI's *walk* policy rather than its explicit-path one. The CLI lints any
    /// `DESCRIPTION` a user names, but an editor sends every file a user so much
    /// as glances at — a grep hit, a git diff, a testthat fixture package — and
    /// answering those with a screenful of missing-field findings is noise about
    /// a file whose author was never addressing us.
    fn start_description(&mut self, req: LintRequest) -> bool {
        // An `untitled:` buffer has no root and so no package to be part of.
        // `send_lint` synthesizes a bare `DESCRIPTION` for it, whose parent is
        // the empty path.
        let Some(root) = req.path.parent().filter(|p| !p.as_os_str().is_empty()) else {
            self.publish_empty(&req);
            return false;
        };
        if !crate::file_discovery::is_own_package_root(root) {
            self.publish_empty(&req);
            return false;
        }
        let root = root.to_path_buf();

        // Seeding walks the package's `R/` files into the db, which is what lets
        // `package_usage_for` answer at all (and so what keeps
        // `unused-dependency` from being silently inert in the editor).
        //
        // This MUST happen before the buffer is written below: seeding runs
        // `refresh_package_graph`, which re-reads every root's `DESCRIPTION`
        // from disk — and would silently overwrite the live buffer with the
        // saved bytes.
        if self.db.lookup_description(&root).is_none() {
            self.seed_workspace(vec![root.clone()]);
        }

        // Make the open buffer authoritative in salsa, so an unsaved dependency
        // already counts for the package's R files. Fan out only when the
        // *facts* moved: `DescriptionFacts` is the range-free `Eq` firewall, so
        // typing in `Description:` prose costs one DCF parse while typing in
        // `Imports:` re-lints the package. The clone is required — the borrow
        // ends at the `&mut db` write below.
        let before = self
            .db
            .lookup_description(&root)
            .map(|file| crate::incremental::description_facts(&self.db, file).clone());
        let (file, _) = self.db.upsert_description(&root, req.buffer.text_arc());
        let after = crate::incremental::description_facts(&self.db, file);
        if before.as_ref() != Some(after) {
            // A `Roxygen` field change can flip the package-wide markdown
            // default, which every roxygen block in `R/` is parsed against.
            self.db.refresh_roxygen_markdown();
            // Terminates after exactly one extra generation: `request_relint_all`
            // re-lints this document too, and `upsert_description` short-circuits
            // on unchanged text, so the second pass finds `before == after`.
            let _ = self.out_tx.send(Outbound::RelintAll);
        }

        self.ensure_index(&root, &req.index_config);
        self.ensure_remote(&root, &req.index_config);
        // A `DESCRIPTION` names exactly the packages its author is entitled to
        // use, so opening one warms the harvest for them — no R source needed.
        let declared = self.declared_packages_for(&req.path);
        if req.index_config.auto_build {
            self.maybe_build(&root, &req.index_config, "", &declared);
        }
        self.maybe_fetch_remote(&root, &req.index_config, "", &declared);

        let rules = match self.resolved_rules(&req.lint_config) {
            Ok(rules) => rules,
            Err(_) => {
                self.publish_empty(&req);
                return false;
            }
        };

        let buffer = Arc::clone(&req.buffer);
        let path = req.path.clone();
        let content = req.buffer.text_arc();
        self.spawn_analyze(&req, buffer, move |analysis| {
            crate::linter::check::check_description_in_project(analysis, &path, &content, &rules)
        })
    }

    /// Occupy the in-flight slot and run `analyze` on the read pool against a db
    /// clone, publishing its findings for `req` and freeing the slot once that
    /// clone has dropped.
    ///
    /// The read-phase shape is the same for both grammars, so this is the one
    /// place it lives — including the drop-before-`done` ordering, which is not
    /// optional (see below). A superseding edit (or any write) trips
    /// `salsa::Cancelled`, caught here so a canceled analyze publishes nothing;
    /// the main loop's version gate is the backstop.
    ///
    /// Returns `true`: a worker was spawned, so the in-flight slot is now busy.
    fn spawn_analyze(
        &mut self,
        req: &LintRequest,
        buffer: Arc<TextBuffer>,
        analyze: impl FnOnce(&Analysis) -> Vec<Diagnostic> + Send + 'static,
    ) -> bool {
        let snapshot = self.db.snapshot();
        let out_tx = self.out_tx.clone();
        let done_tx = self.done_tx.clone();
        let uri = req.uri.clone();
        let version = req.version;
        self.inflight = Some(InflightAnalyze {
            uri: uri.clone(),
            version,
        });

        let encoding = self.position_encoding;
        self.read_spawner.spawn(move || {
            let result = salsa::Cancelled::catch(AssertUnwindSafe(|| analyze(&snapshot)));
            if let Ok(diagnostics) = result {
                let line_index = buffer.line_index();
                let diags: Vec<LspDiagnostic> = diagnostics
                    .iter()
                    .map(|d| to_lsp_diagnostic(d, line_index, encoding))
                    .collect();
                let _ = out_tx.send(Outbound::Diagnostics {
                    uri: uri.clone(),
                    version,
                    diags,
                    findings: Arc::new(diagnostics),
                });
            }
            // The clone MUST drop before we signal `done`: `trigger_cancellation`
            // / the next write-phase blocks until it's gone, so a premature `done`
            // could let the lint thread start a write that deadlocks on this clone.
            drop(snapshot);
            let _ = done_tx.send(AnalyzeDone { uri, version });
        });
        true
    }

    /// Publish the parser's diagnostics for `active` as `syntax-error` findings.
    /// Used when parse errors block the lint rules: the rules can't run on a
    /// broken tree, but the error must still reach the editor rather than being
    /// cleared. The findings are cached (like lint findings) so the pull path
    /// answers with them too.
    fn publish_parse_diagnostics(&self, req: &LintRequest, active: SourceFile) {
        let findings =
            crate::linter::syntax_error_diagnostics(self.db.parse_diagnostics(active), &req.path);
        let line_index = req.buffer.line_index();
        let diags: Vec<LspDiagnostic> = findings
            .iter()
            .map(|d| to_lsp_diagnostic(d, line_index, self.position_encoding))
            .collect();
        let _ = self.out_tx.send(Outbound::Diagnostics {
            uri: req.uri.clone(),
            version: req.version,
            diags,
            findings: Arc::new(findings),
        });
    }

    /// Publish empty diagnostics for `req` (clears any stale findings) without
    /// running a worker. Used when the buffer can't be linted (bad config),
    /// mirroring the old early-return that always sent diagnostics.
    fn publish_empty(&self, req: &LintRequest) {
        let _ = self.out_tx.send(Outbound::Diagnostics {
            uri: req.uri.clone(),
            version: req.version,
            diags: Vec::new(),
            findings: Arc::new(Vec::new()),
        });
    }

    /// Load the index cache for `anchor` into the salsa [`LibraryIndex`] the
    /// first time we see that workspace. Idempotent per anchor. Runs on the lint
    /// thread (sole writer); the HIGH-durability set means subsequent keystrokes
    /// don't revalidate the library subgraph.
    fn ensure_index(&mut self, anchor: &Path, cfg: &IndexConfig) {
        if self.index_loaded.contains(anchor) {
            return;
        }
        let indexed = match resolve_cache_root(None, cfg.cache_dir.as_deref()) {
            Ok(root) => IndexedProvider::from_cache(&Cache::new(root)),
            Err(_) => IndexedProvider::empty(),
        };
        // Unlike a background build, this needs no `RelintAll`: it runs in the
        // write-phase of the first lint for this anchor, so nothing has been
        // analyzed against the empty index yet. Inlay hints are the exception —
        // one may already have been answered, and they have no push channel.
        let loaded_any = indexed.packages().next().is_some();
        self.db.set_library_index(indexed);
        self.index_loaded.insert(anchor.to_path_buf());
        if loaded_any {
            let _ = self.out_tx.send(Outbound::RefreshInlayHints);
        }
    }

    /// Spawn a background harvest for the document's unknown packages. On success
    /// the freshly-loaded index is sent back on `build_tx` for the lint thread to
    /// install. The "already indexed?" check reads the current salsa index.
    /// The packages the `DESCRIPTION` of the package enclosing `path` declares.
    /// Pure: it reads the tracked `DESCRIPTION` input, never disk. Empty for a
    /// loose script outside any package.
    fn declared_packages_for(&self, path: &Path) -> Vec<SmolStr> {
        let Some(root) = crate::project::package_root(path) else {
            return Vec::new();
        };
        let Some(file) = self.db.lookup_description(&root) else {
            return Vec::new();
        };
        crate::incremental::description_facts(&self.db, file)
            .declared_packages()
            .iter()
            .map(SmolStr::new)
            .collect()
    }

    fn maybe_build(
        &mut self,
        anchor: &Path,
        cfg: &IndexConfig,
        source: &str,
        declared: &[SmolStr],
    ) {
        let current = self.db.library_data();
        let empty = IndexedProvider::empty();
        let indexed = current.as_deref().unwrap_or(&empty);
        let to_build = packages_to_build(&mut self.index_attempts, indexed, source, declared);
        if to_build.is_empty() {
            return;
        }
        let Ok(cache_root) = resolve_cache_root(None, cfg.cache_dir.as_deref()) else {
            return;
        };
        let cfg = cfg.clone();
        let anchor = anchor.to_path_buf();
        let build_tx = self.build_tx.clone();
        let out_tx = self.out_tx.clone();
        self.index_pool.spawn(move || {
            // `build_index` harvests packages in parallel (`rayon`), so it's opaque
            // to per-package progress; report it indeterminately (begin/end only).
            let reporter = ProgressReporter::begin(
                out_tx,
                "Indexing R packages",
                Some(package_count(to_build.len())),
            );
            let now = now_unix_secs();
            let cache = Cache::new(cache_root);
            let search = LibrarySearch::discover(Some(&anchor), &cfg.library_paths);
            let report = build_index(
                &to_build,
                &cache,
                &search,
                BuildOptions {
                    help: cfg.help,
                    force: false,
                    // Background builds honor only the per-user env consent.
                    attach_probe: crate::rindex::attach_probe::enabled_by_env(),
                },
                now,
            );
            let newly = report.newly_indexed().count();
            if newly > 0 {
                let _ = build_tx.send(IndexedProvider::from_cache(&cache));
            }
            reporter.end(Some(format!("Indexed {}", package_count(newly))));
        });
    }

    /// Warm the remote sidecar's disk cache into the salsa [`LibraryIndex`]'s
    /// `remote` field the first time we see a workspace, when a sidecar URL is
    /// configured. Idempotent per anchor, network-free (disk only). Runs on the
    /// lint thread (sole writer); HIGH durability means later keystrokes don't
    /// revalidate resolution.
    fn ensure_remote(&mut self, anchor: &Path, cfg: &IndexConfig) {
        let Some(url) = cfg.remote_url.as_deref() else {
            return;
        };
        if !self.remote_loaded.insert(anchor.to_path_buf()) {
            return;
        }
        let Ok(root) = resolve_cache_root(None, cfg.cache_dir.as_deref()) else {
            return;
        };
        let cached = Sidecar::http(url, &root).load_cached();
        if !cached.is_empty() {
            self.db.set_remote_exports(cached);
        }
    }

    /// Spawn a background sidecar fetch for the document's referenced packages
    /// that the offline tiers (base, harvested, bundled) don't already cover. On
    /// success the fetched names-only batch is sent back on `remote_tx` for the
    /// lint thread to merge and install. No-op unless a sidecar URL is configured.
    fn maybe_fetch_remote(
        &mut self,
        anchor: &Path,
        cfg: &IndexConfig,
        source: &str,
        declared: &[SmolStr],
    ) {
        let Some(url) = cfg.remote_url.as_deref() else {
            return;
        };
        let empty_index = IndexedProvider::empty();
        let index = self.db.library_data();
        let index = index.as_deref().unwrap_or(&empty_index);
        let empty_remote = RemoteExports::new();
        let remote = self.db.remote_exports();
        let remote = remote.as_deref().unwrap_or(&empty_remote);
        let to_fetch =
            packages_to_fetch(&mut self.remote_attempts, index, remote, source, declared);
        if to_fetch.is_empty() {
            return;
        }
        let Ok(cache_root) = resolve_cache_root(None, cfg.cache_dir.as_deref()) else {
            return;
        };
        let url = url.to_string();
        let _ = anchor;
        let remote_tx = self.remote_tx.clone();
        let out_tx = self.out_tx.clone();
        self.index_pool.spawn(move || {
            // The fetch loop is our own code, so report determinate per-package
            // progress (a percentage across `to_fetch`).
            let total = to_fetch.len();
            let reporter = ProgressReporter::begin(
                out_tx,
                "Fetching R package exports",
                Some(package_count(total)),
            );
            let mut sidecar = Sidecar::http(url, &cache_root);
            let mut fetched = RemoteExports::new();
            let mut any = false;
            for (i, pkg) in to_fetch.into_iter().enumerate() {
                let percentage = (((i + 1) * 100) / total.max(1)) as u32;
                reporter.report(pkg.to_string(), Some(percentage));
                if let Some(names) = sidecar.package_names(&pkg) {
                    fetched.insert_package(pkg, names);
                    any = true;
                }
            }
            if any {
                let _ = remote_tx.send(fetched);
            }
            reporter.end(None);
        });
    }
}

/// Packages to fetch from the sidecar for `source`: everything it references —
/// plus everything the enclosing package's `DESCRIPTION` declares, which is why
/// a dependency mentioned in no `.R` file still gets fetched — minus what the
/// offline tiers already cover (base/default packages, locally
/// harvested packages, and the bundled CRAN list — all captured by
/// [`package_indexed`]) and minus what we've already attempted this session.
/// Marks the returned packages as attempted so they aren't fetched twice.
pub(crate) fn packages_to_fetch(
    attempts: &mut HashSet<SmolStr>,
    indexed: &IndexedProvider,
    remote: &RemoteExports,
    source: &str,
    declared: &[SmolStr],
) -> Vec<SmolStr> {
    referenced_in_source(source)
        .into_iter()
        .chain(declared.iter().cloned())
        .filter(|pkg| !package_indexed(indexed, remote, pkg) && attempts.insert(pkg.clone()))
        .collect()
}

/// Packages to harvest for `source`: the always-attached default packages plus
/// everything `source` references and everything the enclosing package
/// declares, minus what we already hold a *harvested*
/// index for and minus what we've already attempted this session. Marks the
/// returned packages as attempted so they aren't built twice.
///
/// The skip test is [`IndexedProvider::has_package`] (do we have the rich,
/// harvested data?), not mere name-resolvability: the default packages and the
/// bundled-CRAN packages resolve by name from static lists, but those carry no
/// help or formals, so they still need a real harvest for hover and signatures.
pub(crate) fn packages_to_build(
    attempts: &mut HashSet<SmolStr>,
    indexed: &IndexedProvider,
    source: &str,
    declared: &[SmolStr],
) -> Vec<SmolStr> {
    // A referenced meta-package also attaches its members (harvested attach
    // set, static table fallback), and the undefined-symbol gates require each
    // member to be indexed — so queue them for harvest too. A member learned
    // only from a harvested attach set arrives one pass late by construction:
    // pass 1 harvests the meta, the installed index triggers a re-lint, and
    // pass 2 sees the meta's attaches (its members weren't marked attempted).
    let mut candidates: Vec<SmolStr> = Vec::new();
    let referenced: Vec<SmolStr> = referenced_in_source(source)
        .into_iter()
        .chain(declared.iter().cloned())
        .collect();
    for pkg in with_default_packages(referenced) {
        for member in attach_members(indexed, &pkg) {
            let member = SmolStr::new(member);
            if !candidates.contains(&member) {
                candidates.push(member);
            }
        }
        if !candidates.contains(&pkg) {
            candidates.push(pkg);
        }
    }
    candidates
        .into_iter()
        .filter(|pkg| !indexed.has_package(pkg) && attempts.insert(pkg.clone()))
        .collect()
}

/// Whether a watched `DESCRIPTION` event can be applied by re-reading that one
/// file, instead of refreshing the whole package graph.
///
/// `tracked` is whether the db already holds an input for the file's root. Both
/// halves are about *package-ness* moving, which is what the graph records:
///
/// - **An untracked root** may have just become a package — the file is new.
/// - **A file no longer on disk** may have just stopped being one. The watcher
///   collapses create/change/delete into one event, so a deletion arrives here
///   looking exactly like an edit; without the disk check it would only blank
///   the tracked text and leave the graph pointing at a package that is gone.
fn description_edit_is_local(path: &Path, tracked: bool) -> bool {
    tracked && path.is_file()
}

/// Format a package count for a progress message, pluralizing the noun
/// ("1 package", "3 packages").
fn package_count(n: usize) -> String {
    format!("{n} package{}", if n == 1 { "" } else { "s" })
}

pub(crate) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The watcher reports a create, an edit, and a delete as the same event, so
    /// the decision has to be read off the disk state, not off the event.
    #[test]
    fn a_description_delete_is_not_a_local_edit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("DESCRIPTION");
        std::fs::write(&path, "Package: p\n").expect("DESCRIPTION");

        assert!(
            description_edit_is_local(&path, true),
            "an edit to a tracked root's DESCRIPTION only re-reads that file"
        );
        assert!(
            !description_edit_is_local(&path, false),
            "an untracked root may have just become a package"
        );

        // The regression this guards: a deleted DESCRIPTION means the directory
        // may no longer be a package at all, which only the graph refresh sees.
        std::fs::remove_file(&path).expect("remove");
        assert!(
            !description_edit_is_local(&path, true),
            "a deleted DESCRIPTION reshapes package roots, tracked or not"
        );
    }

    #[test]
    fn guard_contains_a_panic_and_reports_completion() {
        // A panicking closure is caught (returns false) so the lint thread's
        // `select!` loop keeps running instead of taking the whole server down;
        // a normal closure runs to completion (returns true).
        assert!(!guard("test", || panic!("boom")));
        let mut ran = false;
        assert!(guard("test", || ran = true));
        assert!(ran);
    }

    #[test]
    fn decide_idle_starts_a_pending_uri() {
        let a = uri_named("a.R");
        let pending = HashMap::from([(a.clone(), 1)]);
        assert_eq!(decide(None, &pending), DispatchAction::Start(a));
    }

    #[test]
    fn decide_idle_empty_queue_waits() {
        let pending: HashMap<Uri, i32> = HashMap::new();
        assert_eq!(decide(None, &pending), DispatchAction::Wait);
    }

    #[test]
    fn decide_supersedes_same_uri_newer_version() {
        let a = uri_named("a.R");
        let pending = HashMap::from([(a.clone(), 2)]);
        assert_eq!(
            decide(Some((&a, 1)), &pending),
            DispatchAction::SupersedeAndStart(a)
        );
    }

    #[test]
    fn decide_waits_when_pending_same_uri_not_newer() {
        // A duplicate / same-version request for the in-flight URI must not
        // restart it.
        let a = uri_named("a.R");
        let pending = HashMap::from([(a.clone(), 1)]);
        assert_eq!(decide(Some((&a, 1)), &pending), DispatchAction::Wait);
    }

    #[test]
    fn decide_never_cancels_a_different_uri() {
        // The core RelintAll guard: with A in flight and only *other* URIs
        // queued, we wait for A's `done` — we never cancel A to start B/C, which
        // would silently drop A's diagnostics.
        let a = uri_named("a.R");
        let pending = HashMap::from([(uri_named("b.R"), 5), (uri_named("c.R"), 9)]);
        assert_eq!(decide(Some((&a, 1)), &pending), DispatchAction::Wait);
    }

    #[test]
    fn decide_relint_all_drains_one_uri_at_a_time() {
        // Simulate a multi-URI RelintAll: each file is dispatched only once the
        // slot is free, and `decide` never returns SupersedeAndStart for a URI
        // other than the in-flight one.
        let (a, b, c) = (uri_named("a.R"), uri_named("b.R"), uri_named("c.R"));
        let mut pending = HashMap::from([(a.clone(), 1), (b.clone(), 1), (c.clone(), 1)]);

        // Idle: start some URI.
        let DispatchAction::Start(first) = decide(None, &pending) else {
            panic!("expected Start");
        };
        assert!(pending.contains_key(&first));
        pending.remove(&first);

        // Busy with `first`, two others still queued → wait, never supersede.
        let action = decide(Some((&first, 1)), &pending);
        assert_eq!(action, DispatchAction::Wait);

        // first's `done` frees the slot; the next URI starts. Repeat to drain.
        let mut started = vec![first];
        while !pending.is_empty() {
            let DispatchAction::Start(next) = decide(None, &pending) else {
                panic!("expected Start");
            };
            pending.remove(&next);
            started.push(next);
        }
        started.sort_by_key(|u| u.as_str().to_string());
        assert_eq!(started, {
            let mut all = vec![a, b, c];
            all.sort_by_key(|u| u.as_str().to_string());
            all
        });
    }

    #[test]
    fn packages_to_build_covers_defaults_and_unharvested_deps() {
        let mut attempts = HashSet::new();
        let indexed = indexed_dplyr();
        // dplyr is already harvested (skipped). The default packages and any
        // referenced-but-unharvested package (stats is a default; notarealpkg
        // is neither default nor harvested) still need a build for rich data.
        let src = "library(dplyr)\nlibrary(stats)\nlibrary(notarealpkg)\n";
        let first = packages_to_build(&mut attempts, &indexed, src, &[]);
        assert!(
            !first.contains(&SmolStr::new("dplyr")),
            "harvested dep skipped"
        );
        for default in crate::semantic::symbols::default_packages() {
            assert!(
                first.contains(&SmolStr::new(*default)),
                "default package {default} should be built, got {first:?}"
            );
        }
        assert!(first.contains(&SmolStr::new("notarealpkg")));
        // A second pass returns nothing — every package was already attempted.
        let second = packages_to_build(&mut attempts, &indexed, src, &[]);
        assert!(second.is_empty(), "expected no re-attempt, got {second:?}");
    }

    #[test]
    fn packages_to_build_expands_meta_package_members() {
        use crate::rindex::schema::{PackageIndex, SCHEMA_VERSION};
        // A harvested meta-package whose attach set names an un-harvested
        // member: the member must be queued for build too, or the
        // undefined-symbol gates would suppress files indefinitely.
        let meta = PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "metaverse".into(),
            version: "1.0".into(),
            lib_path: "/lib".into(),
            title: None,
            r_version: None,
            harvested_at: 0,
            attaches: vec!["helperpkg".into()],
            symbols: Vec::new(),
        };
        let indexed = IndexedProvider::from_indices([meta]);
        let mut attempts = HashSet::new();
        let got = packages_to_build(&mut attempts, &indexed, "library(metaverse)\n", &[]);
        assert!(
            got.contains(&SmolStr::new("helperpkg")),
            "harvested attach member should be queued, got {got:?}"
        );
        assert!(
            !got.contains(&SmolStr::new("metaverse")),
            "the meta itself is already harvested"
        );

        // Static fallback: an un-harvested tidyverse queues the curated
        // members alongside itself.
        let mut attempts = HashSet::new();
        let got = packages_to_build(
            &mut attempts,
            &IndexedProvider::empty(),
            "library(tidyverse)\n",
            &[],
        );
        assert!(got.contains(&SmolStr::new("tidyverse")));
        assert!(
            got.contains(&SmolStr::new("dplyr")),
            "static members should be queued, got {got:?}"
        );
    }

    #[test]
    fn packages_to_fetch_skips_offline_covered_and_marks_attempts() {
        let mut attempts = HashSet::new();
        let indexed = indexed_dplyr();
        let mut remote = RemoteExports::new();
        remote.insert_package("alreadyremote", [SmolStr::new("x")]);
        // dplyr: locally harvested. data.table: bundled. stats: a default package.
        // alreadyremote: already in the sidecar. notonremote: genuinely uncovered.
        let src = "library(dplyr)\nlibrary(data.table)\nlibrary(stats)\nlibrary(alreadyremote)\nlibrary(notonremote)\n";
        let first = packages_to_fetch(&mut attempts, &indexed, &remote, src, &[]);
        assert!(
            first.contains(&SmolStr::new("notonremote")),
            "uncovered package fetched, got {first:?}"
        );
        for skip in ["dplyr", "data.table", "stats", "alreadyremote"] {
            assert!(
                !first.contains(&SmolStr::new(skip)),
                "{skip} is offline-covered and must not be fetched, got {first:?}"
            );
        }
        // Second pass: nothing left to attempt.
        let second = packages_to_fetch(&mut attempts, &indexed, &remote, src, &[]);
        assert!(second.is_empty(), "expected no re-attempt, got {second:?}");
    }
}
