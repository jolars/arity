//! `description-text-format`: the lexical conventions R and CRAN apply to a
//! package's `Description` field.
//!
//! R requires the field to end in sentence punctuation. CRAN's incoming checks
//! also reject a lowercase initial and descriptions that begin by repeating the
//! package name or generic phrases such as `This package`. The Title,
//! `Functions for`, and quoted function identifiers are the corresponding
//! DESCRIPTION-writing conventions that make the same opening prose hard to
//! scan in package listings.
//!
//! References are deliberately narrow. Bare HTTP(S) URLs, DOI references that
//! begin with `doi:10.`, and recognizable arXiv identifiers belong in angle
//! brackets. A space after their scheme is reported too. Wrapping a recognized
//! bare reference is the only safe fix: capitalization, prose, and quote removal
//! all require the author to choose words.

use std::sync::LazyLock;

use regex::Regex;
use rowan::{TextRange, TextSize};

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::packaging::scalar_field::{Folded, folded};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionTextFormat;

const RULE: &str = "description-text-format";

const EXAMPLES: &[Example] = &[
    Example {
        caption: "Boilerplate prose, a quoted function identifier, and a missing final period:",
        source: "Package: mypkg\nTitle: Model Fitting\n\
                 Description: This package calls 'fit_model()'\n",
    },
    Example {
        caption: "A bare URL, for which arity can safely add angle brackets:",
        source: "Package: mypkg\nTitle: Model Fitting\n\
                 Description: See https://example.com for details.\n",
    },
];

static QUOTED_FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"'([\p{Alphabetic}.][\p{Alphabetic}\p{Number}._]*)\(\)'")
        .expect("quoted-function regex is valid")
});

static BARE_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:https?://[^\s<>]+|doi:10\.[0-9]{4,9}/[^\s<>]+|arxiv:(?:[0-9]{4}\.[0-9]{4,5}(?:v[0-9]+)?|[a-z][a-z.-]*/[0-9]{7}(?:v[0-9]+)?))",
    )
    .expect("bare-reference regex is valid")
});

static SPACED_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:https?:[ \t]+//[^\s<>]+|doi:[ \t]+10\.[0-9]{4,9}/[^\s<>]+|arxiv:[ \t]+(?:[0-9]{4}\.[0-9]{4,5}(?:v[0-9]+)?|[a-z][a-z.-]*/[0-9]{7}(?:v[0-9]+)?))",
    )
    .expect("spaced-reference regex is valid")
});

impl DcfRule for DescriptionTextFormat {
    fn id(&self) -> &'static str {
        RULE
    }

    fn description(&self) -> &'static str {
        "Check the package `Description` field against R's sentence-level and \
         CRAN's lexical conventions. The text must begin with a capital and end \
         in `.`, `!`, or `?` (optionally followed by one quote or closing \
         parenthesis). It must not begin by repeating the package name or Title, \
         or with `This package`, `Functions for`, `The package`, `A package`, \
         `In this package`, or `In the package`.\n\nSingle-quoted function \
         identifiers such as `'case_when()'` are flagged too. Write function \
         names without single quotes.\n\nHTTP(S) URLs, `doi:10.../...` \
         references, and recognizable `arXiv:` identifiers must be enclosed in \
         angle brackets, with no whitespace after the colon. Arity safely wraps \
         an unambiguous bare reference. All prose changes and quote removal are \
         report-only because they require the author's judgment."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(field) = ctx.document.field("Description") else {
            return;
        };
        let Some(description) = folded(&field) else {
            return;
        };

        check_ending(&description, sink);
        check_opening(ctx, &description, sink);
        check_quoted_functions(&description, sink);
        check_references(&description, sink);
    }
}

fn check_ending(description: &Folded, sink: &mut Vec<Diagnostic>) {
    let mut prose = description.text.as_str();
    if prose
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '\'' | '"' | ')'))
    {
        prose = &prose[..prose.len() - 1];
    }
    if prose.ends_with(['.', '!', '?']) {
        return;
    }
    let Some((start, ch)) = prose.char_indices().next_back() else {
        return;
    };
    sink.push(diagnostic(
        description.map(text_range(start, start + ch.len_utf8())),
        "the `Description` field must end with `.`, `!`, or `?`",
        "End the description with sentence punctuation.",
        None,
    ));
}

fn check_opening(ctx: &DcfRuleContext<'_>, description: &Folded, sink: &mut Vec<Diagnostic>) {
    let opening = description.text.replace(['\n', '\t'], " ");
    let quote_len = opening
        .chars()
        .next()
        .filter(|ch| matches!(ch, '\'' | '"'))
        .map_or(0, char::len_utf8);
    let prose = &opening[quote_len..];

    let package = ctx
        .document
        .field("Package")
        .and_then(|field| folded(&field))
        .map(|value| value.text);
    let title = ctx
        .document
        .field("Title")
        .and_then(|field| folded(&field))
        .map(|value| value.text.replace(['\n', '\t'], " "));

    let bad_prefix = package
        .as_deref()
        .filter(|prefix| starts_with_phrase(prose, prefix, false))
        .or_else(|| {
            title
                .as_deref()
                .filter(|prefix| starts_with_phrase(prose, prefix, true))
        })
        .or_else(|| {
            [
                "This package",
                "Functions for",
                "The package",
                "A package",
                "In this package",
                "In the package",
            ]
            .into_iter()
            .find(|prefix| starts_with_phrase(prose, prefix, true))
        });

    if let Some(prefix) = bad_prefix {
        let start = quote_len;
        let end = start + prefix.len();
        sink.push(diagnostic(
            description.map(text_range(start, end)),
            format!("the `Description` field must not start with `{prefix}`"),
            "Start with a concise statement of what the package does.",
            None,
        ));
        return;
    }

    let Some((start, initial)) = prose.char_indices().next() else {
        return;
    };
    if initial.is_uppercase() {
        return;
    }
    let start = quote_len + start;
    sink.push(diagnostic(
        description.map(text_range(start, start + initial.len_utf8())),
        "the `Description` field must start with a capital letter",
        "Capitalize the first word of the description.",
        None,
    ));
}

