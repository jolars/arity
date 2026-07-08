//! Static extraction of top-level `source()` dependencies from a file's CST.
//!
//! R scripts wire files together with `source("other.R")`, which evaluates the
//! target file's top-level expressions in the caller's environment (the global
//! environment by default, or the calling environment under `local = TRUE`). We
//! model only what is statically knowable: literal-string targets of top-level
//! `source()` calls. Non-literal arguments (`source(paste0(...))`,
//! `source(path)`) become [`SourceTarget::Dynamic`] so cross-file resolution can
//! stay conservative and avoid false `undefined-symbol` findings.

use std::path::{Path, PathBuf};

use rowan::NodeOrToken;
use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

type SyntaxToken = rowan::SyntaxToken<RLanguage>;
type SyntaxElement = NodeOrToken<SyntaxNode, SyntaxToken>;

/// The target file of a `source()` call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub enum SourceTarget {
    /// A statically-resolved path: the literal string argument, joined onto the
    /// sourcing file's directory when relative and a base directory is known.
    Path(PathBuf),
    /// A non-literal argument we cannot resolve without evaluating R.
    Dynamic,
}

/// A `source()` edge stripped of its byte range — the part the cross-file graph
/// depends on. Carries no positional data, so a body edit that merely shifts a
/// `source()` call's offset leaves it unchanged and the project graph memo holds
/// (the firewall this module feeds). It also satisfies `salsa::Update`, which
/// [`SourceEdge`] cannot because of its `TextRange` field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct SourceEdgeKey {
    pub target: SourceTarget,
    pub local: bool,
}

/// A top-level `source(...)` dependency edge extracted from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEdge {
    pub target: SourceTarget,
    /// `source(..., local = TRUE)` evaluates in the calling environment, so it
    /// does not contribute the target's top-level bindings to the global scope.
    pub local: bool,
    /// Range of the `source(...)` call, for diagnostics.
    pub range: TextRange,
}

impl SourceEdge {
    /// True when this edge folds the target's top-level bindings into the
    /// sourcing file's global scope: a non-`local`, statically-resolved source.
    pub fn contributes_scope(&self) -> bool {
        !self.local && matches!(self.target, SourceTarget::Path(_))
    }

    /// Project this edge onto its range-free [`SourceEdgeKey`].
    pub fn key(&self) -> SourceEdgeKey {
        SourceEdgeKey {
            target: self.target.clone(),
            local: self.local,
        }
    }
}

/// One event in a file's top-level execution sequence, range-free.
///
/// Order is carried by position in the enclosing `Vec`, never by a span — so a
/// body edit that shifts offsets re-extracts to an *equal* sequence and the
/// firewall this feeds backdates, exactly like [`SourceEdgeKey`]. It is to a
/// span what [`crate::project::DefKind`] is to a value: the order-bearing,
/// range-free projection cross-file load-order resolution consumes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub enum TopLevelEvent {
    /// A top-level binding `name <- ...` becomes live at this point.
    Define(String),
    /// A top-level `source(...)`/`sys.source(...)` edge folds the target's
    /// bindings into scope here.
    SourceEdge(SourceEdgeKey),
    /// A top-level *bare free read* of `name` observes the scope as of here.
    /// Only file-scope-direct reads — a read inside a function/block body runs at
    /// call time and sees the final post-execution scope, so it is not gated by
    /// position and is omitted.
    Read(String),
}

/// The [`SourceEdgeKey`] for a top-level `child` that is a `source(...)` or
/// `sys.source(...)` call, else `None`. `sys.source` is mapped to
/// [`SourceTarget::Dynamic`] (we don't model its argument resolution), so it
/// poisons order resolution conservatively rather than being silently ignored.
pub fn top_level_source_edge_key(
    child: &SyntaxNode,
    base_dir: Option<&Path>,
) -> Option<SourceEdgeKey> {
    let call = CallExpr::cast(child.clone())?;
    let callee = call.callee_token()?;
    if callee.kind() != SyntaxKind::IDENT {
        return None;
    }
    match callee.text() {
        "source" => Some(source_edge(&call, base_dir).key()),
        "sys.source" => Some(SourceEdgeKey {
            target: SourceTarget::Dynamic,
            local: false,
        }),
        _ => None,
    }
}

