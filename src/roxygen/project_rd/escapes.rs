use super::*;

/// Apply roxygen2's prose-text pipeline to a raw source run *without* normalizing
/// whitespace — the caller normalizes once over the fully coalesced atom (see
/// [`flush_run`]), so a `Final` block-quote segment can glue seamlessly onto the
/// processed prose on either side. With markdown off the run is literal Rd, where
/// an unescaped `%` begins a comment to end of line (parse_Rd's rule), so the
/// comment is stripped per physical line; with markdown on roxygen2 escapes `%`
/// (`\%`) so it survives and the markdown escapes (backslash runs, `[`/`]`, HTML
/// entities) resolve instead. (Source line breaks stay as `\n`, which the caller's
/// `norm_ws` later collapses, so a comment's end-of-line is honored either way.)
pub(super) fn process_prose(run: &str, md: bool) -> String {
    if md {
        // cmark decodes HTML entities (`&amp;`, `&copy;`, `&#65;`) as the final
        // text transform: they are inert with respect to the `%`-swallow, bracket,
        // and backslash rules (an entity-produced `[`/`%`/`\` is literal text, not a
        // delimiter), so decode after those run on the raw source.
        decode_html_entities(&unescape_md_brackets(&collapse_md_backslash_runs(
            &md_percent_swallow(run),
        )))
    } else {
        resolve_rd_text_escapes(&strip_rd_comments(run))
    }
}

/// Resolve parse_Rd's literal-text escapes in non-`@md` prose. Backslashes pair
/// left-to-right (`\\` → one literal `\`); an unpaired trailing backslash
/// before one of the Rd escape characters `%`, `{`, `}` is consumed with the
/// escape resolved (`\%` → `%`, `\{` → `{`); an unpaired backslash before a
/// brace-required known macro name not followed by `{` re-forms the macro whose
/// missing argument triggers parse_Rd's drop-recovery — the `\name` vanishes
/// and the text continues (`\emph z` → ` z`; see
/// [`is_rd_braceless_drop_macro`], which excludes the sticky code/verbatim-mode
/// names left literal as backlog); an unpaired backslash before anything else
/// stays literal (`a \ b` keeps its backslash). Runs before `%` interact with
/// the line comment: [`strip_rd_comments`] runs first with the same pairing
/// (its `escaped` flip-flop), so a `%` that survives it is always
/// escape-consumed here.
pub(super) fn resolve_rd_text_escapes(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&run[start..i]);
            continue;
        }
        let mut k = 0usize;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
            k += 1;
        }
        for _ in 0..k / 2 {
            out.push('\\');
        }
        if k % 2 == 1 {
            match bytes.get(i) {
                Some(b'%' | b'{' | b'}') => {
                    out.push(bytes[i] as char);
                    i += 1;
                }
                _ => {
                    if let Some(end) = braceless_drop_name_end(run, i) {
                        i = end;
                    } else {
                        out.push('\\');
                    }
                }
            }
        }
    }
    out
}

/// Resolve parse_Rd's Rd-string escapes inside a **literal Rd macro's braced
/// argument** (`\code{…}`, `\verb{…}`, `\emph{…}`, `\link{…}`, `\url{…}`, …).
/// parse_Rd lexes every braced argument — verbatim `RCODE`/`VERB` or prose `TEXT`
/// alike — with the same escape rules: backslashes pair left-to-right (`\\` → one
/// literal `\`), and an unpaired trailing backslash before one of the Rd
/// metacharacters `{`, `}`, `%` is consumed with the character rendered bare
/// (`\{` → `{`, `\}` → `}`, `\%` → `%`); any other unpaired backslash stays literal.
///
/// Unlike [`resolve_rd_text_escapes`] this does **no** brace-less-macro drop
/// recovery (an in-argument `\word` is a real nested macro, already carved into a
/// child node, so the run between children is pure argument text) and **no**
/// `%`-comment stripping (`%` inside a braced argument is literal, never a comment).
///
/// Mode-independent: parse_Rd resolves these escapes in a fragile macro's argument
/// under `@md` exactly as it does with markdown off (engine-probed). A markdown
/// code span or fence keeps its `\{` because it projects through a *different* path
/// ([`md_code_atom`]/[`serialize_md_code_block`]/[`verb_atoms`]), never this one.
pub(super) fn resolve_rd_arg_escapes(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&run[start..i]);
            continue;
        }
        let mut k = 0usize;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
            k += 1;
        }
        for _ in 0..k / 2 {
            out.push('\\');
        }
        if k % 2 == 1 {
            match bytes.get(i) {
                Some(b'%' | b'{' | b'}') => {
                    out.push(bytes[i] as char);
                    i += 1;
                }
                _ => out.push('\\'),
            }
        }
    }
    out
}

