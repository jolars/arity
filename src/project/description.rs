//! Static discovery of roxygen2's package-wide options from `DESCRIPTION`.
//!
//! roxygen2 (7.3.3, `R/options.R`) resolves its options by evaluating the
//! `Roxygen` field of `DESCRIPTION` as an R expression (conventionally
//! `list(markdown = TRUE)`), overlaying the value of `man/roxygen/meta.R`
//! (sourced; its last expression's value) via `modifyList(desc, meta)`, and
//! falling back to defaults — `markdown = FALSE`.
//!
//! Arity's semantics stay **static** (no R evaluation), so this module
//! approximates that resolution by parsing the same texts with arity's own
//! parser and reading the `markdown` argument only when it is a literal
//! `TRUE`/`FALSE` in a plain `list(...)` call. Anything dynamic (a variable, a
//! computed list, an unparseable field) resolves to "unknown", which falls
//! through to the next layer exactly like an absent key would: an unknown
//! `meta.R` defers to the `DESCRIPTION` field, and an unknown field defers to
//! roxygen2's off default. The approximation therefore only misses packages
//! that compute their markdown flag at roxygenize time — it never *invents* a
//! markdown default.

use std::path::Path;

use crate::ast::{Arg, AstNode, CallExpr, HasArgList};
use crate::parser::parse;
use crate::project::scope::package_root;
use crate::rindex::harvest::parse_dcf;
use crate::syntax::SyntaxKind;

/// The package-wide roxygen markdown default for the package at `root`
/// (a directory holding `DESCRIPTION`): `man/roxygen/meta.R` when statically
/// resolvable, else the `Roxygen` field of `DESCRIPTION`, else `false`
/// (roxygen2's default). Touches disk.
pub fn roxygen_markdown_default(root: &Path) -> bool {
    let desc = std::fs::read_to_string(root.join("DESCRIPTION"))
        .ok()
        .and_then(|text| roxygen_field(&text))
        .and_then(|expr| markdown_from_r_text(&expr));
    let meta = std::fs::read_to_string(root.join("man/roxygen/meta.R"))
        .ok()
        .and_then(|text| markdown_from_r_text(&text));
    meta.or(desc).unwrap_or(false)
}

/// [`roxygen_markdown_default`] resolved for a single file: walk up to the
/// enclosing package root (`DESCRIPTION` + `R/`). A loose file outside any
/// package keeps roxygen2's off default. Touches disk.
pub fn roxygen_markdown_default_for_file(path: &Path) -> bool {
    package_root(path).is_some_and(|root| roxygen_markdown_default(&root))
}

/// [`roxygen_markdown_default_for_file`] anchored at a directory instead of a
/// file — for stdin input, where the only location is the working directory.
/// The walk starts at `dir` itself (a `package_root` walk starts at the
/// argument's parent). Touches disk.
pub fn roxygen_markdown_default_for_dir(dir: &Path) -> bool {
    roxygen_markdown_default_for_file(&dir.join("_stdin_.R"))
}

/// A per-directory-memoized [`roxygen_markdown_default_for_file`], for batch
/// walks (format/lint over a package) where every file in `R/` would otherwise
/// re-walk to the root and re-read `DESCRIPTION`. Two files sharing a parent
/// directory always share a package root, so the memo key is the parent.
#[derive(Debug, Default)]
pub struct MarkdownDefaultResolver {
    by_dir: std::collections::HashMap<std::path::PathBuf, bool>,
}

impl MarkdownDefaultResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// The markdown default for `file`, resolved once per parent directory.
    pub fn resolve(&mut self, file: &Path) -> bool {
        match file.parent() {
            Some(dir) => *self
                .by_dir
                .entry(dir.to_path_buf())
                .or_insert_with(|| roxygen_markdown_default_for_file(file)),
            None => false,
        }
    }
}

/// The name declared by the `Package` field of the DESCRIPTION at `root`
/// (a directory holding one). `None` when there is no readable DESCRIPTION or
/// it declares no `Package`. Touches disk.
pub fn package_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("DESCRIPTION")).ok()?;
    parse_dcf(&text)
        .into_iter()
        .find(|(key, _)| key == "Package")
        .map(|(_, value)| value)
        .filter(|name| !name.is_empty())
}

/// [`package_name`] resolved for a single file: walk up to the enclosing
/// package root (`DESCRIPTION` + `R/`) and read its `Package` field. `None` for
/// a loose file outside any package. Touches disk.
///
/// Note the walk anchors on [`package_root`], which requires an `R/` directory,
/// so a file under `tests/testthat/` of a package resolves to that package —
/// which is exactly the case that matters for `internal-function`.
pub fn package_name_for_file(path: &Path) -> Option<String> {
    package_root(path).and_then(|root| package_name(&root))
}

/// The `Roxygen` field's value from DESCRIPTION text (continuation lines
/// joined), if present.
fn roxygen_field(description: &str) -> Option<String> {
    parse_dcf(description)
        .into_iter()
        .find(|(key, _)| key == "Roxygen")
        .map(|(_, value)| value)
}

