//! Opt-in formatter verification.
//!
//! The ordinary formatting entry points stay single-pass. These counterparts
//! parse and render twice, comparing a layout-free projection between passes so
//! callers can pay for stronger guarantees when correctness matters more than
//! throughput.

use std::collections::HashSet;

use rowan::{NodeOrToken, TextRange};

use super::core::{FormatError, format_node};
use super::style::FormatStyle;
use crate::ast::{AstNode, Expr, ForExpr, FunctionExpr, IfExpr, RepeatExpr, WhileExpr};
use crate::parser::{ParseOptions, parse_with_options};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

/// Why verified formatting could not establish its guarantees.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormatVerificationError {
    /// The original input could not be formatted.
    Format(FormatError),
    /// The first pass emitted output that could not be parsed or reformatted.
    OutputInvalid(FormatError),
    /// The normalized syntax changed during formatting.
    SyntaxChanged { detail: String },
    /// An ordinary comment was changed, lost, duplicated, or reordered.
    CommentsChanged { detail: String },
    /// The second formatting pass did not reproduce the first pass byte-for-byte.
    NonIdempotent,
}

impl std::fmt::Display for FormatVerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(err) => err.fmt(f),
            Self::OutputInvalid(err) => {
                write!(f, "formatted output failed verification: {err}")
            }
            Self::SyntaxChanged { detail } => write!(
                f,
                "formatter verification failed (formatted output changed R syntax): {detail}"
            ),
            Self::CommentsChanged { detail } => write!(
                f,
                "formatter verification failed (formatted output changed ordinary comments): {detail}"
            ),
            Self::NonIdempotent => {
                f.write_str("formatter verification failed (non-idempotent output)")
            }
        }
    }
}

impl std::error::Error for FormatVerificationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Format(err) | Self::OutputInvalid(err) => Some(err),
            _ => None,
        }
    }
}

impl From<FormatError> for FormatVerificationError {
    fn from(value: FormatError) -> Self {
        Self::Format(value)
    }
}

/// Format with the default style, then verify syntax preservation, ordinary
/// comment preservation, and idempotence.
pub fn format_verified(input: &str) -> Result<String, FormatVerificationError> {
    format_verified_with_style(input, FormatStyle::default())
}

/// [`format_verified`] with an explicit style.
pub fn format_verified_with_style(
    input: &str,
    style: FormatStyle,
) -> Result<String, FormatVerificationError> {
    format_verified_with_options(input, style, &ParseOptions::default())
}

