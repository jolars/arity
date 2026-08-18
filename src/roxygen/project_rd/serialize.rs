use super::*;

/// Serialize a prose body into canonical atoms, applying roxygen2's
/// `add_linkrefs_to_md` poisoning model under `@md`. Every field text that
/// roxygen2 runs through `markdown_if_active` is subject to the same leak — a
/// prose section ([`push_section`]), each `@field`/`@slot` definition
/// ([`describe_section`]), and the `@section` body — so they share this path.
///
/// Two steps, both `@md`-only: a leaked link-reference block de-links the
/// shortcut/reference links in its poisoned tail, so [`demote_poisoned_links`]
/// rewrites them to literal bracket text *first* (keeping body and leaked
/// definitions consistent), then [`leaked_linkref_text`] appends the leaked
/// definitions to the trailing prose.
pub(super) fn serialize_prose_with_linkrefs(body: &[Inline], md: bool) -> Vec<String> {
    serialize_prose(body, md, true, &LinkDefs::new())
}

/// [`serialize_prose_with_linkrefs`] for one piece of a heading-split field:
/// `seed` is the field-wide definition map, so a link reference resolves across
/// the field's headings (cm-216/217).
pub(super) fn serialize_prose_seeded(body: &[Inline], md: bool, seed: &LinkDefs) -> Vec<String> {
    serialize_prose(body, md, true, seed)
}

/// The shared prose serializer behind [`serialize_prose_with_linkrefs`]. `group`
/// controls whether a balanced bare `{…}` run is partitioned into an Rd `LIST`
/// group ([`group_brace_lists`]).
///
/// **Output uses `group = true`** — a bare group renders `(LIST …)` in both
/// markdown modes. **The md `rdComplete` drop scan uses `group = false`**: roxygen2
/// decides the drop on `markdown(text)`, whose braces are the flat cmark-rendered
/// ones, and grouping loses the backslash parity that scan weighs. A structural
/// brace comes from an *even* (non-escaping) run, which [`collapse_md_backslash_runs`]
/// keeps verbatim while it abuts the brace; grouping consumes the brace, so the run
/// collapses early (`\\` → `\`) and a trailing `\` before the `LIST`'s brace would
/// read as a spurious escape in the reconstruction. Scanning the ungrouped flat
/// atoms sidesteps that — they *are* `markdown(text)`.
pub(super) fn serialize_prose(
    body: &[Inline],
    md: bool,
    group: bool,
    seed: &LinkDefs,
) -> Vec<String> {
    // The md leaked-linkref scan reconstructs the raw markdown source, so it
    // reads the **original** body — roxygen2's `get_md_linkrefs` runs before
    // cmark consumes user definition lines, so a consumed def line's own
    // candidate still leaks (cm-196's first leaked segment). The user-def map
    // is collected here too: cmark parses the leaked block *in the document*,
    // so a leaked candidate whose label the user defined re-links against it.
    let (leaked, leak_defs) = if md {
        let leaked = leaked_linkref_text(&leak_source_skeleton(body));
        let mut defs = LinkDefs::new();
        if !leaked.is_empty() {
            defs = seed.clone();
            collect_user_linkrefs_tree(body, &mut defs);
        }
        (leaked, defs)
    } else {
        (Vec::new(), LinkDefs::new())
    };
    let transformed = md.then(|| resolve_linkrefs(body, seed)).flatten();
    let body = transformed.as_deref().unwrap_or(body);
    // A bare `{…}` in prose is an Rd `LIST` group in both modes. The brace/comment
    // parity is shared (an odd backslash run escapes the brace, an even run opens
    // it); only the `%`-comment trigger differs by mode (`group_brace_lists` handles
    // that internally).
    let grouped = group.then(|| group_brace_lists(body, md));
    let scan = grouped.as_deref().unwrap_or(body);
    // A brace-less `\item` in prose is parse_Rd's out-of-list recovery: an
    // `(UNKNOWN "\item")` node splitting the surrounding text. Carve it out of the
    // (already brace-grouped) run before serializing. Output path only: the
    // `group = false` md `rdComplete` scan reads the raw `\item` text, which counts
    // no braces either way, so it needs no split.
    let split = group.then(|| split_braceless_items(scan)).flatten();
    let scan = split.as_deref().unwrap_or(scan);
    // Output path only: the `group = false` md `rdComplete` scan reads the body as
    // roxygen2's markdown left it, before parse_Rd expanded any system Rd macro
    // (see [`serialize_inlines_unexpanded`]).
    let mut atoms = if group {
        serialize_inlines(scan, md)
    } else {
        serialize_inlines_unexpanded(scan, md)
    };
    if !leaked.is_empty() {
        append_leaked_defs(&mut atoms, &leaked, &leak_defs);
    }
    atoms
}

/// Append a leaked link-reference block (see [`leaked_linkref_text`]) to the
/// serialized atoms. cmark parses the leaked lines as markdown **in the
/// document**, so inline structure resolves within them: emphasis pairs
/// (cm-196's `[Foo*bar\]: R:Foo*bar%5C` pairs its `*`s into `\emph`), a leaked
/// candidate whose label the user defined re-links (`urls`), and any other
/// shortcut/reference bracket demotes to literal text (its own synthesized
/// definition is in the leaked block itself, which roxygen2 never re-scans).
/// The lines join on soft breaks and the leading text glues onto a trailing
/// `(TEXT …)` atom — roxygen2 renders no separator before the leak.
pub(super) fn append_leaked_defs(atoms: &mut Vec<String>, leaked: &[String], urls: &LinkDefs) {
    // The leaked lines are cmark-stage (double-escaped) bytes; the fragment
    // resolver and its text pipeline model source-stage bytes, so convert
    // first ([`escaped_md_to_source`]) — a `[bad\\\]` leak renders `[bad\]`,
    // cmark's pairing, not the source-stage lose-one rule.
    let fragment = escaped_md_to_source(&leaked.join("\n"));
    // A leaked label spanning a blank line makes the leak a multi-**block**
    // markdown fragment — cmark forms paragraphs and indented code across the
    // blank lines (cm-184's CDATA-body label) — which the inline resolver cannot
    // model. Re-parse it as a synthesized `#' @md` block instead; a blank-free
    // leak stays on the inline path (a single paragraph either way).
    let blocky = fragment.split('\n').any(|line| line.trim().is_empty());
    let leak_atoms = blocky
        .then(|| leak_block_atoms(&fragment, urls))
        .flatten()
        .unwrap_or_else(|| leak_inline_atoms(&fragment, urls));
    let mut leak_atoms = leak_atoms.into_iter();
    if let Some(first) = leak_atoms.next() {
        match decode_text_atom(&first) {
            Some(text) => append_rendered_text(atoms, &text),
            None => atoms.push(first),
        }
    }
    atoms.extend(leak_atoms);
}

/// Resolve a blank-line-free leaked fragment as one markdown paragraph: emphasis
/// pairs within it, a leaked candidate whose label the user defined re-links
/// (`urls`), and any other shortcut/reference bracket demotes to literal text
/// (its own synthesized definition is in the leaked block itself, which roxygen2
/// never re-scans).
fn leak_inline_atoms(fragment: &str, urls: &LinkDefs) -> Vec<String> {
    let inlines = resolve_macro_arg_inlines(fragment);
    serialize_inlines(&leak_resolve(inlines, urls), true)
}

