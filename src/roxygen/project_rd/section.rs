use super::*;

/// One topic's worth of sections from a single roxygen block.
///
/// The block already owns logical structure: its children are `ROXYGEN_SECTION`s
/// (the intro, then one per `@tag`), each holding a `ROXYGEN_TAG` heading and/or
/// `ROXYGEN_PARAGRAPH`s. So the projector is a direct walk — the line-reassembly
/// state machine the line-flat CST forced is gone. A tag section's body is the
/// tag's own inline prose followed by its paragraphs (continuation and
/// paragraph-break both collapse to a single space under `norm_ws`).
/// The block's resolved markdown mode, mirroring the lexer's
/// `resolve_roxygen_block`: start from the caller's package-wide default, then
/// let a standalone `@md` turn it on or `@noMd` turn it off, with the last
/// directive winning. A directive is a tag named `md`/`noMd` with no argument
/// or prose value (roxygen2 errors on a directive line carrying other content).
fn block_md(block: &RoxygenBlock, md_default: bool) -> bool {
    let mut md = md_default;
    for section in block.sections() {
        if let Some(tag) = section.tag()
            && tag.arg().is_none()
            && tag.text().is_none()
        {
            match tag.name().as_deref() {
                Some("md") => md = true,
                Some("noMd") => md = false,
                _ => {}
            }
        }
    }
    md
}

pub(super) fn project_block(block: &RoxygenBlock, out: &mut Vec<String>, md_default: bool) {
    project_block_impl(block, out, true, md_default);
}

/// Project a single block's sections into `out`. `apply_title_fallback` gates
/// roxygen2's title-as-description fallback (`topics_add_default_description`):
/// on for a standalone topic (the public [`project_block`]), **off** when the
/// block is one member of a merged topic — there the fallback runs once on the
/// *merged* title value vector (see [`project_merged_topic`]), not per block.
pub(super) fn project_block_impl(
    block: &RoxygenBlock,
    out: &mut Vec<String>,
    apply_title_fallback: bool,
    md_default: bool,
) {
    // Resolve the block's markdown mode the way the lexer's `resolve_roxygen_block`
    // does (start from the caller's default; a standalone `@md`/`@noMd`
    // directive line wins, with the last one taking precedence).
    // Plain prose text leaves carry no mode (their kind is identical in both modes),
    // so the projector re-derives it here: it keys whether prose is literal Rd
    // (where an unescaped `%` is a comment) or escaped markdown (where it survives).
    let md = block_md(block, md_default);
    // The section strings this block contributes; a `@md` block's `TEXT` leaves get
    // their escaped braces (`\{`/`\}`) resolved to bare braces once every section
    // (and its `rdComplete` drop decision) is built (see `resolve_md_text_braces`).
    let block_start = out.len();
    let mut intro_paras: Vec<Vec<Inline>> = Vec::new();
    let mut tag_sections: Vec<(String, Vec<Inline>)> = Vec::new();
    // `@slot` (S4) and `@field` (reference class) each aggregate every tag of a
    // topic into one Slots/Fields section, so they are collected here as
    // (name, definition) pairs rather than projected per-tag.
    let mut slots: Vec<(String, Vec<Inline>)> = Vec::new();
    let mut fields: Vec<(String, Vec<Inline>)> = Vec::new();
    // `@examples`/`@examplesIf` is an aggregating field: every examples tag of a
    // topic concatenates into a single `\examples` section. The body is
    // reformatted R, so the projector only records *that* one exists.
    let mut has_examples = false;
    // Every tag name the block carries, for the recovery pass's safe-tag gate
    // (see `parse_rd_recovery`).
    let mut tag_names: Vec<String> = Vec::new();

    for section in block.sections() {
        if let Some(tag) = section.tag() {
            let name = tag.name().map(|n| n.to_string()).unwrap_or_default();
            tag_names.push(name.clone());
            let mut body = tag_inlines(&tag);
            for part in section_body_parts(&section) {
                // A part that leads with a block quote carries no separator: its
                // flattened text glues onto whatever precedes (roxygen2 emits no
                // paragraph break around an unsupported block quote). Any other part
                // is a fresh roxygen paragraph, joined by a line break (norm_ws
                // collapses it to a space, but it bounds an Rd `%` comment in
                // non-markdown prose).
                if !body.is_empty() && !matches!(part.first(), Some(Inline::MdBlockQuote(_))) {
                    body.push(Inline::Text("\n".to_string()));
                }
                body.extend(part);
            }
            match name.as_str() {
                "slot" | "field" => {
                    // roxygen2 parses `@slot`/`@field` with `tag_two_part`, which
                    // runs `rdComplete(x$raw, is_code = FALSE)` on the *raw* tag
                    // value (name + description, before markdown) and drops the
                    // whole tag to NULL on a brace imbalance — mode-independently,
                    // so a dropped tag contributes no `\describe` item (and an
                    // all-dropped Slots/Fields aggregate emits no section at all).
                    // The raw value's only rd_complete-relevant chars (`{}`, `\`,
                    // `%`) never appear in the `#'`/`@slot` scaffolding, and
                    // `is_code = FALSE` ignores quotes, so the section's full source
                    // text scans identically to roxygen2's `x$raw`.
                    if !rd_complete(&section.syntax().text().to_string()) {
                        continue;
                    }
                    // A backtick-quoted name may contain spaces; roxygen2's
                    // `split_two_part` strips the backticks (8.0.0, so the name
                    // matches `names(formals(fn))`).
                    let mut arg = tag.arg().map(|t| t.text().to_string()).unwrap_or_default();
                    if arg.len() >= 2 && arg.starts_with('`') && arg.ends_with('`') {
                        arg = arg[1..arg.len() - 1].to_string();
                    }
                    if name == "slot" {
                        slots.push((arg, body));
                    } else {
                        fields.push((arg, body));
                    }
                }
                // `@section` uses `tag_markdown` (`sections = FALSE`), so it never
                // gets the per-section `rdComplete` drop under markdown. But in
                // markdown-OFF mode `markdown_if_active`'s else-branch runs
                // `rdComplete(x$raw)` unconditionally on the whole `title: body`
                // value and replaces it with "" on a brace imbalance. `roxy_tag_rd`
                // then splits "" on its first `:` (`re_split_half`) → title=""
                // and content="", rendering `\section{}{}` → a childless
                // `(\section)`. As with `@slot`/`@field`, the raw value's only
                // rd_complete-relevant chars (`{}`, `\`, `%`) never appear in the
                // scaffolding and `is_code = FALSE` ignores quotes, so the full
                // section source scans identically to `x$raw`.
                "section" if !md && !rd_complete(&section.syntax().text().to_string()) => {
                    out.push("(\\section)".to_string());
                }
                "examples" | "examplesIf" => has_examples = true,
                _ => tag_sections.push((name, body)),
            }
        } else {
            intro_paras.extend(section_body_parts(&section));
        }
    }

    // roxygen2's `parse_description` (R/block.R) splits the intro prose by
    // paragraph: 1st = title, 2nd = description, the rest = details (merged with
    // any explicit @details). A tag whose value is the literal "NULL" is the
    // `rd_section()` suppression sentinel (`R/field.R`), so it does not count as
    // an explicit title/description — a suppressed `@description NULL` re-triggers
    // the title-as-description fallback (`topics_add_default_description`).
    let has_explicit_title = tag_sections
        .iter()
        .any(|(n, b)| n == "title" && !is_null_section(b, md));
    let has_explicit_desc = tag_sections
        .iter()
        .any(|(n, b)| n == "description" && !is_null_section(b, md));
    let explicit_title_body = tag_sections
        .iter()
        .find(|(n, b)| n == "title" && !is_null_section(b, md))
        .map(|(_, b)| b.clone());

    // 1st intro paragraph = title. An explicit @title claims the role and leaves
    // the intro paragraphs to shift down into description/details.
    let mut cursor = 0usize;
    let intro_title = if has_explicit_title {
        None
    } else {
        intro_paras.get(cursor).inspect(|_| cursor += 1).cloned()
    };
    // 2nd intro paragraph = description (unless an explicit @description claims it).
    let intro_desc = if has_explicit_desc {
        None
    } else {
        intro_paras.get(cursor).inspect(|_| cursor += 1).cloned()
    };
    let intro_details = &intro_paras[cursor..];
    let merge_details = !intro_details.is_empty();

    if let Some(title) = &intro_title {
        // The intro title is re-emitted as a `tag_markdown` (`sections = FALSE`)
        // tag, which does not get the per-section `rdComplete` drop.
        push_section(out, "title", title, md, false);
    }

    // Description: the intro's 2nd paragraph, else roxygen2's
    // title-as-description fallback — when no description exists anywhere, the
    // title value (intro title, else explicit @title) is reused.
    let explicit_titles = tag_sections
        .iter()
        .filter(|(n, b)| n == "title" && !is_null_section(b, md))
        .count();
    let description = match intro_desc {
        Some(d) => Some(d),
        None if has_explicit_desc => None, // emitted by the tag loop below
        None if !apply_title_fallback => None, // fallback deferred to the merged topic
        // Repeated `@title`: the fallback joins every *rendered* title value,
        // which only exists after the tag loop — deferred to the post-hoc site.
        None if explicit_titles > 1 => None,
        None => intro_title.clone().or_else(|| explicit_title_body.clone()),
    };
    if let Some(description) = description {
        emit_section_with_headings(out, "description", &description, md, true);
    }

    // The intro-derived details (and any folded-in @details).
    if merge_details {
        let mut body = join_paras(intro_details);
        for (_, ed) in tag_sections.iter().filter(|(n, _)| n == "details") {
            body.push(Inline::Text("\n".to_string()));
            body.extend(join_paras(std::slice::from_ref(ed)));
        }
        emit_section_with_headings(out, "details", &body, md, true);
    }

    for (name, body) in &tag_sections {
        if merge_details && name == "details" {
            continue;
        }
        project_tag_section(name, body, out, md);
    }

    let title_inners = collapse_same_head_sections(out, block_start);

    // roxygen2's `topics_add_default_description` runs *after* the roclet has
    // processed every tag, so an explicit `@description` whose content hoisted
    // entirely into top-level `\section`s (a leading `# heading`, line-start or
    // as the tag's same-line value) leaves the topic without a `\description`
    // and the title fallback re-fires. A *dropped* description (`rdComplete`)
    // still emits an empty `(\description)` atom — the section object exists —
    // so it correctly suppresses the fallback here.
    // The presence check is scoped to *this block's* sections — the fallback is
    // per-topic, so an earlier topic's `\description` in the same file must not
    // suppress it.
    if apply_title_fallback
        && !out[block_start..]
            .iter()
            .any(|s| s.starts_with("(\\description"))
    {
        if title_inners.len() > 1 {
            // Repeated `@title`: `topics_add_default_description` reuses the WHOLE
            // title value vector, so the fallback description collapses every
            // rendered title value even though `\title` itself keeps only the first.
            let inner =
                collapse_inners(&title_inners.iter().map(String::as_str).collect::<Vec<_>>());
            out.push(if inner.is_empty() {
                "(\\description)".to_string()
            } else {
                format!("(\\description {inner})")
            });
        } else if let Some(title) = intro_title.as_ref().or(explicit_title_body.as_ref()) {
            emit_section_with_headings(out, "description", title, md, true);
        }
    }

    // The aggregated `@slot`/`@field` sections (roxygen2's Slots/Fields).
    if !slots.is_empty() {
        out.push(describe_section("Slots", &slots, md));
    }
    if !fields.is_empty() {
        out.push(describe_section("Fields", &fields, md));
    }

    // The single aggregated `\examples` section (body reformatted R → placeholder).
    if has_examples {
        out.push("(\\examples ...)".to_string());
    }

    // A kept-incomplete `@md` prose section's imbalance reaches parse_Rd, whose
    // error recovery restructures the affected tail of the Rd file (see
    // `parse_rd_recovery`). Runs on the standalone-topic path only: for a
    // merged-topic member the recovery belongs to the *merged* file, which the
    // pass does not model (a member's incomplete section stays backlog).
    if apply_title_fallback {
        parse_rd_recovery(out, block_start, md, &tag_names);
    }

    // parse_Rd renders a `@md` prose escape `\{`/`\}` as a bare brace in the final
    // TEXT node. This runs after every section (and its `rdComplete` drop) is built
    // so the drop scan still weighs the escaped brace (an unbalanced *escaped* brace
    // does not drop the section), while the rendered TEXT carries the bare brace.
    if md {
        for section in &mut out[block_start..] {
            *section = resolve_md_text_braces(section);
        }
    }
}

