use std::{fs, path::Path};

use arity_parser::namespace::{self, Entry};
use insta::assert_snapshot;

#[test]
fn namespace_fixtures_are_lossless_and_snapshot_the_typed_surface() {
    for name in fixture_names() {
        let input = fixture_input(name);
        let output = namespace::parse(&input);

        assert_snapshot!(
            format!("{name}_surface"),
            format_surface(&input, &output.document())
        );
        assert_snapshot!(
            format!("{name}_diagnostics"),
            format!("{:#?}", output.diagnostics)
        );
        assert_eq!(namespace::reconstruct(&input), input, "{name}");
    }
}

fn format_surface(input: &str, document: &namespace::Document) -> String {
    let mut out = String::new();
    for entry in document.entries() {
        match entry {
            Entry::Directive(directive) => {
                let range = directive.text_range();
                out.push_str(&format!(
                    "directive {:?} {:?} {}..{} {:?}\n",
                    directive.kind(),
                    directive.name(),
                    u32::from(range.start()),
                    u32::from(range.end()),
                    slice(input, range),
                ));
                for argument in directive.arguments() {
                    let range = argument.text_range();
                    let value = argument.value_range().map(|range| slice(input, range));
                    out.push_str(&format!(
                        "  argument {:?} {}..{} {:?} value={value:?}\n",
                        argument.name(),
                        u32::from(range.start()),
                        u32::from(range.end()),
                        slice(input, range),
                    ));
                }
            }
            Entry::Malformed(malformed) => {
                let range = malformed.text_range();
                out.push_str(&format!(
                    "malformed {}..{} {:?}\n",
                    u32::from(range.start()),
                    u32::from(range.end()),
                    slice(input, range),
                ));
            }
        }
    }
    out
}

fn slice(input: &str, range: rowan::TextRange) -> &str {
    &input[usize::from(range.start())..usize::from(range.end())]
}

fn fixture_input(name: &str) -> String {
    let path = Path::new("tests")
        .join("fixtures")
        .join("namespace")
        .join(name)
        .join("NAMESPACE");
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()))
}

fn fixture_names() -> &'static [&'static str] {
    &["directives", "conditional", "unsupported", "malformed"]
}