/// The end index of a brace-required known macro name at `run[start..]` whose
/// brace-less misuse parse_Rd drop-recovers, or `None` when the bytes there are
/// not such a name (or a `{` follows, making it a real macro call — those are
/// carved by the lexer and never reach prose text, but the guard keeps the
/// transform faithful on any input). Shared by the non-`@md` escape resolution
/// and the `@md` backslash-run collapse: the preceding unpaired `\` plus this
/// name vanish from the rendered text.
fn braceless_drop_name_end(run: &str, start: usize) -> Option<usize> {
    let bytes = run.as_bytes();
    let end = crate::parser::roxygen::rd_macro_name_end(bytes, start);
    (end > start && is_rd_braceless_drop_macro(&run[start..end]) && bytes.get(end) != Some(&b'{'))
        .then_some(end)
}

/// Split each unescaped, brace-less `\item` in a prose inline run into its own
/// [`Inline::BracelessItem`] node, mirroring parse_Rd's out-of-list recovery
/// (`a \item b` → `(TEXT "a") (UNKNOWN "\item") (TEXT "b")`). Recurses into bare
/// brace groups (`{a \item b}` → `(LIST (TEXT "a") (UNKNOWN "\item") (TEXT "b"))`)
/// and emphasis spans so a nested `\item` splits too. Returns `None` when the run
/// contains no brace-less `\item` (the caller keeps the original body). Runs after
/// [`group_brace_lists`], so a following `{…}` has already become an
/// `Inline::BraceGroup` (`\item{x}` → `(UNKNOWN "\item") (LIST (TEXT "x"))`,
/// matching parse_Rd, which never binds the group to the unknown macro).
pub(super) fn split_braceless_items(body: &[Inline]) -> Option<Vec<Inline>> {
    let mut changed = false;
    let mut out: Vec<Inline> = Vec::with_capacity(body.len());
    for inl in body {
        match inl {
            Inline::Text(s) => match split_item_text(s) {
                Some(pieces) => {
                    changed = true;
                    out.extend(pieces);
                }
                None => out.push(inl.clone()),
            },
            Inline::BraceGroup(children) => match split_braceless_items(children) {
                Some(new) => {
                    changed = true;
                    out.push(Inline::BraceGroup(new));
                }
                None => out.push(inl.clone()),
            },
            Inline::MdEmphasis { strong, children } => match split_braceless_items(children) {
                Some(new) => {
                    changed = true;
                    out.push(Inline::MdEmphasis {
                        strong: *strong,
                        children: new,
                    });
                }
                None => out.push(inl.clone()),
            },
            _ => out.push(inl.clone()),
        }
    }
    changed.then_some(out)
}