/// The prose section heads roxygen2 formats with `format_collapse` (its merged
/// value vector joined by a paragraph break into a single macro). Every other
/// prose head keeps only the first value (`\title` → `format_first`); non-prose
/// aggregates (`\section`, `\examples`, Slots/Fields) fall to the keep-all arm.
const COLLAPSE_HEADS: &[&str] = &[
    "description",
    "details",
    "seealso",
    "value",
    "note",
    "references",
    "author",
    "format",
    "source",
];

/// Merge repeated same-head sections *within* one block's projected run
/// (`out[block_start..]`), the way `RoxyTopic$add` + the per-type `format`
/// render them: repeated [`COLLAPSE_HEADS`] sections join into one macro
/// (`format_collapse`, first occurrence anchors the merged section), repeated
/// `\title`s keep the first (`format_first`). Every other head — hoisted or
/// explicit `\section`s, `@rawRd`'s bare top-level atoms, aggregates — is kept
/// in place. Returns every `\title` inner-atom run in order (pre-first-wins),
/// which the title-as-description fallback consumes: roxygen2's
/// `topics_add_default_description` takes the whole title value vector.
fn collapse_same_head_sections(out: &mut Vec<String>, block_start: usize) -> Vec<String> {
    let mut title_inners: Vec<String> = Vec::new();
    // (head, first-occurrence index into `out`, inners in value order).
    let mut collapse: Vec<(String, usize, Vec<String>)> = Vec::new();
    let mut keep = vec![true; out.len()];
    for (i, s) in out.iter().enumerate().skip(block_start) {
        let head = section_head(s);
        if head == "\\title" {
            title_inners.push(section_inner(s).to_string());
            if title_inners.len() > 1 {
                keep[i] = false;
            }
            continue;
        }
        let Some(bare) = head.strip_prefix('\\') else {
            continue;
        };
        if !COLLAPSE_HEADS.contains(&bare) {
            continue;
        }
        match collapse.iter_mut().find(|(h, _, _)| h == head) {
            Some((_, _, inners)) => {
                inners.push(section_inner(s).to_string());
                keep[i] = false;
            }
            None => collapse.push((head.to_string(), i, vec![section_inner(s).to_string()])),
        }
    }
    for (head, first, inners) in &collapse {
        if inners.len() > 1 {
            let inner = collapse_inners(&inners.iter().map(String::as_str).collect::<Vec<_>>());
            out[*first] = if inner.is_empty() {
                format!("({head})")
            } else {
                format!("({head} {inner})")
            };
        }
    }
    let mut it = keep.into_iter();
    out.retain(|_| it.next().unwrap());
    title_inners
}

/// Project a set of blocks that share one topic (`@name`/`@rdname`), merging
/// their sections the way roxygen2's `RoxyTopic$add` does. Each block is
/// projected on its own (with the title-as-description fallback deferred), then
/// same-head sections are combined: `\title` keeps the first, the
/// [`COLLAPSE_HEADS`] concatenate their bodies into one macro (`format_collapse`,
/// with adjacent `(TEXT …)` runs coalesced as `parse_Rd` + the driver would), and
/// any other head keeps each distinct section. Finally the fallback runs once on
/// the *merged* title values: with no description, the topic's title(s) supply one.
pub(super) fn project_merged_topic(
    blocks: &[RoxygenBlock],
    out: &mut Vec<String>,
    md_default: bool,
) {
    // Project each member block, deferring the fallback to the merged topic.
    let per_block: Vec<Vec<String>> = blocks
        .iter()
        .map(|b| {
            let mut v = Vec::new();
            project_block_impl(b, &mut v, false, md_default);
            v
        })
        .collect();

    // Bucket every section by head, preserving first-seen order (irrelevant to the
    // final sorted output, but keeps the merge deterministic).
    let mut by_head: Vec<(String, Vec<String>)> = Vec::new();
    for s in per_block.iter().flatten() {
        let head = section_head(s).to_string();
        match by_head.iter_mut().find(|(h, _)| *h == head) {
            Some((_, v)) => v.push(s.clone()),
            None => by_head.push((head, vec![s.clone()])),
        }
    }

    let mut has_description = false;
    for (head, secs) in &by_head {
        let bare = head.strip_prefix('\\').unwrap_or(head);
        if head == "\\title" {
            out.push(secs[0].clone());
        } else if COLLAPSE_HEADS.contains(&bare) {
            has_description |= head == "\\description";
            out.push(collapse_sections(head, secs));
        } else {
            // format_rd / aggregates: keep each distinct section (a repeated
            // identical aggregate — e.g. two `\examples` placeholders — dedups).
            for s in secs {
                if !out.contains(s) {
                    out.push(s.clone());
                }
            }
        }
    }

    // Title-as-description fallback on the merged title value vector: with no
    // description the topic reuses every block's title, collapsed.
    if !has_description && let Some((_, titles)) = by_head.iter().find(|(h, _)| h == "\\title") {
        let inner = collapse_inners(&titles.iter().map(|s| section_inner(s)).collect::<Vec<_>>());
        out.push(if inner.is_empty() {
            "(\\description)".to_string()
        } else {
            format!("(\\description {inner})")
        });
    }
}

