//! Generate the mdBook rule reference from rule metadata.
//!
//! Run with `cargo run --example docgen`. This renders the same markdown the
//! snapshot test pins ([`arity::linter::docs::render_rules_page`]) — every
//! documented rule's section plus the index over them — and writes it as the
//! single `reference/rules.md` page in the mdBook source tree. It also stamps
//! the crate version into `version.md`, included by the introduction page.
//!
//! Living as an `examples/` target (not a `[[bin]]`) keeps arity a single,
//! publishable crate: `examples/` is outside the Cargo `include` whitelist, so
//! this never ships to crates.io.

use std::fs;
use std::io;
use std::path::Path;

use arity::bench_docs::render_partials;
use arity::linter::docs::render_rules_page;

fn main() -> io::Result<()> {
    write_if_changed(
        Path::new("docs/src/reference/rules.md"),
        &render_rules_page(),
    )?;

    // Version stamp, included by the introduction page.
    let version = format!("arity v{}\n", env!("CARGO_PKG_VERSION"));
    write_if_changed(Path::new("docs/src/version.md"), &version)?;

    generate_benchmarks()?;

    Ok(())
}

/// Render the benchmark partials included by `guide/performance.md` from the
/// committed artifact `benches/benchmark_results.json`. The JSON is read but
/// never regenerated here, so the benchmark is only ever run manually (via
/// `task bench`), not at doc-gen time. A missing artifact degrades to an
/// "unavailable" note so a fresh checkout still builds.
fn generate_benchmarks() -> io::Result<()> {
    let guide_dir = Path::new("docs/src/guide");
    let json = fs::read_to_string("benches/benchmark_results.json").ok();
    let (meta, results) = render_partials(json.as_deref());
    write_if_changed(&guide_dir.join("benchmarks_meta.md"), &meta)?;
    write_if_changed(&guide_dir.join("benchmarks_results.md"), &results)?;
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
