//! Shared test helpers for the LSP submodule tests.

use super::*;
use std::path::Path;

pub(crate) fn pos(line: u32, character: u32) -> Position {
    Position { line, character }
}

pub(crate) fn full_line_0() -> Range {
    Range {
        start: pos(0, 0),
        end: pos(0, 100),
    }
}

/// An absolute path valid on the host platform (the URI conversion needs an
/// absolute path, and `/tmp/t.R` is not absolute on Windows).
pub(crate) fn test_path() -> &'static Path {
    if cfg!(windows) {
        Path::new(r"C:\tmp\t.R")
    } else {
        Path::new("/tmp/t.R")
    }
}

pub(crate) fn test_uri() -> Uri {
    uri::from_path(test_path()).expect("valid file uri")
}

// --- scheduler: decide() ----------------------------------------------

pub(crate) fn uri_named(name: &str) -> Uri {
    let path = if cfg!(windows) {
        PathBuf::from(format!(r"C:\tmp\{name}"))
    } else {
        PathBuf::from(format!("/tmp/{name}"))
    };
    uri::from_path(&path).expect("valid file uri")
}

pub(crate) fn indexed_dplyr() -> IndexedProvider {
    use crate::rindex::schema::{PackageIndex, SCHEMA_VERSION, SymbolEntry, SymbolKind};
    let idx = PackageIndex {
        schema_version: SCHEMA_VERSION,
        package: "dplyr".into(),
        version: "1.0".into(),
        lib_path: "/lib".into(),
        r_version: None,
        harvested_at: 0,
        attaches: Vec::new(),
        symbols: vec![SymbolEntry {
            name: "across".into(),
            kind: SymbolKind::Function,
            exported: true,
            formals: None,
            help: None,
        }],
    };
    IndexedProvider::from_indices([idx])
}

// --- hover ------------------------------------------------------------

/// dplyr with one richly-documented export (`across`).
pub(crate) fn documented_dplyr() -> IndexedProvider {
    use crate::rindex::schema::{Formal, HelpArg, HelpDoc, PackageIndex, SCHEMA_VERSION};
    let idx = PackageIndex {
        schema_version: SCHEMA_VERSION,
        package: "dplyr".into(),
        version: "1.0".into(),
        lib_path: "/lib".into(),
        r_version: None,
        harvested_at: 0,
        attaches: Vec::new(),
        symbols: vec![SymbolEntry {
            name: "across".into(),
            kind: SymbolKind::Function,
            exported: true,
            formals: Some(vec![
                Formal {
                    name: ".cols".into(),
                    default: Some("everything()".into()),
                },
                Formal {
                    name: ".fns".into(),
                    default: None,
                },
            ]),
            help: Some(HelpDoc {
                title: Some("Apply a function across columns".into()),
                description: Some("Apply one or more functions to a set of columns.".into()),
                usage: Some("across(.cols, .fns)".into()),
                arguments: vec![HelpArg {
                    name: ".cols".into(),
                    description: "Columns to transform.".into(),
                }],
            }),
        }],
    };
    IndexedProvider::from_indices([idx])
}

/// Byte offset of the first occurrence of `needle` in `src`.
pub(crate) fn offset_of(src: &str, needle: &str) -> usize {
    src.find(needle).expect("needle present") + 1
}

pub(crate) fn hover_markdown(src: &str, needle: &str, indexed: &IndexedProvider) -> Option<String> {
    compute_hover(
        src,
        offset_of(src, needle),
        indexed,
        PositionEncoding::Utf16,
    )
    .map(|h| match h.contents {
        HoverContents::Markup(m) => m.value,
        other => panic!("expected markup, got {other:?}"),
    })
}

// --- cross-file rename (rename_via_db) --------------------------------

/// A workspace root valid on the host (absolute, so URI conversion works).
pub(crate) fn ws_root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\s")
    } else {
        PathBuf::from("/s")
    }
}