/// Partition one prose text string at each unescaped, brace-less `\item` (see
/// [`split_braceless_items`]). The interleaved `Inline::Text` (raw, escapes
/// unresolved for the downstream [`process_prose`]) and [`Inline::BracelessItem`]
/// pieces, or `None` when there is no split. Parity-gated like every `\`-carve: a
/// backslash run of even length keeps the final `\` escaped (`\\item` stays literal
/// `\item` text), so only a run of odd length that abuts the exact name `item`
/// begins the macro. The name must be exactly `item` (a longer `\itemize`/`\itemx`
/// is a different macro — the lexer or the drop-recovery handles those).
fn split_item_text(s: &str) -> Option<Vec<Inline>> {
    let bytes = s.as_bytes();
    let mut out: Vec<Inline> = Vec::new();
    let mut seg_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
        }
        // An even run pairs off entirely (the `\item` is escaped); an odd run leaves
        // the final `\` (at `i - 1`) unescaped, opening a macro at `run[i..]`.
        if (i - run_start) % 2 == 1 {
            let name_end = crate::parser::roxygen::rd_macro_name_end(bytes, i);
            if &s[i..name_end] == "item" {
                let before = &s[seg_start..i - 1];
                if !before.is_empty() {
                    out.push(Inline::Text(before.to_string()));
                }
                out.push(Inline::BracelessItem);
                seg_start = name_end;
                i = name_end;
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    let tail = &s[seg_start..];
    if !tail.is_empty() {
        out.push(Inline::Text(tail.to_string()));
    }
    Some(out)
}

/// Rewrite an explicit prose tag's body when a brace-less sticky code/verbatim Rd
/// macro (`\code`/`\verb`/…, see [`sticky_braceless_code_mode`]) triggers
/// parse_Rd's argument-mode swallow, or `None` when no such trigger applies (the
/// caller keeps the body). The dropped `\name` and everything after it, to section
/// end, becomes an [`Inline::StickyVerbatim`] — one `RCODE`/`VERB` atom per
/// physical source line — while the prose *before* the trigger stays ordinary text
/// (`a \code z here` → `(TEXT "a") (RCODE " z here\n")`).
///
/// **Scope (this slice): an explicit prose tag with a single-paragraph plain-text
/// tail.** The swallow crosses paragraph breaks in roxygen2, but arity's paragraph
/// model collapses blank-line *counts* (many blanks → one part boundary), so a
/// cross-paragraph tail cannot be reconstructed faithfully from the inline run and
/// is withheld. The tail must also be free of macros/lists/emphasis (they still
/// parse inside the swallow, splitting the RCODE — `\code z \emph{x}` →
/// `(RCODE " z ") (\emph …) …`) and of the raw chars `\ { } %` (a bare `{`/`}`
/// breaks the section's field braces, a `%` acts as an Rd line comment, a stray
/// `\` risks a nested carve) — all withheld as backlog. Withholding leaves the
/// `\code` literal (its current projection), never a wrong shape.
///
/// The intro paragraph is deliberately excluded: its swallow crosses roxygen2's
/// generated field braces (`{`/`}` scaffolding leaks into the atoms), which is
/// unmodelable at section granularity. Only [`project_tag_section`] calls this, so
/// the intro (emitted directly in [`project_block`]) never reaches it.
pub(super) fn split_sticky_braceless_swallow(body: &[Inline], md: bool) -> Option<Vec<Inline>> {
    for (idx, inl) in body.iter().enumerate() {
        let Inline::Text(s) = inl else { continue };
        let Some((backslash, name_end, code)) = find_sticky_trigger(s) else {
            continue;
        };
        // Reject a same-line `%` before the trigger: a non-`@md` Rd comment there
        // would swallow the `\name` itself (there is no `SOFT_BREAK` between them in
        // one text piece, so the whole pre-trigger portion is one physical line).
        if s[..backslash].contains('%') {
            return None;
        }
        // The tail = this text from `name_end`, plus every following inline. A
        // single-paragraph plain-text tail is all `Inline::Text` with none of the
        // section-breaking / mode-shifting chars; anything else is withheld.
        let mut content = s[name_end..].to_string();
        for later in &body[idx + 1..] {
            match later {
                Inline::Text(t) => content.push_str(t),
                _ => return None,
            }
        }
        if content.contains(['\n', '\\', '{', '}', '%']) {
            return None;
        }
        let mut out: Vec<Inline> = body[..idx].to_vec();
        let before = &s[..backslash];
        if !before.is_empty() {
            out.push(Inline::Text(before.to_string()));
        }
        out.push(Inline::StickyVerbatim {
            code,
            lines: sticky_swallow_lines(&content, md),
        });
        return Some(out);
    }
    None
}

/// Find the first brace-less sticky code/verbatim macro trigger in one prose text
/// piece: `(backslash index, name end, code)` where `backslash` is the position of
/// the opening `\`, `name_end` is one past the macro name, and `code` is
/// [`sticky_braceless_code_mode`]'s R-code/verbatim flag. Parity-gated like every
/// `\`-carve — only an odd-length backslash run opens a macro (`\\code` is an
/// escaped literal) — and brace-less only (a `{` follows a real macro call, carved
/// by the lexer and never reaching prose text; guarded regardless).
fn find_sticky_trigger(s: &str) -> Option<(usize, usize, bool)> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
        }
        if (i - run_start) % 2 == 1 {
            let name_end = crate::parser::roxygen::rd_macro_name_end(bytes, i);
            if name_end > i
                && bytes.get(name_end) != Some(&b'{')
                && let Some(code) = sticky_braceless_code_mode(&s[i..name_end])
            {
                return Some((i - 1, name_end, code));
            }
        }
    }
    None
}

