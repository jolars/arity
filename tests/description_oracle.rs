//! Differential oracle: arity's `DESCRIPTION` rules against `R CMD check`'s own.
//!
//! `Writing R Extensions` paraphrases the checks and is looser than they are, so
//! the Packaging rules are pinned against the code that actually decides —
//! `tools:::.check_package_description(strict = TRUE)`, the `Authors@R` checker
//! at its strict tier, the `duplicates` half of `.check_package_description2`,
//! and the version and `Maintainer` components of
//! `.check_package_CRAN_incoming`. The last
//! two are cherry-picked rather than taken whole, because most of what those
//! checkers report is about files, URLs, network state, and installed packages,
//! which a text-only oracle has no business simulating. The driver is
//! `tests/oracle/description_oracle.R`; its report is a *set* of
//! `(signal, detail)` pairs.
//!
//! The harness is deliberately two-sided, because arity implements a fraction of
//! what R checks:
//!
//! - **Gated** ([`GATES`]): for a rule arity ships, every finding it reports must
//!   be backed by one of R's signals on the same case. That is the
//!   false-positive direction, and it is a hard failure. The reverse direction is
//!   *not* gated: `description-version-constraint` deliberately says nothing
//!   about a malformed package *name*, and demanding parity would be demanding a
//!   rule that does not exist yet.
//! - **Planned** ([`PLANNED`]): a signal no rule covers yet. Counted and listed,
//!   never failed. That report is the work-list for the DESCRIPTION items in
//!   `TODO.md`, ordered by how often each signal actually fires.
//!
//! Two structural failures keep the oracle honest, and they are the reason it is
//! worth having before the rules land:
//!
//! 1. **An unknown signal fails.** R's checkers are data that changes with R
//!    itself; a new check must be classified deliberately, not absorbed
//!    silently.
//! 2. **A gated signal that no case exercises fails.** An oracle that tests
//!    nothing passes quietly forever, which is the failure mode that makes
//!    oracles worthless.
//!
//! `#[ignore]`d because it needs R: run via `task description-oracle`. A missing
//! `Rscript` is a skip, never a failure, exactly as in `dcf_oracle` and
//! `deps_oracle`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use arity::config::LintConfig;
use arity::linter::check_description_document;

/// A rule arity ships, and the signals that justify its findings.
///
/// The check is containment, not equality: arity must not report what R
/// considers fine. Extend this as the rules in `TODO.md` land.
struct Gate {
    rule: &'static str,
    signals: &'static [&'static str],
    backing: Backing,
}

/// What it takes for one of R's signals to back one of arity's findings.
enum Backing {
    /// R's detail names the offender, so findings compare entry by entry: the
    /// text arity spans must be one of the entries R rejected. The stronger
    /// check, and the default.
    Entry,
    /// R's detail is prose rather than an offender — `bad_package` reports
    /// "Malformed package name", not the name — so what backs a finding is the
    /// signal being raised for this file at all. That is the whole claim a rule
    /// keyed on a single scalar field makes, since there is only one `Package`
    /// to be wrong about.
    Signal,
}

