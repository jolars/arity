use super::*;

/// The display inlines of a referencing link — what `\href{url}{…}` renders when the
/// link's label resolves to a user destination. For a shortcut/reference *node* this
/// is the resolved `display`; for an opaque shortcut/reference leaf the bracketed
/// text becomes one `Text` (plain by construction — marked-up displays nodeify).
/// `None` for an inline link or autolink (own destination, not reference-resolved).
pub(super) fn link_display_inlines(inl: &Inline) -> Option<Vec<Inline>> {
    match inl {
        Inline::MdShortcutLink { display } | Inline::MdRefLink { display, .. } => {
            Some(display.clone())
        }
        Inline::MdLink(raw) => {
            let bytes = raw.as_bytes();
            if bytes.first() == Some(&b'<') {
                return None; // autolink
            }
            let text_end = scan_delimited(bytes, 0, b'[', b']')?;
            match bytes.get(text_end) {
                Some(&b'(') => None, // inline link — own destination
                _ => Some(vec![Inline::Text(raw[1..text_end - 1].to_string())]),
            }
        }
        _ => None,
    }
}

/// Resolve a `ROXYGEN_MD_LINK` leaf into its Rd atom, mirroring roxygen2's
/// `parse_link` (`markdown-link.R`). Three forms:
///
/// - **inline** `[text](url)` → `(\href (VERB url) (TEXT text))`;
/// - **reference** `[text][ref]` → `(\link (TEXT text))` — the has-link-text
///   branch (always `\link`, `\code`-wrapped iff the display text is a code span);
/// - **shortcut** `[dest]` → `(\link …)`/`(\linkS4class …)`, `\code`-wrapped when
///   `dest` is a code span or ends in `()`.
///
/// The `\link[…]`/`\linkS4class[…]` *topic option* is dropped by roxygen2's
/// section serializer, so only the macro head, the display text, and the
/// `\code`-wrap survive. Package resolution (`resolve_link_package`) is inherently
/// non-static, so the projector models exactly what roxygen2 does with no
/// resolvable package context (the corpus's `current_package == ""`): a package
/// prefix in the display text comes only from an explicit `pkg::` in the link.
///
/// Returns `None` for an unrecognized shape (the leaf then stays literal prose).
pub(super) fn resolve_md_link(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    // A CommonMark autolink `<…>`. cmark has two disjoint forms:
    //   * a URI autolink `<scheme:…>` whose destination equals its text →
    //     `\url{…}` (roxygen2's `mdxml_link` `dest == xml_text(xml)` branch);
    //   * an email autolink `<addr>` (no URI scheme), for which cmark sets the
    //     destination to `mailto:addr` → `\href{mailto:addr}{addr}` (the address
    //     is both destination and display).
    // The lexer only carves a valid autolink here, so distinguishing the two
    // reduces to whether a URI scheme is present ([`autolink_has_uri_scheme`]).
    if bytes.first() == Some(&b'<') {
        let inner = raw.strip_prefix('<')?.strip_suffix('>')?;
        return Some(if autolink_has_uri_scheme(inner) {
            url_atom(inner)
        } else {
            href_atom(inner, &format!("mailto:{inner}"))
        });
    }
    let text_end = scan_delimited(bytes, 0, b'[', b']')?;
    let text = &raw[1..text_end - 1];
    match bytes.get(text_end) {
        Some(&b'(') => {
            let url_end = scan_delimited(bytes, text_end, b'(', b')')?;
            (url_end == bytes.len()).then(|| {
                inline_link_atom(
                    text,
                    &inline_link_destination(&raw[text_end + 1..url_end - 1]),
                )
            })
        }
        Some(&b'[') => {
            let ref_end = scan_delimited(bytes, text_end, b'[', b']')?;
            (ref_end == bytes.len()).then(|| ref_link_atom(text, &raw[text_end + 1..ref_end - 1]))
        }
        // A bare `[dest]` is the whole leaf (the lexer carves nothing after it).
        None => Some(shortcut_link_atom(text)),
        _ => None,
    }
}