/// Split a sticky swallow's tail into its per-physical-line contents. Physical
/// lines are the [`SOFT_BREAK`] boundaries of the folded run; the first line (the
/// trigger line's remainder) keeps its leading whitespace verbatim
/// (`(RCODE " z here\n")`) in both modes.
///
/// Continuation-line leading whitespace differs by mode. **Non-`@md`:** roxygen2
/// strips only the `#'` comment prefix and one following space, so a `#'   cont`
/// line surfaces as `"  cont"` (two spaces surviving). **`@md`:** cmark
/// additionally strips a paragraph continuation line's remaining leading
/// whitespace before the swallow captures the rendered text, so `#'   cont`
/// surfaces as `"cont"` (fully flush). Trailing `\n` is added per line by the
/// serializer.
fn sticky_swallow_lines(content: &str, md: bool) -> Vec<String> {
    content
        .split(SOFT_BREAK)
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                line.to_string()
            } else if md {
                line.trim_start_matches(is_posix_space).to_string()
            } else {
                line.strip_prefix(' ').unwrap_or(line).to_string()
            }
        })
        .collect()
}

/// In `@md` prose, roxygen2 honors a CommonMark backslash escape for the square
/// brackets `[`/`]` only: an escaped `\[`/`\]` is literal (never a link delimiter)
/// *and the backslash is consumed* (`\[`→`[`, `\]`→`]`). This is unique to
/// brackets — roxygen2's `double_escape_md` doubles every backslash but then
/// reverts `\\[`→`\[` and `\\]`→`\]`, so only the bracket escape survives cmark;
/// every other punctuation escape (`\*`, `` \` ``, `\%`, …) keeps its backslash
/// because the doubling neutralizes it. The lexer already suppresses the link at an
/// escaped `[` ([`bracket_is_escaped`](crate::parser::roxygen)); this drops the
/// now-redundant backslash so the projected literal text matches roxygen2.
///
/// Only a *single* adjacent backslash is consumed (`\\[`→`\[`); deeper backslash
/// runs follow `double_escape_md`'s non-overlapping `gsub` semantics and are left
/// as backlog (a `\\\[` run is rare in real docs).
pub(super) fn unescape_md_brackets(run: &str) -> String {
    let mut out = String::with_capacity(run.len());
    let mut chars = run.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.peek(), Some('[' | ']')) {
            out.push(chars.next().expect("peeked bracket"));
        } else {
            out.push(c);
        }
    }
    out
}

/// Resolve `parse_Rd`'s escaped-brace rendering in the **projected `@md` TEXT**
/// leaves. The leaf arrives carrying the cmark-stage backslash runs
/// ([`collapse_md_backslash_runs`] leaves a run abutting `{`/`}` verbatim), so a run
/// of `k` backslashes before a brace is exactly what parse_Rd receives from
/// `markdown(text)`: it pairs the backslashes left-to-right into `floor(k/2)` literal
/// backslashes, and for **odd** `k` the trailing unpaired `\` escapes the brace to a
/// **bare** literal (`\{` → `{`, `\\\{` → `\{`, `\\\\\{` → `\\{`). A run before any
/// non-brace character — or a bare brace with no preceding backslash — is untouched.
///
/// This runs **after** the section's `rdComplete` drop decision (see
/// [`resolve_md_text_braces`]), which is why it lives here and not in
/// [`process_prose`]: the drop scan must weigh the *escaped* (pre-resolution) brace
/// (roxygen2 runs `rdComplete(markdown(text))` where `\{` is still escaped, so an
/// unbalanced *escaped* brace does not drop the section — `a \{ b` is kept), whereas
/// the rendered TEXT wants the bare brace. Resolving before the scan would count the
/// bare brace and false-drop.
///
/// An **even** `k` pairs off entirely and leaves the brace *unescaped* — a real Rd
/// brace group parse_Rd models as `(LIST …)`. arity keeps flat `TEXT` there (the
/// bare-brace-group model is separate backlog); this transform still halves the
/// backslashes to `k/2` and copies the brace bare, so the projection stays divergent
/// for that shape without compounding the backslash count.
pub(super) fn resolve_md_brace_runs(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Measure the maximal backslash run.
            let mut k = 0usize;
            while i + k < bytes.len() && bytes[i + k] == b'\\' {
                k += 1;
            }
            if matches!(bytes.get(i + k), Some(b'{' | b'}')) {
                // parse_Rd pairs the run into `floor(k/2)` literal backslashes; an
                // odd trailing `\` escapes the brace bare, an even run leaves it bare
                // too (the `(LIST …)` group backlog). Either way: emit the bare brace.
                for _ in 0..k / 2 {
                    out.push('\\');
                }
                out.push(bytes[i + k] as char);
                i += k + 1;
                continue;
            }
            for _ in 0..k {
                out.push('\\');
            }
            i += k;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'\\' {
            i += 1;
        }
        out.push_str(&run[start..i]);
    }
    out
}

