//! parse_Rd brace **recovery** over kept-incomplete `@md` prose sections.
//!
//! With markdown on, a `sections = FALSE` prose tag (`tag_markdown`: `@note`,
//! `@return`, `@seealso`, …) gets **no** `rdComplete` check — roxygen2 keeps the
//! rendered body even when its braces are unbalanced (`markdown_if_active`,
//! R/tag-parser.R), and the imbalance flows into the generated `.Rd` file where
//! `tools::parse_Rd`'s error recovery restructures it (engine-probed, 8.0.0):
//!
//! * a stray `{` opens a `LIST` group that swallows everything to the next `}` —
//!   including the section's own closing brace, so each **following** section's
//!   header errors out ("unexpected section header") and is dropped while its
//!   `{…}` body folds into the still-open section as another `LIST`;
//! * a stray `}` closes the section early; the remaining body text spills as
//!   **top-level** `TEXT` sections (one per run — a dropped stray brace splits
//!   runs), stray braces at top level are dropped with an error, and a following
//!   section header at top level parses normally;
//! * at end of input every still-open group keeps its content ("unexpected
//!   END_OF_INPUT"), and an empty swallowed group stays a childless `(LIST)`.
//!
//! This module is the faithful translation of that recovery: a bracket machine
//! over the projected section strings, run per standalone topic once all its
//! sections are built (before [`resolve_md_text_braces`], so a `\{` escape in a
//! `TEXT` atom is still distinguishable from a structural brace). Sections are
//! consumed in roxygen2's **physical emission order** (`RoxyTopic$format`'s fixed
//! `order`, R/topic.R), not the projector's; the facade's final sort makes the
//! splice order irrelevant.
//!
//! The pass is **bounded**: it only runs when every consequence of the recovery
//! is modelable from the projected strings alone. It bails (leaving the current
//! projection, a recorded backlog divergence) when the block carries a tag whose
//! rendered output the projector does not place (`@keywords`, `@family`,
//! `@describeIn`, …), when the affected tail contains a section the machine
//! cannot tokenize (`\section` — two-arg — or `\examples`, whose body is
//! reformatted R), when the topic is a merged-topic member (recovery belongs to
//! the *merged* file), or when the imbalance is not attributable to structural
//! braces in top-level `TEXT` atoms (e.g. a `\emph{\}` rendering, whose drop
//! parity [`link_display_render_drops`] models separately). Incomplete `@title`/
//! `@format`/`@source` stay backlog: their tails cross the roclet-generated
//! `\usage`/`\arguments`, which the projector cannot see.

use super::*;

/// roxygen2's `RoxyTopic$format` emission-order index for a projected section
/// head (R/topic.R). `section` is assigned its **maximum** plausible position —
/// a `(\section …)` string may be an `@section` (18), a Slots/Fields aggregate
/// (14/15), or a heading hoisted out of `\description`/`\details` (11/12) — so
/// the tail check errs toward bailing, never toward mis-ordering.
fn emission_position(head: &str) -> Option<usize> {
    Some(match head {
        "title" => 5,
        "format" => 6,
        "source" => 7,
        "value" => 10,
        "description" => 11,
        "details" => 12,
        "note" => 17,
        "section" => 18,
        "examples" => 19,
        "references" => 20,
        "seealso" => 21,
        "author" => 22,
        _ => return None,
    })
}

/// The kept-incomplete candidates: md-on prose sections whose rendered body can
/// reach parse_Rd unbalanced. `@description`/`@details` drop a brace-*count*
/// imbalance (`sections = TRUE`), but a net-zero **dip** (`a } b { c`) passes
/// `rdComplete` and is kept, so they trigger too. `title`/`format`/`source` are
/// excluded (their tails cross roclet-generated sections).
const TRIGGER_HEADS: &[&str] = &[
    "value",
    "description",
    "details",
    "note",
    "references",
    "seealso",
    "author",
];

/// Heads the bracket machine can tokenize as `\head{body}`: single-argument
/// prose sections. Anything else in the affected tail bails the pass.
const CONSUMABLE_HEADS: &[&str] = &[
    "value",
    "description",
    "details",
    "note",
    "references",
    "seealso",
    "author",
];

