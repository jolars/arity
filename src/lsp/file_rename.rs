use super::*;

/// Resolve `workspace/willRenameFiles` against a db `snapshot`: rewrite the
/// `source("old")` literals in dependents to each renamed target, via
/// [`Analysis::source_rename_edits`]. Each `(sourcer, literal range, new
/// literal)` triple becomes a [`TextEdit`] positioned against the sourcer's
/// tracked text (reusing [`text_edit_in`]); they are grouped per URI and wrapped
/// like a rename. The salsa read is wrapped in [`salsa::Cancelled::catch`] (a
/// write may race the snapshot), yielding `None` on cancellation.
pub(crate) fn will_rename_via_db(
    snapshot: &Analysis,
    renames: &[(PathBuf, PathBuf)],
) -> Option<WorkspaceEdit> {
    let edits =
        salsa::Cancelled::catch(AssertUnwindSafe(|| snapshot.source_rename_edits(renames))).ok()?;
    let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
    for (sourcer, range, new_text) in edits {
        if let Some((uri, edit)) = text_edit_in(snapshot, &sourcer, range, &new_text) {
            changes.entry(uri).or_default().push(edit);
        }
    }
    finalize_rename(changes)
}

/// Apply on-disk file renames to the db's workspace membership: track each new
/// path (read from disk, where the move already landed) and swap it in for the
/// old member. The old [`SourceFile`] input lingers — there is no removal
/// primitive — but is dropped from the member set, so cross-file scope ignores
/// it, the same posture as a closed file. Returns whether anything changed (so
/// the caller can skip a needless re-lint). No-op when no workspace is seeded.
pub(crate) fn apply_file_renames(
    db: &mut IncrementalDatabase,
    renames: &[(PathBuf, PathBuf)],
) -> bool {
    let Some(ws) = db.workspace() else {
        return false;
    };
    let mut members: Vec<SourceFile> = ws.members(db).to_vec();
    let roots = ws.roots(db).to_vec();

    let mut changed = false;
    for (old, new) in renames {
        let Ok(text) = std::fs::read_to_string(new) else {
            continue;
        };
        let new_file = db.upsert_file(new, text);
        if let Some(old_file) = db.lookup_file(old) {
            members.retain(|&f| f != old_file);
        }
        members.push(new_file);
        changed = true;
    }
    if changed {
        db.set_workspace_members(members, roots);
    }
    changed
}

