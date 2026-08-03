//! `unused-binding`: a local binding that is never read in the same file.
//!
//! Excludes function parameters and `for`-loop variables (those have semantic
//! meaning even when unused — they're part of the API surface). Names starting
//! with `.` are skipped too, following R convention for intentionally unused
//! identifiers.

use rowan::TextRange;

use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, matchers};
use crate::semantic::ScopeKind;
use crate::syntax::{SyntaxKind, SyntaxNode};

pub struct UnusedBinding;

impl Rule for UnusedBinding {
    fn id(&self) -> &'static str {
        "unused-binding"
    }

    fn description(&self) -> &'static str {
        "Flag a local binding that is never read in the same file. Function \
         parameters, `for`-loop variables, and names beginning with `.` are \
         exempt, since those are meaningful even when unused."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "`x` is assigned but never used:",
            source: "x <- 1\ny <- 2\nprint(y)\n",
        }]
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let src = ctx.root.text().to_string();
        sink.extend(
            ctx.model
                .unused_local_bindings()
                // A top-level binding read by a sibling file (same package or
                // source-closure) is used cross-file, so it isn't unused.
                .filter(|id| {
                    let b = ctx.model.binding(*id);
                    let top_level = ctx.model.scope(b.scope).kind == ScopeKind::File;
                    !(top_level && ctx.project.is_some_and(|p| p.used_elsewhere(&b.name)))
                })
                .map(|id| {
                    let b = ctx.model.binding(id);
                    let fix = deletion_fix(ctx.root, &src, &b.name, b.def_range);
                    Diagnostic {
                        rule: "unused-binding",
                        severity: Default::default(),
                        path: Default::default(),
                        range: b.def_range,
                        message: ViolationData::new(
                            "unused-binding",
                            format!("local binding `{}` is assigned but never read", b.name),
                        )
                        .with_suggestion("Remove the assignment, or prefix the name with `.` to mark it intentional."),
                        fix,
                    }
                }),
        );
    }
}

/// Build an (unsafe) fix that deletes the entire assignment statement that
/// binds `def_range`. Returns `None` unless the binding is the direct LHS of an
/// assignment that is itself a statement (a child of `ROOT`/`BLOCK_EXPR`) — a
/// nested or chained assignment (`z <- (x <- 1)`) is too risky to rewrite.
fn deletion_fix(root: &SyntaxNode, src: &str, name: &str, def_range: TextRange) -> Option<Fix> {
    let token = root.covering_element(def_range).into_token()?;
    let assign = token.parent()?;
    if assign.kind() != SyntaxKind::ASSIGNMENT_EXPR {
        return None;
    }
    let parent = assign.parent()?;
    if !matches!(parent.kind(), SyntaxKind::ROOT | SyntaxKind::BLOCK_EXPR) {
        return None;
    }
    // Confirm the binding identifier is the LHS (first IDENT child), not an
    // identifier elsewhere in the assignment.
    let lhs = assign
        .children_with_tokens()
        .find_map(|e| e.into_token().filter(|t| t.kind() == SyntaxKind::IDENT))?;
    if lhs.text_range() != def_range {
        return None;
    }

    // A chained assignment (`a <- b <- expr`) parses as `a <- (b <- expr)`, so
    // this statement's value side is itself an ASSIGNMENT_EXPR. Deleting the whole
    // statement to drop the unused outer target `a` would also drop the inner
    // `b <- expr` binding, which may be live (read elsewhere) — a semantic change.
    // Withhold; the finding is still reported.
    if assign
        .children()
        .any(|child| child.kind() == SyntaxKind::ASSIGNMENT_EXPR)
    {
        return None;
    }

    // Autofix correctness: never produce output the formatter would rewrite.
    // Inside a block, a pure deletion is unsafe when it would leave the block empty
    // (`{\n}` → `{}`) or shrink a function body to a single statement (which
    // flattens to a bare body). Withhold the fix for those shapes — the
    // finding is still reported.
    if parent.kind() == SyntaxKind::BLOCK_EXPR {
        let remaining = block_statement_count(&parent).saturating_sub(1);
        let is_function_body = parent.parent().map(|g| g.kind()) == Some(SyntaxKind::FUNCTION_EXPR);
        if remaining == 0 || (remaining == 1 && is_function_body) {
            return None;
        }
    }

    let (start, end) = matchers::deletion_span(src, assign.text_range());
    Some(Fix::unsafe_(
        start,
        end,
        "",
        format!("Remove unused binding `{name}`"),
    ))
}

/// Count the statements in a `BLOCK_EXPR` — its child elements other than the
/// braces and trivia (whitespace / newlines / comments).
fn block_statement_count(block: &SyntaxNode) -> usize {
    block
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::LBRACE
                    | SyntaxKind::RBRACE
                    | SyntaxKind::WHITESPACE
                    | SyntaxKind::NEWLINE
                    | SyntaxKind::COMMENT
            )
        })
        .count()
}
