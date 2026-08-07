use super::*;

/// Reconstruct the pre-parse Rd string from one projected S-expression atom,
/// appending to `out`. Node atoms are balanced by construction --- a
/// `(\macro c1 c2 …)` renders `\macro{c1}{c2}…` and a `(GRP …)` concatenates its
/// children (the wrapping braces come from its parent), so the only brace
/// imbalance can come from a leaf's decoded text (a trailing `\` escaping the next
/// `}`, exactly roxygen2's `\emph{\}` case). A leaf (`TEXT`/`RCODE`/`VERB`/
/// `UNKNOWN`) contributes its decoded content; under `@md` every `%` is re-escaped
/// to `\%` to mirror roxygen2's markdown render (which escapes `%` in prose, URLs,
/// verbatim, and code alike, so none opens an Rd comment), keeping the count
/// faithful.
pub(super) fn sexpr_to_rd(atom: &str, md: bool, out: &mut String) {
    let bytes = atom.as_bytes();
    let mut i = 0;
    render_sexpr(bytes, &mut i, md, false, out);
}

/// `verbatim` marks a subtree living inside a **fragile** macro (`\verb`, `\code`,
/// `\link`, `\url`, `\preformatted`, …). markdown() keeps such a macro's argument
/// **raw** — an escaped `\{`/`\}` stays backslash-escaped — so it contributes a
/// brace-balanced `{…}` pair to the `rdComplete` scan regardless of its interior.
/// The projected atoms, however, have already resolved `\{` → a bare `{` (see
/// [`resolve_rd_arg_escapes`]), so counting those bare braces would false-drop a
/// section whose fragile argument merely holds an odd escaped brace
/// (`\verb{d \{ e}`). Neutralizing the leaf braces inside a fragile subtree
/// ([`append_leaf_text`] drops them) restores the balanced count markdown()
/// actually presents. Cmark-*derived* braces (`*\**` → `\emph{\}`) are outside
/// any fragile macro, so they still count and drop correctly.
fn render_sexpr(bytes: &[u8], i: &mut usize, md: bool, verbatim: bool, out: &mut String) {
    if bytes.get(*i) != Some(&b'(') {
        return;
    }
    *i += 1; // consume '('
    let head_start = *i;
    while let Some(&c) = bytes.get(*i) {
        if c == b' ' || c == b')' {
            break;
        }
        *i += 1;
    }
    let head = &bytes[head_start..*i];
    let is_leaf = matches!(head, b"TEXT" | b"RCODE" | b"VERB" | b"UNKNOWN");
    // Under `@md`, roxygen2 escapes every `%` to `\%` in the rendered Rd --- in
    // prose, URLs, verbatim, and code alike --- so none opens an Rd comment.
    let escape_percent = md;
    if is_leaf {
        skip_spaces(bytes, i);
        if bytes.get(*i) == Some(&b'"') {
            let text = read_quoted(bytes, i);
            append_leaf_text(&text, escape_percent, verbatim, out);
        }
        // consume the closing ')'
        while let Some(&c) = bytes.get(*i) {
            *i += 1;
            if c == b')' {
                break;
            }
        }
        return;
    }
    let is_grp = head == b"GRP";
    // A generated `\Sexpr` (an `` `Rd expr` `` span) carries ONE argument — the
    // span text verbatim — but its atoms are parse_Rd's *artifact* split (a
    // lone-backslash `RCODE` leaf before each in-code macro). Re-joining the
    // children inside a single brace pair reconstructs the true rendered body,
    // so a genuinely unbalanced span (`` `Rd foo{` ``) still fails the scan
    // while the artifact split alone does not. A `LIST` is likewise ONE bare
    // `{…}` group whose children are parse_Rd's split of a single run (a
    // demoted macro's argument, a grouped brace pair) — wrapping each child in
    // its own pair would let a child's trailing `\` escape a brace that does
    // not exist in the rendered text.
    let single_group = head == b"\\Sexpr" || head == b"LIST";
    // A fragile macro keeps its argument raw in markdown() output, so its whole
    // subtree stays brace-balanced regardless of the resolved leaf braces beneath.
    let child_verbatim = verbatim
        || (!is_grp
            && !single_group
            && is_fragile_for_md(
                std::str::from_utf8(head)
                    .unwrap_or("")
                    .trim_start_matches('\\'),
            ));
    if !is_grp {
        // A macro head: `\name`. Its leading backslash escapes the first name
        // letter for `rd_complete`, which is harmless (a letter, never a brace).
        out.push_str(std::str::from_utf8(head).unwrap_or(""));
    }
    if single_group {
        out.push('{');
    }
    loop {
        skip_spaces(bytes, i);
        match bytes.get(*i) {
            None => break,
            Some(&b')') => {
                *i += 1;
                break;
            }
            Some(_) => {
                if is_grp || single_group {
                    render_sexpr(bytes, i, md, child_verbatim, out);
                } else {
                    out.push('{');
                    render_sexpr(bytes, i, md, child_verbatim, out);
                    out.push('}');
                }
            }
        }
    }
    if single_group {
        out.push('}');
    }
}