/// Resolve a blank-line-bearing leaked fragment at the **block** level: the
/// lines re-parse as a synthesized `#' @md` fragment through the real parser
/// (the same mould as [`quote_synthesized_block`]), so the leak's own block
/// structure forms — its first lines lazily gather into one paragraph, a
/// post-blank 4-column line is indented code, and so on. The re-parsed body
/// then runs the leak's link treatment ([`leak_resolve`]) and — because the
/// rendered lines are field text to parse_Rd — the bare-brace `LIST` grouping
/// ([`group_brace_lists`]), so a brace pair spanning the leak's paragraphs
/// nests exactly as parse_Rd nests it (cm-184). `None` withholds to the inline
/// path when the synthesized re-parse mis-sections (an `@`-opening leak line).
fn leak_block_atoms(fragment: &str, urls: &LinkDefs) -> Option<Vec<String>> {
    let lines: Vec<String> = fragment.split('\n').map(str::to_string).collect();
    let block = quote_synthesized_block(&lines)?;
    let mut body: Vec<Inline> = Vec::new();
    for section in block.sections() {
        for part in section_body_parts(&section) {
            // The same part joining as `project_block`: a fresh roxygen paragraph
            // joins on a line break (collapsed by `norm_ws`), a leading block
            // quote glues separator-free.
            if !body.is_empty() && !matches!(part.first(), Some(Inline::MdBlockQuote(_))) {
                body.push(Inline::Text("\n".to_string()));
            }
            body.extend(part);
        }
    }
    let resolved = leak_resolve(body, urls);
    Some(serialize_inlines(&group_brace_lists(&resolved, true), true))
}

/// The leak's link treatment, shared by both resolution paths: re-link a leaked
/// candidate whose label the user defined, demote every other shortcut or
/// reference bracket to literal text (no definition can resolve inside the
/// leaked block — roxygen2 never re-scans it).
fn leak_resolve(inlines: Vec<Inline>, urls: &LinkDefs) -> Vec<Inline> {
    let linked = (!urls.is_empty())
        .then(|| apply_user_linkrefs(&inlines, urls, false))
        .flatten();
    let linked = linked.unwrap_or(inlines);
    let demoted = demote_undefined_links(&linked, &std::collections::HashSet::new());
    demoted.unwrap_or(linked)
}

/// Convert cmark-stage (double-escaped) text back to its source-stage
/// equivalent, inverting `double_escape_md`: a backslash run of `k` before a
/// square bracket came from `(k + 1) / 2` source backslashes (doubling to `2k`
/// then the `\\[`→`\[`/`\\]`→`\]` de-dup leaves `2k - 1`, always odd), and any
/// other run came from `k / 2` (plain doubling, always even). The leaked
/// definition lines are cmark-stage bytes ([`leaked_linkref_text`]); the
/// fragment resolver models source-stage bytes, so they convert before
/// resolution ([`append_leaked_defs`]).
pub(super) fn escaped_md_to_source(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&s[start..i]);
            continue;
        }
        let mut k = 0usize;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
            k += 1;
        }
        let source_k = if matches!(bytes.get(i), Some(b'[' | b']')) {
            k.div_ceil(2)
        } else {
            k / 2
        };
        for _ in 0..source_k {
            out.push('\\');
        }
    }
    out
}

/// Apply roxygen2's full markdown link-reference pipeline to a prose body,
/// returning the rewritten inline run (or `None` when nothing changed). The
/// caller has already checked that markdown is active. Four composing stages,
/// each turning links into other links or literal text:
///
/// 0. **Refmap-dependent re-pairing** ([`repair_ref_link_chains`]): the arena
///    pairs an adjacent bracket chain `[foo][bar][baz]` eagerly without the
///    refmap; cmark consumes a following `[label]` only when the label is
///    defined, rewinding it otherwise so it re-pairs with what follows. Runs
///    first, on the original body, so the later stages see cmark's pairing.
/// 1. **User definitions** (`[ref]: url`): a referencing shortcut/reference link
///    whose label is defined becomes a `\href{url}{display}` (display kept), and
///    the definition lines are consumed. Runs before the demotions so the refmap
///    below still sees every bracket the way roxygen2's raw-source scan does.
/// 2. **Undefined-label demotion** (the `(?<!\])`/`(?=[^\[{])` link-reference-map
///    gap): a shortcut/reference link whose label roxygen2 never defines demotes
///    to literal bracket text.
/// 3. **Positional poisoning** (`add_linkrefs_to_md`): a valid candidate whose
///    synthesized definition leaks demotes its tail.
///
/// Both demotions only turn links into literal text, so order is immaterial to
/// correctness; the refmap (stage 2) runs after stage 1 so it sees every bracket
/// the user defs left behind.
///
/// `seed` is the field-wide definition map ([`LinkDefs`]) for a piece of a
/// heading-split field; it was collected in document order, so it takes
/// precedence over this piece's own collection (first definition wins, per
/// cmark). Empty for a body that is its own whole field.
pub(super) fn resolve_linkrefs(body: &[Inline], seed: &LinkDefs) -> Option<Vec<Inline>> {
    let repaired = repair_ref_link_chains(body, &linkref_keys(body));
    let b0 = repaired.as_deref().unwrap_or(body);
    let mut urls: LinkDefs = seed.clone();
    collect_user_linkrefs_tree(b0, &mut urls);
    let resolved = (!urls.is_empty())
        .then(|| apply_user_linkrefs(b0, &urls, true))
        .flatten();
    let b1 = resolved.as_deref().unwrap_or(b0);
    let undefined = demote_undefined_links(b1, &linkref_keys(b1));
    let b2 = undefined.as_deref().unwrap_or(b1);
    let demoted = demote_poisoned_links(b2);
    // Materialize an owned body only when some stage actually rewrote it.
    if repaired.is_some() || resolved.is_some() || undefined.is_some() || demoted.is_some() {
        Some(demoted.unwrap_or_else(|| b2.to_vec()))
    } else {
        None
    }
}

/// A pending piece of the `(TEXT …)` atom [`serialize_inlines`] is coalescing.
/// Ordinary prose is `Raw` (source text awaiting the markdown/comment pipeline);
/// a block quote's already-flattened text is `Final` (pre-processed) so it *glues*
/// into the surrounding atom instead of splitting off as its own — roxygen2 emits
/// no paragraph separator around an unsupported block quote, so its text runs
/// straight onto adjacent prose (`before` + `> q` → `beforeq`).
pub(super) enum RunSeg {
    Raw(String),
    Final(String),
}

/// Append raw source text to the pending run, coalescing into a trailing `Raw`
/// segment so a contiguous prose run stays one segment (processed as a whole).
fn push_raw(run: &mut Vec<RunSeg>, s: &str) {
    match run.last_mut() {
        Some(RunSeg::Raw(last)) => last.push_str(s),
        _ => run.push(RunSeg::Raw(s.to_string())),
    }
}

/// Drop trailing whitespace (spaces, source line breaks, `SOFT_BREAK`s) from the
/// pending run, popping now-empty trailing `Raw` segments. Used before a block
/// quote glues on, so the preceding paragraph's trailing break does not survive as
/// a separating space (`norm_ws` would collapse it to one). A `Final` segment (an
/// already-flattened block quote) is left untouched — its own whitespace is fixed.
fn trim_trailing_run_ws(run: &mut Vec<RunSeg>) {
    while let Some(RunSeg::Raw(last)) = run.last_mut() {
        let trimmed = last.trim_end_matches(is_posix_space);
        if trimmed.len() == last.len() {
            break;
        }
        last.truncate(trimmed.len());
        if last.is_empty() {
            run.pop();
        } else {
            break;
        }
    }
}