/// Collect top-level `source(...)` calls in `root`. `base_dir` is the directory
/// of the file being scanned; relative literal targets are resolved against it.
///
/// Only direct children of the root are scanned: a `source()` nested inside a
/// function or block runs at call time into a non-global environment, so it is
/// not a static top-level dependency (the same posture the semantic builder
/// takes for `library()`).
pub fn collect_source_edges(root: &SyntaxNode, base_dir: Option<&Path>) -> Vec<SourceEdge> {
    root.children()
        .filter_map(|child| source_call(&child))
        .map(|call| source_edge(&call, base_dir))
        .collect()
}

/// Like [`collect_source_edges`] but projected onto range-free
/// [`SourceEdgeKey`]s — the form the cross-file graph query consumes.
pub fn collect_source_edge_keys(root: &SyntaxNode, base_dir: Option<&Path>) -> Vec<SourceEdgeKey> {
    root.children()
        .filter_map(|child| source_call(&child))
        .map(|call| source_edge(&call, base_dir).key())
        .collect()
}

/// A statically-resolved literal `source()` argument, carrying the byte range
/// and quoting needed to rewrite it in place (file rename). Unlike
/// [`SourceEdge`], whose `range` spans the whole call, [`Self::literal_range`]
/// spans only the string token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLiteralEdge {
    /// Resolved target: `base_dir.join(spelling)` when relative and a base dir
    /// is known, else the spelling verbatim. Un-normalized, like [`SourceEdge`].
    pub target: PathBuf,
    /// Range of the string token (including its quotes).
    pub literal_range: TextRange,
    /// The opening quote byte (`"`, `'`, or `` ` ``), preserved on rewrite.
    pub quote: u8,
    /// The inner text as written, without quotes.
    pub spelling: String,
    /// Whether the original spelling was a relative path.
    pub was_relative: bool,
}

/// Collect top-level `source("literal")` edges with the string token's range and
/// quoting — the form a file rename rewrites. Mirrors [`collect_source_edges`]
/// but skips [`SourceTarget::Dynamic`] arguments (a computed path can't be
/// rewritten) and named/positional `file` resolution is shared via
/// [`source_file_value`].
pub fn collect_source_literal_edges(
    root: &SyntaxNode,
    base_dir: Option<&Path>,
) -> Vec<SourceLiteralEdge> {
    root.children()
        .filter_map(|child| source_call(&child))
        .filter_map(|call| source_literal_edge(&call, base_dir))
        .collect()
}

/// A string literal, carrying the byte range of the string token (quotes
/// included) and the unquoted spelling. Collected from *every* string literal in
/// the file (not tied to any call), so read-only LSP walks can classify string
/// constants: the document-link walk turns file-naming spellings into clickable
/// links, and the document-color walk recognizes color spellings. Classification
/// and any filesystem access are the caller's job, keeping this extractor pure.
///
/// Only genuine string tokens (`"`/`'`/raw `r"..."`) are collected;
/// backtick-quoted non-syntactic names lex as `IDENT`, not `STRING`, so they
/// never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringLiteral {
    /// Range of the string token (including its quotes).
    pub literal_range: TextRange,
    /// The inner text as written, without quotes (raw-string aware). Never empty.
    pub spelling: String,
}

