//! `description-malformed-name`: a `Package` value R will not accept as a name.
//!
//! Two defects, one field, because `R CMD check` treats them as one signal
//! (`bad_package`) and the repair is the same either way — rename the package:
//!
//! 1. **Malformed.** R's `valid_package_name` is
//!    `[[:alpha:]][[:alnum:].]*[[:alnum:]]`: a letter, then letters, digits, and
//!    periods, ending in a letter or digit. Note what that excludes and what it
//!    implies — no underscores, no hyphens, no leading period, and, because the
//!    first and last characters are written separately, **at least two
//!    characters**. R checks `^(R|<that>)$`, so the language's own name is
//!    spelled out as an alternative and survives the two-character floor.
//! 2. **The name of a base package.** `Package: stats` describes a package that
//!    can never be installed alongside the one R ships. R exempts a description
//!    that declares `Priority: base`, which is how the real base packages name
//!    themselves, and this rule exempts it for the same reason.
//!
//! **`[[:alpha:]]` is matched as Unicode, not ASCII.** R runs `grepl` in the
//! session's locale, where under UTF-8 the POSIX classes match Unicode letters
//! and digits, so `café` is a name `R CMD check` accepts. Tightening this to
//! ASCII would report a defect R does not have; CRAN's separate opinion about
//! non-ASCII names is not this rule's subject.
//!
//! A `Package` field that is absent or empty is **not** this rule's finding —
//! that is `description-missing-field`, and "malformed" is the wrong word for a
//! field with no value in it.
//!
//! **No autofix.** The package's name is also in its NAMESPACE, its file names,
//! its tests, and every `pkg::` that reaches it. Renaming is the author's, and a
//! textual edit to this one field would only make the description disagree with
//! the package.

use rowan::TextRange;

use crate::dcf;
use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};
use crate::semantic::symbols::base_priority_packages;

pub struct DescriptionMalformedName;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "A name R's `valid_package_name` rejects, since underscores are not \
                  name characters:",
        source: "Package: my_pkg\nVersion: 0.1.0\n",
    },
    Example {
        caption: "A name R already ships:",
        source: "Package: stats\nVersion: 0.1.0\n",
    },
];

impl DcfRule for DescriptionMalformedName {
    fn id(&self) -> &'static str {
        "description-malformed-name"
    }

    fn description(&self) -> &'static str {
        "Flag a `Package` value R will not accept as a package name.\n\nR's \
         `valid_package_name` is `[[:alpha:]][[:alnum:].]*[[:alnum:]]`: a \
         letter, then letters, digits, and periods, ending in a letter or \
         digit. So underscores, hyphens, and a leading period are all out, and \
         a name is at least two characters long—except the literal `R`, which \
         R's check spells out as an alternative.\n\nA `Package` naming one of \
         the packages R itself ships (`stats`, `utils`, `methods`, …) is \
         reported too, since that package could never be installed alongside \
         the one R ships. A description declaring `Priority: base` is exempt, \
         which is how the base packages name themselves.\n\nThe letter and \
         digit classes are matched as Unicode, exactly as R matches them under \
         a UTF-8 locale, so `café` is accepted—a stricter reading would report \
         a defect `R CMD check` does not have.\n\nAn absent or empty `Package` \
         is `description-missing-field`'s finding, not this one's.\n\nThere is \
         no autofix: the name is also in the NAMESPACE, the file names, the \
         tests, and every `pkg::` that reaches the package, so renaming is the \
         author's."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(field) = ctx.document.field("Package") else {
            return;
        };
        let Some((name, range)) = value(&field) else {
            return;
        };

        let (message, suggestion) = if !is_valid_package_name(&name) {
            (
                format!(
                    "`{}` is not a valid package name: R requires a letter, then letters, \
                     digits, and periods, ending in a letter or digit",
                    escape(&name),
                ),
                "Rename the package: at least two characters, starting with a letter, \
                 ending in a letter or digit, and made of letters, digits, and periods.",
            )
        } else if names_a_base_package(&name, ctx) {
            (
                format!("`{name}` is the name of a base R package"),
                "Rename the package to one R does not already ship.",
            )
        } else {
            return;
        };

        sink.push(Diagnostic {
            rule: "description-malformed-name",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new("description-malformed-name", message)
                .with_suggestion(suggestion.to_string()),
            fix: None,
        });
    }
}

/// R's `^(R|[[:alpha:]][[:alnum:].]*[[:alnum:]])$`.
///
/// Splitting the first and last characters off the iterator is what encodes the
/// two-character floor: a one-character name has no `last` and is rejected,
/// which is also why `R` has to be named explicitly.
fn is_valid_package_name(name: &str) -> bool {
    if name == "R" {
        return true;
    }
    let mut middle = name.chars();
    let (Some(first), Some(last)) = (middle.next(), middle.next_back()) else {
        return false;
    };
    first.is_alphabetic()
        && last.is_alphanumeric()
        && middle.all(|c| c.is_alphanumeric() || c == '.')
}

/// Whether `name` is one R ships, and this description is not one of them.
fn names_a_base_package(name: &str, ctx: &DcfRuleContext<'_>) -> bool {
    let declares_base_priority = ctx
        .document
        .field("Priority")
        .is_some_and(|field| field.folded_value().trim() == "base");
    !declares_base_priority && base_priority_packages().contains(&name)
}

/// The field's logical value and the source range spanning it, whitespace
/// excluded on both ends. `None` when the field carries no value at all.
///
/// The fold is `read.dcf`'s rather than [`dcf::Field::folded_value`]'s: an empty
/// value line contributes nothing, so `Package:\n  mypkg` reads as `mypkg` here
/// and in R, instead of arity's leading-`\n` spelling. A value that really does
/// wrap still folds with the `\n` R rejects it for.
fn value(field: &dcf::Field) -> Option<(String, TextRange)> {
    let lines: Vec<dcf::ValueLine> = field
        .value_lines()
        .filter(|line| !line.trimmed_text().is_empty())
        .collect();
    let (first, last) = (lines.first()?, lines.last()?);
    let text = lines
        .iter()
        .map(dcf::ValueLine::trimmed_text)
        .collect::<Vec<_>>()
        .join("\n");
    Some((
        text,
        TextRange::new(trimmed_start(first), trimmed_end(last)),
    ))
}

/// The offset of a value line's content, past the whitespace `VALUE_TEXT` may
/// still carry — the caret belongs on the name, not on the space after the colon.
fn trimmed_start(line: &dcf::ValueLine) -> rowan::TextSize {
    let range = line.content_range();
    match line.content() {
        Some(tok) => {
            let lead = tok.text().len() - tok.text().trim_start().len();
            range.start() + rowan::TextSize::from(lead as u32)
        }
        None => range.start(),
    }
}

/// The offset just past a value line's content. See [`trimmed_start`].
fn trimmed_end(line: &dcf::ValueLine) -> rowan::TextSize {
    let range = line.content_range();
    match line.content() {
        Some(tok) => {
            let trail = tok.text().len() - tok.text().trim_end().len();
            range.end() - rowan::TextSize::from(trail as u32)
        }
        None => range.end(),
    }
}

/// A value spanning continuation lines carries the fold's `\n`, and the report
/// is line-oriented. Shown the way R's regexp reads it: as a character that is
/// not part of any name.
fn escape(text: &str) -> String {
    text.replace('\n', "\\n")
}