/// Finalize the pending run into one coalesced `(TEXT …)` atom (or `None` when it
/// normalizes to empty), clearing it. Each `Raw` segment runs through the prose
/// pipeline ([`process_prose`]: markdown escaping or Rd `%`-comment stripping)
/// *without* normalizing whitespace; a `Final` (pre-flattened block quote) segment
/// passes through verbatim; the concatenation is whitespace-normalized once, so a
/// boundary line break collapses to a single space while a glued block quote stays
/// seamless.
pub(super) fn flush_run(run: &mut Vec<RunSeg>, md: bool) -> Option<String> {
    if run.is_empty() {
        return None;
    }
    let mut combined = String::new();
    for seg in run.iter() {
        match seg {
            RunSeg::Raw(s) => combined.push_str(&process_prose(s, md)),
            RunSeg::Final(s) => combined.push_str(s),
        }
    }
    run.clear();
    text_atom(&combined)
}

/// Whether the pending prose run ends with an **odd** backslash run at cmark's
/// stage — the parity that decides parse_Rd's pairing across the text/macro
/// boundary in the rendered field. A source run of `k` backslashes reaches the
/// field as `k` (`double_escape_md` doubles, cmark halves), and a following
/// md-generated macro contributes its own `\`, so parse_Rd sees `k + 1`: odd `k`
/// pairs the macro's backslash away — the name is absorbed into the TEXT and each
/// braced argument re-parses as a bare `LIST` group (cm-014's `\*x*` →
/// `(TEXT "\emph") (LIST …)`) — while even `k` leaves the macro intact. The
/// collapsed text's `ceil(k/2)` backslashes already equal parse_Rd's paired count
/// in both parities, so only this structural demotion is parity-dependent. The
/// run is read from the trailing `Raw` segment (contiguous raw prose coalesces
/// into one); a `Final` segment is already parse-level, its parity unrecoverable.
fn run_ends_odd_backslash_run(run: &[RunSeg]) -> bool {
    let Some(RunSeg::Raw(last)) = run.last() else {
        return false;
    };
    last.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1
}

/// Emit a demoted macro (see [`run_ends_odd_backslash_run`]): the bare name glues
/// onto the flushed TEXT atom (as a `Final` segment, so the prose pipeline never
/// reads it as a brace-less macro misuse), and each argument follows as a
/// `(LIST …)` atom — parse_Rd's re-parse of the now-bare brace groups.
fn push_demoted_macro(
    atoms: &mut Vec<String>,
    run: &mut Vec<RunSeg>,
    md: bool,
    name: &str,
    args: Vec<String>,
) {
    run.push(RunSeg::Final(name.to_string()));
    if let Some(atom) = flush_run(run, md) {
        atoms.push(atom);
    }
    for arg in args {
        atoms.push(if arg.is_empty() {
            "(LIST)".to_string()
        } else {
            format!("(LIST {arg})")
        });
    }
}

/// Partition a non-`@md` prose run's bare `{…}` brace groups into [`Inline::BraceGroup`]
/// nodes. parse_Rd treats an unescaped brace pair in prose text as a `LIST`
/// delimiter (a macro's own braces live inside its CST node, so only *bare* text
/// braces reach here): `a {b c} d` → `(TEXT "a") (LIST (TEXT "b c")) (TEXT "d")`.
/// Groups nest, span macros (an `Inline::Macro` between the braces lands inside the
/// group), and cross soft breaks; the inner text pieces keep their raw form so the
/// downstream [`process_prose`] still resolves escapes and strips `%` comments.
///
/// Brace parity mirrors [`resolve_rd_text_escapes`] and is mode-independent: an
/// odd-length backslash run escapes the following brace (`\{`/`\}` stay literal, no
/// group), an even run leaves it bare (`\\{` opens a group). Under `@md` a source
/// `\\{y}` is exactly what parse_Rd receives from `markdown(text)` — cmark pairs the
/// doubled run back to `\` and leaves the brace bare — so the same parity applies.
///
/// The **`%` comment trigger is inverted between modes**, mirroring
/// [`md_percent_swallow`]. Non-md: a bare `%` opens an Rd comment that hides braces
/// to the physical line end (an escaped `\%` was already consumed by the backslash
/// arm, so only a bare `%` reaches the `%` arm). Md: roxygen2 escapes a rendered `%`
/// to `\%`, so a bare/even-preceded `%` stays literal and does **not** hide braces;
/// only a `%` preceded by an **odd** backslash run renders bare and opens a comment
/// (the escaping backslash collides). Either way a comment's text is copied verbatim
/// (the prose pipeline drops it later) without treating its braces as delimiters.
///
/// Only a **balanced** run is grouped; an unbalanced one is returned unchanged (its
/// section drops via `rdComplete` before the atoms are used — see
/// [`section_rd_complete`]), so the pass never models parse_Rd's error recovery. A
/// brace-free run is likewise returned as-is, keeping its byte-identical
/// serialization.
pub(super) fn group_brace_lists(body: &[Inline], md: bool) -> Vec<Inline> {
    // `stack[0]` is the output level; each deeper frame is an open `{` group.
    let mut stack: Vec<Vec<Inline>> = vec![Vec::new()];
    let mut buf = String::new();
    let mut grouped = false;
    let flush = |stack: &mut Vec<Vec<Inline>>, buf: &mut String| {
        if !buf.is_empty() {
            stack
                .last_mut()
                .unwrap()
                .push(Inline::Text(std::mem::take(buf)));
        }
    };
    for inl in body {
        let Inline::Text(s) = inl else {
            // A non-text inline (macro, resolved md node) is opaque to brace
            // scanning; flush the pending text and drop it into the current group.
            flush(&mut stack, &mut buf);
            stack.last_mut().unwrap().push(inl.clone());
            continue;
        };
        let bytes = s.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => {
                    let start = i;
                    while i < bytes.len() && bytes[i] == b'\\' {
                        i += 1;
                    }
                    let run_len = i - start;
                    buf.push_str(&s[start..i]);
                    // An odd run escapes the next char (brace/letter): copy it
                    // verbatim so it is never read as a group delimiter. Under `@md`
                    // a `%` is *not* escape-consumed here — its comment-ness is
                    // decided by the `%` arm (odd preceding run → bare `%` → comment).
                    if run_len % 2 == 1 && i < bytes.len() && !(md && bytes[i] == b'%') {
                        let mut end = i + 1;
                        while !s.is_char_boundary(end) {
                            end += 1;
                        }
                        buf.push_str(&s[i..end]);
                        i = end;
                    }
                }
                b'%' => {
                    // Whether this `%` opens an Rd comment (hiding following braces to
                    // the physical line end). Non-md: a bare `%` always does (an
                    // escaped `\%` was consumed by the backslash arm). Md: only a `%`
                    // rendered bare — one preceded by an *odd* backslash run — does
                    // (roxygen2 escapes an even/zero-run `%` to `\%`); mirrors
                    // [`md_percent_swallow`].
                    let opens_comment = if md {
                        let mut k = 0usize;
                        while k < i && bytes[i - 1 - k] == b'\\' {
                            k += 1;
                        }
                        k % 2 == 1
                    } else {
                        true
                    };
                    if opens_comment {
                        // Copy verbatim to the physical line end (a real newline or a
                        // SOFT_BREAK); its braces are inert.
                        let start = i;
                        while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\x0c' {
                            i += 1;
                        }
                        buf.push_str(&s[start..i]);
                    } else {
                        buf.push('%');
                        i += 1;
                    }
                }
                b'{' => {
                    flush(&mut stack, &mut buf);
                    stack.push(Vec::new());
                    grouped = true;
                    i += 1;
                }
                b'}' if stack.len() > 1 => {
                    flush(&mut stack, &mut buf);
                    let g = stack.pop().unwrap();
                    stack.last_mut().unwrap().push(Inline::BraceGroup(g));
                    grouped = true;
                    i += 1;
                }
                _ => {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && !matches!(bytes[i], b'\\' | b'%' | b'{' | b'}') {
                        i += 1;
                    }
                    buf.push_str(&s[start..i]);
                }
            }
        }
        flush(&mut stack, &mut buf);
    }
    // An unclosed group means the braces are unbalanced: leave the run flat (the
    // section drops anyway), never a partial tree.
    if !grouped || stack.len() != 1 {
        return body.to_vec();
    }
    stack.pop().unwrap()
}

