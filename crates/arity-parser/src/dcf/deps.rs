//! The dependency-field grammar: `name (>= version), name, ...`.
//!
//! `Depends`, `Imports`, `Suggests`, `LinkingTo`, and `Enhances` all carry a
//! comma-separated list of package names, each optionally followed by a
//! parenthesized version constraint. That shape is *grammar*, so it lives here
//! next to the DCF parser rather than in a consumer.
//!
//! What is deliberately **not** here is R semantics: that `R` names the
//! language and not a package, that `Depends` attaches to the search path while
//! `Imports` does not, and how two version strings order. Those are the
//! consumer's, and the version is handed back as text for exactly that reason.
//!
//! Every range is a **source** range, into the same buffer the field was parsed
//! from — so a diagnostic or a hover can point at one entry without a second
//! mapping layer. Entries are reported across continuation lines: the field's
//! logical value is what R splits, and a list wrapped one-per-line is the
//! canonical style.
//!
//! Parsing never fails. An entry whose parenthesized part does not yield a
//! constraint keeps its name and reports [`DependencyEntry::malformed_constraint`],
//! because a broken constraint must never hide a package name from resolution.

use rowan::{TextRange, TextSize};
use smol_str::SmolStr;

use crate::dcf::ast::Field;

/// The DCF fields that carry a dependency list, in R's canonical order.
pub const DEPENDENCY_FIELDS: [&str; 5] =
    ["Depends", "Imports", "Suggests", "LinkingTo", "Enhances"];

/// Whether `name` is one of [`DEPENDENCY_FIELDS`].
pub fn is_dependency_field(name: &str) -> bool {
    DEPENDENCY_FIELDS.contains(&name)
}

/// The comparison operator of a version constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionOp {
    /// `>=`
    Ge,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `<`
    Lt,
    /// `==`
    Eq,
    /// `!=`
    Ne,
}

impl VersionOp {
    /// The operator this text starts with, and its length in bytes. Two-byte
    /// operators are tried first, so `>=` never lexes as `>` followed by `=`.
    fn lex(text: &str) -> Option<(VersionOp, usize)> {
        const TWO: [(&str, VersionOp); 4] = [
            (">=", VersionOp::Ge),
            ("<=", VersionOp::Le),
            ("==", VersionOp::Eq),
            ("!=", VersionOp::Ne),
        ];
        const ONE: [(&str, VersionOp); 2] = [(">", VersionOp::Gt), ("<", VersionOp::Lt)];
        TWO.iter()
            .find(|(s, _)| text.starts_with(s))
            .map(|(s, op)| (*op, s.len()))
            .or_else(|| {
                ONE.iter()
                    .find(|(s, _)| text.starts_with(s))
                    .map(|(s, op)| (*op, s.len()))
            })
    }

    /// Whether this operator states a *lower* bound — the only kind a version
    /// floor can be read from.
    pub fn is_lower_bound(self) -> bool {
        matches!(self, VersionOp::Ge | VersionOp::Gt)
    }
}

/// One version constraint: an operator and the version it compares against.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionConstraint {
    pub op: VersionOp,
    /// The version exactly as written, trimmed (`4.1.0`, `1.2-3`). Text, not a
    /// parsed version: how two versions order is the consumer's policy.
    pub version: SmolStr,
    /// The whole constraint's source range, operator included.
    pub range: TextRange,
}

/// One entry of a dependency field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DependencyEntry {
    /// The entry's name as written: a package name, or the literal `R`.
    pub name: SmolStr,
    pub name_range: TextRange,
    /// The text between the parentheses, verbatim, and its range — `None` when
    /// the entry carries no parenthesized part at all.
    pub constraint_text: Option<(SmolStr, TextRange)>,
    /// The constraints parsed out of `constraint_text`, in source order. R
    /// writes one, but a comma-separated pair (`(>= 1.0, < 2.0)`) is the reason
    /// entry splitting has to be paren-aware, so both are supported.
    pub constraints: Vec<VersionConstraint>,
    /// The whole entry's source range.
    pub range: TextRange,
}

