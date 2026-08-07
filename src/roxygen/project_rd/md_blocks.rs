use super::*;

/// Project a `ROXYGEN_MD_LIST` node into `(\itemize …)` or `(\enumerate …)`: each
/// `ROXYGEN_MD_LIST_ITEM` contributes a name-only `(\item)` followed by its
/// content atoms (the same inline serialization as prose), mirroring roxygen2's
/// translation of a markdown list into an Rd `\itemize`/`\enumerate`. The list is
/// ordered iff its first item's marker is a number.
pub(super) fn serialize_md_list(node: &SyntaxNode) -> String {
    let head = if md_list_is_ordered(node) {
        "\\enumerate"
    } else {
        "\\itemize"
    };
    let mut atoms: Vec<String> = Vec::new();
    for item in node
        .children()
        .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
    {
        atoms.push("(\\item)".to_string());
        // A markdown list exists only under `@md`, so its item content is markdown.
        atoms.extend(md_item_atoms(&md_list_item_inlines(&item)));
    }
    if atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", atoms.join(" "))
    }
}

/// Serialize a markdown list whose item contents have already been rewritten by the
/// whole-field link-reference pipeline ([`apply_user_linkrefs`]) — the resolved-items
/// analog of [`serialize_md_list`]. Each item renders a name-only `(\item)` followed
/// by its resolved inline run (markdown stays active inside a list item). An item the
/// pipeline emptied (it held only a consumed definition) renders a bare `(\item)`.
pub(super) fn serialize_md_list_resolved(ordered: bool, items: &[Vec<Inline>]) -> String {
    let head = if ordered { "\\enumerate" } else { "\\itemize" };
    let mut atoms: Vec<String> = Vec::new();
    for item in items {
        atoms.push("(\\item)".to_string());
        atoms.extend(md_item_atoms(item));
    }
    if atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", atoms.join(" "))
    }
}

/// Serialize one markdown list item's inline run: its content atoms, with any
/// level >= 2 in-item heading (a same-line `- ## Sub`, a content-column ATX
/// line, or a promoted setext paragraph) rendered as roxygen2's `\subsection`
/// over the item's *following* content — `- bar` / `  ## Sub` / `  baz`
/// renders `\item bar \subsection{Sub}{baz}` (engine-probed), the subsection a
/// sibling atom after the item's own text. Deeper headings nest by level, the
/// same outline rule as the section-level frames. The subsection closes at the
/// item's end; roxygen2's flat-string close instead swallows a *following
/// sibling item* into the subsection body (`- Bar`/`  ---`/`  baz`/`- qux` puts
/// `\item qux` inside the subsection) — a recorded divergence (backlog), never
/// pinned. A level-1 heading here belongs to a non-sections tag (under
/// `@description`/`@details` the in-list hoist consumes the whole list before
/// serialization) and stays literal title text (`serialize_inlines`' fallback
/// arm), matching `mdxml_heading`'s no-sections rendering.
fn md_item_atoms(inlines: &[Inline]) -> Vec<String> {
    let is_subsection =
        |inl: &Inline| matches!(inl, Inline::MdHeading(node) if parse_md_heading(node).0 >= 2);
    if !inlines.iter().any(is_subsection) {
        return serialize_inlines(inlines, true);
    }
    // Segment the item: the pre-heading run, then one (heading, following-run)
    // per level >= 2 heading. A level-1 heading stays inside its run (the
    // literal-title fallback).
    let mut segments: Vec<(Option<SyntaxNode>, Vec<Inline>)> = vec![(None, Vec::new())];
    for inl in inlines {
        if let Inline::MdHeading(node) = inl
            && is_subsection(inl)
        {
            segments.push((Some(node.clone()), Vec::new()));
        } else {
            segments.last_mut().unwrap().1.push(inl.clone());
        }
    }
    // The outline: frame 0 is the item itself (level 1 — every subsection
    // heading is deeper); each heading's parent is the nearest open frame of a
    // strictly lower level, exactly as in `emit_section_with_headings`.
    let mut frames: Vec<HeadingFrame> = vec![HeadingFrame {
        level: 1,
        title: Vec::new(),
        body: std::mem::take(&mut segments[0].1),
        children: Vec::new(),
    }];
    let mut stack = vec![0usize];
    for (node, run) in segments.into_iter().skip(1) {
        let node = node.expect("a non-leading segment always carries a heading");
        let (level, title_text) = parse_md_heading(&node);
        let title = resolve_macro_arg_inlines(&title_text);
        while *stack.last().unwrap() != 0 && frames[*stack.last().unwrap()].level >= level {
            stack.pop();
        }
        let parent = *stack.last().unwrap();
        let idx = frames.len();
        frames.push(HeadingFrame {
            level,
            title,
            body: run,
            children: Vec::new(),
        });
        frames[parent].children.push(idx);
        stack.push(idx);
    }
    let mut atoms = serialize_inlines(&frames[0].body, true);
    for &c in &frames[0].children {
        atoms.push(item_subsection_atom(&frames, c));
    }
    atoms
}

/// Project a `ROXYGEN_MD_CODE_BLOCK` node into roxygen2's three-atom fenced-code
/// rendering (`mdxml_code_block`, `R/markdown.R`): an opening
/// `\if{html}{\out{<div class="sourceCode[ <info>]">}}`, a `\preformatted{<code>}`,
/// and a closing `\if{html}{\out{</div>}}`. The `<div>` class carries the fence's
/// info string (empty → bare `sourceCode`); the code is the verbatim block content
/// with a trailing newline (commonmark's `xml_text`). The body's `%`/`{`/`}` are
/// `escape_verb`-escaped by roxygen2 but `parse_Rd` decodes them, so the pins (and
/// thus the projector) carry the raw characters.
pub(super) fn serialize_md_code_block(node: &SyntaxNode) -> Vec<String> {
    let (info, code) = md_code_block_parts(node);
    // cmark entity-decodes the info string during parsing (its XML `info`
    // attribute carries the decoded text), so the class does too (cm-034). A
    // backslash escape is a net no-op here: `double_escape_md` doubles every
    // `\`, and cmark resolves each pair back to a literal `\`.
    let info = decode_html_entities(&info);
    // A knitr chunk header never reaches the class: pass 1 replaces the whole
    // fence with its knit result, whose info is the chunk language alone.
    let info = knitr_chunk_language(&info).unwrap_or(info);
    let class = if info.is_empty() {
        "sourceCode".to_string()
    } else {
        format!("sourceCode {info}")
    };
    // parse_Rd splits a `\preformatted` body into one `VERB` leaf per line (each
    // carrying its trailing `\n`), and an empty body yields no child at all — the
    // same treatment as an indented code block (`serialize_md_indented_code`).
    let body = verb_atoms(&code);
    let preformatted = if body.is_empty() {
        "(\\preformatted)".to_string()
    } else {
        format!("(\\preformatted {})", body.join(" "))
    };
    let html = encode_text("html");
    vec![
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text(&format!("<div class=\"{class}\">"))
        ),
        preformatted,
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text("</div>")
        ),
    ]
}