/// Serialize an inline run into the canonical atom sequence: maximal prose runs
/// coalesce into one whitespace-normalized `(TEXT …)`, and each macro becomes a
/// nested subtree — mirroring the R driver's `serialize_children`. `md` is the
/// block's resolved markdown mode: with markdown off a prose run is literal Rd, so
/// `process_prose` strips its `%` line comments.
pub(super) fn serialize_inlines(body: &[Inline], md: bool) -> Vec<String> {
    // parse_Rd expands R's system Rd macros (`\doi`, `\CRANpkg`, …) into a
    // `USERMACRO` leaf plus their spliced expansion. Rewriting the run first lets
    // an expansion that is plain text (`\I`) coalesce with the prose around it.
    let expanded = expand_user_macros(body);
    serialize_inlines_unexpanded(expanded.as_deref().unwrap_or(body), md)
}

/// [`serialize_inlines`] without the system-Rd-macro expansion — the form the md
/// `rdComplete` drop scan needs. roxygen2 decides that drop on `markdown(text)`,
/// which is *pre*-`parse_Rd`: the macros are still written as `\doi{…}` there, so
/// counting the braces of an expansion parse_Rd has not performed yet would both
/// mis-weigh the scan and feed [`parse_rd_recovery`] a disturbance that is not in
/// roxygen2's input.
pub(super) fn serialize_inlines_unexpanded(body: &[Inline], md: bool) -> Vec<String> {
    let mut atoms: Vec<String> = Vec::new();
    let mut run: Vec<RunSeg> = Vec::new();
    for inl in body {
        match inl {
            Inline::Text(s) => push_raw(&mut run, s),
            Inline::UserMacro(leaf) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(format!("(USERMACRO {})", encode_text(leaf)));
            }
            Inline::Macro(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_macro(node, md));
            }
            Inline::MdCode(content) => {
                if md && run_ends_odd_backslash_run(&run) {
                    // Demoted, the span's verbatim-ness is lost: the bare brace
                    // group re-parses as ordinary Rd text — fragile macros
                    // become real nodes, bare braces nest as `LIST` groups
                    // ([`demoted_md_code_parts`]).
                    let (name, arg) = demoted_md_code_parts(content);
                    push_demoted_macro(&mut atoms, &mut run, md, &name, vec![arg]);
                    continue;
                }
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(md_code_atom(content));
            }
            Inline::MdEmphasis { strong, children } => {
                // Recurse into the inner inline run (nesting projects as structure),
                // then wrap. The block's `@md` mode holds inside an emphasis span.
                let inner = serialize_inlines(children, md).join(" ");
                if md && run_ends_odd_backslash_run(&run) {
                    let name = if *strong { "strong" } else { "emph" };
                    push_demoted_macro(&mut atoms, &mut run, md, name, vec![inner]);
                    continue;
                }
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                let head = if *strong { "\\strong" } else { "\\emph" };
                atoms.push(if inner.is_empty() {
                    format!("({head})")
                } else {
                    format!("({head} {inner})")
                });
            }
            Inline::MdList(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_list(node));
            }
            Inline::MdListResolved { ordered, items } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_list_resolved(*ordered, items));
            }
            Inline::MdLink(raw) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(resolve_md_link(raw).unwrap_or_default());
            }
            Inline::MdInlineLink { url, display } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(inline_link_node_atom(url, display, md));
            }
            // A reference/shortcut link whose display is not plain text renders
            // the display's resolved markup inside the `\link` body (roxygen2
            // 8.0.0's `mdxml_link_text`; 7.x dropped such links outright).
            Inline::MdRefLink { dest, display } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                // A *collapsed* reference (`[text][]`, empty `dest`) resolves its
                // label from the display, so cmark hands roxygen2 the synthesized
                // `R:label` destination — exactly the shortcut shape.
                atoms.push(if dest.is_empty() {
                    shortcut_link_node_atom(display)
                } else {
                    ref_link_node_atom(display, dest)
                });
            }
            Inline::MdShortcutLink { display } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(shortcut_link_node_atom(display));
            }
            Inline::MdImage(raw) => match resolve_md_image(raw) {
                Some(atom) => {
                    if md && run_ends_odd_backslash_run(&run) {
                        // A bare `\figure` demotes whole (verbatim args re-parse
                        // as plain text); a format-keyed `\if` demotes only
                        // itself, its inner `\figure` still parsing inside the
                        // bare group.
                        if let Some((name, args)) = resolve_md_image_demoted(raw) {
                            push_demoted_macro(&mut atoms, &mut run, md, name, args);
                            continue;
                        }
                    }
                    if let Some(flushed) = flush_run(&mut run, md) {
                        atoms.push(flushed);
                    }
                    atoms.push(atom);
                }
                // An image that resolves to nothing — an undefined *collapsed*
                // reference `![alt][]` (see [`resolve_md_image`]) — is literal
                // cmark text: it stays in the prose run, gluing with the
                // surrounding text.
                None => push_raw(&mut run, raw),
            },
            Inline::MdCodeBlock(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.extend(serialize_md_code_block(node));
            }
            Inline::MdIndentedCode(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.extend(serialize_md_indented_code(node));
            }
            Inline::MdHtml(raw) => {
                if md && run_ends_odd_backslash_run(&run) {
                    // Both of `\if{html}{\out{…}}`'s args demote to LISTs; the
                    // `\out` inside the second still parses (parse_Rd knows it
                    // anywhere), keeping its verbatim body.
                    let args = vec![
                        format!("(TEXT {})", encode_text("html")),
                        format!("(\\out {})", html_out_verbs(raw)),
                    ];
                    push_demoted_macro(&mut atoms, &mut run, md, "if", args);
                    continue;
                }
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(html_inline_atom(raw));
            }
            Inline::MdHtmlBlock(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_html_block(node));
            }
            Inline::BracelessItem => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(format!("(UNKNOWN {})", encode_text("\\item")));
            }
            // A brace-less sticky code/verbatim macro's swallowed tail: one verbatim
            // `(RCODE …)`/`(VERB …)` atom per physical source line, each carrying its
            // own trailing `\n` (`\code z here` line-wrapped → `(RCODE " z here\n")
            // (RCODE "continued\n")`).
            Inline::StickyVerbatim { code, lines } => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                let head = if *code { "RCODE" } else { "VERB" };
                for line in lines {
                    atoms.push(format!("({head} {})", encode_text(&format!("{line}\n"))));
                }
            }
            Inline::BraceGroup(children) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                let inner = serialize_inlines(children, md).join(" ");
                atoms.push(if inner.is_empty() {
                    "(LIST)".to_string()
                } else {
                    format!("(LIST {inner})")
                });
            }
            Inline::MdBlockQuote(node) => {
                // roxygen2 has no block-quote support: it renders the flattened
                // plain text with *no* surrounding paragraph separator, so the text
                // glues straight onto adjacent prose (`before` + `> q` → `beforeq`).
                // Push it as a pre-flattened `Final` segment so it coalesces into
                // the current `(TEXT …)` atom instead of splitting off as its own.
                // The preceding node keeps a trailing line break (its own newline,
                // or the part-join break) which `norm_ws` would otherwise turn into a
                // separating space, so drop that trailing whitespace first — cmark
                // strips a paragraph's trailing whitespace before the quote appends.
                let flat = block_quote_flat_text(node);
                if !flat.is_empty() {
                    trim_trailing_run_ws(&mut run);
                    run.push(RunSeg::Final(flat));
                }
            }
            Inline::MdTable(node) => {
                if let Some(atom) = flush_run(&mut run, md) {
                    atoms.push(atom);
                }
                atoms.push(serialize_md_table(node));
            }
            // A heading is normally consumed by the outline builder before it
            // reaches here (`emit_section_with_headings`). Reaching this arm means a
            // heading in a context roxygen2 does not turn into a section (e.g.
            // `@seealso`, where roxygen2 errors on a level-1 heading) — out of scope
            // for the projector. Fall back to rendering the title text inline so the
            // walk never panics; such a case is never pinned in the corpus.
            Inline::MdHeading(node) => {
                let (_, title) = parse_md_heading(node);
                for atom in serialize_inlines(&resolve_macro_arg_inlines(&title), md) {
                    if let Some(prose) = flush_run(&mut run, md) {
                        atoms.push(prose);
                    }
                    atoms.push(atom);
                }
            }
        }
    }
    if let Some(atom) = flush_run(&mut run, md) {
        atoms.push(atom);
    }
    atoms
}

