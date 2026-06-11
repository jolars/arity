use std::fs;
use std::path::Path;

use air_r_parser::{RParserOptions, parse as air_parse};
use arity::parser::parse;

#[test]
fn air_parser_accepts_arity_parseable_fixtures() {
    for (name, input) in fixture_inputs() {
        let output = parse(&input);
        if !output.diagnostics.is_empty() {
            continue;
        }

        let air = air_parse(&input, RParserOptions::default());
        assert!(
            !air.has_error(),
            "air parser reported error for fixture {name}"
        );
    }
}

fn fixture_inputs() -> Vec<(String, String)> {
    let root = Path::new("tests").join("fixtures").join("parser");
    let entries = fs::read_dir(&root).unwrap_or_else(|err| {
        panic!(
            "failed to read parser fixtures at {}: {err}",
            root.display()
        );
    });
    let mut inputs = Vec::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read fixture entry: {err}"));
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let input_path = entry.path().join("input.R");
        if !input_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&input_path).unwrap_or_else(|err| {
            panic!("failed to read fixture {}: {err}", input_path.display());
        });
        inputs.push((name, text));
    }
    inputs.sort_by(|a, b| a.0.cmp(&b.0));
    inputs
}
