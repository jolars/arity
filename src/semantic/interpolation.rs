//! Static discovery of names read through string-based runtime syntax.

use rowan::TextRange;
use rowan::ast::AstNode as _;
use smol_str::SmolStr;

use crate::ast::{Arg, BinaryExpr, CallExpr, HasArgList as _};
use crate::semantic::SemanticModel;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxToken};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TemplateKind {
    Glue,
    SafeGlue,
    SqlGlue,
    Cli,
    StyledGlue,
}

pub(super) struct Candidate {
    pub name: SmolStr,
    pub range: TextRange,
    pub gate: Option<(SmolStr, TextRange)>,
}

/// Return use-only names with the containing literal's range.
pub(super) fn use_only_reads(call: &CallExpr) -> Vec<Candidate> {
    let Some(name) = call.callee_name() else {
        return Vec::new();
    };
    let package = call_package(call);

    if is_get(&name, package.as_deref()) {
        return literal_get(call)
            .into_iter()
            .map(|(read, range)| candidate(call, read, range))
            .collect();
    }

    let Some(kind) = template_kind(&name, package.as_deref()) else {
        return Vec::new();
    };
    if has_named_arg(call, ".envir") || has_named_arg(call, ".transformer") {
        return Vec::new();
    }

    let (open, close) = match static_delimiters(call) {
        Some(pair) => pair,
        None => return Vec::new(),
    };
    let temporary: Vec<SmolStr> = if matches!(
        kind,
        TemplateKind::Glue
            | TemplateKind::SafeGlue
            | TemplateKind::SqlGlue
            | TemplateKind::StyledGlue
    ) {
        call.args().filter_map(|arg| arg.name()).collect()
    } else {
        Vec::new()
    };

    let mut out = Vec::new();
    for (text, range) in template_literals(call, kind, &name) {
        let Some(text) = decode_string(&text) else {
            continue;
        };
        let literal = static_logical_arg(call, ".literal").unwrap_or(false);
        for expression in interpolation_bodies(&text, &open, &close, kind, literal) {
            for name in names_in_body(expression, &open, &close, kind, literal) {
                if !temporary.contains(&name) {
                    out.push(candidate(call, name, range));
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.range.start().cmp(&b.range.start()))
    });
    out.dedup_by(|a, b| a.name == b.name && a.range == b.range);
    out
}

fn candidate(call: &CallExpr, name: SmolStr, range: TextRange) -> Candidate {
    let gate = call_package(call).is_none().then(|| {
        let token = call
            .callee_token()
            .expect("a classified call has a callee token");
        (SmolStr::new(token.text()), token.text_range())
    });
    Candidate { name, range, gate }
}

fn call_package(call: &CallExpr) -> Option<SmolStr> {
    let SyntaxElement::Node(base) = call.base()? else {
        return None;
    };
    BinaryExpr::cast(base)?
        .namespace_access()
        .map(|ns| ns.package)
}

fn is_get(name: &str, package: Option<&str>) -> bool {
    name == "get" && package.is_none_or(|pkg| pkg == "base")
}

fn literal_get(call: &CallExpr) -> Option<(SmolStr, TextRange)> {
    if call
        .args()
        .any(|arg| matches!(arg.name().as_deref(), Some("pos" | "envir")))
    {
        return None;
    }
    let args: Vec<_> = call.args().collect();
    let arg = args
        .iter()
        .find(|arg| arg.name().as_deref() == Some("x"))
        .or_else(|| args.iter().find(|arg| arg.name().is_none()))?;
    let token = string_token(arg)?;
    Some((
        SmolStr::new(decode_string(token.text())?),
        token.text_range(),
    ))
}

fn template_kind(name: &str, package: Option<&str>) -> Option<TemplateKind> {
    const GLUE: &[&str] = &["glue", "glue_data"];
    const SAFE_GLUE: &[&str] = &["glue_safe", "glue_data_safe"];
    const SQL_GLUE: &[&str] = &["glue_sql", "glue_data_sql"];
    const STYLED_GLUE: &[&str] = &["glue_col", "glue_data_col"];
    const CLI: &[&str] = &[
        "format_inline",
        "format_error",
        "format_warning",
        "format_message",
        "cli_abort",
        "cli_warn",
        "cli_inform",
        "cli_text",
        "cli_h1",
        "cli_h2",
        "cli_h3",
        "cli_ul",
        "cli_ol",
        "cli_dl",
        "cli_li",
        "cli_alert",
        "cli_alert_success",
        "cli_alert_danger",
        "cli_alert_warning",
        "cli_alert_info",
        "cli_rule",
        "cli_blockquote",
        "cli_bullets",
        "cli_status",
        "cli_status_update",
        "cli_status_clear",
        "cli_process_start",
        "cli_process_done",
        "cli_process_failed",
        "cli_progress_bar",
        "cli_progress_along",
        "cli_progress_output",
        "cli_progress_message",
        "cli_progress_step",
    ];
    match package {
        Some("glue") if GLUE.contains(&name) => Some(TemplateKind::Glue),
        Some("glue") if SAFE_GLUE.contains(&name) => Some(TemplateKind::SafeGlue),
        Some("glue") if SQL_GLUE.contains(&name) => Some(TemplateKind::SqlGlue),
        Some("glue") if STYLED_GLUE.contains(&name) => Some(TemplateKind::StyledGlue),
        Some("cli") if CLI.contains(&name) => Some(TemplateKind::Cli),
        Some(_) => None,
        None if GLUE.contains(&name) => Some(TemplateKind::Glue),
        None if SAFE_GLUE.contains(&name) => Some(TemplateKind::SafeGlue),
        None if SQL_GLUE.contains(&name) => Some(TemplateKind::SqlGlue),
        None if STYLED_GLUE.contains(&name) => Some(TemplateKind::StyledGlue),
        None if CLI.contains(&name) => Some(TemplateKind::Cli),
        None => None,
    }
}

fn has_named_arg(call: &CallExpr, name: &str) -> bool {
    call.args().any(|arg| arg.name().as_deref() == Some(name))
}

fn static_logical_arg(call: &CallExpr, name: &str) -> Option<bool> {
    let arg = call
        .args()
        .find(|arg| arg.name().as_deref() == Some(name))?;
    let token = arg.value()?.into_token()?;
    match token.text() {
        "TRUE" => Some(true),
        "FALSE" => Some(false),
        _ => None,
    }
}

fn static_delimiters(call: &CallExpr) -> Option<(String, String)> {
    let mut open = "{".to_string();
    let mut close = "}".to_string();
    for arg in call.args() {
        match arg.name().as_deref() {
            Some(".open") => open = decode_string(string_token(&arg)?.text())?,
            Some(".close") => close = decode_string(string_token(&arg)?.text())?,
            _ => {}
        }
    }
    (!open.is_empty() && !close.is_empty()).then_some((open, close))
}

fn template_literals(
    call: &CallExpr,
    kind: TemplateKind,
    callee: &str,
) -> Vec<(String, TextRange)> {
    let mut out = Vec::new();
    for (index, arg) in call.args().enumerate() {
        let is_template = match kind {
            TemplateKind::Glue
            | TemplateKind::SafeGlue
            | TemplateKind::SqlGlue
            | TemplateKind::StyledGlue => {
                arg.name().is_none() && !(index == 0 && callee.starts_with("glue_data"))
            }
            TemplateKind::Cli => {
                arg.name().is_none() && cli_positional_arg_is_template(callee, index)
                    || cli_template_arg(arg.name().as_deref())
            }
        };
        if is_template {
            collect_string_tokens(arg.value(), &mut out);
        }
    }
    out
}

fn cli_positional_arg_is_template(callee: &str, index: usize) -> bool {
    match callee {
        "cli_status_update" | "cli_process_done" => index == 1,
        "cli_process_failed" => matches!(index, 1 | 2),
        "cli_progress_along" => index == 1,
        "cli_status_clear" => false,
        _ => index == 0,
    }
}

fn cli_template_arg(name: Option<&str>) -> bool {
    matches!(
        name,
        Some(
            "message"
                | "text"
                | "msg"
                | "left"
                | "center"
                | "right"
                | "quote"
                | "citation"
                | "msg_done"
                | "msg_failed"
                | "format"
                | "format_done"
                | "format_failed"
                | "name"
        )
    )
}

fn collect_string_tokens(value: Option<SyntaxElement>, out: &mut Vec<(String, TextRange)>) {
    let Some(value) = value else { return };
    match value {
        SyntaxElement::Token(token) if token.kind() == SyntaxKind::STRING => {
            out.push((token.text().to_string(), token.text_range()));
        }
        SyntaxElement::Node(node) => {
            let Some(call) = CallExpr::cast(node) else {
                return;
            };
            if call.callee_name().as_deref() != Some("c") || call_package(&call).is_some() {
                return;
            }
            for arg in call.args() {
                collect_string_tokens(arg.value(), out);
            }
        }
        _ => {}
    }
}

fn string_token(arg: &Arg) -> Option<SyntaxToken> {
    let token = arg.value()?.into_token()?;
    (token.kind() == SyntaxKind::STRING).then_some(token)
}

fn free_names(expression: &str) -> Option<Vec<SmolStr>> {
    let parsed = crate::parser::parse(expression);
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let model = SemanticModel::build(&parsed.cst);
    let mut names: Vec<SmolStr> = model
        .idents()
        .iter()
        .filter(|ident| !ident.data_masked && model.resolve_local(ident).is_none())
        .map(|ident| ident.name.clone())
        .chain(model.free_use_only_reads().iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    Some(names)
}

fn names_in_body(
    body: &str,
    open: &str,
    close: &str,
    kind: TemplateKind,
    literal: bool,
) -> Vec<SmolStr> {
    match kind {
        TemplateKind::SafeGlue => return vec![SmolStr::new(body)],
        TemplateKind::SqlGlue => {
            return free_names(body.trim_end().strip_suffix('*').unwrap_or(body))
                .unwrap_or_default();
        }
        TemplateKind::Glue | TemplateKind::Cli => return free_names(body).unwrap_or_default(),
        TemplateKind::StyledGlue => {}
    }
    if let Some(names) = free_names(body) {
        return names;
    }
    interpolation_bodies(body, open, close, kind, literal)
        .into_iter()
        .flat_map(|inner| names_in_body(inner, open, close, kind, literal))
        .collect()
}

fn interpolation_bodies<'a>(
    text: &'a str,
    open: &str,
    close: &str,
    kind: TemplateKind,
    literal: bool,
) -> Vec<&'a str> {
    let mut bodies = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find(open) {
        let start = cursor + rel;
        let content = start + open.len();
        if text[content..].starts_with(open) {
            cursor = content + open.len();
            continue;
        }
        let Some(end) = matching_close(text, content, open, close, literal) else {
            break;
        };
        let body = &text[content..end];
        let trimmed = body.trim_start();
        if kind == TemplateKind::Cli
            && matches!(trimmed.as_bytes().first(), Some(b'.' | b'?' | b'#'))
        {
            bodies.extend(interpolation_bodies(body, open, close, kind, literal));
        } else {
            bodies.push(body);
        }
        cursor = end + close.len();
    }
    bodies
}

