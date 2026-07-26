use super::*;

/// Reconstruct a field's markdown **source** text for link-reference scanning:
/// the literal prose (`Inline::Text`) verbatim, with every *resolved* inline (a
/// link/code/macro/emphasis arity already turned into structure) flattened to a
/// single space. A resolved inline is never a leaked-definition candidate —
/// roxygen2 either linkified it (a valid, consumed definition) or it is a non-link
/// macro — and a space keeps it a neutral boundary that cannot fuse adjacent
/// brackets into a spurious span.
pub(super) fn inline_source_skeleton(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        s.push_str(&inline_skeleton_fragment(inl));
    }
    s
}

/// The markdown-source fragment an inline contributes to [`inline_source_skeleton`]:
/// the literal prose (`Inline::Text`) verbatim, and a single space for every
/// *resolved* inline arity already turned into structure.
///
/// **Inline links are the exception.** roxygen2's `get_md_linkrefs` synthesizes a
/// `[text]: R:text` reference definition for an inline `[text](url)` link too (its
/// `[text]` is a bracket-free shortcut candidate followed by `(`, which the regex
/// lookahead allows), so in a poisoned tail that definition leaks even though the
/// `\href` itself survives (the link carries its own destination). The skeleton
/// therefore exposes the link's bracketed display text — `[text]` followed by a
/// space placeholder for the consumed `(url)` — so the link-reference scan sees the
/// candidate. **Images are handled the same way:** an image `![alt](url)`'s `[alt]`
/// is a bracket-free candidate too (the `[` is preceded by `!`, allowed, and
/// followed by `(`, lookahead-allowed), so its synthesized `[alt]: R:alt`
/// definition leaks in a poisoned tail even though the `\figure` survives (it
/// carries its own destination). The node-form (`MdInlineLink`/`MdImage`) is
/// handled; an **opaque** inline-link leaf is handled too — a nested-bracket
/// display (`[a [b] c](url)`) keeps the link opaque (the lexer only nodes a
/// bracket-free display), yet `get_md_linkrefs` still finds the *inner*
/// bracket-free `[b]` candidate, so the display is exposed verbatim. Autolinks
/// (`<url>`) carry no bracket candidate, so they stay a single space.
pub(super) fn inline_skeleton_fragment(inl: &Inline) -> Cow<'_, str> {
    match inl {
        Inline::Text(t) => Cow::Borrowed(t),
        Inline::MdInlineLink { display, .. } => {
            Cow::Owned(format!("[{}] ", inline_plain_text(display)))
        }
        Inline::MdImage(raw) => match image_skeleton_fragment(raw) {
            Some(fragment) => Cow::Owned(fragment),
            None => Cow::Borrowed(SKELETON_STAND_IN_STR),
        },
        // An opaque inline-link leaf (nested-bracket display): expose the raw
        // display so its inner bracket-free candidate surfaces, with a space
        // placeholder for the consumed `(url)`. A shortcut/reference leaf is
        // handled by demotion, an autolink carries no candidate — both a stand-in.
        Inline::MdLink(raw) => match opaque_inline_link_display(raw) {
            Some(display) => Cow::Owned(format!("[{display}] ")),
            None => Cow::Borrowed(SKELETON_STAND_IN_STR),
        },
        // A markdown list is part of the same cmark document, so an escaped-close
        // candidate (or a post-demotion literal bracket) inside a list item must
        // surface in the whole-field poisoning skeleton. Recurse into each item,
        // space-guarded per item (the raw source separates items with newlines, so
        // a `[` opening an item is never seen as preceded by the previous item's
        // `]`) — the same shape as `linkref_skeleton_push_exact`, and the offset walk in
        // `demote_poisoned_walk` stays byte-aligned with this.
        Inline::MdList(node) => {
            let mut s = String::new();
            for item in node
                .children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
            {
                s.push(' ');
                for child in md_list_item_inlines(&item) {
                    s.push_str(&inline_skeleton_fragment(&child));
                }
            }
            s.push(' ');
            Cow::Owned(s)
        }
        Inline::MdListResolved { items, .. } => {
            let mut s = String::new();
            for item in items {
                s.push(' ');
                for child in item {
                    s.push_str(&inline_skeleton_fragment(child));
                }
            }
            s.push(' ');
            Cow::Owned(s)
        }
        _ => Cow::Borrowed(SKELETON_STAND_IN_STR),
    }
}

/// One-byte stand-in for resolved or opaque structure in
/// [`inline_source_skeleton`]. Deliberately **non-whitespace**: the source it
/// replaces (a resolved link, emphasis run, code span, autolink, macro, …) always
/// carries non-whitespace bytes, so a literal `[` + resolved inner link + literal
/// `]` must not read as a *blank* link-reference label
/// ([`linkref_label_is_blank`]) and spuriously poison the field (cm-550, cm-592,
/// `md_nested_link`). Otherwise neutral to the candidate scan — not
/// `[`/`]`/`\`/`{`, non-blocking exactly like the space it replaces. The list
/// guards stay real spaces (they stand for source *newlines*, genuine whitespace).
const SKELETON_STAND_IN_STR: &str = "\u{1}";

/// The verbatim bracketed display of an **opaque inline-link** leaf
/// (`[display](url)` → `display`), or `None` for a shortcut/reference leaf (no `(`
/// after the balanced display) or an autolink (`<url>`). The display is taken
/// verbatim because it is what roxygen2's `get_md_linkrefs` scans for *inner*
/// bracket-free candidates — a nested-bracket display `[a [b] c]` yields a `[b]`
/// candidate (the outer `[a [b] c]` is not a candidate: its content has brackets).
/// Only nested-bracket displays reach here; a bracket-free inline link is a
/// `ROXYGEN_MD_LINK` *node* (`MdInlineLink`), handled above.
pub(super) fn opaque_inline_link_display(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    if bytes.first() == Some(&b'<') {
        return None; // autolink
    }
    let text_end = scan_delimited(bytes, 0, b'[', b']')?;
    // Inline link iff the balanced display is immediately followed by `(`.
    (bytes.get(text_end) == Some(&b'(')).then(|| &raw[1..text_end - 1])
}

/// The literal alt-text span of an image leaf (`![alt](url)` → `alt`), or `None`
/// if the leaf does not open with a balanced `![…]`. The alt is taken verbatim
/// from the source (it is what roxygen2's `get_md_linkrefs` scans for a `[alt]`
/// candidate), so it is not markdown-resolved.
pub(super) fn image_alt_text(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    // The leaf always begins `![`; the alt span is `[…]` starting at index 1.
    let alt_end = scan_delimited(bytes, 1, b'[', b']')?;
    Some(&raw[2..alt_end - 1])
}

/// Whether an image leaf is the **collapsed** reference form `![alt][]` — the
/// shape that resolves only through a user definition ([`image_user_ref`]) and
/// whose skeleton exposure must re-emit the trailing `[]` (the `[alt]` span is
/// followed by `[` in the real source, so `get_md_linkrefs`' `(?=[^\[{])`
/// lookahead blocks its candidate — a space stand-in would spuriously unblock it).
pub(super) fn image_is_collapsed(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let Some(alt_end) = scan_delimited(bytes, 1, b'[', b']') else {
        return false;
    };
    alt_end + 2 == bytes.len() && bytes[alt_end..] == *b"[]"
}

/// The candidate-scan exposure of an image leaf in the link-reference skeletons:
/// the verbatim `[alt]` bracket, followed by what the real source follows it with
/// — the literal `[]` for a collapsed form (candidate-blocking, see
/// [`image_is_collapsed`]), a space stand-in for a consumed `(url)`/`[ref]`
/// otherwise (non-blocking like the real `(`). `None` when the leaf has no
/// balanced alt span (exposed as a plain stand-in by the callers).
fn image_skeleton_fragment(raw: &str) -> Option<String> {
    let alt = image_alt_text(raw)?;
    Some(if image_is_collapsed(raw) {
        format!("[{alt}][]")
    } else {
        format!("[{alt}] ")
    })
}