fn skip_spaces(bytes: &[u8], i: &mut usize) {
    while bytes.get(*i) == Some(&b' ') {
        *i += 1;
    }
}

/// Read and decode a `"…"` quoted leaf string at `bytes[*i]` (which must be the
/// opening quote), inverting [`encode_text`] (`\\`→`\`, `\"`→`"`, `\n`→newline).
/// Leaves `*i` just past the closing quote.
pub(super) fn read_quoted(bytes: &[u8], i: &mut usize) -> String {
    *i += 1; // consume opening quote
    let mut out = String::new();
    while let Some(&c) = bytes.get(*i) {
        if c == b'\\' {
            *i += 1;
            match bytes.get(*i) {
                Some(b'n') => out.push('\n'),
                Some(&other) => out.push(other as char),
                None => out.push('\\'),
            }
            *i += 1;
        } else if c == b'"' {
            *i += 1; // consume closing quote
            break;
        } else {
            // Copy a full UTF-8 char so multibyte content survives.
            let start = *i;
            *i += 1;
            while bytes.get(*i).is_some_and(|b| b & 0xC0 == 0x80) {
                *i += 1;
            }
            out.push_str(std::str::from_utf8(&bytes[start..*i]).unwrap_or(""));
        }
    }
    out
}

/// Append a leaf's decoded text to the reconstructed Rd, re-escaping `%`→`\%` when
/// `escape_percent` (any leaf under `@md`, where roxygen2 escapes `%` so it never
/// opens an Rd comment). Other special chars (`{`/`}`/`\`) pass through verbatim —
/// they are exactly what `rd_complete` must weigh against the structural braces.
pub(super) fn append_leaf_text(text: &str, escape_percent: bool, verbatim: bool, out: &mut String) {
    if verbatim {
        // A fragile macro's argument is raw and brace-balanced in markdown()
        // output, so it contributes exactly one balanced `{…}` pair (the enclosing
        // delimiters) to the `rd_complete` scan regardless of its interior. The
        // projected atom, however, may carry either **resolved** braces (a literal
        // `\verb{d \{ e}` -> bare `{`, via [`resolve_rd_arg_escapes`]) or
        // **escaped** ones (a markdown code span -> `\{`, kept verbatim), so neither
        // re-escaping nor pass-through is uniformly right. Drop every
        // `rd_complete`-significant char (`{`, `}`, `\`, `%`) so the interior is
        // inert and only the enclosing pair counts.
        for c in text.chars() {
            if !matches!(c, '{' | '}' | '\\' | '%') {
                out.push(c);
            }
        }
    } else if escape_percent {
        // Under `@md`, roxygen2 escapes every `%` to `\%` in the rendered Rd, so
        // none opens an Rd comment against the brace count.
        for c in text.chars() {
            if c == '%' {
                out.push('\\');
            }
            out.push(c);
        }
    } else {
        out.push_str(text);
    }
}
