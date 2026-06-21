//! `unreachable-code`: statements after an unconditional `return()`/`stop()`.
//!
//! Once a block runs `return(...)` (hand control back to the caller) or
//! `stop(...)` (raise an error), nothing after it in the same block can execute.
//! Such trailing code is almost always a bug — leftover dead code, or a
//! statement that was meant to run before the terminator.
//!
//! The rule fires only on the unambiguous shape: a terminator that is a **direct
//! statement** of a `BLOCK_EXPR` with at least one statement following it. A
//! `return()`/`stop()` nested inside an `if` (or any other expression) is not a
//! direct statement, so the tail stays reachable and is correctly left alone.
//!
//! It is **namespace-confirmed** (`ns`): the callee must resolve to base R via
//! [`RuleContext::resolves_to_base`]; a local redefinition of `return`/`stop`
//! no longer terminates, so the following code is reachable. `return` is
//! additionally gated on an enclosing `FUNCTION_EXPR` — outside a function it is
//! not the unreachable-after-return shape (`stop` halts anywhere, so it needs no
//! such gate).
//!
//! The fix deletes the unreachable statements. It is **unsafe** (deleting code,
//! even provably-dead code, can change behavior if the analysis is imperfect or
//! the code had side effects the author wanted) and is **withheld** when a
//! comment sits inside the deleted region, which the textual edit would silently
//! drop (autofix-correctness discipline) — the finding is still reported.
//!
//! Known limitation: a block whose `if`/`else` returns in *both* branches also
//! leaves its tail unreachable, but proving that needs control-flow analysis and
//! is out of scope here.

use rowan::TextRange;

use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, matchers};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

pub struct UnreachableCode;

const EXAMPLES: &[Example] = &[Example {
    caption: "A statement after `return()` can never run:",
    source: "f <- function() {\n  return(1)\n  2\n}\n",
}];

impl Rule for UnreachableCode {
    fn id(&self) -> &'static str {
        "unreachable-code"
    }

    fn description(&self) -> &'static str {
        "Flag statements that follow an unconditional `return()` or `stop()` in a \
         block — once either runs, nothing after it in the same block can be \
         reached, so the trailing code is dead.\n\nThe rule fires only when the \
         terminator is a direct statement of the block (a `return()`/`stop()` \
         guarded by an `if` leaves the tail reachable) and only when the callee \
         resolves to base R; a local redefinition is left alone. `return` is \
         additionally required to sit inside a function. The deletion fix is \
         unsafe, and withheld when it would drop a comment."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BLOCK_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(block) = el.as_node() else {
            return;
        };

        // The block's statements, in order — its children other than the braces,
        // statement separators, and trivia. A statement is a node *or* a bare
        // token (e.g. `2`), so keep them as elements.
        let stmts: Vec<SyntaxElement> = block
            .children_with_tokens()
            .filter(|e| {
                !matches!(
                    e.kind(),
                    SyntaxKind::LBRACE
                        | SyntaxKind::RBRACE
                        | SyntaxKind::SEMICOLON
                        | SyntaxKind::WHITESPACE
                        | SyntaxKind::NEWLINE
                        | SyntaxKind::COMMENT
                )
            })
            .collect();

        // First statement that unconditionally terminates the block.
        let Some((idx, term)) = stmts
            .iter()
            .enumerate()
            .find_map(|(i, s)| terminator_name(s, ctx, block).map(|n| (i, n)))
        else {
            return;
        };
        // Nothing follows it — nothing unreachable.
        if idx + 1 >= stmts.len() {
            return;
        }

        let first = &stmts[idx + 1];
        let last = stmts.last().expect("at least one statement follows");
        let region = TextRange::new(first.text_range().start(), last.text_range().end());

        // The fix deletes `region`; withhold it if a comment lives inside there,
        // which a textual edit would silently drop.
        let drops_comment = block
            .descendants_with_tokens()
            .any(|e| e.kind() == SyntaxKind::COMMENT && region.contains_range(e.text_range()));
        let fix = (!drops_comment).then(|| {
            let src = ctx.root.text().to_string();
            let (start, end) = matchers::deletion_span(&src, region);
            Fix::unsafe_(start, end, "", "Remove the unreachable code")
        });

        sink.push(Diagnostic {
            rule: "unreachable-code",
            severity: Severity::Warning,
            path: Default::default(),
            range: region,
            message: ViolationData::new(
                "unreachable-code",
                format!("code after `{term}()` can never be reached"),
            )
            .with_suggestion("Remove the unreachable code, or fix the control flow."),
            fix,
        });
    }
}

/// The terminator name (`"return"`/`"stop"`) if `stmt` is an unconditional,
/// base-R terminating call statement of `block`; `None` otherwise. `return` is
/// gated on an enclosing function — outside one it does not terminate the way the
/// rule means.
fn terminator_name(
    stmt: &SyntaxElement,
    ctx: &RuleContext<'_>,
    block: &SyntaxNode,
) -> Option<&'static str> {
    let node = stmt.as_node()?;
    for name in ["return", "stop"] {
        if let Some(call) = matchers::call_named(node, name) {
            // A local redefinition means the call no longer terminates.
            if !ctx.resolves_to_base(&call) {
                return None;
            }
            if name == "return" && !in_function(block) {
                return None;
            }
            return Some(name);
        }
    }
    None
}

/// Whether `block` is nested (at any depth) inside a function body.
fn in_function(block: &SyntaxNode) -> bool {
    block
        .ancestors()
        .any(|n| n.kind() == SyntaxKind::FUNCTION_EXPR)
}
