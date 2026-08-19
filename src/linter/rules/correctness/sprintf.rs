//! `sprintf`: statically validate literal base `sprintf()` formats. The format
//! scanner models escaped percent signs, numbered fields, and argument-taking
//! `*` widths and precisions. It reports only definite arity errors across all
//! literal formats, because `sprintf()` recycles format vectors and values.

use std::collections::HashSet;

use rowan::ast::AstNode as _;

use crate::ast::CallExpr;
use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::matchers;
use crate::linter::rules::{Example, Rule, RuleContext};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxToken};

pub struct Sprintf;

impl Rule for Sprintf {
    fn id(&self) -> &'static str {
        "sprintf"
    }

    fn description(&self) -> &'static str {
        "Validate literal formats passed to base `sprintf()`: flag invalid conversions, definitely missing or excess arguments, and a call whose format contains no fields. The literal-only case has a safe autofix."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "A literal format with no fields needs no `sprintf()` call:",
            source: "label <- sprintf(\"ready: 100%%\")\n",
        }]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        if matchers::callee_name(&call).as_deref() != Some("sprintf")
            || !ctx.resolves_to_base(&call)
        {
            return;
        }

        let args = matchers::args(&call);
        let fmt_index = args
            .iter()
            .position(|arg| arg.name.as_deref() == Some("fmt"))
            .or_else(|| args.iter().position(|arg| arg.name.is_none()));
        let Some(fmt_index) = fmt_index else { return };
        let Some(fmt) = args[fmt_index].value.as_ref() else {
            return;
        };
        let Some(formats) = literal_formats(fmt, ctx) else {
            return;
        };
        let value_count = args
            .iter()
            .enumerate()
            .filter(|(i, arg)| *i != fmt_index && arg.value.is_some())
            .count();

        let mut used = HashSet::new();
        let mut fields = 0;
        let mut encoded_percent = false;
        for token in &formats {
            let Some((_, inner)) = matchers::string_literal(token) else {
                return;
            };
            let (decoded, encoded) = decode_format(inner);
            encoded_percent |= encoded;
            match scan_format(&decoded) {
                Ok(analysis) => {
                    fields += analysis.fields;
                    used.extend(analysis.used);
                }
                Err(conversion) => {
                    push_finding(
                        sink,
                        token.text_range(),
                        format!("invalid `sprintf()` conversion `{conversion}`"),
                        "Use a conversion supported by base `sprintf()`.",
                        None,
                    );
                    return;
                }
            }
        }

        if let Some(missing) = used.iter().copied().filter(|n| *n > value_count).min() {
            push_finding(
                sink,
                fmt.text_range(),
                format!("`sprintf()` format references missing argument {missing}"),
                "Supply every argument referenced by the format.",
                None,
            );
        } else if value_count > used.len() {
            push_finding(
                sink,
                call.syntax().text_range(),
                format!(
                    "`sprintf()` has {} excess argument(s)",
                    value_count - used.len()
                ),
                "Remove arguments that the format does not use.",
                None,
            );
        } else if fields == 0 && value_count == 0 && formats.len() == 1 {
            let token = &formats[0];
            let Some((quote, inner)) = matchers::string_literal(token) else {
                return;
            };
            let range = call.syntax().text_range();
            let fix = if encoded_percent {
                None
            } else {
                let replacement = format!("{quote}{}{quote}", inner.replace("%%", "%"));
                Some(Fix::safe(
                    range.start().into(),
                    range.end().into(),
                    replacement,
                    "Replace `sprintf()` with the literal",
                ))
            };
            push_finding(
                sink,
                range,
                "`sprintf()` is pointless because the format has no fields".into(),
                "Use the string literal directly.",
                fix,
            );
        }
    }
}

fn literal_formats(value: &SyntaxElement, ctx: &RuleContext<'_>) -> Option<Vec<SyntaxToken>> {
    if let Some(token) = value.as_token() {
        return matchers::string_literal(token).map(|_| vec![token.clone()]);
    }
    let call = value.as_node().cloned().and_then(CallExpr::cast)?;
    if matchers::callee_name(&call).as_deref() != Some("c") || !ctx.resolves_to_base(&call) {
        return None;
    }
    matchers::args(&call)
        .into_iter()
        .map(|arg| {
            let token = arg.value?.into_token()?;
            matchers::string_literal(&token)?;
            Some(token)
        })
        .collect()
}

