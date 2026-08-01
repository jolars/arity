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
//! It also fires on the **both-branches** shape: a direct-statement `if`/`else`
//! that exits in *both* arms (each arm diverges via `return()`/`stop()`) leaves
//! the block's tail unreachable too. This case is control-flow driven — the
//! per-file CFG ([`RuleContext::cfg`]) marks the statements after such an `if`
//! unreachable — so the rule reads that verdict rather than re-deriving it.
//!
//! It is **namespace-confirmed** (`ns`): the callee must resolve to base R via
//! [`RuleContext::resolves_to_base`]; a local redefinition of `return`/`stop`
//! no longer terminates, so the following code is reachable (for the
//! both-branches shape, *every* `return`/`stop` responsible for the divergence
//! must so resolve). `return` is additionally gated on an enclosing
//! `FUNCTION_EXPR` — outside a function it is not the unreachable-after-return
//! shape (`stop` halts anywhere, so it needs no such gate).
//!
//! The fix deletes the unreachable statements. It is **unsafe** (deleting code,
//! even provably-dead code, can change behavior if the analysis is imperfect or
//! the code had side effects the author wanted) and is **withheld** when a
//! comment sits inside the deleted region, which the textual edit would silently
//! drop (autofix-correctness discipline) — the finding is still reported.

use rowan::TextRange;
use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, matchers};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

pub struct UnreachableCode;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "A statement after `return()` can never run:",
        source: "f <- function() {\n  return(1)\n  2\n}\n",
    },
    Example {
        caption: "An `if`/`else` that exits in both branches leaves its tail dead:",
        source: "f <- function() {\n  if (x) return(1) else return(2)\n  3\n}\n",
    },
];

impl Rule for UnreachableCode {
    fn id(&self) -> &'static str {
        "unreachable-code"
    }

    fn description(&self) -> &'static str {
        "Flag statements that follow an unconditional `return()` or `stop()` in a \
         block—once either runs, nothing after it in the same block can be \
         reached, so the trailing code is dead. A direct-statement `if`/`else` \
         that exits in both branches likewise leaves its tail unreachable (a \
         control-flow-graph verdict).\n\nThe rule fires only when the terminator \
         is a direct statement of the block (a lone `return()`/`stop()` guarded \
         by an `if` leaves the tail reachable) and only when the callee resolves \
         to base R; a local redefinition is left alone. `return` is additionally \
         required to sit inside a function. The deletion fix is unsafe, and \
         withheld when it would drop a comment."
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

        // First statement after which control cannot fall through — either a
        // direct `return()`/`stop()` call, or an `if`/`else` that exits in both
        // arms (a CFG verdict). A terminator only matters when something follows
        // it, so pair the search with "there is a next statement."
        let Some((idx, term)) = stmts.iter().enumerate().find_map(|(i, s)| {
            if i + 1 >= stmts.len() {
                return None;
            }
            if let Some(name) = terminator_name(s, ctx, block) {
                return Some((i, Terminator::Call(name)));
            }
            both_branches_diverge(s, &stmts[i + 1], ctx, block).then_some((i, Terminator::BothArms))
        }) else {
            return;
        };

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
            severity: Default::default(),
            path: Default::default(),
            range: region,
            message: ViolationData::new("unreachable-code", term.message())
                .with_suggestion("Remove the unreachable code, or fix the control flow."),
            fix,
        });
    }
}

/// What made a block's tail unreachable, for the diagnostic message.
enum Terminator {
    /// A direct `return()`/`stop()` call statement (the name).
    Call(&'static str),
    /// An `if`/`else` that exits in both arms.
    BothArms,
}

impl Terminator {
    fn message(&self) -> String {
        match self {
            Terminator::Call(name) => format!("code after `{name}()` can never be reached"),
            Terminator::BothArms => {
                "code after this `if` can never be reached (both branches exit)".to_string()
            }
        }
    }
}

/// Whether `stmt` is an `if`/`else` after which control cannot fall through:
/// the CFG marks the following statement unreachable (both arms diverge), and
/// every `return`/`stop` responsible for that divergence resolves to base R
/// (with `return` gated on an enclosing function, as for the direct shape). A
/// local redefinition of `return`/`stop` breaks the divergence, so the tail is
/// then reachable and the rule stays silent.
fn both_branches_diverge(
    stmt: &SyntaxElement,
    next: &SyntaxElement,
    ctx: &RuleContext<'_>,
    block: &SyntaxNode,
) -> bool {
    let Some(node) = stmt.as_node() else {
        return false;
    };
    if node.kind() != SyntaxKind::IF_EXPR {
        return false;
    }
    // The CFG's reachability verdict: is the statement right after the `if`
    // provably dead? (True only when both arms exit.)
    if !ctx.cfg.is_unreachable(next.text_range()) {
        return false;
    }
    // Namespace-confirm every terminating call, and gate `return` on a function.
    let mut saw_return = false;
    for call in node.descendants().filter_map(CallExpr::cast) {
        if let Some(name @ ("return" | "stop")) = call.callee_name().as_deref() {
            if !ctx.resolves_to_base(&call) {
                return false;
            }
            saw_return |= name == "return";
        }
    }
    !saw_return || in_function(block)
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
