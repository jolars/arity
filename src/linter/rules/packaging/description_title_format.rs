//! Package titles have source and editorial constraints beyond ordinary prose.
//! No fix is offered because choosing a replacement title needs author judgment.

use std::sync::LazyLock;

use regex::Regex;
use rowan::{TextRange, TextSize};

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::packaging::scalar_field::folded;
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionTitleFormat;

const RULE: &str = "description-title-format";

// Preserve R's unanchored exception, including occurrences before the ending.
static PERIOD_EXCEPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[[:space:]]\.\.\.|et[[:space:]]al\.")
        .expect("R's title punctuation regex is valid")
});

impl DcfRule for DescriptionTitleFormat {
    fn id(&self) -> &'static str {
        RULE
    }

    fn description(&self) -> &'static str {
        "Check the package `Title` field: it must occupy one physical line, \
         contain at most 65 Unicode characters, and avoid a trailing period \
         unless R's `et al.` or space-prefixed `...` exception applies. R accepts \
         those exceptions anywhere in the title.\n\nThe title must not equal \
         the package name or begin with that name followed by a space or colon \
         (case-insensitively), matching CRAN's redundancy check. The usethis \
         placeholder `What the package does...` must be replaced. These findings \
         are report-only because choosing title wording requires the author's judgment."
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                caption: "A title ending in a period:",
                source: "Package: mypkg\nTitle: Model Fitting.\n",
            },
            Example {
                caption: "A title repeating the package name:",
                source: "Package: mypkg\nTitle: mypkg: Model Fitting\n",
            },
        ]
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(field) = ctx.document.field("Title") else {
            return;
        };
        let Some(title) = folded(&field) else {
            return;
        };
        if let Some(line) = field.value_lines().nth(1) {
            sink.push(diagnostic(
                line.content_range(),
                "the `Title` field must not have continuation lines",
            ));
        }
        if title.text.ends_with('.') && !PERIOD_EXCEPTION.is_match(&title.text) {
            sink.push(diagnostic(
                TextRange::new(title.range.end() - TextSize::from(1), title.range.end()),
                "the `Title` field must not end in a period",
            ));
        }
        if title.text.chars().count() > 65 {
            sink.push(diagnostic(
                title.range,
                "the `Title` field must contain at most 65 Unicode characters",
            ));
        }
        if let Some(package) = ctx
            .document
            .field("Package")
            .and_then(|field| folded(&field))
        {
            let text = title.text.replace(['\n', '\t'], " ").to_lowercase();
            let name = package.text.to_lowercase();
            if text == name
                || text
                    .strip_prefix(&name)
                    .is_some_and(|rest| rest.starts_with([' ', ':']))
            {
                sink.push(diagnostic(
                    title.range,
                    "the `Title` field redundantly repeats the package name",
                ));
            }
        }
        if title.text.eq_ignore_ascii_case("What the package does...") {
            sink.push(diagnostic(
                title.range,
                "replace the usethis placeholder in the `Title` field",
            ));
        }
    }
}

fn diagnostic(range: TextRange, body: &str) -> Diagnostic {
    Diagnostic {
        rule: RULE,
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new(RULE, body),
        fix: None,
    }
}
