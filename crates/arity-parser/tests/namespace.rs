use arity_parser::namespace::{self, DirectiveKind, Entry};

#[test]
fn classifies_every_supported_directive() {
    let source = include_str!("fixtures/namespace/directives/NAMESPACE");
    let output = namespace::parse(source);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);

    let actual: Vec<_> = output
        .document()
        .directives()
        .map(|directive| directive.kind())
        .collect();
    assert_eq!(
        actual,
        [
            DirectiveKind::Export,
            DirectiveKind::ExportPattern,
            DirectiveKind::ExportClassPattern,
            DirectiveKind::ExportClass,
            DirectiveKind::ExportClasses,
            DirectiveKind::ExportMethods,
            DirectiveKind::Import,
            DirectiveKind::ImportFrom,
            DirectiveKind::ImportClassFrom,
            DirectiveKind::ImportClassesFrom,
            DirectiveKind::ImportMethodsFrom,
            DirectiveKind::UseDynLib,
            DirectiveKind::S3Method,
        ]
    );
}

#[test]
fn exposes_exact_directive_and_argument_ranges() {
    let source = "export(alias = \"original\", `odd name`, c(foo, bar))\n";
    let output = namespace::parse(source);
    let directive = output.document().directives().next().expect("directive");

    assert_eq!(slice(source, directive.text_range()), source.trim_end());
    assert_eq!(directive.name().as_deref(), Some("export"));
    assert_eq!(
        directive.name_token().map(|token| token.text().to_string()),
        Some("export".into())
    );
    assert_eq!(
        directive.l_paren().map(|token| token.text().to_string()),
        Some("(".into())
    );
    assert_eq!(
        directive.r_paren().map(|token| token.text().to_string()),
        Some(")".into())
    );

    let arguments: Vec<_> = directive.arguments().collect();
    assert_eq!(arguments.len(), 3);
    assert_eq!(arguments[0].name().as_deref(), Some("alias"));
    assert_eq!(
        slice(source, arguments[0].text_range()),
        "alias = \"original\""
    );
    assert_eq!(
        arguments[0].value_range().map(|range| slice(source, range)),
        Some("\"original\"")
    );
    assert_eq!(arguments[1].name(), None);
    assert_eq!(slice(source, arguments[1].text_range()), "`odd name`");
    assert_eq!(
        arguments[2].value_range().map(|range| slice(source, range)),
        Some("c(foo, bar)")
    );
}

#[test]
fn traverses_namespace_containers_without_evaluating_conditions() {
    let source = include_str!("fixtures/namespace/conditional/NAMESPACE");
    let output = namespace::parse(source);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);

    let names: Vec<_> = output
        .document()
        .directives()
        .map(|directive| directive.name().unwrap())
        .collect();
    assert_eq!(names, ["export", "export", "useDynLib"]);
}

#[test]
fn reports_but_retains_unsupported_directives() {
    let source = "mystery(foo)\nexport(bar)\n";
    let output = namespace::parse(source);
    let directives: Vec<_> = output.document().directives().collect();

    assert_eq!(directives.len(), 2);
    assert_eq!(directives[0].kind(), DirectiveKind::Unsupported);
    assert_eq!(directives[0].name().as_deref(), Some("mystery"));
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("unsupported NAMESPACE directive")
            && &source[diagnostic.start..diagnostic.end] == "mystery"
    }));
    assert_eq!(directives[1].kind(), DirectiveKind::Export);
}

#[test]
fn does_not_promote_calls_outside_directive_positions() {
    let source = r#"
# export(comment)
mystery("import(in_string)", c(export(nested)))
if (requireNamespace("pkg") && export(in_condition)) import(real)
pkg::export(qualified)
"#;
    let output = namespace::parse(source);
    let directives: Vec<_> = output
        .document()
        .directives()
        .map(|directive| directive.name().unwrap())
        .collect();
    assert_eq!(directives, ["mystery", "import"]);
}

#[test]
fn preserves_crlf_and_roxygen_shaped_comments_as_trivia() {
    let source = "#' export(fake)\r\nexport(real)\r\n";
    let output = namespace::parse(source);

    assert_eq!(namespace::reconstruct(source), source);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let names: Vec<_> = output
        .document()
        .directives()
        .map(|directive| directive.name().unwrap())
        .collect();
    assert_eq!(names, ["export"]);
}

#[test]
fn exposes_malformed_entries_and_recovers_later_directives() {
    let source = "42\nexport(good)\nfor (x in y) export(bad)\nexport(later)\n";
    let output = namespace::parse(source);
    let entries: Vec<_> = output.document().entries().collect();

    assert!(matches!(entries[0], Entry::Malformed(_)));
    assert!(matches!(entries[1], Entry::Directive(_)));
    assert!(matches!(entries[2], Entry::Malformed(_)));
    assert!(matches!(entries[3], Entry::Directive(_)));
    assert!(output.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("expected a NAMESPACE directive")
    }));
    assert_eq!(namespace::reconstruct(source), source);
}

fn slice(source: &str, range: rowan::TextRange) -> &str {
    &source[usize::from(range.start())..usize::from(range.end())]
}