/// The **leaked** link-reference definitions roxygen2's `add_linkrefs_to_md`
/// appends to a markdown field's rendered text (`markdown-link.R`).
///
/// roxygen2 scans the (double-escaped) field text with `get_md_linkrefs` and, for
/// **every** bracket-free `[…]` shortcut candidate, appends a synthesized
/// `[label]: R:URLencode(label)` reference definition so cmark resolves the
/// shortcut as a link to `R:label`. A candidate whose closing bracket is
/// **backslash-escaped** (`[text\]`) yields a definition whose own label never
/// closes — *not* a valid CommonMark link reference definition — so cmark leaves it
/// as **literal text**, and it leaks into the rendered Rd as trailing prose
/// (`… [text]: R:text%5C`). arity already renders such a shortcut literally (the
/// lexer never pairs an escaped-close bracket); this models the leaked *definition*.
///
/// **Whole-field poisoning.** The synthesized definitions are appended as one
/// block (each candidate on its own line, in source order). cmark parses it
/// top-down: a *valid* candidate's definition closes and is consumed (the shortcut
/// becomes a link, which arity resolves via its own link path). But the **first
/// invalid** (escaped-close) candidate's label never closes — it runs into the next
/// line's `[`, which is illegal inside a link label — so that definition *and every
/// definition after it* fail to parse and leak as literal text (a definition cannot
/// interrupt the paragraph the failed one started). So the leaked block begins at
/// the first invalid candidate and runs to the end, **valid candidates included**.
/// Correspondingly, a shortcut/reference link whose definition is in that leaked
/// tail is de-linked in the body — handled upstream by [`demote_poisoned_links`],
/// which rewrites those links to literal bracket text *before* the skeleton is
/// built, so they reappear here as the candidates whose definitions leak.
///
/// Returns the leaked definition lines as cmark **sees** them (escapes still
/// active — `\]` alive, backslashes as in the escaped document), in document
/// order; [`append_leaked_defs`] resolves them as a markdown fragment, whose
/// text pipeline performs the final unescape. `@md` only; empty when no
/// candidate is invalid.
pub(super) fn leaked_linkref_text(source: &str) -> Vec<String> {
    // The skeleton stands in for the raw markdown source, whose soft wraps are
    // real newlines — map the sentinel back so a multi-line label URL-encodes as
    // roxygen2's does (`%0A`, cm-554). Same byte width, so offsets are unmoved.
    let escaped = double_escape_md(&source.replace(SOFT_BREAK, "\n"));
    let labels = md_linkref_labels(&escaped);
    let Some(first_invalid) = labels
        .iter()
        .position(|label| !linkref_label_closes(label) || linkref_label_is_blank(label))
    else {
        return Vec::new();
    };
    labels[first_invalid..]
        .iter()
        .map(|label| format!("[{label}]: R:{}", url_encode(label)))
        .collect()
}

/// Rewrite the shortcut/reference links that a leaked link-reference block
/// de-links (see [`leaked_linkref_text`]) into literal bracket text.
///
/// roxygen2's `add_linkrefs_to_md` appends one synthesized `[label]: R:…`
/// definition per bracket-free `[…]` candidate. The **first invalid** (escaped-close)
/// candidate poisons every definition after it, so any shortcut or reference link
/// occurring after that point loses its definition and renders literally. The
/// escaped-close candidate itself is already literal text (the lexer never pairs an
/// escaped-close bracket), so it lives in a `Text` inline; once such a `Text` is
/// seen, every following shortcut/reference link node is demoted to its source
/// bracket text. Demoting *before* the skeleton is built means those links reappear
/// as candidates, so their now-leaked definitions surface naturally.
///
/// Inline links (`[text](url)`), autolinks (`<url>`), and code spans do **not**
/// need a reference definition, so they survive poisoning and are left untouched.
/// `@md` only; returns `None` (no rewrite) when the body has no invalid candidate.
pub(super) fn demote_poisoned_links(body: &[Inline]) -> Option<Vec<Inline>> {
    // The poisoning boundary is found on the whole-body skeleton (an escaped-close
    // candidate can straddle several inlines — the lexer splits `[stop\]` into a
    // `Text` plus a leftover `]` delimiter), then mapped back by skeleton offset.
    // The skeleton descends into list items (see `inline_skeleton_fragment`), so
    // the demotion walk must descend identically to keep offsets aligned.
    let skeleton = inline_source_skeleton(body);
    let boundary = first_invalid_linkref_offset(&skeleton)?;
    let mut offset = 0;
    let mut changed = false;
    let out = demote_poisoned_walk(body, boundary, &mut offset, &mut changed);
    Some(relink_demoted_inline_links(out))
}

/// Recursive offset-threaded walk for [`demote_poisoned_links`]: demote every
/// shortcut/reference link whose skeleton offset starts after `boundary` to its
/// literal bracket source, descending into list items with the same per-item
/// space-guard offset accounting as [`inline_skeleton_fragment`]'s list arms (so
/// the boundary maps back consistently). `offset` advances inline-by-inline;
/// `changed` is set when any inline is rewritten. A list whose items change becomes
/// an `MdListResolved`; an untouched list keeps its opaque `MdList` form (so its
/// serialization stays byte-identical).
fn demote_poisoned_walk(
    body: &[Inline],
    boundary: usize,
    offset: &mut usize,
    changed: &mut bool,
) -> Vec<Inline> {
    let mut out = Vec::with_capacity(body.len());
    for inl in body {
        match inl {
            Inline::MdList(node) => {
                let items: Vec<Vec<Inline>> = node
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                    .map(|item| md_list_item_inlines(&item))
                    .collect();
                let mut item_changed = false;
                let new_items = demote_poisoned_items(&items, boundary, offset, &mut item_changed);
                if item_changed {
                    *changed = true;
                    out.push(Inline::MdListResolved {
                        ordered: md_list_is_ordered(node),
                        items: new_items,
                    });
                } else {
                    out.push(inl.clone());
                }
            }
            Inline::MdListResolved { ordered, items } => {
                let mut item_changed = false;
                let new_items = demote_poisoned_items(items, boundary, offset, &mut item_changed);
                if item_changed {
                    *changed = true;
                    out.push(Inline::MdListResolved {
                        ordered: *ordered,
                        items: new_items,
                    });
                } else {
                    out.push(inl.clone());
                }
            }
            _ => {
                let start = *offset;
                *offset += skeleton_len(inl);
                if start > boundary
                    && let Some(text) = demoted_link_source(inl)
                {
                    *changed = true;
                    out.push(Inline::Text(text));
                } else {
                    out.push(inl.clone());
                }
            }
        }
    }
    out
}

/// Walk a list's items for [`demote_poisoned_walk`], advancing `offset` with the
/// per-item leading space and overall trailing space that
/// [`inline_skeleton_fragment`]'s list arm emits, so offsets stay byte-aligned.
fn demote_poisoned_items(
    items: &[Vec<Inline>],
    boundary: usize,
    offset: &mut usize,
    item_changed: &mut bool,
) -> Vec<Vec<Inline>> {
    let mut new_items = Vec::with_capacity(items.len());
    for item in items {
        *offset += 1; // the per-item leading space guard
        new_items.push(demote_poisoned_walk(item, boundary, offset, item_changed));
    }
    *offset += 1; // the list's trailing space
    new_items
}

/// Re-form enclosing inline links that a poisoning demotion exposes.
///
/// When a nested link's inner shortcut is de-linked by poisoning (the arena
/// resolves the optimistic CommonMark structure — inner link wins, outer bracket
/// literal — so [`demote_poisoned_links`] rewrites the now-dead inner shortcut to
/// literal text), the enclosing brackets are no longer deactivated by a live inner
/// link, so roxygen2 (cmark) resolves the *outer* `[…](url)` as an inline link
/// instead. Concretely `[a [b] c](url)` parses to literal `[a `, `\link{b}`,
/// literal ` c](url)`; once `[b]` is demoted to text the whole span is consecutive
/// literal text and re-forms as `\href{url}{a [b] c}`.
///
/// Scans each maximal run of consecutive `Inline::Text` for an *unescaped*
/// `[display](url)` inline-link pattern and splits it into `Text` + `MdInlineLink`.
/// The consecutive-text constraint is what scopes this to the poisoned case: a
/// surviving inner inline link is a node that interrupts the run (so a nested
/// inline-in-inline like `[a [b](u) c](o)` keeps its outer bracket literal, exactly
/// as CommonMark does), and a non-poisoned nested link still has its inner `\link`
/// node interrupting the run. An escaped `\[` keeps its backslash in the text
/// inline (unescaping happens later in `process_prose`), so it is skipped here and
/// stays literal — `\[bracket](x)` never relinks. Shortcuts/references in a poisoned
/// tail are dead, so only inline `(url)` links re-form; the re-formed display is
/// therefore plain literal text.
fn relink_demoted_inline_links(body: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(body.len());
    let mut text_run = String::new();
    for inl in body {
        match inl {
            Inline::Text(s) => text_run.push_str(&s),
            other => {
                relink_text_run(&text_run, &mut out);
                text_run.clear();
                out.push(other);
            }
        }
    }
    relink_text_run(&text_run, &mut out);
    out
}

/// Push `s` to `out`, re-forming any unescaped `[display](url)` inline link into an
/// `Inline::MdInlineLink` (display as plain text — see [`relink_demoted_inline_links`]).
fn relink_text_run(s: &str, out: &mut Vec<Inline>) {
    if s.is_empty() {
        return;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut run_start = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && !(i > 0 && bytes[i - 1] == b'\\')
            && let Some(text_end) = scan_delimited(bytes, i, b'[', b']')
            && bytes.get(text_end) == Some(&b'(')
            && let Some(url_end) = scan_delimited(bytes, text_end, b'(', b')')
        {
            if run_start < i {
                out.push(Inline::Text(s[run_start..i].to_string()));
            }
            let display = s[i + 1..text_end - 1].to_string();
            let url = s[text_end + 1..url_end - 1].to_string();
            out.push(Inline::MdInlineLink {
                url,
                display: vec![Inline::Text(display)],
            });
            i = url_end;
            run_start = i;
            continue;
        }
        i += 1;
    }
    if run_start < s.len() {
        out.push(Inline::Text(s[run_start..].to_string()));
    }
}

