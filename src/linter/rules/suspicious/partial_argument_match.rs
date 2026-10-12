//! Warn about successful partial argument matches to statically known closures.
//!
//! Expanding a name protects callers from future API additions. The fix is
//! unsafe because a closure can observe the original names through `sys.call()`.
//! This semantic (`sem`) rule consumes definition sites and project summaries
//! once per file, without walking the CST independently.

use std::collections::HashMap;

use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{CallExpr, HasArgList as _};
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, matchers};
use crate::semantic::calls::{has_dynamic_bindings, stable_closure};
use crate::semantic::symbols::unbacktick;
use crate::semantic::{BindingId, checked_match_args_to_formals};
use crate::syntax::SyntaxKind;

pub struct PartialArgumentMatch;

impl Rule for PartialArgumentMatch {
    fn id(&self) -> &'static str {
        "partial-argument-match"
    }

    fn description(&self) -> &'static str {
        "Flag successful partial argument-name matches to statically resolved \
         local and package-local closures. Use the full formal name so an API \
         addition cannot make the abbreviation ambiguous or redirect it to \
         another argument. Exact matches and names captured by `...` are \
         left alone. Unknown callees, reassignments, conditional definitions, \
         dispatch, forwarded `...`, and pipe calls are skipped. Indexed package \
         functions are not checked yet. The name-only fix is unsafe because \
         functions can inspect the original call's argument names."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "Spell out the formal name instead of relying on its prefix:",
            source: "f <- function(alpha) alpha\nf(al = 1)\n",
        }]
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if has_dynamic_bindings(ctx.model) {
            return;
        }
        let mut signatures: HashMap<BindingId, Option<Vec<SmolStr>>> = HashMap::new();
        for (index, ident) in ctx.model.idents().iter().enumerate() {
            if ident.data_masked {
                continue;
            }
            let Some(call) = ctx
                .root
                .covering_element(ident.range)
                .into_token()
                .and_then(|token| token.parent())
                .and_then(CallExpr::cast)
                .filter(|call| {
                    call.callee_token()
                        .is_some_and(|token| token.text_range() == ident.range)
                })
            else {
                continue;
            };
            if in_pipe(&call) {
                continue;
            }
            let formals = if let Some(id) = ctx.model.resolve_local(ident) {
                if ctx.model.ident_bindings(index) != [id] {
                    continue;
                }
                let binding = ctx.model.binding(id);
                if ctx.model.frame_scope(ident.scope) == ctx.model.frame_scope(binding.scope)
                    && ident.range.start() < binding.def_range.end()
                {
                    continue;
                }
                signatures
                    .entry(id)
                    .or_insert_with(|| {
                        stable_closure(ctx.model, ctx.root, id).map(|function| {
                            function
                                .params()
                                .iter()
                                .map(|param| param.name.clone())
                                .collect()
                        })
                    })
                    .as_deref()
            } else {
                None
            };
            // Keep project facts range-free; only this per-file pass owns ranges.
            let project_formals;
            let formals = if let Some(formals) = formals {
                formals
            } else {
                if ctx.model.resolve_local(ident).is_some() {
                    continue;
                }
                let Some(summary) = ctx
                    .resolution
                    .and_then(|resolution| resolution.local_functions.get(ident.name.as_str()))
                    .filter(|summary| summary.signature_known)
                else {
                    continue;
                };
                project_formals = summary.formals.iter().map(SmolStr::new).collect::<Vec<_>>();
                &project_formals
            };
            check_call(&call, formals, sink);
        }
    }
}

fn in_pipe(call: &CallExpr) -> bool {
    call.syntax()
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
}

fn check_call(call: &CallExpr, formals: &[SmolStr], sink: &mut Vec<Diagnostic>) {
    let args: Vec<_> = call.args().collect();
    if args.iter().any(|arg| {
        arg.value()
            .is_none_or(|value| matchers::element_text(&value) == "...")
    }) {
        return;
    }
    let names: Vec<_> = args
        .iter()
        .map(|arg| arg.name().map(|name| SmolStr::new(unbacktick(&name))))
        .collect();
    let bare_formals: Vec<_> = formals
        .iter()
        .map(|formal| SmolStr::new(unbacktick(formal)))
        .collect();
    // AST names retain escape spelling. Without a decoded name, matching an
    // escaped identifier would guess at the runtime name.
    if names
        .iter()
        .flatten()
        .chain(&bare_formals)
        .any(|name| name.contains('\\'))
    {
        return;
    }
    let Ok(matched) = checked_match_args_to_formals(&names, &bare_formals) else {
        return;
    };
    for ((arg, name), formal) in args.iter().zip(&names).zip(matched) {
        let (Some(name), Some(formal), Some(token)) = (name, formal, arg.name_token()) else {
            continue;
        };
        if name == formal {
            continue;
        }
        let range = token.text_range();
        // Preserve call-site quoting, or reuse the declaration's spelling so
        // keywords and non-syntactic names remain parseable.
        let quote = token
            .text()
            .chars()
            .next()
            .filter(|c| matches!(c, '`' | '\'' | '"'));
        let replacement = if let Some(quote) = quote {
            format!(
                "{quote}{}{quote}",
                formal
                    .replace('\\', "\\\\")
                    .replace(quote, &format!("\\{quote}"))
            )
        } else {
            formals[bare_formals
                .iter()
                .position(|name| name == formal)
                .expect("matched formal")]
            .to_string()
        };
        sink.push(Diagnostic {
            rule: "partial-argument-match",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                "partial-argument-match",
                format!("argument `{name}` partially matches `{formal}`"),
            )
            .with_suggestion(format!("Use the full argument name `{formal}`.")),
            fix: Some(Fix::unsafe_(
                usize::from(range.start()),
                usize::from(range.end()),
                replacement,
                format!("Expand argument name to `{formal}`"),
            )),
        });
    }
}