/// Whether a fenced code block's **raw** info string makes roxygen2's rendered
/// fragment `rdComplete`-incomplete, dropping the enclosing section (cm-143).
/// `mdxml_code_block` pastes the info string into the `sourceCode` div class with
/// no escaping — only the body goes through `escape_verb` — so a `%` in the info
/// comments out the rest of the rendered line (`">}}\preformatted{` and the code's
/// first line) and an unbalanced brace or trailing escape breaks the balance
/// directly. The scan fragment reproduces roxygen2's rendering with the info raw
/// and the body reduced to its newline structure: the body is always neutral
/// (`escape_verb` escapes `%`/`{`/`}`, and `double_escape_md` keeps its backslash
/// runs even), but its line boundaries scope any info-string comment exactly as in
/// the real field. The trailing newline stands in for the `\n\n` that always
/// follows the fragment in the rendered field.
pub(super) fn md_fence_info_drops(node: &SyntaxNode) -> bool {
    let (info, code) = md_code_block_parts(node);
    let info = decode_html_entities(&info);
    // A knitr chunk's raw header never reaches the rendered class (pass 1
    // rewrites the fence first), so the drop scan sees the language instead.
    let info = knitr_chunk_language(&info).unwrap_or(info);
    if info.is_empty() {
        return false;
    }
    let mut frag =
        format!("\\if{{html}}{{\\out{{<div class=\"sourceCode {info}\">}}}}\\preformatted{{");
    frag.extend(code.chars().filter(|&c| c == '\n'));
    frag.push_str("}\\if{html}{\\out{</div>}}\n");
    !rd_complete(&frag)
}

/// The chunk language of a knitr-style fence info string, or `None` when the info
/// is not a chunk header. roxygen2's pass 1 (`is_markdown_code_node`,
/// `R/markdown.R`) treats a fence whose (entity-decoded) info matches
/// `^[{][a-zA-z]+[}, ]` as a knitr chunk, knits it, and splices the result back
/// into the text before the markdown render — so the re-rendered fence carries the
/// chunk's engine as its whole info string (`sourceCode r`), never the raw header.
/// The character class is roxygen2's literal `[a-zA-z]` (the `A-z` half also spans
/// `[`, `\`, `]`, `^`, `_`, and a backquote), ported as written. Knitting is
/// dynamic evaluation and out of arity's static scope: the projector models the
/// *silent* chunk (an output-free knit echoes the source unchanged), so a chunk
/// whose evaluation adds output lines stays a divergence (blocked, not backlog).
fn knitr_chunk_language(info: &str) -> Option<String> {
    let rest = info.strip_prefix('{')?;
    let end = rest
        .find(|c| !('A'..='z').contains(&c))
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    matches!(rest[end..].chars().next(), Some('}' | ',' | ' ')).then(|| rest[..end].to_string())
}

/// Project a `ROXYGEN_MD_INDENTED_CODE` node into the same three-atom rendering as
/// a fenced code block (`mdxml_code_block`), but with a bare `sourceCode` class (an
/// indented code block has no info string) and each line's indentation stripped: a
/// CommonMark indented code block drops four columns of indentation, on top of the
/// `#'` marker and the one space roxygen2 strips first. Each line therefore has its
/// marker, one following space, then up to four further leading spaces removed; the
/// result is joined with newlines (a trailing newline per line, commonmark's
/// `xml_text`) and split into one `VERB` per line by `parse_Rd`.
pub(super) fn serialize_md_indented_code(node: &SyntaxNode) -> Vec<String> {
    let code = md_indented_code_text(node);
    let html = encode_text("html");
    vec![
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text("<div class=\"sourceCode\">")
        ),
        format!("(\\preformatted {})", verb_atoms(&code).join(" ")),
        format!(
            "(\\if (TEXT {html}) (\\out (VERB {})))",
            encode_text("</div>")
        ),
    ]
}

/// Strip up to `ncols` leading whitespace **columns** from `text`, whose first
/// character sits at value column `start_col`: spaces are one column, a tab
/// expands to the next 4-column stop ([`advance_md_col`]), and a tab straddling
/// the strip boundary surfaces its leftover columns as spaces — cmark splits a
/// partially consumed tab (`\t\tbar` less six columns is `  bar`, cm-005/007).
/// Stripping stops early at the first non-whitespace character.
fn strip_md_columns(text: &str, start_col: usize, ncols: usize) -> String {
    let target = start_col + ncols;
    let mut col = start_col;
    for (idx, c) in text.char_indices() {
        if col >= target || (c != ' ' && c != '\t') {
            return text[idx..].to_string();
        }
        let next = advance_md_col(col, c);
        if next > target {
            // Only a tab can overshoot: split it into its leftover columns.
            return " ".repeat(next - target) + &text[idx + c.len_utf8()..];
        }
        col = next;
    }
    String::new()
}