/// Project one `ROXYGEN_RD_MACRO` node into `(\name <children…>)`: the `[opt]` and
/// `{`/`}` delimiters are dropped, prose text coalesces into `(TEXT …)`, verbatim
/// content becomes `(VERB …)` (no whitespace collapse), and nested macros recurse.
///
/// A *structural* macro (`\item`, `\tabular` --- [`is_multi_arg_rd_macro`]) models
/// each `{…}` argument as a list, so a multi-atom argument projects to a
/// `(GRP …)` wrapper (`\tabular{rl}{a \tab b}` → `(\tabular (TEXT "rl") (GRP …))`)
/// while a single-atom argument unwraps (`\item{a}{first}` → `(\item (TEXT "a")
/// (TEXT "first"))`). A latexlike macro (`\code`, `\emph`, …) inlines its single
/// argument's atoms directly, never wrapping.
///
/// Under `@md`, a **non-fragile** inline text macro (`\emph`, `\strong`, `\sQuote`,
/// …) has its argument **markdown-processed** ([`is_md_inline_text_macro`]):
/// roxygen2 protects only its `escaped_for_md` set from cmark, so a non-fragile
/// macro's `{…}` body is parsed as a markdown inline run (`\emph{*x*}` →
/// `\emph{\emph{x}}`). A fragile nested macro (`\code`/`\link`/…) stays literal —
/// this resolves recursively, so each macro re-checks its own fragility. A
/// non-fragile **structural** macro (`\item`, `\tabular`, `\href` —
/// [`is_md_structural_macro`]) likewise markdown-processes each of its arguments:
/// the `md_structural` flag below routes prose runs through the inline pass while
/// the loop's existing arms keep nested macros (`\tab`/`\cr`), verbatim args (the
/// `\href` URL), and the per-argument `(GRP …)` wrap intact.
pub(super) fn serialize_macro(node: &SyntaxNode, md: bool) -> String {
    let head_full = macro_head(node);
    let name = head_full.trim_start_matches('\\');
    if name == "preformatted" || (is_verbatim_rd_macro(name) && threads_markers(node)) {
        return serialize_verbatim_block(node, &head_full);
    }
    if md
        && is_md_inline_text_macro(name)
        && let Some(content) = macro_single_arg_content(node)
    {
        // A bare `{…}` in the argument is an Rd `LIST` group, exactly as in prose
        // (`\emph{a {b} c}` → `(\emph (TEXT "a") (LIST (TEXT "b")) (TEXT "c"))`).
        let grouped = group_brace_lists(&resolve_macro_arg_inlines(&content), md);
        let atoms = serialize_inlines(&grouped, md);
        return if atoms.is_empty() {
            format!("({head_full})")
        } else {
            format!("({head_full} {})", atoms.join(" "))
        };
    }
    // A structural two-arg macro (`\item`, `\tabular`, `\href`) under `@md` has
    // each non-verbatim argument markdown-processed as **one** cmark run (so an
    // emphasis/link span crosses a nested macro). That needs a whole-argument
    // resolution from the pre-carved children, handled by a dedicated walk.
    if md && is_md_structural_macro(name) {
        return serialize_md_structural_macro(node, &head_full);
    }
    let mut head = String::new();
    let mut structural = false;
    let mut out_atoms: Vec<String> = Vec::new();
    // Per-argument pieces: raw prose text (escapes unresolved) and already-serialized
    // atoms (nested macros, verbatim `(VERB …)`). Collected between the argument's
    // `{`…`}` so a bare `{…}` brace group can be folded across a nested macro (see
    // [`finalize_macro_arg`] / [`group_arg_pieces`]).
    let mut pieces: Vec<ArgPiece> = Vec::new();
    let mut text_buf = String::new();
    // Push the pending prose run as one text piece, coalescing contiguous text tokens
    // (parse_Rd models one `(TEXT …)` per uninterrupted run).
    let flush_text = |text_buf: &mut String, pieces: &mut Vec<ArgPiece>| {
        if !text_buf.is_empty() {
            pieces.push(ArgPiece::Text(std::mem::take(text_buf)));
        }
    };
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_NAME => {
                head = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                structural = is_multi_arg_rd_macro(head.trim_start_matches('\\'));
            }
            SyntaxKind::ROXYGEN_RD_MACRO_VERB => {
                flush_text(&mut text_buf, &mut pieces);
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                // parse_Rd resolves Rd-string escapes in verbatim arguments —
                // except `\eqn`/`\deqn`, whose LaTeX-like text keeps them raw
                // (see `serialize_verbatim_block`).
                let resolved = if matches!(head.trim_start_matches('\\'), "eqn" | "deqn") {
                    raw
                } else {
                    resolve_rd_arg_escapes(&raw)
                };
                pieces.push(ArgPiece::Atom(format!("(VERB {})", encode_text(&resolved))));
            }
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush_text(&mut text_buf, &mut pieces);
                if let Some(n) = el.as_node() {
                    // A system Rd macro nested in this argument expands in place
                    // (`\code{\CRANpkg{utils}}`), contributing its `USERMACRO`
                    // leaf and expansion as sibling atoms.
                    match user_macro_atoms(n, md) {
                        Some(expanded) => pieces.extend(expanded.into_iter().map(ArgPiece::Atom)),
                        None => pieces.push(ArgPiece::Atom(serialize_macro(n, md))),
                    }
                }
            }
            // A markdown block construct inside the body (`@md`): cmark parses the
            // field text flat, so a list or fenced block at the body's column is a
            // real block that renders into this argument. It projects exactly as it
            // would at section level, and is opaque to the argument's brace scan.
            SyntaxKind::ROXYGEN_MD_LIST | SyntaxKind::ROXYGEN_MD_CODE_BLOCK => {
                flush_text(&mut text_buf, &mut pieces);
                if let Some(n) = el.as_node() {
                    pieces.extend(md_block_atoms(n).into_iter().map(ArgPiece::Atom));
                }
            }
            // A closing `}` ends an argument group: flush the run, then atomize the
            // argument's pieces (folding bare `{…}` groups into `(LIST …)`) and
            // finalize (GRP-wrapping a structural macro's multi-atom argument). The
            // opening `{` carries no content.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                if el.as_token().is_some_and(|t| t.text() == "}") {
                    flush_text(&mut text_buf, &mut pieces);
                    finalize_macro_arg(
                        &mut pieces,
                        is_rcode_body_macro(&head),
                        structural,
                        &mut out_atoms,
                    );
                }
            }
            // The dropped option and the `#'` markers threaded into a multi-line
            // block macro carry no projected content; any other leaf (text, and
            // the collapsed newline/whitespace trivia) is prose.
            SyntaxKind::ROXYGEN_RD_MACRO_OPT | SyntaxKind::ROXYGEN_MARKER => {}
            _ => {
                if let Some(t) = el.as_token() {
                    text_buf.push_str(t.text());
                }
            }
        }
    }
    // Defensive: trailing content with no closing brace (a malformed macro).
    flush_text(&mut text_buf, &mut pieces);
    finalize_macro_arg(
        &mut pieces,
        is_rcode_body_macro(&head),
        structural,
        &mut out_atoms,
    );
    if out_atoms.is_empty() {
        // A name-only macro node (no `{…}` content). A known zero-argument macro
        // (`\cr`, or a list child `\item` under `\itemize`) renders name-only;
        // an **unknown** brace-less `\word` is tagged `UNKNOWN` by parse_Rd.
        let name = head.trim_start_matches('\\');
        if is_known_rd_macro(name) {
            format!("({head})")
        } else {
            format!("(UNKNOWN {})", encode_text(&head))
        }
    } else {
        format!("({head} {})", out_atoms.join(" "))
    }
}

