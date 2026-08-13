//! `description-malformed-version`: a `Version` value R or CRAN will object to.
//!
//! Three defects, one field, because the repair is the same either way — pick a
//! different version number:
//!
//! 1. **Malformed.** R's `valid_package_version` is
//!    `([[:digit:]]+[.-]){1,}[[:digit:]]+`: runs of digits joined by `.` or `-`.
//!    Because the trailing run is written separately from the repeated group,
//!    that means **at least two components**, so a bare `Version: 1` is
//!    rejected. `R CMD check` reports this as `bad_version`, and R will not
//!    install the package.
//! 2. **A component with a leading zero** (CRAN's
//!    `version_with_leading_zeroes`, `(^|[.-])0[0-9]+`). `1.01` sorts before
//!    `1.1` in text and equal to it as a version, which is how a release goes
//!    out twice. A lone `0` is not a leading zero: `0.1.0` is the most ordinary
//!    version there is.
//! 3. **An implausibly large component** (CRAN's
//!    `version_with_large_components`, threshold 1234). Almost always a typo or
//!    a date pasted into the wrong slot. The **trailing** `.9000` of a
//!    development version is exempt where CRAN's check is not — see
//!    [`absurd_component`].
//!
//! **`[[:digit:]]` is ASCII here, unlike `[[:alpha:]]` in
//! `description-malformed-name`.** The two rules read their POSIX classes
//! differently because R does: verified against `grepl` under a UTF-8 locale,
//! where `café` is a package name R accepts and an Arabic-Indic digit is not a
//! version character.
//!
//! **The year band, and why it is not a clock read.** CRAN exempts a component
//! equal to the *submission year*, so that calendar versioning (`2026.1`)
//! survives. arity has no submission date, and a diagnostic that appears on the
//! first of January is a diagnostic nobody can reproduce, so the whole
//! four-digit year band is exempt instead. That is strictly more permissive than
//! CRAN — the current year is always inside the band, so every component arity
//! flags is one CRAN flags too — which is the direction the oracle's containment
//! gate requires, in any year through 2999.
//!
//! **`Priority: base` exempts the field.** R guards its own clause with
//! `!is_base_package`, since a base package's version is R's to spell,
//! `@VERSION@` placeholder included; CRAN's two clauses never see a base package
//! at all.
//!
//! A `Version` field that is absent or empty is **not** this rule's finding —
//! that is `description-missing-field`, and "malformed" is the wrong word for a
//! field with no value in it.
//!
//! **No autofix.** Which number a release carries is a decision about the
//! release, not a spelling: it is also in the package's tags, its `NEWS.md`, and
//! every constraint a dependent puts on it.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::packaging::scalar_field::{escape, value};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionMalformedVersion;

/// CRAN's threshold for a component that cannot be a release number.
const LARGE_COMPONENT: u64 = 1234;

/// The four-digit components that read as a calendar year, and so are a version
/// scheme rather than a typo. See the module docs for why this is a band and not
/// the current year.
const YEAR_BAND: std::ops::RangeInclusive<u64> = 1900..=2999;

/// Where `usethis::use_dev_version()` starts counting a trailing development
/// component. See [`absurd_component`].
const DEV_COMPONENT: u64 = 9000;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "A version R's `valid_package_version` rejects, since a \
                  component has to be digits:",
        source: "Package: mypkg\nVersion: 1.0.0-beta\n",
    },
    Example {
        caption: "A component with a leading zero, which sorts one way as text \
                  and another as a version:",
        source: "Package: mypkg\nVersion: 1.01\n",
    },
    Example {
        caption: "A component too large to be a release number:",
        source: "Package: mypkg\nVersion: 1.0.5000\n",
    },
];