/// The dedented verbatim text of a `ROXYGEN_MD_INDENTED_CODE` node — commonmark's
/// `xml_text` for the block, one trailing `\n` per line.
fn md_indented_code_text(node: &SyntaxNode) -> String {
    let text = node.text().to_string();
    // When the block is folded into a list item, roxygen2 strips the item's
    // content column *before* CommonMark's four (the item container consumes its
    // content indentation), so each line loses `content_col + 4` leading columns
    // rather than just four. A section-level block adds nothing (`extra == 0`).
    let extra = md_indented_code_extra_strip(node);
    // A block opening as a tag's same-line value (`@details      x`) has a
    // marker-less first line (the `#'` belongs to the enclosing tag): roxygen2
    // strips only the single separator space there — `strip_marker` would trim
    // the whole run, eating the code's semantic indent. A mid-line block inside
    // a list item (cm-275/276) likewise starts marker-less, but its columns are
    // absolute: the strip anchors at the item marker's end column, covering the
    // one separator column plus CommonMark's four (a separator tab spans to its
    // stop, cm-007).
    let from_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let in_item_col = node
        .parent()
        .filter(|p| p.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
        .and_then(|_| md_item_marker_end_col(node));
    let mut code = String::new();
    for (idx, line) in text.split('\n').enumerate() {
        // `strip_marker` removes the `#'` marker and the single conventional space;
        // the indented code block then consumes up to four further leading columns.
        if idx == 0 && from_value {
            if let Some(marker_end) = in_item_col {
                // Mid-line in an item: one separator column plus four code
                // columns, tab stops anchored at the marker's end column.
                code.push_str(&strip_md_columns(line, marker_end, 5));
            } else {
                // A tag's same-line value: the value is its own markdown
                // document, so drop the single separator character and strip
                // four columns from a fresh column zero.
                let after = line.strip_prefix([' ', '\t']).unwrap_or(line);
                code.push_str(&strip_md_columns(after, 0, 4));
            }
        } else {
            code.push_str(&strip_md_columns(strip_marker(line), 0, 4 + extra));
        }
        code.push('\n');
    }
    code
}

/// The value column just past a `ROXYGEN_MD_INDENTED_CODE` node's enclosing
/// list-item marker — the tab-stop anchor for a mid-line block's first-line
/// strip. `None` when no item marker precedes the node.
fn md_item_marker_end_col(node: &SyntaxNode) -> Option<usize> {
    let item = node
        .parent()
        .filter(|p| p.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)?;
    let marker = item
        .children_with_tokens()
        .find(|el| el.kind() == SyntaxKind::ROXYGEN_MD_LIST_MARKER)
        .and_then(|el| el.into_token())?;
    Some(md_marker_col(&marker) + marker.text().chars().count())
}

/// The number of **extra** leading columns [`serialize_md_indented_code`] must
/// strip when an indented code block is folded into a list item: the item's
/// content column, which CommonMark's item container consumes before the code
/// block's own four columns apply. Zero for a section-level block (no
/// `ROXYGEN_MD_LIST_ITEM` parent).
///
/// The content column is `marker_col + marker_width + content_leading`, all in
/// the markdown-source coordinate roxygen2 works in (the `#'` marker and one
/// conventional space already stripped). `marker_col` is the indentation before
/// the item's marker (its enclosing list's nesting) less that one stripped
/// space; `content_leading` is the spaces between the marker and the item text,
/// clamped to CommonMark's 1..=4.
fn md_indented_code_extra_strip(node: &SyntaxNode) -> usize {
    let Some(item) = node
        .parent()
        .filter(|p| p.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
    else {
        return 0;
    };
    let Some(marker) = item
        .children_with_tokens()
        .find(|el| el.kind() == SyntaxKind::ROXYGEN_MD_LIST_MARKER)
        .and_then(|el| el.into_token())
    else {
        return 0;
    };
    let marker_width = marker.text().chars().count();
    let marker_col = md_marker_col(&marker);
    // The item's content leading spaces (the text right after the marker leaf),
    // clamped to CommonMark's 1..=4 — snapped to one when the marker line's
    // remainder is blank (content on the next line, cm-280/281) or five or more
    // columns deep (the item starts with indented code, cm-275/276), mirroring
    // the builder's `content_leading_spaces`.
    let content_leading = md_item_content_leading(&marker);
    marker_col + marker_width + content_leading
}

/// The value column of a list-item marker token: the gauge of the whitespace
/// run before it (the enclosing list's indentation, less the one whitespace
/// character roxygen2 strips — [`md_ws_gauge`]) converted back to a zero-based
/// value column. Tab stops expand (cm-009's `\t - baz` marker sits at column
/// five).
fn md_marker_col(marker: &SyntaxToken) -> usize {
    let mut texts = Vec::new();
    let mut prev = marker.prev_token();
    while let Some(tok) = prev {
        if tok.kind() != SyntaxKind::WHITESPACE {
            break;
        }
        texts.push(tok.text().to_string());
        prev = tok.prev_token();
    }
    texts.reverse();
    md_ws_gauge(texts.iter().map(String::as_str)).saturating_sub(1)
}

/// A list item's content-leading columns as CommonMark counts them: the
/// whitespace columns between the marker and the first-line content (tab stops
/// anchored at the marker's end column, [`advance_md_col`]), clamped to 1..=4 —
/// one when the first line has no content after the marker or the content sits
/// five or more columns past it (both start conditions snap the content indent
/// to marker + 1). The parser-side twin is `content_leading_spaces` (build.rs).
fn md_item_content_leading(marker: &SyntaxToken) -> usize {
    let start_col = md_marker_col(marker) + marker.text().chars().count();
    let mut leading = None;
    let mut has_content = false;
    let mut tok = marker.next_token();
    while let Some(t) = tok {
        if t.kind() == SyntaxKind::NEWLINE {
            break;
        }
        if leading.is_none() {
            let mut col = start_col;
            for c in t.text().chars().take_while(|c| *c == ' ' || *c == '\t') {
                col = advance_md_col(col, c);
            }
            leading = Some(col - start_col);
        }
        if !t.text().trim().is_empty() {
            has_content = true;
            break;
        }
        tok = t.next_token();
    }
    let leading = leading.unwrap_or(1);
    if !has_content || leading >= 5 {
        return 1;
    }
    leading.clamp(1, 4)
}

/// Project a `ROXYGEN_MD_HTML_BLOCK` node into roxygen2's `\if{html}{\out{…}}`
/// (`mdxml_html_block`, `R/markdown.R`): the block's verbatim text — a leading
/// newline then each line with a trailing newline — goes into a single `\out`
/// inside an `\if{html}{…}`. parse_Rd splits the verbatim `\out` body at newlines
/// into one `VERB` per line (the leading `\n` becomes a bare `(VERB "\n")`). The
/// block lines are reconstructed from the node text with each `#'` marker and the
/// single following space stripped (like the fenced code block).
pub(super) fn serialize_md_html_block(node: &SyntaxNode) -> String {
    let mut body = String::from("\n");
    for ch in md_html_block_field_text(node).chars() {
        body.push(if ch == SOFT_BREAK { '\n' } else { ch });
    }
    body.push('\n');
    format!(
        "(\\if (TEXT {}) (\\out {}))",
        encode_text("html"),
        verb_atoms(&body).join(" ")
    )
}

/// A `ROXYGEN_MD_HTML_BLOCK`'s lines as roxygen2's *field text* sees them — each
/// `#'` marker and its single following space stripped, lines joined on the
/// soft-break sentinel (blank block lines survive as empty lines). This is what
/// `get_md_linkrefs` scans (the candidate regex runs on the whole raw field,
/// markup included), so the link-reference skeleton pushes it verbatim; the
/// `\if{html}{\out{…}}` serialization re-joins it on real newlines.
pub(super) fn md_html_block_field_text(node: &SyntaxNode) -> String {
    let text = node.text().to_string();
    // A block opening as a tag's same-line value has a marker-less first line
    // (the `#'` belongs to the enclosing tag): roxygen2 strips only the single
    // separator space after the tag head, so drop exactly one leading whitespace
    // char and keep any further indent (it is part of the rendered line —
    // `strip_marker` would trim the whole run).
    let mid_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let mut out = String::new();
    for (idx, line) in text.split('\n').enumerate() {
        if idx > 0 {
            out.push(SOFT_BREAK);
        }
        if idx == 0 && mid_value {
            out.push_str(line.strip_prefix([' ', '\t']).unwrap_or(line));
        } else {
            out.push_str(strip_marker(line));
        }
    }
    out
}

/// The **un-normalized** flattened plain text of a `ROXYGEN_MD_BLOCK_QUOTE`.
/// roxygen2 has no block-quote support (`mdxml_unsupported`, `R/markdown.R`): it
/// warns, then renders `escape_comment(xml_text(node))` — the concatenation of
/// every descendant text node of cmark's **parsed** quote body, with the `>`
/// markers and all markup (block *and* inline: nested quote markers, heading
/// `#`s, list bullets, fence lines, emphasis delimiters) dropped and **no
/// separator** between blocks or lines (softbreaks and paragraph breaks
/// contribute nothing; code blocks keep their literal content and newlines).
///
/// The projection mirrors that pipeline: each line drops its `#'` marker and
/// **one** quote level ([`strip_one_quote_level`]; a lazy line keeps its content,
/// including indentation), the stripped body **re-parses as a fresh `@md`
/// roxygen fragment through the real parser**, and the resulting block tree
/// flattens recursively ([`quote_flat_section`]) — a nested quote strips its next
/// level on recursion. A *lazy* setext underline glues onto the previous line at
/// synthesis (it is paragraph text in the quote; re-emitted on its own line it
/// would spuriously promote a heading). A body whose re-parse yields any tag
/// section besides the synthesized `@md` (an `@`-opening line inside the quote —
/// literal text to cmark) withholds the re-parse and falls back to the legacy
/// per-line flatten.
///
/// Returned raw (not wrapped in a `(TEXT …)` atom, not whitespace-normalized) so
/// [`serialize_inlines`] can push it as a `Final` segment that glues onto adjacent
/// prose (roxygen2 emits no `\n\n` before a quote, so `before` + `> q` renders
/// `beforeq`), deferring the single `norm_ws` to the coalesced atom.
///
/// Known limits (unpinned backlog): an inner Rd macro contributes nothing
/// (roxygen2 keeps its placeholder-swapped source); an opaque link/image/HTML
/// leaf contributes nothing (`xml_text` keeps its display/alt/raw text); trailing
/// spaces on a prose line survive (cmark strips them).
pub(super) fn block_quote_flat_text(node: &SyntaxNode) -> String {
    let lines = quote_stripped_lines(node);
    quote_flat_reparse(&lines).unwrap_or_else(|| {
        // Legacy per-line flatten: each line's markdown resolves as an inline
        // fragment; block structure stays literal.
        let mut flat = String::new();
        for line in &lines {
            let inlines = resolve_macro_arg_inlines(line);
            for ch in inline_plain_text(&inlines).chars() {
                if ch != SOFT_BREAK {
                    flat.push(ch);
                }
            }
        }
        flat
    })
}

/// The one-level-stripped body lines of a `ROXYGEN_MD_BLOCK_QUOTE` node: each
/// line drops its `#'` marker, the enclosing item's container columns
/// ([`md_indented_code_extra_strip`]), and one quote level
/// ([`strip_one_quote_level`]; a lazy line keeps its content, a lazy
/// setext-underline-shaped one glues onto the previous line — see
/// [`block_quote_flat_text`]). Shared by the flatten and the quote's
/// link-reference-definition scan, so both read the same synthesized body.
pub(super) fn quote_stripped_lines(node: &SyntaxNode) -> Vec<String> {
    let text = node.text().to_string();
    // A quote folded into a list item carries the item's content column on every
    // line; the item container consumes it before the quote marker is read (the
    // same coordinate shift as an in-item indented code block).
    let container = md_indented_code_extra_strip(node);
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let content = strip_marker(line);
        let consumed = content
            .bytes()
            .take(container)
            .take_while(|&b| b == b' ')
            .count();
        let content = &content[consumed..];
        match strip_one_quote_level(content) {
            Some(inner) => lines.push(inner.to_string()),
            // A lazy continuation line: paragraph text of the quote's open
            // paragraph. A setext-underline-shaped one must not be re-emitted at
            // line start (the re-parse would promote the paragraph into a
            // heading, dropping the underline text); cmark reads it as
            // continuation text, so glue it onto the previous line — softbreaks
            // contribute nothing to `xml_text`, so the glue is separator-free.
            None if is_lazy_setext_shape(content) && !lines.is_empty() => {
                let last = lines.last_mut().expect("checked non-empty");
                last.push_str(content.trim_start_matches(' '));
            }
            None => lines.push(content.to_string()),
        }
    }
    lines
}

/// Strip **one** block-quote marker level from a quote line's content: up to
/// three leading spaces, the `>`, then one optional space. Returns `None` for a
/// lazy continuation line (no `>` within the first three columns), whose content
/// — indentation included — is paragraph text and must survive unchanged.
fn strip_one_quote_level(content: &str) -> Option<&str> {
    let b = content.as_bytes();
    let mut j = 0;
    while j < 3 && b.get(j) == Some(&b' ') {
        j += 1;
    }
    if b.get(j) != Some(&b'>') {
        return None;
    }
    j += 1;
    if b.get(j) == Some(&b' ') {
        j += 1;
    }
    Some(&content[j..])
}

/// Whether a lazy continuation line is setext-underline-shaped — a run of `=`, or
/// a dash run too short to be a thematic break (`--`), with only trailing
/// whitespace. Mirrors the parser's lazy-setext fold (`finish_md_block_quote`):
/// only such lines fold into a quote without a `>` marker while *not* being plain
/// paragraph prose to a fresh re-parse.
fn is_lazy_setext_shape(content: &str) -> bool {
    let t = content.trim_start_matches(' ');
    let ch = match t.as_bytes().first() {
        Some(&c @ (b'=' | b'-')) => c,
        _ => return false,
    };
    let run = t.bytes().take_while(|&b| b == ch).count();
    if !t[run..].trim().is_empty() {
        return false;
    }
    // `---` (three or more dashes, no interior spaces here) is a thematic break,
    // which the parser never folds lazily — it cannot reach this path.
    !(ch == b'-' && run >= 3)
}

/// Re-parse a quote's one-level-stripped body as a fresh `@md` roxygen fragment
/// and flatten its block tree ([`quote_flat_section`]). Returns `None` when the
/// re-parse yields any tag section besides the synthesized `@md` (an `@`-opening
/// body line — cmark reads it as literal text, so the re-parse mis-sections it).
pub(super) fn quote_flat_reparse(lines: &[String]) -> Option<String> {
    let block = quote_synthesized_block(lines)?;
    let mut flat = String::new();
    for section in block.sections() {
        quote_flat_section(&section, &mut flat);
    }
    Some(flat)
}

/// Parse a quote's one-level-stripped body lines as a fresh synthesized `@md`
/// roxygen fragment, returning its `ROXYGEN_BLOCK` (the rowan node keeps the
/// re-parsed tree alive). `None` withholds the reparse: the body yields a tag
/// section besides the synthesized `@md` (an `@`-opening line is literal text
/// to cmark, so the re-parse mis-sections it). Shared by the flatten
/// ([`quote_flat_reparse`]) and the quote's link-reference-definition scan
/// ([`collect_user_linkrefs_tree`]).
pub(super) fn quote_synthesized_block(lines: &[String]) -> Option<RoxygenBlock> {
    let mut src = String::from("#' @md\n");
    for line in lines {
        if line.is_empty() {
            src.push_str("#'\n");
        } else {
            src.push_str("#' ");
            src.push_str(line);
            src.push('\n');
        }
    }
    let cst = crate::parser::parse(&src).cst;
    let block = cst.descendants().find_map(RoxygenBlock::cast)?;
    if block.sections().filter(|s| s.tag().is_some()).count() != 1 {
        return None;
    }
    Some(block)
}

/// Flatten one re-parsed section of a quote body: every block child contributes
/// its `xml_text` — prose units via [`quote_flat_unit`], block nodes via
/// [`quote_flat_node`] — glued with no separator. The synthesized `@md` tag node
/// contributes nothing.
fn quote_flat_section(section: &RoxygenSection, out: &mut String) {
    for el in section.syntax().children_with_tokens() {
        let NodeOrToken::Node(node) = el else {
            continue;
        };
        match node.kind() {
            SyntaxKind::ROXYGEN_TAG => {}
            SyntaxKind::ROXYGEN_PARAGRAPH => {
                if let Some(p) = RoxygenParagraph::cast(node) {
                    out.push_str(&quote_flat_unit(&consume_linkref_defs(paragraph_inlines(
                        &p,
                    ))));
                }
            }
            _ => quote_flat_node(&node, out),
        }
    }
}

/// Flatten one re-parsed block node of a quote body to its `xml_text`.
fn quote_flat_node(node: &SyntaxNode, out: &mut String) {
    match node.kind() {
        SyntaxKind::ROXYGEN_MD_LIST => {
            for item in node
                .children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
            {
                out.push_str(&quote_flat_unit(&consume_linkref_defs(
                    md_list_item_inlines(&item),
                )));
            }
        }
        SyntaxKind::ROXYGEN_MD_HEADING => {
            let (_, title) = parse_md_heading(node);
            out.push_str(&quote_flat_unit(&resolve_macro_arg_inlines(&title)));
        }
        SyntaxKind::ROXYGEN_MD_BLOCK_QUOTE => out.push_str(&block_quote_flat_text(node)),
        SyntaxKind::ROXYGEN_MD_CODE_BLOCK => {
            let (_, code) = md_code_block_parts(node);
            out.push_str(&code);
        }
        SyntaxKind::ROXYGEN_MD_INDENTED_CODE => out.push_str(&md_indented_code_text(node)),
        // `xml_text` over a table is its cell text glued in source order; the
        // delimiter row is table syntax, not content.
        SyntaxKind::ROXYGEN_MD_TABLE => {
            let text = node.text().to_string();
            for (idx, line) in text.split('\n').enumerate() {
                if idx == 1 {
                    continue; // delimiter row
                }
                for cell in split_table_row_cells(strip_marker(line)) {
                    let content = unescape_table_pipes(cell.trim());
                    out.push_str(&quote_flat_unit(&resolve_macro_arg_inlines(&content)));
                }
            }
        }
        // An HTML block's `xml_text` is its raw content, one line each.
        SyntaxKind::ROXYGEN_MD_HTML_BLOCK => {
            let text = node.text().to_string();
            for line in text.split('\n') {
                out.push_str(strip_marker(line));
                out.push('\n');
            }
        }
        // A thematic break renders empty; an Rd macro contributes nothing
        // (roxygen2 keeps its placeholder-swapped source — deferred backlog).
        _ => {}
    }
}

/// Flatten one prose unit (a paragraph, list item, or heading title) to its
/// `xml_text`: inline markup contributes its textual content
/// ([`quote_flat_inlines`]), then every soft line break — with the line-trailing
/// whitespace before it and the continuation indent after it — collapses to
/// nothing, and the unit's edges trim (cmark strips a line's leading and trailing
/// whitespace; breaks contribute nothing to `xml_text`).
fn quote_flat_unit(inlines: &[Inline]) -> String {
    let mut s = String::new();
    quote_flat_inlines(inlines, &mut s);
    let mut out = String::new();
    let mut pending_ws = String::new();
    let mut after_break = false;
    for ch in s.chars() {
        match ch {
            ' ' | '\t' => pending_ws.push(ch),
            SOFT_BREAK => {
                pending_ws.clear();
                after_break = true;
            }
            _ => {
                if !after_break {
                    out.push_str(&pending_ws);
                }
                pending_ws.clear();
                after_break = false;
                out.push(ch);
            }
        }
    }
    out.trim_matches([' ', '\t', '\n']).to_string()
}

/// Drop the link-reference-definition runs from a re-parsed quote unit before
/// flattening: cmark consumes a definition inside the quote's own block context,
/// so it contributes nothing to the quote's `xml_text` (cm-220). A `Text` whose
/// leading lines a definition consumed keeps its remainder as prose.
fn consume_linkref_defs(inlines: Vec<Inline>) -> Vec<Inline> {
    let (_, dropped) = collect_user_linkrefs(&inlines);
    if dropped.is_empty() {
        return inlines;
    }
    inlines
        .into_iter()
        .enumerate()
        .filter_map(|(i, inl)| match dropped.get(&i) {
            Some(None) => None,
            Some(Some(leftover)) => Some(Inline::Text(leftover.clone())),
            None => Some(inl),
        })
        .collect()
}

/// Flatten a resolved inline run to its `xml_text`: text and code spans
/// contribute their content (a code span's soft break is a space — cmark converts
/// a code span's interior line ending), emphasis and link nodes their inner text,
/// and a nested block node ([`quote_flat_node`]) its own flattened text. Opaque
/// leaves (an unresolved link/image/HTML leaf, an Rd macro) contribute nothing —
/// the legacy behavior, kept deliberately (see [`block_quote_flat_text`]).
fn quote_flat_inlines(inlines: &[Inline], out: &mut String) {
    for inl in inlines {
        match inl {
            Inline::Text(t) => out.push_str(t),
            Inline::MdCode(t) => {
                for ch in t.chars() {
                    out.push(if ch == SOFT_BREAK { ' ' } else { ch });
                }
            }
            Inline::MdEmphasis { children, .. } => quote_flat_inlines(children, out),
            Inline::MdInlineLink { display, .. }
            | Inline::MdRefLink { display, .. }
            | Inline::MdShortcutLink { display } => quote_flat_inlines(display, out),
            Inline::MdList(n)
            | Inline::MdCodeBlock(n)
            | Inline::MdIndentedCode(n)
            | Inline::MdTable(n)
            | Inline::MdHtmlBlock(n)
            | Inline::MdHeading(n) => quote_flat_node(n, out),
            Inline::MdBlockQuote(n) => out.push_str(&block_quote_flat_text(n)),
            Inline::MdListResolved { items, .. } => {
                for item in items {
                    out.push_str(&quote_flat_unit(item));
                }
            }
            _ => {}
        }
    }
}

/// Per-column alignment of a GFM table, from the delimiter row's colon markers.
#[derive(Clone, Copy)]
enum TableAlign {
    Left,
    Center,
    Right,
}

impl TableAlign {
    /// The `\tabular` format letter (`l`/`c`/`r`).
    fn code(self) -> char {
        match self {
            TableAlign::Left => 'l',
            TableAlign::Center => 'c',
            TableAlign::Right => 'r',
        }
    }
}

/// Project a `ROXYGEN_MD_TABLE` node into roxygen2's `\tabular{<align>}{<cells>}`
/// (`mdxml_table`, `R/markdown-table.R` via cmark-gfm). The delimiter row (the
/// node's second line) supplies the per-column alignment; the header row and every
/// body row fill one `GRP` in source order, each cell's content resolved as a
/// markdown inline run, separated by `\tab` and terminated by `\cr`. A row is
/// padded with empty cells (a `\tab` with no atom) or truncated to the column
/// count, matching cmark-gfm's ragged-row handling.
pub(super) fn serialize_md_table(node: &SyntaxNode) -> String {
    let text = node.text().to_string();
    // A table whose header row opened as a tag's same-line value has a
    // marker-less first line (the `#'` belongs to the enclosing tag): drop only
    // the single separator space there (`strip_marker` would eat a leading `#`
    // in the header's first cell).
    let from_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let mut lines: Vec<&str> = text.split('\n').map(strip_marker).collect();
    if from_value && let Some(first) = text.split('\n').next() {
        lines[0] = first.strip_prefix([' ', '\t']).unwrap_or(first);
    }
    // The header is the first line and the delimiter the second; body rows follow.
    // The gate guarantees both exist, but stay defensive against a malformed node.
    if lines.len() < 2 {
        return "(\\tabular)".to_string();
    }
    let aligns = parse_table_delim(lines[1]);
    let ncol = aligns.len();
    let align_str: String = aligns.iter().map(|a| a.code()).collect();

    let mut grp: Vec<String> = Vec::new();
    let rows = std::iter::once(lines[0]).chain(lines[2..].iter().copied());
    for row in rows {
        let cells = split_table_row_cells(row);
        for c in 0..ncol {
            if c > 0 {
                grp.push("(\\tab)".to_string());
            }
            if let Some(cell) = cells.get(c) {
                let content = unescape_table_pipes(cell.trim());
                grp.extend(serialize_inlines(
                    &resolve_macro_arg_inlines(&content),
                    true,
                ));
            }
        }
        grp.push("(\\cr)".to_string());
    }
    format!(
        "(\\tabular (TEXT {}) (GRP {}))",
        encode_text(&align_str),
        grp.join(" ")
    )
}

/// The per-column alignments of a GFM delimiter row: a leading colon means left,
/// a trailing colon means right, both means center, none means default (left).
fn parse_table_delim(line: &str) -> Vec<TableAlign> {
    split_table_row_cells(line)
        .iter()
        .map(|cell| {
            let t = cell.trim();
            match (t.starts_with(':'), t.ends_with(':')) {
                (true, true) => TableAlign::Center,
                (false, true) => TableAlign::Right,
                _ => TableAlign::Left,
            }
        })
        .collect()
}

/// Unescape a GFM table cell's `\|` to a literal `|` — the one escape the table
/// extension resolves during block parsing, before the cell's inline content is
/// parsed (`x \| y` renders as `x | y`).
fn unescape_table_pipes(cell: &str) -> String {
    cell.replace("\\|", "|")
}

/// The verbatim `(VERB …)` atoms for an `\out` body, splitting at newlines and
/// attaching each `\n` to the atom it ends (parse_Rd's verbatim splitting, the
/// `VERB` analog of [`rcode_atoms`]).
pub(super) fn verb_atoms(body: &str) -> Vec<String> {
    let mut atoms = Vec::new();
    let mut rest = body;
    while let Some(idx) = rest.find('\n') {
        let (seg, tail) = rest.split_at(idx + 1);
        atoms.push(format!("(VERB {})", encode_text(seg)));
        rest = tail;
    }
    if !rest.is_empty() {
        atoms.push(format!("(VERB {})", encode_text(rest)));
    }
    atoms
}

/// Project a raw inline-HTML span into roxygen2's `\if{html}{\out{<tag>}}`
/// (`mdxml_html_inline`, `markdown.R`): the tag text goes verbatim into a `\out`
/// inside an `\if{html}{…}`. roxygen2 `escape_verb`-escapes `}` (→ `\}`) but
/// `parse_Rd` decodes it, so the pin (and thus the projector) carries the raw
/// tag. A **multi-line** span keeps its newlines: `parse_Rd` splits the `\out`
/// body into one VERB leaf per line, each carrying its trailing `\n`
/// (engine-probed: `<!-- x`⏎`y -->` → `(VERB "<!-- x\n") (VERB "y -->")`).
pub(super) fn html_inline_atom(raw: &str) -> String {
    format!(
        "(\\if (TEXT {}) (\\out {}))",
        encode_text("html"),
        html_out_verbs(raw)
    )
}

/// The `\out` body's `(VERB …)` atoms for a raw inline-HTML span, one per line
/// (see [`html_inline_atom`]); shared with the demoted-macro emission, where the
/// `\out` survives inside a bare `LIST` group.
pub(super) fn html_out_verbs(raw: &str) -> String {
    let verbs: Vec<String> = raw
        .split_inclusive('\n')
        .map(|seg| format!("(VERB {})", encode_text(seg)))
        .collect();
    verbs.join(" ")
}

/// Extract a fenced code block's `(info, code)` from its node. The info string is
/// the opener `ROXYGEN_MD_FENCE` leaf with its leading fence run (backticks or
/// tildes) stripped and trimmed (matching commonmark's `info` attribute). The
/// code is every line between the opener and closer fence lines, each with its
/// `#'` marker and the single following space stripped, joined by newlines with a
/// trailing newline (commonmark's `xml_text` for a code block).
pub(super) fn md_code_block_parts(node: &SyntaxNode) -> (String, String) {
    let text = node.text().to_string();
    let lines: Vec<&str> = text.split('\n').collect();
    // The opener is the first line, the closer the last; the code is in between.
    // The opener fence may itself be indented (up to three columns at the top
    // level, or to a list item's content column when the block is folded into an
    // item). CommonMark removes the opener fence's own indentation both before
    // the info string and from up to that many leading columns of each content
    // line, so the leading spaces surviving in the code are `max(0, body_col -
    // fence_col)` — computable from the node text alone (`content_col` cancels).
    let opener = lines.first().map(|l| strip_marker(l)).unwrap_or_default();
    let fence_indent = opener.bytes().take_while(|&b| b == b' ').count();
    let fence = &opener[fence_indent..];
    let info = fence
        .trim_start_matches(fence.chars().next().unwrap_or('`'))
        .trim()
        .to_string();
    // A fence opening mid-line on a list item's marker line (`- ```` ``` ````,
    // cm-320/326) has a marker-less first line starting at the fence itself, so
    // the opener carries none of the item's content-column indent and the
    // cancellation above breaks: the container strip and the closer window key
    // off the item's content column instead ([`md_indented_code_extra_strip`] —
    // zero for the tag-value shape, which has no list-item parent).
    let from_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let container = if from_value {
        md_indented_code_extra_strip(node)
    } else {
        0
    };
    let fence_indent = fence_indent + container;
    // An *unterminated* block (the builder stopped at a tag opener or the block
    // end without a matching closer) ends on a content line, not a closing
    // fence: mirror the builder's closer test (same character, run length >=
    // the opener's, no info string, at most three columns further indented) to
    // decide whether the last line is the closer (dropped) or code (kept).
    let has_closer = lines.len() > 1
        && lines.last().is_some_and(|l| {
            let content = strip_marker(l);
            let indent = content.bytes().take_while(|&b| b == b' ').count();
            indent <= fence_indent + 3 && md_fence_run_closes(fence, &content[indent..])
        });
    let body = if has_closer {
        &lines[1..lines.len() - 1]
    } else {
        &lines[1..]
    };
    let mut code = String::new();
    for line in body {
        let after_marker = strip_marker(line);
        // Strip up to the opener fence's indentation, never past what the line has.
        let strip = after_marker
            .bytes()
            .take_while(|&b| b == b' ')
            .count()
            .min(fence_indent);
        code.push_str(&after_marker[strip..]);
        code.push('\n');
    }
    (info, code)
}

/// Strip a `#'` line's marker prefix and the single following space, returning the
/// line's content. Tolerates leading indentation before the marker (inter-line
/// trivia) and a multi-`#` marker.
pub(super) fn strip_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    let after_hashes = trimmed.trim_start_matches('#');
    let body = after_hashes.strip_prefix('\'').unwrap_or(after_hashes);
    // roxygen2's tokenizer strips one whitespace *character* after the sigil
    // (`consumeWhitespace(1)`, parser2.cpp) — a tab counts, not just a space.
    body.strip_prefix([' ', '\t']).unwrap_or(body)
}

