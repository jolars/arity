//! `description-malformed-maintainer`: a `Maintainer` value R or CRAN will
//! object to.
//!
//! Four defects, one field, because they are one conversation — who maintains
//! this package, and how the field spells them:
//!
//! 1. **Malformed.** R's `.valid_maintainer_field_regexp` wants
//!    `Name <address>` or the literal `ORPHANED`. A **missing address**
//!    (`Maintainer: Jane Doe`) is the common case and fails it outright;
//!    `R CMD check` reports that as `bad_maintainer`.
//! 2. **More than one person** (CRAN's `Maintainer_invalid_or_multi_person`):
//!    anything after the address, which is what two maintainers look like. R's
//!    own regexp *accepts* those — its `.*` before the `<` happily swallows the
//!    first person — so this clause is not redundant with the first, it is the
//!    one that catches the shape.
//! 3. **No name** (CRAN's `empty_Maintainer_name`): a bare `<jane@example.com>`
//!    with nobody in front of it.
//! 4. **An unquoted comma in the name** (CRAN's `Maintainer_needs_quotes`):
//!    `Doe, Jane <...>` reads as a list of people to everything that parses one.
//!
//! **R's regexp is ported verbatim, and deliberately not tightened.** It is
//! looser than RFC 5322 in ways that matter: a quoted local part is a single
//! `".+"` (so `<"jane doe"@example.com>` is fine), a domain needs no TLD
//! (`<jane@example>`), and a domain label may start with `-`. Every one of those
//! is an address `R CMD check` accepts, so a stricter grammar here would report
//! defects R does not have. The address character classes are spelled out as
//! ASCII in R's regexp, so — unlike the *name* half, which is `.*` and takes any
//! text — they are matched as ASCII here too.
//!
//! **The fold is not a defect.** `read.dcf` joins a continuation line with a
//! `\n`, and R matches the field with a `.` that matches one, so a `Maintainer`
//! wrapped across two lines is one R accepts.
//!
//! A `Maintainer` field that is absent or empty is **not** this rule's finding.
//! R derives one from `Authors@R` when the field is missing, and a derived
//! maintainer is well formed by construction; whether the package names one at
//! all is `description-missing-field`'s subject.
//!
//! **No autofix.** Three of the four defects have nothing to edit *to*: an
//! address cannot be invented, a name cannot be invented, and choosing which of
//! two people maintains the package is not a spelling. The fourth, quoting a
//! comma'd name, is a judgment about whether that comma separates a surname from
//! a given name or separates two maintainers — which is the same question again.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::packaging::scalar_field::{escape, value};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionMalformedMaintainer;

/// R's second alternative: the package has no maintainer, spelled in capitals.
const ORPHANED: &str = "ORPHANED";

/// The local-part character class of R's regexp, less the alphanumerics.
const LOCAL_SPECIALS: &str = "!#$%*/?|^{}`~&'+=_-";

const EXAMPLES: &[Example] = &[
    Example {
        caption: "A maintainer with no address, which is what R's \
                  `.valid_maintainer_field_regexp` mostly catches:",
        source: "Package: mypkg\nVersion: 0.1.0\nMaintainer: Jane Doe\n",
    },
    Example {
        caption: "Two maintainers, where R's `Maintainer` holds exactly one:",
        source: "Package: mypkg\nVersion: 0.1.0\n\
                 Maintainer: Jane Doe <jane@example.com>, John Roe <john@example.org>\n",
    },
    Example {
        caption: "A comma in an unquoted display name, which reads as a list of \
                  people:",
        source: "Package: mypkg\nVersion: 0.1.0\nMaintainer: Doe, Jane <jane@example.com>\n",
    },
];

impl DcfRule for DescriptionMalformedMaintainer {
    fn id(&self) -> &'static str {
        "description-malformed-maintainer"
    }

    fn description(&self) -> &'static str {
        "Flag a `Maintainer` value R or CRAN will object to.\n\nR's \
         `.valid_maintainer_field_regexp` wants exactly one `Name <address>`, or \
         the literal `ORPHANED`. A **missing address** (`Maintainer: Jane Doe`) \
         is the common case and fails it outright.\n\nThree CRAN pretest checks \
         cover the rest of the field and are reported by the same rule, since \
         they are one conversation about who maintains the package: text after \
         the address, which is what **two maintainers** look like (R's own \
         regexp accepts those, so this is the clause that catches them); an \
         address with **no name** in front of it; and a **comma in an unquoted \
         display name**, which reads as a list of people—`\"Doe, Jane\" \
         <jane@example.com>` is the repair.\n\nR's regexp is ported as written \
         and deliberately not tightened to RFC 5322: a quoted local part, a \
         domain with no TLD, and a domain label starting with `-` are all \
         addresses `R CMD check` accepts. A `Maintainer` wrapped across \
         continuation lines is accepted too, exactly as R accepts it.\n\nAn \
         absent or empty `Maintainer` is not this rule's finding: R derives one \
         from `Authors@R`, and whether the package names a maintainer at all is \
         `description-missing-field`'s subject.\n\nThere is no autofix: an \
         address cannot be invented, a name cannot be invented, and whether a \
         comma separates a surname from a given name or separates two people is \
         a question only the author can answer."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(field) = ctx.document.field("Maintainer") else {
            return;
        };
        let Some((maintainer, range)) = value(&field) else {
            return;
        };
        if maintainer == ORPHANED {
            return;
        }
        let display = display_name(&maintainer);

        // First clause wins: a field R rejects outright has no name half worth
        // weighing, and the three CRAN clauses are ordered from the whole field
        // inward.
        let (message, suggestion) = if !is_valid_maintainer_field(&maintainer) {
            if contains_an_address(&maintainer) {
                (
                    format!(
                        "`{}` is not a valid `Maintainer` field: R requires one \
                         `Name <address>`, or `ORPHANED`",
                        escape(&maintainer),
                    ),
                    "Write the field as `Name <name@example.com>`.",
                )
            } else {
                (
                    format!("`{}` has no email address", escape(&maintainer)),
                    "Add the maintainer's address: `Name <name@example.com>`, or \
                     `ORPHANED` if the package has no maintainer.",
                )
            }
        } else if names_more_than_one_person(&maintainer) {
            (
                format!("`{}` names more than one person", escape(&maintainer)),
                "Name one maintainer here and credit everyone else in `Authors@R`: \
                 R's `Maintainer` is the single person to write to.",
            )
        } else if display.is_empty() {
            (
                format!("`{}` gives an address but no name", escape(&maintainer)),
                "Put the maintainer's name in front of the address: \
                 `Name <name@example.com>`.",
            )
        } else if needs_quotes(display) {
            (
                format!(
                    "the maintainer name `{}` contains a comma but is not quoted",
                    escape(display),
                ),
                "Quote the name (`\"Doe, Jane\" <jane@example.com>`), so the comma \
                 does not read as a second maintainer.",
            )
        } else {
            return;
        };

        sink.push(Diagnostic {
            rule: "description-malformed-maintainer",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new("description-malformed-maintainer", message)
                .with_suggestion(suggestion.to_string()),
            fix: None,
        });
    }
}