/// The destination of an inline-link closer leaf `](dest)`: the text between the
/// parentheses ([`inline_link_destination`] parses the CommonMark destination out of
/// it, dropping any title).
pub(super) fn inline_link_dest(close: &str) -> String {
    let content = close
        .strip_prefix("](")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or("");
    inline_link_destination(content)
}

/// Parse the destination from an inline link's parenthesized content
/// `( destination [title] )`, mirroring cmark's inline-link parse: optional
/// surrounding whitespace, then an angle-bracketed (`<…>`, brackets stripped) or
/// bare (a run up to the first whitespace) destination, then an **optional title**
/// (`"…"`/`'…'`/`(…)`) that roxygen2 discards. So `[t](url "x")`, `[t](url 'x')`, and
/// `[t](url (x))` all render `\href{url}{t}` (probed), while a `"` not preceded by
/// whitespace stays part of the destination (`[t](url"x")` → `url"x"`). The
/// destination is entity-decoded (`&amp;`→`&`) like a reference definition's
/// ([`parse_linkref_def_dest`]). Cross-line links carry [`SOFT_BREAK`] separators,
/// which count as whitespace here (a CommonMark destination spans no line break).
fn inline_link_destination(content: &str) -> String {
    let rest = content.trim_start();
    let url = if let Some(r) = rest.strip_prefix('<') {
        match r.find('>') {
            Some(close) => &r[..close],
            None => rest, // unterminated angle destination: keep verbatim
        }
    } else {
        // ASCII whitespace only, like cmark: a U+00A0 is destination content, so
        // it never separates a title (`[t](/url\u{a0}"x")` keeps the whole run).
        let end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        &rest[..end]
    };
    decode_html_entities(url)
}

/// Project a `ROXYGEN_MD_LINK` node `[display](url)`: `\href{url}{display}` with
/// the display GRP-wrapped when it is more than one atom (`\href` is a two-argument
/// structural macro), falling back to `\url{text}` when the destination is empty or
/// equals the link text (roxygen2's `mdxml_link`, the auto-generated-destination
/// branch). Mirrors [`inline_link_atom`], but the display renders the link's
/// resolved markdown *children* rather than a flat string.
pub(super) fn inline_link_node_atom(url: &str, display: &[Inline], md: bool) -> String {
    let display_text = inline_plain_text(display);
    if url.is_empty() || norm_ws(url) == norm_ws(&display_text) {
        return url_atom(&display_text);
    }
    let arg = grp_arg(&serialize_inlines(display, md));
    format!("(\\href (VERB {}){})", encode_text(url), prefix_space(&arg))
}

/// Project a `ROXYGEN_MD_LINK` node with a reference closer (`[display][ref]`):
/// `\link{display}` — roxygen2's section serializer drops the `[ref]` topic option,
/// so only the `\link` head and the display survive, `\code`-wrapped when the
/// display is a single code span. When the display text equals the reference label,
/// roxygen2 treats the text as auto-generated and falls back to the shortcut path.
/// Mirrors [`ref_link_atom`], but the display is the link's resolved markdown
/// *children* rather than a flat string.
pub(super) fn ref_link_node_atom(display: &[Inline], dest: &str) -> String {
    let display_text = inline_plain_text(display);
    if norm_ws(&display_text) == norm_ws(dest) {
        return shortcut_link_atom(dest);
    }
    if display_has_macro(display) {
        return link_over_display(display);
    }
    let (inner, is_code) = match display {
        [Inline::MdCode(content)] => (content.clone(), true),
        _ => (display_text, false),
    };
    code_wrap(
        format!("(\\link {})", text_atom(&inner).unwrap_or_default()),
        is_code,
    )
}

