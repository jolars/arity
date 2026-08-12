use super::*;

/// The glob patterns registered with the client for `didChangeWatchedFiles`. R
/// sources drive workspace membership; `arity.toml` reshapes config; `DESCRIPTION`
/// and `NAMESPACE` reshape package metadata — all of which affect cross-file
/// analysis when they change on disk.
pub(crate) const WATCHED_GLOBS: [&str; 4] = [
    "**/*.{R,r}",
    "**/arity.toml",
    "**/DESCRIPTION",
    "**/NAMESPACE",
];

/// A batch of on-disk changes from `workspace/didChangeWatchedFiles`, already
/// converted to filesystem paths and split by what each one forces the lint
/// thread (the sole db writer) to redo. `arity.toml` changes are handled on the
/// main loop — it owns the config cache — so they never reach here (see
/// [`WatchedClassification::config_changed`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WatchedFilesBatch {
    /// `.R`/`.r` files created on disk — add to the workspace member set.
    pub(crate) r_created: Vec<PathBuf>,
    /// `.R`/`.r` files deleted on disk — drop from the member set.
    pub(crate) r_deleted: Vec<PathBuf>,
    /// `.R`/`.r` files whose content changed on disk and are **not** open in the
    /// editor — re-`upsert_file` from disk. Open buffers are authoritative, so the
    /// classifier filters them out here (see [`classify_watched_files`]).
    pub(crate) r_changed: Vec<PathBuf>,
    /// Package metadata that changed on disk, with what each path is. Carrying
    /// the paths rather than a single bool is what lets the lint thread refresh
    /// only what moved: a `DESCRIPTION` edit for a known root re-reads that one
    /// file, where a `NAMESPACE` change still reshapes the package graph.
    pub(crate) meta_changed: Vec<(PathBuf, WatchedKind)>,
}

impl WatchedFilesBatch {
    /// Nothing for the lint thread to do.
    pub(crate) fn is_empty(&self) -> bool {
        self.r_created.is_empty()
            && self.r_deleted.is_empty()
            && self.r_changed.is_empty()
            && self.meta_changed.is_empty()
    }

    /// Whether anything here can flip the package-wide roxygen markdown
    /// default: the `Roxygen` field of a `DESCRIPTION`, or `man/roxygen/meta.R`.
    /// A `NAMESPACE` change cannot, so it must not trigger the re-resolve.
    pub(crate) fn touches_roxygen_options(&self) -> bool {
        self.meta_changed
            .iter()
            .any(|(_, kind)| matches!(kind, WatchedKind::Description | WatchedKind::RoxygenMeta))
    }
}

/// The result of classifying a `didChangeWatchedFiles` batch: the lint-thread
/// work ([`batch`](Self::batch)), plus whether an `arity.toml` changed (handled
/// on the main loop, which owns the config cache).
pub(crate) struct WatchedClassification {
    pub(crate) batch: WatchedFilesBatch,
    /// An `arity.toml` was created, changed, or deleted; the main loop clears its
    /// config cache and re-lints open documents so the new settings take effect.
    pub(crate) config_changed: bool,
}

/// What a watched path is, by name/extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchedKind {
    /// An R source (`.R`/`.r`).
    RSource,
    /// A package `DESCRIPTION`.
    Description,
    /// A package `NAMESPACE`.
    Namespace,
    /// `man/roxygen/meta.R` — roxygen2 *options*, not source.
    RoxygenMeta,
    /// `arity.toml` (configuration).
    Config,
    /// Anything else (ignored — a watcher glob may over-match).
    Other,
}

fn classify_path(path: &Path) -> WatchedKind {
    // `arity.toml`/`DESCRIPTION`/`NAMESPACE` match by exact file name; R sources
    // by extension. Name wins (a file literally named `NAMESPACE` has no ext).
    match path.file_name().and_then(|n| n.to_str()) {
        Some("arity.toml") => return WatchedKind::Config,
        Some("DESCRIPTION") => return WatchedKind::Description,
        Some("NAMESPACE") => return WatchedKind::Namespace,
        _ => {}
    }
    // `man/roxygen/meta.R` is an `.R` file by extension but carries roxygen2
    // *options* (it can flip the package-wide markdown default), not source:
    // treat it as package metadata so an edit re-resolves the flag.
    if path.ends_with("man/roxygen/meta.R") {
        return WatchedKind::RoxygenMeta;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("R") | Some("r") => WatchedKind::RSource,
        _ => WatchedKind::Other,
    }
}

