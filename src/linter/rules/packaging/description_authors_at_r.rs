//! `description-authors-at-r`: an `Authors@R` field R will not read the way its
//! author meant it.
//!
//! `Authors@R` is the field `R CMD build` derives `Author` and `Maintainer`
//! from, and the derivation is exacting: it needs a person with the `cre` role,
//! a **non-empty name**, and an **email**, and errors out with "Authors@R field
//! gives no person with maintainer role, valid email address and non-empty
//! name" when no one qualifies. That is the headline finding here, and
//! `person("Jane", "Doe", role = c("aut", "cre"))` — no `email` — is the shape
//! that hits it.
//!
//! Around it sit the rest of R's `.check_package_description_authors_at_R_field`
//! clauses that are decidable from the text:
//!
//! - the field is **not R** (`str2expression` fails), or holds a **call R
//!   refuses to evaluate** — `.read_authors_at_R_field(strict = TRUE)` allows
//!   only `person`, `as.person`, `c`, `list`, `paste`, `paste0`, and `(`;
//! - a person with **no name** or **no role**, credited nowhere at all;
//! - a **role R does not know**, which `person()` drops on the floor;
//! - **more than one `cre`**, where R stores exactly one maintainer;
//! - a **malformed or duplicated ORCID iD or ROR ID**. Both identifiers are
//!   self-validating — an ORCID carries a MOD 11-2 check digit — so this needs
//!   no network, which is the whole reason it can be a lint.
//!
//! Two CRAN pretest checks on the neighboring **`Author`** field belong to the
//! same conversation, since both are `Authors@R` content written under the
//! wrong key: a value that literally begins `Author:` (a pasted-in field
//! header), and a value that is a `person(...)` or `c(...)` call, which R
//! stores as a plain string and never evaluates — so the brackets and quotes
//! land verbatim in the rendered credit.
//!
//! **Nothing is evaluated** (`.claude/rules/linter.md`; the static-semantics
//! tenet). The value is parsed with arity's own R parser and resolved only as
//! far as literal text goes, exactly as `src/project/description.rs` resolves
//! the `Roxygen` field. A computed argument resolves to "unknown" and every
//! finding that depends on it is withheld, so the rule reports strictly less
//! than R does and never more.
//!
//! **No autofix.** An email, a name, a role, and a check digit are all facts
//! about a person that only that person has; there is nothing to edit *to*.

use std::collections::HashMap;

use rowan::TextRange;

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::packaging::authors::{
    self, Authors, Literal, Person, Value, is_known_role, is_orcid_url, is_valid_orcid,
    is_valid_ror,
};
use crate::linter::rules::packaging::scalar_field::{escape, folded, value};
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionAuthorsAtR;

const RULE: &str = "description-authors-at-r";

const EXAMPLES: &[Example] = &[
    Example {
        caption: "A creator with no `email`, which is what R needs to derive a \
                  `Maintainer` and refuses to build without:",
        source: "Package: mypkg\nVersion: 0.1.0\n\
                 Authors@R: person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))\n",
    },
    Example {
        caption: "A person credited nowhere, because `person()` drops anyone with \
                  no role:",
        source: "Package: mypkg\nVersion: 0.1.0\n\
                 Authors@R: c(\n    \
                 person(\"Jane\", \"Doe\", , \"jane@example.com\", c(\"aut\", \"cre\")),\n    \
                 person(\"John\", \"Roe\")\n  )\n",
    },
    Example {
        caption: "An ORCID iD whose MOD 11-2 check digit does not add up:",
        source: "Package: mypkg\nVersion: 0.1.0\n\
                 Authors@R: person(\"Jane\", \"Doe\", , \"jane@example.com\", c(\"aut\", \"cre\"),\n    \
                 comment = c(ORCID = \"0000-0002-1825-0098\"))\n",
    },
    Example {
        caption: "R code under the `Author` key, which R stores as a plain string \
                  and never evaluates:",
        source: "Package: mypkg\nVersion: 0.1.0\n\
                 Author: person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))\n",
    },
];