/// R's `^[[:space:]]*(.*<LOCAL@DOMAIN>|ORPHANED)[[:space:]]*$`, with the value
/// already trimmed by [`value`].
///
/// The `.*` is greedy and neither half of the address can hold a `<`, so the
/// address R matches is the one opened by the value's **last** `<` and closed by
/// its final `>` — which is the whole of what [`bracketed_address`] finds.
fn is_valid_maintainer_field(maintainer: &str) -> bool {
    maintainer == ORPHANED || bracketed_address(maintainer).is_some_and(is_valid_address)
}

/// The text R's regexp would read as the address: between the last `<` and a
/// final `>`. `None` when the value does not end in one.
fn bracketed_address(maintainer: &str) -> Option<&str> {
    let inner = maintainer.strip_suffix('>')?;
    let open = inner.rfind('<')?;
    Some(&inner[open + 1..])
}

/// Whether the value holds a `<...>` at all, which is what separates "this
/// address is wrong" from "there is no address here".
fn contains_an_address(maintainer: &str) -> bool {
    maintainer
        .find('<')
        .is_some_and(|open| maintainer[open + 1..].contains('>'))
}

/// R's `LOCAL@DOMAIN`. The domain half admits no `@`, so the separator is the
/// last one — which is also what lets a quoted local part contain one.
fn is_valid_address(address: &str) -> bool {
    let Some((local, domain)) = address.rsplit_once('@') else {
        return false;
    };
    is_valid_local_part(local) && is_valid_domain(domain)
}

/// R's `(\".+\"|(ATOM+\.)*ATOM+)`. The quoted form is a single `.+` between
/// quotes, so it takes any characters at all — including a `\"` or an `@`.
fn is_valid_local_part(local: &str) -> bool {
    if local.len() > 2 && local.starts_with('"') && local.ends_with('"') {
        return true;
    }
    local
        .split('.')
        .all(|atom| !atom.is_empty() && atom.chars().all(is_local_char))
}

/// R's `([ABC...z0-9!#$%*/?|^{}`~&'+=_-]`, spelled out as ASCII in the regexp
/// and so read as ASCII here — the name half is `.*` and takes any text, but the
/// address is not.
fn is_local_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || LOCAL_SPECIALS.contains(c)
}

/// R's `([A-Za-z0-9-]+\.)*[A-Za-z0-9-]+`: no empty label, no TLD requirement,
/// and a leading `-` in a label is allowed.
fn is_valid_domain(domain: &str) -> bool {
    domain.split('.').all(|label| {
        !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// CRAN's `Maintainer_invalid_or_multi_person`: its regexp reads the **first**
/// `<...>` in the field and demands nothing follow it, so a value whose *last*
/// address is well formed still fails when an earlier one exists. That is the
/// two-maintainer shape, and it is the only one of R's checks that catches it.
///
/// Newlines are spaces to that check (it folds them first), so trailing text on
/// a continuation line counts exactly as it would inline.
fn names_more_than_one_person(maintainer: &str) -> bool {
    let folded = maintainer.replace('\n', " ");
    let Some(open) = folded.find('<') else {
        return true;
    };
    let rest = &folded[open + 1..];
    // `<[^>]+>` needs contents, so an empty `<>` matches nothing at all.
    match rest.find('>') {
        Some(0) | None => true,
        Some(close) => !rest[close + 1..].trim().is_empty(),
    }
}

/// CRAN's `display`: everything before the first `<`, trimmed. `.` matches a
/// newline in R, so the cut runs to the end of the value, continuation lines
/// included.
fn display_name(maintainer: &str) -> &str {
    match maintainer.find('<') {
        Some(open) => maintainer[..open].trim(),
        None => maintainer.trim(),
    }
}

/// CRAN's `grepl("[,]", display) && !grepl("^\".*\"$", display)`.
fn needs_quotes(display: &str) -> bool {
    let quoted = display.len() >= 2 && display.starts_with('"') && display.ends_with('"');
    display.contains(',') && !quoted
}