impl DependencyEntry {
    /// The entry declares a parenthesized part that yielded no constraint —
    /// exactly the "unparseable version constraint" case, kept as a fact here
    /// and reported as a diagnostic by the consumer.
    pub fn malformed_constraint(&self) -> bool {
        self.constraint_text.is_some() && self.constraints.is_empty()
    }

    /// The first constraint stating a lower bound, which is what a version
    /// floor asks for.
    pub fn lower_bound(&self) -> Option<&VersionConstraint> {
        self.constraints.iter().find(|c| c.op.is_lower_bound())
    }
}

/// Split a dependency field into its entries, in source order.
///
/// Empty entries (a trailing or doubled comma) are dropped — R tolerates them
/// and they name no package.
pub fn dependency_entries(field: &Field) -> Vec<DependencyEntry> {
    let folded = Folded::new(field);
    split_top_level(&folded.text, 0)
        .into_iter()
        .filter_map(|span| entry(&folded, span))
        .collect()
}

/// One entry from its span in the folded text.
fn entry(folded: &Folded, span: Span) -> Option<DependencyEntry> {
    let text = &folded.text[span.start..span.end];
    // The parenthesized part starts at the first `(`; everything before it is
    // the name. An unclosed `(` still yields a name, which is the point.
    let (name_span, paren) = match text.find('(') {
        Some(open) => {
            let close = text.rfind(')').filter(|c| *c > open).unwrap_or(text.len());
            (
                Span::new(span.start, span.start + open),
                Some(Span::new(span.start + open + 1, span.start + close)),
            )
        }
        None => (span, None),
    };

    let name_span = trim(&folded.text, name_span);
    if name_span.is_empty() {
        return None;
    }

    let constraint_text = paren.map(|p| {
        let inner = trim(&folded.text, p);
        (
            SmolStr::new(&folded.text[inner.start..inner.end]),
            folded.source_range(inner),
        )
    });
    let constraints = paren.map(|p| constraints_in(folded, p)).unwrap_or_default();

    Some(DependencyEntry {
        name: SmolStr::new(&folded.text[name_span.start..name_span.end]),
        name_range: folded.source_range(name_span),
        constraint_text,
        constraints,
        range: folded.source_range(trim(&folded.text, span)),
    })
}

/// The constraints inside a parenthesized span.
fn constraints_in(folded: &Folded, paren: Span) -> Vec<VersionConstraint> {
    split_top_level(&folded.text[paren.start..paren.end], paren.start)
        .into_iter()
        .filter_map(|span| constraint(folded, span))
        .collect()
}

/// One `op version` constraint from its span in the folded text.
fn constraint(folded: &Folded, span: Span) -> Option<VersionConstraint> {
    let span = trim(&folded.text, span);
    let (op, len) = VersionOp::lex(&folded.text[span.start..span.end])?;
    let version = trim(&folded.text, Span::new(span.start + len, span.end));
    if version.is_empty() {
        return None;
    }
    Some(VersionConstraint {
        op,
        version: SmolStr::new(&folded.text[version.start..version.end]),
        range: folded.source_range(span),
    })
}

/// A half-open byte span of the folded text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

impl Span {
    fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// `span` with leading and trailing ASCII whitespace removed.
fn trim(text: &str, span: Span) -> Span {
    let slice = &text[span.start..span.end];
    let lead = slice.len() - slice.trim_start().len();
    let trail = slice.len() - slice.trim_end().len();
    if lead + trail >= slice.len() {
        return Span::new(span.start, span.start);
    }
    Span::new(span.start + lead, span.end - trail)
}

/// Split `text` on commas that are not inside parentheses, returning spans
/// offset by `base`. Paren-awareness is not optional: `pkg (>= 1.0, < 2.0)` is
/// one entry, and splitting it naively would invent a package named `< 2.0`.
fn split_top_level(text: &str, base: usize) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                spans.push(Span::new(base + start, base + i));
                start = i + 1;
            }
            _ => {}
        }
    }
    spans.push(Span::new(base + start, base + text.len()));
    spans
}

/// A field's folded value next to a map from folded offsets back to source
/// offsets.
///
/// The fold is exactly [`Field::folded_value`]'s — each nonempty value line's
/// content run, joined by `\n` — which is what makes the map trivial: a
/// `VALUE_TEXT` token is already the trimmed run, so each segment is a verbatim
/// slice of the source and offsets inside it shift by a constant.
struct Folded {
    text: String,
    /// `(folded start, source start, byte length)` per contributing line.
    segments: Vec<(usize, TextSize, usize)>,
}