/// The macro head of a projected section string, e.g. `\description` from
/// `(\description (TEXT "x"))`.
fn section_head(s: &str) -> &str {
    let body = s.strip_prefix('(').unwrap_or(s);
    let end = body.find([' ', ')']).unwrap_or(body.len());
    &body[..end]
}

/// The inner atoms of a projected section string (`(TEXT "x") …` from
/// `(\head (TEXT "x") …)`), or `""` for an empty `(\head)`.
fn section_inner(s: &str) -> &str {
    let core = &s[1..s.len().saturating_sub(1)];
    core.split_once(' ').map(|(_, inner)| inner).unwrap_or("")
}

/// Collapse several same-head sections into one (`format_collapse`): concatenate
/// their inner atom runs, coalescing adjacent `(TEXT …)` leaves.
fn collapse_sections(head: &str, secs: &[String]) -> String {
    let inner = collapse_inners(&secs.iter().map(|s| section_inner(s)).collect::<Vec<_>>());
    if inner.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {inner})")
    }
}

/// Join inner atom runs (a paragraph break between values renders as whitespace
/// `parse_Rd` and the driver coalesce), dropping empties and merging adjacent
/// `(TEXT …)` leaves into one space-separated leaf.
fn collapse_inners(inners: &[&str]) -> String {
    let joined = inners
        .iter()
        .filter(|i| !i.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    coalesce_text_atoms(&joined)
}

/// Coalesce adjacent `(TEXT "…")` leaves in a run of top-level Rd atoms, matching
/// the driver's TEXT-run coalescing (`is_text_leaf`) after `parse_Rd`.
fn coalesce_text_atoms(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for atom in split_top_level_atoms(s) {
        if let Some(content) = text_atom_content(atom)
            && let Some(prev) = out.last().and_then(|l| text_atom_content(l))
        {
            let merged = format!("(TEXT \"{prev} {content}\")");
            *out.last_mut().unwrap() = merged;
            continue;
        }
        out.push(atom.to_string());
    }
    out.join(" ")
}

/// The escaped string content of a `(TEXT "…")` atom, or `None` for any other atom.
fn text_atom_content(atom: &str) -> Option<&str> {
    atom.strip_prefix("(TEXT \"")?.strip_suffix("\")")
}

/// Split a run of space-separated top-level parenthesized atoms, respecting nested
/// parens and `"…"` string literals (with `\`-escapes).
pub(super) fn split_top_level_atoms(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut atoms = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        let mut depth = 0i32;
        let mut in_str = false;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' if in_str => {
                    i += 1;
                }
                b'"' => in_str = !in_str,
                b'(' if !in_str => depth += 1,
                b')' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        atoms.push(&s[start..i]);
    }
    atoms
}

/// Project the aggregated `@slot`/`@field` tags of a topic into a single
/// `\section{<title>}{\describe{\item{\code{name}}{def}…}}`. roxygen2 collects
/// every `@slot` (S4) and `@field` (reference class) into one Slots/Fields
/// section; each tag becomes a `\describe` item whose term is the verbatim
/// `\code{name}` (the name is R-code, tagged `RCODE` like a `\code` body) and
/// whose definition is the tag's prose.
pub(super) fn describe_section(title: &str, items: &[(String, Vec<Inline>)], md: bool) -> String {
    let mut item_atoms: Vec<String> = Vec::new();
    for (name, def) in items {
        let code_atoms = rcode_atoms(name);
        let term = if code_atoms.is_empty() {
            "(\\code)".to_string()
        } else {
            format!("(\\code {})", code_atoms.join(" "))
        };
        // The definition is `\item`'s second (structural) argument: a multi-atom
        // prose+macro run is `(GRP …)`-wrapped, a single atom stays bare. It is a
        // `markdown_if_active` field (the description half of `tag_two_part`), so it
        // carries the `add_linkrefs_to_md` poisoning like any other prose section.
        let mut parts = vec![term];
        let def_arg = grp_arg(&serialize_prose_with_linkrefs(def, md));
        if !def_arg.is_empty() {
            parts.push(def_arg);
        }
        item_atoms.push(format!("(\\item {})", parts.join(" ")));
    }
    format!(
        "(\\section (TEXT {}) (\\describe {}))",
        encode_text(title),
        item_atoms.join(" ")
    )
}

/// Map a tag to its Rd section macro and push the projected subtree. Tags that
/// roxygen2 does not turn into a parser-owned section (`@param` feeds the excluded
/// `\arguments`; `@export`/`@md`/`@name`/… are directives) are skipped. The
/// aggregating `@slot`/`@field` tags are handled by [`describe_section`], not here.
pub(super) fn project_tag_section(name: &str, body: &[Inline], out: &mut Vec<String>, md: bool) {
    // roxygen2's `rd_section()` drops any section whose value is the literal
    // string "NULL" (`R/field.R`), a sentinel to suppress that field (e.g.
    // `@format NULL` to override an auto-generated data `\format`). This applies
    // to every prose tag that maps to a plain-string `rd_section`; `@section`
    // (a two-part value) and the excluded `@param`/… are unaffected.
    if NULL_SUPPRESSIBLE.contains(&name) && is_null_section(body, md) {
        return;
    }
    // A brace-less sticky code/verbatim macro (`\code z`/`\verb z`/…) swallows its
    // tail into per-line `RCODE`/`VERB` atoms. It is a plain-prose-section effect,
    // so it applies to the ordinary prose tags but not `@rawRd` (raw Rd, injected
    // verbatim) or `@section` (a two-part `title: body` value); the intro paragraph
    // (out of scope) is excluded structurally — it never reaches this function.
    let swallowed = (!matches!(name, "rawRd" | "section"))
        .then(|| split_sticky_braceless_swallow(body, md))
        .flatten();
    let body = swallowed.as_deref().unwrap_or(body);
    match name {
        // `@rawRd` injects its content verbatim into the Rd file at top level;
        // roxygen2 does not wrap it in a section macro. parse_Rd then splits it
        // into a sequence of top-level Rd nodes, each a section in its own right.
        // arity's roxygen lexer already recognizes inline Rd macros in prose, so
        // serializing the body yields the same atom granularity (a prose run, a
        // `\emph`, …); each atom is pushed as a bare top-level section. The
        // content is raw Rd, never markdown: the lexer keys `@rawRd` bodies to
        // non-markdown even inside an `@md` block (`is_raw_rd_tag`), so a
        // `[bracket]`/`*star*` here stays literal Rd text rather than a spurious
        // `\link`/`\emph`.
        "rawRd" => {
            for atom in serialize_inlines(body, md) {
                out.push(atom);
            }
        }
        // Direct prose → section-macro mappings. `@description`/`@details` are
        // `tag_markdown_with_sections` (`sections = TRUE`), so a brace-incomplete
        // rendered section is dropped to empty; the rest are plain `tag_markdown`.
        "description" => emit_section_with_headings(out, "description", body, md, true),
        "details" => emit_section_with_headings(out, "details", body, md, true),
        "return" => push_section(out, "value", body, md, false),
        "seealso" => push_section(out, "seealso", body, md, false),
        "source" => push_section(out, "source", body, md, false),
        "format" => push_section(out, "format", body, md, false),
        "references" => push_section(out, "references", body, md, false),
        "note" => push_section(out, "note", body, md, false),
        "author" => push_section(out, "author", body, md, false),
        "title" => push_section(out, "title", body, md, false),
        // `@section Title: body` → \section{Title}{body}. roxygen2 splits the
        // field value on its first `:`; parse_Rd then models `\section` as a
        // two-arg structural macro, so each side sub-parses inline macros/markdown
        // and a multi-atom argument is `(GRP …)`-wrapped while a single-atom one
        // stays bare (the same rule `serialize_macro` applies to `\item`/`\tabular`).
        "section" => {
            // roxygen2 `markdown_if_active`-processes the **whole** `title: body`
            // (so the link-reference pipeline — user `[ref]: url` defs, undefined-
            // label demotion, and `add_linkrefs_to_md` poisoning — spans both
            // halves), then splits the rendered Rd on the first `:`. Run the whole
            // pipeline on the whole body, split the result, and append any leaked
            // definitions to the content — they render at the very end of the
            // field, i.e. after the `:`.
            let (leaked, leak_defs) = if md {
                // Scanned on the **original** body (a consumed def line's
                // candidate still leaks) with the field's user-def map, the
                // same as `serialize_prose`.
                let leaked = leaked_linkref_text(&leak_source_skeleton(body));
                let mut defs = LinkDefs::new();
                if !leaked.is_empty() {
                    collect_user_linkrefs_tree(body, &mut defs);
                }
                (leaked, defs)
            } else {
                (Vec::new(), LinkDefs::new())
            };
            let transformed = md
                .then(|| resolve_linkrefs(body, &LinkDefs::new()))
                .flatten();
            let body = transformed.as_deref().unwrap_or(body);
            let (heading, content) = split_section_title(body);
            let mut title = serialize_inlines(&heading, md);
            let mut content_atoms = serialize_inlines(&content, md);
            if !leaked.is_empty() {
                append_leaked_defs(&mut content_atoms, &leaked, &leak_defs);
            }
            // The whole `title: content` value trims as one string (`str_trim`,
            // Unicode White_Space — see `trim_field_atoms`): the title carries
            // the field's leading edge, the content its trailing edge; the
            // edges at the `:` split are interior and keep their whitespace.
            trim_field_atoms_start(&mut title);
            trim_field_atoms_end(&mut content_atoms);
            let mut inner = grp_arg(&title);
            if !content_atoms.is_empty() {
                if !inner.is_empty() {
                    inner.push(' ');
                }
                inner.push_str(&grp_arg(&content_atoms));
            }
            out.push(format!("(\\section{})", prefix_space(&inner)));
        }
        // `@examples`/`@examplesIf` is an aggregating field, emitted once by
        // `project_block`, so it never reaches this per-tag dispatch.
        // Everything else is roclet scaffolding or an excluded section.
        _ => {}
    }
}

