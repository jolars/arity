//! Static declarations of custom roxygen2 tag parsers.

use std::collections::BTreeSet;

use rowan::ast::AstNode as _;

use crate::ast::{Arg, BinaryExpr, CallExpr, HasArgList};
use crate::semantic::{checked_match_args_to_formals, decode_string_literal};
use crate::syntax::{SyntaxKind, SyntaxNode};

pub(crate) fn registered_tags(root: &SyntaxNode) -> BTreeSet<String> {
    root.descendants()
        .filter_map(CallExpr::cast)
        .filter_map(registered_tag)
        .collect()
}

fn registered_tag(call: CallExpr) -> Option<String> {
    if call.callee_name()?.as_str() != "s3_register" {
        return None;
    }
    let callee = BinaryExpr::cast(call.base()?.into_node()?)?.namespace_access()?;
    if callee.package != "vctrs" {
        return None;
    }

    let args: Vec<_> = call.args().collect();
    let names: Vec<_> = args.iter().map(Arg::name).collect();
    let matched = checked_match_args_to_formals(&names, &["generic", "class", "method"]).ok()?;
    let literal = |formal| {
        let index = matched.iter().position(|name| *name == Some(formal))?;
        let token = args[index].value()?.into_token()?;
        if token.kind() != SyntaxKind::STRING {
            return None;
        }
        decode_string_literal(token.text())
    };
    if literal("generic")? != "roxygen2::roxy_tag_parse" {
        return None;
    }
    let class = literal("class")?;
    let tag = class.strip_prefix("roxy_tag_")?;
    // Match the roxygen lexer's [A-Za-z][A-Za-z0-9]* tag names.
    if !tag.as_bytes().first()?.is_ascii_alphabetic()
        || !tag.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(tag.to_owned())
}