/// Whether the macro whose head is `head` (with its leading `\`) has an **R code**
/// body — projected as verbatim `(RCODE …)` line atoms rather than normalized
/// `(TEXT …)` prose. parse_Rd keeps `\code`'s body verbatim, and `\Sexpr`'s body
/// *is* R (it is evaluated), so both take the code path.
fn is_rcode_body_macro(head: &str) -> bool {
    matches!(head.trim_start_matches('\\'), "code" | "Sexpr")
}

/// A piece of a non-`@md` (or fragile) macro argument while folding bare `{…}`
/// brace groups: raw prose text (escapes unresolved) or an already-serialized atom
/// (a nested macro or a verbatim `(VERB …)`), opaque to the brace scan.
enum ArgPiece {
    Text(String),
    Atom(String),
}

/// Atomize one macro argument's [`ArgPiece`]s into `out`, folding bare `{…}` groups
/// into `(LIST …)` atoms and GRP-wrapping a structural macro's multi-atom argument.
///
/// A **verbatim** argument (`\code`'s RCODE body, `code == true`) is never grouped:
/// its braces are literal R code, so each text piece splits into `(RCODE …)` line
/// atoms and nested macros splice in. A **prose** argument folds bare groups via
/// [`group_arg_pieces`]; when that finds no group (or the braces are unbalanced —
/// the section drops via `rdComplete`), each text piece coalesces into one
/// whitespace-normalized `(TEXT …)`, byte-identical to the ungrouped path.
///
/// Finalization matches parse_Rd: a structural two-arg macro (`\item`/`\tabular`/
/// `\href`) wraps a multi-atom argument in `(GRP …)` (a bare group counts as one
/// atom); a single-atom argument or a latexlike macro's inlined content splices in.
fn finalize_macro_arg(
    pieces: &mut Vec<ArgPiece>,
    code: bool,
    structural: bool,
    out: &mut Vec<String>,
) {
    if pieces.is_empty() {
        return;
    }
    let atoms = if code {
        let mut v = Vec::new();
        for p in pieces.drain(..) {
            match p {
                ArgPiece::Text(s) => v.extend(rcode_atoms(&resolve_rd_arg_escapes(&s))),
                ArgPiece::Atom(a) => v.push(a),
            }
        }
        v
    } else if let Some(a) = group_arg_pieces(pieces) {
        pieces.clear();
        a
    } else {
        let mut v = Vec::new();
        for p in pieces.drain(..) {
            match p {
                ArgPiece::Text(s) => {
                    if let Some(a) = text_atom(&resolve_rd_arg_escapes(&s)) {
                        v.push(a);
                    }
                }
                ArgPiece::Atom(a) => v.push(a),
            }
        }
        v
    };
    if structural && atoms.len() > 1 {
        out.push(format!("(GRP {})", atoms.join(" ")));
    } else {
        out.extend(atoms);
    }
}

/// Fold a **prose** macro argument's [`ArgPiece`]s into serialized atoms, turning
/// each bare `{…}` brace pair into a `(LIST …)` atom (empty group → `(LIST)`).
/// parse_Rd lexes a braced argument with the same bare-group rule as prose text —
/// an unescaped `{`/`}` is a `LIST` delimiter — so `\emph{a {b} c}` projects
/// `(\emph (TEXT "a") (LIST (TEXT "b")) (TEXT "c"))`; groups nest and span nested
/// macros (an opaque `Atom` lands inside the group). Mirrors [`group_brace_lists`]
/// but on already-carved pieces, and unlike prose text a braced argument has **no**
/// `%` comment (an in-arg `%` is literal) and no brace-less-macro drop, so the scan
/// only weighs backslash-escaping and braces.
///
/// Brace parity matches [`resolve_rd_arg_escapes`]: an odd-length backslash run
/// escapes the following brace (`\{`/`\}` stay literal, no group), an even run opens
/// it. Text pieces keep their raw backslashes; [`text_atom`] resolves them once each
/// run flushes.
///
/// Returns `None` when the argument holds no bare group (so the caller keeps the
/// byte-identical ungrouped atomization) or the braces are unbalanced (the section
/// drops via `rdComplete` before these atoms are used — never a partial tree).
fn group_arg_pieces(pieces: &[ArgPiece]) -> Option<Vec<String>> {
    // `stack[0]` is the argument's output level; each deeper frame is an open `{`.
    let mut stack: Vec<Vec<String>> = vec![Vec::new()];
    let mut buf = String::new();
    let mut grouped = false;
    let flush = |stack: &mut Vec<Vec<String>>, buf: &mut String| {
        if !buf.is_empty() {
            if let Some(a) = text_atom(&resolve_rd_arg_escapes(buf)) {
                stack.last_mut().unwrap().push(a);
            }
            buf.clear();
        }
    };
    for piece in pieces {
        match piece {
            // A nested macro / verbatim atom is opaque to the brace scan: flush the
            // pending text and drop it into the current group.
            ArgPiece::Atom(a) => {
                flush(&mut stack, &mut buf);
                stack.last_mut().unwrap().push(a.clone());
            }
            ArgPiece::Text(s) => {
                let bytes = s.as_bytes();
                let mut i = 0usize;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => {
                            let start = i;
                            while i < bytes.len() && bytes[i] == b'\\' {
                                i += 1;
                            }
                            let run_len = i - start;
                            buf.push_str(&s[start..i]);
                            // An odd run escapes the next char (a brace stays literal):
                            // copy it verbatim so it is never read as a delimiter.
                            if run_len % 2 == 1 && i < bytes.len() {
                                let mut end = i + 1;
                                while !s.is_char_boundary(end) {
                                    end += 1;
                                }
                                buf.push_str(&s[i..end]);
                                i = end;
                            }
                        }
                        b'{' => {
                            flush(&mut stack, &mut buf);
                            stack.push(Vec::new());
                            grouped = true;
                            i += 1;
                        }
                        b'}' if stack.len() > 1 => {
                            flush(&mut stack, &mut buf);
                            let g = stack.pop().unwrap();
                            let inner = g.join(" ");
                            stack.last_mut().unwrap().push(if inner.is_empty() {
                                "(LIST)".to_string()
                            } else {
                                format!("(LIST {inner})")
                            });
                            grouped = true;
                            i += 1;
                        }
                        _ => {
                            let start = i;
                            i += 1;
                            while i < bytes.len() && !matches!(bytes[i], b'\\' | b'{' | b'}') {
                                i += 1;
                            }
                            buf.push_str(&s[start..i]);
                        }
                    }
                }
            }
        }
    }
    flush(&mut stack, &mut buf);
    // No bare group, or unbalanced braces (the section drops anyway): keep the run
    // flat, never a partial tree.
    if !grouped || stack.len() != 1 {
        return None;
    }
    Some(stack.pop().unwrap())
}

