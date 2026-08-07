use super::*;

/// Flatten paragraphs into a single inline run, with a space between each (the
/// canonical serializer collapses the paragraph break to one space anyway).
pub(super) fn join_paras(paras: &[Vec<Inline>]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        if i > 0 {
            // A paragraph break is ≥1 line break; norm_ws collapses it to a space,
            // but it bounds an Rd `%` comment in non-markdown prose.
            out.push(Inline::Text("\n".to_string()));
        }
        out.extend(p.iter().cloned());
    }
    out
}

/// Render a structural macro argument from its serialized atoms: a multi-atom
/// argument is `(GRP …)`-wrapped (parse_Rd models it as a list), a single-atom one
/// stays bare, and an empty one yields nothing. Mirrors `serialize_macro`'s
/// `finalize`, used for the `\section` title/body arguments.
pub(super) fn grp_arg(atoms: &[String]) -> String {
    match atoms {
        [] => String::new(),
        [one] => one.clone(),
        many => format!("(GRP {})", many.join(" ")),
    }
}

pub(super) fn prefix_space(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else {
        format!(" {s}")
    }
}

/// A `(TEXT "…")` atom with the body whitespace-normalized (matching the R
/// driver's `norm_ws`), or `None` if the body is blank.
pub(super) fn text_atom(body: &str) -> Option<String> {
    let t = norm_ws(body);
    (!t.is_empty()).then(|| format!("(TEXT {})", encode_text(&t)))
}

/// roxygen2's `double_escape_md` (`markdown-link.R`): double every backslash, then
/// revert the two bracket escapes (`\\[`→`\[`, `\\]`→`\]`) — so only a bracket
/// escape survives cmark, every other punctuation escape is neutralized. The
/// `gsub(fixed = TRUE)` passes are non-overlapping left-to-right, matching
/// `str::replace`.
pub(super) fn double_escape_md(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace("\\\\[", "\\[")
        .replace("\\\\]", "\\]")
}

/// Append already-rendered prose `extra` to a section's atom list, coalescing it
/// into a trailing `(TEXT …)` atom (the way roxygen2's appended link-ref text
/// coalesces with the section's prose under the canonical TEXT-run merge), or
/// pushing a fresh `(TEXT …)` when the last atom is not prose.
pub(super) fn append_rendered_text(atoms: &mut Vec<String>, extra: &str) {
    if let Some(last) = atoms.last_mut()
        && let Some(text) = decode_text_atom(last)
        && let Some(merged) = text_atom(&format!("{text} {extra}"))
    {
        *last = merged;
        return;
    }
    if let Some(atom) = text_atom(extra) {
        atoms.push(atom);
    }
}

/// Reverse [`encode_text`] for a `(TEXT "…")` atom, returning the decoded inner
/// string, or `None` if `atom` is not a text atom.
pub(super) fn decode_text_atom(atom: &str) -> Option<String> {
    let inner = atom.strip_prefix("(TEXT \"")?.strip_suffix("\")")?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => out.push(other), // `\\` → `\`, `\"` → `"`
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Strip Rd `%` line comments from literal-Rd prose: on each physical line, an
/// unescaped `%` (one not preceded by a `\`) begins a comment that runs to the end
/// of that line. Lines are rejoined with `\n` (collapsed downstream by `norm_ws`).
pub(super) fn strip_rd_comments(s: &str) -> String {
    physical_lines(s)
        .map(strip_rd_line_comment)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prefix of `line` before its first unescaped `%` (the whole line if none).
pub(super) fn strip_rd_line_comment(line: &str) -> &str {
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '%' {
            return &line[..i];
        }
    }
    line
}

/// The verbatim `(RCODE …)` atoms for a `\code` body. parse_Rd keeps `\code`
/// content verbatim (no whitespace collapse) but splits it at newlines, attaching
/// each `\n` to the atom it ends (`\code{a\nb}` → `(RCODE "a\n") (RCODE "b")`). An
/// empty body yields no atom.
pub(super) fn rcode_atoms(body: &str) -> Vec<String> {
    let mut atoms = Vec::new();
    let mut rest = body;
    while let Some(idx) = rest.find('\n') {
        let (seg, tail) = rest.split_at(idx + 1);
        atoms.push(format!("(RCODE {})", encode_text(seg)));
        rest = tail;
    }
    if !rest.is_empty() {
        atoms.push(format!("(RCODE {})", encode_text(rest)));
    }
    atoms
}

/// Collapse every whitespace run to a single space and trim (the R `norm_ws`,
/// `gsub("[[:space:]]+", " ")` then `trimws`).
///
/// The R driver's `[[:space:]]` is **ASCII-only** even in a UTF-8 locale, so
/// non-ASCII Unicode whitespace (NBSP `U+00A0`, NEL `U+0085`, the `Zs`
/// separators) is *preserved verbatim*, never collapsed. This matters for
/// flanking-rejected emphasis: `*\u{a0}a\u{a0}*` stays the literal text
/// `*\u{a0}a\u{a0}*` (a NBSP can't flank, so no `\emph`), and the NBSP must
/// survive the projection. Rust's `split_whitespace`/`char::is_whitespace` is
/// Unicode-aware and would wrongly fold NBSP to a plain space, so we classify
/// against the ASCII POSIX-space set instead.
pub(super) fn norm_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if is_posix_space(c) {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
    }
    out
}

