//! `misnamed-suppression`: a directive naming a rule that does not exist, or
//! one that does not parse as a directive at all.
//!
//! Both fail silently by construction. `# arity-lint skip unusd-binding: …`
//! suppresses nothing, and because a suppression's whole job is to make output
//! disappear, there is no signal that it went wrong. The same is true of the
//! comma-list shape `# arity-lint skip a, b`: the parser takes the rule ID up to
//! the first whitespace, so the directive names `a,` and silences neither rule —
//! and of `# arity-format skipp`, which names no verb arity knows and so reads
//! as an ordinary comment.
//!
//! The fix rewrites the ID alone (the directive's `RuleRef` range), leaving the
//! author's reason prose in place. It is `Safe` — a suppression comment carries
//! no program behavior, so rewriting one cannot change what the code does — but
//! it is withheld unless the intent is unambiguous: exactly one shipped rule ID
//! is within a short edit distance, and no other candidate ties it. A wrong
//! guess would start hiding a diagnostic the author never asked to hide.

use crate::linter::diagnostic::{Diagnostic, Fix, ViolationData};
use crate::linter::rules::{Example, Rule, RuleContext, all_rule_ids, is_known_rule};
use crate::linter::suppression::{Malformed, MalformedKind, RuleRef};

pub struct MisnamedSuppression;

const EXAMPLES: &[Example] = &[
    Example {
        caption: "The rule ID is misspelled, so the directive suppresses nothing:",
        source: "# arity-lint skip unusd-binding: leftover from a refactor\nx <- 1\n",
    },
    Example {
        caption: "A comma-separated list is not supported — write one directive per rule:",
        source: "# arity-lint skip browser, repeat: debugging\nx <- 1\n",
    },
];

impl Rule for MisnamedSuppression {
    fn id(&self) -> &'static str {
        "misnamed-suppression"
    }

    fn description(&self) -> &'static str {
        "Flags an `# arity` directive that names a rule arity does not ship, \
or that does not parse as a directive at all (an unknown verb, a missing one, a \
rule named where the form takes none). Either way it suppresses nothing, and \
does so silently — the failure mode of a suppression is that no output appears, \
which is also what success looks like. When exactly one shipped rule ID is an \
unambiguous near-match, the fix rewrites the ID and leaves the reason text \
alone; otherwise the finding is report-only. Note that `syntax-error` is not a \
lint rule: parse errors are reported before any rule runs and cannot be \
suppressed."
    }

    fn examples(&self) -> &'static [Example] {
        EXAMPLES
    }

    fn check_file(&self, ctx: &RuleContext<'_>, sink: &mut Vec<Diagnostic>) {
        for directive in ctx.suppressions.directives() {
            let Some(rule) = directive.rule() else {
                continue; // names no rule at all — `blanket-suppression`'s job
            };
            if is_known_rule(&rule.id) {
                continue;
            }
            sink.push(report(rule));
        }
        for bad in ctx.suppressions.malformed() {
            sink.push(report_malformed(bad));
        }
    }
}

/// A comment that announced itself as a directive and does not parse as one.
///
/// The same silent failure as a misspelled rule ID, one level up: `# arity-format
/// skipp` reads as an ordinary comment and skips nothing.
fn report_malformed(bad: &Malformed) -> Diagnostic {
    let word = &bad.word;
    let prefix = bad.tool.prefix();
    let (body, suggestion) = match bad.kind {
        MalformedKind::UnknownVerb => (
            format!("`{word}` is not an arity directive verb, so this directive does nothing"),
            format!("write one of `skip`, `off`, `on`, `skip-file`: `# {prefix} skip: <reason>`"),
        ),
        MalformedKind::MissingVerb => (
            format!("`{word}` names no verb, so this directive does nothing"),
            format!("say what to do: `# {prefix} skip: <reason>`"),
        ),
        MalformedKind::UnexpectedWord => (
            format!("`{word}` is not something a `# {prefix}` directive can take"),
            "a format directive names no rule, and `# arity` covers every rule; \
write the reason after a `:`"
                .to_string(),
        ),
        MalformedKind::UnexpectedVerb => (
            format!("`{word}` mixes the deprecated `# arity-ignore` with the verb spelling"),
            format!("write one or the other: `# arity-lint {word} <rule>: <reason>`"),
        ),
    };
    Diagnostic {
        rule: "misnamed-suppression",
        severity: Default::default(),
        path: Default::default(),
        range: bad.range,
        message: ViolationData::new("misnamed-suppression", body).with_suggestion(suggestion),
        fix: None,
    }
}

