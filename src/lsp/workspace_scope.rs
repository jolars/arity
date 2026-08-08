use super::*;

/// Whether paths belong in the workspace member set, batch-amortized: **one disk
/// walk per touched root**, not one per path.
///
/// A path belongs if it sits under a tracked root and survives that root's
/// exclude config — that is, if the initial seed
/// ([`scope_members_at`](crate::linter::check::scope_members_at)) would have
/// found it, so the include rules can't drift between the seed and the
/// incremental updates. The deepest matching root wins (a nested package root is
/// more specific than its parent workspace root), and only that root is walked.
///
/// Deliberately short-lived: construct one per notification, query it, drop it.
/// The walk samples the filesystem *once* for the whole batch, which is exactly
/// right for the two callers ([`apply_r_membership`] and [`apply_file_renames`],
/// both handed changes that already landed on disk) and is why it must never be
/// cached across notifications.
pub(crate) struct WorkspaceScope {
    /// The tracked roots, normalized (see [`WorkspaceScope::new`]).
    roots: Vec<PathBuf>,
    /// Per-root scope, filled lazily on the first query that reaches that root.
    walked: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl WorkspaceScope {
    /// Snapshot `roots`, normalizing each so a root spelled with `.`/`..` still
    /// prefix-matches the queried paths. Walks nothing yet.
    pub(crate) fn new(roots: &[PathBuf]) -> Self {
        Self {
            roots: roots.iter().map(|r| normalize_path(r)).collect(),
            walked: HashMap::new(),
        }
    }

    /// Whether `path` belongs in the member set, walking its owning root on the
    /// first query that reaches it. A path under no root answers `false` without
    /// touching disk.
    ///
    /// Both sides are run through [`normalize_path`], matching how
    /// [`upsert_file`](IncrementalDatabase::upsert_file) keys the db itself.
    /// Normalization is lexical only, so a symlinked root (macOS `/var` vs.
    /// `/private/var`) still won't match — consistent with those db keys.
    pub(crate) fn contains(&mut self, path: &Path) -> bool {
        let path = normalize_path(path);
        let Some(root) = owning_root(&self.roots, &path) else {
            return false;
        };
        self.walked
            .entry(root.to_path_buf())
            .or_insert_with_key(|root| {
                crate::linter::check::scope_members_at(root)
                    .iter()
                    .map(|p| normalize_path(p))
                    .collect()
            })
            .contains(&path)
    }
}

/// The deepest tracked root that is a prefix of `path`, or `None` when it sits
/// under none. Deepest wins so a nested package root beats its parent workspace
/// root — the two can carry different `arity.toml` excludes.
fn owning_root<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a Path> {
    roots
        .iter()
        .filter(|r| path.starts_with(r))
        .max_by_key(|r| r.components().count())
        .map(PathBuf::as_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_prefers_the_deepest_root() {
        // The outer root excludes `pkg/`, the nested one doesn't. A file under
        // `pkg/` is judged by the *deeper* root's own config, so it is in scope.
        let dir = tempfile::tempdir().expect("tempdir");
        let outer = dir.path();
        std::fs::write(outer.join("arity.toml"), "exclude = [\"pkg/\"]\n").expect("outer config");
        let inner = outer.join("pkg");
        std::fs::create_dir_all(inner.join("R")).expect("pkg/R");
        std::fs::write(inner.join("arity.toml"), "").expect("inner config");
        let a = inner.join("R").join("a.R");
        std::fs::write(&a, "foo <- function() 1\n").expect("a.R");

        let roots = vec![outer.to_path_buf(), inner.clone()];
        assert!(
            WorkspaceScope::new(&roots).contains(&a),
            "the deepest root's config governs"
        );
        // With only the outer root tracked, its `exclude` drops the same file.
        assert!(
            !WorkspaceScope::new(&[outer.to_path_buf()]).contains(&a),
            "the outer root excludes pkg/"
        );
    }

    #[test]
    fn scope_rejects_a_path_under_no_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stray = if cfg!(windows) {
            PathBuf::from(r"C:\elsewhere\stray.R")
        } else {
            PathBuf::from("/elsewhere/stray.R")
        };
        assert!(!WorkspaceScope::new(&[dir.path().to_path_buf()]).contains(&stray));
    }

    #[test]
    fn scope_answers_repeated_queries_from_one_walk() {
        let (dir, _db, a) = seeded_package();
        let root = dir.path().to_path_buf();
        let b = dir.path().join("R").join("b.R");

        // The first query walks the root; `b.R` only appears afterwards, so the
        // memo answers it from that one snapshot rather than re-walking. Pinning
        // the negative is what proves there is a single walk per root.
        let mut scope = WorkspaceScope::new(std::slice::from_ref(&root));
        assert!(scope.contains(&a));
        std::fs::write(&b, "bar <- function() 2\n").expect("b.R");
        assert!(!scope.contains(&b), "one walk per root, taken up front");

        // A scope is per-notification, so the next one sees the new file.
        assert!(WorkspaceScope::new(&[root]).contains(&b));
    }
}
