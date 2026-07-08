//! `string-boundary`: `grepl("^abc", x)` is the clearer `startsWith(x, "abc")`;
//! `grepl("abc$", x)` is `endsWith(x, "abc")`.
//!
//! A regex anchored at one end against an otherwise fixed string is really a
//! prefix/suffix test. `startsWith`/`endsWith` say that directly, skip regex
//! compilation, and are the idiomatic base-R spelling.
//!
//! The rule fires only on the clean, unambiguous shape: `grepl` with exactly two
//! positional arguments (pattern, subject) and no named arguments—so a
//! `fixed = TRUE`, `ignore.case`, or `perl` flag that would change what the match
//! means can't be present. The pattern must be a single string literal that is
//! anchored at exactly one end (`^…` or `…$`, not both) with a plain-literal
//! remainder (no other regex metacharacter or backslash escape); otherwise the
//! rewrite would not be equivalent.
//!
//! It is **namespace-confirmed** (`ns`): the `grepl` callee must resolve to base
//! R (not a local redefinition, namespace-qualified, or package-masked name).
//!
//! **The fix is `Unsafe`.** `startsWith`/`endsWith` are not perfectly equivalent
//! to `grepl` at the edges: on an `NA` element `grepl` yields `FALSE` while
//! `startsWith`/`endsWith` yield `NA`, and on a non-character subject `grepl`
//! coerces via `as.character` while `startsWith`/`endsWith` error. These are real
//! runtime differences, so the fix is applied only under `--unsafe-fixes`; the
//! finding is always reported. The replacement is a call (a primary), so it
//! splices in for the whole `grepl(...)` without a precedence guard. It is
//! withheld when a comment outside the preserved subject/pattern would be
//! dropped.

use rowan::ast::AstNode as _;

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxToken};

pub struct StringBoundary;

const EXAMPLES: &[Example] = &[Example {
    caption: "Anchored fixed-string matches:",
    source: "grepl(\"^abc\", x)\n",
}];

impl Rule for StringBoundary {
    fn id(&self) -> &'static str {
        "string-boundary"
    }

    fn description(&self) -> &'static str {
        "Flag `grepl(\"^abc\", x)` and `grepl(\"abc$\", x)`, single-anchored \
         fixed-string matches that are the clearer `startsWith(x, \"abc\")` and \
         `endsWith(x, \"abc\")`—they state the prefix/suffix test directly and \
         skip regex compilation.\n\nThe rule fires only on the clean shape (two \
         positional arguments, a one-end-anchored plain-literal pattern) and only \
         when `grepl` resolves to base R. The fix is **unsafe**: on `NA` or \
         non-character input `startsWith`/`endsWith` diverge from `grepl` (`NA` \
         vs `FALSE`, an error vs coercion)."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some(call) = matchers::call_named(node, "grepl") else {
            return;
        };

        // Require exactly two positional, value-bearing arguments and no named
        // ones: a `fixed`/`ignore.case`/`perl` flag would change what the pattern
        // means, and only `grepl(pattern, x)` maps cleanly onto the rewrite.
        let args = matchers::args(&call);
        let valued: Vec<_> = args.iter().filter(|a| a.value.is_some()).collect();
        if valued.len() != 2 || valued.iter().any(|a| a.name.is_some()) {
            return;
        }
        let pattern_el = valued[0].value.as_ref().unwrap();
        let subject_el = valued[1].value.as_ref().unwrap();

        // The pattern must be a single string literal anchored at one end.
        let Some(pattern_tok) = pattern_el.as_token() else {
            return;
        };
        let Some((func, new_literal)) = classify(pattern_tok) else {
            return;
        };

        // Namespace-confirm `grepl` is base R; otherwise the rewrite could change
        // which function runs.
        if !ctx.resolves_to_base(&call) {
            return;
        }

        let r = call.syntax().text_range();
        // The fix reconstructs from the subject and the rebuilt pattern only. A
        // comment anywhere else in the call would be dropped, so withhold it there.
        let (subject_range, pattern_range) = (subject_el.text_range(), pattern_el.text_range());
        let drops_comment = call.syntax().descendants_with_tokens().any(|e| {
            e.kind() == SyntaxKind::COMMENT
                && !subject_range.contains_range(e.text_range())
                && !pattern_range.contains_range(e.text_range())
        });

        let fix = (!drops_comment).then(|| {
            Fix::unsafe_(
                usize::from(r.start()),
                usize::from(r.end()),
                format!(
                    "{func}({}, {new_literal})",
                    matchers::element_text(subject_el)
                ),
                format!("Replace `grepl(...)` with `{func}(...)`"),
            )
        });

        sink.push(Diagnostic {
            rule: "string-boundary",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "string-boundary",
                format!("single-anchor `grepl()` is the clearer `{func}()`"),
            )
            .with_suggestion(format!("Use `{func}()`.")),
            fix,
        });
    }
}

/// If `token` is a string literal anchored at exactly one end against a plain
/// literal (`"^abc"` / `"abc$"`), the target function (`startsWith`/`endsWith`)
/// and the anchor-stripped literal, rebuilt with the original quote character.
fn classify(token: &SyntaxToken) -> Option<(&'static str, String)> {
    let (quote, inner) = matchers::string_literal(token)?;
    let (func, rest) = if let Some(rest) = inner.strip_prefix('^') {
        // `^abc$` is a both-ends anchor (an exact match), not a boundary test.
        if rest.ends_with('$') {
            return None;
        }
        ("startsWith", rest)
    } else if let Some(rest) = inner.strip_suffix('$') {
        // Reached only when `inner` does not start with `^`, so this is a lone
        // trailing anchor.
        ("endsWith", rest)
    } else {
        return None;
    };
    if rest.is_empty() || !matchers::is_plain_regex_literal(rest) {
        return None;
    }
    Some((func, format!("{quote}{rest}{quote}")))
}
