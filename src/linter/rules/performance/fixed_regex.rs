//! `fixed-regex`: a base-R regex call whose pattern is a plain literal should
//! pass `fixed = TRUE`.
//!
//! When the pattern contains no regex metacharacter, `grepl("abc", x)` and
//! `grepl("abc", x, fixed = TRUE)` match identically—but the regex form still
//! compiles a (trivial) pattern on every call. `fixed = TRUE` skips that and
//! signals intent ("this is a literal, not a pattern").
//!
//! The rule fires on the base-R regex functions that take `pattern` as their
//! first positional argument (`grepl`, `grep`, `sub`, `gsub`, `regexpr`,
//! `gregexpr`, `regexec`) when that argument is a single non-empty string literal
//! with no regex metacharacter or backslash escape. It is skipped when a
//! `fixed`, `ignore.case`, or `perl` argument is already present, since those
//! interact with (or contradict) `fixed = TRUE`.
//!
//! It is **namespace-confirmed** (`ns`): the callee must resolve to base R.
//!
//! For a plain literal the results are identical (including on `NA` and
//! non-character input, which the regex engine and `fixed = TRUE` handle the same
//! way), so the fix is `Safe`. It is a pure insertion of `, fixed = TRUE` after
//! the last argument—nothing is removed, so it is lossless by construction and
//! carries no comment-withholding case.

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct FixedRegex;

/// Base-R regex functions whose first positional argument is `pattern`.
const REGEX_FUNCTIONS: &[&str] = &[
    "grepl", "grep", "sub", "gsub", "regexpr", "gregexpr", "regexec",
];

const EXAMPLES: &[Example] = &[Example {
    caption: "A literal pattern matched as a regex:",
    source: "grepl(\"abc\", x)\n",
}];

impl Rule for FixedRegex {
    fn id(&self) -> &'static str {
        "fixed-regex"
    }

    fn description(&self) -> &'static str {
        "Flag a base-R regex call (`grepl`, `grep`, `sub`, `gsub`, `regexpr`, \
         `gregexpr`, `regexec`) whose pattern is a plain string literal with no \
         regex metacharacter, and add `fixed = TRUE`—it skips regex compilation \
         and states that the pattern is a literal.\n\nThe rule fires only when the \
         callee resolves to base R and no `fixed`/`ignore.case`/`perl` argument is \
         already present. Because a metacharacter-free pattern matches identically \
         either way, the fix (inserting `, fixed = TRUE`) is safe."
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
        let Some(call) = CallExpr::cast(node.clone()) else {
            return;
        };
        let Some(name) = matchers::callee_name(&call) else {
            return;
        };
        if !REGEX_FUNCTIONS.contains(&name.as_str()) {
            return;
        }

        let args = matchers::args(&call);
        // A `fixed`/`ignore.case`/`perl` argument already governs matching mode;
        // adding `fixed = TRUE` would be redundant or contradictory.
        if args
            .iter()
            .any(|a| matches!(a.name.as_deref(), Some("fixed" | "ignore.case" | "perl")))
        {
            return;
        }

        // The pattern is the first positional argument; it must be a single
        // string literal with no regex metacharacter.
        let Some(pattern) = args.iter().find(|a| a.name.is_none() && a.value.is_some()) else {
            return;
        };
        let Some(tok) = pattern.value.as_ref().and_then(|v| v.as_token()) else {
            return;
        };
        let Some((_, inner)) = matchers::string_literal(tok) else {
            return;
        };
        if inner.is_empty() || !matchers::is_plain_regex_literal(inner) {
            return;
        }

        // Namespace-confirm the callee is base R; a redefinition may not accept
        // `fixed`.
        if !ctx.resolves_to_base(&call) {
            return;
        }

        // Insert `, fixed = TRUE` immediately after the last argument. A pure
        // insertion drops nothing, so it is lossless regardless of the arguments'
        // shape; the argument list is non-empty (we just matched the pattern).
        let Some(arg_list) = call.arg_list() else {
            return;
        };
        let Some(last_end) = arg_list
            .args()
            .last()
            .map(|a| a.syntax().text_range().end())
        else {
            return;
        };
        let at = usize::from(last_end);

        sink.push(Diagnostic {
            rule: "fixed-regex",
            severity: Default::default(),
            path: Default::default(),
            range: tok.text_range(),
            message: ViolationData::new(
                "fixed-regex",
                format!("`{name}()` with a literal pattern should use `fixed = TRUE`"),
            )
            .with_suggestion("Add `fixed = TRUE`."),
            fix: Some(Fix::safe(at, at, ", fixed = TRUE", "Add `fixed = TRUE`")),
        });
    }
}
