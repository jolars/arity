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
    encoding: PositionEncoding,
) -> Option<WorkspaceEdit> {
    let edits =
        salsa::Cancelled::catch(AssertUnwindSafe(|| snapshot.source_rename_edits(renames))).ok()?;
    let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
    for (sourcer, range, new_text) in edits {
        if let Some((uri, edit)) = text_edit_in(snapshot, &sourcer, range, &new_text, encoding) {
            changes.entry(uri).or_default().push(edit);
        }
    }
    finalize_rename(changes)
}

/// Apply on-disk file renames to the db's workspace membership: track each new
/// path (read from disk, where the move already landed) and swap it in for the
/// old member. A *directory* rename is fanned out over the members beneath it by
/// [`expand_dir_renames`], so a folder move swaps every file it carried.
///
/// The old [`SourceFile`] input lingers — there is no removal primitive — but is
/// dropped from the member set, so cross-file scope ignores it, the same posture
/// as a closed file; a folder move just does that N times over. Reading the *new*
/// path is deliberate: the client has already applied the `willRenameFiles` edits
/// there. Returns whether anything changed (so the caller can skip a needless
/// re-lint). No-op when no workspace is seeded.
pub(crate) fn apply_file_renames(
    db: &mut IncrementalDatabase,
    renames: &[(PathBuf, PathBuf)],
) -> bool {
    let Some(ws) = db.workspace() else {
        return false;
    };
    let mut members: Vec<SourceFile> = ws.members(db).to_vec();
    let roots = ws.roots(db).to_vec();
    let known: Vec<PathBuf> = members
        .iter()
        .filter_map(|&f| db.file_path(f).map(Path::to_path_buf))
        .collect();
    let expanded = expand_dir_renames(renames, known.iter().map(PathBuf::as_path));

    let mut changed = false;
    for (old, new) in &expanded {
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

        let edit = will_rename_via_db(
            &snapshot,
            &[(ws_path("a.R"), ws_path("a_renamed.R"))],
            PositionEncoding::Utf16,
        )
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

        let edit = will_rename_via_db(
            &snapshot,
            &[(ws_path("a.R"), ws_path("a2.R"))],
            PositionEncoding::Utf16,
        )
        .expect("rewritten");
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

        let edit = will_rename_via_db(&snapshot, &[(a_path, new_a)], PositionEncoding::Utf16)
            .expect("rewritten");
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
            PositionEncoding::Utf16,
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
            will_rename_via_db(
                &snapshot,
                &[(ws_path("a.R"), ws_path("a2.R"))],
                PositionEncoding::Utf16
            )
            .is_none(),
            "a dynamic source() is not rewritten"
        );
    }

    #[test]
    fn will_rename_ignores_a_noop_rename() {
        let snapshot = rename_workspace("foo <- function() 1\n", "source(\"a.R\")\n");
        assert!(
            will_rename_via_db(
                &snapshot,
                &[(ws_path("a.R"), ws_path("a.R"))],
                PositionEncoding::Utf16
            )
            .is_none(),
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

    #[test]
    fn apply_file_renames_ignores_a_folder_holding_nothing_tracked() {
        // `expand_dir_renames` keeps a pair that claimed nothing verbatim, so the
        // directory pair itself reaches the loop. A directory was never upserted
        // and `scope_members` yields only files, so nothing moves either way.
        let (dir, mut db, _a) = seeded_package();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).expect("data/");
        let data2 = dir.path().join("data2");
        let before = db.workspace().unwrap().members(&db).to_vec();

        std::fs::rename(&data, &data2).expect("move data -> data2");
        assert!(
            !apply_file_renames(&mut db, &[(data, data2)]),
            "a folder holding nothing tracked is not a membership change"
        );
        assert!(db.workspace().unwrap().members(&db).to_vec() == before);
    }

    // --- folder renames -------------------------------------------------

    #[test]
    fn will_rename_folder_leaves_a_colocated_literal_untouched() {
        // R/a.R sources its sibling R/b.R. Renaming the whole folder moves both,
        // so the relative spelling still resolves — the correct edit is none.
        let snapshot = rename_workspace_files(&[
            ("R/a.R", "source(\"b.R\")\n"),
            ("R/b.R", "foo <- function() 1\n"),
        ]);
        assert!(
            will_rename_via_db(
                &snapshot,
                &[(ws_path("R"), ws_path("src"))],
                PositionEncoding::Utf16
            )
            .is_none(),
            "a literal that still resolves from the new folder is not rewritten"
        );
    }

    #[test]
    fn will_rename_folder_rewrites_a_literal_escaping_the_folder() {
        // R/a.R reaches outside the renamed folder. Moving R/ deeper changes how
        // far it must climb, even though the target itself never moved.
        let snapshot = rename_workspace_files(&[
            ("R/a.R", "source(\"../data/x.R\")\n"),
            ("data/x.R", "foo <- function() 1\n"),
        ]);
        let uri_a = uri::from_path(&ws_path("R/a.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[(ws_path("R"), ws_path("nested/src"))],
            PositionEncoding::Utf16,
        )
        .expect("the moved sourcer's own literal is rebased");
        assert_eq!(sole_edit(&edit, &uri_a).1, "\"../../data/x.R\"");
    }

    #[test]
    fn will_rename_folder_rewrites_an_outside_sourcer() {
        // main.R stays put while its target moves with the folder.
        let snapshot = rename_workspace_files(&[
            ("main.R", "source(\"R/a.R\")\n"),
            ("R/a.R", "foo <- function() 1\n"),
        ]);
        let uri_main = uri::from_path(&ws_path("main.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[(ws_path("R"), ws_path("src"))],
            PositionEncoding::Utf16,
        )
        .expect("the outside dependent is rewritten");
        assert_eq!(sole_edit(&edit, &uri_main).1, "\"src/a.R\"");
    }

    #[test]
    fn will_rename_folder_batched_with_a_file_rename() {
        // Both ends move in one request: the folder holding the target, and the
        // sourcer itself.
        let snapshot = rename_workspace_files(&[
            ("main.R", "source(\"R/a.R\")\n"),
            ("R/a.R", "foo <- function() 1\n"),
        ]);
        let uri_main = uri::from_path(&ws_path("main.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[
                (ws_path("R"), ws_path("src")),
                (ws_path("main.R"), ws_path("sub/main2.R")),
            ],
            PositionEncoding::Utf16,
        )
        .expect("both moves are accounted for");
        assert_eq!(sole_edit(&edit, &uri_main).1, "\"../src/a.R\"");
    }

    #[test]
    fn will_rename_folder_prefers_the_deepest_rename() {
        // `R/sub/a.R` matches both `R` and `R/sub`; the more specific pair wins.
        let snapshot = rename_workspace_files(&[
            ("main.R", "source(\"R/sub/a.R\")\n"),
            ("R/sub/a.R", "foo <- function() 1\n"),
        ]);
        let uri_main = uri::from_path(&ws_path("main.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[
                (ws_path("R"), ws_path("src")),
                (ws_path("R/sub"), ws_path("other")),
            ],
            PositionEncoding::Utf16,
        )
        .expect("the deepest matching prefix decides");
        assert_eq!(sole_edit(&edit, &uri_main).1, "\"other/a.R\"");
    }

    #[test]
    fn will_rename_folder_rewrites_a_non_member_target() {
        // `scripts/x.R` is sourced but never seeded as a member, so it exists only
        // as a key in the reverse graph. It still has to remap.
        let snapshot = rename_workspace_files(&[("R/a.R", "source(\"../scripts/x.R\")\n")]);
        let uri_a = uri::from_path(&ws_path("R/a.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[(ws_path("scripts"), ws_path("tools"))],
            PositionEncoding::Utf16,
        )
        .expect("a non-member source target is remapped");
        assert_eq!(sole_edit(&edit, &uri_a).1, "\"../tools/x.R\"");
    }

    #[test]
    fn will_rename_folder_leaves_a_noncanonical_spelling_alone() {
        // `./b.R` still resolves after the folder moves. Comparing recomputed
        // spellings as strings would "fix" it to `b.R` — a cosmetic edit in a file
        // the rename did not affect. The check is resolution, not spelling.
        let snapshot = rename_workspace_files(&[
            ("R/a.R", "source(\"./b.R\")\n"),
            ("R/b.R", "foo <- function() 1\n"),
        ]);
        assert!(
            will_rename_via_db(
                &snapshot,
                &[(ws_path("R"), ws_path("src"))],
                PositionEncoding::Utf16
            )
            .is_none(),
            "a still-resolving literal is left exactly as written"
        );
    }

    #[test]
    fn will_rename_folder_rewrites_an_absolute_literal() {
        // An absolute spelling never depends on the sourcer's location, but it
        // does have to follow its target into the renamed folder.
        let absolute = ws_path("R/a.R").display().to_string().replace('\\', "/");
        let snapshot = rename_workspace_files(&[
            ("main.R", &format!("source(\"{absolute}\")\n")),
            ("R/a.R", "foo <- function() 1\n"),
        ]);
        let uri_main = uri::from_path(&ws_path("main.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[(ws_path("R"), ws_path("src"))],
            PositionEncoding::Utf16,
        )
        .expect("the absolute literal follows its target");
        let expected = ws_path("src/a.R").display().to_string().replace('\\', "/");
        assert_eq!(sole_edit(&edit, &uri_main).1, format!("\"{expected}\""));
    }

    #[test]
    fn will_rename_folder_with_no_members_is_a_noop() {
        let snapshot = rename_workspace("foo <- function() 1\n", "source(\"a.R\")\n");
        assert!(
            will_rename_via_db(
                &snapshot,
                &[(ws_path("data"), ws_path("data2"))],
                PositionEncoding::Utf16
            )
            .is_none(),
            "a folder holding nothing we track produces no edits"
        );
    }

    #[test]
    fn will_rename_moved_file_rebases_its_own_literal() {
        // A single-file move across directories: a.R's own literal has to climb
        // one more level. (Latent bug the folder work fixes.)
        let snapshot = rename_workspace("source(\"b.R\")\n", "foo <- function() 1\n");
        let uri_a = uri::from_path(&ws_path("a.R")).unwrap();

        let edit = will_rename_via_db(
            &snapshot,
            &[(ws_path("a.R"), ws_path("sub/a.R"))],
            PositionEncoding::Utf16,
        )
        .expect("the moved file's own literal is rebased");
        assert_eq!(sole_edit(&edit, &uri_a).1, "\"../b.R\"");
    }

    #[test]
    fn apply_file_renames_expands_a_folder_rename() {
        // didRenameFiles for a folder: every member under it swaps to its new path.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let old_dir = root.join("R");
        let new_dir = root.join("src");
        std::fs::create_dir(&old_dir).expect("create R/");
        let src = "foo <- function() 1\n";
        std::fs::write(old_dir.join("a.R"), src).expect("write a.R");
        std::fs::write(old_dir.join("b.R"), src).expect("write b.R");

        let mut db = IncrementalDatabase::default();
        let a = db.upsert_file(&old_dir.join("a.R"), src.to_string());
        let b = db.upsert_file(&old_dir.join("b.R"), src.to_string());
        db.set_workspace_members(vec![a, b], vec![root.to_path_buf()]);

        std::fs::rename(&old_dir, &new_dir).expect("move R/ -> src/");
        assert!(apply_file_renames(&mut db, &[(old_dir, new_dir.clone())]));

        let members = db.workspace().unwrap().members(&db).to_vec();
        for name in ["a.R", "b.R"] {
            let file = db
                .lookup_file(&new_dir.join(name))
                .expect("new path is tracked");
            assert!(members.contains(&file), "{name} moved with the folder");
        }
        assert!(!members.contains(&a), "old a.R is dropped");
        assert!(!members.contains(&b), "old b.R is dropped");
    }
}
