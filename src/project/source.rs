//! Static extraction of top-level `source()` dependencies from a file's CST.
//!
//! R scripts wire files together with `source("other.R")`, which evaluates the
//! target file's top-level expressions in the caller's environment (the global
//! environment by default, or the calling environment under `local = TRUE`). We
//! model only what is statically knowable: literal-string targets of top-level
//! `source()` calls. Non-literal arguments (`source(paste0(...))`,
//! `source(path)`) become [`SourceTarget::Dynamic`] so cross-file resolution can
//! stay conservative and avoid false `undefined-symbol` findings.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rowan::NodeOrToken;
use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::semantic::{BindingKind, SemanticModel};
use crate::syntax::{RLanguage, SyntaxKind, SyntaxNode};

type SyntaxToken = rowan::SyntaxToken<RLanguage>;
type SyntaxElement = NodeOrToken<SyntaxNode, SyntaxToken>;

/// The target file of a `source()` call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum SourceTarget {
    /// A statically-resolved path: the literal string argument, joined onto the
    /// assumed working directory when relative and a base directory is known.
    Path(PathBuf),
    /// A non-literal argument we cannot resolve without evaluating R.
    Dynamic,
}

/// A `source()` edge stripped of its byte range — the part the cross-file graph
/// depends on. Carries no positional data, so a body edit that merely shifts a
/// `source()` call's offset leaves it unchanged and the project graph memo holds
/// (the firewall this module feeds). It also satisfies `salsa::SalsaValue`, which
/// [`SourceEdge`] cannot because of its `TextRange` field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub struct SourceEdgeKey {
    pub target: SourceTarget,
    /// Whether the destination is separate from the file's evaluation frame.
    pub local: bool,
    /// Original relative spelling, retained to resolve an inherited directory.
    pub relative_path: Option<PathBuf>,
    /// Temporarily execute the target in its containing directory.
    pub chdir: bool,
}

/// A top-level `source(...)` dependency edge extracted from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEdge {
    pub target: SourceTarget,
    /// The destination is separate from the file's evaluation environment.
    /// `local = TRUE` at file scope shares that environment, like a plain block.
    pub local: bool,
    pub relative_path: Option<PathBuf>,
    pub chdir: bool,
    /// Range of the `source(...)` call, for diagnostics.
    pub range: TextRange,
}

impl SourceEdge {
    /// True when this edge folds the target's top-level bindings into the
    /// sourcing file's evaluation frame: a shared, statically resolved source.
    pub fn contributes_scope(&self) -> bool {
        !self.local && matches!(self.target, SourceTarget::Path(_))
    }