/// Project a **structural** two-arg macro (`\item`/`\tabular`/`\href`) under `@md`,
/// markdown-processing each non-verbatim argument as a single cmark run.
///
/// roxygen2 resolves a structural argument as **one** markdown run, so an emphasis
/// or link span crosses a nested Rd macro (`*a \strong{x} b*` →
/// `\emph{a \strong{x} b}`, and even `*a \tab b*` →
/// `\emph{a \tab b}` across a brace-less separator). The general `serialize_macro`
/// loop resolves each prose run between the carved macros *separately*, which
/// leaves the unmatched `*` delimiters literal. Here each argument group's
/// already-carved children are collected into [`MdArgPiece`]s — prose runs as
/// markdown-lexed text, every nested macro (braced `\strong`, brace-less
/// `\tab`/`\cr`) as one opaque token — and resolved together by
/// [`resolve_md_inline_pieces`], so the delimiter-stack arena spans the macros.
///
/// A *verbatim* argument (the `\href` URL) keeps its `(VERB …)` projection
/// untouched. A multi-atom argument `(GRP …)`-wraps (parse_Rd models it as a list);
/// a single-atom argument (e.g. one `\emph` owning the whole argument) stays bare.
fn serialize_md_structural_macro(node: &SyntaxNode, head_full: &str) -> String {
    let mut out_atoms: Vec<String> = Vec::new();
    let mut pieces: Vec<MdArgPiece> = Vec::new();
    let mut run = String::new();
    // The current argument's finished atoms. A markdown *block* child splits the
    // argument's inline run, so atoms accumulate here rather than being produced
    // once at the closing `}`.
    let mut arg_atoms: Vec<String> = Vec::new();
    // A verbatim argument projects as a single `(VERB …)`, never markdown.
    let mut verb: Option<String> = None;

    // Flush the pending prose run into a markdown-lexed text piece.
    let flush = |run: &mut String, pieces: &mut Vec<MdArgPiece>| {
        if !run.is_empty() {
            pieces.push(MdArgPiece::Text(std::mem::take(run)));
        }
    };

    // Resolve the pending pieces as one markdown inline run into `arg_atoms`.
    let flush_inlines = |pieces: &mut Vec<MdArgPiece>, arg_atoms: &mut Vec<String>| {
        if pieces.is_empty() {
            return;
        }
        let para = resolve_md_inline_pieces(pieces);
        // Fold bare `{…}` groups (Rd `LIST`s) in the resolved run.
        let grouped = group_brace_lists(&para_to_inlines(&para), true);
        arg_atoms.extend(serialize_inlines(&grouped, true));
        pieces.clear();
    };

    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_NAME => {}
            SyntaxKind::ROXYGEN_RD_MACRO_VERB => {
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                verb = Some(format!("(VERB {})", encode_text(&raw)));
            }
            // A nested macro is opaque to the markdown run: emit its raw source as
            // one piece so emphasis/links span across it.
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush(&mut run, &mut pieces);
                if let Some(n) = el.as_node() {
                    pieces.push(MdArgPiece::Macro(n.text().to_string()));
                }
            }
            // A markdown block construct inside the argument (`@md`): cmark parses
            // the field text flat, so a list or fenced block at the body's column is
            // a real block. It terminates the pending inline run and contributes its
            // own atoms, exactly as it would at section level.
            SyntaxKind::ROXYGEN_MD_LIST | SyntaxKind::ROXYGEN_MD_CODE_BLOCK => {
                flush(&mut run, &mut pieces);
                flush_inlines(&mut pieces, &mut arg_atoms);
                if let Some(n) = el.as_node() {
                    arg_atoms.extend(md_block_atoms(n));
                }
            }
            // The closing `}` of an argument group: resolve its pieces as one
            // markdown run (or emit the verbatim atom), GRP-wrapping a multi-atom
            // result. The opening `{` carries no content.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                if el.as_token().is_some_and(|t| t.text() == "}") {
                    flush(&mut run, &mut pieces);
                    if let Some(v) = verb.take() {
                        out_atoms.push(v);
                        pieces.clear();
                        arg_atoms.clear();
                    } else {
                        flush_inlines(&mut pieces, &mut arg_atoms);
                        match arg_atoms.len() {
                            0 => {}
                            1 => out_atoms.push(arg_atoms.pop().unwrap()),
                            _ => out_atoms.push(format!("(GRP {})", arg_atoms.join(" "))),
                        }
                        arg_atoms.clear();
                    }
                }
            }
            SyntaxKind::ROXYGEN_RD_MACRO_OPT | SyntaxKind::ROXYGEN_MARKER => {}
            _ => {
                if let Some(t) = el.as_token() {
                    run.push_str(t.text());
                }
            }
        }
    }
    if out_atoms.is_empty() {
        format!("({head_full})")
    } else {
        format!("({head_full} {})", out_atoms.join(" "))
    }
}

/// Whether macro `name` (without the leading `\`) has its single argument
/// **markdown-processed** when it appears inline under `@md`. roxygen2 protects
/// only its `escaped_for_md` set ([`is_fragile_for_md`]) from cmark, so *every*
/// other macro's argument is markdown — but arity already models the block and
/// multi-argument macros (`\itemize`/`\describe`/`\tabular`/`\Sexpr`/…) as their
/// own constructs, so resolving their bodies as inline prose would be wrong; they
/// are excluded here. The remainder are the inline text macros (`\emph`,
/// `\strong`, `\sQuote`, `\value`, …) whose body is a latexlike inline run.
pub(super) fn is_md_inline_text_macro(name: &str) -> bool {
    is_known_rd_macro(name)
        && !is_fragile_for_md(name)
        && !is_multi_arg_rd_macro(name)
        && !matches!(
            name,
            "itemize" | "enumerate" | "describe" | "Sexpr" | "RdOpts"
        )
}

/// Whether macro `name` (without the leading `\`) is a **structural**
/// multi-argument macro whose arguments are markdown-processed when it appears
/// under `@md`. These are the non-fragile members of [`is_multi_arg_rd_macro`]
/// (`\item`, `\tabular`, `\href`, `\enc`, `\subsection`) --- `\figure`, `\if`,
/// `\ifelse`, `\method`, `\S3method`, and `\S4method` are fragile
/// ([`is_fragile_for_md`]), so they stay literal. Unlike a latexlike single-arg
/// macro ([`is_md_inline_text_macro`]), each `{…}` argument is resolved
/// independently and a multi-atom one wraps in `(GRP …)`.
fn is_md_structural_macro(name: &str) -> bool {
    is_known_rd_macro(name) && !is_fragile_for_md(name) && is_multi_arg_rd_macro(name)
}

