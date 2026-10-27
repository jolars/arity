use arity_parser::rd::{self, SyntaxKind};
use rowan::ast::AstNode;

#[test]
fn ordinary_help_page_has_typed_macros_and_lossless_source() {
    let input = "\\name{foo}\r\n\\title{A title}\r\n\\description{Use \\link[pkg]{foo} and \\emph{care}.}\r\n\\usage{foo(x = \"}\")}\r\n\\arguments{\\item{x}{Input.}}\r\n\\examples{foo(1)}";
    let parsed = rd::parse(input);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(rd::reconstruct(input), input);
    let macros: Vec<_> = parsed.document().macros().collect();
    assert_eq!(macros[0].name(), "name");
    assert_eq!(macros[0].arguments().count(), 1);
    let link = parsed
        .cst
        .descendants()
        .filter_map(rd::Macro::cast)
        .find(|node| node.name() == "link")
        .unwrap();
    assert_eq!(link.option().unwrap().text(), "pkg");
    assert!(
        parsed
            .cst
            .descendants_with_tokens()
            .any(|element| element.kind() == SyntaxKind::RCODE)
    );
}

#[test]
fn malformed_rd_recovers_without_losing_bytes() {
    for input in [
        "}",
        "\\title{unfinished",
        "\\title",
        "\\description{\\link[pkg{foo}}",
        "#endif\n",
        "#ifdef unix\n\\title{x}",
        "\\examples{\"unfinished}",
        "\\emph{x}",
        "{\\title{x}}",
        "\\description{\\RdOpts{echo=FALSE}}",
    ] {
        let parsed = rd::parse(input);
        assert!(
            !parsed.diagnostics.is_empty(),
            "missing diagnostic: {input:?}"
        );
        assert_eq!(rd::reconstruct(input), input);
        for diagnostic in parsed.diagnostics {
            assert!(diagnostic.start <= diagnostic.end && diagnostic.end <= input.len());
            assert!(input.is_char_boundary(diagnostic.start));
            assert!(input.is_char_boundary(diagnostic.end));
        }
    }
}

#[test]
fn unknown_and_unexpanded_macros_are_preserved_quietly() {
    let input = "\\newcommand{\\custom}{\\emph{#1}}\n\\description{\\custom{hello} \\CRANpkg{cli} \\doi{10.1/x} \\Sexpr[results=rd]{stop(\"never run\")}}";
    let parsed = rd::parse(input);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(rd::reconstruct(input), input);
    let macros: Vec<_> = parsed
        .cst
        .descendants()
        .filter_map(rd::Macro::cast)
        .collect();
    assert_eq!(
        macros
            .iter()
            .find(|node| node.name() == "CRANpkg")
            .unwrap()
            .arguments()
            .count(),
        1
    );
    assert_eq!(
        macros
            .iter()
            .find(|node| node.name() == "custom")
            .unwrap()
            .arguments()
            .count(),
        0
    );
}

#[test]
fn empty_tiny_and_deep_inputs_are_total_and_lossless() {
    for input in [
        "",
        "\\",
        "%",
        "é",
        "\r\n",
        "{",
        "[",
        "\\description{\\unknown123{x}}",
    ] {
        assert_eq!(rd::reconstruct(input), input);
    }
    let deep = format!("\\description{{{}{}}}", "{".repeat(1000), "}".repeat(1000));
    let parsed = rd::parse(&deep);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("nesting limit"))
    );
    assert_eq!(parsed.cst.text().to_string(), deep);
}

#[test]
fn conditional_wrappers_retain_their_source_and_targets() {
    let input = "#ifdef unix\r\n\\title{Unix}\r\n#endif\r\n";
    let parsed = rd::parse(input);
    assert!(parsed.diagnostics.is_empty());
    let conditional = parsed
        .cst
        .children()
        .find_map(rd::Conditional::cast)
        .unwrap();
    assert_eq!(conditional.directive().unwrap().text(), "#ifdef");
    assert_eq!(conditional.target().unwrap().text(), " unix\r\n");
    assert_eq!(conditional.syntax().text().to_string(), input);
}