/// An inline's byte length in [`inline_source_skeleton`] — the length of its
/// [`inline_skeleton_fragment`], so the poisoning-boundary offset maps back onto
/// the body inline-by-inline.
pub(super) fn skeleton_len(inl: &Inline) -> usize {
    inline_skeleton_fragment(inl).len()
}

/// The literal source bracket text for a shortcut or reference link that a leaked
/// definition block de-links, or `None` for any inline that survives poisoning (an
/// inline link, autolink, code span, macro, or plain text — none of which depend on
/// a reference definition). Used by [`demote_poisoned_links`].
pub(super) fn demoted_link_source(inl: &Inline) -> Option<String> {
    match inl {
        Inline::MdShortcutLink { display } => Some(format!("[{}]", link_label_text(display))),
        Inline::MdRefLink { dest, display } => {
            Some(format!("[{}][{}]", link_label_text(display), dest))
        }
        // The opaque same-line leaf is its own verbatim source; demote only the
        // shortcut/reference forms (an inline link or autolink survives).
        Inline::MdLink(raw) if opaque_link_is_shortcut_or_ref(raw) => Some(raw.clone()),
        _ => None,
    }
}

/// Whether an opaque `ROXYGEN_MD_LINK` leaf is a shortcut (`[dest]`) or reference
/// (`[text][ref]`) link — the forms that depend on a reference definition and are
/// thus de-linked by poisoning — rather than an inline link (`[text](url)`) or
/// autolink (`<url>`), which carry their own destination. Mirrors the closer
/// dispatch in [`resolve_md_link`].
fn opaque_link_is_shortcut_or_ref(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.first() == Some(&b'<') {
        return false; // autolink
    }
    let Some(text_end) = scan_delimited(bytes, 0, b'[', b']') else {
        return false;
    };
    // `(` → inline link (own destination); `[` → reference; nothing → shortcut.
    !matches!(bytes.get(text_end), Some(&b'('))
}

/// The set of normalized link-reference labels roxygen2 **defines** for a markdown
/// field — its *link-reference map*.
///
/// roxygen2's `add_linkrefs_to_md` synthesizes a `[label]: R:…` definition for
/// every bracket-free `[…]` shortcut candidate found by `get_md_linkrefs`, and
/// cmark resolves a shortcut or reference link only when its (normalized) label is
/// one of those definitions. The candidate scan's `(?<!\])` lookbehind skips a `[`
/// immediately preceded by `]` and its `(?=[^\[{])` lookahead skips one followed by
/// `[`/`{`, so a bracketed span in those positions defines nothing — and a link
/// using such a label stays literal unless the label is *also* defined by some
/// other candidate in the same field (e.g. a standalone `[b]` elsewhere defines `b`
/// for an `a][b]`). arity's arena resolves every shortcut optimistically, so
/// [`demote_undefined_links`] uses this map to drop the ones roxygen2 leaves literal.
///
/// The map is built from a faithful reconstruction of the field's raw markdown
/// source ([`linkref_source_skeleton`]) — every link/image bracket re-exposed — so
/// the candidate scan ([`md_linkref_scan`], the same port the poisoning path uses)
/// sees what roxygen2 saw before parsing.
pub(super) fn linkref_keys(body: &[Inline]) -> std::collections::HashSet<String> {
    md_linkref_scan(&linkref_source_skeleton(body))
        .into_iter()
        .map(|(label, _)| normalize_linkref_label(&label))
        .collect()
}

/// Reconstruct a markdown field's raw source from its resolved inline body,
/// re-exposing the bracket text of every link and image so the link-reference
/// candidate scan ([`md_linkref_scan`]) sees the same `[…]` spans roxygen2 scanned
/// before parsing. Unlike [`inline_source_skeleton`] — which renders a *resolved*
/// shortcut/reference link as a single space because the poisoning path handles it
/// positionally — this exposes those brackets so [`linkref_keys`] can decide which
/// labels are defined at all.
///
/// The reconstruction is faithful with respect to the candidate scan's lookbehind
/// and lookahead: a shortcut/reference link re-emits its trailing `]` (so a span
/// right after it is correctly seen as preceded by `]`), an inline link/image
/// re-emits `[display] `/`[alt] ` (a space stands in for the consumed `(url)`,
/// non-blocking like the real `)`), and emphasis children recurse between space
/// guards (the dropped `*`/`_` markers were non-blocking too). A code span and
/// other resolved inlines contribute a single space.
fn linkref_source_skeleton(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        linkref_skeleton_push_exact(inl, &mut s, false);
    }
    s
}

/// [`linkref_source_skeleton`] with **source-exact** display reconstruction: a
/// resolved emphasis inside a shortcut/reference display re-emits its
/// delimiters ([`display_source_text`]) instead of flattening them away. The
/// *leaked-definition* scan needs this — the flatten turns `[a\*b\*]` into
/// `[a\b\]`, whose trailing backslash fabricates an invalid candidate and a
/// spurious leak (the true label `a\*b\*` is valid, so roxygen2 leaks
/// nothing). [`linkref_keys`] keeps the flatten form: the link-side label
/// lookup ([`link_ref_label`]) flattens too, and the refmap only works while
/// both sides flatten identically.
pub(super) fn leak_source_skeleton(body: &[Inline]) -> String {
    let mut s = String::new();
    for inl in body {
        linkref_skeleton_push_exact(inl, &mut s, true);
    }
    s
}

/// A display's source text with resolved inline structure re-exposed: emphasis
/// re-emits `*`/`**` delimiters, a code span its backticks. The delimiter
/// *character* is not recorded on the node (`_`-emphasis reconstructs as `*`) —
/// an approximation that preserves what the leak scan weighs (label validity
/// and shape), noted as backlog for a leaked emphasis-bearing label's exact
/// `R:` bytes.
fn display_source_text(display: &[Inline]) -> String {
    let mut s = String::new();
    for inl in display {
        match inl {
            Inline::Text(t) => s.push_str(t),
            Inline::MdEmphasis { strong, children } => {
                let d = if *strong { "**" } else { "*" };
                s.push_str(d);
                s.push_str(&display_source_text(children));
                s.push_str(d);
            }
            Inline::MdCode(t) => {
                s.push('`');
                s.push_str(t);
                s.push('`');
            }
            other => s.push_str(&inline_plain_text(std::slice::from_ref(other))),
        }
    }
    s
}

pub(super) fn linkref_skeleton_push_exact(inl: &Inline, s: &mut String, exact: bool) {
    let label = |display: &[Inline]| {
        if exact {
            display_source_text(display)
        } else {
            link_label_text(display)
        }
    };
    match inl {
        Inline::Text(t) => s.push_str(t),
        Inline::MdShortcutLink { display } => {
            s.push('[');
            s.push_str(&label(display));
            s.push(']');
        }
        Inline::MdRefLink { dest, display } => {
            s.push('[');
            s.push_str(&label(display));
            s.push_str("][");
            s.push_str(dest);
            s.push(']');
        }
        Inline::MdInlineLink { display, .. } => {
            s.push('[');
            s.push_str(&inline_plain_text(display));
            s.push_str("] ");
        }
        Inline::MdImage(raw) => match image_skeleton_fragment(raw) {
            Some(fragment) => s.push_str(&fragment),
            None => s.push(' '),
        },
        // An opaque leaf is its own verbatim source — a same-line shortcut/reference
        // (`[*foo*]`/`[t][r]`), a nested-bracket inline link (whose inner `[b]` is a
        // candidate), or an autolink (no bracket). All are faithful as-is.
        Inline::MdLink(raw) => s.push_str(raw),
        Inline::MdEmphasis { children, .. } => {
            s.push(' ');
            for child in children {
                linkref_skeleton_push_exact(child, s, exact);
            }
            s.push(' ');
        }
        // A markdown list is part of the same cmark document, so its items'
        // brackets are link-reference candidates for the whole-field refmap (a
        // label defined inside a list resolves a reference anywhere in the field,
        // and vice versa). Each item is space-guarded — the raw source separates
        // items with newlines, so a `[` opening an item is never seen as preceded
        // by the previous item's closing `]`.
        Inline::MdList(node) => {
            for item in node
                .children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
            {
                s.push(' ');
                for child in md_list_item_inlines(&item) {
                    linkref_skeleton_push_exact(&child, s, exact);
                }
            }
            s.push(' ');
        }
        Inline::MdListResolved { items, .. } => {
            for item in items {
                s.push(' ');
                for child in item {
                    linkref_skeleton_push_exact(child, s, exact);
                }
            }
            s.push(' ');
        }
        _ => s.push(' '),
    }
}