    /// Project this edge onto its range-free [`SourceEdgeKey`].
    pub fn key(&self) -> SourceEdgeKey {
        SourceEdgeKey {
            target: self.target.clone(),
            local: self.local,
            relative_path: self.relative_path.clone(),
            chdir: self.chdir,
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::SalsaValue)]
pub enum TopLevelEvent {
    /// A top-level binding `name <- ...` becomes live at this point.
    Define(String),
    /// A top-level `source(...)`/`sys.source(...)` edge folds the target's
    /// bindings into scope here.
    SourceEdge(SourceEdgeKey),
    /// A top-level *bare free read* of `name` observes the scope as of here.
    /// Only file-scope-direct reads — a read inside a function body runs at
    /// call time and sees the final post-execution scope, so it is not gated by
    /// position and is omitted.
    Read(String),
}

/// The [`SourceEdgeKey`] for a top-level `child` that is a `source(...)` or
/// `sys.source(...)` call, else `None`. Its destination is classified separately
/// from its literal path and working directory.
pub fn top_level_source_edge_key(
    child: &SyntaxNode,
    base_dir: Option<&Path>,
) -> Option<SourceEdgeKey> {
    source_call(child).map(|call| source_edge(&call, base_dir).key())
}

pub(crate) fn unconditional_statements(root: &SyntaxNode) -> Vec<SyntaxElement> {
    let mut statements = Vec::new();
    for element in root.children_with_tokens() {
        if let Some(block) = element
            .as_node()
            .filter(|node| node.kind() == SyntaxKind::BLOCK_EXPR)
        {
            statements.extend(unconditional_statements(block));
        } else if !is_trivia(element.kind())
            && !matches!(element.kind(), SyntaxKind::LBRACE | SyntaxKind::RBRACE)
        {
            statements.push(element);
        }
    }
    statements
}

/// Collect top-level `source(...)` calls in `root`. `base_dir` is the directory
/// assumed for execution; relative literal targets are resolved against it.
///
/// Plain blocks share their enclosing evaluation environment. Conditional
/// calls and function bodies are modeled by `source_env`, not folded into this
/// unconditional file-scope closure.
pub fn collect_source_edges(root: &SyntaxNode, base_dir: Option<&Path>) -> Vec<SourceEdge> {
    collect_source_edges_in(root, base_dir, &SemanticModel::build(root))
}

pub(crate) fn collect_source_edges_in(
    root: &SyntaxNode,
    base_dir: Option<&Path>,
    model: &SemanticModel,
) -> Vec<SourceEdge> {
    let calls: HashSet<_> = loading_calls(root, model, LoadingCallKind::Unshadowed)
        .map(|call| call.syntax().text_range())
        .collect();
    unconditional_statements(root)
        .into_iter()
        .filter_map(|child| child.as_node().and_then(source_call))
        .filter(|call| calls.contains(&call.syntax().text_range()))
        .map(|call| source_edge(&call, base_dir))
        .collect()
}

/// Like [`collect_source_edges`] but projected onto range-free
/// [`SourceEdgeKey`]s — the form the cross-file graph query consumes.
pub fn collect_source_edge_keys(root: &SyntaxNode, base_dir: Option<&Path>) -> Vec<SourceEdgeKey> {
    collect_source_edges(root, base_dir)
        .into_iter()
        .map(|edge| edge.key())
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

/// Collect literal loading calls at any depth with the string token's range and
/// quoting — the form a file rename rewrites. Skips
/// [`SourceTarget::Dynamic`] arguments (a computed path can't be
/// rewritten) and named/positional `file` resolution is shared via
/// [`source_file_value`].
pub fn collect_source_literal_edges(
    root: &SyntaxNode,
    base_dir: Option<&Path>,
) -> Vec<SourceLiteralEdge> {
    collect_source_literal_edges_in(root, base_dir, &SemanticModel::build(root))
}

pub(crate) fn collect_source_literal_edges_in(
    root: &SyntaxNode,
    base_dir: Option<&Path>,
    model: &SemanticModel,
) -> Vec<SourceLiteralEdge> {
    loading_calls(root, model, LoadingCallKind::Unshadowed)
        .filter_map(|call| source_literal_edge(&call, base_dir))
        .collect()
}

pub(crate) enum LoadingCallKind {
    Possible,
    Unshadowed,
}

pub(crate) fn loading_calls(
    root: &SyntaxNode,
    model: &SemanticModel,
    kind: LoadingCallKind,
) -> impl Iterator<Item = CallExpr> {
    let shadowed: HashSet<_> = model
        .idents()
        .iter()
        .enumerate()
        .filter(|(_, ident)| {
            matches!(
                crate::semantic::symbols::unbacktick(&ident.name),
                "source" | "sys.source"
            )
        })
        .filter(|(index, ident)| {
            if matches!(kind, LoadingCallKind::Unshadowed) {
                return !model.ident_bindings(*index).is_empty();
            }
            model.ident_bindings(*index).iter().any(|id| {
                let binding = model.binding(*id);
                match binding.kind {
                    BindingKind::Param => true,
                    BindingKind::ForVar | BindingKind::Implicit => false,
                    BindingKind::Local => {
                        let target = root.covering_element(binding.def_range);
                        // A conditional definition still permits the base loader
                        // on the path where that definition does not execute.
                        for ancestor in target.ancestors().skip(1) {
                            match ancestor.kind() {
                                SyntaxKind::BLOCK_EXPR => {
                                    if ancestor.text_range().contains_range(ident.range) {
                                        return true;
                                    }
                                }
                                SyntaxKind::ROOT | SyntaxKind::FUNCTION_EXPR => return true,
                                _ => return false,
                            }
                        }
                        false
                    }
                }
            })
        })
        .map(|(_, ident)| ident.range)
        .collect();
    root.descendants()
        .filter_map(|child| source_call(&child))
        .filter(move |call| {
            call.base()
                .and_then(SyntaxElement::into_token)
                .is_none_or(|callee| !shadowed.contains(&callee.text_range()))
        })
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
pub(crate) fn string_literal_content(text: &str) -> Option<&str> {
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
    let (file_value, _local, _chdir) = source_file_value(call);
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

/// A `source` or `sys.source` call, bare or explicitly qualified with `base`.
pub(crate) fn source_call(node: &SyntaxNode) -> Option<CallExpr> {
    let call = CallExpr::cast(node.clone())?;
    let name = call.callee_name()?;
    if !matches!(
        crate::semantic::symbols::unbacktick(&name),
        "source" | "sys.source"
    ) {
        return None;
    }
    if let Some(node) = call.base().and_then(SyntaxElement::into_node) {
        let access = crate::ast::BinaryExpr::cast(node)?.namespace_access()?;
        if access.package != "base" {
            return None;
        }
    }
    Some(call)
}

/// Walk a `source(...)` call's arguments, returning the element that supplies
/// `file`, destination (`local` or `envir`), and `chdir` arguments. Match named (including unique partial)
/// arguments before positional arguments, as R does. Shared by [`source_edge`]
/// and [`collect_source_literal_edges`].
pub(crate) fn source_file_value(
    call: &CallExpr,
) -> (Option<SyntaxElement>, Option<SyntaxElement>, Option<bool>) {
    // `chdir` is the twelfth formal. Include the later formals so partial
    // matching doesn't confuse it with `catch.aborts` or `continue.echo`.
    const FORMALS: [&str; 17] = [
        "file",
        "local",
        "echo",
        "print.eval",
        "exprs",
        "spaced",
        "verbose",
        "prompt.echo",
        "max.deparse.length",
        "width.cutoff",
        "deparseCtrl",
        "chdir",
        "catch.aborts",
        "encoding",
        "continue.echo",
        "skip.echo",
        "keep.source",
    ];
    const SYS_FORMALS: [&str; 6] = [
        "file",
        "envir",
        "chdir",
        "keep.source",
        "keep.parse.data",
        "toplevel.env",
    ];
    let formals: &[&str] = if call.callee_name().as_deref() == Some("sys.source") {
        &SYS_FORMALS
    } else {
        &FORMALS
    };
    let mut values: Vec<Option<SyntaxElement>> = vec![None; formals.len()];
    let mut assigned = vec![false; formals.len()];
    let mut positional = Vec::new();
    if let Some(arg_list) = call.arg_list() {
        for arg in arg_list.args() {
            let (name, value) = arg_parts(arg.syntax());
            let Some(name) = name else {
                positional.push(value);
                continue;
            };
            let index = formals
                .iter()
                .position(|formal| *formal == name)
                .or_else(|| {
                    let mut matches = formals
                        .iter()
                        .enumerate()
                        .filter(|(_, formal)| formal.starts_with(&name));
                    let (index, _) = matches.next()?;
                    matches.next().is_none().then_some(index)
                });
            let Some(index) = index else {
                return (None, None, None);
            };
            if assigned[index] {
                return (None, None, None);
            }
            assigned[index] = true;
            values[index] = value;
        }
    }
    let mut unassigned = assigned
        .iter()
        .enumerate()
        .filter_map(|(index, assigned)| (!assigned).then_some(index));
    for value in positional {
        let Some(index) = unassigned.next() else {
            return (None, None, None);
        };
        values[index] = value;
    }
    let chdir_index = if formals == SYS_FORMALS { 2 } else { 11 };
    let chdir = match values[chdir_index].as_ref() {
        None => Some(false),
        Some(NodeOrToken::Token(token)) if token.kind() == SyntaxKind::IDENT => {
            match token.text() {
                "TRUE" => Some(true),
                "FALSE" => Some(false),
                _ => None,
            }
        }
        _ => None,
    };
    (values[0].take(), values[1].take(), chdir)
}

pub(crate) fn source_edge(call: &CallExpr, base_dir: Option<&Path>) -> SourceEdge {
    let (file_value, _, chdir) = source_file_value(call);
    let local = !super::source_env::source_destination(call).shares_file_scope();
    let target = match (&file_value, chdir) {
        (Some(value), Some(_)) => target_from_value(value, base_dir),
        _ => SourceTarget::Dynamic,
    };
    let relative_path = match (&file_value, &target) {
        (Some(NodeOrToken::Token(token)), SourceTarget::Path(_)) => strip_quotes(token.text())
            .filter(|path| is_relative_spelling(path))
            .map(PathBuf::from),
        _ => None,
    };
    SourceEdge {
        target,
        local,
        relative_path,
        chdir: chdir.unwrap_or(false),
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
    fn local_true_shares_the_file_environment() {
        let e = edges("source(\"helpers.R\", local = TRUE)\n", None);
        assert!(!e[0].local);
        assert!(e[0].contributes_scope());
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
