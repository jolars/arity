//! `boolean-arithmetic`: counting `TRUE` values to test whether any exist is
//! less direct than using `any()`.
//!
//! The rule starts with two shapes whose base-vector `NA` behavior is exact:
//! `length(which(p))` is rewritten with `na.rm = TRUE`, because `which()` omits
//! missing values, and `sum(p, na.rm = TRUE)` is considered only when `p` is
//! syntactically known to be logical. Empty and non-empty comparisons against
//! zero or one, including mirrored spellings, are recognized.
//!
//! This is a namespace-confirmed (`ns`) rule: `length`, `which`, `sum`, and any
//! logical-producing call used to justify a `sum()` rewrite must resolve to base
//! R. A fix is withheld if the introduced `any` name is shadowed, if it would
//! discard a comment, or if a negated replacement would bind incorrectly.
//!
//! The fix is `Unsafe`. Although the rewrite preserves ordinary logical-vector
//! and missing-value behavior, `any()` and `sum()` participate in `Summary`
//! method dispatch, while `which()` does not; a class method can therefore make
//! the calls behave differently.

use rowan::ast::AstNode as _;

use crate::ast::{CallExpr, ParenExpr, UnaryExpr};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers::{self, CountTest};
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct BooleanArithmetic;

const EXAMPLES: &[Example] = &[Example {
    caption: "Testing whether any condition holds:",
    source: "if (length(which(ok)) == 0) stop()\n",
}];

impl Rule for BooleanArithmetic {
    fn id(&self) -> &'static str {
        "boolean-arithmetic"
    }

    fn description(&self) -> &'static str {
        "Flag arithmetic existence tests such as `length(which(p)) == 0` and \
         `sum(p, na.rm = TRUE) > 0` when `any(p, na.rm = TRUE)` states the \
         intent more directly and avoids materializing indices or counting \
         matches. Empty tests use `!any(...)`; positive tests use `any(...)`.\n\n\
         The rule recognizes comparisons against zero or one, including mirrored \
         spellings. A `sum()` argument must be syntactically logical and use \
         `na.rm = TRUE`; a bare `sum(p) == 0` is left alone because `sum()` and \
         `any()` differ when `TRUE` and `NA` coexist. Source calls must resolve \
         to base R. The fix is **unsafe** because classed values can change \
         `Summary` method dispatch."
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

        let (count, test) = if let Some(test) = matchers::count_test(op.kind(), &rhs, true) {
            let Some(count) = count_expression(&lhs, ctx) else {
                return;
            };
            (count, test)
        } else if let Some(test) = matchers::count_test(op.kind(), &lhs, false) {
            let Some(count) = count_expression(&rhs, ctx) else {
                return;
            };
            (count, test)
        } else {
            return;
        };

        let range = node.text_range();
        let kept = count.kept_range();
        let drops_comment = node.descendants_with_tokens().any(|child| {
            child.kind() == SyntaxKind::COMMENT && !kept.contains_range(child.text_range())
        });
        let negate = test == CountTest::Empty;
        let context_ok = !negate || matchers::is_safe_splice_context(node);
        let fixable = !drops_comment && context_ok && ctx.introduced_call_resolves_to_base("any");
        let fix = fixable.then(|| {
            let replacement = count
                .replacement(negate)
                .expect("a matched count expression has a callee");
            Fix::unsafe_(
                range.start().into(),
                range.end().into(),
                replacement,
                if negate {
                    "Replace the empty-count test with `!any(...)`"
                } else {
                    "Replace the positive-count test with `any(...)`"
                },
            )
        });

        let direct = if negate { "!any(...)" } else { "any(...)" };
        sink.push(Diagnostic {
            rule: "boolean-arithmetic",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "boolean-arithmetic",
                format!("counting logical values is less direct than `{direct}`"),
            )
            .with_suggestion(format!("Use `{direct}` with `na.rm = TRUE`.")),
            fix,
        });
    }
}

enum CountExpression {
    LengthWhich { predicate: SyntaxElement },
    LogicalSum { call: CallExpr },
}

impl CountExpression {
    fn kept_range(&self) -> rowan::TextRange {
        match self {
            Self::LengthWhich { predicate } => predicate.text_range(),
            Self::LogicalSum { call } => call.syntax().text_range(),
        }
    }