pub(crate) fn ws_path(name: &str) -> PathBuf {
    ws_root().join(name)
}

/// A two-file workspace (`a.R`, `b.R`) seeded as members, snapshotted for a
/// read job. Flat scripts with no `source()` edge between them, so under the
/// scope-aware model they are *disjoint* — a shared top-level name in both is
/// two unrelated bindings.
pub(crate) fn rename_workspace(a_src: &str, b_src: &str) -> Analysis {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(&ws_path("a.R"), a_src.to_string());
    let b = db.upsert_file(&ws_path("b.R"), b_src.to_string());
    db.set_workspace_members(vec![a, b], vec![ws_root()]);
    db.snapshot()
}

/// A three-file flat workspace (`a.R`, `b.R`, `c.R`) seeded as members. Like
/// [`rename_workspace`] but with a third file, for scenarios where one member
/// carries a `source()` edge (often dynamic) that is unrelated to the renamed
/// name.
pub(crate) fn rename_workspace3(a_src: &str, b_src: &str, c_src: &str) -> Analysis {
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(&ws_path("a.R"), a_src.to_string());
    let b = db.upsert_file(&ws_path("b.R"), b_src.to_string());
    let c = db.upsert_file(&ws_path("c.R"), c_src.to_string());
    db.set_workspace_members(vec![a, b, c], vec![ws_root()]);
    db.snapshot()
}

/// A flat workspace of arbitrarily many `(name, src)` members, seeded and
/// snapshotted. Like [`rename_workspace`]/[`rename_workspace3`] but for
/// scenarios needing four or more files (e.g. a sourced closure with two
/// definers of the same name).
pub(crate) fn rename_workspace_files(files: &[(&str, &str)]) -> Analysis {
    let mut db = IncrementalDatabase::default();
    let members: Vec<_> = files
        .iter()
        .map(|(name, src)| db.upsert_file(&ws_path(name), src.to_string()))
        .collect();
    db.set_workspace_members(members, vec![ws_root()]);
    db.snapshot()
}

/// A real on-disk R package (`DESCRIPTION` + `R/a.R`) seeded as one workspace
/// member, with `R/a.R`'s path returned as the anchor. Used by the membership
/// tests, which reason about disk state rather than about analysis.
///
/// The empty `arity.toml` is load-bearing: [`Config::resolve`] discovers config
/// by walking *upward* from the anchor, so without a bound here a stray
/// `arity.toml` in a real ancestor of the system temp dir would replace
/// [`DEFAULT_EXCLUDE`](crate::config::DEFAULT_EXCLUDE) and silently change what
/// counts as in scope. An empty file takes the serde default (so `renv/` and
/// friends still apply) and roots the patterns at the tempdir.
pub(crate) fn seeded_package() -> (tempfile::TempDir, IncrementalDatabase, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(root.join("arity.toml"), "").expect("arity.toml");
    std::fs::write(root.join("DESCRIPTION"), "Package: testpkg\n").expect("DESCRIPTION");
    let r_dir = root.join("R");
    std::fs::create_dir(&r_dir).expect("R/");
    let a = r_dir.join("a.R");
    std::fs::write(&a, "foo <- function() 1\n").expect("a.R");
    let mut db = IncrementalDatabase::default();
    let file = db.upsert_file(&a, "foo <- function() 1\n".to_string());
    db.set_workspace_members(vec![file], vec![root.to_path_buf()]);
    (dir, db, a)
}

