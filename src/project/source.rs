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

/// The `CallExpr` if `node` is a call to the bare function `source`.
fn source_call(node: &SyntaxNode) -> Option<CallExpr> {
    let call = CallExpr::cast(node.clone())?;
    let callee = call.callee_token()?;
    (callee.kind() == SyntaxKind::IDENT && callee.text() == "source").then_some(call)
}

fn source_edge(call: &CallExpr, base_dir: Option<&Path>) -> SourceEdge {
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
}