fn matching_close(
    text: &str,
    mut cursor: usize,
    open: &str,
    close: &str,
    literal: bool,
) -> Option<usize> {
    let mut depth = 1usize;
    let mut quote = None;
    let bytes = text.as_bytes();
    while cursor < text.len() {
        let byte = bytes[cursor];
        if let Some(delim) = quote {
            if byte == b'\\' {
                cursor += 2;
                continue;
            }
            if byte == delim {
                quote = None;
            }
            cursor += 1;
            continue;
        }
        if !literal && matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            cursor += 1;
            continue;
        }
        if !literal && byte == b'#' {
            cursor = text[cursor..]
                .find('\n')
                .map_or(text.len(), |n| cursor + n + 1);
            continue;
        }
        if text[cursor..].starts_with(open) {
            depth += 1;
            cursor += open.len();
            continue;
        }
        if text[cursor..].starts_with(close) {
            depth -= 1;
            if depth == 0 {
                return Some(cursor);
            }
            cursor += close.len();
            continue;
        }
        cursor += text[cursor..].chars().next()?.len_utf8();
    }
    None
}

fn decode_string(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.len() >= 5 && matches!(bytes[0], b'r' | b'R') && matches!(bytes[1], b'\'' | b'"') {
        let mut open = 2;
        while bytes.get(open) == Some(&b'-') {
            open += 1;
        }
        let close = match *bytes.get(open)? {
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            _ => return None,
        };
        let dash_count = open - 2;
        let suffix_len = 1 + dash_count + 1;
        let end = text.len().checked_sub(suffix_len)?;
        if bytes.get(end) != Some(&close)
            || bytes.get(end + 1..end + 1 + dash_count)? != vec![b'-'; dash_count]
            || bytes.last() != Some(&bytes[1])
        {
            return None;
        }
        return Some(text[open + 1..end].to_string());
    }

    let quote = *bytes.first()?;
    if !matches!(quote, b'\'' | b'"') || bytes.last() != Some(&quote) {
        return None;
    }
    let mut out = String::new();
    let mut chars = text[1..text.len() - 1].chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let escaped = chars.next()?;
        let decoded = match escaped {
            'a' => '\x07',
            'b' => '\x08',
            'f' => '\x0c',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\x0b',
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            '\n' => continue,
            '\r' => {
                if chars.clone().next() == Some('\n') {
                    chars.next();
                }
                continue;
            }
            '0'..='7' => {
                let mut value = escaped.to_digit(8)?;
                for _ in 0..2 {
                    let Some(next) = chars.clone().next().and_then(|ch| ch.to_digit(8)) else {
                        break;
                    };
                    chars.next();
                    value = value * 8 + next;
                }
                char::from_u32(value)?
            }
            'x' => char::from_u32(read_digits(&mut chars, 16, 1, 2)?)?,
            'u' => char::from_u32(read_digits(&mut chars, 16, 4, 4)?)?,
            'U' => char::from_u32(read_digits(&mut chars, 16, 8, 8)?)?,
            _ => return None,
        };
        out.push(decoded);
    }
    Some(out)
}

fn read_digits(
    chars: &mut std::str::Chars<'_>,
    radix: u32,
    minimum: usize,
    maximum: usize,
) -> Option<u32> {
    let mut value = 0;
    let mut count = 0;
    while count < maximum {
        let Some(digit) = chars.clone().next().and_then(|ch| ch.to_digit(radix)) else {
            break;
        };
        chars.next();
        value = value * radix + digit;
        count += 1;
    }
    (count >= minimum).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_raw_string_delimiters() {
        assert_eq!(decode_string(r#"r"--[{x}]--""#).as_deref(), Some("{x}"));
    }

    #[test]
    fn scans_nested_r_braces() {
        assert_eq!(
            interpolation_bodies("{if (x) { y } else z}", "{", "}", TemplateKind::Glue, false,),
            ["if (x) { y } else z"]
        );
    }

    #[test]
    fn decodes_r_string_escapes_used_by_templates() {
        assert_eq!(
            decode_string(r#""\x7bna\u006de\175""#).as_deref(),
            Some("{name}")
        );
    }
}