/// Classify a `didChangeWatchedFiles` batch. `is_open` reports whether a URI is
/// an open editor buffer; a disk *change* to an open file is dropped, because the
/// buffer (kept current by `didChange`) is authoritative for that file. Non-`file`
/// URIs and unrecognized paths are ignored.
pub(crate) fn classify_watched_files(
    params: &DidChangeWatchedFilesParams,
    is_open: impl Fn(&Uri) -> bool,
) -> WatchedClassification {
    let mut batch = WatchedFilesBatch::default();
    let mut config_changed = false;
    for ev in &params.changes {
        let Some(path) = uri::to_path(&ev.uri) else {
            continue;
        };
        match classify_path(&path) {
            WatchedKind::RSource => {
                let t = ev.typ;
                if t == FileChangeType::CREATED {
                    batch.r_created.push(path);
                } else if t == FileChangeType::DELETED {
                    batch.r_deleted.push(path);
                } else if t == FileChangeType::CHANGED && !is_open(&ev.uri) {
                    batch.r_changed.push(path);
                }
            }
            // A create/change/delete all reduce to the same refresh; the db
            // re-reads whatever is (or is not) on disk now.
            kind @ (WatchedKind::Description
            | WatchedKind::Namespace
            | WatchedKind::RoxygenMeta) => batch.meta_changed.push((path, kind)),
            WatchedKind::Config => config_changed = true,
            WatchedKind::Other => {}
        }
    }
    WatchedClassification {
        batch,
        config_changed,
    }
}