/// The prose tags whose section is a plain-string `rd_section` and is therefore
/// suppressed when its value is the literal "NULL" (roxygen2's `R/field.R`
/// sentinel). `@section` is excluded: its value is a (title, body) pair, never the
/// bare string "NULL".
const NULL_SUPPRESSIBLE: &[&str] = &[
    "description",
    "details",
    "return",
    "seealso",
    "source",
    "format",
    "references",
    "note",
    "author",
    "title",
];

/// Whether a tag body is roxygen2's "NULL" suppression sentinel: it coalesces to
/// exactly one `(TEXT "NULL")` atom (a plain-string value of "NULL", any
/// surrounding whitespace already normalized away), with no macro or markdown
/// structure that would make the value something other than that string.
fn is_null_section(body: &[Inline], md: bool) -> bool {
    let atoms = serialize_inlines(body, md);
    atoms.len() == 1 && atoms[0] == "(TEXT \"NULL\")"
}

/// Push `(\<macro> <atoms…>)` for a prose section, or `(\<macro>)` when the body
/// has no content (after coalescing). Under `@md`, roxygen2's `add_linkrefs_to_md`
/// may append leaked link-reference definitions to the field text (see
/// [`leaked_linkref_text`]); they coalesce into the section's trailing prose.
///
/// `drop_on_incomplete` mirrors roxygen2's `markdown_if_active` per-section drop —
/// but the rule is **mode-dependent**. With markdown **on**, only the
/// `sections = TRUE` tags (`@description`/`@details`, including the intro paragraphs
/// `parse_description` re-emits as those tags) run `rdComplete` and replace the body
/// with `""` on a brace imbalance; the other prose tags use plain `tag_markdown`
/// (`sections = FALSE`) and do not drop, so they pass `false`. With markdown **off**,
/// `markdown_if_active`'s else-branch runs `rdComplete(text)` *unconditionally*, so
/// **every** prose section it produces (title included) drops to empty on a brace
/// imbalance regardless of `drop_on_incomplete` (`R/markdown.R`, `src/isComplete.cpp`).
/// (`@field`/`@slot` use `tag_two_part`, a separate whole-tag drop, not modeled here.)
pub(super) fn push_section(
    out: &mut Vec<String>,
    macro_name: &str,
    body: &[Inline],
    md: bool,
    drop_on_incomplete: bool,
) {
    push_section_seeded(
        out,
        macro_name,
        body,
        md,
        drop_on_incomplete,
        &LinkDefs::new(),
    );
}

/// [`push_section`] for one piece of a heading-split field, threading the
/// field-wide definition map (see [`LinkDefs`]).
fn push_section_seeded(
    out: &mut Vec<String>,
    macro_name: &str,
    body: &[Inline],
    md: bool,
    drop_on_incomplete: bool,
    seed: &LinkDefs,
) {
    let mut atoms = serialize_prose_seeded(body, md, seed);
    trim_field_atoms(&mut atoms);
    let check_drop = if md { drop_on_incomplete } else { true };
    if check_drop && !section_rd_complete_seeded(body, md, seed) {
        out.push(format!("(\\{macro_name})"));
        return;
    }
    if atoms.is_empty() {
        out.push(format!("(\\{macro_name})"));
    } else {
        out.push(format!("(\\{macro_name} {})", atoms.join(" ")));
    }
}

/// One node in a markdown-heading outline: an enclosing tag (level 0, no title) or
/// a heading (level 1-6). `body` is the prose before the frame's first child
/// heading; `children` index into the flat frame arena in source order.
pub(super) struct HeadingFrame {
    pub(super) level: usize,
    pub(super) title: Vec<Inline>,
    pub(super) body: Vec<Inline>,
    pub(super) children: Vec<usize>,
}

/// Emit a `@description`/`@details` section (macro `macro_name`) as roxygen2's
/// markdown-heading outline. Prose before the first heading — plus any level->=2
/// heading with no enclosing level-1 heading — stays inside the enclosing
/// `\<macro_name>`; each level-1 heading hoists to a **top-level** `\section`
/// sibling; a deeper heading nests as `\subsection` under the nearest shallower
/// heading. Falls back to a plain [`push_section`] when the body has no heading.
///
/// roxygen2 markdown-processes the whole field as one document (so link references
/// span it) and only *then* splits it into sections; arity splits first and
/// resolves each piece independently, so a link reference cannot yet cross a
/// heading (backlog). The common case — self-contained heading sections — is exact.
/// The per-piece `rdComplete` drop is applied per level-1 piece
/// ([`heading_piece_complete`]): an incomplete piece empties, a `\section` title
/// surviving (cm-090).
pub(super) fn emit_section_with_headings(
    out: &mut Vec<String>,
    macro_name: &str,
    body: &[Inline],
    md: bool,
    drop_on_incomplete: bool,
) {
    if emit_section_with_list_hoist(out, macro_name, body, md, drop_on_incomplete) {
        return;
    }
    struct Seg {
        head: Option<(usize, Vec<Inline>)>,
        run: Vec<Inline>,
    }
    let mut field_defs = LinkDefs::new();
    let mut segments: Vec<Seg> = vec![Seg {
        head: None,
        run: Vec::new(),
    }];
    for inl in body {
        let Inline::MdHeading(node) = inl else {
            segments.last_mut().unwrap().run.push(inl.clone());
            continue;
        };
        // Close the running piece's defs first: document order decides which
        // definition of a duplicated label wins (cmark keeps the first).
        collect_user_linkrefs_tree(&segments.last().unwrap().run, &mut field_defs);
        if md && let Some(strip) = setext_title_strip(node) {
            for (label, def) in strip.defs {
                field_defs.entry(label).or_insert(def);
            }
            match strip.title {
                Some(title) => segments.push(Seg {
                    head: Some((strip.level, title)),
                    run: Vec::new(),
                }),
                // An all-defs title: the heading never forms, and the underline
                // is literal paragraph text (cm-218).
                None => segments
                    .last_mut()
                    .unwrap()
                    .run
                    .push(Inline::Text(strip.underline)),
            }
            continue;
        }
        let (level, title_text) = parse_md_heading(node);
        segments.push(Seg {
            head: Some((level, resolve_macro_arg_inlines(&title_text))),
            run: Vec::new(),
        });
    }
    collect_user_linkrefs_tree(&segments.last().unwrap().run, &mut field_defs);
    if segments.len() == 1 {
        // No heading (every candidate demoted, or none existed) — the ordinary
        // prose section path, over the run (a demoted underline is part of it).
        push_section_seeded(
            out,
            macro_name,
            &segments[0].run,
            md,
            drop_on_incomplete,
            &field_defs,
        );
        return;
    }

    // Build the outline. Frame 0 is the enclosing tag (level 0); each heading's
    // parent is the nearest open frame of a strictly lower level.
    let mut frames: Vec<HeadingFrame> = vec![HeadingFrame {
        level: 0,
        title: Vec::new(),
        body: std::mem::take(&mut segments[0].run),
        children: Vec::new(),
    }];
    let mut stack = vec![0usize];
    for seg in segments.into_iter().skip(1) {
        let (level, title) = seg
            .head
            .expect("a non-leading segment always carries a heading");
        while frames[*stack.last().unwrap()].level >= level {
            stack.pop();
        }
        let parent = *stack.last().unwrap();
        let idx = frames.len();
        frames.push(HeadingFrame {
            level,
            title,
            body: seg.run,
            children: Vec::new(),
        });
        frames[parent].children.push(idx);
        stack.push(idx);
    }

    let has_section = frames[0].children.iter().any(|&c| frames[c].level == 1);
    if drop_on_incomplete && !heading_piece_complete(&frames, 0, md, &field_defs) {
        if !has_section {
            out.push(format!("(\\{macro_name})"));
        }
    } else {
        let mut inner = serialize_prose_seeded(&frames[0].body, md, &field_defs);
        for &c in &frames[0].children {
            if frames[c].level >= 2 {
                inner.push(render_heading_frame(
                    &frames,
                    c,
                    md,
                    "subsection",
                    &field_defs,
                ));
            }
        }
        trim_field_atoms(&mut inner);
        if !inner.is_empty() {
            out.push(format!("(\\{macro_name} {})", inner.join(" ")));
        }
    }

    // Each level-1 child is a top-level `\section` sibling in the output. An
    // incomplete `\section` piece keeps its title (it lives in the split marker)
    // over an emptied body.
    for &c in &frames[0].children {
        if frames[c].level != 1 {
            continue;
        }
        if drop_on_incomplete && !heading_piece_complete(&frames, c, md, &field_defs) {
            let title = grp_arg(&frame_title_atoms(&frames[c], md, &field_defs, true));
            out.push(format!("(\\section{})", prefix_space(&title)));
        } else {
            out.push(render_heading_frame(&frames, c, md, "section", &field_defs));
        }
    }
}

