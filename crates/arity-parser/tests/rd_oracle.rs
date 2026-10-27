//! Pinned R comparisons run without R; the live oracle is explicitly opt-in.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use arity_parser::rd::{self, SyntaxElement, SyntaxKind as K, SyntaxNode};
use serde_json::{Value, json};

fn fixture_names() -> &'static [&'static str] {
    &[
        "sections",
        "markup",
        "mode_boundaries",
        "lists",
        "verbatim",
        "rlike",
        "rlike_escapes",
        "conditionals",
        "escapes",
        "crlf",
        "no_final_newline",
        "cli_ansi_strip",
        "rlang_type_predicates",
    ]
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rd")
}

#[test]
fn rd_matches_pinned_r_trees() {
    for name in fixture_names() {
        let directory = fixtures().join(name);
        let input = fs::read_to_string(directory.join("input.Rd")).unwrap();
        let pin: Value =
            serde_json::from_str(&fs::read_to_string(directory.join("expected.json")).unwrap())
                .unwrap();
        assert!(pin["r_version"].as_str().unwrap().starts_with("R version "));
        let parsed = rd::parse(&input);
        assert!(
            parsed.diagnostics.is_empty(),
            "{name}: {:?}",
            parsed.diagnostics
        );
        assert_eq!(parsed.cst.text().to_string(), input, "losslessness: {name}");
        assert_eq!(project(&parsed.cst, &input), pin["tree"], "oracle: {name}");
    }
}

#[test]
fn bundled_macro_shapes_match_r_without_expansion() {
    let pin: Value =
        serde_json::from_str(&fs::read_to_string(fixtures().join("system-macros.json")).unwrap())
            .unwrap();
    check_system_shapes(&pin);
}

fn check_system_shapes(pin: &Value) {
    use rowan::ast::AstNode;
    let shapes = pin["shapes"].as_array().unwrap();
    assert!(!shapes.is_empty());
    for shape in shapes {
        let name = shape[0].as_str().unwrap();
        let arity = shape[1].as_u64().unwrap() as usize;
        let input = format!(
            "\\description{{\\{name}{}}}",
            "{a \\literal{b}}".repeat(arity)
        );
        let parsed = rd::parse(&input);
        assert!(
            parsed.diagnostics.is_empty(),
            "{name}: {:?}",
            parsed.diagnostics
        );
        assert_eq!(parsed.cst.text().to_string(), input);
        let call = parsed
            .cst
            .descendants()
            .filter_map(rd::Macro::cast)
            .find(|call| call.name() == name)
            .unwrap();
        assert_eq!(call.arguments().count(), arity, "system macro: {name}");
        assert_eq!(
            call.syntax()
                .descendants()
                .filter_map(rd::Macro::cast)
                .count(),
            1,
            "system macro arguments must be verbatim: {name}"
        );
    }
}

#[test]
#[ignore = "needs Rscript; run task rd-oracle"]
fn rd_matches_live_r() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/rd-oracle.R");
    for name in fixture_names() {
        let path = fixtures().join(name).join("input.Rd");
        let output = Command::new("Rscript")
            .arg("--vanilla")
            .arg(&script)
            .arg(&path)
            .output()
            .expect("rd-oracle requires Rscript");
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let oracle: Value = serde_json::from_slice(&output.stdout).unwrap();
        let input = fs::read_to_string(path).unwrap();
        let parsed = rd::parse(&input);
        assert!(
            parsed.diagnostics.is_empty(),
            "{name}: {:?}",
            parsed.diagnostics
        );
        assert_eq!(
            project(&parsed.cst, &input),
            oracle["tree"],
            "live oracle: {name}"
        );
    }
    let output = Command::new("Rscript")
        .arg("--vanilla")
        .arg(&script)
        .arg("--system-macros")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    check_system_shapes(&serde_json::from_slice(&output.stdout).unwrap());
}