/// Collect every non-empty string literal in `root` as a [`StringLiteral`]. Pure:
/// no filesystem access and no classification — the caller inspects each
/// spelling. Empty spellings are skipped since they carry no useful content.
pub fn collect_string_literals(root: &SyntaxNode) -> Vec<StringLiteral> {
    root.descendants_with_tokens()
        .filter_map(|element| match element {
            NodeOrToken::Token(token) if token.kind() == SyntaxKind::STRING => Some(token),
            _ => None,
        })
        .filter_map(|token| {
            let spelling = string_literal_content(token.text())?;
            if spelling.is_empty() {
                return None;
            }
            Some(StringLiteral {
                literal_range: token.text_range(),
                spelling: spelling.to_string(),
            })
        })
        .collect()
}

/// A string literal that may name a file, carrying the byte range of the string
/// token (quotes included) and the unquoted spelling. Unlike
/// [`SourceLiteralEdge`], this is not tied to `source()`: it is collected from
/// *every* string literal in the file, so the LSP document-link walk can turn
/// any file-naming constant into a clickable link. Filesystem resolution and
/// existence checks are the caller's job, keeping this extractor pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkLiteral {
    /// Range of the string token (including its quotes).
    pub literal_range: TextRange,
    /// The inner text as written, without quotes (raw-string aware).
    pub spelling: String,
    /// Whether the spelling is a relative path (see [`is_relative_spelling`]).
    pub was_relative: bool,
}

/// Collect every string literal in `root` as a [`LinkLiteral`]. Pure: no
/// filesystem access and no `base_dir` resolution — the caller resolves each
/// spelling and filters by existence (see `compute_document_links`). Empty
/// spellings are skipped since they can never name a file. Layered over
/// [`collect_string_literals`], adding the path-relativity classification.
pub fn collect_link_literals(root: &SyntaxNode) -> Vec<LinkLiteral> {
    collect_string_literals(root)
        .into_iter()
        .map(|literal| LinkLiteral {
            was_relative: is_relative_spelling(&literal.spelling),
            literal_range: literal.literal_range,
            spelling: literal.spelling,
        })
        .collect()
}

/// The inner text of a string literal, stripped of its quotes. Handles the three
/// plain quote bytes (`"`, `'`, `` ` ``) like [`strip_quotes`], and additionally
/// R raw strings of the form `r"delim(content)delim"` (also `R`, `'` quote, and
/// `[`/`{` bracket variants). Escape sequences inside plain strings are *not*
/// decoded — the inner text is returned verbatim, matching [`strip_quotes`].
fn string_literal_content(text: &str) -> Option<&str> {
    if let Some(raw) = raw_string_content(text) {
        return Some(raw);
    }
    strip_quotes(text)
}

