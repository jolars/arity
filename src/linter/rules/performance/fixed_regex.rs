//! Suggest fixed matching for base R calls with a literal pattern.

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, matchers, regex};
use crate::semantic::{checked_match_args_to_formals, decode_string_literal, symbols::unbacktick};
use crate::syntax::{SyntaxElement, SyntaxKind};

pub struct FixedRegex;

fn formals(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "grepl" => &["pattern", "x", "ignore.case", "perl", "fixed", "useBytes"],
        "grep" => &[
            "pattern",
            "x",
            "ignore.case",
            "perl",
            "value",
            "fixed",
            "useBytes",
            "invert",
        ],
        "sub" | "gsub" => &[
            "pattern",
            "replacement",
            "x",
            "ignore.case",
            "perl",
            "fixed",
            "useBytes",
        ],
        "regexpr" | "gregexpr" | "regexec" => &[
            "pattern",
            "text",
            "ignore.case",
            "perl",
            "fixed",
            "useBytes",
        ],
        _ => return None,
    })
}

const EXAMPLES: &[Example] = &[Example {
    caption: "A literal pattern matched as a regex:",
    source: "grepl(\"abc\", x)\n",
}];

impl Rule for FixedRegex {
    fn id(&self) -> &'static str {
        "fixed-regex"
    }

    fn description(&self) -> &'static str {
        "Add `fixed = TRUE` to a base R regex call whose pattern is a plain, nonempty \
         string literal. Fixed matching skips regex compilation.\n\nThe rule skips calls \
         that already supply `fixed`, `ignore.case`, or `perl`, including by position or \
         partial name. For `sub()` and `gsub()`, the replacement must also be a string \
         literal whose value contains no backslash. The insertion is a safe fix."
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
        let Some(name) = call.callee_name() else {
            return;
        };
        let Some(formals) = formals(&name) else {
            return;
        };
        if matchers::in_pipe(&call) {
            return;
        }
        let Some(arg_list) = call.arg_list() else {
            return;
        };

        let mut names = Vec::new();
        let mut values = Vec::new();
        for arg in arg_list.args() {
            let name = arg.name();
            let value = arg.value();
            // Comments can have their own ARG nodes. Empty positional slots
            // instead have zero-width nodes, and named slots have an '='.
            if !arg.syntax().text_range().is_empty()
                && arg.eq_token().is_none()
                && value
                    .as_ref()
                    .is_none_or(|el| el.kind() == SyntaxKind::ROXYGEN_BLOCK)
            {
                continue;
            }
            let Some(value) = value else { return };
            let name = name.map(|name| unbacktick(&name).into());
            if name
                .as_ref()
                .is_some_and(|name: &smol_str::SmolStr| name.contains('\\'))
                || value.as_token().is_some_and(|token| {
                    token.kind() == SyntaxKind::IDENT
                        && (unbacktick(token.text()) == "..." || token.text().contains('\\'))
                })
            {
                return;
            }
            names.push(name);
            values.push(value);
        }

        // A trailing comma supplies a missing argument without an ARG node.
        let commas = arg_list
            .syntax()
            .children_with_tokens()
            .filter(|el| el.kind() == SyntaxKind::COMMA)
            .count();
        if commas + 1 != values.len() {
            return;
        }
        let Ok(matched) = checked_match_args_to_formals(&names, formals) else {
            return;
        };
        if matched
            .iter()
            .any(|name| matches!(name, Some("fixed" | "ignore.case" | "perl")))
            || !matched
                .iter()
                .any(|name| matches!(name, Some("x" | "text")))
        {
            return;
        }

        let argument = |formal| {
            matched
                .iter()
                .position(|name| *name == Some(formal))
                .map(|i| &values[i])
        };
        let Some(tok) = argument("pattern").and_then(SyntaxElement::as_token) else {
            return;
        };
        let Some((_, inner)) = matchers::string_literal(tok) else {
            return;
        };
        if !regex::is_fixed_string(inner) || !ctx.resolves_to_base(&call) {
            return;
        }
        if formals.contains(&"replacement")
            && !argument("replacement")
                .and_then(SyntaxElement::as_token)
                .filter(|token| token.kind() == SyntaxKind::STRING)
                .and_then(|token| decode_string_literal(token.text()))
                .is_some_and(|replacement| !replacement.contains('\\'))
        {
            return;
        }

        let at = usize::from(values.last().unwrap().text_range().end());
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