/// Tags whose rendered Rd output the projector fully places relative to the
/// trigger heads: they either project to a placed section string, render only
/// roclet scaffolding **before** every trigger position (`\usage`, `\arguments`,
/// `\alias`, `\docType`, …), or render nothing (NAMESPACE directives). A tag
/// outside this set can inject Rd content the machine cannot see (`@keywords` →
/// trailing `\keyword`, `@family` → `\seealso` text, `@describeIn` →
/// `\minidesc`, `@rawRd`, templates, …), so its presence bails the pass.
const SAFE_RECOVERY_TAGS: &[&str] = &[
    "md",
    "noMd",
    "name",
    "rdname",
    "aliases",
    "title",
    "description",
    "details",
    "return",
    "seealso",
    "source",
    "format",
    "references",
    "note",
    "author",
    "section",
    "examples",
    "examplesIf",
    "param",
    "inheritParams",
    "usage",
    "encoding",
    "backref",
    "docType",
    "slot",
    "field",
    "export",
    "exportClass",
    "exportMethod",
    "exportPattern",
    "exportS3Method",
    "import",
    "importFrom",
    "importClassesFrom",
    "importMethodsFrom",
    "rawNamespace",
    "useDynLib",
];

/// One structural piece of a `TEXT` atom's decoded content: prose, or a bare
/// (structural) brace. Mirrors `rd_complete`'s escape pairing — a `\` consumes
/// the next char, so a brace after an odd backslash run stays prose (`\{`), and
/// `%` is inert because the `@md` render escapes it (`append_leaf_text`).
enum Piece {
    Text(String),
    Open,
    Close,
}

/// Split a decoded `TEXT` content string at its structural braces.
fn split_structural(text: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut buf = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                buf.push('\\');
                if let Some(next) = chars.next() {
                    buf.push(next);
                }
            }
            '{' | '}' => {
                if !buf.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut buf)));
                }
                pieces.push(if c == '{' { Piece::Open } else { Piece::Close });
            }
            _ => buf.push(c),
        }
    }
    if !buf.is_empty() {
        pieces.push(Piece::Text(buf));
    }
    pieces
}

/// Whether the section's top-level `TEXT` braces disturb parse_Rd: the running
/// depth dips negative (an early close) or ends nonzero (an unclosed group).
/// This is deliberately **not** `rd_complete` — a net-zero dip passes roxygen2's
/// count-based scan (the section is kept) yet still restructures under parse_Rd.
fn text_brace_disturbance(children: &[String]) -> bool {
    let mut depth: i64 = 0;
    for child in children {
        let Some(text) = decode_text_atom(child) else {
            continue;
        };
        for piece in split_structural(&text) {
            match piece {
                Piece::Open => depth += 1,
                Piece::Close => {
                    depth -= 1;
                    if depth < 0 {
                        return true;
                    }
                }
                Piece::Text(_) => {}
            }
        }
    }
    depth != 0
}

/// Whether the section's brace balance is fully attributable to structural
/// braces in its top-level `TEXT` atoms: with those braces removed, the
/// remaining render must scan complete. If not, the imbalance hides inside an
/// opaque atom (a `\emph{\}`-style rendering) the machine cannot split — bail.
fn text_braces_attributable(children: &[String]) -> bool {
    let neutered: Vec<String> = children
        .iter()
        .map(|child| match decode_text_atom(child) {
            Some(text) => {
                let joined: String = split_structural(&text)
                    .into_iter()
                    .filter_map(|p| match p {
                        Piece::Text(t) => Some(t),
                        _ => None,
                    })
                    .collect();
                format!("(TEXT {})", encode_text(&joined))
            }
            None => child.clone(),
        })
        .collect();
    section_atoms_rd_complete(&neutered, true)
}

/// Parse a projected section string into its head and top-level child atoms.
/// Returns `None` for anything that is not a `(\head …)` form (e.g. a bare
/// top-level atom from `@rawRd`).
fn parse_section_string(s: &str) -> Option<(&str, Vec<&str>)> {
    let rest = s.strip_prefix("(\\")?;
    let head_end = rest.find([' ', ')'])?;
    let head = &rest[..head_end];
    let inner = rest[head_end..].strip_suffix(')')?;
    Some((head, split_top_level_atoms(inner)))
}