/// The content of an R raw string (`r"delim(content)delim"`), or `None` if `text`
/// is not a raw string. Accepts the `r`/`R` prefix, a `"` or `'` quote, an
/// optional run of `-` delimiter characters, and a `(`/`[`/`{` open bracket with
/// its matching close. The lexer currently only emits the paren+double-quote
/// form, but accepting the variants keeps this robust to future lexer growth.
fn raw_string_content(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let prefix = bytes.first()?;
    if *prefix != b'r' && *prefix != b'R' {
        return None;
    }
    let quote = *bytes.get(1)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    // Delimiter dashes, then the opening bracket.
    let mut i = 2;
    while bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    let (open, close) = match bytes.get(i)? {
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let dashes = i - 2;
    let content_start = i + 1;
    // The closer mirrors the opener: `)`, the same dash run, then the quote.
    if bytes.last() != Some(&quote) {
        return None;
    }
    let close_seq_len = 1 + dashes + 1; // close bracket + dashes + quote
    if bytes.len() < content_start + close_seq_len {
        return None;
    }
    let content_end = bytes.len() - close_seq_len;
    if bytes[content_end] != close {
        return None;
    }
    let _ = open;
    Some(&text[content_start..content_end])
}

/// Whether a `source()` path spelling should be resolved against the base
/// directory. Decided by the spelling alone, independent of the host OS, so the
/// classification is identical on Unix and Windows: a leading `/` or `\`, a
/// Windows drive prefix (`C:`), or a UNC prefix (`\\`) is rooted; everything
/// else is relative. (Rust's `Path::is_relative` is platform-dependent --- it
/// treats `/abs/util.R` as relative on Windows because it lacks a drive.)
fn is_relative_spelling(spelling: &str) -> bool {
    let bytes = spelling.as_bytes();
    match bytes {
        [b'/' | b'\\', ..] => false,
        // Drive-letter prefix, e.g. `C:` or `C:\`.
        [drive, b':', ..] if drive.is_ascii_alphabetic() => false,
        _ => true,
    }
}

fn source_literal_edge(call: &CallExpr, base_dir: Option<&Path>) -> Option<SourceLiteralEdge> {
    let (file_value, _local) = source_file_value(call);
    let NodeOrToken::Token(token) = file_value? else {
        return None;
    };
    if token.kind() != SyntaxKind::STRING {
        return None;
    }
    let spelling = strip_quotes(token.text())?.to_string();
    let quote = token.text().as_bytes()[0];
    let path = PathBuf::from(&spelling);
    let was_relative = is_relative_spelling(&spelling);
    let target = match base_dir {
        Some(dir) if was_relative => dir.join(&path),
        _ => path,
    };
    Some(SourceLiteralEdge {
        target,
        literal_range: token.text_range(),
        quote,
        spelling,
        was_relative,
    })
}

/// A relative path from `base_dir` to `target`, both assumed normalized
/// (absolute, no `.`/`..` components). Drops the shared component prefix, emits
/// one `..` per leftover `base_dir` component, then the leftover `target`
/// components. Returns `None` when the two share no root (a leftover root or
/// prefix on either side — e.g. distinct Windows drives), so the caller can fall
/// back to an absolute spelling. Pure and platform-component based; the caller
/// renders the result with forward slashes.
pub fn relative_path(base_dir: &Path, target: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut base = base_dir.components().peekable();
    let mut targ = target.components().peekable();
    while let (Some(b), Some(t)) = (base.peek(), targ.peek()) {
        if b == t {
            base.next();
            targ.next();
        } else {
            break;
        }
    }
    let mut result = PathBuf::new();
    for comp in base {
        match comp {
            Component::Normal(_) => result.push(".."),
            Component::CurDir => {}
            // A leftover root or prefix means the paths don't share a root, so no
            // relative form is sensible. (`ParentDir` shouldn't appear in a
            // normalized path, but is unrepresentable as a relative base step.)
            Component::RootDir | Component::Prefix(_) | Component::ParentDir => return None,
        }
    }
    for comp in targ {
        result.push(comp.as_os_str());
    }
    Some(result)
}

/// The `CallExpr` if `node` is a call to the bare function `source`.
fn source_call(node: &SyntaxNode) -> Option<CallExpr> {
    let call = CallExpr::cast(node.clone())?;
    let callee = call.callee_token()?;
    (callee.kind() == SyntaxKind::IDENT && callee.text() == "source").then_some(call)
}

/// Walk a `source(...)` call's arguments, returning the element that supplies
/// the `file` (first positional or named `file=`) and whether `local = TRUE` is
/// set. Shared by [`source_edge`] and [`collect_source_literal_edges`].
fn source_file_value(call: &CallExpr) -> (Option<SyntaxElement>, bool) {
    let mut file_value: Option<SyntaxElement> = None;
    let mut local = false;
    let mut seen_positional = false;

    if let Some(arg_list) = call.arg_list() {
        for arg in arg_list.args() {
            let (name, value) = arg_parts(arg.syntax());
            match name.as_deref() {
                // R's first formal is `file`; honor it whether named or positional.
                Some("file") => file_value = file_value.or(value),
                Some("local") => local = value.as_ref().is_some_and(is_true_literal),
                Some(_) => {}
                None => {
                    if !seen_positional {
                        file_value = file_value.or(value);
                        seen_positional = true;
                    }
                }
            }
        }
    }
    (file_value, local)
}

fn source_edge(call: &CallExpr, base_dir: Option<&Path>) -> SourceEdge {
    let (file_value, local) = source_file_value(call);
    let target = match file_value {
        Some(value) => target_from_value(&value, base_dir),
        None => SourceTarget::Dynamic,
    };
    SourceEdge {
        target,
        local,
        range: call.syntax().text_range(),
    }
}

/// Split an `ARG` node into its optional name (text, unquoted for strings) and
/// its value element (the first non-trivia element after `=`, or the whole arg
/// when positional).
fn arg_parts(arg: &SyntaxNode) -> (Option<String>, Option<SyntaxElement>) {
    let elements: Vec<SyntaxElement> = arg.children_with_tokens().collect();
    match elements
        .iter()
        .position(|e| e.kind() == SyntaxKind::ASSIGN_EQ)
    {
        Some(eq) => {
            let name = elements[..eq].iter().rev().find_map(token_name);
            let value = elements[eq + 1..]
                .iter()
                .find(|e| !is_trivia(e.kind()))
                .cloned();
            (name, value)
        }
        None => {
            let value = elements.iter().find(|e| !is_trivia(e.kind())).cloned();
            (None, value)
        }
    }
}

fn target_from_value(value: &SyntaxElement, base_dir: Option<&Path>) -> SourceTarget {
    if let NodeOrToken::Token(token) = value
        && token.kind() == SyntaxKind::STRING
        && let Some(literal) = strip_quotes(token.text())
    {
        let path = PathBuf::from(literal);
        let resolved = match base_dir {
            Some(dir) if path.is_relative() => dir.join(path),
            _ => path,
        };
        return SourceTarget::Path(resolved);
    }
    SourceTarget::Dynamic
}

/// `TRUE` / `T` as a bare token.
fn is_true_literal(value: &SyntaxElement) -> bool {
    matches!(value, NodeOrToken::Token(t)
        if t.kind() == SyntaxKind::IDENT && matches!(t.text(), "TRUE" | "T"))
}

fn token_name(element: &SyntaxElement) -> Option<String> {
    let NodeOrToken::Token(token) = element else {
        return None;
    };
    match token.kind() {
        SyntaxKind::IDENT => Some(token.text().to_string()),
        SyntaxKind::STRING => strip_quotes(token.text()).map(str::to_string),
        _ => None,
    }
}

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
    )
}