/// [`format_verified_with_style`] with parser options such as the package-wide
/// roxygen markdown default.
pub fn format_verified_with_options(
    input: &str,
    style: FormatStyle,
    options: &ParseOptions,
) -> Result<String, FormatVerificationError> {
    let before = parse_with_options(input, options);
    if !before.diagnostics.is_empty() {
        return Err(FormatVerificationError::Format(FormatError::ParseErrors {
            count: before.diagnostics.len(),
        }));
    }

    let formatted = format_node(&before.cst, style, input)?;
    let after = parse_with_options(&formatted, options);
    if !after.diagnostics.is_empty() {
        return Err(FormatVerificationError::OutputInvalid(
            FormatError::ParseErrors {
                count: after.diagnostics.len(),
            },
        ));
    }

    verify_preservation(&before.cst, &after.cst).map_err(|err| match err {
        PreservationError::SyntaxChanged { detail } => {
            FormatVerificationError::SyntaxChanged { detail }
        }
        PreservationError::CommentsChanged { detail } => {
            FormatVerificationError::CommentsChanged { detail }
        }
    })?;

    let reformatted = format_node(&after.cst, style, &formatted)
        .map_err(FormatVerificationError::OutputInvalid)?;
    if reformatted != formatted {
        return Err(FormatVerificationError::NonIdempotent);
    }
    Ok(formatted)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreservationError {
    SyntaxChanged { detail: String },
    CommentsChanged { detail: String },
}

impl std::fmt::Display for PreservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SyntaxChanged { detail } => write!(f, "syntax changed: {detail}"),
            Self::CommentsChanged { detail } => write!(f, "comments changed: {detail}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CanonicalEvent {
    Enter(SyntaxKind),
    Token(SyntaxKind, String),
    Leave(SyntaxKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocatedEvent {
    event: CanonicalEvent,
    range: TextRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ElementKey {
    range: TextRange,
    kind: SyntaxKind,
    node: bool,
}

impl ElementKey {
    fn new(element: &SyntaxElement) -> Self {
        Self {
            range: element.text_range(),
            kind: element.kind(),
            node: element.as_node().is_some(),
        }
    }
}

fn verify_preservation(before: &SyntaxNode, after: &SyntaxNode) -> Result<(), PreservationError> {
    let before_syntax = canonical_syntax(before);
    let after_syntax = canonical_syntax(after);
    let syntax_equal = before_syntax.len() == after_syntax.len()
        && before_syntax
            .iter()
            .zip(&after_syntax)
            .all(|(left, right)| left.event == right.event);
    if !syntax_equal {
        return Err(PreservationError::SyntaxChanged {
            detail: first_event_difference(&before_syntax, &after_syntax),
        });
    }

    let before_comments = ordinary_comments(before);
    let after_comments = ordinary_comments(after);
    if before_comments != after_comments {
        let index = before_comments
            .iter()
            .zip(&after_comments)
            .position(|(left, right)| left != right)
            .unwrap_or_else(|| before_comments.len().min(after_comments.len()));
        return Err(PreservationError::CommentsChanged {
            detail: format!(
                "comment {index}: input {:?}, formatted {:?}",
                before_comments.get(index),
                after_comments.get(index)
            ),
        });
    }
    Ok(())
}

fn first_event_difference(before: &[LocatedEvent], after: &[LocatedEvent]) -> String {
    let index = before
        .iter()
        .zip(after)
        .position(|(left, right)| left.event != right.event)
        .unwrap_or_else(|| before.len().min(after.len()));
    format!(
        "event {index}: input {}, formatted {}",
        describe_event(before.get(index)),
        describe_event(after.get(index))
    )
}

fn describe_event(event: Option<&LocatedEvent>) -> String {
    match event {
        Some(event) => format!("{:?} at {:?}", event.event, event.range),
        None => "<end of tree>".to_string(),
    }
}

fn canonical_syntax(root: &SyntaxNode) -> Vec<LocatedEvent> {
    let body_elements = body_elements(root);
    let mut events = Vec::new();
    emit_node(root, &body_elements, &mut events);
    events
}

fn body_elements(root: &SyntaxNode) -> HashSet<ElementKey> {
    let mut bodies = HashSet::new();
    for node in root.descendants() {
        match node.kind() {
            SyntaxKind::FUNCTION_EXPR => {
                if let Some(body) = FunctionExpr::cast(node).and_then(|expr| expr.body()) {
                    bodies.insert(ElementKey::new(&body));
                }
            }
            SyntaxKind::IF_EXPR => {
                if let Some(expr) = IfExpr::cast(node) {
                    if let Some(body) = expr.then_elements().and_then(sole_expression) {
                        bodies.insert(ElementKey::new(&body));
                    }
                    if let Some(body) = expr.else_elements().and_then(sole_expression) {
                        bodies.insert(ElementKey::new(&body));
                    }
                }
            }
            SyntaxKind::FOR_EXPR => {
                if let Some(body) = ForExpr::cast(node).and_then(|expr| expr.body_element()) {
                    bodies.insert(ElementKey::new(&body));
                }
            }
            SyntaxKind::WHILE_EXPR => {
                if let Some(body) = WhileExpr::cast(node).and_then(|expr| expr.body_element()) {
                    bodies.insert(ElementKey::new(&body));
                }
            }
            SyntaxKind::REPEAT_EXPR => {
                if let Some(body) = RepeatExpr::cast(node).and_then(|expr| expr.body()) {
                    bodies.insert(ElementKey::new(&body));
                }
            }
            _ => {}
        }
    }
    bodies
}

fn sole_expression(elements: Vec<SyntaxElement>) -> Option<SyntaxElement> {
    let mut expression = None;
    for element in elements {
        if is_ignored_token(element.kind()) || element.kind() == SyntaxKind::ROXYGEN_BLOCK {
            continue;
        }
        Expr::cast(element.clone())?;
        if expression.replace(element).is_some() {
            return None;
        }
    }
    expression
}

fn emit_node(node: &SyntaxNode, bodies: &HashSet<ElementKey>, events: &mut Vec<LocatedEvent>) {
    if node.kind() == SyntaxKind::ROXYGEN_BLOCK {
        return;
    }
    // `ARG` is parser scaffolding around comma-delimited content. In
    // particular, a comment-only call line becomes an empty `ARG` after
    // comments are projected out; commas still preserve real argument holes.
    let transparent = node.kind() == SyntaxKind::ARG;
    if !transparent {
        events.push(LocatedEvent {
            event: CanonicalEvent::Enter(node.kind()),
            range: node.text_range(),
        });
    }
    for child in node.children_with_tokens() {
        emit_element(child, bodies, events);
    }
    if !transparent {
        events.push(LocatedEvent {
            event: CanonicalEvent::Leave(node.kind()),
            range: node.text_range(),
        });
    }
}

fn emit_element(
    element: SyntaxElement,
    bodies: &HashSet<ElementKey>,
    events: &mut Vec<LocatedEvent>,
) {
    if bodies.contains(&ElementKey::new(&element))
        && let Some(node) = element.as_node()
        && let Some(expression) = single_block_expression(node)
    {
        emit_element(expression, bodies, events);
        return;
    }

    match element {
        NodeOrToken::Node(node) => emit_node(&node, bodies, events),
        NodeOrToken::Token(token) if is_ignored_token(token.kind()) => {}
        NodeOrToken::Token(token) => events.push(LocatedEvent {
            event: CanonicalEvent::Token(token.kind(), token.text().to_string()),
            range: token.text_range(),
        }),
    }
}

fn single_block_expression(node: &SyntaxNode) -> Option<SyntaxElement> {
    if node.kind() != SyntaxKind::BLOCK_EXPR {
        return None;
    }
    let mut expression = None;
    for element in node.children_with_tokens() {
        if is_ignored_token(element.kind())
            || element.kind() == SyntaxKind::ROXYGEN_BLOCK
            || matches!(element.kind(), SyntaxKind::LBRACE | SyntaxKind::RBRACE)
        {
            continue;
        }
        Expr::cast(element.clone())?;
        if expression.replace(element).is_some() {
            return None;
        }
    }
    expression
}

fn is_ignored_token(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::SEMICOLON | SyntaxKind::COMMENT
    )
}

fn ordinary_comments(root: &SyntaxNode) -> Vec<String> {
    let mut comments = Vec::new();
    collect_comments(root, false, &mut comments);
    comments
}

fn collect_comments(node: &SyntaxNode, in_roxygen: bool, comments: &mut Vec<String>) {
    let in_roxygen = in_roxygen || node.kind() == SyntaxKind::ROXYGEN_BLOCK;
    for child in node.children_with_tokens() {
        match child {
            NodeOrToken::Node(node) => collect_comments(&node, in_roxygen, comments),
            NodeOrToken::Token(token) if !in_roxygen && token.kind() == SyntaxKind::COMMENT => {
                comments.push(token.text().trim_end().to_string());
            }
            NodeOrToken::Token(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PreservationError, verify_preservation};
    use crate::parser::parse;

    fn verify(before: &str, after: &str) -> Result<(), PreservationError> {
        verify_preservation(&parse(before).cst, &parse(after).cst)
    }

    #[test]
    fn ignores_layout_comments_and_statement_separators() {
        verify("x <- 1; # first\ny <- 2\n", "# first\nx<-1\ny<-2\n").unwrap();
    }

    #[test]
    fn ignores_trailing_whitespace_in_ordinary_comments() {
        verify("x # comment \n", "x # comment\n").unwrap();
    }

    #[test]
    fn normalizes_single_expression_body_blocks() {
        for (before, after) in [
            ("function(x) x", "function(x) { x }"),
            ("if (x) y else z", "if (x) { y } else { z }"),
            ("for (x in xs) f(x)", "for (x in xs) { f(x) }"),
            ("while (x) f()", "while (x) { f() }"),
            ("repeat f()", "repeat { f() }"),
        ] {
            verify(before, after).unwrap_or_else(|err| panic!("{before:?} != {after:?}: {err}"));
            verify(after, before).unwrap_or_else(|err| panic!("{after:?} != {before:?}: {err}"));
        }
    }

    #[test]
    fn retains_blocks_outside_body_positions_and_nested_blocks() {
        assert!(matches!(
            verify("x <- { y }", "x <- y"),
            Err(PreservationError::SyntaxChanged { .. })
        ));
        assert!(matches!(
            verify("function() {{ x }}", "function() x"),
            Err(PreservationError::SyntaxChanged { .. })
        ));
        assert!(matches!(
            verify("function() { x; y }", "function() x"),
            Err(PreservationError::SyntaxChanged { .. })
        ));
    }

    #[test]
    fn detects_tree_token_and_comment_changes() {
        for (before, after) in [
            ("x + y", "x * y"),
            ("(x + y) * z", "x + y * z"),
            ("f(x, , y)", "f(x, y)"),
            ("x <- { y }", "x <- y"),
        ] {
            assert!(matches!(
                verify(before, after),
                Err(PreservationError::SyntaxChanged { .. })
            ));
        }

        assert!(matches!(
            verify("x # one\n", "x # two\n"),
            Err(PreservationError::CommentsChanged { .. })
        ));
        assert!(matches!(
            verify("# one\n# two\nx\n", "# two\n# one\nx\n"),
            Err(PreservationError::CommentsChanged { .. })
        ));
    }

    #[test]
    fn comment_only_arguments_do_not_change_syntax() {
        verify("f(\n  # comment\n)\n", "# comment\nf()\n").unwrap();
    }

    #[test]
    fn ignores_roxygen_content_but_not_executable_code() {
        verify("#' one\nf()\n", "#' completely different\nf()\n").unwrap();
        assert!(matches!(
            verify("#' one\nf()\n", "one\nf()\n"),
            Err(PreservationError::SyntaxChanged { .. })
        ));
    }
}
