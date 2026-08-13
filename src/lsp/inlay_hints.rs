//! `textDocument/inlayHint`, currently a `DESCRIPTION`-only feature: the
//! installed version of each declared dependency, rendered after the entry.
//!
//! R argument-name hints are a separate, unimplemented feature (`TODO.md`); when
//! they land, the R arm attaches here, and the main loop's early decline in
//! `on_inlay_hint` is what goes away.

use super::*;

/// Resolve inlay hints for the document at `path` off the db snapshot, which is
/// what carries the harvested package index. `None` for a grammar with no hints
/// to offer — the client sees `null` and asks again after its next edit.
///
/// `visible` is the request's range verbatim; only hints anchored inside it are
/// computed.
pub(crate) fn inlay_hints_via_db(
    snapshot: &Analysis,
    path: &Path,
    buffer: &TextBuffer,
    visible: Range,
    encoding: PositionEncoding,
) -> Option<Vec<InlayHint>> {
    if DocumentKind::from_path(path) != DocumentKind::Description {
        return None;
    }
    // A cancel means the lint thread is writing; an empty index yields no hints,
    // and the refresh that follows the write asks the client to come back. The
    // read *must* return, since this request fires on every scroll: a `Cancelled`
    // unwinding out of here would leave the reply unsent and the id in flight.
    let index = salsa::Cancelled::catch(AssertUnwindSafe(|| snapshot.library_data()))
        .unwrap_or_default()
        .unwrap_or_default();
    // No `SourceFile` lookup: a `DESCRIPTION` has no cached parse in the db, and
    // its DCF parse is kilobytes (`hover_via_db` reasons the same way).
    Some(compute_description_inlay_hints(
        buffer.text(),
        visible,
        &index,
        buffer.line_index(),
        encoding,
    ))
}
