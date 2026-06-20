//! Living-documentation tests: rule reference pages are rendered from the rule
//! metadata by running the real linter, and pinned by snapshot so the docs
//! cannot drift from behavior. The generator (`examples/docgen.rs`) writes the
//! same `render_rule_doc` output to the mdBook source tree.

use std::path::Path;

use arity::config::LintConfig;
use arity::linter::check_document;
use arity::linter::docs::render_rule_doc;
use arity::linter::rules::{Rule, all_rules};

fn rule(id: &str) -> Box<dyn Rule> {
    all_rules()
        .into_iter()
        .find(|r| r.id() == id)
        .unwrap_or_else(|| panic!("no rule with id `{id}`"))
}

/// Pin the rendered reference page for `true-false-symbol`. Any change to the
/// rule's diagnostic or fix that alters the rendered page fails here before the
/// docs go stale.
#[test]
fn true_false_symbol_doc_renders() {
    insta::assert_snapshot!(
        "true_false_symbol",
        render_rule_doc(rule("true-false-symbol").as_ref())
    );
}

/// Every documented example must actually produce a finding of its own rule —
/// guards against a snippet that looks plausible but no longer triggers.
#[test]
fn documented_examples_actually_trigger() {
    for r in all_rules() {
        for example in r.examples() {
            let config = LintConfig {
                select: Some(vec![r.id().to_string()]),
                ..Default::default()
            };
            let diagnostics = check_document(Path::new("example.R"), example.source, &config)
                .expect("linting a documented example should not error");
            assert!(
                diagnostics.iter().any(|d| d.rule == r.id()),
                "example for rule `{}` produced no finding of that rule:\n{}",
                r.id(),
                example.source,
            );
        }
    }
}