/// A setext heading's title after cmark's leading link-reference-definition
/// strip: `defs` are the stripped definitions, `title` the remaining title
/// inline run — `None` when the definitions consumed the whole title paragraph,
/// so the heading never forms and `underline` is ordinary paragraph text
/// (cm-218).
struct SetextStrip {
    level: usize,
    defs: LinkDefs,
    title: Option<Vec<Inline>>,
    underline: String,
}

/// Strip the leading link-reference definitions from a **setext** heading's
/// title, mirroring cmark's `resolve_reference_link_definitions`: when a setext
/// underline closes a paragraph, the paragraph's leading definitions strip
/// *before* the promotion is decided, so a definition line never joins the
/// title (cm-217) and an all-defs paragraph is never promoted (cm-218 — the
/// `===` stays literal text). `None` for an ATX heading, or when no definition
/// leads the title (the caller keeps its ordinary path, byte-identical).
///
/// The title lines re-resolve as one inline run with [`SOFT_BREAK`] line
/// boundaries (the definition grammar is line-keyed; the space-join
/// [`parse_md_heading`] uses would put the next title line on the definition's
/// line as junk), and the standard block-start scan ([`collect_user_linkrefs`])
/// consumes the leading run — with no `\n` in the joined title, only the
/// *leading* lines can be definitions, exactly cmark's strip-from-the-start.
///
/// A `-` underline over an all-defs paragraph falls through to cmark's
/// *thematic break*, not literal text — unmodeled here, so that shape keeps its
/// heading form (backlog).
fn setext_title_strip(node: &SyntaxNode) -> Option<SetextStrip> {
    let text = node.text().to_string();
    let lines: Vec<&str> = text.split('\n').map(strip_marker).collect();
    if lines.len() < 2 {
        return None;
    }
    let underline = lines.last().unwrap().trim();
    let level = setext_underline_level(underline)?;
    let title_src = lines[..lines.len() - 1]
        .iter()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(&SOFT_BREAK.to_string());
    let inlines = resolve_macro_arg_inlines(&title_src);
    let (defs, dropped) = collect_user_linkrefs(&inlines);
    if dropped.is_empty() {
        return None;
    }
    let mut rest: Vec<Inline> = Vec::new();
    for (i, inl) in inlines.iter().enumerate() {
        match dropped.get(&i) {
            // Wholly part of a consumed definition.
            Some(None) => {}
            // A definition consumed this text's leading lines; the remainder
            // opens the title.
            Some(Some(leftover)) => rest.push(Inline::Text(leftover.clone())),
            None => rest.push(inl.clone()),
        }
    }
    let empty = rest
        .iter()
        .all(|inl| matches!(inl, Inline::Text(t) if t.trim().is_empty()));
    if empty && level == 2 {
        return None;
    }
    Some(SetextStrip {
        level,
        defs,
        title: (!empty).then_some(rest),
        underline: underline.to_string(),
    })
}

/// Render a heading frame as `(\<macro_name> <title> <body>)` — a two-arg
/// structural macro (like `@section`): the title, then the body. Each argument is
/// bare when a single atom, `(GRP …)`-wrapped when several, absent when empty. The
/// frame's own prose comes first; every child heading nests one level deeper as a
/// `\subsection`, appended in source order.
///
/// `seed` is the field-wide definition map: it resolves a reference in the
/// heading *title* too (cm-216 — roxygen2 markdown-processes the whole field
/// before splitting sections). Defs are never consumed in a title: it is inline
/// content, so a def-shaped `[ref]: url` there is ordinary prose.
pub(super) fn render_heading_frame(
    frames: &[HeadingFrame],
    idx: usize,
    md: bool,
    macro_name: &str,
    seed: &LinkDefs,
) -> String {
    let f = &frames[idx];
    let mut title_atoms = frame_title_atoms(f, md, seed, true);
    let mut body_atoms = serialize_prose_seeded(&f.body, md, seed);
    for &c in &f.children {
        body_atoms.push(render_heading_frame(frames, c, md, "subsection", seed));
    }
    // A level-1 frame's body is a whole split piece (the heading emits only the
    // split marker, no braces), so its edges trim; a `\subsection` body is
    // interior to its piece and keeps its edges.
    if f.level == 1 {
        trim_field_atoms(&mut body_atoms);
    }
    // parse_Rd pairs backslashes left-to-right over the rendered
    // `\section{title}{body}` string, so a title whose final brace group's
    // content ends in an **odd** backslash run has that group's closing brace
    // escaped (`\\emph{baz\}` — the demoted macro's argument renders verbatim,
    // cm-066): the group re-closes on the *title's* own closing brace (its
    // content gains the escaped `}` as a literal), the body's `{…}` then opens
    // as a bare group *inside* the still-open title, and the body's closer ends
    // the title — so the body folds into the title argument and the `\section`
    // renders with a single argument (parse_Rd recovers at end of input).
    if md
        && let Some(last) = title_atoms.last_mut()
        && let Some(repaired) = extend_escaped_list_closer(last)
    {
        *last = repaired;
        title_atoms.push(format!("(LIST{})", prefix_space(&body_atoms.join(" "))));
        body_atoms.clear();
    }
    let mut inner = grp_arg(&title_atoms);
    let body_arg = grp_arg(&body_atoms);
    if !body_arg.is_empty() {
        if !inner.is_empty() {
            inner.push(' ');
        }
        inner.push_str(&body_arg);
    }
    format!("(\\{macro_name}{})", prefix_space(&inner))
}

/// If `atom` is a `(LIST …)` whose final `TEXT` leaf's decoded content ends in
/// an **odd** backslash run — so the group's rendered closing brace is escaped
/// by parse_Rd's left-to-right pairing — return the repaired atom: the odd
/// trailing backslash is consumed by the pairing and the escaped `}` joins the
/// content as a literal (`baz\` → `baz}`). `None` when the shape does not apply
/// (not a LIST, no final TEXT leaf, or an even run).
fn extend_escaped_list_closer(atom: &str) -> Option<String> {
    let inner = atom.strip_prefix("(LIST ")?.strip_suffix(')')?;
    let text_start = inner.rfind("(TEXT \"")?;
    // The final child must be that TEXT leaf (nothing but its closer follows).
    let leaf = inner[text_start..].strip_suffix(')')?;
    let bytes = leaf.as_bytes();
    let mut i = leaf.find('"')?;
    let text = read_quoted(bytes, &mut i);
    if i != leaf.len() {
        return None;
    }
    let run = text.chars().rev().take_while(|&c| c == '\\').count();
    if run % 2 == 0 {
        return None;
    }
    let mut repaired = text[..text.len() - 1].to_string();
    repaired.push('}');
    Some(format!(
        "(LIST {}(TEXT {}))",
        &inner[..text_start],
        encode_text(&repaired)
    ))
}

