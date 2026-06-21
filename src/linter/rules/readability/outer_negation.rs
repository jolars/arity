//! `outer-negation`: De Morgan — `any(!x)` is `!all(x)` and `all(!x)` is
//! `!any(x)`. Pulling the negation outside the aggregation reads more clearly.
//!
//! The rule fires only on the clean De Morgan shape: a call to `any`/`all`
//! whose every positional argument is a `!`-negation. A `na.rm` argument is
//! allowed and passed through untouched; any other named argument, or a mix of
//! negated and plain arguments, is left alone (the transformation would not be
//! a simple operator swap). The equivalence holds under R's three-valued logic
//! and `na.rm`, so the fix is `Safe`.
//!
//! The rewrite replaces a call (a primary) with a `!`-expression, which binds
//! *looser* than the call it replaces. In a parent context that binds tighter
//! than `!` (arithmetic, comparison, indexing, `$`/`@`, …) the bare `!all(x)`
//! would misparse, so the fix is withheld there — see [`is_safe_context`]. The
//! finding is still reported.

use rowan::NodeOrToken;
use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

pub struct OuterNegation;

const EXAMPLES: &[Example] = &[Example {
    caption: "Negating every element of an aggregation:",
    source: "if (any(!ok)) stop()\n",
}];

impl Rule for OuterNegation {
    fn id(&self) -> &'static str {
        "outer-negation"
    }

    fn description(&self) -> &'static str {
        "Flag `any(!x)` / `all(!x)`, which by De Morgan's law read more clearly \
         with the negation pulled outside: `!all(x)` and `!any(x)`.\n\nThe rule \
         fires only when *every* positional argument is negated (a `na.rm` \
         argument is allowed and preserved). The fix is withheld when the call \
         sits in a context that binds tighter than `!`, where the rewrite would \
         need parentheses."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, _ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(call) = CallExpr::cast(node.clone()) else {
            return;
        };
        let other = match matchers::callee_name(&call).as_deref() {
            Some("any") => "all",
            Some("all") => "any",
            _ => return,
        };

        // Every positional argument must be a `!`-negation; the only named
        // argument allowed (and preserved) is `na.rm`.
        let args = matchers::args(&call);
        let mut negated = 0usize;
        for arg in &args {
            match arg.name.as_deref() {
                Some("na.rm") => {}
                Some(_) => return,
                None => {
                    let Some(value) = &arg.value else { return };
                    if bang_of(value).is_none() {
                        return;
                    }
                    negated += 1;
                }
            }
        }
        if negated == 0 {
            return;
        }

        let r = call.syntax().text_range();
        let fix = if is_safe_context(call.syntax()) {
            build_replacement(&call, other).map(|content| {
                Fix::safe(
                    usize::from(r.start()),
                    usize::from(r.end()),
                    content,
                    format!("Rewrite as `!{other}(...)`"),
                )
            })
        } else {
            None
        };
        sink.push(Diagnostic {
            rule: "outer-negation",
            severity: Severity::Warning,
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "outer-negation",
                "negating an aggregation is clearer with the negation outside",
            )
            .with_suggestion("`any(!x)` is `!all(x)`; `all(!x)` is `!any(x)`."),
            fix,
        });
    }
}

/// The leading `!` token of a positional argument value (`!x`), or `None` if the
/// value is not a `!`-negation.
fn bang_of(value: &SyntaxElement) -> Option<SyntaxToken> {
    let node = value.as_node()?;
    if node.kind() != SyntaxKind::UNARY_EXPR {
        return None;
    }
    let tok = node.children_with_tokens().find_map(|e| e.into_token())?;
    (tok.kind() == SyntaxKind::BANG).then_some(tok)
}

/// Build `!<other>(<args>)` from the call, swapping the callee and stripping the
/// leading `!` from each negated positional argument. All other text (commas,
/// whitespace, comments, the `na.rm` argument) is preserved verbatim.
fn build_replacement(call: &CallExpr, other: &str) -> Option<String> {
    let callee = call.callee_token()?;
    let mut out = String::from("!");
    for child in call.syntax().children_with_tokens() {
        match child {
            NodeOrToken::Token(t) if t.text_range() == callee.text_range() => out.push_str(other),
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ARG_LIST => {
                out.push_str(&render_arg_list(&n));
            }
            el => out.push_str(&matchers::element_text(&el)),
        }
    }
    Some(out)
}

fn render_arg_list(list: &SyntaxNode) -> String {
    let mut out = String::new();
    for child in list.children_with_tokens() {
        match child {
            NodeOrToken::Node(n) if n.kind() == SyntaxKind::ARG => out.push_str(&render_arg(&n)),
            el => out.push_str(&matchers::element_text(&el)),
        }
    }
    out
}

/// An argument's text with the leading `!` removed if it is a negated positional
/// argument; otherwise verbatim (named arguments like `na.rm = !x` keep theirs).
fn render_arg(arg: &SyntaxNode) -> String {
    let text = arg.text().to_string();
    // A named argument (`name = value`) is never stripped.
    let named = arg
        .children_with_tokens()
        .any(|e| e.kind() == SyntaxKind::ASSIGN_EQ);
    if !named
        && let Some(unary) = arg.children().next()
        && unary.kind() == SyntaxKind::UNARY_EXPR
        && let Some(bang) = unary.children_with_tokens().find_map(|e| e.into_token())
        && bang.kind() == SyntaxKind::BANG
    {
        let rel = usize::from(bang.text_range().start()) - usize::from(arg.text_range().start());
        // `!` is a single byte.
        return format!("{}{}", &text[..rel], &text[rel + 1..]);
    }
    text
}

/// Whether a `!`-expression is safe to splice in unparenthesized at `node`'s
/// position. Safe when the parent does not bind tighter than `!`: a statement
/// position, a delimited clause/argument, an assignment, an outer `!`, or a
/// looser logical/formula operator. Anything tighter (arithmetic, comparison,
/// indexing, `$`/`@`, a call) would capture the rewrite, so it is unsafe.
fn is_safe_context(node: &SyntaxNode) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    match parent.kind() {
        SyntaxKind::ROOT
        | SyntaxKind::BLOCK_EXPR
        | SyntaxKind::PAREN_EXPR
        | SyntaxKind::ARG
        | SyntaxKind::IF_EXPR
        | SyntaxKind::WHILE_EXPR
        | SyntaxKind::FOR_EXPR
        | SyntaxKind::REPEAT_EXPR
        | SyntaxKind::ASSIGNMENT_EXPR => true,
        SyntaxKind::BINARY_EXPR => matchers::binary_parts(&parent).is_some_and(|(_, op, _)| {
            matches!(
                op.kind(),
                SyntaxKind::AND
                    | SyntaxKind::AND2
                    | SyntaxKind::OR
                    | SyntaxKind::OR2
                    | SyntaxKind::TILDE
            )
        }),
        SyntaxKind::UNARY_EXPR => parent
            .children_with_tokens()
            .find_map(|e| e.into_token())
            .is_some_and(|t| t.kind() == SyntaxKind::BANG),
        _ => false,
    }
}
