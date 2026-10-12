//! `missing-argument`: empty call slots are usually accidental. Trailing and
//! named holes are checked only for established rejecting base calls.
//! Function formals use a different CST shape
//! and are likewise outside this rule. There is no generally safe deletion or
//! replacement, so findings carry no autofix.

use rowan::ast::AstNode as _;

use crate::ast::{Arg, BinaryExpr, CallExpr, HasArgList};
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct MissingArgument;

impl Rule for MissingArgument {
    fn id(&self) -> &'static str {
        "missing-argument"
    }

    fn description(&self) -> &'static str {
        "Flag empty call arguments, including trailing and named holes in resolved base `list()` calls and `c()` calls with only atomic literal values. Unknown or potentially dispatching callees retain the trailing-hole exemption. Function-formal defaults are excluded."
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                caption: "The second call argument is missing:",
                source: "paste(\"a\", , \"b\")\n",
            },
            Example {
                caption: "Base list and literal c calls reject trailing or named holes:",
                source: "list(1,)\nc(value = )\n",
            },
        ]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::ARG, SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) {
            check_rejecting_call(&call, ctx, sink);
            return;
        }
        let Some(arg) = el.as_node().cloned().and_then(Arg::cast) else {
            return;
        };
        if arg.name().is_some() || arg.value().is_some() {
            return;
        }
        // Missing dimensions are the standard spelling for selecting every
        // row or column, so they are intentional rather than omitted calls.
        let in_subset = arg
            .syntax()
            .parent()
            .and_then(|arg_list| arg_list.parent())
            .is_some_and(|parent| {
                matches!(
                    parent.kind(),
                    SyntaxKind::SUBSET_EXPR | SyntaxKind::SUBSET2_EXPR
                )
            });
        if in_subset {
            return;
        }
        let Some(comma) = arg
            .syntax()
            .next_sibling_or_token()
            .and_then(|el| el.into_token())
            .filter(|token| token.kind() == SyntaxKind::COMMA)
        else {
            return;
        };

        sink.push(Diagnostic {
            rule: "missing-argument",
            severity: Default::default(),
            path: Default::default(),
            range: comma.text_range(),
            message: ViolationData::new(
                "missing-argument",
                "call contains an empty argument before this comma",
            )
            .with_suggestion("Supply the intended argument explicitly."),
            fix: None,
        });
    }
}

fn check_rejecting_call(call: &CallExpr, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
    let Some(name) = matchers::callee_name(call) else {
        return;
    };
    if !matches!(name.as_str(), "list" | "c") {
        return;
    }
    let qualified_base = call
        .base()
        .and_then(SyntaxElement::into_node)
        .and_then(BinaryExpr::cast)
        .and_then(|binary| binary.namespace_access())
        .is_some_and(|access| access.package == "base");
    if !qualified_base && !ctx.resolves_to_base(call) {
        return;
    }
    if call
        .syntax()
        .ancestors()
        .skip(1)
        .take_while(|node| node.kind() != SyntaxKind::FUNCTION_EXPR)
        .any(|node| {
            matchers::binary_parts(&node).is_some_and(|(_, op, _)| {
                op.kind() == SyntaxKind::PIPE
                    || (op.kind() == SyntaxKind::USER_OP
                        && matches!(op.text(), "%>%" | "%T>%" | "%<>%" | "%$%"))
            })
        })
    {
        return;
    }
    // R rejects holes in list() and in the default c() path. Literal-only c()
    // calls cannot supply a classed dispatch receiver; arbitrary values can.
    // Forwarded dots can alter the call, so neither shape is classified here.
    let args: Vec<_> = call.args().collect();
    if args.iter().filter_map(Arg::value).any(|value| {
        matchers::element_text(&value) == "..."
            || (name == "c"
                && !matches!(
                    value.kind(),
                    SyntaxKind::INT | SyntaxKind::FLOAT | SyntaxKind::STRING
                ))
    }) {
        return;
    }
    // Non-trailing unnamed holes are already reported by the ARG visitor.
    for arg in &args {
        if arg.value().is_none() && arg.name().is_some() {
            report_hole(arg.syntax().text_range(), sink);
        }
    }
    let Some(list) = call.arg_list() else {
        return;
    };
    // A trailing comma has no value ARG node. Comments may have their own ARG
    // nodes, but they do not fill the missing slot.
    if let Some(comma) = list
        .syntax()
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
            )
        })
        .filter(|el| {
            !el.as_node().is_some_and(|node| {
                Arg::cast(node.clone()).is_some_and(|arg| {
                    arg.name().is_none()
                        && arg.value().is_none()
                        && !arg.syntax().text_range().is_empty()
                })
            })
        })
        .last()
        .filter(|el| el.kind() == SyntaxKind::COMMA)
    {
        report_hole(comma.text_range(), sink);
    }
}

fn report_hole(range: rowan::TextRange, sink: &mut Vec<Diagnostic>) {
    sink.push(Diagnostic {
        rule: "missing-argument",
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new("missing-argument", "base call rejects this empty argument")
            .with_suggestion("Supply the intended argument explicitly."),
        fix: None,
    });
}