/// Whether a resolved link display carries a `ROXYGEN_RD_MACRO` child (a
/// backslash-word or `\name{…}` written in the markdown source). Such a display is
/// rendered as `\link` over the serialized display atoms ([`link_over_display`])
/// rather than collapsed to a flat destination string, so the macro surfaces as a
/// nested Rd subtree the way parse_Rd reads it (`[a\b]` → `(\link (TEXT "a")
/// (UNKNOWN "\\b"))`).
fn display_has_macro(display: &[Inline]) -> bool {
    display.iter().any(|inl| matches!(inl, Inline::Macro(_)))
}

/// Render `\link` over a macro-bearing display: the topic is the serialized display
/// atoms (text runs plus each Rd macro as a nested subtree), mirroring roxygen2's
/// `\link{<markdown display>}` whose body parse_Rd then parses. The `\linkS4class` /
/// `pkg::` / `()` shortcut-destination refinements operate on a flat string and so do
/// not apply to a macro-bearing destination (a vanishingly-rare combination — left as
/// backlog).
fn link_over_display(display: &[Inline]) -> String {
    let body = serialize_inlines(display, true).join(" ");
    format!("(\\link {body})")
}

/// Project a `ROXYGEN_MD_LINK` node with a bare `]` shortcut closer (`[display]`):
/// the display text *is* the destination, so this mirrors [`shortcut_link_atom`] but
/// takes the code-span-ness from the resolved children rather than from backticks in
/// a raw string. A single code-span display re-wraps its content in backticks so the
/// shared resolver detects it (`\code`-wrapped `\link`); any other display passes its
/// coalesced plain text through unchanged.
pub(super) fn shortcut_link_node_atom(display: &[Inline]) -> String {
    match display {
        [Inline::MdCode(content)] => shortcut_link_atom(&format!("`{content}`")),
        _ if display_has_macro(display) => link_over_display(display),
        _ => shortcut_link_atom(&inline_plain_text(display)),
    }
}

/// Whether roxygen2's `parse_link` would *drop* a shortcut/reference link with this
/// resolved display, rendering nothing ("markdown links must contain plain text").
/// `parse_link` first unwraps a display that is a *single* code span (which then
/// links as `\code{\link{…}}`) and otherwise requires every child to be text (a
/// softbreak/linebreak, both projected as `Inline::Text(" ")`, also count): any
/// emphasis, a second code span, an image, an autolink, or raw HTML makes the link
/// non-plain and roxygen2 discards it. (An inline `[text](url)` link is never
/// subject to this — it carries its own destination and renders `\href`.)
///
/// An `Inline::Macro` child counts as **plain text** *unless* its argument carries
/// cmark-active markdown: a bare backslash-word (`\b`) or a `\name{…}` whose body is
/// literal (`\emph{x}`, or any fragile macro like `\code{*x*}`) is literal text to
/// cmark (a backslash escapes only punctuation, macro braces are literal), so the
/// link is kept and parse_Rd reinterprets it as an Rd macro. But a non-fragile
/// macro whose argument *is* markdown-processed and resolves to active markup
/// (`\emph{*x*}`, `\emph{a \strong{*x*}}`, `` \emph{`c`} ``) makes the display
/// non-plain to cmark, so roxygen2 drops the link ([`macro_arg_has_active_markdown`]).
pub(super) fn link_display_is_droppable(display: &[Inline]) -> bool {
    if matches!(display, [Inline::MdCode(_)]) {
        return false;
    }
    !display.iter().all(|inl| match inl {
        Inline::Text(_) => true,
        Inline::Macro(n) => !macro_arg_has_active_markdown(n),
        _ => false,
    })
}

/// Whether a non-fragile Rd macro's markdown-processed argument resolves to any
/// **cmark-active** markup (emphasis, a code span, a link, an image, raw HTML, or a
/// nested non-fragile macro whose own argument is active). Such a macro is *not*
/// plain text to cmark, so a link display containing it is dropped. A fragile macro
/// (`\code`/`\link`/…) or a non-fragile one with a literal argument (`\emph{x}`) is
/// inert. Mirrors the resolution [`serialize_macro`] performs, so the drop decision
/// and the render agree.
fn macro_arg_has_active_markdown(node: &SyntaxNode) -> bool {
    let head = macro_head(node);
    let name = head.trim_start_matches('\\');
    is_md_inline_text_macro(name)
        && macro_single_arg_content(node).is_some_and(|content| {
            inlines_have_active_markdown(&resolve_macro_arg_inlines(&content))
        })
}