impl DcfRule for DescriptionAuthorsAtR {
    fn id(&self) -> &'static str {
        RULE
    }

    fn description(&self) -> &'static str {
        "Flag an `Authors@R` field R will not read the way its author \
         meant it.\n\n`R CMD build` derives `Author` and `Maintainer` from this \
         field, and the derivation is exacting: it needs a person with the \
         `cre` role, a non-empty name, **and** an email. Without one it errors \
         out—\"Authors@R field gives no person with maintainer role, valid \
         email address and non-empty name\"—so \
         `person(\"Jane\", \"Doe\", role = c(\"aut\", \"cre\"))` is a package that \
         does not build.\n\nThe rest of R's \
         `.check_package_description_authors_at_R_field` is reported by the same \
         rule: a field that is not R, or that holds a call R refuses to \
         evaluate (only `person`, `as.person`, `c`, `list`, `paste`, and \
         `paste0` are allowed); a person with no name or no role, who is \
         credited nowhere at all; a role outside the MARC relator table, which \
         `person()` silently drops; more than one `cre`, where R stores exactly \
         one maintainer; and a malformed or duplicated ORCID iD or ROR ID. Both \
         identifiers are self-validating—an ORCID carries a MOD 11-2 check \
         digit—so no network is involved.\n\nTwo checks on the neighboring \
         `Author` field are here for the same reason, since both are `Authors@R` \
         content written under the wrong key: a value that begins with the field \
         header `Author:`, and a value that is a `person(...)` or `c(...)` call, \
         which R stores verbatim and never evaluates.\n\nNothing is evaluated. \
         The value is parsed with arity's own R parser and resolved only as far \
         as literal text goes; a computed argument resolves to unknown and every \
         finding that depends on it is withheld, so the rule reports strictly \
         less than `R CMD check` does and never more.\n\nThere is no autofix: an \
         email, a name, a role, and a check digit are all facts about a person \
         that only that person has."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        check_author_field(ctx, sink);
        check_authors_at_r(ctx, sink);
    }
}

/// The two CRAN pretest checks on `Author`, both of which are `Authors@R`
/// content filed under the wrong key.
fn check_author_field(ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
    let Some((author, range)) = ctx.document.field("Author").and_then(|f| value(&f)) else {
        return;
    };

    let (message, suggestion) = if authors::starts_with_its_own_field_name(&author) {
        (
            format!(
                "the `Author` value `{}` starts with its own field name",
                escape(&author)
            ),
            "Drop the repeated `Author:` header: the field's value is everything \
             after the first colon.",
        )
    } else if authors::looks_like_r_code(&author) {
        (
            format!("`{}` is R code under the `Author` key", escape(&author)),
            "Move the call to `Authors@R`, which R evaluates. `Author` is a plain \
             string R prints as written, brackets and quotes included.",
        )
    } else {
        return;
    };

    sink.push(finding(range, message, suggestion));
}

fn check_authors_at_r(ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
    let Some(field) = ctx.document.field("Authors@R") else {
        return;
    };
    let Some(source) = folded(&field) else {
        return;
    };

    let persons = match authors::resolve(&source) {
        Authors::Unparseable => {
            sink.push(finding(
                source.range,
                "`Authors@R` does not parse as R".to_string(),
                "R reads this field with `str2expression` and evaluates the result, \
                 so it has to be valid R before anything else about it matters.",
            ));
            return;
        }
        Authors::UnsafeCall(name, range) => {
            sink.push(finding(
                range,
                format!("`{name}` is a call R refuses to evaluate in `Authors@R`"),
                "R evaluates this field, so it allows only `person`, `as.person`, \
                 `c`, `list`, `paste`, and `paste0`. Write the value out literally.",
            ));
            return;
        }
        // Not statically resolvable: the rule has nothing to say, exactly as
        // the roxygen markdown resolver stays silent on a computed list.
        Authors::Unresolved => return,
        Authors::Persons(persons) => persons,
    };

    let creators = creators(&persons);
    for person in &persons {
        check_roles(person, sink);
        check_identifiers(person, sink);
        check_person(person, creators.as_deref(), sink);
    }
    check_duplicate_identifiers(&persons, sink);

    let Some(creators) = creators else {
        return;
    };
    match creators.len() {
        0 => sink.push(finding(
            source.range,
            "`Authors@R` names no person with the `cre` role".to_string(),
            "Give the package's maintainer `role = \"cre\"`, a name, and an \
             `email`: that is the person `R CMD build` derives `Maintainer` from.",
        )),
        1 => {}
        n => sink.push(finding(
            source.range,
            format!("`Authors@R` gives the `cre` role to {n} people"),
            "R's `Maintainer` is one person. Leave `cre` on the one to write to \
             and credit the rest with `aut` or `ctb`.",
        )),
    }
}

/// The people carrying `cre`, or `None` when a computed role means arity cannot
/// say who does.
fn creators(persons: &[Person]) -> Option<Vec<&Person>> {
    let mut creators = Vec::new();
    for person in persons {
        if person.has_role("cre")? {
            creators.push(person);
        }
    }
    Some(creators)
}