struct FormatAnalysis {
    used: HashSet<usize>,
    fields: usize,
}

fn scan_format(bytes: &[u8]) -> Result<FormatAnalysis, String> {
    let mut i = 0;
    let mut next = 1;
    let mut used = HashSet::new();
    let mut fields = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        i += 1;
        if bytes.get(i) == Some(&b'%') {
            i += 1;
            continue;
        }
        fields += 1;
        let field_arg = numbered(bytes, &mut i);
        while bytes.get(i).is_some_and(|b| b"-+ 0#".contains(b)) {
            i += 1;
        }
        if bytes.get(i) == Some(&b'*') {
            i += 1;
            consume(numbered(bytes, &mut i), &mut next, &mut used);
        } else {
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        if bytes.get(i) == Some(&b'.') {
            i += 1;
            if bytes.get(i) == Some(&b'*') {
                i += 1;
                consume(numbered(bytes, &mut i), &mut next, &mut used);
            } else {
                while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
        }
        let Some(&conversion) = bytes.get(i) else {
            return Err("%<end>".into());
        };
        if !b"diouxXfFeEgGaAcs".contains(&conversion) {
            return Err(format!("%{}", char::from(conversion)));
        }
        i += 1;
        consume(field_arg, &mut next, &mut used);
    }
    Ok(FormatAnalysis { used, fields })
}

/// Decode the R escapes that can introduce a `%` byte. Other escapes cannot
/// change the format grammar, so retaining their escaped character is enough
/// for this scanner and avoids growing a second general-purpose R lexer.
fn decode_format(raw: &str) -> (Vec<u8>, bool) {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut encoded_percent = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&escaped) = bytes.get(i) else { break };
        if escaped == b'x' {
            i += 1;
            let start = i;
            while i < bytes.len() && i - start < 2 && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            if let Ok(value) = u8::from_str_radix(&raw[start..i], 16) {
                out.push(value);
                encoded_percent |= value == b'%';
            }
        } else if (b'0'..=b'7').contains(&escaped) {
            let start = i;
            while i < bytes.len() && i - start < 3 && (b'0'..=b'7').contains(&bytes[i]) {
                i += 1;
            }
            if let Ok(value) = u8::from_str_radix(&raw[start..i], 8) {
                out.push(value);
                encoded_percent |= value == b'%';
            }
        } else if matches!(escaped, b'u' | b'U') {
            let digits = if escaped == b'u' { 4 } else { 8 };
            i += 1;
            let braced = bytes.get(i) == Some(&b'{');
            if braced {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && i - start < digits && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            let value = u32::from_str_radix(&raw[start..i], 16).ok();
            if braced && bytes.get(i) == Some(&b'}') {
                i += 1;
            }
            if value == Some(u32::from(b'%')) {
                out.push(b'%');
                encoded_percent = true;
            }
        } else {
            out.push(escaped);
            i += 1;
        }
    }
    (out, encoded_percent)
}

fn numbered(bytes: &[u8], i: &mut usize) -> Option<usize> {
    let start = *i;
    while bytes.get(*i).is_some_and(u8::is_ascii_digit) {
        *i += 1;
    }
    if *i > start && bytes.get(*i) == Some(&b'$') {
        let number = std::str::from_utf8(&bytes[start..*i]).ok()?.parse().ok();
        *i += 1;
        number
    } else {
        *i = start;
        None
    }
}

fn consume(explicit: Option<usize>, next: &mut usize, used: &mut HashSet<usize>) {
    let index = explicit.unwrap_or_else(|| {
        let index = *next;
        *next += 1;
        index
    });
    used.insert(index);
}

fn push_finding(
    sink: &mut Vec<Diagnostic>,
    range: rowan::TextRange,
    body: String,
    suggestion: &str,
    fix: Option<Fix>,
) {
    sink.push(Diagnostic {
        rule: "sprintf",
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new("sprintf", body).with_suggestion(suggestion),
        fix,
    });
}