/// Whether a resolved inline run carries cmark-active markup: any element that is
/// neither plain text nor an inert macro (see [`macro_arg_has_active_markdown`],
/// which recurses through nested non-fragile macros).
fn inlines_have_active_markdown(inlines: &[Inline]) -> bool {
    inlines.iter().any(|inl| match inl {
        Inline::Text(_) => false,
        Inline::Macro(n) => macro_arg_has_active_markdown(n),
        _ => true,
    })
}

/// A best-effort plain-text rendering of a resolved inline run, used only to test a
/// link's destination against its text (the `\url` auto-destination branch). Rich
/// inlines (emphasis, code) contribute their textual content; non-textual inlines
/// contribute nothing — a link whose destination equals such a text is vanishingly
/// rare, and the `\href` branch is the safe default.
pub(super) fn inline_plain_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inl in inlines {
        match inl {
            Inline::Text(t) => s.push_str(t),
            Inline::MdCode(t) => s.push_str(t),
            Inline::MdEmphasis { children, .. } => s.push_str(&inline_plain_text(children)),
            Inline::MdInlineLink { display, .. } => s.push_str(&inline_plain_text(display)),
            Inline::MdShortcutLink { display } => s.push_str(&inline_plain_text(display)),
            _ => {}
        }
    }
    s
}

/// The text of a link's display for **link-reference purposes** — its resolution
/// label ([`link_ref_label`]), its candidate in the refmap skeleton
/// ([`linkref_skeleton_push_exact`]), and the literal a demoted link rewrites to
/// ([`demoted_link_source`]). Identical to [`inline_plain_text`] except an
/// `Inline::Macro` contributes its **verbatim source** (`\emph{*x*}`) rather than
/// nothing: a pure-macro display (`[\emph{*x*}]`) would otherwise produce the empty
/// label `""`, whose `[]` candidate registers no refmap key
/// ([`bracket_free_group`]) — so the link was spuriously demoted to a literal `[]`
/// instead of reaching the drop/keep decision in [`serialize_inlines`]. The macro
/// source is also exactly what roxygen2's own `get_md_linkrefs` candidate scan sees,
/// and keeping the skeleton and the resolution label both routed through this helper
/// keeps them self-consistent (so a defined label never spuriously demotes).
pub(super) fn link_label_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inl in inlines {
        match inl {
            Inline::Text(t) => s.push_str(t),
            Inline::MdCode(t) => s.push_str(t),
            Inline::MdEmphasis { children, .. } => s.push_str(&link_label_text(children)),
            Inline::MdInlineLink { display, .. } => s.push_str(&link_label_text(display)),
            Inline::MdShortcutLink { display } => s.push_str(&link_label_text(display)),
            Inline::Macro(n) => s.push_str(&n.text().to_string()),
            _ => {}
        }
    }
    s
}

/// An inline `[text](url)` link, mirroring roxygen2's `mdxml_link`: an empty
/// destination — or one equal to the rendered link text — projects to `\url{text}`
/// (the destination is auto-generated from the text); otherwise `\href{url}{text}`.
fn inline_link_atom(text: &str, url: &str) -> String {
    if url.is_empty() || norm_ws(url) == norm_ws(text) {
        url_atom(text)
    } else {
        href_atom(text, url)
    }
}

