/// Add page identity to mdBook's head without duplicating existing metadata.
pub fn insert_metadata(html: &str, canonical_url: &str) -> Option<String> {
    let pos = html.find("</head>")?;
    let href = canonical_url.replace('&', "&amp;").replace('"', "&quot;");
    let mut tags = String::new();
    if !html.contains("rel=\"canonical\"") {
        tags.push_str(&format!("    <link rel=\"canonical\" href=\"{href}\">\n"));
    }
    if !html.contains("property=\"og:url\"") {
        tags.push_str(&format!(
            "    <meta property=\"og:url\" content=\"{href}\">\n"
        ));
    }
    if tags.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(html.len() + tags.len());
    out.push_str(&html[..pos]);
    out.push_str(&tags);
    out.push_str(&html[pos..]);
    Some(out)
}