// This is an encoding translation of parser-owned structure. It never scans
// source for macros, delimiters, strings, or argument boundaries.
fn project(root: &SyntaxNode, input: &str) -> Value {
    let mut nodes = children(root);
    // parse_Rd reads physical lines and supplies a final newline when absent.
    if !input.ends_with('\n') {
        append(&mut nodes, json!(["TEXT", "\n"]));
    }
    Value::Array(nodes)
}

fn append(nodes: &mut Vec<Value>, node: Value) {
    let tag = node[0].as_str().unwrap();
    if matches!(tag, "TEXT" | "RCODE" | "VERB")
        && nodes.last().is_some_and(|last| last[0] == node[0])
    {
        let last = nodes.last_mut().unwrap();
        last[1] = Value::String(format!(
            "{}{}",
            last[1].as_str().unwrap(),
            node[1].as_str().unwrap()
        ));
    } else {
        nodes.push(node);
    }
}

fn children(node: &SyntaxNode) -> Vec<Value> {
    let mut output = Vec::new();
    for element in node.children_with_tokens() {
        match element {
            SyntaxElement::Node(node) => append(&mut output, branch(&node)),
            SyntaxElement::Token(token) => {
                let (tag, text) = match token.kind() {
                    K::TEXT => ("TEXT", token.text().to_string()),
                    K::RCODE => ("RCODE", token.text().to_string()),
                    K::VERBATIM => ("VERB", token.text().to_string()),
                    K::COMMENT => ("COMMENT", token.text().to_string()),
                    K::ESCAPED_TEXT => ("TEXT", token.text()[1..].to_owned()),
                    K::ESCAPED_RCODE => ("RCODE", token.text()[1..].to_owned()),
                    K::ESCAPED_VERBATIM => ("VERB", token.text()[1..].to_owned()),
                    K::R_STRING_ESCAPE => {
                        let count = token.text().len() - 1;
                        (
                            "RCODE",
                            format!(
                                "{}{}",
                                "\\".repeat((count / 4) * 2 + usize::from(count % 4 != 0)),
                                &token.text()[count..]
                            ),
                        )
                    }
                    _ => continue,
                };
                append(&mut output, json!([tag, text.replace("\r\n", "\n")]));
            }
        }
    }
    output
}

fn branch(node: &SyntaxNode) -> Value {
    if node.kind() == K::CONDITIONAL {
        let directive = node
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == K::DIRECTIVE)
            .unwrap();
        let target = node
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .find(|token| token.kind() == K::TARGET)
            .unwrap();
        return json!([
            directive.text(),
            null,
            [
                ["ARG", null, [["TEXT", target.text().replace("\r\n", "\n")]]],
                ["ARG", null, children(node)]
            ]
        ]);
    }
    if node.kind() == K::GROUP {
        return json!(["LIST", null, children(node)]);
    }
    if node.kind() == K::ARGUMENT {
        return json!(["ARG", null, children(node)]);
    }
    let call = <rd::Macro as rowan::ast::AstNode>::cast(node.clone()).expect("a macro node");
    let arguments: Vec<_> = call.arguments().collect();
    let name = call.name();
    // R flattens single-argument macros but retains wrappers for the grammar's
    // multi-argument forms, including optional second arguments when absent.
    let grouped = matches!(
        name.as_str(),
        "section"
            | "subsection"
            | "enc"
            | "if"
            | "ifelse"
            | "method"
            | "S3method"
            | "S4method"
            | "tabular"
            | "href"
            | "eqn"
            | "deqn"
            | "figure"
    ) || (name == "item" && !arguments.is_empty());
    let body: Vec<Value> = if grouped {
        arguments
            .iter()
            .map(|argument| branch(rowan::ast::AstNode::syntax(argument)))
            .collect()
    } else {
        arguments
            .iter()
            .flat_map(|argument| children(rowan::ast::AstNode::syntax(argument)))
            .collect()
    };
    let option = call.option().map(|option| {
        children(rowan::ast::AstNode::syntax(&option))
            .into_iter()
            .map(|leaf| {
                assert_eq!(leaf[0], "TEXT", "an oracle option must contain text");
                leaf[1].as_str().unwrap().to_owned()
            })
            .collect::<String>()
    });
    json!([format!("\\{name}"), option, body])
}
