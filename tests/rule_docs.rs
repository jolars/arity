//! Living-documentation tests: rule reference pages are rendered from the rule
//! metadata by running the real linter, and pinned by snapshot so the docs
//! cannot drift from behavior. The generator (`examples/docgen.rs`) writes the
//! same `render_rule_doc` output to the mdBook source tree.

use std::path::Path;

use arity::config::LintConfig;
use arity::linter::check_document;
use arity::linter::docs::render_rule_doc;
use arity::linter::rules::all_rules;

/// Pin the rendered reference page for every documented rule. Any change to a
/// rule's diagnostic or fix that alters its page fails here before the docs go
/// stale.
#[test]
fn rule_docs_render() {
    for rule in all_rules() {
        if rule.examples().is_empty() {
            continue;
        }
        insta::assert_snapshot!(rule.id().replace('-', "_"), render_rule_doc(rule.as_ref()));
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
            let config = LintConfig {
                select: Some(vec![rule.id().to_string()]),
                ..Default::default()
            };
            let diagnostics = check_document(Path::new("example.R"), example.source, &config)
                .expect("linting a documented example should not error");
            assert!(
                diagnostics.iter().any(|d| d.rule == rule.id()),
                "example for rule `{}` produced no finding of that rule:\n{}",
                rule.id(),
                example.source,
            );
        }
    }
}