/// A heading frame's title as serialized atoms, resolving a field-wide link
/// reference exactly as [`render_heading_frame`] does (defs never consumed in a
/// title — it is inline content). `group` selects the output shape (a bare
/// `{…}` becomes an Rd `LIST` group); the per-piece `rdComplete` scan passes
/// `false` — grouping loses the backslash parity the scan weighs (see
/// [`serialize_prose`]).
fn frame_title_atoms(f: &HeadingFrame, md: bool, seed: &LinkDefs, group: bool) -> Vec<String> {
    let title = (md && !seed.is_empty())
        .then(|| apply_user_linkrefs(&f.title, seed, false))
        .flatten();
    let title = title.as_deref().unwrap_or(&f.title);
    // A bare `{…}` in the heading title is an Rd `LIST` group, exactly as in prose
    // and macro args (`# H {a b}` → `(GRP (TEXT "H") (LIST (TEXT "a b")))`).
    let grouped = group.then(|| group_brace_lists(title, md));
    serialize_inlines(grouped.as_deref().unwrap_or(title), md)
}

/// Whether one level-1 **piece** of a heading-split field renders brace-complete
/// Rd. roxygen2 markdown-renders the whole field, splits it at the level-1
/// `\section` markers, and runs `rdComplete` per piece (`mdxml_children_to_rd_top`,
/// R/markdown.R): a failing piece is replaced by `""` — the enclosing tag's body
/// (piece 0, its `\subsection`s included) empties, a `\section`'s body empties
/// while its title (part of the split marker) survives. The scan text mirrors
/// that split: the frame's prose atoms plus each nested `\subsection{title}{body}`
/// wrapper, reconstructed like [`section_rd_complete_seeded`]'s md arm (ungrouped
/// atoms, `%`-comment regions stripped, a dropping `\href` destination checked
/// directly on the parsed URL). A level-1 title with its own imbalance is not
/// modeled (it lives inside the split marker, a different failure — backlog).
fn heading_piece_complete(frames: &[HeadingFrame], idx: usize, md: bool, seed: &LinkDefs) -> bool {
    let mut rd = String::new();
    heading_piece_rd(frames, idx, md, seed, &mut rd) && rd_complete(&rd)
}

/// Append the rendered-Rd scan text for the piece rooted at `frames[idx]` —
/// skipping level-1 children (each is its own piece) — returning `false` when a
/// body holds an inline whose rendering alone drops the piece
/// ([`body_has_md_drop`]).
fn heading_piece_rd(
    frames: &[HeadingFrame],
    idx: usize,
    md: bool,
    seed: &LinkDefs,
    rd: &mut String,
) -> bool {
    let f = &frames[idx];
    if body_has_md_drop(&f.body) {
        return false;
    }
    let stripped = strip_scan_percent_comments(&f.body);
    let body = stripped.as_deref().unwrap_or(&f.body);
    for atom in serialize_prose(body, md, false, seed) {
        sexpr_to_rd(&atom, md, rd);
    }
    for &c in &f.children {
        if frames[c].level == 1 {
            continue;
        }
        rd.push_str("\\subsection{");
        for atom in frame_title_atoms(&frames[c], md, seed, false) {
            sexpr_to_rd(&atom, md, rd);
        }
        rd.push_str("}{");
        if !heading_piece_rd(frames, c, md, seed, rd) {
            return false;
        }
        rd.push('}');
    }
    true
}

/// One level-1 heading found by [`emit_section_with_list_hoist`]'s scan: the cut
/// point where roxygen2 splices its section marker into the flat Rd string.
struct HoistCut {
    node: SyntaxNode,
    /// The containing (list, item-index) path — empty for a top-level heading.
    /// List identity is an index into the scan's normalized-list arena.
    chain: Vec<(usize, usize)>,
    /// Index of the containing top-level inline in the body.
    top_pos: usize,
    /// For an in-list cut: the heading's inline index within its innermost item.
    item_pos: usize,
}

/// Collect the level-1 headings inside a list inline (recursing into nested
/// lists) as [`HoistCut`]s, normalizing each visited list's items into the
/// `lists` arena so the stranded-piece renderer can slice them later.
fn hoist_scan_list(
    inl: &Inline,
    chain: &[(usize, usize)],
    top_pos: usize,
    lists: &mut Vec<Vec<Vec<Inline>>>,
    cuts: &mut Vec<HoistCut>,
) {
    let items: Vec<Vec<Inline>> = match inl {
        Inline::MdList(node) => node
            .children()
            .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
            .map(|it| md_list_item_inlines(&it))
            .collect(),
        Inline::MdListResolved { items, .. } => items.clone(),
        _ => return,
    };
    let id = lists.len();
    lists.push(items);
    for it_idx in 0..lists[id].len() {
        for pos in 0..lists[id][it_idx].len() {
            let inl2 = lists[id][it_idx][pos].clone();
            let mut sub = chain.to_vec();
            sub.push((id, it_idx));
            match &inl2 {
                Inline::MdHeading(node) if parse_md_heading(node).0 == 1 => {
                    cuts.push(HoistCut {
                        node: node.clone(),
                        chain: sub,
                        top_pos,
                        item_pos: pos,
                    });
                }
                _ => hoist_scan_list(&inl2, &sub, top_pos, lists, cuts),
            }
        }
    }
}

/// Emit a `@description`/`@details` body containing a level-1 heading **inside a
/// list** (cm-302; returns `false`, leaving the body to the ordinary outline,
/// when no in-list level-1 heading exists). roxygen2 renders the whole field to
/// one flat Rd string and splices a section marker at *every* level-1 heading —
/// even mid-`\itemize{…}` — then splits at the markers and `rdComplete`-checks
/// each piece: a piece sliced mid-list has an unmatched `{`/`}`, so it is
/// emptied with a "mismatched braces" warning (the section *title* survives).
/// Engine-probed: the tag piece before an in-list heading always drops (the
/// unclosed `\itemize{`), as does any piece crossing a list boundary; the piece
/// *between* two headings in the same list stays balanced and renders its
/// stranded items brace-less (`\item` outside `\itemize` — parse_Rd's unknown
/// macro, probe p4).
fn emit_section_with_list_hoist(
    out: &mut Vec<String>,
    macro_name: &str,
    body: &[Inline],
    md: bool,
    drop_on_incomplete: bool,
) -> bool {
    let mut lists: Vec<Vec<Vec<Inline>>> = Vec::new();
    let mut cuts: Vec<HoistCut> = Vec::new();
    for (idx, inl) in body.iter().enumerate() {
        match inl {
            Inline::MdHeading(node) if parse_md_heading(node).0 == 1 => {
                cuts.push(HoistCut {
                    node: node.clone(),
                    chain: Vec::new(),
                    top_pos: idx,
                    item_pos: 0,
                });
            }
            _ => hoist_scan_list(inl, &[], idx, &mut lists, &mut cuts),
        }
    }
    if !cuts.iter().any(|c| !c.chain.is_empty()) {
        return false;
    }

    // The tag's own piece: everything before the first cut. An in-list first
    // cut leaves it mid-`\itemize{` — dropped whole. A top-level first cut
    // leaves a balanced prefix, rendered by the ordinary path (the recursion
    // finds no in-list cut in it).
    if cuts[0].chain.is_empty() && cuts[0].top_pos > 0 {
        emit_section_with_headings(
            out,
            macro_name,
            &body[..cuts[0].top_pos],
            md,
            drop_on_incomplete,
        );
    }

    // Each cut is one top-level `\section`; its body piece runs to the next cut
    // (or the field's end). A piece whose list-container path differs from the
    // next cut's is sliced across a list boundary — unmatched braces, emptied.
    let lists_of = |c: &HoistCut| c.chain.iter().map(|&(l, _)| l).collect::<Vec<_>>();
    for k in 0..cuts.len() {
        let (_, title_text) = parse_md_heading(&cuts[k].node);
        let title = resolve_macro_arg_inlines(&title_text);
        let cur = lists_of(&cuts[k]);
        let next = cuts.get(k + 1).map(&lists_of).unwrap_or_default();
        if cur != next {
            // Sliced across a list boundary: the body is emptied, the title kept.
            let title_atoms = serialize_inlines(&group_brace_lists(&title, md), md);
            out.push(format!(
                "(\\section{})",
                prefix_space(&grp_arg(&title_atoms))
            ));
            continue;
        }
        if cur.is_empty() {
            // A top-level pair (or trailing top-level cut): the ordinary
            // outline over the piece, rooted at this heading.
            let end = cuts.get(k + 1).map(|c| c.top_pos).unwrap_or(body.len());
            out.push(hoisted_section_atom(
                title,
                &body[cuts[k].top_pos + 1..end],
                md,
            ));
            continue;
        }
        // An in-list pair: the stranded items between the two headings within
        // the shared innermost list, each `\item` brace-less.
        let &(list_id, it_k) = cuts[k].chain.last().unwrap();
        let &(_, it_next) = cuts[k + 1].chain.last().unwrap();
        let items = &lists[list_id];
        let mut atoms: Vec<String> = Vec::new();
        if it_k == it_next {
            let piece = &items[it_k][cuts[k].item_pos + 1..cuts[k + 1].item_pos];
            atoms.extend(serialize_inlines(piece, true));
        } else {
            atoms.extend(serialize_inlines(
                &items[it_k][cuts[k].item_pos + 1..],
                true,
            ));
            for (j, item) in items.iter().enumerate().take(it_next + 1).skip(it_k + 1) {
                atoms.push(format!("(UNKNOWN {})", encode_text("\\item")));
                let end = if j == it_next {
                    cuts[k + 1].item_pos
                } else {
                    item.len()
                };
                atoms.extend(serialize_inlines(&item[..end], true));
            }
        }
        let title_atoms = serialize_inlines(&group_brace_lists(&title, md), md);
        let mut inner = grp_arg(&title_atoms);
        let body_arg = grp_arg(&atoms);
        if !body_arg.is_empty() {
            if !inner.is_empty() {
                inner.push(' ');
            }
            inner.push_str(&body_arg);
        }
        out.push(format!("(\\section{})", prefix_space(&inner)));
    }
    true
}