/// Normalize a link-reference label for matching, mirroring cmark's
/// `normalize_reference`: trim and fold internal whitespace runs to one space
/// (**ASCII** whitespace only — `cmark_isspace`; a NBSP is label content), then
/// apply the full Unicode case fold (CaseFolding C+F: `ẞ`/`SS` → `ss`, cm-542),
/// not a mere lowercase.
fn normalize_linkref_label(label: &str) -> String {
    let collapsed = label
        .split(|c: char| c.is_ascii_whitespace())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    crate::roxygen::casefold::case_fold(&collapsed)
}

/// The resolution label of a shortcut or reference link — the label that must be
/// defined in the field's link-reference map for the link to resolve — or `None`
/// for any inline that does not depend on a reference definition (an inline link or
/// autolink carries its own destination; text/code/macros are not links). For a
/// reference link the label is the `[ref]` topic; for a shortcut — or a *collapsed*
/// reference (`[text][]`, an empty `dest`) — it is the display.
pub(super) fn link_ref_label(inl: &Inline) -> Option<String> {
    match inl {
        Inline::MdShortcutLink { display } => Some(link_label_text(display)),
        Inline::MdRefLink { dest, display } => Some(if dest.is_empty() {
            link_label_text(display)
        } else {
            dest.clone()
        }),
        Inline::MdLink(raw) => opaque_link_ref_label(raw),
        _ => None,
    }
}

/// Whether a link's resolution label can never define or resolve in roxygen2's
/// pipeline ([`linkref_label_is_usable`]) — judged on a **source-exact** label
/// only. A reference link's `dest` and an opaque leaf's bracket text are verbatim
/// source; a shortcut's (or collapsed reference's) label is its display flatten,
/// which is source-exact only when every display inline is plain `Inline::Text`
/// (an emphasis or code-span flatten drops its source delimiters, so
/// `[a\*b\*]`'s flatten `a\b\` spuriously ends in a backslash while the real
/// label `a\*b\*` closes fine — that link must instead reach the non-plain-display
/// drop decision in `serialize_inlines`).
fn link_ref_label_unusable(inl: &Inline) -> bool {
    let display_unusable = |display: &[Inline]| {
        display.iter().all(|d| matches!(d, Inline::Text(_)))
            && !linkref_label_is_usable(&link_label_text(display))
    };
    match inl {
        Inline::MdShortcutLink { display } => display_unusable(display),
        Inline::MdRefLink { dest, display } => {
            if dest.is_empty() {
                display_unusable(display)
            } else {
                !linkref_label_is_usable(dest)
            }
        }
        Inline::MdLink(raw) => {
            opaque_link_ref_label(raw).is_some_and(|label| !linkref_label_is_usable(&label))
        }
        _ => false,
    }
}

/// The resolution label of an *opaque* shortcut/reference `ROXYGEN_MD_LINK` leaf
/// (`[dest]` → `dest`, `[text][ref]` → `ref`), or `None` for an inline link
/// (own destination) or autolink. Mirrors the closer dispatch in [`resolve_md_link`].
fn opaque_link_ref_label(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.first() == Some(&b'<') {
        return None; // autolink
    }
    let text_end = scan_delimited(bytes, 0, b'[', b']')?;
    match bytes.get(text_end) {
        Some(&b'(') => None, // inline link — own destination
        Some(&b'[') => {
            let ref_end = scan_delimited(bytes, text_end, b'[', b']')?;
            Some(raw[text_end + 1..ref_end - 1].to_string())
        }
        _ => Some(raw[1..text_end - 1].to_string()), // shortcut
    }
}

/// One bracket unit of an adjacent shortcut/reference-link chain, for the
/// refmap-dependent re-pairing scan ([`repair_ref_link_chains`]). A reference
/// link contributes two units (its display and its `[label]`); a shortcut one.
struct ChainUnit {
    /// The unit's resolved display inlines when it serves as a link *display*
    /// (a label unit carries its source-exact label text as a single `Text`).
    display: Vec<Inline>,
    /// The refmap-lookup label when the unit serves as a *label* or *shortcut*:
    /// a label unit's source-exact `dest`, or a display flatten (mirrors
    /// [`link_ref_label`]). `None` = never resolves (an unusable source-exact
    /// label, per [`link_ref_label_unusable`]'s all-`Text` gate).
    label: Option<String>,
    /// Index of the originating inline in the chain run, and whether this unit
    /// is a reference link's label slot — used to detect when the scan merely
    /// reproduces the arena's original pairing.
    node: usize,
    is_label_slot: bool,
}

/// Re-pair each maximal run of *adjacent* shortcut/reference links against the
/// whole-field refmap `keys`, mirroring cmark's `handle_close_bracket`: a closer
/// consumes a following `[label]` only when the label is **defined** — on lookup
/// failure the label is rewound, the `]` goes literal with no shortcut fallback,
/// and the label's own bracket re-opens to pair with what follows. The arena is
/// refmap-blind and pairs eagerly left (`[foo][bar][baz]` → `[foo][bar]` +
/// `[baz]`), which is right only when `bar` is defined (cm-572); with `bar`
/// undefined cmark instead gives literal `[foo]` + `[bar][baz]` (cm-571/573).
///
/// Adjacency = consecutive inline positions (any intervening text, even a soft
/// break, breaks a run). A collapsed reference (`[text][]`, empty dest) binds
/// positionally, not by refmap, so it breaks a run too. Recurses into emphasis
/// children; a chain inside a list item is backlog (stage 1 cannot descend into
/// an `MdListResolved` this early). Returns `None` when nothing changed.
pub(super) fn repair_ref_link_chains(
    body: &[Inline],
    keys: &std::collections::HashSet<String>,
) -> Option<Vec<Inline>> {
    let is_chain_node = |inl: &Inline| {
        matches!(inl, Inline::MdShortcutLink { .. })
            || matches!(inl, Inline::MdRefLink { dest, .. } if !dest.is_empty())
    };
    let mut out = Vec::with_capacity(body.len());
    let mut changed = false;
    let mut i = 0;
    while i < body.len() {
        if is_chain_node(&body[i]) {
            let mut j = i + 1;
            while j < body.len() && is_chain_node(&body[j]) {
                j += 1;
            }
            match (j - i >= 2)
                .then(|| repair_chain_run(&body[i..j], keys))
                .flatten()
            {
                Some(rewritten) => {
                    out.extend(rewritten);
                    changed = true;
                }
                None => out.extend(body[i..j].iter().cloned()),
            }
            i = j;
            continue;
        }
        if let Inline::MdEmphasis { strong, children } = &body[i]
            && let Some(rewritten) = repair_ref_link_chains(children, keys)
        {
            out.push(Inline::MdEmphasis {
                strong: *strong,
                children: rewritten,
            });
            changed = true;
        } else {
            out.push(body[i].clone());
        }
        i += 1;
    }
    changed.then_some(out)
}

/// cmark's sequential bracket scan over one adjacent chain run (every inline a
/// shortcut or non-collapsed reference link): at each unit, a following defined
/// label forms a reference link (consuming both units); a following *undefined*
/// label leaves this unit's brackets literal (found-label failure has no
/// shortcut fallback) and frees the label to re-pair; the last unit is a
/// shortcut attempt. A unit that reproduces its original inline is cloned
/// unchanged, so an all-defined run (or one whose failures the ordinary
/// demotion stage would rewrite identically) reports no change. An undefined
/// *final* shortcut node is also left for the demotion stage (identical
/// output, single choke point).
fn repair_chain_run(
    run: &[Inline],
    keys: &std::collections::HashSet<String>,
) -> Option<Vec<Inline>> {
    let mut units = Vec::new();
    for (n, inl) in run.iter().enumerate() {
        match inl {
            Inline::MdShortcutLink { display } => units.push(ChainUnit {
                label: display_chain_label(display),
                display: display.clone(),
                node: n,
                is_label_slot: false,
            }),
            Inline::MdRefLink { dest, display } if !dest.is_empty() => {
                units.push(ChainUnit {
                    label: display_chain_label(display),
                    display: display.clone(),
                    node: n,
                    is_label_slot: false,
                });
                units.push(ChainUnit {
                    display: vec![Inline::Text(dest.clone())],
                    label: linkref_label_is_usable(dest).then(|| dest.clone()),
                    node: n,
                    is_label_slot: true,
                });
            }
            _ => return None,
        }
    }
    let defined = |u: &ChainUnit| {
        u.label
            .as_ref()
            .is_some_and(|l| keys.contains(&normalize_linkref_label(l)))
    };
    let mut out = Vec::new();
    let mut changed = false;
    let mut i = 0;
    while i < units.len() {
        if i + 1 < units.len() {
            if defined(&units[i + 1]) {
                let aligned = units[i + 1].is_label_slot
                    && !units[i].is_label_slot
                    && units[i].node == units[i + 1].node;
                if aligned {
                    out.push(run[units[i].node].clone());
                } else {
                    out.push(Inline::MdRefLink {
                        dest: units[i + 1].label.clone().unwrap_or_default(),
                        display: units[i].display.clone(),
                    });
                    changed = true;
                }
                i += 2;
            } else {
                out.push(Inline::Text(format!(
                    "[{}]",
                    link_label_text(&units[i].display)
                )));
                changed = true;
                i += 1;
            }
        } else {
            // Last unit: a shortcut attempt. An original shortcut node passes
            // through untouched either way (a defined one already is a shortcut
            // link; an undefined one is the demotion stage's job) — only a
            // label-slot leftover materializes here.
            if units[i].is_label_slot {
                if defined(&units[i]) {
                    out.push(Inline::MdShortcutLink {
                        display: units[i].display.clone(),
                    });
                } else {
                    out.push(Inline::Text(format!(
                        "[{}]",
                        link_label_text(&units[i].display)
                    )));
                }
                changed = true;
            } else {
                out.push(run[units[i].node].clone());
            }
            i += 1;
        }
    }
    changed.then_some(out)
}