/// Whether an autolink's inner text (the `…` in `<…>`) is a CommonMark **URI**
/// autolink rather than an **email** autolink: an ASCII letter, then 1–31 more of
/// letter/digit/`+`/`.`/`-`, then `:` (scheme length 2–32). Mirrors the scheme
/// check in the parser's `scan_md_autolink`; the two autolink forms are disjoint
/// (an email address has no `:`), so a `false` here means an email autolink.
fn autolink_has_uri_scheme(inner: &str) -> bool {
    let b = inner.as_bytes();
    if !b.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let mut j = 1;
    while j < b.len()
        && matches!(b[j], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'.' | b'-')
    {
        j += 1;
    }
    (2..=32).contains(&j) && b.get(j) == Some(&b':')
}

/// A bare URL → `(\url (VERB url))` (roxygen2's `\url{…}`; the URL is verbatim).
/// An empty URL (`[]()` — roxygen2's `\url{}`) has no content atom: parse_Rd
/// gives the macro no child, so the projection is a bare `(\url)`.
fn url_atom(url: &str) -> String {
    if url.is_empty() {
        return "(\\url)".to_string();
    }
    format!("(\\url (VERB {}))", encode_text(url))
}

/// An inline `[text](url)` link → `(\href (VERB url) <text>)`: the URL is verbatim
/// (no whitespace collapse), the display rendered by [`link_display_atom`] (a code
/// span sub-renders to `\verb`/`\code`; other text is whitespace-normalized prose,
/// an empty display contributing no atom).
fn href_atom(text: &str, url: &str) -> String {
    let mut atoms = vec![format!("(VERB {})", encode_text(url))];
    if let Some(atom) = link_display_atom(text) {
        atoms.push(atom);
    }
    format!("(\\href {})", atoms.join(" "))
}

/// The display-text atom for an inline `[text](url)` link. roxygen2 renders the
/// link's markdown *children*, so a single code-span text becomes `\verb`/`\code`
/// (via [`md_code_atom`], mirroring `mdxml_code`) rather than literal prose; any
/// other text is whitespace-normalized `(TEXT …)` (`None` when blank). General
/// inline sub-rendering of *mixed* markdown in link text (e.g. emphasis) is not
/// yet modeled — such a text stays plain prose (faithful under-handling, backlog).
fn link_display_atom(text: &str) -> Option<String> {
    let (inner, is_code) = unwrap_code_span(text);
    if is_code {
        Some(md_code_atom(inner))
    } else {
        text_atom(text)
    }
}

/// A reference link `[text][ref]` (explicit link text) → always `\link` over the
/// display text, `\code`-wrapped iff the display is a single code span. When the
/// display text equals the destination, roxygen2 treats the text as
/// auto-generated and falls back to the shortcut path.
fn ref_link_atom(text: &str, dest: &str) -> String {
    let (display, is_code) = unwrap_code_span(text);
    if norm_ws(display) == norm_ws(dest) {
        return shortcut_link_atom(dest);
    }
    code_wrap(
        format!("(\\link {})", text_atom(display).unwrap_or_default()),
        is_code,
    )
}

/// A shortcut link `[dest]` (no explicit link text) → roxygen2's `!has_link_text`
/// branch: `\linkS4class` for an `-class` destination without a package, else
/// `\link`; `\code`-wrapped when the destination is a code span or a `()` call.
/// The display text is `pkg::` + the object (with any `-class` suffix dropped).
pub(super) fn shortcut_link_atom(dest: &str) -> String {
    let (dest, code_span) = unwrap_code_span(dest);
    let is_code = code_span || dest.ends_with("()");
    let (pkg, fun) = match dest.rsplit_once("::") {
        Some((p, f)) => (Some(p), f),
        None => (None, dest),
    };
    let s4 = dest.ends_with("-class");
    let body = if s4 {
        fun.strip_suffix("-class").unwrap_or(fun)
    } else {
        fun
    };
    let head = if s4 && pkg.is_none() {
        "\\linkS4class"
    } else {
        "\\link"
    };
    let display = match pkg {
        Some(p) => format!("{p}::{body}"),
        None => body.to_string(),
    };
    code_wrap(
        format!("({head} {})", text_atom(&display).unwrap_or_default()),
        is_code,
    )
}