/// Add created `.R` files to, and drop deleted ones from, the db's workspace
/// member set, then reinstall it (which refreshes the package graph via
/// [`set_workspace_members`](IncrementalDatabase::set_workspace_members)). A
/// created file is only added if the workspace scope (excludes applied) would
/// include it, so a generated/vendored source (`renv/`, …) doesn't leak in; the
/// whole batch is judged against one [`WorkspaceScope`], so a mass create costs
/// one disk walk per touched root rather than one per file.
/// Deleted files are dropped from the set but their [`SourceFile`] input lingers,
/// the same posture as [`apply_file_renames`]. Returns whether membership
/// actually changed. No-op when no workspace is seeded.
pub(crate) fn apply_r_membership(
    db: &mut IncrementalDatabase,
    created: &[PathBuf],
    deleted: &[PathBuf],
) -> bool {
    let Some(ws) = db.workspace() else {
        return false;
    };
    let mut members: Vec<SourceFile> = ws.members(db).to_vec();
    let roots = ws.roots(db).to_vec();

    let mut changed = false;
    for path in deleted {
        if let Some(old) = db.lookup_file(path) {
            let before = members.len();
            members.retain(|&m| m != old);
            changed |= members.len() != before;
        }
    }
    let mut scope = WorkspaceScope::new(&roots);
    for path in created {
        if !scope.contains(path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let file = db.upsert_file(path, text);
        if !members.contains(&file) {
            members.push(file);
            changed = true;
        }
    }

    if changed {
        db.set_workspace_members(members, roots);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::FileEvent;

    fn ws_uri(path: &str) -> Uri {
        let p = if cfg!(windows) {
            PathBuf::from(format!(r"C:\ws\{path}"))
        } else {
            PathBuf::from(format!("/ws/{path}"))
        };
        uri::from_path(&p).expect("file uri")
    }

    fn event(path: &str, typ: FileChangeType) -> FileEvent {
        FileEvent::new(ws_uri(path), typ)
    }

    fn classify(changes: Vec<FileEvent>, open: &[&str]) -> WatchedClassification {
        let open_uris: HashSet<Uri> = open.iter().map(|n| ws_uri(n)).collect();
        classify_watched_files(&DidChangeWatchedFilesParams { changes }, |uri| {
            open_uris.contains(uri)
        })
    }

    #[test]
    fn splits_r_sources_by_change_type() {
        let c = classify(
            vec![
                event("R/new.R", FileChangeType::CREATED),
                event("R/gone.R", FileChangeType::DELETED),
                event("R/edited.R", FileChangeType::CHANGED),
            ],
            &[],
        );
        assert_eq!(c.batch.r_created.len(), 1);
        assert_eq!(c.batch.r_deleted.len(), 1);
        assert_eq!(c.batch.r_changed.len(), 1);
        assert!(c.batch.meta_changed.is_empty());
        assert!(!c.config_changed);
    }

    #[test]
    fn a_disk_change_to_an_open_file_is_dropped() {
        // `edited.R` is open in the editor, so its disk change is ignored (the
        // buffer wins).
        let c = classify(
            vec![event("edited.R", FileChangeType::CHANGED)],
            &["edited.R"],
        );
        assert!(
            c.batch.is_empty(),
            "open-file change dropped: {:?}",
            c.batch
        );
    }

    #[test]
    fn package_metadata_is_classified_by_kind() {
        // The kinds stay apart so the lint thread can refresh only what moved:
        // a NAMESPACE reshapes the package graph, a DESCRIPTION usually does not,
        // and only the latter two can flip the roxygen markdown default.
        let cases = [
            ("DESCRIPTION", WatchedKind::Description, true),
            ("NAMESPACE", WatchedKind::Namespace, false),
            ("man/roxygen/meta.R", WatchedKind::RoxygenMeta, true),
        ];
        for (name, kind, roxygen) in cases {
            let c = classify(vec![event(name, FileChangeType::CHANGED)], &[]);
            assert_eq!(
                c.batch
                    .meta_changed
                    .iter()
                    .map(|(_, k)| *k)
                    .collect::<Vec<_>>(),
                vec![kind],
                "{name}"
            );
            assert_eq!(
                c.batch.touches_roxygen_options(),
                roxygen,
                "{name} roxygen options"
            );
            assert!(!c.config_changed);
        }
    }

    #[test]
    fn arity_toml_sets_config_changed_only() {
        let c = classify(vec![event("arity.toml", FileChangeType::CHANGED)], &[]);
        assert!(c.config_changed);
        assert!(c.batch.is_empty(), "config change is not lint-thread work");
    }

    #[test]
    fn unrelated_files_are_ignored() {
        let c = classify(vec![event("README.md", FileChangeType::CHANGED)], &[]);
        assert!(c.batch.is_empty());
        assert!(!c.config_changed);
    }

    #[test]
    fn apply_r_membership_adds_created_and_drops_deleted() {
        let (dir, mut db, a) = seeded_package();
        let b = dir.path().join("R").join("b.R");
        std::fs::write(&b, "bar <- function() 2\n").expect("b.R");

        // A newly-created sibling under R/ joins the member set.
        assert!(apply_r_membership(&mut db, std::slice::from_ref(&b), &[]));
        let b_file = db.lookup_file(&b).expect("b.R tracked");
        assert!(db.workspace().unwrap().members(&db).contains(&b_file));

        // Deleting the original member drops it from the set.
        let a_file = db.lookup_file(&a).expect("a.R tracked");
        assert!(apply_r_membership(&mut db, &[], std::slice::from_ref(&a)));
        assert!(!db.workspace().unwrap().members(&db).contains(&a_file));
    }

    #[test]
    fn apply_r_membership_ignores_a_create_outside_any_root() {
        let (_dir, mut db, _a) = seeded_package();
        let stray = if cfg!(windows) {
            PathBuf::from(r"C:\elsewhere\stray.R")
        } else {
            PathBuf::from("/elsewhere/stray.R")
        };
        assert!(
            !apply_r_membership(&mut db, std::slice::from_ref(&stray), &[]),
            "a file outside every tracked root is not added"
        );
        assert!(db.lookup_file(&stray).is_none());
    }

    #[test]
    fn apply_r_membership_ignores_a_create_excluded_by_config() {
        // `renv/` is in `DEFAULT_EXCLUDE`, so a vendored source appearing under
        // the root is neither added nor tracked — the same posture as a file
        // outside every root.
        let (dir, mut db, _a) = seeded_package();
        let renv = dir.path().join("renv");
        std::fs::create_dir(&renv).expect("renv/");
        let activate = renv.join("activate.R");
        std::fs::write(&activate, "invisible(NULL)\n").expect("activate.R");

        assert!(
            !apply_r_membership(&mut db, std::slice::from_ref(&activate), &[]),
            "an excluded create does not join the member set"
        );
        assert!(db.lookup_file(&activate).is_none());
    }

    #[test]
    fn apply_r_membership_adds_a_batch_of_creates() {
        // A whole batch is judged against one `WorkspaceScope`, so every sibling
        // still lands (behavioral stand-in for the one-walk-per-root property).
        let (dir, mut db, _a) = seeded_package();
        let r_dir = dir.path().join("R");
        let created: Vec<PathBuf> = ["b.R", "c.R", "d.R"]
            .iter()
            .map(|name| {
                let path = r_dir.join(name);
                std::fs::write(&path, "x <- 1\n").expect("create");
                path
            })
            .collect();

        assert!(apply_r_membership(&mut db, &created, &[]));
        let members = db.workspace().unwrap().members(&db).to_vec();
        for path in &created {
            let file = db.lookup_file(path).expect("tracked");
            assert!(members.contains(&file), "{} joined the set", path.display());
        }
    }
}