/// The refmap-lookup label a chain unit presents when its *display* serves as a
/// shortcut/collapsed label: the display flatten (mirrors [`link_ref_label`]),
/// or `None` when the flatten is source-exact yet unusable (mirrors
/// [`link_ref_label_unusable`]'s all-`Text` gate — a rich display's flatten is
/// not source-exact, so it stays eligible for the plain refmap lookup).
fn display_chain_label(display: &[Inline]) -> Option<String> {
    let flat = link_label_text(display);
    let source_exact = display.iter().all(|d| matches!(d, Inline::Text(_)));
    (!source_exact || linkref_label_is_usable(&flat)).then_some(flat)
}

/// Demote shortcut/reference links whose label is absent from the field's
/// link-reference map (`keys`) to their literal bracket source — roxygen2 leaves
/// such links unresolved (see [`linkref_keys`]). Returns `None` when nothing is
/// demoted (the common case, so the body is reused unchanged). `@md` only.
pub(super) fn demote_undefined_links(
    body: &[Inline],
    keys: &std::collections::HashSet<String>,
) -> Option<Vec<Inline>> {
    let mut changed = false;
    let out: Vec<Inline> = body
        .iter()
        .map(|inl| {
            // An unusable label (trailing backslash, blank) never closes/resolves
            // in roxygen2's pipeline, so the link is literal bracket text no matter
            // what the refmap holds (cm-552, cm-554).
            if let Some(label) = link_ref_label(inl)
                && (!keys.contains(&normalize_linkref_label(&label))
                    || link_ref_label_unusable(inl))
                && let Some(text) = demoted_link_source(inl)
            {
                changed = true;
                return Inline::Text(text);
            }
            // Descend into list items: a referencing link inside a list item is
            // checked against the same whole-field refmap (`keys`), since roxygen2
            // runs the entire tag value through cmark as one document. A list whose
            // items actually changed becomes an `MdListResolved` carrying the
            // rewritten runs; an untouched list keeps its opaque form (byte-identical
            // serialization). Mirrors the descent in `apply_user_linkrefs`.
            match demote_undefined_in_list(inl, keys) {
                Some(resolved) => {
                    changed = true;
                    resolved
                }
                None => inl.clone(),
            }
        })
        .collect();
    changed.then_some(out)
}

/// If `inl` is a markdown list (opaque `MdList` or already-`MdListResolved`),
/// run undefined-label demotion over each item against the whole-field refmap
/// `keys`, returning a rewritten `Inline::MdListResolved` when any item changed
/// (else `None`). Not a list, or no change ⇒ `None`. Helper for
/// [`demote_undefined_links`].
fn demote_undefined_in_list(
    inl: &Inline,
    keys: &std::collections::HashSet<String>,
) -> Option<Inline> {
    let (ordered, items): (bool, Vec<Vec<Inline>>) = match inl {
        Inline::MdList(node) => (
            md_list_is_ordered(node),
            node.children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                .map(|item| md_list_item_inlines(&item))
                .collect(),
        ),
        Inline::MdListResolved { ordered, items } => (*ordered, items.clone()),
        _ => return None,
    };
    let mut new_items = Vec::with_capacity(items.len());
    let mut item_changed = false;
    for item in &items {
        match demote_undefined_links(item, keys) {
            Some(rewritten) => {
                new_items.push(rewritten);
                item_changed = true;
            }
            None => new_items.push(item.clone()),
        }
    }
    item_changed.then_some(Inline::MdListResolved {
        ordered,
        items: new_items,
    })
}

