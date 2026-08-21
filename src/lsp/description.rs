//! Language features for an open `DESCRIPTION`: completion of package names in
//! dependency fields, and hover over a declared dependency.
//!
//! Everything here works off a fresh [`dcf::parse`] rather than a salsa query.
//! There is no DCF parse query, deliberately: `DescriptionFile` holds text only
//! and `description_facts` is range-free, because a red `rowan` tree is neither
//! `Send` nor `Eq` and has no business in the database (`dcf/ast.rs`). A
//! `DESCRIPTION` is a few kilobytes scanned line by line, so the parse is not
//! worth caching anyway.
//!
//! **Every range reported here is a source range**, taken from the CST via
//! [`dependency_entries`]. Never compute one from `folded_value()`: the folded
//! text drops the continuation indents, so its offsets do not index the buffer
//! — and a one-package-per-line `Imports` is the canonical style, not an edge
//! case.

use super::*;

use crate::dcf::{dependency_entries, is_dependency_field};

/// Where the cursor is, for the purposes of completing or hovering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DescriptionContext {
    /// Inside a dependency field, where a package name goes.
    DependencyName {
        /// The field it sits in. `Depends` alone may also take `R`.
        field: SmolStr,
        /// Text typed so far, which the candidate list filters on.
        prefix: String,
        /// The **source** range a completion should overwrite. Empty when the
        /// cursor is on fresh ground (after a comma, or on a blank
        /// continuation line).
        replace: TextRange,
        /// Whether the cursor sits inside an existing entry's name, as opposed
        /// to on fresh ground. Hover requires this; completion does not.
        on_entry: bool,
        /// Every package already named across the five dependency fields,
        /// excluding the one under the cursor. Offering one of these would
        /// invite a duplicate declaration.
        declared: Vec<SmolStr>,
    },
    /// Nowhere interesting: prose, a field name, a comment, or inside a version
    /// constraint.
    None,
}

/// Classify `offset` in `text`.
pub(crate) fn classify(text: &str, offset: usize) -> DescriptionContext {
    let document = crate::dcf::parse(text).document();
    let at = TextSize::new(offset as u32);

    let Some(field) = document
        .fields()
        .find(|f| f.syntax().text_range().contains_inclusive(at))
    else {
        return DescriptionContext::None;
    };
    // On the field name or its colon: nothing to complete. Documenting a field
    // itself is a different feature with a different candidate list.
    if at <= field.name_range().end() || !is_dependency_field(&field.name()) {
        return DescriptionContext::None;
    }
    let value = field.value_range();
    if !value.contains_inclusive(at) {
        return DescriptionContext::None;
    }
    // A comment line is a child of the field it follows, so it lands inside the
    // value range without being part of the value.
    if line_of(text, offset).trim_start().starts_with('#') {
        return DescriptionContext::None;
    }

    let entries = dependency_entries(&field);
    let on_entry = entries
        .iter()
        .find(|e| e.name_range.contains_inclusive(at))
        .map(|e| (e.name_range, e.name.clone()));

    let (replace, prefix, on_entry) = match on_entry {
        Some((range, _)) => {
            let start: usize = range.start().into();
            (range, text[start..offset].to_string(), true)
        }
        None => {
            // Fresh ground. Walk back to the comma that opens this entry; if the
            // walk crosses an unclosed `(` the cursor is inside a version
            // constraint, where a package name is not what comes next.
            let Some(start) = entry_start(text, value, offset) else {
                return DescriptionContext::None;
            };
            (
                TextRange::new(TextSize::new(start as u32), at),
                text[start..offset].to_string(),
                false,
            )
        }
    };

    // A prefix with a space or a paren in it means the cursor trails a complete
    // entry (`dplyr |`), where the next token is a constraint, not a package.
    if prefix.contains(|c: char| c.is_whitespace() || c == '(') {
        return DescriptionContext::None;
    }

    DescriptionContext::DependencyName {
        field: field.name(),
        prefix,
        replace,
        on_entry,
        declared: declared_elsewhere(&document, replace),
    }
}