/// R buckets a bad dependency entry three ways, and
/// `description-version-constraint` cuts across all three: `dplyr (1.0.0)` is a
/// `bad_dep_entry` (the parenthesized part is not `op version` at all),
/// `dplyr (=> 1.0)` a `bad_dep_op`, and `dplyr (>= foo)` a `bad_dep_version`.
/// The rule's subject is the union, so the union is what backs it.
const GATES: &[Gate] = &[
    Gate {
        rule: "description-version-constraint",
        signals: &["bad_dep_entry", "bad_dep_op", "bad_dep_version"],
        backing: Backing::Entry,
    },
    // R reports the bare package name, and the rule spans the bare package
    // name, so the two are directly comparable. Note R's own exclusions the
    // rule has to mirror for containment to hold: `LinkingTo` is not one of
    // the compared fields, `R` is dropped from `Depends`, and each field is
    // uniqued before the comparison, so a within-field repeat is not a
    // duplicate.
    Gate {
        rule: "description-package-in-multiple-fields",
        signals: &["duplicates"],
        backing: Backing::Entry,
    },
    // R folds "malformed" and "this is a base package's name" into one signal
    // whose detail is its own message, so this one is backed by presence. Note
    // the exclusions the rule mirrors for containment to hold: `Package: R` is
    // spelled out in R's regexp, `[[:alpha:]]` is Unicode under a UTF-8 locale,
    // and `Priority: base` exempts a base package from naming itself.
    Gate {
        rule: "description-malformed-name",
        signals: &["bad_package"],
        backing: Backing::Signal,
    },
    // The rule's subject spans two checkers: `R CMD check` rejects a version
    // that is not `digits[.-]digits...`, and CRAN's pretest adds the two NOTEs
    // about a component's *value*. All three details are the version string
    // itself, which is what the rule spans, so they compare entry to entry.
    //
    // Containment holds in the direction that matters even though arity is
    // deliberately looser than CRAN in two places, each of which only ever
    // *withholds* a finding: the year band stands in for CRAN's "equal to the
    // submission year" (the current year is always inside it), and a trailing
    // `.9000` is read as the development-version marker CRAN's check has no
    // reason to expect.
    Gate {
        rule: "description-malformed-version",
        signals: &[
            "bad_version",
            "version_with_leading_zeroes",
            "version_with_large_components",
        ],
        backing: Backing::Entry,
    },
    // Four signals, two checkers, one field: R's regexp looks only at the
    // address half, and CRAN's three NOTEs cover the name half and the
    // "exactly one person" part R's `.*` lets through. Three of the four are
    // logical flags with no offender in them, so the gate is presence — the
    // same argument as `description-malformed-name`, and the same one a rule
    // keyed on a single scalar field always makes.
    //
    // Note the exclusions the rule mirrors for containment to hold: `ORPHANED`
    // is R's second alternative, the address classes are looser than RFC 5322
    // (a quoted local part, no TLD, a leading `-` in a domain label), and a
    // `Maintainer` folded across continuation lines is one R accepts, since it
    // matches the field with a `.` that takes a newline.
    Gate {
        rule: "description-malformed-maintainer",
        signals: &[
            "bad_maintainer",
            "empty_Maintainer_name",
            "Maintainer_needs_quotes",
            "Maintainer_invalid_or_multi_person",
        ],
        backing: Backing::Signal,
    },
    // Thirteen signals, three checkers, two fields — the widest gate here, and
    // presence-backed for the usual reason: most of them are logical flags, and
    // the ones that do name an offender name R's *formatted* person
    // (`Jane Doe <jane@example.com> [aut, cre]`), which is not text that appears
    // in the file at all, so there is nothing to compare span to span.
    //
    // What the gate is really worth is the real-world half of the corpus: a
    // clean `DESCRIPTION` raises none of these, so any finding on one is a
    // false positive and fails here.
    //
    // Note the exclusions the rule mirrors for containment to hold: it resolves
    // the field statically and withholds every finding that depends on a
    // computed argument, roles are matched against the whole 302-code relator
    // table (not the eleven codes CRAN suggests) plus the terms R's own
    // fallback would resolve, and `bad_authors_at_R_field_has_no_author_roles`
    // is left to R — see `PLANNED`.
    //
    // The role signal is the driver's own, and it has to be: `person()` drops a
    // role it cannot match *before* any check component runs, so R says so only
    // in a warning. `authors_at_R_field_has_persons_with_nonstandard_roles`
    // could never back the finding — by the time it looks, the role is gone.
    Gate {
        rule: "description-authors-at-r",
        signals: &[
            "bad_authors_at_R_field",
            "bad_authors_at_R_field_has_no_author",
            "bad_authors_at_R_field_has_no_valid_maintainer",
            "bad_authors_at_R_field_too_many_maintainers",
            "bad_authors_at_R_field_has_persons_with_no_name",
            "bad_authors_at_R_field_has_persons_with_no_role",
            "bad_authors_at_R_field_has_persons_with_bad_ORCID_identifiers",
            "bad_authors_at_R_field_has_persons_with_dup_ORCID_identifiers",
            "bad_authors_at_R_field_has_persons_with_bad_ROR_identifiers",
            "bad_authors_at_R_field_has_persons_with_dup_ROR_identifiers",
            "authors_at_R_field_has_invalid_role_specifications",
            "author_starts_with_Author",
            "author_should_be_authors_at_R",
        ],
        backing: Backing::Signal,
    },
];