fn starts_with_phrase(text: &str, prefix: &str, require_boundary: bool) -> bool {
    let Some(candidate) = text.get(..prefix.len()) else {
        return false;
    };
    if !candidate.eq_ignore_ascii_case(prefix) {
        return false;
    }
    !require_boundary
        || text[prefix.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
}

fn check_quoted_functions(description: &Folded, sink: &mut Vec<Diagnostic>) {
    for found in QUOTED_FUNCTION.find_iter(&description.text) {
        let name = &description.text[found.start() + 1..found.end() - 3];
        if name.starts_with('.') && name.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        sink.push(diagnostic(
            description.map(text_range(found.start(), found.end())),
            format!("the function identifier `{name}()` must not be single-quoted"),
            format!("Write it as `{name}()` without quotes."),
            None,
        ));
    }
}

fn check_references(description: &Folded, sink: &mut Vec<Diagnostic>) {
    let angles = angle_ranges(&description.text);

    for &(start, end) in &angles {
        let inner = &description.text[start + 1..end - 1];
        if has_space_after_reference_colon(inner) {
            sink.push(diagnostic(
                description.map(text_range(start, end)),
                "a reference must not contain whitespace after its scheme",
                "Remove the whitespace after the colon.",
                None,
            ));
        }
    }

    for found in SPACED_REFERENCE.find_iter(&description.text) {
        if is_in_angle(found.start(), &angles)
            || !has_reference_boundary(&description.text, found.start())
        {
            continue;
        }
        sink.push(diagnostic(
            description.map(text_range(found.start(), found.end())),
            "a reference must not contain whitespace after its scheme",
            "Remove the whitespace after the colon and enclose the reference in angle brackets.",
            None,
        ));
    }

    for found in BARE_REFERENCE.find_iter(&description.text) {
        if is_in_angle(found.start(), &angles)
            || !has_reference_boundary(&description.text, found.start())
        {
            continue;
        }
        let end = unambiguous_reference_end(&description.text, found.start(), found.end());
        let reference = &description.text[found.start()..end];
        let range = description.map(text_range(found.start(), end));
        let source_start: usize = range.start().into();
        let source_end: usize = range.end().into();
        let fix = (end == found.end()).then(|| {
            Fix::safe(
                source_start,
                source_end,
                format!("<{reference}>"),
                "Enclose the reference in angle brackets",
            )
        });
        sink.push(diagnostic(
            range,
            format!("the reference `{reference}` must be enclosed in angle brackets"),
            format!("Write it as `<{reference}>`."),
            fix,
        ));
    }
}

fn angle_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut opening = None;
    for (offset, ch) in text.char_indices() {
        match ch {
            '<' => opening = Some(offset),
            '>' => {
                if let Some(start) = opening.take() {
                    ranges.push((start, offset + 1));
                }
            }
            '\n' => opening = None,
            _ => {}
        }
    }
    ranges
}

fn has_space_after_reference_colon(text: &str) -> bool {
    SPACED_REFERENCE
        .find(text)
        .is_some_and(|found| found.start() == 0)
}

fn is_in_angle(offset: usize, angles: &[(usize, usize)]) -> bool {
    angles
        .iter()
        .any(|&(start, end)| start < offset && offset < end)
}

fn has_reference_boundary(text: &str, start: usize) -> bool {
    text[..start]
        .chars()
        .next_back()
        .is_none_or(|ch| !ch.is_alphanumeric() && !matches!(ch, '_' | '.'))
}

fn unambiguous_reference_end(text: &str, start: usize, end: usize) -> usize {
    let reference = &text[start..end];
    if reference.chars().next_back().is_some_and(|ch| {
        matches!(
            ch,
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '\'' | '"'
        )
    }) {
        reference
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'', '"'])
            .len()
            + start
    } else {
        end
    }
}

fn diagnostic(
    range: TextRange,
    body: impl Into<String>,
    suggestion: impl Into<String>,
    fix: Option<Fix>,
) -> Diagnostic {
    Diagnostic {
        rule: RULE,
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new(RULE, body).with_suggestion(suggestion),
        fix,
    }
}

fn text_range(start: usize, end: usize) -> TextRange {
    TextRange::new(
        TextSize::try_from(start).expect("folded offset fits in TextSize"),
        TextSize::try_from(end).expect("folded offset fits in TextSize"),
    )
}