/// Statically resolve the `markdown` element of the R text's value: the text's
/// **last** top-level expression (both `eval(parse(text = field))` and
/// `source(meta.R)$value` yield the last expression's value) must be a plain
/// `list(...)` call carrying a literal `markdown = TRUE`/`FALSE`. `None` when
/// the value is absent or not statically resolvable.
fn markdown_from_r_text(text: &str) -> Option<bool> {
    let output = parse(text);
    if !output.diagnostics.is_empty() {
        return None;
    }
    // The last top-level expression must itself be the `list(...)` call. Walk
    // elements, not nodes: an atom statement (`x` after the list) is a bare
    // token in this CST and a node-level `last_child` would skip right past it.
    let last = output
        .cst
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE
                    | SyntaxKind::NEWLINE
                    | SyntaxKind::COMMENT
                    | SyntaxKind::SEMICOLON
            )
        })
        .last()?;
    let last = CallExpr::cast(last.into_node()?)?;
    if last.callee_name().as_deref() != Some("list") {
        return None;
    }
    last.args()
        .filter(|arg| arg.name().as_deref() == Some("markdown"))
        .last()
        .and_then(|arg| literal_logical(&arg))
}

/// The literal logical value of a named argument, when its value is exactly
/// the `TRUE` or `FALSE` constant (an identifier token — R's special constants
/// are bare tokens in the CST). `None` for anything else, including `T`/`F`
/// (reassignable in R) and computed values.
fn literal_logical(arg: &Arg) -> Option<bool> {
    let value = arg.value()?;
    let token = value.into_token()?;
    match token.text() {
        "TRUE" => Some(true),
        "FALSE" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(description: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("R")).expect("R/");
        std::fs::write(dir.path().join("R/a.R"), "NULL\n").expect("a.R");
        std::fs::write(dir.path().join("DESCRIPTION"), description).expect("DESCRIPTION");
        dir
    }

    fn write_meta(dir: &tempfile::TempDir, source: &str) {
        let meta_dir = dir.path().join("man/roxygen");
        std::fs::create_dir_all(&meta_dir).expect("man/roxygen/");
        std::fs::write(meta_dir.join("meta.R"), source).expect("meta.R");
    }

    #[test]
    fn package_name_from_description() {
        let dir = package("Package: mypkg\nVersion: 1.0\n");
        assert_eq!(package_name(dir.path()).as_deref(), Some("mypkg"));
        // Walks up from a file, including one nested well below the root.
        assert_eq!(
            package_name_for_file(&dir.path().join("R/a.R")).as_deref(),
            Some("mypkg")
        );
        assert_eq!(
            package_name_for_file(&dir.path().join("tests/testthat/test-a.R")).as_deref(),
            Some("mypkg")
        );
    }

    #[test]
    fn package_name_is_none_without_a_package() {
        // No DESCRIPTION at all.
        let loose = tempfile::tempdir().expect("tempdir");
        assert_eq!(package_name(loose.path()), None);
        assert_eq!(package_name_for_file(&loose.path().join("a.R")), None);
        // A DESCRIPTION that declares no `Package`.
        let dir = package("Version: 1.0\n");
        assert_eq!(package_name(dir.path()), None);
    }

    #[test]
    fn markdown_true_from_roxygen_field() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn markdown_defaults_off_without_field() {
        let dir = package("Package: p\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn markdown_false_is_explicit_off() {
        let dir = package("Package: p\nRoxygen: list(markdown = FALSE)\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn field_with_more_options_and_continuation() {
        let dir = package(
            "Package: p\nRoxygen: list(load = \"installed\",\n    markdown = TRUE)\nDepends: R\n",
        );
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn non_literal_markdown_value_is_unknown() {
        let dir = package("Package: p\nRoxygen: list(markdown = flag)\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn non_list_field_is_unknown() {
        let dir = package("Package: p\nRoxygen: make_options()\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_overrides_description_on() {
        let dir = package("Package: p\nRoxygen: list(markdown = FALSE)\n");
        write_meta(&dir, "list(markdown = TRUE)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_overrides_description_off() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        write_meta(&dir, "list(markdown = FALSE)\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_last_expression_wins_after_other_statements() {
        let dir = package("Package: p\n");
        write_meta(&dir, "x <- 1\nlist(markdown = TRUE)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_trailing_atom_is_not_the_list() {
        // `source()`'s value is `x`, not the list — statically unknown, so the
        // DESCRIPTION field wins.
        let dir = package("Package: p\nRoxygen: list(markdown = FALSE)\n");
        write_meta(&dir, "list(markdown = TRUE)\nx\n");
        assert!(!roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn unresolvable_meta_defers_to_description() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        write_meta(&dir, "build_meta()\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn meta_list_without_markdown_defers_to_description() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        write_meta(&dir, "list(knitr_chunk_options = NULL)\n");
        assert!(roxygen_markdown_default(dir.path()));
    }

    #[test]
    fn for_file_resolves_via_package_root() {
        let dir = package("Package: p\nRoxygen: list(markdown = TRUE)\n");
        assert!(roxygen_markdown_default_for_file(&dir.path().join("R/a.R")));
    }

    #[test]
    fn for_file_outside_a_package_is_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loose = dir.path().join("loose.R");
        std::fs::write(&loose, "NULL\n").expect("loose.R");
        assert!(!roxygen_markdown_default_for_file(&loose));
    }
}
