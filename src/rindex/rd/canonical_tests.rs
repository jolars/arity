use super::*;
use rd_ast::{RawRdValue, RdDocument, RdNode, RdTag, producer};

fn section(tag: RdTag, children: Vec<RdNode>) -> RdNode {
    RdNode::tagged(tag, None, children)
}

fn opaque() -> RdNode {
    RdNode::Raw(producer::raw_node(None, None, Vec::new(), None, Vec::new()))
}

fn user_macro(name: &str, definition: &str) -> RdNode {
    let attr = |name: &str, value| {
        producer::raw_attribute(name.into(), producer::raw_object(value, Vec::new()))
    };
    let srcref = producer::raw_attribute(
        "srcref".into(),
        producer::raw_object(
            RawRdValue::Integer(vec![Some(1); 6]),
            vec![
                attr(
                    "srcfile",
                    RawRdValue::Persisted(vec![Some("env::1".into())]),
                ),
                attr("class", RawRdValue::Character(vec![Some("srcref".into())])),
            ],
        ),
    );
    RdNode::Raw(producer::raw_node(
        Some("USERMACRO".into()),
        None,
        vec![RdNode::Text(definition.into())],
        None,
        vec![
            srcref,
            attr("macro", RawRdValue::Character(vec![Some(name.into())])),
        ],
    ))
}

fn cran_pkg() -> Vec<RdNode> {
    vec![
        user_macro(
            r"\CRANpkg",
            r"\href{https://CRAN.R-project.org/package=#1}{\pkg{#1}}stats",
        ),
        section(
            RdTag::Href,
            vec![
                RdNode::group(vec![RdNode::Verb(
                    "https://CRAN.R-project.org/package=stats".into(),
                )]),
                RdNode::group(vec![section(
                    RdTag::Pkg,
                    vec![RdNode::Text("stats".into())],
                )]),
            ],
        ),
    ]
}

#[test]
fn system_macro_preserves_description_and_surrounding_markup() {
    let document = RdDocument::new(vec![section(
        RdTag::Description,
        vec![
            RdNode::Text("Uses ".into()),
            section(RdTag::Emph, cran_pkg()),
            RdNode::Text(" for statistics.".into()),
        ],
    )]);
    let rendered = render_document(&document);
    assert_eq!(
        rendered.sections.description.as_deref(),
        Some("Uses *stats* for statistics.")
    );
    assert!(rendered.issues.is_empty(), "{:?}", rendered.issues);
}

#[test]
fn system_macro_preserves_all_argument_documentation() {
    let document = RdDocument::new(vec![section(
        RdTag::Arguments,
        vec![
            section(
                RdTag::Item,
                vec![
                    RdNode::group(vec![RdNode::Text("x".into())]),
                    RdNode::group(
                        [
                            vec![RdNode::Text("See ".into())],
                            cran_pkg(),
                            vec![RdNode::Text(".".into())],
                        ]
                        .concat(),
                    ),
                ],
            ),
            section(
                RdTag::Item,
                vec![
                    RdNode::group(vec![RdNode::Text("y".into())]),
                    RdNode::group(vec![RdNode::Text("Other values.".into())]),
                ],
            ),
        ],
    )]);
    let rendered = render_document(&document);
    assert_eq!(
        rendered.sections.arguments,
        vec![
            HelpArg {
                name: "x".into(),
                description: "See stats.".into()
            },
            HelpArg {
                name: "y".into(),
                description: "Other values.".into()
            },
        ]
    );
    assert!(rendered.issues.is_empty(), "{:?}", rendered.issues);
}