/// One structural finding per person, first match winning: a person with no
/// name has no email problem worth raising, and "credited nowhere" is the more
/// basic reading of the same `person()` call.
fn check_person(person: &Person, creators: Option<&[&Person]>, sink: &mut Vec<Diagnostic>) {
    // Every clause below asks what R does with a person it has; a person built
    // out of computed arguments may be R's zero-length vector instead.
    if !person.is_materialized() {
        return;
    }
    let sole_creator =
        creators.is_some_and(|creators| creators.len() == 1 && std::ptr::eq(creators[0], person));

    let (message, suggestion) = if person.has_name() == Some(false) {
        (
            "this person has no name, so R credits them nowhere".to_string(),
            "Give the person a `given` or a `family` name: `person()` drops \
             anyone with neither from both `Author` and `Maintainer`.",
        )
    } else if matches!(person.role, Value::Missing) {
        (
            "this person has no role, so R credits them nowhere".to_string(),
            "Add a `role`, such as `\"aut\"` for an author or `\"ctb\"` for a \
             contributor: `person()` drops anyone with no role from `Author`.",
        )
    } else if sole_creator && person.email.is_present() == Some(false) {
        (
            "the package's `cre` has no email address, so R can derive no \
             `Maintainer`"
                .to_string(),
            "Add `email = \"name@example.com\"`. `R CMD build` needs a `cre` with \
             a name and an address, and errors out without one.",
        )
    } else {
        return;
    };

    sink.push(finding(person.range, message, suggestion));
}

/// A role outside the MARC relator table is dropped by `person()`, so the
/// credit the author wrote is silently lost.
fn check_roles(person: &Person, sink: &mut Vec<Diagnostic>) {
    for role in person.role.literals() {
        if is_known_role(&role.text) {
            continue;
        }
        sink.push(finding(
            role.range,
            format!("`{}` is not a role R knows", escape(&role.text)),
            "Use a MARC relator code — `aut`, `cre`, `ctb`, `cph`, `fnd`, … — \
             or R drops the role and the credit with it.",
        ));
    }
}

fn check_identifiers(person: &Person, sink: &mut Vec<Diagnostic>) {
    if let Some(orcid) = orcid(person)
        && !is_valid_orcid(&orcid.text)
    {
        sink.push(finding(
            orcid.range,
            format!("`{}` is not a valid ORCID iD", escape(&orcid.text)),
            "An ORCID iD is `0000-0002-1825-0097` and carries a check digit, so a \
             mistyped one is decidable without asking orcid.org.",
        ));
    }
    if let Some(ror) = person.comment.named("ROR")
        && !is_valid_ror(&ror.text)
    {
        sink.push(finding(
            ror.range,
            format!("`{}` is not a valid ROR ID", escape(&ror.text)),
            "A ROR ID is nine characters, as in `03wc8by49`, optionally written \
             out as `https://ror.org/03wc8by49`.",
        ));
    }
}

/// Two people cannot share one identifier; the later occurrence is the copy.
fn check_duplicate_identifiers(persons: &[Person], sink: &mut Vec<Diagnostic>) {
    for (label, ids) in [
        ("ORCID iD", collect(persons, orcid)),
        ("ROR ID", collect(persons, |p| p.comment.named("ROR"))),
    ] {
        let mut seen: HashMap<&str, ()> = HashMap::new();
        for id in ids {
            if seen.insert(id.text.as_str(), ()).is_none() {
                continue;
            }
            sink.push(finding(
                id.range,
                format!(
                    "two people are given the same {label}, `{}`",
                    escape(&id.text)
                ),
                "One of the two is a copy of the other's: an identifier names \
                 exactly one person or organization.",
            ));
        }
    }
}

fn collect<'a>(
    persons: &'a [Person],
    of: impl Fn(&'a Person) -> Option<&'a Literal>,
) -> Vec<&'a Literal> {
    persons.iter().filter_map(of).collect()
}

/// The ORCID iD `person()` would read: the `ORCID` element of `comment`, or an
/// unnamed element R labels itself because it is an `orcid.org` URL.
fn orcid(person: &Person) -> Option<&Literal> {
    person.comment.named("ORCID").or_else(|| {
        person
            .comment
            .literals()
            .iter()
            .find(|item| item.name.is_none() && is_orcid_url(&item.text))
    })
}

fn finding(range: TextRange, message: String, suggestion: &str) -> Diagnostic {
    Diagnostic {
        rule: RULE,
        severity: Default::default(),
        path: Default::default(),
        range,
        message: ViolationData::new(RULE, message).with_suggestion(suggestion.to_string()),
        fix: None,
    }
}
