//! `class-equals`: comparing `class(x)` against a string is `inherits()`.
//!
//! `class(x)` returns the whole class *vector*, so `class(x) == "cls"`
//! compares elementwise: on a multi-class object it yields a multi-element
//! logical (an error in an `if ()` condition), and it silently misses
//! subclasses. `inherits(x, "cls")` asks the intended question directly,
//! without materializing and scanning the class vector. The membership
//! spellings — `"cls" %in% class(x)` and `class(x) %in% "cls"` — are the
//! same question, and `class(x) != "cls"` is the negation.
//!
//! The rule fires only on the clean shape: a `class(...)` call with a single
//! positional argument compared (`==`, `!=`, `%in%`) against a single string
//! literal, and is **namespace-confirmed** (`ns`): the `class` callee must
//! resolve to base R.
//!
//! **The fix is `Unsafe`.** On a multi-class object the comparison is an
//! elementwise vector while `inherits()` is a scalar membership test, and for
//! S4 objects `inherits()` follows the formal inheritance chain rather than
//! the class attribute — real runtime differences (usually the bug being
//! fixed, but not equivalence). The positive replacement is a call (a primary)
//! replacing a comparison, so it needs no precedence guard; the negated
//! `!inherits(...)` binds looser than the comparison it replaces, so that form
//! is withheld in a context that binds tighter than `!` (see
//! [`matchers::is_safe_splice_context`]). Both forms are withheld when a
//! comment outside the preserved argument and string would be dropped.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct ClassEquals;

const EXAMPLES: &[Example] = &[Example {
    caption: "Testing an object's class:",
    source: "if (class(x) == \"factor\") levels(x)\n",
}];

impl Rule for ClassEquals {
    fn id(&self) -> &'static str {
        "class-equals"
    }

    fn description(&self) -> &'static str {
        "Flag comparisons of `class(x)` against a string literal—`class(x) == \
         \"cls\"`, `class(x) != \"cls\"`, and the `%in%` membership \
         spellings—which are `inherits(x, \"cls\")` (or its negation): \
         `class()` returns the whole class vector, so the comparison is \
         elementwise (an error in an `if ()` condition on a multi-class \
         object) and misses subclasses, while `inherits()` asks the intended \
         question directly without materializing the vector.\n\nThe rule \
         fires only on the clean single-argument shape and only when `class` \
         resolves to base R; a local redefinition is left alone. The fix is \
         **unsafe**: on a multi-class object the comparison yields an \
         elementwise vector where `inherits()` yields a scalar, and for S4 \
         objects `inherits()` follows the formal inheritance chain."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::BINARY_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(node) = el.as_node() else {
            return;
        };
        let Some((lhs, op, rhs)) = matchers::binary_parts(node) else {
            return;
        };
        let negate = match op.kind() {
            SyntaxKind::EQUAL2 => false,
            SyntaxKind::NOT_EQUAL => true,
            SyntaxKind::USER_OP if op.text() == "%in%" => false,
            _ => return,
        };
        // One side is the `class(...)` call, the other a single string literal.
        let (call_el, string_el) = if is_string_literal(&rhs) {
            (&lhs, &rhs)
        } else if is_string_literal(&lhs) {
            (&rhs, &lhs)
        } else {
            return;
        };
        let Some(call) = call_el
            .as_node()
            .and_then(|n| matchers::call_named(n, "class"))
        else {
            return;
        };
        let Some(arg) = matchers::sole_positional(&call) else {
            return;
        };

        // Namespace-confirm `class` is base R; otherwise the comparison does
        // not inspect the class attribute at all.
        if !ctx.resolves_to_base(&call) {
            return;
        }

        let r = node.text_range();
        // The fix preserves only the inner argument and the string literal. A
        // comment anywhere else inside the comparison would be dropped, so
        // withhold the fix there.
        let (arg_range, string_range) = (arg.text_range(), string_el.text_range());
        let drops_comment = node.descendants_with_tokens().any(|e| {
            e.kind() == SyntaxKind::COMMENT
                && !arg_range.contains_range(e.text_range())
                && !string_range.contains_range(e.text_range())
        });
        // `inherits(...)` is a primary and splices anywhere a comparison sat,
        // but the negated `!inherits(...)` binds looser and needs a loose
        // context.
        let context_ok = !negate || matchers::is_safe_splice_context(node);
        let fix = (!drops_comment && context_ok).then(|| {
            let bang = if negate { "!" } else { "" };
            Fix::unsafe_(
                usize::from(r.start()),
                usize::from(r.end()),
                format!(
                    "{bang}inherits({}, {})",
                    matchers::element_text(&arg),
                    matchers::element_text(string_el)
                ),
                format!("Replace the `class()` comparison with `{bang}inherits(...)`"),
            )
        });

        sink.push(Diagnostic {
            rule: "class-equals",
            severity: Default::default(),
            path: Default::default(),
            range: r,
            message: ViolationData::new(
                "class-equals",
                "comparing `class(x)` to a string is fragile—`class()` returns a vector; \
                 `inherits()` asks directly",
            )
            .with_suggestion("Use `inherits(x, \"cls\")` (or `!inherits(x, \"cls\")`)."),
            fix,
        });
    }
}

/// Whether `el` is a single quoted string-literal token.
fn is_string_literal(el: &SyntaxElement) -> bool {
    el.as_token()
        .is_some_and(|t| matchers::string_literal(t).is_some())
}