/// The physical line containing `offset`.
fn line_of(text: &str, offset: usize) -> &str {
    let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    &text[start..end]
}

/// Where the entry under `offset` begins: just past the previous top-level
/// comma, with leading whitespace and continuation indent skipped. `None` when
/// the walk crosses an unclosed `(` — the cursor is inside a version
/// constraint.
fn entry_start(text: &str, value: TextRange, offset: usize) -> Option<usize> {
    let lo: usize = value.start().into();
    let mut depth = 0usize;
    let mut start = lo;
    for (i, c) in text[lo..offset]
        .char_indices()
        .rev()
        .map(|(i, c)| (lo + i, c))
    {
        match c {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                start = i + 1;
                break;
            }
            _ => {}
        }
    }
    // Skip forward over the whitespace and continuation indent between the
    // comma and the name.
    let skipped = text[start..offset]
        .find(|c: char| !c.is_whitespace())
        .map_or(offset, |i| start + i);
    Some(skipped)
}

/// Every package named in a dependency field, except the entry `replace`
/// covers — that one is what the user is editing.
fn declared_elsewhere(document: &crate::dcf::Document, replace: TextRange) -> Vec<SmolStr> {
    let mut names = Vec::new();
    for field in document.fields() {
        if !is_dependency_field(&field.name()) {
            continue;
        }
        for entry in dependency_entries(&field) {
            if entry.name_range == replace {
                continue;
            }
            names.push(entry.name.clone());
        }
    }
    names
}

/// Where a candidate came from, lowest first — the same sort-group convention
/// the R completion path uses (`completion.rs`), so the two read alike.
const GROUP_INSTALLED: u8 = 0;
const GROUP_BASE: u8 = 1;
const GROUP_BUNDLED: u8 = 2;
const GROUP_REMOTE: u8 = 3;

struct Candidate {
    name: SmolStr,
    group: u8,
    /// Installed version, when locally harvested.
    version: Option<SmolStr>,
    /// The package's own `Title`, when locally harvested.
    title: Option<SmolStr>,
}

/// Complete a package name in a dependency field.
///
/// Tiers, in precedence order: locally harvested packages (which alone can show
/// a version and a `Title`), the base packages, the bundled CRAN list, then the
/// remote sidecar. A name present in several tiers keeps its best group.
pub fn compute_description_completions(
    text: &str,
    offset: usize,
    indexed: &IndexedProvider,
    remote: &RemoteExports,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Option<CompletionResponse> {
    let DescriptionContext::DependencyName {
        field,
        prefix,
        replace,
        declared,
        ..
    } = classify(text, offset)
    else {
        return None;
    };

    let mut candidates: Vec<Candidate> = Vec::new();
    for name in indexed.packages() {
        // Peeks only: this labels *every* harvested package at once, and under
        // the lazy provider a filling accessor here would synchronously
        // deserialize the whole cache on the read pool and pin it resident
        // (issue #116). The version needs no read; the `Title` is shown when
        // its package happens to be resident, and `completionItem/resolve`
        // reads the one chosen package for the full card.
        candidates.push(Candidate {
            name: name.clone(),
            group: GROUP_INSTALLED,
            version: indexed.version(name).cloned(),
            title: indexed
                .package_if_resident(name)
                .and_then(|i| i.title.clone()),
        });
    }
    // `R` is the language, not a package, and only `Depends` may name it.
    if field == "Depends" {
        candidates.push(Candidate {
            name: SmolStr::new("R"),
            group: GROUP_INSTALLED,
            version: None,
            title: None,
        });
    }
    for name in crate::semantic::symbols::base_priority_packages() {
        candidates.push(Candidate {
            name: SmolStr::new(*name),
            group: GROUP_BASE,
            version: None,
            title: None,
        });
    }
    for name in crate::rindex::provider::bundled_packages() {
        candidates.push(Candidate {
            name: name.clone(),
            group: GROUP_BUNDLED,
            version: None,
            title: None,
        });
    }
    for name in remote.packages() {
        candidates.push(Candidate {
            name: name.clone(),
            group: GROUP_REMOTE,
            version: None,
            title: None,
        });
    }

    candidates.retain(|c| c.name.starts_with(&prefix) && !declared.contains(&c.name));
    candidates.sort_by(|a, b| a.name.cmp(&b.name).then(a.group.cmp(&b.group)));
    candidates.dedup_by(|a, b| a.name == b.name);

    let range = text_range_to_lsp_range(line_index, replace, encoding);
    let items: Vec<CompletionItem> = candidates
        .into_iter()
        .map(|c| CompletionItem {
            label: c.name.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            label_details: Some(CompletionItemLabelDetails {
                detail: c.version.as_ref().map(|v| format!(" {v}")),
                // The `Title` when its index is resident (see the peek above);
                // otherwise the provenance label.
                description: Some(
                    c.title
                        .as_ref()
                        .map_or_else(|| origin_label(&c).to_string(), |t| t.to_string()),
                ),
            }),
            sort_text: Some(format!("{}{}", c.group, c.name)),
            filter_text: Some(c.name.to_string()),
            // An explicit edit, unlike the R path, which lets the client derive
            // the replace range from the language's word pattern. That pattern
            // belongs to a language id we just invented, so relying on it would
            // be guesswork — and it is exactly the wrapped-continuation case
            // that would break.
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: c.name.to_string(),
            })),
            data: serde_json::to_value(CompletionData::Package {
                name: c.name.clone(),
            })
            .ok(),
            ..Default::default()
        })
        .collect();

    Some(CompletionResponse::List(CompletionList {
        // The candidate pool does not change as the prefix grows, so the client
        // can filter the rest of the way itself.
        is_incomplete: false,
        items,
    }))
}