/// Apply [`resolve_md_brace_runs`] to every `(TEXT "…")` leaf in a projected
/// `@md` section string, leaving every other leaf verbatim. The escaped-brace
/// resolution is a **`@md`-mode, prose-TEXT-only** encoding difference: a code
/// span's verbatim `VERB` keeps its `\{` (roxygen2 renders `\verb` content
/// literally), a data-name `RCODE`/`\code` body resolves the brace but is left as
/// backlog here, and non-`@md` TEXT already had its escapes resolved upstream
/// ([`resolve_rd_text_escapes`]). So the transform is gated to `@md` sections by
/// its single caller ([`project_block`]) and scoped to `TEXT` heads here.
///
/// The scan tracks quote state so a literal `(TEXT "` *inside* another leaf's
/// string (e.g. a code span) is copied as data, never mistaken for a leaf opener.
pub(super) fn resolve_md_text_braces(sexpr: &str) -> String {
    let bytes = sexpr.as_bytes();
    let mut out = String::with_capacity(sexpr.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                out.push('(');
                i += 1;
                // Read the head token (up to a space, ')', or '"').
                let head_start = i;
                while i < bytes.len() && !matches!(bytes[i], b' ' | b')' | b'"') {
                    i += 1;
                }
                let head = &sexpr[head_start..i];
                out.push_str(head);
                if head == "TEXT" {
                    // Copy the single separating space, then transform the quoted
                    // content (a `TEXT` leaf is always `(TEXT "…")`).
                    while i < bytes.len() && bytes[i] == b' ' {
                        out.push(' ');
                        i += 1;
                    }
                    if bytes.get(i) == Some(&b'"') {
                        let text = read_quoted(bytes, &mut i);
                        out.push_str(&encode_text(&resolve_md_brace_runs(&text)));
                    }
                }
            }
            b'"' => {
                // A leaf's quoted string in non-`TEXT` position: copy it verbatim,
                // honoring `\"`/`\\` escapes so an interior `(` or `"` is data.
                let start = i;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += if i + 1 < bytes.len() { 2 } else { 1 },
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                out.push_str(&sexpr[start..i]);
            }
            _ => {
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b'(' | b'"') {
                    i += 1;
                }
                out.push_str(&sexpr[start..i]);
            }
        }
    }
    out
}