    fn replacement(&self, negate: bool) -> Option<String> {
        let bang = if negate { "!" } else { "" };
        match self {
            Self::LengthWhich { predicate } => Some(format!(
                "{bang}any({}, na.rm = TRUE)",
                matchers::element_text(predicate)
            )),
            Self::LogicalSum { call } => Some(format!("{bang}{}", rename_callee(call, "any")?)),
        }
    }
}

fn count_expression(el: &SyntaxElement, ctx: &RuleContext<'_>) -> Option<CountExpression> {
    let node = el.as_node()?;

    if let Some(length) = matchers::call_named(node, "length") {
        let which_el = matchers::sole_positional(&length)?;
        let which = matchers::call_named(which_el.as_node()?, "which")?;
        let predicate = matchers::sole_positional(&which)?;
        if !ctx.resolves_to_base(&length) || !ctx.resolves_to_base(&which) {
            return None;
        }
        return Some(CountExpression::LengthWhich { predicate });
    }

    let sum = matchers::call_named(node, "sum")?;
    if !ctx.resolves_to_base(&sum) || !is_na_removed_logical_sum(&sum, ctx) {
        return None;
    }
    Some(CountExpression::LogicalSum { call: sum })
}

fn is_na_removed_logical_sum(call: &CallExpr, ctx: &RuleContext<'_>) -> bool {
    let valued: Vec<_> = matchers::args(call)
        .into_iter()
        .filter(|arg| arg.value.is_some())
        .collect();
    if valued.len() != 2 {
        return false;
    }

    let mut predicate = None;
    let mut removes_na = false;
    for arg in valued {
        match arg.name.as_deref() {
            None if predicate.is_none() => predicate = arg.value,
            Some("na.rm") if !removes_na => {
                removes_na = arg.value.as_ref().is_some_and(matchers::is_true);
            }
            _ => return false,
        }
    }

    removes_na && predicate.as_ref().is_some_and(|p| is_logical(p, ctx))
}

fn is_logical(el: &SyntaxElement, ctx: &RuleContext<'_>) -> bool {
    let Some(node) = el.as_node() else {
        return false;
    };
    match node.kind() {
        SyntaxKind::BINARY_EXPR => matchers::binary_parts(node).is_some_and(|(_, op, _)| {
            matches!(
                op.kind(),
                SyntaxKind::EQUAL2
                    | SyntaxKind::NOT_EQUAL
                    | SyntaxKind::LESS_THAN
                    | SyntaxKind::LESS_THAN_OR_EQUAL
                    | SyntaxKind::GREATER_THAN
                    | SyntaxKind::GREATER_THAN_OR_EQUAL
                    | SyntaxKind::AND
                    | SyntaxKind::AND2
                    | SyntaxKind::OR
                    | SyntaxKind::OR2
            )
        }),
        SyntaxKind::UNARY_EXPR => UnaryExpr::cast(node.clone())
            .is_some_and(|expr| expr.op_kind() == Some(SyntaxKind::BANG)),
        SyntaxKind::PAREN_EXPR => ParenExpr::cast(node.clone())
            .and_then(|expr| expr.inner())
            .is_some_and(|inner| is_logical(&inner, ctx)),
        SyntaxKind::CALL_EXPR => CallExpr::cast(node.clone()).is_some_and(|call| {
            matches!(
                matchers::callee_name(&call).as_deref(),
                Some(
                    "grepl"
                        | "nzchar"
                        | "startsWith"
                        | "endsWith"
                        | "xor"
                        | "is.element"
                        | "duplicated"
                        | "is.na"
                        | "is.nan"
                        | "is.finite"
                        | "is.infinite"
                        | "isTRUE"
                        | "isFALSE"
                        | "anyNA"
                )
            ) && ctx.resolves_to_base(&call)
        }),
        _ => false,
    }
}

fn rename_callee(call: &CallExpr, replacement: &str) -> Option<String> {
    let callee = call.callee_token()?;
    let call_range = call.syntax().text_range();
    let text = call.syntax().text().to_string();
    let start = usize::from(callee.text_range().start() - call_range.start());
    let end = usize::from(callee.text_range().end() - call_range.start());
    Some(format!("{}{replacement}{}", &text[..start], &text[end..]))
}
