//! Post-build canonical and Open Graph URL injector for the arity docs.
//!
//! mdBook has no `canonical-site-url` setting (see
//! <https://github.com/rust-lang/mdBook/pull/2706>), so rendered pages ship
//! without a `<link rel="canonical">`. This tool walks the built book after
//! `mdbook build` and inserts a canonical link into each content page's
//! `<head>`, pointing at the page's public URL under the given base. Run it as:
//!
//! ```text
//! canonical <book-dir> <base-url>
//! ```
//!
//! e.g. `cargo run --example canonical -- docs/book https://arity.cc/`. The
//! canonical URL of each page is derived exactly as the sitemap derives its
//! `<loc>` (both go through `postbuild::collect_pages`), so a page's canonical
//! link and its sitemap entry always agree.
//!
//! Each missing identity tag is inserted independently, so re-running over an
//! already-processed tree is a no-op.

#[path = "util/metadata.rs"]
mod metadata;

#[path = "util/postbuild.rs"]
mod postbuild;

use std::path::Path;

use postbuild::{collect_pages, normalize_base};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(book_dir), Some(base_url)) = (args.next(), args.next()) else {
        eprintln!("usage: canonical <book-dir> <base-url>");
        std::process::exit(1);
    };

    let book_dir = Path::new(&book_dir);
    let base = normalize_base(&base_url);
    let pages = collect_pages(book_dir);

    let mut injected = 0usize;
    for page in &pages {
        let Ok(html) = std::fs::read_to_string(&page.path) else {
            eprintln!("warning: could not read {}", page.path.display());
            continue;
        };
        let url = format!("{base}{}", page.loc);
        let Some(out) = metadata::insert_metadata(&html, &url) else {
            continue;
        };
        if let Err(e) = std::fs::write(&page.path, out) {
            eprintln!("failed to write {}: {e}", page.path.display());
            std::process::exit(1);
        }
        injected += 1;
    }
    eprintln!(
        "injected canonical and Open Graph URLs into {injected}/{} pages",
        pages.len()
    );
}