fn report(rule: &RuleRef) -> Diagnostic {
    let id = &rule.id;
    let mut message = ViolationData::new(
        "misnamed-suppression",
        format!("`{id}` is not an arity lint rule, so this directive suppresses nothing"),
    );
    // A comma means the author wrote a list. Naming a single near-match would
    // silently drop the other rules, so explain instead of guessing.
    let is_list = id.contains(',');
    let suggestion = if is_list {
        Some(
            "a directive names one rule; write a separate `# arity-lint skip` per rule".to_string(),
        )
    } else {
        closest_match(id).map(|best| format!("did you mean `{best}`?"))
    };
    if let Some(suggestion) = suggestion {
        message = message.with_suggestion(suggestion);
    }
    Diagnostic {
        rule: "misnamed-suppression",
        severity: Default::default(),
        path: Default::default(),
        range: rule.range,
        message,
        fix: (!is_list).then(|| closest_match(id)).flatten().map(|best| {
            Fix::safe(
                rule.range.start().into(),
                rule.range.end().into(),
                best,
                format!("Replace `{id}` with `{best}`"),
            )
        }),
    }
}

/// The single shipped rule ID that `typo` unambiguously meant, if there is one.
///
/// "Unambiguous" is deliberately strict: the candidate must be within a short
/// edit distance *and* be strictly closer than every other candidate. A tie
/// means we cannot tell, and a guess would suppress a rule the author never
/// named.
fn closest_match(typo: &str) -> Option<&'static str> {
    let max = (typo.chars().count() / 3).clamp(1, 2);
    let mut best: Option<(usize, &'static str)> = None;
    let mut ties = 0usize;
    for id in all_rule_ids() {
        let distance = levenshtein(typo, id, max);
        if distance > max {
            continue;
        }
        match best {
            Some((best_distance, _)) if distance > best_distance => {}
            Some((best_distance, _)) if distance == best_distance => ties += 1,
            _ => {
                best = Some((distance, id));
                ties = 0;
            }
        }
    }
    best.filter(|_| ties == 0).map(|(_, id)| id)
}

/// Levenshtein distance between `a` and `b`, giving up (returning `max + 1`)
/// as soon as every alignment is known to exceed `max`.
fn levenshtein(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != cb);
            cur[j + 1] = substitute.min(prev[j + 1] + 1).min(cur[j] + 1);
            row_min = row_min.min(cur[j + 1]);
        }
        if row_min > max {
            return max + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_counts_edits() {
        assert_eq!(levenshtein("browser", "browser", 2), 0);
        assert_eq!(levenshtein("browsr", "browser", 2), 1);
        assert_eq!(levenshtein("brwsr", "browser", 2), 2);
    }

    #[test]
    fn levenshtein_gives_up_past_the_bound() {
        assert!(levenshtein("zzzzzzzzzz", "browser", 2) > 2);
        assert!(levenshtein("a", "browser", 2) > 2);
    }

    #[test]
    fn closest_match_finds_a_single_near_miss() {
        assert_eq!(closest_match("unusd-binding"), Some("unused-binding"));
        assert_eq!(closest_match("browsr"), Some("browser"));
    }

    #[test]
    fn closest_match_declines_when_nothing_is_close() {
        assert_eq!(closest_match("zzzzzzzzzz"), None);
        assert_eq!(closest_match(""), None);
    }

    #[test]
    fn closest_match_declines_on_a_tie() {
        // `for-loop-index` and `for-loop-dup-index` differ by 4, so no tie
        // there; construct one against the `seq`/`sort` pair, both distance 2
        // from `sor`… guard the general property instead: a typo equidistant
        // from two shipped IDs yields no suggestion.
        let candidates: Vec<&str> = all_rule_ids()
            .into_iter()
            .filter(|id| levenshtein("sorts", id, 2) <= 2)
            .collect();
        if candidates.len() > 1 {
            assert_eq!(closest_match("sorts"), None, "ambiguous: {candidates:?}");
        }
    }
}