/// Hover over a declared dependency: its installed version and its own `Title`.
///
/// Requires the cursor to be *on* an entry's name. Fresh ground after a comma
/// is a place to complete, not a thing to describe, and a version constraint is
/// not a package.
pub fn compute_description_hover(
    text: &str,
    offset: usize,
    indexed: &IndexedProvider,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Option<Hover> {
    let DescriptionContext::DependencyName {
        replace,
        on_entry: true,
        ..
    } = classify(text, offset)
    else {
        return None;
    };
    let start: usize = replace.start().into();
    let end: usize = replace.end().into();
    let name = &text[start..end];
    // `R` names the language, and `PackageIndex::r_version` is the R that built
    // some package — not the R this constraint is about. Say nothing rather
    // than something false.
    if name == "R" {
        return None;
    }
    let markdown = render_package_markdown(name, indexed)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(text_range_to_lsp_range(line_index, replace, encoding)),
    })
}

/// The installed version of every declared dependency, rendered after the entry
/// it belongs to.
///
/// Anchored at the **end of the whole entry** rather than the end of the name:
/// the two coincide for a constraint-free entry, and where they differ
/// (`dplyr (>= 1.0.0)`) the fact belongs after the declaration instead of
/// between the name and the floor it declares.
///
/// Only locally harvested packages get a hint — the same rule hover follows, for
/// the same reason: there is nothing to state about a package we have not read.
pub fn compute_description_inlay_hints(
    text: &str,
    visible: Range,
    indexed: &IndexedProvider,
    line_index: &LineIndex,
    encoding: PositionEncoding,
) -> Vec<InlayHint> {
    let visible = lsp_range_to_text_range(line_index, visible, encoding);
    let document = crate::dcf::parse(text).document();
    let mut hints = Vec::new();
    for field in document.fields() {
        if !is_dependency_field(&field.name()) {
            continue;
        }
        for entry in dependency_entries(&field) {
            // `R` names the language, not a package (see
            // `compute_description_hover`).
            if entry.name == "R" {
                continue;
            }
            let anchor = entry.range.end();
            // Filter on the anchor, not the entry, so a hint is computed exactly
            // when it would be drawn.
            if !visible.contains_inclusive(anchor) {
                continue;
            }
            let Some(index) = indexed.package(&entry.name) else {
                continue;
            };
            hints.push(InlayHint {
                position: line_index.byte_to_position(u32::from(anchor) as usize, encoding),
                label: InlayHintLabel::String(index.version.to_string()),
                // No kind: neither `TYPE` nor `PARAMETER` describes an installed
                // version, and the kind is what a client's hint filters key on.
                kind: None,
                // Accepting a hint inserts its label, and a bare version is not
                // valid DCF there; rewriting the floor to the installed version
                // pins the dev machine's library, which is a code action's
                // decision to offer, not a hint's to make silently.
                text_edits: None,
                tooltip: Some(InlayHintTooltip::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: package_card(index),
                })),
                // The label sits flush against the entry's own comma.
                padding_left: Some(true),
                padding_right: None,
                data: None,
            });
        }
    }
    hints
}