/// Whether a `ROXYGEN_MD_LIST` is ordered (`\enumerate`): its first item's
/// `ROXYGEN_MD_LIST_MARKER` begins with a digit (`1.`/`1)`), as opposed to a
/// bullet (`-`/`*`/`+`).
pub(super) fn md_list_is_ordered(node: &SyntaxNode) -> bool {
    // Only this list's own items decide its kind — a nested sublist's markers
    // (its own `\itemize`/`\enumerate`) must not flip the parent, so look at the
    // first *direct* item's marker, not any descendant marker.
    node.children()
        .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
        .find_map(|item| {
            item.children_with_tokens()
                .filter_map(|el| el.into_token())
                .find(|t| t.kind() == SyntaxKind::ROXYGEN_MD_LIST_MARKER)
        })
        .is_some_and(|t| t.text().starts_with(|c: char| c.is_ascii_digit()))
}

/// The inline elements of a markdown list item: its content after the marker
/// leaf, with the threaded `#'` markers dropped and inter-line newlines turned
/// into joining spaces (the same treatment as a prose paragraph). Only the
/// **first** `ROXYGEN_MD_LIST_MARKER` leaf is the item bullet; a later one is a
/// lazily folded over-indented marker line (would-be indented code cannot
/// interrupt the item's paragraph, cm-314), which roxygen2 renders as literal
/// text — the generic inline fallback handles it.
pub(super) fn md_list_item_inlines(item: &SyntaxNode) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut bullet_seen = false;
    for el in item.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MD_LIST_MARKER if !bullet_seen => bullet_seen = true,
            SyntaxKind::ROXYGEN_MARKER => {}
            SyntaxKind::NEWLINE => out.push(Inline::Text(SOFT_BREAK.to_string())),
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// roxygen2 renders a markdown code span as `\code{…}` when its content parses as
/// a single R expression (or is one of a fixed set of operator/keyword tokens),
/// and `\verb{…}` otherwise (`R/markdown.R`'s `mdxml_code`/`can_parse`). The
/// projector replicates the decision with arity's own parser: parseable ⇒
/// `(\code (RCODE …))`, else `(\verb (VERB …))`. Both bodies are verbatim (no
/// whitespace collapse).
///
/// A span whose content opens with `Rd ` is roxygen2 8.0.0's render-time
/// expression syntax, checked *before* the `can_parse` decision: it renders
/// `\Sexpr[stage=render,results=rd]{<rest>}` with the rest verbatim — no `%`
/// escaping, no evaluation (the evaluation happens when the Rd renders, so the
/// construct is fully static, unlike the knitr-evaluated `` `r …` `` form).
/// The `[…]` option is dropped by the section serializer ([`sexpr_atom`]).
pub(super) fn md_code_atom(content: &str) -> String {
    if let Some(body) = content.strip_prefix("Rd ") {
        return sexpr_atom(body);
    }
    if code_span_is_r(content) {
        format!("(\\code (RCODE {}))", encode_text(content))
    } else {
        format!("(\\verb (VERB {}))", encode_text(content))
    }
}