/// A real on-disk R package (`DESCRIPTION` + `R/`) with two member files,
/// seeded and snapshotted. Package siblings share one flat namespace, which
/// the scope layer derives from the `package_root` disk walk — so this can't
/// be faked with flat in-memory paths. The returned [`tempfile::TempDir`]
/// must be kept alive for the duration of the test.
pub(crate) fn rename_package(
    a_src: &str,
    b_src: &str,
) -> (tempfile::TempDir, Analysis, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(root.join("DESCRIPTION"), "Package: testpkg\n").expect("write DESCRIPTION");
    let r_dir = root.join("R");
    std::fs::create_dir(&r_dir).expect("create R/");
    let a_path = r_dir.join("a.R");
    let b_path = r_dir.join("b.R");
    std::fs::write(&a_path, a_src).expect("write a.R");
    std::fs::write(&b_path, b_src).expect("write b.R");
    let mut db = IncrementalDatabase::default();
    let a = db.upsert_file(&a_path, a_src.to_string());
    let b = db.upsert_file(&b_path, b_src.to_string());
    db.set_workspace_members(vec![a, b], vec![root.to_path_buf()]);
    (dir, db.snapshot(), a_path, b_path)
}

/// A real on-disk package with a custom `DESCRIPTION` and an arbitrary set of
/// `R/<name>` files, each flagged whether to also *seed* it as a workspace
/// member. A file on disk but unseeded simulates an unanalyzed member; a
/// seeded file with a parse error is dropped by `workspace_project`. Both
/// make the package's analyzed set incomplete. Returns the snapshot and the
/// path of the first file (the rename anchor).
pub(crate) fn package_workspace(
    description: &str,
    files: &[(&str, &str, bool)],
) -> (tempfile::TempDir, Analysis, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(root.join("DESCRIPTION"), description).expect("write DESCRIPTION");
    let r_dir = root.join("R");
    std::fs::create_dir(&r_dir).expect("create R/");
    let mut db = IncrementalDatabase::default();
    let mut members = Vec::new();
    let mut first: Option<PathBuf> = None;
    for (name, src, seed) in files {
        let path = r_dir.join(name);
        std::fs::write(&path, src).expect("write R file");
        first.get_or_insert_with(|| path.clone());
        if *seed {
            members.push(db.upsert_file(&path, src.to_string()));
        }
    }
    db.set_workspace_members(members, vec![root.to_path_buf()]);
    (dir, db.snapshot(), first.expect("at least one file"))
}

/// Rename a.R's `foo` in a package where a.R + b.R both define `foo` (a
/// multi-def cohort). Returns whether the rename was offered.
pub(crate) fn package_multidef_rename_offered(
    description: &str,
    files: &[(&str, &str, bool)],
) -> bool {
    let (_dir, snapshot, a_path) = package_workspace(description, files);
    let uri_a = uri::from_path(&a_path).unwrap();
    let a_src = files[0].1;
    let offset = a_src.find("foo").unwrap();
    rename_via_db(
        &snapshot,
        &a_path,
        &uri_a,
        &buf(a_src),
        offset,
        "renamed",
        PositionEncoding::Utf16,
    )
    .is_some()
}

// --- scope-aware cross-file references (references_via_db) -------------

pub(crate) fn pos_at(text: &str, offset: usize) -> Position {
    LineIndex::new(text).byte_to_position(offset, PositionEncoding::Utf16)
}

/// A live buffer over `text`, for the `*_via_db` entry points.
pub(crate) fn buf(text: &str) -> TextBuffer {
    TextBuffer::from(text)
}

/// The set of distinct URIs touched by a reference result.
pub(crate) fn ref_uris(locations: &[Location]) -> std::collections::HashSet<&Uri> {
    locations.iter().map(|loc| &loc.uri).collect()
}

// --- file rename (will_rename_via_db / didRenameFiles) ----------------

/// The single edit a `WorkspaceEdit` makes to `uri`: `(range, new_text)`.
pub(crate) fn sole_edit(edit: &WorkspaceEdit, uri: &Uri) -> (Range, String) {
    let edits = edit
        .changes
        .as_ref()
        .and_then(|c| c.get(uri))
        .unwrap_or_else(|| panic!("expected an edit in {uri:?}"));
    assert_eq!(edits.len(), 1, "exactly one edit in {uri:?}");
    (edits[0].range, edits[0].new_text.clone())
}