/// Resolve a `ROXYGEN_MD_IMAGE` leaf `![alt](url "title")` into its Rd atom,
/// mirroring roxygen2's `mdxml_image` (`markdown.R`). The alt text is *dropped*
/// (roxygen2 uses only the destination and title); the result is
/// `(\figure (VERB url) [(VERB title)])`, wrapped in `(\if (TEXT "html") …)` or
/// `(\if (TEXT "pdf") …)` per the extension-keyed `get_image_format` rule. Returns
/// `None` for an unrecognized shape (the leaf then stays literal prose).
pub(super) fn resolve_md_image(raw: &str) -> Option<String> {
    let (url, title) = image_url_title(raw)?;
    Some(figure_atom(&url, &title))
}

/// The demoted form of a resolved image after an odd trailing backslash run in
/// the preceding prose (see `run_ends_odd_backslash_run`): parse_Rd pairs the
/// literal `\` with the md-generated macro's own backslash, absorbing the name
/// into the TEXT and re-parsing each braced argument as a bare LIST. A bare
/// `\figure` demotes whole — its verbatim args re-parse as plain text — while a
/// format-keyed `\if` wrapper demotes only itself: the inner `\figure` still
/// parses inside the bare group (parse_Rd knows it anywhere), keeping its
/// verbatim args. Returns the demoted macro name and its argument atoms;
/// `None` mirrors [`resolve_md_image`] (the leaf stays literal prose).
pub(super) fn resolve_md_image_demoted(raw: &str) -> Option<(&'static str, Vec<String>)> {
    let (url, title) = image_url_title(raw)?;
    let cond = match image_format(&url) {
        ImageFormat::Html => "html",
        ImageFormat::Pdf => "pdf",
        ImageFormat::All => {
            let mut args = vec![text_atom(&url).unwrap_or_default()];
            if !title.is_empty() {
                args.push(text_atom(&title).unwrap_or_default());
            }
            return Some(("figure", args));
        }
    };
    Some((
        "if",
        vec![
            format!("(TEXT {})", encode_text(cond)),
            bare_figure_atom(&url, &title),
        ],
    ))
}

/// The destination and title a `ROXYGEN_MD_IMAGE` leaf resolves to — the shared
/// front half of [`resolve_md_image`]/[`resolve_md_image_demoted`]. `None` for
/// an unrecognized shape.
fn image_url_title(raw: &str) -> Option<(String, String)> {
    let bytes = raw.as_bytes();
    // The leaf always begins `![`; the alt span is `[…]` starting at index 1.
    let alt_end = scan_delimited(bytes, 1, b'[', b']')?;
    match bytes.get(alt_end) {
        // Inline image `![alt](url "title")`.
        Some(&b'(') => {
            let dest_end = scan_delimited(bytes, alt_end, b'(', b')')?;
            if dest_end != bytes.len() {
                return None;
            }
            let (url, title) = split_image_dest(&raw[alt_end + 1..dest_end - 1]);
            Some((url.to_string(), title.to_string()))
        }
        // Reference image `![alt][ref]`: roxygen2's `add_linkrefs_to_md` synthesizes
        // a `[ref]: R:URLencode(ref)` definition for the bracket-free `ref`
        // candidate, so the image destination is `R:ref`.
        Some(&b'[') => {
            let ref_end = scan_delimited(bytes, alt_end, b'[', b']')?;
            if ref_end != bytes.len() {
                return None;
            }
            let label = &raw[alt_end + 1..ref_end - 1];
            // A collapsed `![alt][]` resolves only through a *user* definition
            // (rewritten to the inline form upstream, `apply_user_linkrefs`):
            // its own `[alt]` candidate is blocked by `get_md_linkrefs`'
            // `(?=[^\[{])` lookahead (the span is followed by `[`), so no
            // `R:alt` definition is synthesized and cmark leaves the undefined
            // image literal — `None` keeps it in the prose run.
            if label.is_empty() {
                return None;
            }
            Some((synthesized_image_dest(label), String::new()))
        }
        // Shortcut image `![alt]`: the label is the alt, resolved against the
        // synthesized `[alt]: R:URLencode(alt)` definition.
        None => Some((synthesized_image_dest(&raw[2..alt_end - 1]), String::new())),
        _ => None,
    }
}