/// Collect user-written CommonMark link-reference definitions (`[ref]: url`) across
/// the **whole field**, recursing into list items, into a global (normalized-label →
/// destination) map. roxygen2 runs the entire tag value through cmark as one
/// document, so a definition in any paragraph or list item resolves a referencing
/// link anywhere else in the field. First definition of a label wins (cmark),
/// approximated by `or_insert` over a top-level-then-descend walk (a duplicate label
/// split across a list and prose — vanishingly rare — is the only case where strict
/// document order would differ; backlog).
pub(super) fn collect_user_linkrefs_tree(
    body: &[Inline],
    urls: &mut std::collections::HashMap<String, UserLinkDef>,
) {
    let (level, _dropped) = collect_user_linkrefs(body);
    for (label, def) in level {
        urls.entry(label).or_insert(def);
    }
    for inl in body {
        match inl {
            Inline::MdList(node) => {
                for item in node
                    .children()
                    .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                {
                    collect_user_linkrefs_tree(&md_list_item_inlines(&item), urls);
                }
            }
            Inline::MdListResolved { items, .. } => {
                for item in items {
                    collect_user_linkrefs_tree(item, urls);
                }
            }
            // A definition inside a block quote is document-global — cmark
            // parses it in the quote's own block context and the refmap spans
            // the document (cm-220). Read the quote the way the flatten does:
            // re-parse the one-level-stripped body as a synthesized `@md`
            // fragment and collect from its section groups (the recursion
            // covers in-quote lists and nested quotes). A withheld reparse
            // (a non-`@md` tag section) collects nothing — the legacy
            // per-line flatten treats that body as literal prose.
            Inline::MdBlockQuote(node) => {
                if let Some(block) = quote_synthesized_block(&quote_stripped_lines(node)) {
                    for section in block.sections() {
                        for group in section_body_parts(&section) {
                            collect_user_linkrefs_tree(&group, urls);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Apply the field's global user link-reference map (`urls`, from
/// [`collect_user_linkrefs_tree`]) to a body, recursing into list items: consume
/// definition runs (they render nothing) and rewrite each referencing shortcut or
/// reference link whose label is defined to an [`Inline::MdInlineLink`] — so an
/// `[*foo*][r1]` with a `[r1]: url` definition renders `\href{url}{\emph{foo}}`
/// (display **kept**, unlike the R-topic `\link` path that drops a non-plain
/// display). The user definition wins over roxygen2's synthesized `[r1]: R:r1`
/// because cmark keeps the first definition and the synthesized block is appended
/// last.
///
/// Returns `None` when nothing in this subtree changed, so a list with no
/// link-reference work keeps its opaque [`Inline::MdList`] form (and thus its
/// byte-identical serialization); a list that *did* change becomes an
/// [`Inline::MdListResolved`] carrying its rewritten items.
///
/// Scope (this slice): single-`Text`-node definitions at a true block start (a
/// definition cannot interrupt a paragraph), bare or `<…>` destinations, an optional
/// same-line title — now resolved across paragraphs and list items of the same
/// field. Multi-line definitions, titles spanning lines, and URL normalization
/// (percent-encoding, entities) are backlog.
///
/// `consume_defs` is true for a body that can hold block-level definitions (a
/// field body, a list item); the emphasis recursion passes false, since emphasis
/// children are mid-paragraph inline content where a def-shaped `[ref]: url` is
/// ordinary prose, never a definition.
pub(super) fn apply_user_linkrefs(
    body: &[Inline],
    urls: &std::collections::HashMap<String, UserLinkDef>,
    consume_defs: bool,
) -> Option<Vec<Inline>> {
    let dropped = if consume_defs {
        collect_user_linkrefs(body).1
    } else {
        std::collections::BTreeMap::new()
    };
    let mut out = Vec::with_capacity(body.len());
    let mut changed = !dropped.is_empty();
    for (i, inl) in body.iter().enumerate() {
        match dropped.get(&i) {
            // Wholly part of a consumed definition run.
            Some(None) => continue,
            // A `Text` whose leading lines a definition consumed: the remainder
            // stays prose (cm-212's `"title" ok`).
            Some(Some(leftover)) => {
                out.push(Inline::Text(leftover.clone()));
                continue;
            }
            None => {}
        }
        if let Some(label) = link_ref_label(inl)
            && let Some(def) = urls.get(&normalize_linkref_label(&label))
            && let Some(display) = link_display_inlines(inl)
        {
            out.push(Inline::MdInlineLink {
                url: def.url.clone(),
                display,
            });
            changed = true;
            continue;
        }
        // A reference/shortcut image whose label is user-defined resolves to that
        // destination, not the synthesized `R:label`. Rewrite it to the inline form
        // `![alt](url)` so the ordinary image path ([`resolve_md_image`]) renders it —
        // reusing its destination split and image-format wrapping (`svg` → `\if{html}`,
        // …). The user definition wins over roxygen2's appended `[label]: R:label` for
        // the same reason as links (cmark keeps the first definition).
        if let Inline::MdImage(raw) = inl
            && let Some((label, alt)) = image_user_ref(raw)
            && let Some(def) = urls.get(&normalize_linkref_label(&md_label_flatten(label)))
        {
            out.push(Inline::MdImage(rebuilt_inline_image(alt, def)));
            changed = true;
            continue;
        }
        // A reference link nested inside emphasis resolves against the same user
        // definitions (`[foo *bar [baz][ref]*][ref]` — the inner `[baz][ref]`
        // rewrites to `\href` exactly like a top-level one, cm-535). Defs are
        // never consumed here (see above).
        if let Inline::MdEmphasis { strong, children } = inl
            && let Some(rewritten) = apply_user_linkrefs(children, urls, false)
        {
            out.push(Inline::MdEmphasis {
                strong: *strong,
                children: rewritten,
            });
            changed = true;
            continue;
        }
        if let Inline::MdList(node) = inl {
            let items: Vec<Vec<Inline>> = node
                .children()
                .filter(|n| n.kind() == SyntaxKind::ROXYGEN_MD_LIST_ITEM)
                .map(|item| md_list_item_inlines(&item))
                .collect();
            let mut new_items = Vec::with_capacity(items.len());
            let mut item_changed = false;
            for item in &items {
                match apply_user_linkrefs(item, urls, true) {
                    Some(rewritten) => {
                        new_items.push(rewritten);
                        item_changed = true;
                    }
                    None => new_items.push(item.clone()),
                }
            }
            if item_changed {
                out.push(Inline::MdListResolved {
                    ordered: md_list_is_ordered(node),
                    items: new_items,
                });
                changed = true;
                continue;
            }
        }
        out.push(inl.clone());
    }
    changed.then_some(out)
}

/// A user link-reference definition's payload: the destination, and the optional
/// title (empty when absent). A *link* resolving through the definition uses only
/// the URL — roxygen2's `mdxml_link` ignores the title — but an *image* keeps it:
/// `mdxml_image` renders the title as `\figure`'s second argument
/// (`![foo][]` + `[foo]: /url "title"` → `\figure{/url}{title}`, cm-586).
#[derive(Clone)]
pub(super) struct UserLinkDef {
    pub(super) url: String,
    pub(super) title: String,
}

/// A user link-reference definition map, keyed by normalized label
/// ([`normalize_linkref_label`]). The *field-wide* map (defs from every piece of
/// a heading-split field, in document order — cmark keeps the first definition
/// of a label) seeds each piece's [`resolve_linkrefs`], so a reference can cross
/// a heading: roxygen2 markdown-processes the whole field as one document and
/// only then splits it into sections.
pub(super) type LinkDefs = std::collections::HashMap<String, UserLinkDef>;

/// Rebuild a reference/shortcut image whose label resolved to a user definition as
/// an *inline* image (`![alt](<url> "title")`), so the ordinary image path
/// ([`resolve_md_image`]) renders it — reusing its destination split and
/// image-format wrapping. The destination is angle-bracketed so a URL with
/// whitespace (an angle-form definition) round-trips; a URL containing `>` cannot
/// have come from an angle form (which closes at the first `>`) and so has no
/// whitespace — it takes the bare form. The title round-trips through
/// [`strip_title_delims`]'s outer-pair strip, so interior quotes survive.
fn rebuilt_inline_image(alt: &str, def: &UserLinkDef) -> String {
    let dest = if def.url.contains('>') {
        def.url.clone()
    } else {
        format!("<{}>", def.url)
    };
    if def.title.is_empty() {
        format!("!{alt}({dest})")
    } else {
        format!("!{alt}({dest} \"{}\")", def.title)
    }
}

/// Scan a field body for user link-reference definitions, returning the
/// (normalized-label → destination) map and the per-index consumption actions: a
/// body index mapped to `None` is wholly part of a definition (dropped from the
/// rendered output); one mapped to `Some(leftover)` is a `Text` whose leading
/// lines a definition consumed, leaving `leftover` as prose (a definition always
/// consumes whole source lines, so a partially-consumed inline is always a `Text`
/// cut at a line boundary). A definition run is consumed only at a *block start*
/// (the body start, or right after a paragraph break): a definition cannot
/// interrupt a paragraph (CommonMark). A paragraph break is a `Text` containing a
/// newline at the section level, or — inside a list item, where
/// [`md_list_item_inlines`] maps each source `NEWLINE` to its own
/// [`SOFT_BREAK`]-only `Text` — two of those adjacent (a blank `#'` line,
/// cm-319). Within a run, consecutive definitions are separated by a
/// whitespace-only `Text` (a soft line break), which is also dropped.
pub(super) fn collect_user_linkrefs(
    body: &[Inline],
) -> (
    std::collections::HashMap<String, UserLinkDef>,
    std::collections::BTreeMap<usize, Option<String>>,
) {
    let is_soft_break = |inl: &Inline| matches!(inl, Inline::Text(t) if !t.is_empty() && t.chars().all(|c| c == SOFT_BREAK));
    let mut urls: std::collections::HashMap<String, UserLinkDef> = std::collections::HashMap::new();
    let mut dropped = std::collections::BTreeMap::new();
    let mut i = 0;
    let mut block_start = true;
    while i < body.len() {
        if block_start && let Some(end) = scan_linkref_run(body, i, &mut urls, &mut dropped) {
            i = end;
            // The remainder of this block (if any) is prose, not definitions.
            // A trimmed `Text` at `end` still feeds the block-start update below
            // (its leftover carries the original's paragraph break, if any).
            block_start = false;
            continue;
        }
        block_start = match &body[i] {
            Inline::Text(t) if t.contains('\n') => true,
            inl if is_soft_break(inl) => i > 0 && is_soft_break(&body[i - 1]),
            _ => false,
        };
        i += 1;
    }
    (urls, dropped)
}

/// Consume a run of consecutive link-reference definitions beginning at a block
/// start (`start`), recording each into `urls` (first definition of a label wins,
/// per cmark) and its inlines into `dropped`. Tolerates CommonMark's
/// leading-whitespace indentation and the whitespace-only soft breaks that separate
/// stacked definitions (both dropped). Returns the exclusive end index of the run,
/// or `None` when no definition begins the block. Whitespace *after* the last
/// definition is left untouched (it belongs to the following prose). A definition
/// that consumes only the leading lines of a `Text` records the remainder as a
/// `Some(leftover)` trim and ends the run — the leftover is mid-block prose, and a
/// definition cannot interrupt a paragraph.
pub(super) fn scan_linkref_run(
    body: &[Inline],
    start: usize,
    urls: &mut std::collections::HashMap<String, UserLinkDef>,
    dropped: &mut std::collections::BTreeMap<usize, Option<String>>,
) -> Option<usize> {
    let mut end = start;
    let mut any = false;
    loop {
        // Skip whitespace-only (non-paragraph-break) Text — leading indentation or a
        // soft break between definitions.
        let mut k = end;
        while let Some(Inline::Text(t)) = body.get(k) {
            if t.is_empty() || t.contains('\n') || !t.chars().all(char::is_whitespace) {
                break;
            }
            k += 1;
        }
        let Some((label, def, def_end, leftover)) = match_linkref_def(body, k) else {
            break;
        };
        urls.entry(normalize_linkref_label(&label)).or_insert(def);
        for idx in end..def_end {
            dropped.insert(idx, None);
        }
        any = true;
        end = def_end;
        if let Some(left) = leftover {
            dropped.insert(end, Some(left));
            break;
        }
    }
    any.then_some(end)
}

/// Match a single link-reference definition at body index `j`: a shortcut-shaped
/// label link (`[label]`) followed by raw text of the form `: <destination>
/// [title]`. The tail is gathered from the following *raw-recoverable* inlines —
/// `Text` verbatim, plus leaves the inline lexer may have carved out of what is
/// really definition text (a next-line `<my url>` destination reads as raw HTML,
/// a `\bar` backslash-word in a destination as an unknown macro) — and parsed by
/// [`parse_linkref_def_tail`], cmark's block-level definition parse (which runs
/// *before* inline resolution in cmark; the raw regather is what restores that
/// ordering here). Returns `(label, definition, def_end, leftover)`: `def_end` is
/// the body index of the first inline not wholly consumed, and `leftover` is
/// `Some` when that inline is a `Text` whose leading lines the definition
/// consumed (the suffix stays prose — cm-212's `"title" ok` line). `None` when
/// the shape does not hold.
fn match_linkref_def(
    body: &[Inline],
    j: usize,
) -> Option<(String, UserLinkDef, usize, Option<String>)> {
    let inl = body.get(j)?;
    let label = linkref_def_label(inl)?;
    // A label that cannot close (trailing backslash) or is blank defines nothing —
    // the would-be definition line stays literal prose (cm-552, cm-554). Judged
    // source-exact only (see `link_ref_label_unusable`).
    if link_ref_label_unusable(inl) {
        return None;
    }
    // Gather the raw tail with each piece's end offset, so consumed bytes map back
    // to inline indices. The gather stops at the first inline whose source is not
    // recoverable (a resolved link node, emphasis, a code span, …).
    let mut text = String::new();
    let mut piece_ends: Vec<usize> = Vec::new();
    let mut k = j + 1;
    while let Some(frag) = body.get(k).and_then(linkref_raw_fragment) {
        text.push_str(&frag);
        piece_ends.push(text.len());
        k += 1;
    }
    let (def, consumed) = parse_linkref_def_tail(&text)?;
    // Everything gathered was consumed, the definition's last line is not closed
    // by a line boundary, and another inline follows: that inline is trailing
    // content on the definition's line — CommonMark forbids anything but
    // whitespace after the destination (and optional title), so this is not a
    // definition (e.g. `[foo]: url \emph{bar}`). A stacked next definition's
    // label is fine: its line boundary was consumed.
    if consumed == text.len() && body.get(k).is_some() && !text[..consumed].ends_with(SOFT_BREAK) {
        return None;
    }
    // Map the consumed bytes back onto the gathered inlines.
    let mut piece_start = 0;
    for (offset, end) in piece_ends.iter().enumerate() {
        let idx = j + 1 + offset;
        if consumed >= *end {
            piece_start = *end;
            continue;
        }
        if consumed == piece_start {
            return Some((label, def, idx, None));
        }
        // A definition consumes whole source lines, and line boundaries live only
        // in `Text` pieces — so a mid-piece cut is always a `Text`.
        let Some(Inline::Text(t)) = body.get(idx) else {
            return None;
        };
        let leftover = t[consumed - piece_start..].to_string();
        return Some((label, def, idx, Some(leftover)));
    }
    Some((label, def, k, None))
}

/// The verbatim source of an inline a link-reference definition's tail may span,
/// or `None` when the source is not recoverable from the resolved form. cmark
/// parses definitions at the *block* level, before inline resolution — so a
/// definition's destination or title may have been carved by arity's inline pass
/// into a raw-HTML leaf (`<my url>`, cm-197) or an unknown Rd macro (`\bar` in
/// `/url\bar\*baz`, cm-204). Those leaves carry their source verbatim and are
/// re-flattened here; resolved nodes that lost delimiters (emphasis, code spans,
/// links) end the gather instead.
fn linkref_raw_fragment(inl: &Inline) -> Option<Cow<'_, str>> {
    match inl {
        Inline::Text(t) => Some(Cow::Borrowed(t)),
        Inline::MdHtml(raw) => Some(Cow::Borrowed(raw)),
        Inline::Macro(node) => Some(Cow::Owned(node.text().to_string())),
        _ => None,
    }
}

/// The label of a shortcut-shaped link (`[label]`) — the form a link-reference
/// definition's leading token takes (a `[label]` followed by `:` is a shortcut, not
/// a reference or inline link). `None` for any other inline.
fn linkref_def_label(inl: &Inline) -> Option<String> {
    match inl {
        Inline::MdShortcutLink { display } => Some(inline_plain_text(display)),
        Inline::MdLink(raw) => {
            let bytes = raw.as_bytes();
            if bytes.first() != Some(&b'[') {
                return None;
            }
            let end = scan_delimited(bytes, 0, b'[', b']')?;
            (end == bytes.len()).then(|| raw[1..end - 1].to_string())
        }
        _ => None,
    }
}

/// Parse a link-reference definition's tail — the raw text after the label
/// (`: <destination> [title]`) — mirroring cmark's block-level definition parse
/// *after* roxygen2's `double_escape_md` (every source backslash doubles, so no
/// source backslash ever escapes an angle bracket or a paren; a title closer
/// *may* be escaped — the longest-match rule, exactly as `inline_dest_span`).
/// Line boundaries: a [`SOFT_BREAK`] is a soft wrap (the definition may
/// continue), a `\n` is a paragraph break (a hard stop). The destination may sit
/// on the line after the `:` (cm-197), is angle-bracketed (`<…>`, possibly empty
/// — cm-202) or a non-whitespace run with raw-balanced parens; the title may
/// follow on the same line or the next, and may itself span soft wraps (its
/// interior soft breaks flatten to spaces). An invalid or junk-followed title
/// that started on its *own* line falls back to a destination-only definition
/// ending at the destination's line (cm-212); on the destination's line it
/// invalidates the whole definition. Returns `(definition, consumed)` where
/// `consumed` is the byte length of `text` the definition consumed — always a
/// line boundary: just past a `SOFT_BREAK`, at a `\n`, or the text's end.
fn parse_linkref_def_tail(text: &str) -> Option<(UserLinkDef, usize)> {
    const SB: u8 = 0x0C; // SOFT_BREAK as a byte
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b':') {
        return None;
    }
    // Whitespace before the destination: spaces/tabs and at most one soft line
    // boundary (CommonMark allows the destination on the line after the colon; a
    // blank line — our `\n` — means there is no destination).
    let mut i = 1;
    let mut broke = false;
    loop {
        match bytes.get(i) {
            Some(&b' ' | &b'\t') => i += 1,
            Some(&SB) if !broke => {
                broke = true;
                i += 1;
            }
            _ => break,
        }
    }
    let (url, p) = if bytes.get(i) == Some(&b'<') {
        // Angle-bracketed destination: to the first `>` on the same line; no raw
        // `<` inside. May be empty (`<>`, cm-202).
        let mut q = i + 1;
        loop {
            match bytes.get(q) {
                None | Some(&(b'<' | b'\n' | SB)) => return None,
                Some(&b'>') => break,
                Some(_) => q += 1,
            }
        }
        (&text[i + 1..q], q + 1)
    } else {
        // Bare destination: a nonempty run to the first ASCII whitespace, parens
        // balanced by raw count (after doubling, `\(`/`\)` are active parens; an
        // unmatched `)` invalidates the definition).
        let mut q = i;
        let mut depth = 0usize;
        loop {
            match bytes.get(q) {
                None => break,
                Some(&b) if b.is_ascii_whitespace() => break,
                Some(&b'(') => {
                    depth += 1;
                    q += 1;
                }
                Some(&b')') if depth == 0 => return None,
                Some(&b')') => {
                    depth -= 1;
                    q += 1;
                }
                Some(_) => q += 1,
            }
        }
        if depth != 0 || q == i {
            return None;
        }
        (&text[i..q], q)
    };
    // cmark entity-decodes link destinations (`&amp;` → `&`), so a defined href
    // carries the decoded URL. (The destination is otherwise verbatim — no
    // percent re-encoding.)
    let url = decode_html_entities(url);
    // Trailing space on the destination's line; then either the line ends (a
    // definition so far — a title may still follow on the next line) or a title
    // must begin after at least one space.
    let mut q = p;
    while matches!(bytes.get(q), Some(&b' ' | &b'\t')) {
        q += 1;
    }
    let dest_only = |consumed: usize| {
        Some((
            UserLinkDef {
                url: url.clone(),
                title: String::new(),
            },
            consumed,
        ))
    };
    // `Some(next)` = the destination's line is closed and `next` is its
    // consumed-through end (the destination-only fallback); `None` = the title
    // (if any) shares the destination's line, so a failed title fails the whole
    // definition.
    let dest_line_end = match bytes.get(q) {
        None => return dest_only(bytes.len()),
        Some(&b'\n') => return dest_only(q),
        Some(&SB) => Some(q + 1),
        _ if q == p => return None, // content abutting the destination
        _ => None,
    };
    let fallback = || dest_line_end.and_then(dest_only);
    let title_start = match dest_line_end {
        Some(next) => {
            let mut t = next;
            while matches!(bytes.get(t), Some(&b' ' | &b'\t')) {
                t += 1;
            }
            t
        }
        None => q,
    };
    let close = match bytes.get(title_start) {
        Some(&b'"') => b'"',
        Some(&b'\'') => b'\'',
        Some(&b'(') => b')',
        _ => return fallback(),
    };
    let nested_open = bytes[title_start];
    // The title is scanned longest-match (cmark's re2c title pattern): after
    // doubling, a source-backslash-preceded closer is *optionally* escaped, so
    // the title runs to the first closer NOT preceded by a `\` — or, when every
    // closer is `\`-preceded, to the last one. An interior `(` in a `(…)` title
    // is likewise allowed only when `\`-preceded. A paragraph break before the
    // closer fails the title.
    let mut m = title_start + 1;
    let mut escapable_close = None;
    let title_end;
    loop {
        match bytes.get(m) {
            None | Some(&b'\n') => match escapable_close {
                Some(e) => {
                    title_end = e;
                    break;
                }
                None => return fallback(),
            },
            Some(&c) if c == close => {
                if bytes[m - 1] == b'\\' {
                    escapable_close = Some(m);
                    m += 1;
                } else {
                    title_end = m;
                    break;
                }
            }
            Some(&c) if nested_open == b'(' && c == b'(' => {
                if bytes[m - 1] == b'\\' {
                    m += 1;
                } else {
                    return fallback();
                }
            }
            Some(_) => m += 1,
        }
    }
    // Only whitespace may follow the title on its line; junk falls back (title
    // on its own line) or fails the definition (title on the destination's).
    let mut r = title_end + 1;
    while matches!(bytes.get(r), Some(&b' ' | &b'\t')) {
        r += 1;
    }
    let consumed = match bytes.get(r) {
        None => bytes.len(),
        Some(&b'\n') => r,
        Some(&SB) => r + 1,
        _ => return fallback(),
    };
    // The title is cmark-visible text, so entities decode there too; an interior
    // soft wrap flattens to a space (the title spanned lines, cm-197's probes).
    let title = decode_html_entities(&text[title_start + 1..title_end]).replace(SOFT_BREAK, " ");
    Some((UserLinkDef { url, title }, consumed))
}

/// Decode the HTML character references cmark resolves: every semicolon-terminated
/// HTML5 named entity (`&amp;`, `&copy;`, `&hellip;`, …) and numeric references
/// (`&#NN;`, `&#xHH;`). CommonMark requires the trailing `;`, so a bare `&amp` (no
/// semicolon) is left verbatim, as is an unrecognized name (`&nope;`). The fast
/// path returns the input unchanged when it has no `&`, so text without entities is
/// byte-identical. Used both for a markdown link destination and, via
/// [`process_prose`], for `@md` prose text (cmark decodes entities everywhere
/// except code spans/blocks, which the projector keeps as separate verbatim leaves).
pub(super) fn decode_html_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'&'
            && let Some(rel) = s[i + 1..].find(';')
            && decode_entity(&s[i + 1..i + 1 + rel], &mut out)
        {
            i += 1 + rel + 1;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Resolve one character-reference body (the text between `&` and `;`), appending
/// its replacement to `out` and returning `true` on success. A named entity is
/// looked up in the full HTML5 table ([`entities::HTML5_ENTITIES`]); a numeric
/// reference decodes its decimal (`#NN`) or hexadecimal (`#xHH`) code point, mapping
/// U+0000, a surrogate, or an out-of-range value to the replacement character
/// U+FFFD (cmark's rule). Returns `false` for an unrecognized name or a malformed
/// numeric body, leaving the source `&…;` verbatim.
fn decode_entity(body: &str, out: &mut String) -> bool {
    if let Some(num) = body.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => num.parse::<u32>().ok(),
        };
        let Some(code) = code else { return false };
        out.push(
            char::from_u32(code)
                .filter(|&c| c != '\0')
                .unwrap_or('\u{FFFD}'),
        );
        return true;
    }
    match entities::HTML5_ENTITIES.binary_search_by_key(&body, |&(name, _)| name) {
        Ok(idx) => {
            out.push_str(entities::HTML5_ENTITIES[idx].1);
            true
        }
        Err(_) => false,
    }
}

/// Scan double-escaped markdown text for `get_md_linkrefs` shortcut candidates,
/// returning each candidate's reference **label** (`refs[,3]`: the second `[…]`
/// group if present, else the first). Ports roxygen2's `get_md_linkrefs` regex
/// (`markdown-link.R`): a bracket-free `[content]`, optionally followed by a
/// bracket-free `[ref]`, **not** preceded by `]`/`\` and **not** followed by
/// `[`/`{`. Matches are non-overlapping, left to right.
pub(super) fn md_linkref_labels(text: &str) -> Vec<String> {
    md_linkref_scan(text).into_iter().map(|(l, _)| l).collect()
}

/// Scan `text` for `get_md_linkrefs` candidates, returning each candidate's
/// reference **label** (see [`md_linkref_labels`]) paired with the **byte offset of
/// its opening `[`**. The position lets the poisoning boundary
/// ([`first_invalid_linkref_offset`]) map back into the body skeleton.
fn md_linkref_scan(text: &str) -> Vec<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Lookbehind: a `[` not preceded by `]` or `\`.
        if bytes[i] != b'[' || (i > 0 && matches!(bytes[i - 1], b']' | b'\\')) {
            i += 1;
            continue;
        }
        let Some((content, content_end)) = bracket_free_group(bytes, i) else {
            i += 1;
            continue;
        };
        // Optional second `[ref]` (a non-empty bracket-free group right after).
        let (label, match_end) = match bracket_free_group(bytes, content_end) {
            Some((reff, ref_end)) => (reff, ref_end),
            None => (content, content_end),
        };
        // Lookahead: not immediately followed by `[` or `{`.
        if matches!(bytes.get(match_end), Some(b'[' | b'{')) {
            i += 1;
            continue;
        }
        out.push((String::from_utf8_lossy(label).into_owned(), i));
        i = match_end;
    }
    out
}

/// The byte offset (in `skeleton`, the body's reconstructed markdown source) of the
/// opening `[` of the first **invalid** link-reference candidate, or `None` if
/// every candidate is a valid definition. This is where leaked-definition poisoning
/// begins — every shortcut/reference link after it is de-linked (see
/// [`demote_poisoned_links`]). The skeleton carries raw (single) backslashes, so
/// [`linkref_label_is_usable`] is the exact test: a trailing backslash becomes an
/// odd (escaping) run after `double_escape_md` (`2k-1`, fails
/// `linkref_label_closes`), and a blank label can never define — matching the
/// classification the leak itself uses.
pub(super) fn first_invalid_linkref_offset(skeleton: &str) -> Option<usize> {
    md_linkref_scan(skeleton)
        .into_iter()
        .find(|(label, _)| !linkref_label_is_usable(label))
        .map(|(_, start)| start)
}

/// If `bytes[open]` is `[`, return the bracket-free content (`[^\]\[]+`, ≥1 byte,
/// no interior `[`/`]`) and the index just past its closing `]`. `None` when there
/// is no such group (empty content, an interior `[`, or no closing `]`).
pub(super) fn bracket_free_group(bytes: &[u8], open: usize) -> Option<(&[u8], usize)> {
    if bytes.get(open) != Some(&b'[') {
        return None;
    }
    let start = open + 1;
    let mut j = start;
    while j < bytes.len() && !matches!(bytes[j], b'[' | b']') {
        j += 1;
    }
    (bytes.get(j) == Some(&b']') && j > start).then_some((&bytes[start..j], j + 1))
}

/// Whether the synthesized definition `[label]: …` is a valid CommonMark link
/// reference definition (its label's closing `]` is *not* backslash-escaped). The
/// label is bracket-free, so the only failure is a trailing odd run of backslashes
/// escaping the `]`. A valid definition is consumed by cmark (the shortcut becomes
/// a link, handled by arity's own link path); an invalid one leaks.
pub(super) fn linkref_label_closes(label: &str) -> bool {
    label.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 0
}

/// Whether a link-reference label is **blank** — no character that is not a space,
/// tab, or line ending (ASCII whitespace; the [`SOFT_BREAK`] sentinel counts, a
/// NBSP is content). CommonMark requires a label to contain at least one
/// non-whitespace character, so a blank label neither defines a reference nor
/// resolves a link, and its synthesized `[label]: R:…` definition leaks (cm-554).
fn linkref_label_is_blank(label: &str) -> bool {
    label.chars().all(|c| c.is_ascii_whitespace())
}

/// Whether a **raw-source** link-reference label can define or resolve at all in
/// roxygen2's pipeline. Two failures, both leaving the def line literal prose and
/// the shortcut/reference un-linked:
/// - a **trailing backslash run** (any length): `double_escape_md` doubles the run
///   and the `\\]`→`\]` revert makes it odd, so the label's closing `]` is escaped
///   and the label never closes (cm-552);
/// - a **blank** label ([`linkref_label_is_blank`], cm-554).
fn linkref_label_is_usable(label: &str) -> bool {
    !label.ends_with('\\') && !linkref_label_is_blank(label)
}

/// R's `URLencode(x, reserved = FALSE)`: keep ASCII alphanumerics and the
/// unreserved/sub-delim set, percent-encode every other byte as `%XX`
/// (uppercase). Matches roxygen2's `map_chr(refs, URLencode)` for the synthesized
/// `R:label` destinations (e.g. `\`→`%5C`, space→`%20`).
pub(super) fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'!' | b'#'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b'-'
                    | b'.'
                    | b'/'
                    | b':'
                    | b';'
                    | b'='
                    | b'?'
                    | b'@'
                    | b'['
                    | b']'
                    | b'_'
                    | b'~'
            )
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
