//! R's quoted Collate lists are grammar, shared by lint and project facts.

use arity_parser::dcf;

#[test]
fn collate_entries_preserve_order_duplicates_and_source_ranges() {
    let source = "Collate: 'a b.R'\r\n# comment\r\n    \"c.R\" 'a b.R'\r\n";
    let parsed = dcf::parse(source);
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(dcf::reconstruct(source), source);
    let entries = dcf::collate::collate_entries(&parsed.document().field("Collate").unwrap());
    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["a b.R", "c.R", "a b.R"]);
    let spans: Vec<_> = entries.iter().map(|entry| &source[entry.range]).collect();
    assert_eq!(spans, ["'a b.R'", "\"c.R\"", "'a b.R'"]);
}

#[test]
fn collate_uses_scan_quotes_and_preserves_literal_backslashes() {
    for (value, expected) in [
        ("'a\\'b.R'", vec!["a'b.R"]),
        ("'a\\b.R'", vec!["a\\b.R"]),
        ("a'b.R", vec!["a'b.R"]),
        ("'a.R'\"b.R\"", vec!["a.R", "b.R"]),
        ("'a.R", vec!["a.R\n"]),
        ("'a\\", vec!["a\\\n"]),
        ("''", vec![]),
        ("'' a.R", vec!["", "a.R"]),
        ("a.R ''", vec!["a.R", ""]),
        ("'a\n    b.R'", vec!["a b.R"]),
        ("a#b.R", vec!["a#b.R"]),
        ("a.R\tb.R", vec!["a.R", "b.R"]),
        ("", vec![]),
    ] {
        let source = format!("Collate: {value}\n");
        let parsed = dcf::parse(&source);
        assert!(parsed.diagnostics.is_empty(), "{source}");
        assert_eq!(dcf::reconstruct(&source), source);
        let entries = dcf::collate::collate_entries(&parsed.document().field("Collate").unwrap());
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, expected, "{source}");
    }
}