/// The markdown card for a package: its installed version and its own `Title`.
///
/// Shared by hover and `completionItem/resolve`, so the two never drift. `None`
/// when the package is not locally harvested — there is nothing to say beyond
/// the name the user already typed, and an empty card is worse than no card.
pub(crate) fn render_package_markdown(package: &str, indexed: &IndexedProvider) -> Option<String> {
    Some(package_card(indexed.package(package)?))
}

/// The card body, taken straight off an index the caller already holds — which
/// is how an inlay hint gets the version and the tooltip from one lookup.
fn package_card(index: &PackageIndex) -> String {
    let mut out = format!("**`{}`** {}", index.package, index.version);
    if let Some(title) = &index.title {
        out.push_str("\n\n");
        out.push_str(title);
    }
    out
}

/// The one-word provenance shown after a candidate.
fn origin_label(c: &Candidate) -> &'static str {
    match c.group {
        GROUP_INSTALLED => "installed",
        GROUP_BASE => "base R",
        GROUP_BUNDLED => "CRAN",
        _ => "remote",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rindex::schema::{PackageIndex, SCHEMA_VERSION};

    const BASE: &str = "\
Package: testpkg
Title: A Test Package
Imports: dplyr
Suggests: testthat
";

    fn at(text: &str, needle: &str) -> usize {
        text.find(needle).expect("needle present") + needle.len()
    }

    fn indexed() -> IndexedProvider {
        IndexedProvider::from_indices([PackageIndex {
            schema_version: SCHEMA_VERSION,
            package: "dplyr".into(),
            version: "1.1.4".into(),
            lib_path: "/lib".into(),
            title: Some("A Grammar of Data Manipulation".into()),
            r_version: None,
            harvested_at: 0,
            attaches: Vec::new(),
            symbols: Vec::new(),
        }])
    }

    fn labels(text: &str, offset: usize) -> Vec<String> {
        let buffer = TextBuffer::new(text.to_string());
        let resp = compute_description_completions(
            text,
            offset,
            &indexed(),
            &RemoteExports::new(),
            buffer.line_index(),
            PositionEncoding::Utf16,
        );
        match resp {
            Some(CompletionResponse::List(list)) => {
                list.items.into_iter().map(|i| i.label).collect()
            }
            Some(CompletionResponse::Array(items)) => items.into_iter().map(|i| i.label).collect(),
            None => Vec::new(),
        }
    }

    #[test]
    fn completion_inside_an_imports_entry_filters_on_the_typed_prefix() {
        let text = "Package: testpkg\nImports: dp\n";
        let got = labels(text, at(text, "Imports: dp"));
        assert!(got.iter().all(|l| l.starts_with("dp")), "{got:?}");
        assert!(got.contains(&"dplyr".to_string()), "{got:?}");
    }

    #[test]
    fn completion_after_a_trailing_comma_offers_every_candidate() {
        let text = "Package: testpkg\nImports: stats, \n";
        let got = labels(text, at(text, "Imports: stats, "));
        assert!(
            got.len() > 100,
            "expected the whole pool, got {}",
            got.len()
        );
        assert!(got.contains(&"dplyr".to_string()), "{got:?}");
    }

    #[test]
    fn completion_inside_a_version_constraint_offers_nothing() {
        // `dplyr (>= 1.|0)` — the cursor is in the version, where a package
        // name is not what comes next.
        let text = "Package: testpkg\nImports: dplyr (>= 1.0)\n";
        assert!(labels(text, at(text, ">= 1.")).is_empty());
    }

    #[test]
    fn completion_in_a_non_dependency_field_offers_nothing() {
        let text = BASE;
        assert!(labels(text, at(text, "Title: A")).is_empty());
    }

    #[test]
    fn completion_on_a_field_name_offers_nothing() {
        let text = BASE;
        assert!(labels(text, at(text, "Impo")).is_empty());
    }

    #[test]
    fn completion_skips_a_package_already_declared() {
        // `testthat` is in `Suggests`, so it must not be offered in `Imports`.
        let text = "Package: testpkg\nImports: te\nSuggests: testthat\n";
        let got = labels(text, at(text, "Imports: te"));
        assert!(!got.contains(&"testthat".to_string()), "{got:?}");
    }

    #[test]
    fn completion_still_offers_the_entry_being_edited() {
        // Re-typing over an existing entry must not suppress it as "declared":
        // the entry under the cursor is the one being replaced.
        let text = "Package: testpkg\nImports: dplyr\n";
        let got = labels(text, at(text, "Imports: dp"));
        assert!(got.contains(&"dplyr".to_string()), "{got:?}");
    }

    #[test]
    fn completion_in_depends_offers_r() {
        let text = "Package: testpkg\nDepends: R\n";
        assert!(labels(text, at(text, "Depends: R")).contains(&"R".to_string()));
        // ...and `Imports` does not: `R` is the language, not a dependency you
        // may import.
        let text = "Package: testpkg\nImports: R\n";
        assert!(!labels(text, at(text, "Imports: R")).contains(&"R".to_string()));
    }

    /// The canonical `usethis` layout is one package per line. The replace range
    /// must land on the real bytes of that line — a range derived from the
    /// folded value would be off by every continuation indent before it.
    #[test]
    fn completion_on_a_continuation_line_replaces_the_source_range() {
        let text = "Package: testpkg\nImports:\n    stats,\n    dp\n";
        let offset = at(text, "    dp");
        let DescriptionContext::DependencyName { replace, .. } = classify(text, offset) else {
            panic!("expected a dependency-name context");
        };
        let start: usize = replace.start().into();
        let end: usize = replace.end().into();
        assert_eq!(&text[start..end], "dp");
        assert_eq!(start, text.rfind("dp").expect("dp present"));
    }

    #[test]
    fn completion_after_a_complete_entry_offers_nothing() {
        // `dplyr |` — what follows a name is a constraint, not a second package.
        let text = "Package: testpkg\nImports: dplyr \n";
        assert!(labels(text, at(text, "Imports: dplyr ")).is_empty());
    }

    #[test]
    fn completion_in_a_comment_line_offers_nothing() {
        let text = "Package: testpkg\nImports:\n    stats,\n# arity-ignore: x\n    dp\n";
        assert!(labels(text, at(text, "# arity-ignore")).is_empty());
    }

    #[test]
    fn an_installed_candidate_carries_its_version_and_title() {
        let text = "Package: testpkg\nImports: dp\n";
        let buffer = TextBuffer::new(text.to_string());
        let resp = compute_description_completions(
            text,
            at(text, "Imports: dp"),
            &indexed(),
            &RemoteExports::new(),
            buffer.line_index(),
            PositionEncoding::Utf16,
        );
        let Some(CompletionResponse::List(list)) = resp else {
            panic!("expected a completion list");
        };
        let dplyr = list
            .items
            .iter()
            .find(|i| i.label == "dplyr")
            .expect("dplyr offered");
        let details = dplyr.label_details.as_ref().expect("label details");
        assert_eq!(details.detail.as_deref(), Some(" 1.1.4"));
        assert_eq!(
            details.description.as_deref(),
            Some("A Grammar of Data Manipulation"),
            "an installed candidate is labeled with its own Title"
        );
    }

    fn hover_at(text: &str, offset: usize, indexed: &IndexedProvider) -> Option<Hover> {
        let buffer = TextBuffer::new(text.to_string());
        compute_description_hover(
            text,
            offset,
            indexed,
            buffer.line_index(),
            PositionEncoding::Utf16,
        )
    }

    fn hover_markdown(hover: &Hover) -> &str {
        match &hover.contents {
            HoverContents::Markup(m) => &m.value,
            other => panic!("expected markup, got {other:?}"),
        }
    }

    #[test]
    fn hover_on_a_dependency_shows_the_installed_version_and_title() {
        let text = "Package: testpkg\nImports: dplyr (>= 1.0)\n";
        let hover = hover_at(text, at(text, "Imports: dpl"), &indexed()).expect("hover on dplyr");
        let md = hover_markdown(&hover);
        assert!(md.contains("`dplyr`"), "{md}");
        assert!(md.contains("1.1.4"), "version: {md}");
        assert!(md.contains("A Grammar of Data Manipulation"), "title: {md}");

        // The range covers the name alone, not the version constraint.
        let range = hover.range.expect("a range");
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 9);
        assert_eq!(range.end.character, 14);
    }

    #[test]
    fn hover_on_a_version_constraint_returns_none() {
        let text = "Package: testpkg\nImports: dplyr (>= 1.0)\n";
        assert!(hover_at(text, at(text, ">= 1."), &indexed()).is_none());
    }

    #[test]
    fn hover_on_an_unindexed_package_returns_none() {
        // Known to CRAN but not harvested here: there is nothing to say beyond
        // the name the user already typed, and an empty card is worse than none.
        let text = "Package: testpkg\nImports: dplyr\n";
        assert!(hover_at(text, at(text, "Imports: dpl"), &IndexedProvider::empty()).is_none());
    }

    #[test]
    fn hover_on_r_in_depends_returns_none() {
        // `PackageIndex::r_version` is the R that *built* some package, not the
        // R this constraint is about. Reporting it here would be a lie.
        let text = "Package: testpkg\nDepends: R (>= 4.1)\n";
        assert!(hover_at(text, at(text, "Depends: R"), &indexed()).is_none());
    }

    #[test]
    fn hover_on_a_field_name_returns_none() {
        let text = "Package: testpkg\nImports: dplyr\n";
        assert!(hover_at(text, at(text, "Impo"), &indexed()).is_none());
    }

    fn hints_in(text: &str, visible: Range, indexed: &IndexedProvider) -> Vec<InlayHint> {
        let buffer = TextBuffer::new(text.to_string());
        compute_description_inlay_hints(
            text,
            visible,
            indexed,
            buffer.line_index(),
            PositionEncoding::Utf16,
        )
    }

    /// The whole document, which is what a client sends for a file this small.
    fn whole(text: &str) -> Range {
        let buffer = TextBuffer::new(text.to_string());
        Range::new(
            Position::new(0, 0),
            buffer
                .line_index()
                .byte_to_position(text.len(), PositionEncoding::Utf16),
        )
    }

    fn hints(text: &str, indexed: &IndexedProvider) -> Vec<InlayHint> {
        hints_in(text, whole(text), indexed)
    }

    fn label_text(hint: &InlayHint) -> &str {
        match &hint.label {
            InlayHintLabel::String(label) => label,
            other => panic!("expected a plain label, got {other:?}"),
        }
    }

    #[test]
    fn inlay_hints_report_the_installed_version_at_the_end_of_each_entry() {
        let text = "Package: testpkg\nImports: dplyr (>= 1.0)\nSuggests: testthat\n";
        let got = hints(text, &indexed());
        // `testthat` is not harvested, so there is no installed version to state.
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(label_text(&got[0]), "1.1.4");
        // Character 23 is just past the closing paren, not the name's end at 14:
        // the fact follows the whole declaration rather than splitting the name
        // from the floor it declares.
        assert_eq!(got[0].position, Position::new(1, 23));
    }

    #[test]
    fn an_inlay_hint_is_kindless_and_carries_the_package_card_as_a_tooltip() {
        let text = "Package: testpkg\nImports: dplyr\n";
        let got = hints(text, &indexed());
        let hint = got.first().expect("a hint on dplyr");
        // `TYPE` and `PARAMETER` are the only kinds and neither describes an
        // installed version — and the kind is what a client's "hide type hints"
        // setting filters on, which would silently eat these.
        assert!(hint.kind.is_none());
        assert_eq!(hint.padding_left, Some(true));
        // Accepting a hint inserts its label, and a bare version is not valid
        // DCF there; rewriting the floor is a code action's call, not a hint's.
        assert!(hint.text_edits.is_none());
        let Some(InlayHintTooltip::MarkupContent(tooltip)) = &hint.tooltip else {
            panic!("expected a markdown tooltip, got {:?}", hint.tooltip);
        };
        assert_eq!(tooltip.kind, MarkupKind::Markdown);
        assert!(tooltip.value.contains("`dplyr`"), "{}", tooltip.value);
        assert!(tooltip.value.contains("1.1.4"), "{}", tooltip.value);
        assert!(
            tooltip.value.contains("A Grammar of Data Manipulation"),
            "the same card hover shows: {}",
            tooltip.value
        );
    }

    #[test]
    fn inlay_hints_skip_r_in_depends() {
        // Same reason `hover_on_r_in_depends_returns_none` does: `r_version` is
        // the R that *built* some package, not the R this constraint is about.
        let text = "Package: testpkg\nDepends: R (>= 4.1)\nImports: dplyr\n";
        let got = hints(text, &indexed());
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].position.line, 2, "the hint is on the dplyr line");
    }

    /// The canonical `usethis` layout is one package per line. A position derived
    /// from the folded value would be off by every continuation indent before it.
    #[test]
    fn an_inlay_hint_on_a_continuation_line_uses_the_source_position() {
        let text = "Package: p\nImports:\n    stats,\n    dplyr (>= 1.0)\n";
        let got = hints(text, &indexed());
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].position, Position::new(3, 18));
    }

    #[test]
    fn inlay_hints_outside_the_visible_range_are_not_computed() {
        let text = "Package: p\nImports: dplyr\nSuggests: dplyr\n";
        // Only the `Suggests` line is on screen.
        let visible = Range::new(Position::new(2, 0), Position::new(3, 0));
        let got = hints_in(text, visible, &indexed());
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].position.line, 2);
    }

    #[test]
    fn inlay_hints_ignore_non_dependency_fields() {
        let text = "Package: testpkg\nTitle: dplyr is great\n";
        assert!(hints(text, &indexed()).is_empty());
    }

    #[test]
    fn a_bundled_candidate_names_its_tier_instead() {
        // Nothing is harvested here, so `dplyr` arrives from the bundled CRAN
        // list: no version, no Title, just where it came from.
        let text = "Package: testpkg\nImports: dp\n";
        let buffer = TextBuffer::new(text.to_string());
        let resp = compute_description_completions(
            text,
            at(text, "Imports: dp"),
            &IndexedProvider::empty(),
            &RemoteExports::new(),
            buffer.line_index(),
            PositionEncoding::Utf16,
        );
        let Some(CompletionResponse::List(list)) = resp else {
            panic!("expected a completion list");
        };
        let dplyr = list
            .items
            .iter()
            .find(|i| i.label == "dplyr")
            .expect("dplyr offered");
        let details = dplyr.label_details.as_ref().expect("label details");
        assert_eq!(details.detail, None, "not installed, so no version");
        assert_eq!(details.description.as_deref(), Some("CRAN"));
    }
}