impl Folded {
    fn new(field: &Field) -> Self {
        let mut text = String::new();
        let mut segments = Vec::new();
        let mut first = true;
        for line in field.value_lines() {
            let Some(tok) = line.content().filter(|tok| !tok.text().is_empty()) else {
                continue;
            };
            if !first {
                text.push('\n');
            }
            first = false;
            let content = tok.text();
            segments.push((text.len(), tok.text_range().start(), content.len()));
            text.push_str(content);
        }
        Folded { text, segments }
    }

    /// The source range of a folded span. A span crossing a fold join covers
    /// the newline and indent in the source too, which is what a reader of the
    /// diagnostic expects to see underlined.
    fn source_range(&self, span: Span) -> TextRange {
        let start = self.source_offset(span.start, false);
        let end = self.source_offset(span.end, true);
        TextRange::new(start, end.max(start))
    }

    /// The source offset of a folded offset. `at_end` breaks the tie when the
    /// offset sits exactly on a segment boundary: a span's end belongs to the
    /// segment it closes, its start to the segment it opens.
    fn source_offset(&self, folded: usize, at_end: bool) -> TextSize {
        let mut last = TextSize::from(0);
        for &(fstart, sstart, len) in &self.segments {
            if folded < fstart {
                // Between two segments (inside the join) — clamp to whichever
                // side the caller is anchored on.
                return if at_end { last } else { sstart };
            }
            if folded < fstart + len || (folded == fstart + len && (at_end || len == 0)) {
                return sstart + TextSize::from((folded - fstart) as u32);
            }
            last = sstart + TextSize::from(len as u32);
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcf;

    /// The single field of a one-field document.
    fn field(text: &str) -> (String, dcf::Field) {
        let output = dcf::parse(text);
        let field = output
            .document()
            .fields()
            .next()
            .expect("the fixture declares one field");
        (text.to_string(), field)
    }

    fn entries(text: &str) -> Vec<DependencyEntry> {
        let (_, f) = field(text);
        dependency_entries(&f)
    }

    fn names(text: &str) -> Vec<String> {
        entries(text).iter().map(|e| e.name.to_string()).collect()
    }

    /// Every entry's name and whole-entry text, sliced out of the *source* by
    /// the reported ranges. This is what proves the spans, not the values.
    fn sliced(text: &str) -> Vec<(String, String)> {
        entries(text)
            .iter()
            .map(|e| (text[e.name_range].to_string(), text[e.range].to_string()))
            .collect()
    }

    #[test]
    fn bare_names_split_on_commas() {
        assert_eq!(
            names("Imports: dplyr, rlang, vctrs\n"),
            ["dplyr", "rlang", "vctrs"]
        );
    }

    #[test]
    fn empty_entries_are_dropped() {
        // A trailing comma and a doubled comma name no package.
        assert_eq!(names("Imports: dplyr, , rlang,\n"), ["dplyr", "rlang"]);
        assert!(names("Imports:\n").is_empty());
        assert!(names("Imports:   \n").is_empty());
    }

    #[test]
    fn r_entry_carries_its_floor() {
        let entries = entries("Depends: R (>= 4.1.0)\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "R");
        let bound = entries[0].lower_bound().expect("a lower bound");
        assert_eq!(bound.op, VersionOp::Ge);
        assert_eq!(bound.version, "4.1.0");
    }

    #[test]
    fn every_operator_lexes() {
        let cases = [
            (">= 1", VersionOp::Ge),
            ("> 1", VersionOp::Gt),
            ("<= 1", VersionOp::Le),
            ("< 1", VersionOp::Lt),
            ("== 1", VersionOp::Eq),
            ("!= 1", VersionOp::Ne),
        ];
        for (constraint, op) in cases {
            let text = format!("Imports: pkg ({constraint})\n");
            let entries = entries(&text);
            assert_eq!(entries[0].constraints.len(), 1, "{constraint}");
            assert_eq!(entries[0].constraints[0].op, op, "{constraint}");
            assert_eq!(entries[0].constraints[0].version, "1", "{constraint}");
        }
    }

    #[test]
    fn whitespace_around_the_operator_is_optional() {
        let entries = entries("Depends: R(>=3.5.0)\n");
        assert_eq!(entries[0].name, "R");
        assert_eq!(entries[0].lower_bound().expect("a bound").version, "3.5.0");
    }

    #[test]
    fn a_comma_inside_parentheses_does_not_split_the_entry() {
        // The reason splitting is paren-aware: naive splitting would invent a
        // package named `< 2.0`.
        let entries = entries("Imports: pkg (>= 1.0, < 2.0), other\n");
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["pkg", "other"]
        );
        assert_eq!(entries[0].constraints.len(), 2);
        assert_eq!(entries[0].constraints[1].op, VersionOp::Lt);
        assert_eq!(entries[0].constraints[1].version, "2.0");
    }

    #[test]
    fn an_unparseable_constraint_keeps_the_name() {
        for text in [
            "Imports: pkg (garbage)\n",
            "Imports: pkg (>=)\n",
            "Imports: pkg ()\n",
        ] {
            let entries = entries(text);
            assert_eq!(entries.len(), 1, "{text}");
            assert_eq!(entries[0].name, "pkg", "{text}");
            assert!(entries[0].constraints.is_empty(), "{text}");
            assert!(entries[0].malformed_constraint(), "{text}");
        }
        // No parenthesized part at all is not malformed — it is the common case.
        assert!(!entries("Imports: pkg\n")[0].malformed_constraint());
    }

    #[test]
    fn entries_span_continuation_lines() {
        // The canonical one-per-line style, and an entry split across the fold.
        let text = "Imports:\n    dplyr (>= 1.0.0),\n    rlang,\n    vctrs\n";
        assert_eq!(names(text), ["dplyr", "rlang", "vctrs"]);

        let text = "Depends:\n    R (>=\n    4.1.0)\n";
        let entries = entries(text);
        assert_eq!(entries[0].name, "R");
        assert_eq!(entries[0].lower_bound().expect("a bound").version, "4.1.0");
    }

    #[test]
    fn ranges_point_at_the_source_bytes() {
        assert_eq!(
            sliced("Imports: dplyr (>= 1.0), rlang\n"),
            [
                ("dplyr".to_string(), "dplyr (>= 1.0)".to_string()),
                ("rlang".to_string(), "rlang".to_string()),
            ]
        );
        // Across a continuation line the whole-entry range covers the newline
        // and indent, and the name range stays tight.
        assert_eq!(
            sliced("Imports:\n    dplyr,\n    rlang\n"),
            [
                ("dplyr".to_string(), "dplyr".to_string()),
                ("rlang".to_string(), "rlang".to_string()),
            ]
        );
    }

    #[test]
    fn constraint_ranges_point_at_the_source_bytes() {
        let text = "Depends: R (>= 4.1.0), stats\n";
        let entries = entries(text);
        let (raw, range) = entries[0]
            .constraint_text
            .clone()
            .expect("a parenthesized part");
        assert_eq!(raw, ">= 4.1.0");
        assert_eq!(&text[range], ">= 4.1.0");
        assert_eq!(&text[entries[0].constraints[0].range], ">= 4.1.0");
        // The second entry has none.
        assert!(entries[1].constraint_text.is_none());
    }

    #[test]
    fn the_fold_matches_the_ast_wrapper() {
        // `Folded` must reproduce `Field::folded_value` byte for byte, or every
        // range it hands out is off. Includes an empty field-header value.
        for text in [
            "Imports: a, b\n",
            "Imports:\n    a,\n    b\n",
            "Imports: a\n    b\n",
            "Imports:\n",
        ] {
            let (_, f) = field(text);
            assert_eq!(Folded::new(&f).text, f.folded_value(), "{text:?}");
        }
    }

    #[test]
    fn dependency_fields_are_recognized_by_name() {
        assert!(DEPENDENCY_FIELDS.iter().all(|f| is_dependency_field(f)));
        assert!(!is_dependency_field("Collate"));
        assert!(!is_dependency_field("depends"));
    }
}
