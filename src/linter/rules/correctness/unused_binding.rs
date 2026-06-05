//! `unused-binding`: a local binding that is never read in the same file.
//!
//! Excludes function parameters and `for`-loop variables (those have semantic
//! meaning even when unused — they're part of the API surface). Names starting
//! with `.` are skipped too, following R convention for intentionally unused
//! identifiers.

use rowan::TextRange;

use crate::linter::diagnostic::{Diagnostic, Fix, Severity, ViolationData};
use crate::linter::rules::{Rule, RuleContext};
use crate::semantic::ScopeKind;
use crate::syntax::{SyntaxKind, SyntaxNode};

pub struct UnusedBinding;

impl Rule for UnusedBinding {
    fn id(&self) -> &'static str {
        "unused-binding"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let src = ctx.root.text().to_string();
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
                    severity: Severity::Warning,
                    path: Default::default(),
                    range: b.def_range,
                    message: ViolationData::new(
                        "unused-binding",
                        format!("local binding `{}` is assigned but never read", b.name),
                    )
                    .with_suggestion("Remove the assignment, or prefix the name with `.` to mark it intentional."),
                    fix,
                }
            })
            .collect()
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

    // Tenet 5: never produce output the formatter would rewrite. Inside a
    // block, a pure deletion is unsafe when it would leave the block empty
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

    let (start, end) = deletion_span(src, assign.text_range());
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

/// Widen a statement's range to swallow its leading indentation, its own line
/// terminator, and any wholly-blank lines that follow — but not the next
/// content line's indentation. When the statement is the last content in the
/// file, preceding blank lines are absorbed too. Together with the block guards
/// in [`deletion_fix`], this keeps the deletion format-clean by construction
/// (tenet 5).
fn deletion_span(src: &str, range: TextRange) -> (usize, usize) {
    let bytes = src.as_bytes();

    // Leading indentation on the statement's line.
    let mut start = usize::from(range.start());
    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }

    // Trailing horizontal whitespace, then the statement's own newline.
    let mut end = usize::from(range.end());
    while end < bytes.len() && matches!(bytes[end], b' ' | b'\t') {
        end += 1;
    }
    end = consume_newline(bytes, end);

    // Absorb any fully-blank lines that follow, stopping before the next
    // line that carries content (so its indentation survives).
    loop {
        let mut probe = end;
        while probe < bytes.len() && matches!(bytes[probe], b' ' | b'\t') {
            probe += 1;
        }
        let after_nl = consume_newline(bytes, probe);
        if after_nl == probe {
            break; // line has content (or EOF) — keep it intact
        }
        end = after_nl;
    }

    // If nothing but whitespace follows (the statement was the last content),
    // also pull back over preceding blank lines so we don't leave a trailing
    // blank, keeping the previous content line's own terminator.
    if end == bytes.len() {
        let mut prev = start;
        while prev > 0 && matches!(bytes[prev - 1], b' ' | b'\t' | b'\n' | b'\r') {
            prev -= 1;
        }
        start = if prev > 0 {
            consume_newline(bytes, prev)
        } else {
            0
        };
    }

    (start, end)
}

/// Advance past a single `\n` or `\r\n` at `i`, else return `i` unchanged.
fn consume_newline(bytes: &[u8], i: usize) -> usize {
    match bytes.get(i) {
        Some(b'\n') => i + 1,
        Some(b'\r') if bytes.get(i + 1) == Some(&b'\n') => i + 2,
        _ => i,
    }
}
