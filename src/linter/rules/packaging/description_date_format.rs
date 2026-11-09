//! Require a complete calendar date in the optional Date field.
//!
//! Calendar validation is static so findings never depend on when lint runs.
//! No fix is offered because an ambiguous date needs the author's decision.

use crate::linter::diagnostic::{Diagnostic, ViolationData};
use crate::linter::rules::packaging::scalar_field::value;
use crate::linter::rules::{DcfRule, DcfRuleContext, Example};

pub struct DescriptionDateFormat;

impl DcfRule for DescriptionDateFormat {
    fn id(&self) -> &'static str {
        "description-date-format"
    }

    fn description(&self) -> &'static str {
        "Require the optional `Date` field to contain a valid calendar date in ISO 8601 `yyyy-mm-dd` form, with a four-digit year from 0000 to 9999, a two-digit month, and a two-digit day. Check month lengths and leap years as well as spelling. Missing or empty values are ignored.\n\nThe date's age and whether it lies in the future are deliberately unchecked: lint results must not change overnight without an edit. No autofix is offered because choosing the intended date requires the author."
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            caption: "A date with an ambiguous order:",
            source: "Package: mypkg\nDate: 09/11/2026\n",
        }]
    }

    fn check_file(&self, ctx: &DcfRuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        let Some(field) = ctx.document.field("Date") else {
            return;
        };
        let Some((date, range)) = value(&field) else {
            return;
        };
        if valid_date(&date) {
            return;
        }
        sink.push(Diagnostic {
            rule: self.id(),
            severity: Default::default(),
            path: Default::default(),
            range,
            message: ViolationData::new(
                self.id(),
                "`Date` must be a valid calendar date in `yyyy-mm-dd` form",
            )
            .with_suggestion(
                "Write the intended date with a four-digit year and two-digit month and day.",
            ),
            fix: None,
        });
    }
}

fn valid_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
    {
        return false;
    }
    // The shape check guarantees ASCII digits, so these slices and parses are total.
    let year: u32 = date[..4].parse().unwrap();
    let month: u32 = date[5..7].parse().unwrap();
    let day: u32 = date[8..].parse().unwrap();
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    (1..=days).contains(&day)
}