#[test]
fn system_macro_profiles_render_as_text_without_exposing_expansions() {
    for (nodes, expected) in [
        (cran_pkg(), "stats"),
        (
            vec![
                user_macro(
                    r"\doi",
                    r##"\Sexpr[results=rd]{tools:::Rd_expr_doi("#1")}10.1/x"##,
                ),
                RdNode::tagged(
                    RdTag::Sexpr,
                    Some(vec![RdNode::Text("results=rd".into())]),
                    vec![RdNode::RCode(r#"tools:::Rd_expr_doi("10.1/x")"#.into())],
                ),
            ],
            "10.1/x",
        ),
        (
            vec![
                RdNode::Text("before".into()),
                user_macro(r"\sspace", r"\ifelse{latex}{\out{~}}{ }"),
                section(
                    RdTag::IfElse,
                    vec![
                        RdNode::group(vec![RdNode::Text("latex".into())]),
                        RdNode::group(vec![section(RdTag::Out, vec![RdNode::Verb("~".into())])]),
                        RdNode::group(vec![RdNode::Text(" ".into())]),
                    ],
                ),
                RdNode::Text("after".into()),
            ],
            "before after",
        ),
        (
            vec![section(
                RdTag::I,
                vec![
                    section(RdTag::CranPkg, vec![RdNode::Text("stats".into())]),
                    section(RdTag::Sspace, Vec::new()),
                    section(RdTag::Doi, vec![RdNode::Text("10.1/x".into())]),
                ],
            )],
            "stats 10.1/x",
        ),
    ] {
        let document = RdDocument::new(vec![
            section(RdTag::Title, nodes.clone()),
            section(RdTag::Description, nodes.clone()),
            section(RdTag::Usage, nodes),
        ]);
        let rendered = render_document(&document);
        assert_eq!(rendered.sections.title.as_deref(), Some(expected));
        assert_eq!(rendered.sections.description.as_deref(), Some(expected));
        assert_eq!(rendered.sections.usage.as_deref(), Some(expected));
        assert!(rendered.issues.is_empty(), "{:?}", rendered.issues);
    }
}

#[test]
fn system_macro_body_does_not_hide_opaque_content() {
    let document = RdDocument::new(vec![section(
        RdTag::Description,
        vec![section(RdTag::I, vec![opaque()])],
    )]);
    let rendered = render_document(&document);
    assert!(rendered.sections.description.is_none());
    assert_eq!(rendered.issues.len(), 1);
    assert_eq!(
        rendered.issues[0].path,
        RdAstPath::new(vec![rd_ast::RdAstPathSegment::TopLevel(0)])
            .with_child(0)
            .with_child(0)
    );
}

#[test]
fn unvalidated_system_macro_sequences_remain_opaque() {
    let valid = cran_pkg();
    for invalid in [
        vec![valid[0].clone()],
        vec![valid[0].clone(), RdNode::Text("stats".into())],
        vec![
            user_macro(r"\CRANpkg", "custom definition"),
            valid[1].clone(),
        ],
        vec![user_macro(r"\custom", "stats"), valid[1].clone()],
        vec![valid[0].clone(), opaque()],
    ] {
        let document = RdDocument::new(vec![
            section(RdTag::Title, vec![RdNode::Text("Title".into())]),
            section(RdTag::Description, invalid),
        ]);
        let rendered = render_document(&document);
        assert!(rendered.sections.description.is_none());
        assert_eq!(rendered.sections.title.as_deref(), Some("Title"));
        assert_eq!(rendered.issues.len(), 1);
        assert_eq!(rendered.issues[0].field, "description");
        assert_eq!(
            rendered.issues[0].path,
            RdAstPath::new(vec![rd_ast::RdAstPathSegment::TopLevel(1)]).with_child(0)
        );
    }
}

#[test]
fn canonical_help_matches_all_existing_magrittr_pages() {
    let pkg =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rindex/magrittr");
    let shared = rd_helpdb::PackageHelpDb::open(&pkg).unwrap();
    let legacy = crate::rindex::lazyload::LazyLoadDb::open(&pkg.join("help/magrittr.rdx")).unwrap();
    let mut count = 0;
    for name in shared.topics() {
        let document = rd_ast::lower_r_object(&shared.raw_topic(name).unwrap()).unwrap();
        let rendered = render_document(&document);
        assert!(rendered.issues.is_empty(), "{name}: {:?}", rendered.issues);
        assert_eq!(
            rendered.sections,
            render_page(&legacy.fetch(name).unwrap()),
            "{name}"
        );
        count += 1;
    }
    assert_eq!(count, 15);
}

#[test]
fn canonical_repeated_sections_and_inline_markup_preserve_presentation() {
    let document = RdDocument::new(vec![
        section(RdTag::Description, vec![RdNode::Text("old".into())]),
        section(
            RdTag::Description,
            vec![
                section(
                    RdTag::Unknown("\\future".into()),
                    vec![RdNode::Text("Keep ".into())],
                ),
                section(RdTag::Code, vec![RdNode::RCode("x  + y".into())]),
                RdNode::Text("\n\nNext ".into()),
                section(RdTag::Emph, vec![RdNode::Text("word".into())]),
            ],
        ),
        section(RdTag::Usage, vec![RdNode::RCode("\nf(x,  y)\n".into())]),
        section(
            RdTag::Arguments,
            vec![section(
                RdTag::Item,
                vec![
                    RdNode::group(vec![RdNode::Text("x, y".into())]),
                    RdNode::group(vec![RdNode::Text("Together.".into())]),
                ],
            )],
        ),
        section(
            RdTag::Arguments,
            vec![section(
                RdTag::Item,
                vec![
                    RdNode::group(vec![RdNode::Text("z".into())]),
                    RdNode::group(vec![RdNode::Text("Last.".into())]),
                ],
            )],
        ),
    ]);
    let rendered = render_document(&document);
    assert!(rendered.issues.is_empty());
    assert_eq!(
        rendered.sections.description.as_deref(),
        Some("Keep `x + y`\n\nNext *word*")
    );
    assert_eq!(rendered.sections.usage.as_deref(), Some("f(x,  y)"));
    assert_eq!(
        rendered
            .sections
            .arguments
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["x, y", "z"]
    );
}

#[test]
fn opaque_content_omits_only_affected_fields() {
    let document = RdDocument::new(vec![
        section(RdTag::Title, vec![RdNode::Text("Title".into())]),
        section(
            RdTag::Description,
            vec![
                RdNode::Text("before".into()),
                opaque(),
                RdNode::Text("after".into()),
            ],
        ),
        section(RdTag::Usage, vec![RdNode::RCode("f()".into())]),
        section(RdTag::Arguments, vec![opaque()]),
    ]);
    let rendered = render_document(&document);
    assert_eq!(rendered.sections.title.as_deref(), Some("Title"));
    assert_eq!(rendered.sections.usage.as_deref(), Some("f()"));
    assert!(rendered.sections.description.is_none());
    assert!(rendered.sections.arguments.is_empty());
    assert_eq!(rendered.issues.len(), 2);
}

#[test]
fn opaque_top_level_section_cannot_silently_keep_an_earlier_value() {
    let document = RdDocument::new(vec![
        section(RdTag::Description, vec![RdNode::Text("old".into())]),
        RdNode::Raw(producer::raw_node(
            Some("\\description".into()),
            None,
            Vec::new(),
            None,
            Vec::new(),
        )),
    ]);
    let rendered = render_document(&document);
    assert!(rendered.sections.description.is_none());
    assert_eq!(rendered.issues.len(), 1);
}

#[test]
fn legacy_renderer_keeps_its_tolerance_of_untyped_text() {
    let page = Robj {
        kind: Rkind::List(vec![Robj {
            kind: Rkind::List(vec![Robj {
                kind: Rkind::Str(vec![Some("kept".into())]),
                attr: vec![],
            }]),
            attr: vec![(
                "Rd_tag".into(),
                Robj {
                    kind: Rkind::Str(vec![Some("\\description".into())]),
                    attr: vec![],
                },
            )],
        }]),
        attr: vec![],
    };
    assert_eq!(render_page(&page).description.as_deref(), Some("kept"));
}