/// Signals no rule covers yet, each tagged with the `TODO.md` rule it is
/// earmarked for. Listing them is the point: this is the work-list, and moving
/// one into [`GATES`] is what "the rule landed" means.
const PLANNED: &[(&str, &str)] = &[
    ("bad_Title", "description-title-format"),
    ("bad_Description", "description-text-format"),
    ("missing_encoding", "description-encoding"),
    ("fields_with_non_ASCII_tags", "description-encoding"),
    ("fields_with_non_ASCII_values", "description-encoding"),
    // `bad_authors_at_R_field_*` is otherwise gated; these three are the ones
    // `description-authors-at-r` deliberately leaves to R.
    //
    // The two formatting failures are reachable only through `format.person`
    // itself, which arity does not reimplement — there is no static reading of
    // "R could not render this person" — and the `Author` half of the pair
    // still gates the more basic `has_no_author` it implies.
    ("bad_authors_at_R_field_for_author", ""),
    ("bad_authors_at_R_field_for_maintainer", ""),
    // Deliberately unclaimed: R raises it only at `strict >= 2`, which
    // `R CMD check` never asks for, and it fires on any package whose sole
    // author writes `role = "cre"` without also writing `"aut"` — a shape CRAN
    // accepts by the thousand. Reporting it would be arity's opinion, not R's.
    ("bad_authors_at_R_field_has_no_author_roles", ""),
    // Deliberately unclaimed. `Priority: base`/`recommended` is reserved for the
    // packages that ship with R, so this fires on roughly nobody, and
    // `VignetteBuilder` is checked only under R's `strict`. Both are classified
    // so they cannot trip the unknown-signal gate, not because a rule is coming.
    ("bad_priority", ""),
    ("bad_vignettebuilder", ""),
];

// ---------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------

