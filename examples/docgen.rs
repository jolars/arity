//! Generate the mdBook rule-reference pages from rule metadata.
//!
//! Run with `cargo run --example docgen`. For each rule that carries examples,
//! this renders the same markdown the snapshot test pins
//! ([`arity::linter::docs::render_rule_doc`]) and writes it under the mdBook
//! source tree. It also stamps the crate version into `version.md`, included by
//! the introduction page.
//!
//! Living as an `examples/` target (not a `[[bin]]`) keeps arity a single,
//! publishable crate: `examples/` is outside the Cargo `include` whitelist, so
//! this never ships to crates.io.

use std::fs;
use std::io;
use std::path::Path;

use arity::linter::docs::render_rule_doc;
use arity::linter::rules::all_rules;

fn main() -> io::Result<()> {
    let rules_dir = Path::new("book/src/reference/rules");
    fs::create_dir_all(rules_dir)?;

    for rule in all_rules() {
        if rule.examples().is_empty() {
            continue;
        }
        let page = render_rule_doc(rule.as_ref());
        let path = rules_dir.join(format!("{}.md", rule.id()));
        write_if_changed(&path, &page)?;
    }

    // Version stamp, included by the introduction page.
    let version = format!("arity v{}\n", env!("CARGO_PKG_VERSION"));
    write_if_changed(Path::new("book/src/version.md"), &version)?;

    Ok(())
}

/// Write `content` to `path` only when it differs from what's already there, so
/// re-running the generator leaves unchanged files (and their mtimes) alone.
fn write_if_changed(path: &Path, content: &str) -> io::Result<()> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    fs::write(path, content)?;
    println!("wrote {}", path.display());
    Ok(())
}