/// Render a hoisted top-level `\section` whose piece content is ordinary
/// (balanced) body: the same segment/frame outline as
/// [`emit_section_with_headings`], rooted at this section's level-1 heading
/// (nested level >= 2 headings become `\subsection`s).
fn hoisted_section_atom(title: Vec<Inline>, content: &[Inline], md: bool) -> String {
    let mut segments: Vec<(Option<SyntaxNode>, Vec<Inline>)> = vec![(None, Vec::new())];
    for inl in content {
        if let Inline::MdHeading(node) = inl {
            segments.push((Some(node.clone()), Vec::new()));
        } else {
            segments.last_mut().unwrap().1.push(inl.clone());
        }
    }
    let mut frames: Vec<HeadingFrame> = vec![HeadingFrame {
        level: 1,
        title,
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
    render_heading_frame(&frames, 0, md, "section", &LinkDefs::new())
}

/// A markdown heading node's level (1-6) and title text (markdown source, resolved
/// later by the caller). Handles both node shapes that reach `ROXYGEN_MD_HEADING`:
///
/// - **ATX** (`# Title`): a single line; the level is the leading `#` run, the title
///   is the rest with the optional closing `#` sequence stripped.
/// - **Setext** (`Title` / `===`): two or more `#'` lines whose last is a `===`/`---`
///   underline; the level comes from the underline (`=` -> 1, `-` -> 2) and the
///   title from the joined prose lines above it (soft breaks become spaces, matching
///   a coalesced paragraph).
pub(super) fn parse_md_heading(node: &SyntaxNode) -> (usize, String) {
    let text = node.text().to_string();
    let lines: Vec<&str> = text.split('\n').map(strip_marker).collect();
    if lines.len() >= 2
        && let Some(level) = setext_underline_level(lines.last().unwrap())
    {
        let title = lines[..lines.len() - 1]
            .iter()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" ");
        return (level, title);
    }
    // An ATX heading opening as a tag's same-line value has a marker-less line
    // (the `#'` belongs to the enclosing tag): drop only the single separator
    // space — `strip_marker` would eat the heading's own `#` run.
    let from_value = node
        .first_token()
        .is_some_and(|t| t.kind() != SyntaxKind::ROXYGEN_MARKER);
    let line = if from_value {
        text.strip_prefix([' ', '\t']).unwrap_or(&text).trim_start()
    } else {
        strip_marker(&text).trim_start()
    };
    let level = line.bytes().take_while(|&b| b == b'#').count().clamp(1, 6);
    let rest = line.get(level..).unwrap_or("").trim();
    (level, strip_atx_closing(rest).to_string())
}

/// The setext heading level of a marker-stripped line, or `None` when it is not a
/// setext underline: a non-empty run of `=` (level 1) or `-` (level 2) with only
/// surrounding whitespace. Mirrors the lexer's `is_setext_underline` (which carved
/// the underline leaf), so a line that reached here as the last child of a heading
/// node always matches.
fn setext_underline_level(line: &str) -> Option<usize> {
    let s = line.trim();
    let ch = s.bytes().next()?;
    if (ch == b'=' || ch == b'-') && s.bytes().all(|b| b == ch) {
        return Some(if ch == b'=' { 1 } else { 2 });
    }
    None
}

/// Strip a CommonMark ATX **closing sequence** — a trailing run of `#` preceded by
/// a space/tab (or forming the whole title) — from a heading title, trimming the
/// remaining trailing whitespace. `foo ###` -> `foo`; `foo#` (no preceding space)
/// stays `foo#`; `###` (empty heading with only a closing run) -> ``.
fn strip_atx_closing(s: &str) -> &str {
    let t = s.trim_end();
    let hashes = t.len() - t.trim_end_matches('#').len();
    if hashes == 0 {
        return t;
    }
    let before = &t[..t.len() - hashes];
    if before.is_empty() || before.ends_with([' ', '\t']) {
        before.trim_end()
    } else {
        t
    }
}

/// Whether roxygen2 would keep (not drop) a prose section, i.e. its Rd is
/// brace-complete under [`rd_complete`] — but from the text roxygen2 *actually*
/// scans, which differs by mode.
///
/// **Markdown on:** roxygen2 runs `rdComplete(markdown(text))` — the
/// markdown-*rendered* Rd, whose structural braces (`\emph{…}`, the
/// trailing-`\` `\emph{\}` case) come from cmark. That render is what the
/// canonical *ungrouped* atoms reconstruct ([`section_atoms_rd_complete`]); the
/// scan re-serializes with grouping off ([`serialize_prose`]) because a bare
/// `{…}` `LIST` group loses the brace-abutting backslash parity — the flat atoms
/// *are* `markdown(text)`.
///
/// **Markdown off:** roxygen2 runs `rdComplete(x$raw)` on the *raw* tag value,
/// where an escaped brace `\{`/`\}` is still backslash-escaped and therefore
/// **not counted**. The atoms are the wrong text here: escape resolution has
/// already collapsed `\{` → a bare `{` (see [`resolve_rd_text_escapes`]), so
/// reconstructing from them would count the brace and false-drop the section
/// (`a \{ b`, which roxygen2 renders "a { b", is complete). Scan the
/// pre-resolution raw text ([`section_raw_rd`]) instead — raw ≈ rendered for the
/// only chars `rd_complete` weighs (`{}`, `\`, `%`), and md-off never synthesizes
/// the cmark-derived braces that make the atoms necessary.
#[cfg(test)]
pub(super) fn section_rd_complete(body: &[Inline], md: bool) -> bool {
    section_rd_complete_seeded(body, md, &LinkDefs::new())
}

/// [`section_rd_complete`] for one piece of a heading-split field: the drop scan
/// re-serializes the body, so it needs the same field-wide definition `seed` the
/// output serialization uses.
fn section_rd_complete_seeded(body: &[Inline], md: bool, seed: &LinkDefs) -> bool {
    if md {
        if body_has_md_drop(body) {
            return false;
        }
        // Scan the *ungrouped* atoms: they reconstruct `markdown(text)` faithfully,
        // whereas grouping a balanced brace pair into a `LIST` loses the backslash
        // parity rd_complete weighs (see [`serialize_prose`]). First drop each
        // odd-run `\%` comment region ([`strip_scan_percent_comments`]): the output
        // swallow keeps `ceil(k/2)` backslashes (parse_Rd's rendered text), which
        // can leave a dangling trailing escape that false-drops a kept section.
        let stripped = strip_scan_percent_comments(body);
        let scan_body = stripped.as_deref().unwrap_or(body);
        section_atoms_rd_complete(&serialize_prose(scan_body, md, false, seed), md)
    } else {
        rd_complete(&section_raw_rd(body))
    }
}

/// Whether `body` holds an inline whose rendering alone makes roxygen2 drop the
/// section — a construct the atom scan cannot see because its atom neutralizes the
/// offending characters. Three shapes: an inline `[text](dest)` link whose destination
/// renders into a brace-incomplete `\href{dest}{text}` ([`md_href_dest_drops`]),
/// a fenced code block whose raw info string breaks the rendered fragment's balance
/// ([`md_fence_info_drops`]), and a `` `Rd …` `` span whose raw body carries a `%`
/// that comments out the generated `\Sexpr`'s closing brace ([`md_sexpr_span_drops`]). Recurses into every container that can hold either
/// (emphasis, bare brace groups, resolved list items, and a link's own display).
/// Reference and shortcut links (`\link`, whose topic option is dropped) never carry
/// the destination into a brace argument, so only the inline-link (`\href`) form is
/// checked — but a kept link's *rendered display* is checked too
/// ([`link_display_render_drops`]).
fn body_has_md_drop(body: &[Inline]) -> bool {
    body.iter().any(|inl| match inl {
        Inline::MdCode(content) => md_sexpr_span_drops(content),
        Inline::MdInlineLink { url, display } => {
            md_href_dest_drops(url) || body_has_md_drop(display)
        }
        Inline::MdRefLink { display, .. } | Inline::MdShortcutLink { display } => {
            body_has_md_drop(display) || link_display_render_drops(display)
        }
        Inline::MdEmphasis { children, .. } => body_has_md_drop(children),
        Inline::BraceGroup(children) => body_has_md_drop(children),
        Inline::MdListResolved { items, .. } => items.iter().any(|it| body_has_md_drop(it)),
        Inline::MdCodeBlock(node) => md_fence_info_drops(node),
        _ => false,
    })
}

/// Whether a kept reference/shortcut link's **rendered display** makes the field
/// brace-incomplete, so roxygen2's `rdComplete` fails and the whole section drops.
/// roxygen2 8.0.0 renders a non-plain display inside the generated
/// `\link[topic]{…}` (`mdxml_link_text`, which renders text **verbatim** — no
/// backslash escaping), so cmark-derived content like an emphasis whose text ends
/// in a backslash renders `\emph{b\}`: the trailing `\` escapes the macro's own
/// closing brace and the whole field string comes up brace-short (`[a\*b\*]` →
/// display `a\` + emph `b\`). The atom scan cannot see this — the generated
/// `\link` head is fragile to [`sexpr_to_rd`], which treats its argument as raw
/// and balanced (right for a *user-written* `\link{…}`, wrong for a generated
/// one) — so the display subtree is scanned directly here, in isolation (prose
/// around a link never contributes a bare unescaped `}` that could re-balance
/// it). A plain-text or single-code-span display renders through the flat
/// shortcut/code paths and is not subject to `mdxml_link_text`'s verbatim
/// rendering.
fn link_display_render_drops(display: &[Inline]) -> bool {
    let plain = display.iter().all(|inl| matches!(inl, Inline::Text(_)));
    if plain || matches!(display, [Inline::MdCode(_)]) {
        return false;
    }
    !section_atoms_rd_complete(&serialize_inlines(display, true), true)
}

/// Whether an inline-link destination `url` (the parsed destination — the lexer's
/// `inline_dest_span` already closes it where cmark does) renders into an `\href{…}`
/// argument that ends in a backslash escaping the closing brace — so roxygen2's
/// `rdComplete` fails and the section drops.
///
/// roxygen2 runs `double_escape_md` (every `\` doubled) before cmark, so a backslash
/// is a **literal** char in the destination and never escapes a paren or an angle
/// bracket: a bare destination closes at the first raw paren-depth-0 `)` (`[t](foo\)bar)`
/// → `foo\`), an angle-bracketed one at the first `>` (`[t](<foo\>)` → `foo\`). A
/// backslash run of length `r` at the end of the destination survives the
/// `double_escape_md` → cmark → `parse_Rd` round-trip as `r` backslashes in the Rd
/// destination (double to `2r`, cmark pairs to `r`), so the `\href{…}` brace is
/// escaped exactly when `r` is **odd**: `foo\` (r = 1) drops, `foo\\` (r = 2) keeps.
/// The depth-0 re-scan below is a no-op on an already-narrow bare destination but
/// keeps an angle destination's interior `)` (`<b)c>` → no drop) out of the count.
pub(super) fn md_href_dest_drops(url: &str) -> bool {
    let bytes = url.as_bytes();
    // cmark's closer: the first paren-depth-0 `)` (a `\` is literal, never escaping).
    let mut depth = 0usize;
    let mut close = bytes.len();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' if depth == 0 => {
                close = i;
                break;
            }
            b')' => depth -= 1,
            _ => {}
        }
    }
    // The backslash run immediately before the closer; an odd run escapes the brace.
    let mut run = 0usize;
    let mut k = close;
    while k > 0 && bytes[k - 1] == b'\\' {
        run += 1;
        k -= 1;
    }
    run % 2 == 1
}