/// A builder frame of the bracket machine: the open section at the stack
/// bottom, or a swallowed `{…}` group (`LIST`) above it. `run` accumulates
/// prose between structural events; each flushed run normalizes like a
/// coalesced parse_Rd `TEXT` node (`norm_ws`), and a whitespace-only run
/// vanishes — exactly how the inter-section newlines wash out.
struct Frame {
    /// `Some(head)` for the section at the stack bottom, `None` for a `LIST`.
    head: Option<String>,
    children: Vec<String>,
    run: String,
}

impl Frame {
    fn section(head: &str) -> Self {
        Frame {
            head: Some(head.to_string()),
            children: Vec::new(),
            run: String::new(),
        }
    }

    fn list() -> Self {
        Frame {
            head: None,
            children: Vec::new(),
            run: String::new(),
        }
    }

    fn flush_run(&mut self) {
        let text = norm_ws(&self.run);
        self.run.clear();
        if !text.is_empty() {
            self.children.push(format!("(TEXT {})", encode_text(&text)));
        }
    }

    fn push_atom(&mut self, atom: String) {
        self.flush_run();
        self.children.push(atom);
    }

    /// Serialize the closed frame. A childless swallowed group stays a
    /// childless `(LIST)` (engine-probed: `\value{r {\n}` keeps the empty
    /// group), and a childless section serializes bare (`(\note)`).
    fn serialize(mut self) -> String {
        self.flush_run();
        let head = match &self.head {
            Some(h) => format!("\\{h}"),
            None => "LIST".to_string(),
        };
        if self.children.is_empty() {
            format!("({head})")
        } else {
            format!("({head} {})", self.children.join(" "))
        }
    }
}

/// Flush a top-level prose run as its own `(TEXT …)` section (a spill after an
/// early close). Any structural event at top level — a dropped stray brace, a
/// section header — bounds the run, so consecutive spills stay separate
/// sections exactly as parse_Rd's error recovery leaves them.
fn flush_top(run: &mut String, results: &mut Vec<String>) {
    let text = norm_ws(run);
    run.clear();
    if !text.is_empty() {
        results.push(format!("(TEXT {})", encode_text(&text)));
    }
}

/// Close the innermost frame: a `LIST` folds into its parent, the bottom
/// section emits, and a stray close at top level is dropped (bounding any
/// spill run).
fn handle_close(stack: &mut Vec<Frame>, top_run: &mut String, results: &mut Vec<String>) {
    match stack.pop() {
        Some(frame) => {
            let s = frame.serialize();
            match stack.last_mut() {
                Some(parent) => parent.push_atom(s),
                None => results.push(s),
            }
        }
        None => flush_top(top_run, results),
    }
}

/// Open a group: a pending top-level header opens its section, an open inside
/// any frame opens a swallowed `LIST`, and a stray open at top level is
/// dropped (bounding any spill run).
fn handle_open(
    stack: &mut Vec<Frame>,
    pending: &mut Option<String>,
    top_run: &mut String,
    results: &mut Vec<String>,
) {
    if let Some(head) = pending.take() {
        stack.push(Frame::section(&head));
    } else if stack.is_empty() {
        flush_top(top_run, results);
    } else {
        stack.push(Frame::list());
    }
}