impl DcfRule for DescriptionMalformedVersion {
    fn id(&self) -> &'static str {
        "description-malformed-version"
    }

    fn description(&self) -> &'static str {
        "Flag a `Version` value R or CRAN will object to.\n\nR's \
         `valid_package_version` is `([[:digit:]]+[.-]){1,}[[:digit:]]+`: runs \
         of digits joined by `.` or `-`. The trailing run is written separately \
         from the repeated group, so a version has **at least two \
         components**—a bare `Version: 1` is one R rejects, and so is any \
         component that is not digits (`1.0.0-beta`, `v1.0`).\n\nTwo CRAN \
         pretest NOTEs are reported by the same rule, since the repair is the \
         same: a component with a **leading zero** (`1.01`, which sorts before \
         `1.1` as text and equal to it as a version), and an **implausibly \
         large** component (1234 or more). Calendar versioning is exempt from \
         both, exactly as CRAN exempts it: `2026.01` keeps its zero, and a \
         four-digit component that reads as a year is not an absurd \
         one.\n\nThe digit class is matched as ASCII, exactly as R matches it \
         under a UTF-8 locale—note that this is the opposite of \
         `description-malformed-name`, whose letter class is Unicode there. A \
         description declaring `Priority: base` is exempt, since a base \
         package's version is R's own to spell.\n\nAn absent or empty `Version` \
         is `description-missing-field`'s finding, not this one's.\n\nThere is \
         no autofix: which number a release carries is a decision about the \
         release, and it is also in the package's tags, its `NEWS.md`, and every \
         constraint a dependent puts on it."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        if declares_base_priority(ctx) {
            return;
        }
        let Some(field) = ctx.document.field("Version") else {
            return;
        };
        let Some((version, range)) = value(&field) else {
            return;
        };

        // First clause wins. A malformed version has no components to weigh,
        // and the two CRAN clauses are one conversation about renumbering.
        let (message, suggestion) = if !is_valid_package_version(&version) {
            (
                format!(
                    "`{}` is not a valid package version: R requires runs of digits \
                     joined by `.` or `-`",
                    escape(&version),
                ),
                "Renumber the release: at least two components, each one digits, \
                 separated by `.` or `-`.",
            )
        } else if has_leading_zero_component(&version) {
            (
                format!("`{version}` has a component with a leading zero"),
                "Drop the leading zero: `1.01` and `1.1` are the same version to R, \
                 but not to anything that sorts the text.",
            )
        } else if let Some(component) = absurd_component(&version) {
            (
                format!("`{version}` has an implausibly large component (`{component}`)"),
                "Check the number: a component of 1234 or more is usually a typo or a \
                 date in the wrong slot.",
            )
        } else {
            return;
        };

        sink.push(Diagnostic {
            rule: "description-malformed-version",
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new("description-malformed-version", message)
                .with_suggestion(suggestion.to_string()),
            fix: None,
        });
    }
}

/// R's `is_base_package`: `Priority` present and equal to `base`.
fn declares_base_priority(ctx: &DcfRuleContext<'_>) -> bool {
    ctx.document
        .field("Priority")
        .is_some_and(|field| field.folded_value().trim() == "base")
}

/// R's `^([[:digit:]]+[.-]){1,}[[:digit:]]+$`, with the digit class read as
/// ASCII the way `grepl` reads it.
///
/// The separator is consumed inside the loop and the final digit run falls out
/// of it, which is what encodes the two-component floor: reaching the end of the
/// string having seen no separator means the repeated group never matched.
fn is_valid_package_version(version: &str) -> bool {
    let mut rest = version;
    let mut separators = 0usize;
    loop {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return false;
        }
        rest = &rest[digits..];
        match rest.as_bytes().first() {
            Some(b'.' | b'-') => {
                separators += 1;
                rest = &rest[1..];
            }
            Some(_) => return false,
            None => return separators >= 1,
        }
    }
}

/// CRAN's `grepl("(^|[.-])0[0-9]+", ver) && !grepl("^[0-9]{4}[.-][0-9]{2}", ver)`.
///
/// Scanning bytes is safe for a value that reached here well-formed, and safe
/// even for one that did not: a UTF-8 continuation byte is never `.`, `-`, or an
/// ASCII digit, so it can neither open a component nor extend one.
fn has_leading_zero_component(version: &str) -> bool {
    if is_calendar_versioned(version) {
        return false;
    }
    let bytes = version.as_bytes();
    bytes.iter().enumerate().any(|(i, &byte)| {
        byte == b'0'
            && (i == 0 || matches!(bytes[i - 1], b'.' | b'-'))
            && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
    })
}

/// CRAN's `^[0-9]{4}[.-][0-9]{2}`: the leading-zero carve-out for a version
/// whose first component is a year.
fn is_calendar_versioned(version: &str) -> bool {
    let bytes = version.as_bytes();
    bytes.len() >= 7
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && matches!(bytes[4], b'.' | b'-')
        && bytes[5..7].iter().all(u8::is_ascii_digit)
}

/// The first component CRAN would call implausibly large, if there is one — less
/// the development-version suffix, which CRAN flags and a linter must not.
///
/// `usethis::use_dev_version()` appends `.9000` and counts up from there, so
/// `0.1.0.9000` is what a package under development looks like nearly
/// everywhere. CRAN's check reports it, and is right to: nobody submits a
/// development version, so the case never reaches the pretest in practice. A
/// linter reads packages in exactly that state, so a **trailing** component of
/// 9000 or more is read as the marker it is. Every other position still counts,
/// which is where the defect the check is for actually lands — a date pasted
/// into the leading slot, or a typo.
///
/// A component too long for a `u64` is as absurd as one can be, so a parse
/// failure counts rather than escaping the check.
fn absurd_component(version: &str) -> Option<&str> {
    let mut components = version.split(['.', '-']).peekable();
    while let Some(component) = components.next() {
        let value: u64 = component.parse().unwrap_or(u64::MAX);
        let is_dev_marker = components.peek().is_none() && value >= DEV_COMPONENT;
        if value >= LARGE_COMPONENT && !YEAR_BAND.contains(&value) && !is_dev_marker {
            return Some(component);
        }
    }
    None
}