/// In `@md` prose, model roxygen2's `%`-swallow. `%` is the Rd comment character,
/// so roxygen2's markdown→Rd pass escapes a rendered `%` to `\%`; but when the
/// markdown already places a literal backslash immediately before the `%`, that
/// escaping backslash collides with the literal one and the `%` is left **bare** in
/// the Rd, starting a comment that eats to end of line. Whether the collision
/// happens is keyed on the **parity of the source backslash run** before the `%`
/// (`double_escape_md` doubles the run to `2k`, cmark resolves the `\\` pairs, and
/// the emitted Rd carries the `k` literal backslashes plus the one escaping the
/// `%` — a run of `k + 1`, which parse_Rd leaves a trailing bare `%` iff `k` is
/// odd):
///
/// - `k` **odd** (`\%`, `\\\%`, …): the `%` comments to end of line. The `k`
///   backslashes are kept (later halved to `ceil(k/2)` by
///   [`collapse_md_backslash_runs`]) and everything from the `%` to the physical
///   line's end is dropped.
/// - `k` **even** (bare `%`, `\\%`, `\\\\%`, …): the `%` survives as a literal
///   percent; the run keeps its `ceil(k/2)` backslashes and the `%`.
///
/// The swallow is line-scoped (roxygen2's comment ends at the newline, and a
/// soft-wrapped continuation on the next `#'` line survives), mirroring the
/// non-`@md` [`strip_rd_comments`]. It runs **before** [`collapse_md_backslash_runs`]
/// so the odd/even decision reads the original run length, not its halved form.
pub(super) fn md_percent_swallow(run: &str) -> String {
    physical_lines(run)
        .map(md_percent_swallow_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prefix of `line` up to (not including) the first `%` whose preceding
/// maximal backslash run has **odd** length (the whole line if none); the kept
/// backslashes are retained for [`collapse_md_backslash_runs`] to halve.
fn md_percent_swallow_line(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, _) in line.char_indices().filter(|&(i, _)| bytes[i] == b'%') {
        let mut k = 0usize;
        while i > k && bytes[i - 1 - k] == b'\\' {
            k += 1;
        }
        if k % 2 == 1 {
            return &line[..i];
        }
    }
    line
}

/// Strip each odd-run `\%` comment region from a prose `TEXT` leaf **for the
/// `@md` `rdComplete` drop scan only**. Per physical line, the backslash run, the
/// `%`, and everything to the line's end are dropped.
///
/// roxygen2 decides the drop on `rdComplete(markdown(text))`. In `markdown(text)`
/// an odd source backslash run before a `%` renders an *even* run plus a bare
/// comment `%` (`\%` → `\\%`, `\\\%` → `\\\%`): the even run pairs cleanly and the
/// bare `%` comments to end of line, so the region contributes nothing to the
/// brace balance and never leaves a dangling escape. The **output** serializer
/// models the same swallow via [`md_percent_swallow`], but it keeps `ceil(k/2)`
/// backslashes — parse_Rd's rendered text (`y \% …` → `y \`) — which can leave an
/// *odd* trailing backslash at the section's end. Reconstructing the scan from
/// those output atoms then reads a dangling escape and false-drops a section
/// roxygen2 keeps (`@details y \% {z} end.`). Dropping the whole region here (not
/// just the tail) removes that backslash so the scan matches `markdown(text)`. An
/// **even**-run `%` — a genuine literal percent roxygen2 escapes to `\%` — is left
/// untouched for render-time re-escaping ([`append_leaf_text`]).
pub(super) fn strip_scan_percent_comment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for seg in text.split_inclusive(['\n', SOFT_BREAK]) {
        match seg.chars().next_back() {
            Some(c @ ('\n' | SOFT_BREAK)) => {
                out.push_str(scan_line_before_odd_percent(
                    &seg[..seg.len() - c.len_utf8()],
                ));
                out.push(c);
            }
            _ => out.push_str(scan_line_before_odd_percent(seg)),
        }
    }
    out
}

/// The prefix of `line` up to (not including) the backslash run of the first `%`
/// whose preceding maximal backslash run has **odd** length (the whole line if
/// none). Unlike [`md_percent_swallow_line`], the backslashes are dropped too, so
/// no trailing escape survives into the [`rd_complete`] scan.
fn scan_line_before_odd_percent(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, _) in line.char_indices().filter(|&(i, _)| bytes[i] == b'%') {
        let mut k = 0usize;
        while i > k && bytes[i - 1 - k] == b'\\' {
            k += 1;
        }
        if k % 2 == 1 {
            return &line[..i - k];
        }
    }
    line
}

/// Rewrite a prose body's top-level `TEXT` leaves for the `@md` `rdComplete` drop
/// scan, applying [`strip_scan_percent_comment`]. Returns `None` when no leaf
/// changed (the common case — no odd-run `\%`), so the scan reuses the original
/// body. Only top-level prose is stripped: a `%`-comment inside a balanced macro
/// argument contributes a balanced pair regardless (backlog).
pub(super) fn strip_scan_percent_comments(body: &[Inline]) -> Option<Vec<Inline>> {
    let mut changed = false;
    let new: Vec<Inline> = body
        .iter()
        .map(|inl| match inl {
            Inline::Text(t) => {
                let stripped = strip_scan_percent_comment(t);
                changed |= stripped != *t;
                Inline::Text(stripped)
            }
            other => other.clone(),
        })
        .collect();
    changed.then_some(new)
}