/// Run the recovery pass over one standalone topic's projected sections
/// (`out[block_start..]`). See the module doc for the model and the bail
/// conditions. `tag_names` is every `@tag` name the block carries, for the
/// safe-tag gate.
pub(super) fn parse_rd_recovery(
    out: &mut Vec<String>,
    block_start: usize,
    md: bool,
    tag_names: &[String],
) {
    // Markdown-off kept sections cannot be incomplete: `markdown_if_active`'s
    // else-branch drops every brace-imbalanced prose section unconditionally.
    if !md {
        return;
    }
    if tag_names
        .iter()
        .any(|n| !SAFE_RECOVERY_TAGS.contains(&n.as_str()))
    {
        return;
    }

    // Parse every section string; an unplaceable one (unknown head, bare atom)
    // means the physical file order is not fully known — bail.
    struct Entry {
        out_index: usize,
        head: String,
        position: usize,
        children: Vec<String>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (i, s) in out[block_start..].iter().enumerate() {
        let Some((head, children)) = parse_section_string(s) else {
            return;
        };
        let Some(position) = emission_position(head) else {
            return;
        };
        entries.push(Entry {
            out_index: block_start + i,
            head: head.to_string(),
            position,
            children: children.into_iter().map(str::to_string).collect(),
        });
    }

    // The first (in emission order) disturbed trigger section starts the
    // recovery; everything at or after its position is in the affected tail.
    let first = entries
        .iter()
        .filter(|e| TRIGGER_HEADS.contains(&e.head.as_str()) && text_brace_disturbance(&e.children))
        .min_by_key(|e| e.position);
    let Some(first) = first else {
        return;
    };
    let start_pos = first.position;

    let mut tail: Vec<&Entry> = entries.iter().filter(|e| e.position >= start_pos).collect();
    tail.sort_by_key(|e| (e.position, e.out_index));
    // Every tail section must be machine-tokenizable, unambiguous in position
    // (no duplicate heads — roxygen2 would have collapsed them), and have its
    // brace balance fully attributable to top-level `TEXT` braces.
    for (i, e) in tail.iter().enumerate() {
        if !CONSUMABLE_HEADS.contains(&e.head.as_str())
            || tail[..i].iter().any(|p| p.head == e.head)
            || !text_braces_attributable(&e.children)
        {
            return;
        }
    }

    // The bracket machine over the tail's physical render: the disturbed
    // section opens; each later section contributes header + `{` + body + `}`.
    let mut stack: Vec<Frame> = Vec::new();
    let mut results: Vec<String> = Vec::new();
    let mut top_run = String::new();
    let mut pending: Option<String> = None;

    for (k, entry) in tail.iter().enumerate() {
        // Header + opening brace. The first section is open by construction;
        // a later header at top level opens normally, while one inside an open
        // section is dropped ("unexpected section header") and its brace opens
        // a plain swallowed group.
        if k == 0 {
            stack.push(Frame::section(&entry.head));
        } else {
            if stack.is_empty() {
                flush_top(&mut top_run, &mut results);
                pending = Some(entry.head.clone());
            }
            handle_open(&mut stack, &mut pending, &mut top_run, &mut results);
        }
        // Body atoms: `TEXT` splits at structural braces; other atoms are
        // opaque balanced units (at top level, a spilled macro is its own
        // section).
        for child in &entry.children {
            match decode_text_atom(child) {
                Some(text) => {
                    for piece in split_structural(&text) {
                        match piece {
                            Piece::Text(t) => match stack.last_mut() {
                                Some(frame) => frame.run.push_str(&t),
                                None => top_run.push_str(&t),
                            },
                            Piece::Open => {
                                handle_open(&mut stack, &mut pending, &mut top_run, &mut results);
                            }
                            Piece::Close => {
                                handle_close(&mut stack, &mut top_run, &mut results);
                            }
                        }
                    }
                }
                None => match stack.last_mut() {
                    Some(frame) => frame.push_atom(child.clone()),
                    None => {
                        flush_top(&mut top_run, &mut results);
                        results.push(child.clone());
                    }
                },
            }
        }
        // The section's own closing brace.
        handle_close(&mut stack, &mut top_run, &mut results);
    }
    // End of input: every still-open group keeps its content.
    while let Some(frame) = stack.pop() {
        let s = frame.serialize();
        match stack.last_mut() {
            Some(parent) => parent.push_atom(s),
            None => results.push(s),
        }
    }
    flush_top(&mut top_run, &mut results);

    // Splice: drop the consumed section strings, append the recovered ones
    // (the facade's final sort makes order irrelevant).
    let mut consumed: Vec<usize> = tail.iter().map(|e| e.out_index).collect();
    consumed.sort_unstable();
    for idx in consumed.into_iter().rev() {
        out.remove(idx);
    }
    out.extend(results);
}