/// For a **reference** (`![alt][ref]`), **collapsed** (`![alt][]`), or **shortcut**
/// (`![alt]`) markdown image, the label roxygen2 resolves against the link-reference
/// map — the `ref` bracket, or the alt for a collapsed/shortcut form — paired with
/// the `[alt]` span (brackets included) needed to rebuild an inline image. `None`
/// for an inline image (`![alt](dest)`, which carries its own destination) or a
/// `{`-followed leaf. Mirrors the arms of [`resolve_md_image`]; used by
/// [`apply_user_linkrefs`] to override the synthesized `R:label` destination with a
/// user-defined one.
pub(super) fn image_user_ref(raw: &str) -> Option<(&str, &str)> {
    let bytes = raw.as_bytes();
    let alt_end = scan_delimited(bytes, 1, b'[', b']')?;
    let alt = &raw[1..alt_end]; // the `[alt]` span, brackets included
    match bytes.get(alt_end) {
        // Reference `![alt][ref]`: resolve `ref`. Collapsed `![alt][]`: the alt
        // is the label (cm-586) — and the only resolution path, since the
        // collapsed occurrence synthesizes no definition of its own (see
        // [`resolve_md_image`]).
        Some(&b'[') => {
            let ref_end = scan_delimited(bytes, alt_end, b'[', b']')?;
            if ref_end != bytes.len() {
                return None;
            }
            let label = &raw[alt_end + 1..ref_end - 1];
            if label.is_empty() {
                let alt_content = &raw[2..alt_end - 1];
                return (!alt_content.is_empty()).then_some((alt_content, alt));
            }
            Some((label, alt))
        }
        // Shortcut `![alt]`: the alt is the label.
        None => {
            let label = &raw[2..alt_end - 1];
            (!label.is_empty()).then_some((label, alt))
        }
        _ => None,
    }
}

/// Flatten a raw-source reference label for user-definition lookup: resolve it as
/// a markdown inline run and take its plain text, so an emphasis-bearing label
/// matches the way its *definition* was keyed. A definition's label arrives as a
/// resolved `MdShortcutLink` display and is flattened by [`inline_plain_text`]
/// (its source delimiters are gone), while an image's label is verbatim source
/// (`foo *bar*`) — flattening both sides through the same text walk makes
/// `![foo *bar*]` find `[foo *bar*]: url` (cm-575). A plain label is unchanged.
/// This matches by *flatten*, not source-exactly (cmark matches raw label text),
/// so a mixed-delimiter pair (`*bar*` vs `_bar_`) would spuriously match — the
/// same approximation the link machinery already makes ([`link_label_text`]).
pub(super) fn md_label_flatten(label: &str) -> String {
    inline_plain_text(&para_to_inlines(&resolve_md_inline(label)))
}

/// The synthesized destination for a shortcut/reference image whose `label` has no
/// user-defined `[label]: url` reference definition: roxygen2's `add_linkrefs_to_md`
/// appends `[label]: R:URLencode(label)`, so the image resolves to `R:label`.
///
/// A *user-defined* destination (`[ref]: https://…`) overrides this — handled by
/// [`apply_user_linkrefs`], which rewrites the reference/shortcut image to the inline
/// form `![alt](url)` before serialization. A label with URL-unsafe characters
/// (a space → `%20`, a backslash → `%5C`) is still backlog: the `%` is the Rd
/// comment char, so roxygen2 renders `\figure{R:see%20this}` and the `%` comments out
/// the closing brace, dropping the whole section — arity's fragile-macro neutralizer
/// keeps the section instead. Both diverge only for such labels; the common
/// bracket-free ASCII label (`R:x`) matches exactly.
fn synthesized_image_dest(label: &str) -> String {
    format!("R:{}", url_encode(label))
}