/// The `(\Sexpr …)` atom for an `` `Rd expr` `` span's body, mirroring what
/// parse_Rd yields for `\Sexpr[…]{body}`. The body is R-code context, so plain
/// text is a verbatim `RCODE` leaf — but parse_Rd still recognizes a `\word{…}`
/// macro inside R code, the macro node carrying a verbatim argument. In a run
/// of backslashes before `word{`, only the **last** one opens the macro; the
/// rest stay in the code text (engine-probed: `` `Rd \eqn{x}` `` →
/// `(\Sexpr (\eqn (VERB "x")))`, `` `Rd \\eqn{x}` `` → `(\Sexpr (RCODE "\")
/// (\eqn (VERB "x")))`). A backslash not opening a `word{…}` shape stays in
/// the code text.
fn sexpr_atom(body: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut text = String::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && let Some((name_end, end)) = rd_code_macro_span(body, i)
        {
            if !text.is_empty() {
                parts.push(format!("(RCODE {})", encode_text(&text)));
                text.clear();
            }
            let name = &body[i + 1..name_end];
            let arg = &body[name_end + 1..end - 1];
            parts.push(format!("(\\{name} (VERB {}))", encode_text(arg)));
            i = end;
        } else {
            // Advance one char; multi-byte chars never start with `\`.
            let ch_len = body[i..].chars().next().map_or(1, char::len_utf8);
            text.push_str(&body[i..i + ch_len]);
            i += ch_len;
        }
    }
    if !text.is_empty() {
        parts.push(format!("(RCODE {})", encode_text(&text)));
    }
    if parts.is_empty() {
        "(\\Sexpr)".to_string()
    } else {
        format!("(\\Sexpr {})", parts.join(" "))
    }
}

