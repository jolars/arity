//! Living-documentation tests: the rule reference is rendered from the rule
//! metadata by running the real linter, and pinned by snapshot so the docs
//! cannot drift from behavior. The generator (`examples/docgen.rs`) assembles
//! the same `render_rule_doc` sections into the single `reference/rules.md`
//! page in the mdBook source tree.

use arity::linter::docs::{lint_example, render_rule_doc, render_rules_page};
use arity::linter::rules::{all_rules, rules_by_category};

/// Pin the rendered reference section for every documented rule. Any change to
/// a rule's diagnostic or fix that alters its section fails here before the
/// docs go stale.
#[test]
fn rule_docs_render() {
    for rule in all_rules() {
        if rule.examples().is_empty() {
            continue;
        }
        insta::assert_snapshot!(rule.id().replace('-', "_"), render_rule_doc(&rule));
    }
}

/// The assembled page carries every rule: a linked index entry and the rule's
/// own section, under its category heading. Guards the generator against a rule
/// that is registered but silently missing from the reference.
#[test]
fn rules_page_covers_every_rule() {
    let page = render_rules_page();

    for (category, rules) in rules_by_category() {
        assert!(
            page.contains(&format!("\n## {}\n", category.title())),
            "rule reference has no `{}` section",
            category.title(),
        );
        for rule in rules {
            let id = rule.id();
            assert!(
                page.contains(&format!("\n### `{id}`\n")),
                "rule reference has no section for `{id}`",
            );
            assert!(
                page.contains(&format!("- [`{id}`](#{id})")),
                "rule reference index has no entry for `{id}`",
            );
        }
    }
}

/// The page's sections appear in registry order, so the index reads top to
/// bottom like the body it links into.
#[test]
fn rules_page_sections_follow_registry_order() {
    let page = render_rules_page();
    let mut previous = 0;

    for rule in all_rules() {
        let heading = format!("\n### `{}`\n", rule.id());
        let at = page
            .find(&heading)
            .unwrap_or_else(|| panic!("rule reference has no section for `{}`", rule.id()));
        assert!(
            at > previous,
            "rule `{}` is out of registry order in the reference",
            rule.id(),
        );
        previous = at;
    }
}

/// Every shipped rule must carry a description and at least one example, so the
/// generated reference is complete.
#[test]
fn every_rule_is_documented() {
    for rule in all_rules() {
        assert!(
            !rule.description().trim().is_empty(),
            "rule `{}` has no description",
            rule.id(),
        );
        assert!(
            !rule.examples().is_empty(),
            "rule `{}` has no examples",
            rule.id(),
        );
    }
}

/// Every documented example must actually produce a finding of its own rule —
/// guards against a snippet that looks plausible but no longer triggers.
#[test]
fn documented_examples_actually_trigger() {
    for rule in all_rules() {
        for example in rule.examples() {
            // The same call the reference page makes, so this check cannot
            // drift from what is actually rendered.
            let diagnostics = lint_example(&rule, example);
            assert!(
                diagnostics.iter().any(|d| d.rule == rule.id()),
                "example for rule `{}` produced no finding of that rule:\n{}",
                rule.id(),
                example.source,
            );
        }
    }
}