/// The raw source text of a single-argument macro's `{…}` content (everything
/// between the first `{` delimiter and its matching `}`), or `None` if the macro
/// has no argument group. See [`macro_arg_contents`].
pub(super) fn macro_single_arg_content(node: &SyntaxNode) -> Option<String> {
    macro_arg_contents(node).into_iter().next()
}

/// The raw source text of each of a macro's `{…}` argument groups, in order
/// (everything between a `{` delimiter and its matching `}`). Empty when the
/// macro has no argument group. Nested macros contribute their *source* (their
/// own braces live inside the child node, not as direct delimiters), so each
/// result re-lexes faithfully; threaded `#'` markers are dropped (defensive — an
/// inline macro carries none).
pub(super) fn macro_arg_contents(node: &SyntaxNode) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                let text = el.as_token().map(|t| t.text()).unwrap_or_default();
                if text == "{" && !inside {
                    inside = true;
                } else if text == "}" && inside {
                    inside = false;
                    args.push(std::mem::take(&mut current));
                }
            }
            SyntaxKind::ROXYGEN_MARKER => {}
            _ if inside => match el {
                NodeOrToken::Node(n) => current.push_str(&n.text().to_string()),
                NodeOrToken::Token(t) => current.push_str(t.text()),
            },
            _ => {}
        }
    }
    // An unterminated final group (a block macro that ran to the block's end)
    // still contributes its content.
    if inside {
        args.push(current);
    }
    args
}

/// Resolve a non-fragile macro's raw argument `content` as a `@md` markdown inline
/// run, returning the projected inline elements. Reuses the real inline pass
/// ([`resolve_md_inline`]) and the ordinary inline collector, so emphasis, links,
/// code spans, and nested macros resolve exactly as in `@md` prose.
pub(super) fn resolve_macro_arg_inlines(content: &str) -> Vec<Inline> {
    para_to_inlines(&resolve_md_inline(content))
}

/// Collect a resolved `ROXYGEN_PARAGRAPH` node's children into projected inline
/// elements: the threaded `#'` markers drop and a soft `NEWLINE` becomes a space
/// (norm_ws-equivalent), everything else projects via [`push_inline`].
pub(super) fn para_to_inlines(para: &SyntaxNode) -> Vec<Inline> {
    let mut out = Vec::new();
    for el in para.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => {}
            SyntaxKind::NEWLINE => out.push(Inline::Text(SOFT_BREAK.to_string())),
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// The macro head (`\name`, with the leading `\`) of a `ROXYGEN_RD_MACRO` node,
/// or `""` if it has no name leaf.
pub(super) fn macro_head(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .find(|el| el.kind() == SyntaxKind::ROXYGEN_RD_MACRO_NAME)
        .and_then(|el| el.as_token().map(|t| t.text().to_string()))
        .unwrap_or_default()
}

/// Whether a `ROXYGEN_RD_MACRO` node is in **block form** — it spans `#'` lines,
/// threading the inter-line markers as direct trivia children. A single-line
/// macro node (expanded from an atomic `RoxygenRdMacro` token) never holds one.
fn threads_markers(node: &SyntaxNode) -> bool {
    node.children_with_tokens()
        .any(|el| el.kind() == SyntaxKind::ROXYGEN_MARKER)
}

/// Project a verbatim-body macro (`\preformatted`, or a block-form `\eqn`/
/// `\deqn`/`\out`/… — see [`is_verbatim_rd_macro`]) by reconstructing its raw
/// argument text from the node: parse_Rd treats the body as verbatim (nested
/// `\macro` markup and markdown stay literal), so the CST's richer structure is
/// flattened back to bytes — reconstructing from the node text rather than
/// walking typed children stays faithful and mirrors [`serialize_md_html_block`].
/// Each continuation `#'` line drops its marker (and the single following
/// whitespace character), the lines rejoin with `\n`, and [`verb_atoms`] splits
/// each `{…}` argument at newlines exactly as parse_Rd does. A multi-argument
/// macro (`\eqn`/`\deqn` — [`rd_macro_arity`]) may carry further consumed
/// groups; each group is a list argument, so a multi-atom one wraps in `(GRP …)`
/// while a single atom splices in (the [`finalize_macro_arg`] rule).
///
/// parse_Rd resolves the Rd-string escapes inside most verbatim bodies (`\{` ->
/// `{`, `\%` -> `%`, `\\` -> `\`) — so the rd_complete scan counts a balanced
/// pair — but keeps them **raw** in `\eqn`/`\deqn`, whose LaTeX-like text passes
/// through untouched (engine-probed: `\eqn{50\% off}` -> `(VERB "50\% off")`).
/// Escape-aware brace *pairing* applies in both regimes.
fn serialize_verbatim_block(node: &SyntaxNode, head_full: &str) -> String {
    let name = head_full.trim_start_matches('\\');
    // Reconstruct from the name leaf on (dropping a top-level form's leading
    // `#'` marker), stripping each continuation line's marker: the logical text
    // roxygen2 writes into the Rd file.
    let text = node.text().to_string();
    let name_at = node
        .children_with_tokens()
        .find(|el| el.kind() == SyntaxKind::ROXYGEN_RD_MACRO_NAME)
        .map(|el| usize::from(el.text_range().start() - node.text_range().start()))
        .unwrap_or(0);
    let mut logical = String::new();
    for (idx, line) in text[name_at..].split('\n').enumerate() {
        if idx == 0 {
            logical.push_str(line);
        } else {
            logical.push('\n');
            logical.push_str(strip_marker(line));
        }
    }
    let bytes = logical.as_bytes();
    let mut j = head_full.len();
    // An `[opt]` group is dropped, exactly as in the generic path.
    if bytes.get(j) == Some(&b'[')
        && let Some(end) = logical[j..].find(']')
    {
        j += end + 1;
    }
    let structural = is_multi_arg_rd_macro(name);
    let arity = rd_macro_arity(name);
    let raw_escapes = matches!(name, "eqn" | "deqn");
    let mut out_atoms: Vec<String> = Vec::new();
    let mut groups = 0usize;
    while bytes.get(j) == Some(&b'{') && groups < arity {
        // parse_Rd's escape-aware brace pairing: an escaped `\{`/`\}` is literal
        // in both escape regimes. An unterminated body (no closing `}` — the
        // macro ended at a tag opener or block end) runs to the node's end.
        let mut depth = 1usize;
        let mut k = j + 1;
        while k < bytes.len() {
            match bytes[k] {
                b'\\' => k += 2,
                b'{' => {
                    depth += 1;
                    k += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    k += 1;
                }
                _ => k += 1,
            }
        }
        let body = &logical[j + 1..k.min(logical.len())];
        let resolved = if raw_escapes {
            body.to_string()
        } else {
            resolve_rd_arg_escapes(body)
        };
        let atoms = verb_atoms(&resolved);
        if structural && atoms.len() > 1 {
            out_atoms.push(format!("(GRP {})", atoms.join(" ")));
        } else {
            out_atoms.extend(atoms);
        }
        groups += 1;
        j = (k + 1).min(logical.len());
    }
    if out_atoms.is_empty() {
        format!("({head_full})")
    } else {
        format!("({head_full} {})", out_atoms.join(" "))
    }
}