/// The `(name end, span end)` of a `\word{…}` Rd macro opening at `body[i]`
/// (which must be a `\`): the name's exclusive end and the index just past the
/// balanced closing `}`. `None` when the shape does not match: no
/// `[A-Za-z][A-Za-z0-9]*` name, no `{` after it, or an unbalanced brace run.
fn rd_code_macro_span(body: &str, i: usize) -> Option<(usize, usize)> {
    let bytes = body.as_bytes();
    let mut name_end = i + 1;
    if !bytes.get(name_end).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    while bytes.get(name_end).is_some_and(u8::is_ascii_alphanumeric) {
        name_end += 1;
    }
    if bytes.get(name_end) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut j = name_end;
    while j < bytes.len() {
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((name_end, j + 1));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Operator and keyword tokens roxygen2's `can_parse` treats as `\code` even
/// though they are not complete expressions on their own (`R/markdown.R`'s
/// `special`).
const SPECIAL_CODE: &[&str] = &[
    "-", ":", "::", ":::", "!", "!=", "(", "[", "[[", "@", "*", "/", "&", "&&", "%*%", "%/%", "%%",
    "%in%", "%o%", "%x%", "^", "+", "<", "<=", "=", "==", ">", ">=", "|", "||", "~", "$", "for",
    "function", "if", "repeat", "while",
];

/// Whether `code` parses as a single R expression, the way roxygen2's `can_parse`
/// (rlang's `parse_expr`) does: exactly one complete top-level expression with no
/// parse diagnostics, or a `special` token. arity's lenient recovery would accept
/// two adjacent symbols (`inline code`) as two expressions, so the one-expression
/// count is what discriminates `\code` from `\verb`.
pub(super) fn code_span_is_r(code: &str) -> bool {
    if SPECIAL_CODE.contains(&code) {
        return true;
    }
    let out = crate::parser::parse(code);
    if !out.diagnostics.is_empty() {
        return false;
    }
    let one_expr = out
        .cst
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
            )
        })
        .count()
        == 1;
    one_expr && !has_invalid_name(&out.cst)
}

/// Name shapes R's parser rejects but arity's more lenient lexer accepts as
/// ordinary identifiers — screened out here to mirror roxygen2's `can_parse`.
/// A `_`-leading name errors in R's lexer, with one exception: a lone `_` used
/// as the native-pipe placeholder, valid only inside a `|>` pipeline (a
/// `_`-leading name of length ≥ 2 is never valid). A zero-length backquoted
/// name `` `` `` errors at parse time ("attempt to use zero-length variable
/// name"); any non-empty backquoted name is valid.
fn has_invalid_name(cst: &SyntaxNode) -> bool {
    let has_pipe = cst
        .descendants_with_tokens()
        .any(|el| el.kind() == SyntaxKind::PIPE);
    cst.descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .filter(|t| t.kind() == SyntaxKind::IDENT)
        .any(|t| {
            let text = t.text();
            text == "``" || (text.starts_with('_') && (text.len() > 1 || !has_pipe))
        })
}