#[test]
#[ignore = "R CMD check DESCRIPTION oracle; run via `task description-oracle`"]
fn description_rules_match_r_cmd_check() {
    let Some(rscript) = locate_rscript() else {
        eprintln!(
            "description-oracle: `Rscript` not found on PATH; skipping (this is not a failure)."
        );
        return;
    };
    let driver = manifest_path("tests/oracle/description_oracle.R");
    if !driver.is_file() {
        eprintln!(
            "description-oracle: driver {} missing; skipping.",
            driver.display()
        );
        return;
    }

    let cases = corpus();
    assert!(!cases.is_empty(), "the oracle corpus should not be empty");

    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut unknown: BTreeMap<String, String> = BTreeMap::new();
    let mut planned_hits: BTreeMap<&str, usize> = BTreeMap::new();
    let mut exercised: BTreeSet<String> = BTreeSet::new();

    for (label, input) in &cases {
        let Some(signals) = run_oracle(&rscript, &driver, input) else {
            eprintln!("description-oracle: {label}: driver could not process; skipped.");
            continue;
        };
        checked += 1;

        for (name, detail) in &signals {
            exercised.insert(name.clone());
            if is_gated(name) {
                continue;
            }
            match PLANNED.iter().find(|(signal, _)| signal == name) {
                Some((_, owner)) => *planned_hits.entry(owner).or_default() += 1,
                None => {
                    unknown
                        .entry(name.clone())
                        .or_insert_with(|| format!("{label}: {detail:?}"));
                }
            }
        }

        if let Err(why) = compare(input, &signals) {
            failures.push(format!("{label}: {why}"));
        }
    }

    report(checked, cases.len(), &planned_hits);

    // An R upgrade that adds a check must be classified deliberately.
    assert!(
        unknown.is_empty(),
        "R raised {} signal(s) this harness does not classify. Add each to \
         `GATES` or `PLANNED`:\n{}",
        unknown.len(),
        unknown
            .iter()
            .map(|(name, first)| format!("  {name}  (first seen at {first})"))
            .collect::<Vec<_>>()
            .join("\n"),
    );

    // An oracle nothing exercises passes quietly forever.
    let unexercised: Vec<&str> = GATES
        .iter()
        .flat_map(|gate| gate.signals.iter().copied())
        .filter(|signal| !exercised.contains(*signal))
        .collect();
    assert!(
        unexercised.is_empty(),
        "no corpus case triggers {unexercised:?}, so the gate on it proves \
         nothing; add an adversarial case",
    );

    assert!(
        failures.is_empty(),
        "arity disagrees with `R CMD check` on {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

/// Every arity finding for a gated rule must be backed by one of R's signals.
fn compare(input: &str, signals: &[(String, String)]) -> Result<(), String> {
    for gate in GATES {
        let backing: BTreeSet<&str> = signals
            .iter()
            .filter(|(name, _)| gate.signals.contains(&name.as_str()))
            .map(|(_, detail)| detail.trim())
            .collect();

        for finding in rule_hits(input, gate.rule) {
            let backed = match gate.backing {
                // R reports the offending dependency entry trimmed, and arity
                // spans the whole entry, so the texts are directly comparable.
                Backing::Entry => backing.contains(finding.trim()),
                Backing::Signal => !backing.is_empty(),
            };
            if !backed {
                return Err(format!(
                    "`{}` flagged {:?}, which R does not consider bad \
                     (R flagged {:?} via {:?})",
                    gate.rule, finding, backing, gate.signals,
                ));
            }
        }
    }
    Ok(())
}

fn is_gated(signal: &str) -> bool {
    GATES
        .iter()
        .any(|gate| gate.signals.contains(&signal.trim()))
}

/// The source text arity spans for each finding of `rule`.
fn rule_hits(source: &str, rule: &str) -> Vec<String> {
    let path = Path::new("DESCRIPTION");
    let Ok(diagnostics) = check_description_document(path, source, &LintConfig::default()) else {
        return Vec::new();
    };
    diagnostics
        .iter()
        .filter(|d| d.rule == rule)
        .filter_map(|d| {
            let start = usize::from(d.range.start());
            let end = usize::from(d.range.end());
            source.get(start..end).map(str::to_string)
        })
        .collect()
}

/// The work-list, printed under `--nocapture`. Ordered by hit count, because
/// how often a signal actually fires is the argument for building its rule next.
fn report(checked: usize, total: usize, planned_hits: &BTreeMap<&str, usize>) {
    eprintln!("description-oracle: {checked}/{total} cases checked.");
    if planned_hits.is_empty() {
        return;
    }
    let mut ranked: Vec<(&&str, &usize)> = planned_hits.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    eprintln!("description-oracle: signals R raises that no rule covers yet:");
    for (owner, count) in ranked {
        let owner = if owner.is_empty() {
            "(unclaimed)"
        } else {
            owner
        };
        eprintln!("  {count:4}  {owner}");
    }
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// Real `DESCRIPTION`s first, then a table of planted defects.
///
/// The real ones are almost all clean, which is exactly what makes them worth
/// running: they are the false-positive gate. The planted table is what gives
/// the oracle teeth, and it exists to make every signal in [`GATES`] and
/// [`PLANNED`] fire at least once.
fn corpus() -> Vec<(String, String)> {
    let mut cases: Vec<(String, String)> = Vec::new();

    collect_dir(
        &mut cases,
        &manifest_path("tests/fixtures/rindex"),
        "DESCRIPTION",
        "rindex",
    );
    // Reference-only checkouts: absent in a fresh clone, and that is normal.
    collect_dir(
        &mut cases,
        &manifest_path("roxygen2-ref/tests/testthat"),
        "DESCRIPTION",
        "roxygen2-ref",
    );
    for (tag, rel) in [
        ("roxygen2-ref", "roxygen2-ref/DESCRIPTION"),
        ("style", "style/DESCRIPTION"),
    ] {
        let path = manifest_path(rel);
        if let Ok(text) = std::fs::read_to_string(&path) {
            cases.push((format!("{tag}/root"), text));
        }
    }

    for (label, text) in PLANTED {
        cases.push((format!("planted/{label}"), (*text).to_string()));
    }
    cases
}

/// Push every `<dir>/*/<file_name>` under `root`, labeled `<tag>/<subdir>`.
fn collect_dir(cases: &mut Vec<(String, String)>, root: &Path, file_name: &str, tag: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join(file_name))
        .filter(|path| path.is_file())
        .collect();
    found.sort();
    for path in found {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let label = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        cases.push((format!("{tag}/{label}"), text));
    }
}

/// One planted defect per signal, plus the clean control.
///
/// Note the package names: R's `valid_package_name` demands **at least two
/// characters**, so a one-letter `Package: p` is itself a `bad_package` and
/// would contaminate every other case here.
const PLANTED: &[(&str, &str)] = &[
    (
        "clean",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Description: Fixture data for arity's own tests.\nLicense: MIT + file LICENSE\n\
         Authors@R: person(\"Test\", \"User\", email = \"test@example.com\", \
         role = c(\"aut\", \"cre\"))\n",
    ),
    // The three ways R rejects a dependency entry — the gated set.
    (
        "dep-entry",
        "Package: testpkg\nVersion: 0.1.0\nImports: dplyr (1.0.0)\n",
    ),
    (
        "dep-op",
        "Package: testpkg\nVersion: 0.1.0\nImports: dplyr (=> 1.0)\n",
    ),
    (
        "dep-version",
        "Package: testpkg\nVersion: 0.1.0\nImports: dplyr (>= foo)\n",
    ),
    (
        "dep-r-svn-revision",
        "Package: testpkg\nVersion: 0.1.0\nDepends: R (>= r12345)\n",
    ),
    (
        "dep-bad-name",
        "Package: testpkg\nVersion: 0.1.0\nImports: 3dplyr\n",
    ),
    (
        "duplicates",
        "Package: testpkg\nVersion: 0.1.0\nImports: dplyr\nSuggests: dplyr\n",
    ),
    ("bad-package-name", "Package: 3bad\nVersion: 0.1.0\n"),
    ("base-package-name", "Package: stats\nVersion: 0.1.0\n"),
    // The two names R accepts that a rule written from the regexp alone would
    // reject: the language's own name is an explicit alternative, and a base
    // package declaring itself is exempt from the base-name clause.
    ("package-name-r", "Package: R\nVersion: 0.1.0\n"),
    (
        "base-package-naming-itself",
        "Package: stats\nVersion: 0.1.0\nPriority: base\n",
    ),
    ("bad-version", "Package: testpkg\nVersion: 1.0.0.beta\n"),
    (
        "bad-maintainer-no-email",
        "Package: testpkg\nVersion: 0.1.0\nMaintainer: Jane Doe\n",
    ),
    // The three CRAN Maintainer NOTEs. Like the version ones below, they need a
    // `Title` and a `Version` before `.check_package_CRAN_incoming` will reach
    // them. Note the second: R's own regexp *accepts* two maintainers, so this
    // case fires no `bad_maintainer` at all and the CRAN NOTE is the only thing
    // backing the rule on it.
    (
        "maintainer-empty-name",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Maintainer: <jane@example.com>\n",
    ),
    (
        "maintainer-multi-person",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Maintainer: Jane Doe <jane@example.com>, John Roe <john@example.org>\n",
    ),
    (
        "maintainer-needs-quotes",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Maintainer: Doe, Jane <jane@example.com>\n",
    ),
    // Where the rule deliberately says nothing, so the gate sees the
    // withholding and not only the reporting: `ORPHANED` is R's second
    // alternative, a wrapped field is one R's `.` folds right through, and the
    // address grammar is looser than RFC 5322 in three separate places.
    (
        "maintainer-orphaned",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\nMaintainer: ORPHANED\n",
    ),
    (
        "maintainer-wrapped",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Maintainer: Jane Doe\n  <jane@example.com>\n",
    ),
    (
        "maintainer-loose-address",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Maintainer: \"Doe, Jane\" <\"jane doe\"@example>\n",
    ),
    (
        "bad-title-trailing-period",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Title Ending In A Period.\n",
    ),
    (
        "bad-description-no-final-punctuation",
        "Package: testpkg\nVersion: 0.1.0\nDescription: no final punctuation\n",
    ),
    (
        "priority",
        "Package: testpkg\nVersion: 0.1.0\nPriority: base\n",
    ),
    (
        "vignettebuilder",
        "Package: testpkg\nVersion: 0.1.0\nVignetteBuilder: knitr-x\n",
    ),
    // `missing_encoding` is *not* "has non-ASCII": R's condition is
    // `!all(.is_ISO_8859(db))`, so a Latin-1-representable `Café` does not
    // trigger it and text outside ISO-8859 does.
    (
        "missing-encoding",
        "Package: testpkg\nVersion: 0.1.0\nTitle: \u{65e5}\u{672c}\u{8a9e}\n",
    ),
    (
        "non-ascii-value",
        "Package: testpkg\nVersion: 0.1.0\nEncoding: UTF-8\nLicense: MIT \u{2014} x\n",
    ),
    (
        "non-ascii-tag",
        "Package: testpkg\nVersion: 0.1.0\nCaf\u{e9}: x\n",
    ),
    (
        "authors-unparseable",
        "Package: testpkg\nVersion: 0.1.0\nAuthors@R: person(\"A\",\n",
    ),
    (
        "authors-no-name",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: person(role = c(\"aut\", \"cre\"), email = \"a@example.com\")\n",
    ),
    (
        "authors-no-role",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: c(person(\"A\", \"B\", role = c(\"aut\", \"cre\"), \
         email = \"a@example.com\"), person(\"C\", \"D\"))\n",
    ),
    // The per-person clauses live inside R's `else` branch, so they are
    // reachable only when the field yields *some* author: a lone nameless
    // person is `bad_authors_at_R_field_has_no_author` and nothing more.
    (
        "authors-person-without-a-name",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: c(person(\"A\", \"B\", role = c(\"aut\", \"cre\"), \
         email = \"a@example.com\"), person(role = \"ctb\"))\n",
    ),
    (
        "authors-cre-without-email",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))\n",
    ),
    (
        "authors-bad-orcid",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: person(\"A\", \"B\", role = c(\"aut\", \"cre\"), \
         email = \"a@example.com\", comment = c(ORCID = \"1234-5678-9012-3456\"))\n",
    ),
    (
        "authors-dup-orcid",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: c(person(\"A\", \"B\", role = c(\"aut\", \"cre\"), \
         email = \"a@example.com\", comment = c(ORCID = \"0000-0002-1825-0097\")), \
         person(\"C\", \"D\", role = \"ctb\", \
         comment = c(ORCID = \"0000-0002-1825-0097\")))\n",
    ),
    (
        "authors-bad-ror",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: person(\"Some Institute\", role = c(\"cph\", \"cre\"), \
         email = \"a@example.com\", comment = c(ROR = \"12345\"))\n",
    ),
    (
        "authors-dup-ror",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: c(person(\"Some Institute\", role = c(\"cph\", \"cre\"), \
         email = \"a@example.com\", comment = c(ROR = \"03wc8by49\")), \
         person(\"Other Institute\", role = \"fnd\", \
         comment = c(ROR = \"03wc8by49\")))\n",
    ),
    (
        "authors-two-creators",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: c(person(\"A\", \"B\", role = c(\"aut\", \"cre\"), \
         email = \"a@example.com\"), person(\"C\", \"D\", role = \"cre\", \
         email = \"c@example.com\"))\n",
    ),
    (
        "authors-nonstandard-role",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: person(\"A\", \"B\", role = c(\"aut\", \"cre\", \"zzz\"), \
         email = \"a@example.com\")\n",
    ),
    // Where the rule deliberately says nothing about a role, so the gate sees
    // the withholding: `spy` is a real relator code however it reads, and
    // `compiler` is a relator *term*, which R's own fallback is what would
    // resolve.
    (
        "authors-relator-code-and-term",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: person(\"A\", \"B\", role = c(\"aut\", \"cre\", \"spy\", \"compiler\"), \
         email = \"a@example.com\")\n",
    ),
    // ...and where it says nothing about the people at all, because a computed
    // argument is not something a static reading may guess at.
    (
        "authors-computed",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: person(\"A\", \"B\", role = ROLES, email = \"a@example.com\")\n",
    ),
    // `person()` with no arguments is a zero-length person vector, not a
    // nameless person — a shape that really does end `xfun`'s `Authors@R`, and
    // one `description-authors-at-r` reported until this case existed. The
    // leftover call is `description-empty-person`'s subject instead, and that
    // rule is deliberately **not** in `GATES`: it is the one packaging finding
    // R does not back, which is exactly why it has an id of its own.
    (
        "authors-argument-less-person",
        "Package: testpkg\nVersion: 0.1.0\n\
         Authors@R: c(person(\"A\", \"B\", role = c(\"aut\", \"cre\"), \
         email = \"a@example.com\"), person())\n",
    ),
    // The two `Author` clauses. Both are CRAN pretest components, so the case
    // needs the `Title` and `Maintainer` that checker insists on.
    (
        "author-starts-with-author",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Maintainer: Jane Doe <jane@example.com>\n\
         Author: Author: Jane Doe [aut, cre]\n",
    ),
    (
        "author-should-be-authors-at-r",
        "Package: testpkg\nVersion: 0.1.0\nTitle: A Test Package\n\
         Maintainer: Jane Doe <jane@example.com>\n\
         Author: person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))\n",
    ),
    // The two CRAN version NOTEs. `.check_package_CRAN_incoming` reaches the
    // version only after a `Maintainer` it can compare against "ORPHANED" and a
    // `Title` it can inspect, and errors on the `NA` otherwise, so these cases
    // carry both — a signal no case fires is a gate that proves nothing.
    (
        "version-leading-zeroes",
        "Package: testpkg\nVersion: 1.01\nTitle: A Test Package\n\
         Maintainer: Jane Doe <jane@example.com>\n",
    ),
    (
        "version-large-component",
        "Package: testpkg\nVersion: 1234.0\nTitle: A Test Package\n\
         Maintainer: Jane Doe <jane@example.com>\n",
    ),
    // Where arity is deliberately looser than CRAN, so the gate sees the
    // withholding and not only the reporting: CRAN NOTEs the second of these
    // and arity says nothing, because a trailing `.9000` is the
    // development-version marker `usethis` writes and CRAN never expects to
    // receive.
    (
        "version-calendar",
        "Package: testpkg\nVersion: 2026.01\nTitle: A Test Package\n\
         Maintainer: Jane Doe <jane@example.com>\n",
    ),
    (
        "version-development-suffix",
        "Package: testpkg\nVersion: 0.1.0.9000\nTitle: A Test Package\n\
         Maintainer: Jane Doe <jane@example.com>\n",
    ),
];

// ---------------------------------------------------------------------------
// Driver plumbing
// ---------------------------------------------------------------------------

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn locate_rscript() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ARITY_RSCRIPT") {
        return Some(PathBuf::from(path));
    }
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
        .then(|| PathBuf::from("Rscript"))
}

/// Run the driver over `input`, or `None` when it could not process the case.
fn run_oracle(rscript: &Path, driver: &Path, input: &str) -> Option<Vec<(String, String)>> {
    let mut child = Command::new(rscript)
        .arg(driver)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // The driver reports `ERROR` when it could not read the file at all. That is
    // `dcf_oracle`'s subject, not this one's, so it is a skip here.
    if text.lines().any(|line| line.starts_with("ERROR\t")) {
        return None;
    }
    Some(
        text.lines()
            .filter_map(|line| line.strip_prefix("SIGNAL\t"))
            .map(|rest| {
                let (name, detail) = rest.split_once('\t').unwrap_or((rest, ""));
                (name.to_string(), unescape(detail))
            })
            .collect(),
    )
}

/// Inverse of the driver's `escape`.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