/// Convert `RenameFilesParams` into `(old, new)` filesystem path pairs, dropping
/// any entry whose URIs aren't parseable `file:` URIs.
pub(crate) fn file_renames_to_paths(params: &RenameFilesParams) -> Vec<(PathBuf, PathBuf)> {
    params
        .files
        .iter()
        .filter_map(|f| {
            let old = f
                .old_uri
                .parse::<Uri>()
                .ok()
                .and_then(|u| uri::to_path(&u))?;
            let new = f
                .new_uri
                .parse::<Uri>()
                .ok()
                .and_then(|u| uri::to_path(&u))?;
            Some((old, new))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn will_rename_rewrites_dependent_source_literal() {
        // b.R sources a.R; renaming a.R rewrites b.R's literal and edits nothing
        // else (a.R itself has no incoming `source()` to rewrite).
        let b_src = "source(\"a.R\")\nbar <- function() foo()\n";
        let snapshot = rename_workspace("foo <- function() 1\n", b_src);
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();

        let edit = will_rename_via_db(&snapshot, &[(ws_path("a.R"), ws_path("a_renamed.R"))])
            .expect("the dependent literal is rewritten");
        let (_, new_text) = sole_edit(&edit, &uri_b);
        assert_eq!(new_text, "\"a_renamed.R\"");
        assert!(
            edit.changes.as_ref().unwrap().get(&uri_a).is_none(),
            "the renamed file itself is not edited"
        );
    }

    #[test]
    fn will_rename_preserves_single_quotes() {
        let b_src = "source('a.R')\n";
        let snapshot = rename_workspace("foo <- function() 1\n", b_src);
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();

        let edit =
            will_rename_via_db(&snapshot, &[(ws_path("a.R"), ws_path("a2.R"))]).expect("rewritten");
        assert_eq!(sole_edit(&edit, &uri_b).1, "'a2.R'");
    }

    #[test]
    fn will_rename_recomputes_relative_path_across_directories() {
        // Move R/a.R into R/sub/; b.R's relative literal becomes "sub/a.R".
        let (_dir, snapshot, a_path, b_path) = rename_package(
            "foo <- function() 1\n",
            "source(\"a.R\")\nbar <- function() foo()\n",
        );
        let uri_b = uri::from_path(&b_path).unwrap();
        let new_a = a_path.parent().unwrap().join("sub").join("a.R");

        let edit = will_rename_via_db(&snapshot, &[(a_path, new_a)]).expect("rewritten");
        assert_eq!(sole_edit(&edit, &uri_b).1, "\"sub/a.R\"");
    }

    #[test]
    fn will_rename_applies_a_batch_of_renames() {
        // b.R sources both a.R and c.R; renaming both in one request rewrites both
        // literals, merged and sorted into b.R's edit list.
        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&ws_path("a.R"), "foo <- function() 1\n".to_string());
        let c = db.upsert_file(&ws_path("c.R"), "qux <- function() 2\n".to_string());
        let b = db.upsert_file(
            &ws_path("b.R"),
            "source(\"a.R\")\nsource(\"c.R\")\n".to_string(),
        );
        db.set_workspace_members(vec![a, b, c], vec![ws_root()]);
        let snapshot = db.snapshot();
        let uri_b = uri::from_path(&ws_path("b.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[
                (ws_path("a.R"), ws_path("a2.R")),
                (ws_path("c.R"), ws_path("c2.R")),
            ],
        )
        .expect("both literals rewritten");
        let edits = edit.changes.unwrap().remove(&uri_b).expect("b.R edited");
        let texts: Vec<&str> = edits.iter().map(|e| e.new_text.as_str()).collect();
        assert_eq!(texts, vec!["\"a2.R\"", "\"c2.R\""], "sorted by position");
    }

    #[test]
    fn will_rename_leaves_dynamic_source_untouched() {
        // A computed `source()` target can't be resolved, so it has no reverse
        // edge and is never rewritten.
        let snapshot = rename_workspace("foo <- function() 1\n", "source(paste0(d, \"a.R\"))\n");
        assert!(
            will_rename_via_db(&snapshot, &[(ws_path("a.R"), ws_path("a2.R"))]).is_none(),
            "a dynamic source() is not rewritten"
        );
    }

    #[test]
    fn will_rename_ignores_a_noop_rename() {
        let snapshot = rename_workspace("foo <- function() 1\n", "source(\"a.R\")\n");
        assert!(
            will_rename_via_db(&snapshot, &[(ws_path("a.R"), ws_path("a.R"))]).is_none(),
            "renaming a file to itself produces no edits"
        );
    }

    #[test]
    fn apply_file_renames_swaps_membership() {
        // didRenameFiles refresh: after a move, the new path is a tracked member
        // and the old path is no longer one.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let old = root.join("a.R");
        let new = root.join("b.R");
        std::fs::write(&old, "foo <- function() 1\n").expect("write a.R");

        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&old, "foo <- function() 1\n".to_string());
        db.set_workspace_members(vec![a], vec![root.to_path_buf()]);

        // The move lands on disk before didRenameFiles arrives.
        std::fs::rename(&old, &new).expect("move a.R -> b.R");
        assert!(apply_file_renames(&mut db, &[(old.clone(), new.clone())]));

        let new_file = db.lookup_file(&new).expect("new path is tracked");
        let members = db.workspace().unwrap().members(&db).to_vec();
        assert!(members.contains(&new_file), "new path is a member");
        assert!(!members.contains(&a), "old member is dropped from the set");
    }
}
