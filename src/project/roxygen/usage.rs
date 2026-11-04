//! Static callable surfaces from generated and manually supplied Rd usage.

use rowan::ast::AstNode as _;

use crate::ast::{AstToken as _, BinaryExpr, CallExpr, Ident, RoxygenBlock, token_name};
use crate::semantic::symbols::unbacktick;
use crate::syntax::{SyntaxElement, SyntaxKind};

use super::{documented_binding_name, documented_function, documented_statement, section_value};

/// One recoverable callable signature, without default expressions or ranges.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, salsa::SalsaValue)]
pub struct UsageSignature {
    pub name: String,
    pub formals: Vec<String>,
}

/// Known signatures and whether additional usage could not be recovered.
/// An empty, known surface is distinct from an unsupported signature.
#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, salsa::SalsaValue)]
pub struct TopicUsage {
    pub signatures: Vec<UsageSignature>,
    pub unknown: bool,
}

pub(super) fn topic_usage(block: &RoxygenBlock) -> TopicUsage {
    let sections: Vec<_> = block
        .sections()
        .filter(|section| {
            section
                .tag()
                .is_some_and(|tag| tag.name().as_deref() == Some("usage"))
        })
        .collect();
    if !sections.is_empty() {
        let mut usage = TopicUsage::default();
        for section in sections {
            manual_usage(&section_value(&section), &mut usage);
        }
        usage.signatures.sort();
        usage.signatures.dedup();
        return usage;
    }
    if let Some(function) = documented_function(block) {
        return TopicUsage {
            signatures: vec![UsageSignature {
                name: documented_binding_name(block)
                    .map_or_else(String::new, |name| unbacktick(&name).to_owned()),
                formals: function
                    .params()
                    .into_iter()
                    .map(|param| unbacktick(&param.name).to_owned())
                    .collect(),
            }],
            unknown: false,
        };
    }
    TopicUsage {
        signatures: Vec::new(),
        // Calls such as setMethod and R6Class can generate callable usage
        // that is invisible to this static projection. NULL and data have
        // no callable surface.
        unknown: documented_statement(block).is_some_and(|node| {
            node.descendants()
                .any(|node| node.kind() == SyntaxKind::CALL_EXPR)
        }),
    }
}

fn manual_usage(value: &str, usage: &mut TopicUsage) {
    if value == "NULL" {
        return;
    }
    if value.is_empty() {
        usage.unknown = true;
        return;
    }
    let parsed = crate::rd::parse(&format!("\\usage{{{value}\n}}"));
    if !parsed.diagnostics.is_empty() {
        usage.unknown = true;
        return;
    }
    let Some(argument) = parsed
        .document()
        .macros()
        .next()
        .and_then(|mac| mac.arguments().next())
    else {
        usage.unknown = true;
        return;
    };
    let mut code = String::new();
    for element in argument.syntax().children_with_tokens() {
        if matches!(
            element.kind(),
            crate::rd::SyntaxKind::LBRACE | crate::rd::SyntaxKind::RBRACE
        ) {
            continue;
        }
        usage_code(element, &mut code, &mut usage.unknown);
    }
    let parsed = crate::parser::parse(&code);
    if !parsed.diagnostics.is_empty() {
        usage.unknown = true;
        return;
    }
    for element in parsed.cst.children_with_tokens() {
        if matches!(
            element.kind(),
            SyntaxKind::WHITESPACE
                | SyntaxKind::NEWLINE
                | SyntaxKind::COMMENT
                | SyntaxKind::SEMICOLON
        ) {
            continue;
        }
        match signature(element) {
            Some(signature) => usage.signatures.push(signature),
            None => usage.unknown = true,
        }
    }
}

fn usage_code(element: crate::rd::SyntaxElement, code: &mut String, unknown: &mut bool) {
    use crate::rd::SyntaxKind as K;
    match element {
        crate::rd::SyntaxElement::Node(node) if node.kind() == K::MACRO => {
            // A computed callee keeps surrounding R parseable without
            // claiming to understand or expand an Rd macro.
            code.push_str("(NULL)");
            *unknown = true;
        }
        crate::rd::SyntaxElement::Node(node) => {
            for child in node.children_with_tokens() {
                usage_code(child, code, unknown);
            }
        }
        crate::rd::SyntaxElement::Token(token) => match token.kind() {
            K::COMMENT => {}
            K::ESCAPED_RCODE => code.push_str(&token.text()[1..]),
            K::R_STRING_ESCAPE => {
                *unknown = true;
                code.push_str(token.text());
            }
            _ => code.push_str(token.text()),
        },
    }
}