fn strip_quotes(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' || first == b'\'' || first == b'`') && first == last {
            return Some(&text[1..text.len() - 1]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn edges(src: &str, base_dir: Option<&Path>) -> Vec<SourceEdge> {
        collect_source_edges(&parse(src).cst, base_dir)
    }

    #[test]
    fn resolves_relative_literal_against_base_dir() {
        let base = PathBuf::from("/proj/R");
        let e = edges("source(\"helpers.R\")\n", Some(&base));
        assert_eq!(e.len(), 1);
        assert_eq!(
            e[0].target,
            SourceTarget::Path(PathBuf::from("/proj/R/helpers.R"))
        );
        assert!(e[0].contributes_scope());
    }

    #[test]
    fn keeps_absolute_literal_as_is() {
        let base = PathBuf::from("/proj");
        let e = edges("source(\"/abs/util.R\")\n", Some(&base));
        assert_eq!(
            e[0].target,
            SourceTarget::Path(PathBuf::from("/abs/util.R"))
        );
    }

    #[test]
    fn relative_literal_without_base_dir_stays_relative() {
        let e = edges("source(\"helpers.R\")\n", None);
        assert_eq!(e[0].target, SourceTarget::Path(PathBuf::from("helpers.R")));
    }

    #[test]
    fn named_file_argument_is_recognized() {
        let e = edges("source(file = \"setup.R\")\n", None);
        assert_eq!(e[0].target, SourceTarget::Path(PathBuf::from("setup.R")));
    }

    #[test]
    fn local_true_does_not_contribute_scope() {
        let e = edges("source(\"helpers.R\", local = TRUE)\n", None);
        assert!(e[0].local);
        assert!(!e[0].contributes_scope());
    }

    #[test]
    fn dynamic_argument_is_unresolved() {
        let e = edges("source(paste0(dir, \"x.R\"))\n", None);
        assert_eq!(e[0].target, SourceTarget::Dynamic);
        assert!(!e[0].contributes_scope());

        let v = edges("source(path)\n", None);
        assert_eq!(v[0].target, SourceTarget::Dynamic);
    }

    #[test]
    fn source_inside_function_is_not_top_level() {
        let e = edges("f <- function() source(\"x.R\")\n", None);
        assert!(e.is_empty());
    }

    #[test]
    fn non_source_calls_are_ignored() {
        let e = edges("library(dplyr)\nprint(\"x.R\")\n", None);
        assert!(e.is_empty());
    }

    fn literal_edges(src: &str, base_dir: Option<&Path>) -> Vec<SourceLiteralEdge> {
        collect_source_literal_edges(&parse(src).cst, base_dir)
    }

    #[test]
    fn literal_edge_captures_range_and_quoting() {
        let src = "source(\"helpers.R\")\n";
        let base = PathBuf::from("/proj/R");
        let e = literal_edges(src, Some(&base));
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].spelling, "helpers.R");
        assert_eq!(e[0].quote, b'"');
        assert!(e[0].was_relative);
        assert_eq!(
            e[0].target,
            PathBuf::from("/proj/R/helpers.R"),
            "relative literal resolves against base dir"
        );
        // The range slices exactly the quoted token, quotes included.
        let range = e[0].literal_range;
        assert_eq!(
            &src[range.start().into()..range.end().into()],
            "\"helpers.R\""
        );
    }

    #[test]
    fn literal_edge_preserves_single_quotes() {
        let e = literal_edges("source('a.R')\n", None);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].quote, b'\'');
        assert_eq!(e[0].spelling, "a.R");
    }

    #[test]
    fn literal_edge_recognizes_named_file_argument() {
        let e = literal_edges("source(file = \"setup.R\")\n", None);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].spelling, "setup.R");
    }

    #[test]
    fn literal_edge_marks_absolute_spelling() {
        let base = PathBuf::from("/proj");
        let e = literal_edges("source(\"/abs/util.R\")\n", Some(&base));
        assert_eq!(e.len(), 1);
        assert!(!e[0].was_relative);
        assert_eq!(e[0].target, PathBuf::from("/abs/util.R"));
    }

    #[test]
    fn relativity_classification_is_host_independent() {
        // Decided by the spelling alone, identical on Unix and Windows.
        assert!(is_relative_spelling("helpers.R"));
        assert!(is_relative_spelling("sub/helpers.R"));
        assert!(!is_relative_spelling("/abs/util.R"));
        assert!(!is_relative_spelling("\\abs\\util.R"));
        assert!(!is_relative_spelling("C:\\abs\\util.R"));
        assert!(!is_relative_spelling("C:/abs/util.R"));
    }

    #[test]
    fn literal_edge_skips_dynamic_arguments() {
        assert!(literal_edges("source(paste0(dir, \"x.R\"))\n", None).is_empty());
        assert!(literal_edges("source(path)\n", None).is_empty());
    }

    fn link_literals(src: &str) -> Vec<LinkLiteral> {
        collect_link_literals(&parse(src).cst)
    }

    #[test]
    fn link_literal_captures_range_and_spelling() {
        let src = "x <- \"helpers.R\"\n";
        let e = link_literals(src);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].spelling, "helpers.R");
        assert!(e[0].was_relative);
        let range = e[0].literal_range;
        assert_eq!(
            &src[range.start().into()..range.end().into()],
            "\"helpers.R\"",
            "range slices the quoted token, quotes included"
        );
    }

    #[test]
    fn link_literal_collects_from_any_position() {
        // Not just source(): assignment RHS, arbitrary call args, bare literal.
        let e = link_literals("readLines('a.R')\nb <- \"c.R\"\n\"d.R\"\n");
        let spellings: Vec<_> = e.iter().map(|l| l.spelling.as_str()).collect();
        assert_eq!(spellings, ["a.R", "c.R", "d.R"]);
    }

    #[test]
    fn link_literal_is_raw_string_aware() {
        let e = link_literals("x <- r\"(sub/helpers.R)\"\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].spelling, "sub/helpers.R");
        assert!(e[0].was_relative);
    }

    #[test]
    fn link_literal_raw_string_with_dashes() {
        let e = link_literals("x <- r\"-(a.R)-\"\n");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].spelling, "a.R");
    }

    #[test]
    fn link_literal_marks_absolute_spelling() {
        let e = link_literals("x <- \"/abs/util.R\"\n");
        assert_eq!(e.len(), 1);
        assert!(!e[0].was_relative);
    }

    #[test]
    fn link_literal_skips_empty_strings() {
        assert!(link_literals("x <- \"\"\n").is_empty());
    }

    fn string_literals(src: &str) -> Vec<StringLiteral> {
        collect_string_literals(&parse(src).cst)
    }

    #[test]
    fn string_literal_captures_range_and_spelling() {
        let src = "x <- \"helpers.R\"\n";
        let e = string_literals(src);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].spelling, "helpers.R");
        let range = e[0].literal_range;
        assert_eq!(
            &src[range.start().into()..range.end().into()],
            "\"helpers.R\"",
            "range slices the quoted token, quotes included"
        );
    }

    #[test]
    fn string_literal_is_raw_string_aware_and_skips_empty() {
        assert_eq!(string_literals("x <- r\"(a.R)\"\n")[0].spelling, "a.R");
        assert!(string_literals("x <- \"\"\n").is_empty());
    }

    #[test]
    fn string_literal_excludes_backtick_names() {
        // Backtick-quoted non-syntactic names lex as IDENT, never STRING.
        assert!(string_literals("`a` <- 1\n").is_empty());
    }

    #[test]
    fn string_literal_content_handles_quote_forms() {
        assert_eq!(string_literal_content("\"a.R\""), Some("a.R"));
        assert_eq!(string_literal_content("'a.R'"), Some("a.R"));
        assert_eq!(string_literal_content("`a.R`"), Some("a.R"));
        assert_eq!(string_literal_content("r\"(a.R)\""), Some("a.R"));
        assert_eq!(string_literal_content("r\"--(a.R)--\""), Some("a.R"));
        assert_eq!(string_literal_content("R\"[a.R]\""), Some("a.R"));
    }

    #[test]
    fn relative_path_same_directory() {
        let r = relative_path(Path::new("/proj/R"), Path::new("/proj/R/a.R")).unwrap();
        assert_eq!(r, PathBuf::from("a.R"));
    }

    #[test]
    fn relative_path_child_directory() {
        let r = relative_path(Path::new("/proj/R"), Path::new("/proj/R/sub/a.R")).unwrap();
        assert_eq!(r, PathBuf::from("sub/a.R"));
    }

    #[test]
    fn relative_path_parent_directory() {
        let r = relative_path(Path::new("/proj/R/sub"), Path::new("/proj/R/a.R")).unwrap();
        assert_eq!(r, PathBuf::from("../a.R"));
    }

    #[test]
    fn relative_path_disjoint_subtree() {
        let r = relative_path(Path::new("/proj/a/b"), Path::new("/proj/c/d.R")).unwrap();
        assert_eq!(r, PathBuf::from("../../c/d.R"));
    }
}