/// In `@md` prose, a run of literal backslashes collapses per CommonMark's
/// backslash escaping. roxygen2's `double_escape_md` doubles every backslash
/// (`k` → `2k`), cmark then resolves each `\\` pair to one literal backslash
/// (`2k` → `k`), and finally `parse_Rd` collapses the rendered `\\` pairs again
/// (`k` → `ceil(k/2)`, the trailing odd backslash escaping the next character).
/// The net effect on the parsed text is that a run of `k` source backslashes
/// renders as `ceil(k/2)` backslashes: a lone `\` (`\*`, `` \` ``, `\_`, …) keeps
/// its single backslash (`ceil(1/2) == 1`, a no-op), while `\\` → `\`,
/// `\\\\` → `\\`, and so on.
///
/// A run immediately before `[`/`]` is left untouched — those bracket escapes
/// follow `double_escape_md`'s revert (`\\[` → `\[`) and are resolved separately
/// by [`unescape_md_brackets`], which runs after this. A run before `{`/`}` is
/// **also** left untouched, at cmark's `k`-backslash stage: parse_Rd's brace
/// resolution is parity-dependent and is deferred to the post-pass
/// ([`resolve_md_brace_runs`]) so the `rdComplete` scan can weigh the still-escaped
/// brace first (halving here would destroy the parity it needs). Runs before `%`
/// (the Rd comment character) are also left to the separate `%`-swallow modeling (a
/// lone `\%` keeps its backslash but the bare `%` still comments to end of line);
/// `ceil(k/2)` is a no-op for the common `k == 1` case there anyway.
///
/// An **odd** run before a brace-required known macro name not followed by `{`
/// is the brace-less misuse: the `k` source backslashes reach parse_Rd intact
/// (double → cmark halves), which pairs them into `k/2` literal backslashes and
/// re-forms the trailing `\name` — whose missing argument drop-recovers, so the
/// name vanishes (`\emph z` → ` z`, `\\\link q` → `\ q`; see
/// [`braceless_drop_name_end`]). Mirrors the non-`@md`
/// [`resolve_rd_text_escapes`].
pub(super) fn collapse_md_backslash_runs(run: &str) -> String {
    let bytes = run.as_bytes();
    let mut out = String::with_capacity(run.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            out.push_str(&run[start..i]);
            continue;
        }
        // Consume a maximal run of backslashes.
        let mut k = 0usize;
        while i < bytes.len() && bytes[i] == b'\\' {
            i += 1;
            k += 1;
        }
        // A run abutting a square bracket is a bracket escape: leave it verbatim
        // for `unescape_md_brackets` (its `\\[` → `\[` revert is a distinct path).
        if matches!(bytes.get(i), Some(b'[' | b']')) {
            for _ in 0..k {
                out.push('\\');
            }
        } else if matches!(bytes.get(i), Some(b'{' | b'}')) {
            // A run abutting a brace is left at cmark's stage: `double_escape_md`
            // doubles the run and cmark halves it back, so the atom carries the
            // same `k` backslashes roxygen2's `rdComplete` scans in `markdown(text)`
            // (a brace's escape does not drop the section — the scan sees `\{`).
            // parse_Rd's brace resolution is parity-dependent (odd `k` escapes the
            // brace bare, even `k` leaves an Rd group), and the general `ceil(k/2)`
            // halving would destroy that parity, so it is deferred to the post-pass
            // ([`resolve_md_text_braces`] → [`resolve_md_brace_runs`]).
            for _ in 0..k {
                out.push('\\');
            }
        } else if k % 2 == 1
            && let Some(end) = braceless_drop_name_end(run, i)
        {
            // Odd run + brace-less drop macro: the unpaired `\` re-forms the
            // macro, whose drop-recovery consumes `\name`.
            for _ in 0..k / 2 {
                out.push('\\');
            }
            i = end;
        } else {
            for _ in 0..k.div_ceil(2) {
                out.push('\\');
            }
        }
    }
    out
}
