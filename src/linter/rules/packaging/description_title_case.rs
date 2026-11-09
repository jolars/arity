//! English package titles follow R's title-case recommendation, including its
//! technical vocabulary and CRAN's exemption for quoted software names.
//!
//! This editorial check is opt-in and report-only: the author decides whether
//! a name or technical term needs a different capitalization. The pinned R
//! oracle exercises the tables and the less intuitive word-boundary behavior.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::packaging::scalar_field::folded;
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionTitleCase;

const RULE: &str = "description-title-case";

// Keep these tables aligned with tools::toTitleCase, rather than choosing a
// second editorial style. tests/oracle/title_case_oracle.R detects R changes.
const LOWERCASE: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "en", "for", "if", "in", "is", "nor",
    "not", "of", "on", "or", "per", "so", "the", "to", "v", "v.", "via", "vs", "vs.", "from",
    "into", "than", "that", "with",
];

const PRESERVE: &[&str] = &[
    "2D", "3D", "AIC", "BayesX", "GoF", "HTML", "LaTeX", "MonetDB", "OpenBUGS", "TeX", "U.S.",
    "U.S.A.", "WinBUGS", "aka", "et", "al.", "ggplot2", "i.e.", "jar", "jars", "ncdf", "netCDF",
    "rgl", "rpart", "xls", "xlsx", "all", "above", "after", "along", "also", "among", "any",
    "both", "can", "few", "it", "less", "log", "many", "may", "more", "over", "some", "their",
    "then", "this", "under", "until", "using", "von", "when", "where", "which", "will", "without",
    "yet", "you", "your",
];

impl DcfRule for DescriptionTitleCase {
    fn id(&self) -> &'static str {
        RULE
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn description(&self) -> &'static str {
        "Check English package titles against \
         [R's `tools::toTitleCase()`](https://search.r-project.org/R/refmans/tools/html/toTitleCase.html), \
         with CRAN's additional exemption for single-quoted software names. \
         R's stopwords, technical terms, mixed-case names, and capitalization \
         after punctuation determine the suggested title.\n\n\
         This rule is **off by default** because technical names and editorial \
         choices often need exceptions. Enable it with `--select description-title-case`. \
         It checks titles when `Language` is absent, `en`, or starts with `en-`. \
         Missing or empty titles and titles equal to the package name are left \
         to other rules, matching CRAN's gating.\n\n\
         There is no autofix: the author must decide which capitalization \
         suits the package and its software names."
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                caption: "Capitalize principal words while preserving stopwords:",
                source: "Package: mypkg\nTitle: Statistical methods for data analysis\n",
            },
            Example {
                caption: "Preserve the spelling of quoted software names:",
                source: "Package: mypkg\nTitle: fitting models with 'mixed-effects software'\n",
            },
        ]
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if let Some(language) = ctx.document.field("Language") {
            let language = language.folded_value();
            if language != "en" && !language.starts_with("en-") {
                return;
            }
        }
        let Some(title) = ctx.document.field("Title").and_then(|field| folded(&field)) else {
            return;
        };
        let text = title.text.replace(['\n', '\t'], " ");
        if ctx
            .document
            .field("Package")
            .is_some_and(|package| package.folded_value().to_lowercase() == text.to_lowercase())
        {
            return;
        }

        let suggested = title_case(&text);
        if text == suggested {
            return;
        }
        sink.push(Diagnostic {
            rule: RULE,
            severity: Default::default(),
            path: Default::default(),
            range: title.range,
            message: ViolationData::new(RULE, "the `Title` field should use English title case")
                .with_suggestion(format!("Consider: {suggested}")),
            fix: None,
        });
    }
}

fn title_case(text: &str) -> String {
    // R's splitter keeps every separator as its own piece. Repeated spaces
    // therefore affect the colon and dash exceptions, and must not collapse.
    let mut pieces = Vec::new();
    let mut start = 0;
    for (offset, ch) in text.char_indices() {
        if " -/\"()\n\t,".contains(ch) {
            if start < offset {
                pieces.push(&text[start..offset]);
            }
            start = offset + ch.len_utf8();
            pieces.push(&text[offset..start]);
        }
    }
    if start < text.len() {
        pieces.push(&text[start..]);
    }

    let mut lowercase: Vec<bool> = pieces
        .iter()
        .map(|piece| LOWERCASE.contains(&lower(piece).as_str()))
        .collect();
    if let Some(first) = lowercase.first_mut() {
        *first = false;
    }
    for (i, piece) in pieces.iter().enumerate() {
        if piece.ends_with(['-', ':'])
            && pieces.get(i + 1) == Some(&" ")
            && let Some(next) = pieces.get(i + 2)
            && next.starts_with(|ch: char| ch == '\'' || ch.is_alphanumeric())
            && !(*piece == "-" && LOWERCASE.contains(next))
        {
            lowercase[i + 2] = false;
        }
        if *piece == "\""
            && let Some(next) = lowercase.get_mut(i + 1)
        {
            *next = false;
        }
    }

    let mut result = String::with_capacity(text.len());
    for (piece, lowercase) in pieces.into_iter().zip(lowercase) {
        let mut chars = piece.chars();
        let first = chars.next().unwrap();
        let have_caps = first.is_alphabetic() && chars.clone().any(char::is_uppercase);
        if lowercase {
            result.push_str(&lower(piece));
        } else if have_caps
            || chars.clone().next().is_none()
            || PRESERVE.contains(&piece)
            || (piece.starts_with('\'') && piece.ends_with('\''))
        {
            result.push_str(piece);
        } else {
            if matches!(first, '\'' | '"') && piece.chars().count() >= 3 {
                result.push(first);
                result.push(upper(chars.next().unwrap()));
            } else {
                result.push(upper(first));
            }
            result.extend(chars.map(lower_char));
        }
    }

    restore_quoted_names(text, &result)
}

// R uses one-character case mappings, so transformations retain character
// positions for CRAN's subsequent restoration of the quoted source slices.
fn upper(ch: char) -> char {
    let mut uppercase = ch.to_uppercase();
    let first = uppercase.next().unwrap();
    if uppercase.next().is_none() {
        first
    } else {
        ch
    }
}

fn lower_char(ch: char) -> char {
    ch.to_lowercase().next().unwrap()
}

fn lower(text: &str) -> String {
    text.chars().map(lower_char).collect()
}

fn restore_quoted_names(text: &str, result: &str) -> String {
    let source: Vec<char> = text.chars().collect();
    let mut result: Vec<char> = result.chars().collect();
    let boundary = |ch: char| ch == ' ' || ch == '\t' || ch.is_ascii_punctuation();
    let mut i = 0;
    while i < source.len() {
        if source[i] == '\''
            && (i == 0 || boundary(source[i - 1]))
            && let Some(end) = (i + 1..source.len()).find(|&j| source[j] == '\'')
            && (end + 1 == source.len() || boundary(source[end + 1]))
        {
            result[i..=end].copy_from_slice(&source[i..=end]);
            i = end + 1;
        } else {
            i += 1;
        }
    }
    result.into_iter().collect()
}
