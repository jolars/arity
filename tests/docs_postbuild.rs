#[path = "../examples/util/postbuild.rs"]
mod postbuild;

#[path = "../examples/util/metadata.rs"]
mod metadata;

#[test]
fn page_identity_is_escaped_and_idempotent() {
    let html = "<html><head><title>Chapter</title></head><body>Content</body></html>";
    let result = metadata::insert_metadata(html, "https://example.org/?a=1&b=\"two\"").unwrap();
    let escaped = "https://example.org/?a=1&amp;b=&quot;two&quot;";
    assert!(result.contains(&format!("rel=\"canonical\" href=\"{escaped}\"")));
    assert!(result.contains(&format!("property=\"og:url\" content=\"{escaped}\"")));
    assert!(result.ends_with("</head><body>Content</body></html>"));
    assert!(metadata::insert_metadata(&result, "https://example.org/").is_none());
    assert!(metadata::insert_metadata("no head", "https://example.org/").is_none());
}

#[test]
fn existing_canonical_does_not_prevent_adding_open_graph_url() {
    let html = r#"<head><link rel="canonical" href="https://arity.cc/"></head>"#;
    let result = metadata::insert_metadata(html, "https://arity.cc/").unwrap();
    assert_eq!(result.matches("rel=\"canonical\"").count(), 1);
    assert_eq!(result.matches("property=\"og:url\"").count(), 1);
}

#[test]
fn homepage_alias_shares_the_root_canonical_without_dropping_the_file() {
    let dir = tempfile::tempdir().unwrap();
    for path in ["index.html", "introduction.html", "guide/introduction.html"] {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "<html><head></head><body>Content</body></html>").unwrap();
    }
    std::fs::write(dir.path().join("print.html"), "<html></html>").unwrap();
    std::fs::write(
        dir.path().join("old.html"),
        r#"<meta http-equiv="refresh" content="0; URL=introduction.html">"#,
    )
    .unwrap();
    let pages = postbuild::collect_pages(dir.path());
    assert_eq!(pages.len(), 3);
    let canonical = |name: &str| {
        pages
            .iter()
            .find(|page| page.path == dir.path().join(name))
            .unwrap()
            .loc
            .as_str()
    };
    assert_eq!(canonical("index.html"), "");
    assert_eq!(canonical("introduction.html"), "");
    assert_eq!(
        canonical("guide/introduction.html"),
        "guide/introduction.html"
    );
    assert_eq!(
        postbuild::normalize_base("https://arity.cc///"),
        "https://arity.cc/"
    );
}
