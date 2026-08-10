use super::*;

use crate::project::collect_link_literals;

/// The clickable file links in `text`: every string literal that resolves to an
/// existing regular file. Relative spellings resolve against `base_dir` (the
/// document's own directory), matching arity's `source()` resolution; absolute
/// spellings (and any when `base_dir` is `None`) are taken verbatim. A pure CST
/// walk plus one `stat` per literal — no semantic model, no workspace snapshot.
///
/// Documents larger than `file_size_limit` bytes are skipped wholesale so link
/// discovery never walks a pathological generated file. Targets are resolved
/// eagerly (no `documentLink/resolve` round-trip); a resolve provider could be
/// added later if the per-literal `stat` ever shows up in a profile.
pub fn compute_document_links(
    text: &str,
    base_dir: Option<&Path>,
    file_size_limit: u64,
    encoding: PositionEncoding,
) -> Vec<DocumentLink> {
    compute_document_links_in(&TextBuffer::from(text), base_dir, file_size_limit, encoding)
}

/// [`compute_document_links`] against a live buffer, reusing its maintained
/// line index instead of rebuilding one per request.
pub(crate) fn compute_document_links_in(
    buffer: &TextBuffer,
    base_dir: Option<&Path>,
    file_size_limit: u64,
    encoding: PositionEncoding,
) -> Vec<DocumentLink> {
    let text = buffer.text();
    if text.len() as u64 > file_size_limit {
        return Vec::new();
    }
    let root = parse(text).cst;
    let line_index = buffer.line_index();
    collect_link_literals(&root)
        .into_iter()
        .filter_map(|literal| {
            // Resolve exactly as `source_literal_edge` does: join relative
            // spellings onto the base dir, take absolute ones verbatim.
            let target_path = match base_dir {
                Some(dir) if literal.was_relative => dir.join(&literal.spelling),
                _ => PathBuf::from(&literal.spelling),
            };
            // Only existing regular files become links (skip dirs and misses).
            if !target_path.is_file() {
                return None;
            }
            let target = uri::from_path(&target_path)?;
            Some(DocumentLink {
                range: text_range_to_lsp_range(line_index, literal.literal_range, encoding),
                target: Some(target),
                tooltip: None,
                data: None,
            })
        })
        .collect()
}