/// Split a CommonMark image destination `url "title"` into `(url, title)`. The URL
/// is angle-bracketed (`<…>`) or runs to the first ASCII whitespace; the optional
/// title that follows is wrapped in `"…"`, `'…'`, or `(…)`. A missing title is an
/// empty string.
fn split_image_dest(dest: &str) -> (&str, &str) {
    let dest = dest.trim();
    let (url, rest) = if dest.as_bytes().first() == Some(&b'<') {
        match dest.find('>') {
            Some(close) => (&dest[1..close], &dest[close + 1..]),
            None => (dest, ""),
        }
    } else {
        match dest.find(char::is_whitespace) {
            Some(sp) => (&dest[..sp], &dest[sp..]),
            None => (dest, ""),
        }
    };
    (url, strip_title_delims(rest.trim()))
}

/// Strip the surrounding title delimiters from a CommonMark image title
/// (`"…"`/`'…'`/`(…)`); return the input unchanged when it is not delimited.
pub(super) fn strip_title_delims(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2
        && matches!(
            (b[0], b[b.len() - 1]),
            (b'"', b'"') | (b'\'', b'\'') | (b'(', b')')
        )
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Build the `\figure` atom for an image, applying roxygen2's `get_image_format`:
/// a destination matching only the HTML extension set (`svg`) is wrapped in
/// `\if{html}{…}`, only the PDF set (`pdf`) in `\if{pdf}{…}`, and one matching both
/// (raster: `jpg`/`jpeg`/`gif`/`png`) or neither stays a bare `\figure`. The title
/// is verbatim and omitted when empty.
fn figure_atom(url: &str, title: &str) -> String {
    let figure = bare_figure_atom(url, title);
    match image_format(url) {
        ImageFormat::Html => format!("(\\if (TEXT {}) {figure})", encode_text("html")),
        ImageFormat::Pdf => format!("(\\if (TEXT {}) {figure})", encode_text("pdf")),
        ImageFormat::All => figure,
    }
}

/// The `\figure` atom without its format conditional — the demoted-`\if` path
/// keeps the inner macro intact while the wrapper's own braces re-parse bare.
fn bare_figure_atom(url: &str, title: &str) -> String {
    let mut args = vec![format!("(VERB {})", encode_text(url))];
    if !title.is_empty() {
        args.push(format!("(VERB {})", encode_text(title)));
    }
    format!("(\\figure {})", args.join(" "))
}

/// The conditional an image destination renders under, per roxygen2's
/// `get_image_format`/`default_image_formats` (`markdown.R`).
enum ImageFormat {
    Html,
    Pdf,
    All,
}

/// Classify an image destination by extension, mirroring roxygen2's
/// `default_image_formats` regexes (`[.](jpg|jpeg|gif|png|svg)$` for HTML,
/// `[.](jpg|jpeg|gif|png|pdf)$` for PDF). Matching both sets (or neither) is
/// `All` (a bare `\figure`); matching one only carves the `\if` wrapper.
fn image_format(url: &str) -> ImageFormat {
    let lower = url.to_ascii_lowercase();
    let has_dot_ext = |exts: &[&str]| {
        exts.iter()
            .any(|e| lower.strip_suffix(e).is_some_and(|p| p.ends_with('.')))
    };
    match (
        has_dot_ext(&["jpg", "jpeg", "gif", "png", "svg"]),
        has_dot_ext(&["jpg", "jpeg", "gif", "png", "pdf"]),
    ) {
        (true, false) => ImageFormat::Html,
        (false, true) => ImageFormat::Pdf,
        _ => ImageFormat::All,
    }
}

/// Index just past the balanced `close` byte matching the `open` at `start`, or
/// `None` if `start` is not `open` or the group never closes. Brackets are ASCII,
/// so a byte scan is sufficient.
pub(super) fn scan_delimited(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if bytes.get(start) != Some(&open) {
        return None;
    }
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
    }
    None
}