/// The C-locale POSIX `[[:space:]]` set: space, tab, newline, vertical tab,
/// form feed, and carriage return. ASCII-only by design (see `norm_ws`).
pub(super) fn is_posix_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// Base R `trimws`'s default whitespace set — literally the regex `[ \t\r\n]`
/// (R's documented default). Narrower than [`is_posix_space`]: no `\v`/`\f`.
pub(super) fn is_trimws_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Trim a rendered field **piece** at its edges the way roxygen2 does: base R's
/// `trimws` over the whole piece string (`mdxml_children_to_rd_top`'s
/// `trimws(rd)`/`trimws(secs)`, R/markdown.R; the non-md `tag_value` runs the
/// same trim on the raw value). `trimws`'s default set is exactly
/// `[ \t\r\n]` ([`is_trimws_space`]) — narrower than both Unicode White_Space
/// and POSIX `[[:space:]]` (no `\f`/`\v`), so an edge NBSP survives the trim
/// (cm-025; roxygen2 8.0.0 replaced stringr's Unicode `str_trim` when it
/// dropped the stringr dependency).
///
/// At the atom level the piece's leading edge is the front of its first `(TEXT …)`
/// atoms and the trailing edge the back of its last; a non-TEXT edge atom renders
/// as a macro (`\…{`), so the string trim — and this one — stops there. An atom
/// the trim empties is dropped whole. A nested `\subsection` body is interior to
/// its piece (brace-wrapped in the rendered string) and is never trimmed.
pub(super) fn trim_field_atoms(atoms: &mut Vec<String>) {
    trim_field_atoms_start(atoms);
    trim_field_atoms_end(atoms);
}

/// The leading half of [`trim_field_atoms`]. `@section` needs the halves
/// separately: the whole `title: content` value trims as one string, so the
/// *title* carries the field's leading edge and the *content* its trailing edge,
/// while the edges at the `:` split are interior and keep their whitespace
/// (engine-probed).
pub(super) fn trim_field_atoms_start(atoms: &mut Vec<String>) {
    while let Some(text) = atoms.first().and_then(|a| decode_text_atom(a)) {
        let trimmed = text.trim_start_matches(is_trimws_space);
        if trimmed.is_empty() {
            atoms.remove(0);
        } else {
            if trimmed.len() != text.len() {
                atoms[0] = format!("(TEXT {})", encode_text(trimmed));
            }
            break;
        }
    }
}

/// The trailing half of [`trim_field_atoms`] (see [`trim_field_atoms_start`]).
pub(super) fn trim_field_atoms_end(atoms: &mut Vec<String>) {
    while let Some(text) = atoms.last().and_then(|a| decode_text_atom(a)) {
        let trimmed = text.trim_end_matches(is_trimws_space);
        if trimmed.is_empty() {
            atoms.pop();
        } else {
            if trimmed.len() != text.len() {
                let last = atoms.len() - 1;
                atoms[last] = format!("(TEXT {})", encode_text(trimmed));
            }
            break;
        }
    }
}

/// A sentinel marking a **physical soft-wrap** line break inside a prose run: the
/// point where one `#'` source line ends and the next continues the *same*
/// roxygen2 paragraph. It is distinct from a paragraph break (`\n`) on purpose.
/// An Rd `%` comment (and the `@md` `%`-swallow) ends at the *physical* source
/// line, so [`strip_rd_comments`]/[`md_percent_swallow`] must stop at a
/// `SOFT_BREAK` just as they stop at a `\n`; but the link-reference block
/// machinery ([`collect_user_linkrefs`]/[`scan_linkref_run`]) treats **only** a
/// `\n` as a paragraph break — a definition may not interrupt a soft-wrapped
/// paragraph. `SOFT_BREAK` is ASCII whitespace (so `norm_ws` collapses it to a
/// single space; a soft-wrapped paragraph with no comment renders identically)
/// but is not `\n` (so it never reads as a paragraph break).
pub(super) const SOFT_BREAK: char = '\u{c}'; // form feed

/// Split a prose run at every **physical** source-line boundary — a paragraph
/// break (`\n`) or a soft-wrap ([`SOFT_BREAK`]). Used by the `%`-comment
/// strippers, whose comments end at the physical line.
pub(super) fn physical_lines(run: &str) -> impl Iterator<Item = &str> {
    run.split(['\n', SOFT_BREAK])
}

/// Escape a string the way the R driver's `encode_text` does (`\`, `"`, `\n`).
pub(super) fn encode_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The content of a markdown code span: drop the matched backtick runs, then
/// apply CommonMark's single-space trim (if the inner text both starts and ends
/// with a space but is not all spaces, one space is removed from each end).
pub(super) fn strip_code_span(text: &str) -> String {
    let ticks = text.bytes().take_while(|&b| b == b'`').count();
    let inner = text
        .get(ticks..text.len() - ticks)
        .unwrap_or("")
        .replace('\n', " ");
    if inner.len() >= 2
        && inner.starts_with(' ')
        && inner.ends_with(' ')
        && !inner.trim().is_empty()
    {
        inner[1..inner.len() - 1].to_string()
    } else {
        inner
    }
}

/// Wrap an atom in `(\code …)` when `is_code`, else return it unchanged.
pub(super) fn code_wrap(inner: String, is_code: bool) -> String {
    if is_code {
        format!("(\\code {inner})")
    } else {
        inner
    }
}

/// If `s` is a single-backtick code span (`` `x` ``), return its inner text and
/// `true`; otherwise return `s` unchanged and `false`.
pub(super) fn unwrap_code_span(s: &str) -> (&str, bool) {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'`' && b[b.len() - 1] == b'`' {
        (&s[1..s.len() - 1], true)
    } else {
        (s, false)
    }
}
