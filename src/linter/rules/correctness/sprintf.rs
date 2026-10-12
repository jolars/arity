//! `sprintf`: validate literal base `sprintf()` and `gettextf()` formats. The format
//! scanner models escaped percent signs, numbered fields, and argument-taking
//! `*` widths and precisions. It reports only definite arity errors across all
//! literal formats, because `sprintf()` recycles format vectors and values.

use std::collections::HashSet;
use std::ops::Range;

use rowan::ast::AstNode as _;

use crate::ast::{BinaryExpr, CallExpr};
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
        "Validate literal formats passed to base `sprintf()` and `gettextf()`: flag invalid conversions, definitely missing or excess arguments, and platform-dependent zero padding of strings (`0` plus a width on `%s`). Use explicit string padding or an appropriate numeric conversion instead; neither is a general safe rewrite. A literal-only `sprintf()` call has a safe autofix. Literal-only `gettextf()` calls retain translation."
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                caption: "A literal format with no fields needs no `sprintf()` call:",
                source: "label <- sprintf(\"ready: 100%%\")\n",
            },
            Example {
                caption: "Zero padding of strings depends on the platform:",
                source: "label <- sprintf(f = \"%05s\", \"x\")\n",
            },
            Example {
                caption: "The same portability check applies to translated formats:",
                source: "label <- gettextf(\"%0*s\", 5, \"x\", domain = NA)\n",
            },
        ]
    }

    fn interests(&self) -> &'static [SyntaxKind] {
        &[SyntaxKind::CALL_EXPR]
    }

    fn check(&self, el: &SyntaxElement, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(call) = el.as_node().cloned().and_then(CallExpr::cast) else {
            return;
        };
        let Some(name) = matchers::callee_name(&call) else {
            return;
        };
        if !matches!(name.as_str(), "sprintf" | "gettextf") || !is_base_call(&call, ctx) {
            return;
        }

        let args = matchers::args(&call);
        let Some(fmt_index) = format_arg_index(&args) else {
            return;
        };
        let Some(fmt) = args[fmt_index].value.as_ref() else {
            return;
        };
        let Some(formats) = literal_formats(fmt, ctx) else {
            return;
        };
        let value_count = args
            .iter()
            .enumerate()
            .filter(|(i, arg)| {
                *i != fmt_index
                    && arg.value.is_some()
                    && !(name == "gettextf"
                        && matches!(arg.name.as_deref(), Some("domain" | "trim")))
            })
            .count();

        let mut used = HashSet::new();
        let mut fields = 0;
        let mut encoded_percent = false;
        for token in &formats {
            let Some((_, inner)) = matchers::string_literal(token) else {
                return;
            };
            let decoded = decode_format(inner);
            encoded_percent |= decoded.encoded_percent;
            match scan_format(&decoded.bytes) {
                Ok(analysis) => {
                    fields += analysis.fields;
                    used.extend(analysis.used);
                    for field in analysis.zero_padded_strings {
                        let start = decoded.source_spans[field.start].start;
                        let end = decoded.source_spans[field.end - 1].end;
                        let offset = token.text_range().start() + rowan::TextSize::from(1);
                        push_finding(
                            sink,
                            rowan::TextRange::new(
                                offset + rowan::TextSize::from(start as u32),
                                offset + rowan::TextSize::from(end as u32),
                            ),
                            format!(
                                "`{name}()` string conversion `{}` has platform-dependent zero padding",
                                String::from_utf8_lossy(&decoded.bytes[field])
                            ),
                            "Use explicit string padding or an appropriate numeric conversion.",
                            None,
                        );
                    }
                }
                Err(conversion) => {
                    push_finding(
                        sink,
                        token.text_range(),
                        format!("invalid `{name}()` conversion `{conversion}`"),
                        "Use a conversion supported by base `sprintf()`.",
                        None,
                    );
                    return;
                }
            }
        }

        // Forwarded dots may supply any number of values. The format itself
        // still establishes portability problems, but cannot prove an arity error.
        if args.iter().any(|arg| {
            arg.value
                .as_ref()
                .and_then(SyntaxElement::as_token)
                .is_some_and(|token| token.kind() == SyntaxKind::IDENT && token.text() == "...")
        }) {
            return;
        }

        if let Some(missing) = used.iter().copied().filter(|n| *n > value_count).min() {
            push_finding(
                sink,
                fmt.text_range(),
                format!("`{name}()` format references missing argument {missing}"),
                "Supply every argument referenced by the format.",
                None,
            );
        } else if value_count > used.len() {
            push_finding(
                sink,
                call.syntax().text_range(),
                format!(
                    "`{name}()` has {} excess argument(s)",
                    value_count - used.len()
                ),
                "Remove arguments that the format does not use.",
                None,
            );
        } else if name == "sprintf" && fields == 0 && value_count == 0 && formats.len() == 1 {
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

fn is_base_call(call: &CallExpr, ctx: &RuleContext<'_>) -> bool {
    ctx.resolves_to_base(call)
        || call
            .base()
            .and_then(SyntaxElement::into_node)
            .and_then(BinaryExpr::cast)
            .and_then(|binary| binary.namespace_access())
            .is_some_and(|access| access.package == "base")
}

fn format_arg_index(args: &[matchers::ArgMatch]) -> Option<usize> {
    // Exact matching precedes partial matching. Once `fmt` has an exact match,
    // `f` and `fm` are values in `...`, but multiple matches to `fmt` are errors.
    for names in [&["fmt"][..], &["f", "fm"][..]] {
        let mut matches = args.iter().enumerate().filter(|(_, arg)| {
            arg.name
                .as_deref()
                .is_some_and(|name| names.contains(&name))
        });
        if let Some((index, _)) = matches.next() {
            return matches.next().is_none().then_some(index);
        }
    }
    args.iter().position(|arg| arg.name.is_none())
}

fn literal_formats(value: &SyntaxElement, ctx: &RuleContext<'_>) -> Option<Vec<SyntaxToken>> {
    if let Some(token) = value.as_token() {
        return matchers::string_literal(token).map(|_| vec![token.clone()]);
    }
    let call = value.as_node().cloned().and_then(CallExpr::cast)?;
    if matchers::callee_name(&call).as_deref() != Some("c") || !is_base_call(&call, ctx) {
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
    zero_padded_strings: Vec<Range<usize>>,
}

fn scan_format(bytes: &[u8]) -> Result<FormatAnalysis, String> {
    let mut i = 0;
    let mut next = 1;
    let mut used = HashSet::new();
    let mut fields = 0;
    let mut zero_padded_strings = Vec::new();
    while i < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        let field_start = i;
        i += 1;
        if bytes.get(i) == Some(&b'%') {
            i += 1;
            continue;
        }
        fields += 1;
        let field_arg = numbered(bytes, &mut i);
        let mut zero_pad = false;
        while bytes.get(i).is_some_and(|b| b"-+ 0#".contains(b)) {
            zero_pad |= bytes[i] == b'0';
            i += 1;
        }
        let has_width = bytes.get(i) == Some(&b'*') || bytes.get(i).is_some_and(u8::is_ascii_digit);
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
        if conversion == b's' && zero_pad && has_width {
            zero_padded_strings.push(field_start..i);
        }
        consume(field_arg, &mut next, &mut used);
    }
    Ok(FormatAnalysis {
        used,
        fields,
        zero_padded_strings,
    })
}

struct DecodedFormat {
    bytes: Vec<u8>,
    source_spans: Vec<Range<usize>>,
    encoded_percent: bool,
}

impl DecodedFormat {
    fn push(&mut self, byte: u8, span: Range<usize>) {
        self.bytes.push(byte);
        self.encoded_percent |= byte == b'%' && span.len() > 1;
        self.source_spans.push(span);
    }
}

/// Keep each decoded byte's source span so escaped flags and fields can be
/// diagnosed without pointing into the decoded rather than original string.
fn decode_format(raw: &str) -> DecodedFormat {
    let bytes = raw.as_bytes();
    let mut out = DecodedFormat {
        bytes: Vec::with_capacity(raw.len()),
        source_spans: Vec::with_capacity(raw.len()),
        encoded_percent: false,
    };
    let mut i = 0;
    while i < bytes.len() {
        let escape_start = i;
        if bytes[i] != b'\\' {
            out.push(bytes[i], i..i + 1);
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
                out.push(value, escape_start..i);
            }
        } else if (b'0'..=b'7').contains(&escaped) {
            let start = i;
            while i < bytes.len() && i - start < 3 && (b'0'..=b'7').contains(&bytes[i]) {
                i += 1;
            }
            if let Ok(value) = u8::from_str_radix(&raw[start..i], 8) {
                out.push(value, escape_start..i);
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
            if let Some(value) = value.and_then(char::from_u32) {
                let mut encoded = [0; 4];
                for byte in value.encode_utf8(&mut encoded).bytes() {
                    out.push(byte, escape_start..i);
                }
            }
        } else {
            i += 1;
            let decoded = match escaped {
                b'a' => 7,
                b'b' => 8,
                b'f' => 12,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'v' => 11,
                _ => escaped,
            };
            out.push(decoded, escape_start..i);
        }
    }
    out
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