/// Whether a section's projected atoms reconstruct to brace-complete Rd, i.e.
/// roxygen2 would *not* drop the section. Rebuilds the pre-parse Rd string from
/// the canonical S-expression atoms ([`sexpr_to_rd`]) and runs [`rd_complete`]
/// (the `is_code = false` form `markdown_if_active` uses).
pub(super) fn section_atoms_rd_complete(atoms: &[String], md: bool) -> bool {
    let mut rd = String::new();
    for atom in atoms {
        sexpr_to_rd(atom, md, &mut rd);
    }
    rd_complete(&rd)
}

/// Reconstruct the raw Rd text roxygen2 runs `rdComplete` on for a markdown-OFF
/// prose section: the tag value verbatim, *before* any escape resolution. A
/// prose leaf contributes its raw source (so `\{`/`\}` keep the backslash that
/// makes `rd_complete` treat the brace as escaped, not structural); an explicit
/// Rd macro contributes its verbatim source (`\emph{x}`, balanced by
/// construction). Markdown-off bodies hold only these two inline kinds — the
/// cmark-synthesized inlines are md-only — so any other variant would be a
/// contradiction and contributes nothing. The soft-wrap sentinel
/// ([`SOFT_BREAK`]) is restored to a real newline so an unescaped `%` comment
/// stays scoped to its physical line exactly as it is in roxygen2's `x$raw`.
fn section_raw_rd(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        match inl {
            Inline::Text(t) => {
                for c in t.chars() {
                    s.push(if c == SOFT_BREAK { '\n' } else { c });
                }
            }
            Inline::Macro(n) => s.push_str(&n.text().to_string()),
            _ => {}
        }
    }
    s
}

/// Port of roxygen2's `rdComplete(string, is_code = FALSE)` (`src/isComplete.cpp`):
/// a brace-balance scan where `\` escapes the next char and `%` starts a comment to
/// end of line. The string is complete iff braces net to zero and the scan does not
/// end mid-escape. (The `is_code = TRUE` string/raw-string handling is unused by
/// `markdown_if_active`, so it is not modeled.)
pub(super) fn rd_complete(s: &str) -> bool {
    #[derive(PartialEq)]
    enum State {
        Rd,
        RdEscape,
        RdComment,
    }
    let mut state = State::Rd;
    let mut braces: i64 = 0;
    for c in s.chars() {
        match state {
            State::Rd => match c {
                '{' => braces += 1,
                '}' => braces -= 1,
                '\\' => state = State::RdEscape,
                '%' => state = State::RdComment,
                _ => {}
            },
            State::RdEscape => state = State::Rd,
            State::RdComment => {
                if c == '\n' {
                    state = State::Rd;
                }
            }
        }
    }
    braces == 0 && state != State::RdEscape
}

/// Split an `@section` body at roxygen2's title separator (the first literal `:`,
/// which lives in a prose `Inline::Text` run) into `(title, content)` inline runs.
/// The `:` is dropped; everything before it is the heading, everything after the
/// body. Macros/markdown carry no `:` separator, so only `Inline::Text` is scanned.
fn split_section_title(body: &[Inline]) -> (Vec<Inline>, Vec<Inline>) {
    let mut title: Vec<Inline> = Vec::new();
    let mut content: Vec<Inline> = Vec::new();
    let mut split = false;
    for inl in body {
        if split {
            content.push(inl.clone());
            continue;
        }
        if let Inline::Text(t) = inl
            && let Some(idx) = t.find(':')
        {
            if idx > 0 {
                title.push(Inline::Text(t[..idx].to_string()));
            }
            let after = &t[idx + 1..];
            if !after.is_empty() {
                content.push(Inline::Text(after.to_string()));
            }
            split = true;
            continue;
        }
        title.push(inl.clone());
    }
    (title, content)
}

/// Render an in-item heading frame as `(\subsection <title> <body>)` — the item
/// analog of [`render_heading_frame`], with the body serialized as item content
/// (`serialize_inlines`; the whole-field link-reference pipeline never runs
/// inside an item).
pub(super) fn item_subsection_atom(frames: &[HeadingFrame], idx: usize) -> String {
    let f = &frames[idx];
    let title_atoms = serialize_inlines(&group_brace_lists(&f.title, true), true);
    let mut body_atoms = serialize_inlines(&f.body, true);
    for &c in &f.children {
        body_atoms.push(item_subsection_atom(frames, c));
    }
    let mut inner = grp_arg(&title_atoms);
    let body_arg = grp_arg(&body_atoms);
    if !body_arg.is_empty() {
        if !inner.is_empty() {
            inner.push(' ');
        }
        inner.push_str(&body_arg);
    }
    format!("(\\subsection{})", prefix_space(&inner))
}