fn formal(element: SyntaxElement) -> Option<String> {
    let token = element.into_token()?;
    let ident = Ident::cast(token.clone())?;
    (!ident.is_reserved_constant()).then(|| unbacktick(&token_name(&token)).to_owned())
}

fn signature(element: SyntaxElement) -> Option<UsageSignature> {
    let node = element.into_node()?;
    if let Some(call) = CallExpr::cast(node.clone()) {
        let base = call.base()?.into_token()?;
        if !matches!(base.kind(), SyntaxKind::IDENT | SyntaxKind::STRING) {
            return None;
        }
        let formals: Option<Vec<_>> = call
            .arg_list()?
            .args()
            .map(|arg| {
                arg.name()
                    .map(|name| unbacktick(&name).to_owned())
                    .or_else(|| formal(arg.value()?))
            })
            .collect();
        return Some(UsageSignature {
            name: unbacktick(&token_name(&base)).to_owned(),
            formals: formals?,
        });
    }
    let binary = BinaryExpr::cast(node)?;
    let op = binary.op()?;
    if !op.text().starts_with('%') {
        return None;
    }
    Some(UsageSignature {
        name: op.text().to_string(),
        formals: vec![formal(binary.lhs()?)?, formal(binary.rhs()?)?],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(source: &str) -> TopicUsage {
        let block = crate::parser::parse(source)
            .cst
            .descendants()
            .find_map(RoxygenBlock::cast)
            .expect("a roxygen block");
        topic_usage(&block)
    }

    #[test]
    fn manual_calls_keep_names_without_defaults_or_ranges() {
        let source = "#' Manual\n#' @name manual\n#' @usage\n#' `g_name`(\n#'   x, option = list(1, 2), ...\n#' )\n#' other(y)\nNULL\n";
        assert_eq!(
            usage(source),
            TopicUsage {
                signatures: vec![
                    UsageSignature {
                        name: "g_name".into(),
                        formals: vec!["x".into(), "option".into(), "...".into()]
                    },
                    UsageSignature {
                        name: "other".into(),
                        formals: vec!["y".into()]
                    },
                ],
                unknown: false,
            }
        );
    }

    #[test]
    fn rd_escapes_are_decoded_by_the_rd_parser() {
        let got =
            usage("#' Manual\n#' @usage left \\%packages\\% packages\nf <- function() NULL\n");
        assert!(!got.unknown);
        assert_eq!(got.signatures[0].name, "%packages%");
        assert_eq!(got.signatures[0].formals, ["left", "packages"]);
    }

    #[test]
    fn unsupported_macros_do_not_hide_other_signatures() {
        let got = usage("#' Manual\n#' @usage \\method{g}{class}(x)\n#' f(y)\nNULL\n");
        assert!(got.unknown);
        assert_eq!(
            got.signatures,
            [UsageSignature {
                name: "f".into(),
                formals: vec!["y".into()]
            }]
        );
    }

    #[test]
    fn null_and_data_have_no_callable_surface() {
        for source in [
            "#' Manual\n#' @name manual\nNULL\n",
            "#' Manual\n#' @name manual\n#' @usage NULL\nf <- function(x) x\n",
            "#' Data\ndata <- 1\n",
        ] {
            assert_eq!(usage(source), TopicUsage::default(), "{source}");
        }
    }

    #[test]
    fn malformed_or_computed_usages_remain_unknown() {
        for value in ["g(", "(factory())(x)", "g(1)", "", "object$method(x)"] {
            let got = usage(&format!(
                "#' Manual\n#' @name manual\n#' @usage {value}\nNULL\n"
            ));
            assert!(got.unknown, "{value}: {got:?}");
            assert!(got.signatures.is_empty(), "{value}: {got:?}");
        }
    }
}
