//! CST → Rd-tree projector: the primary roxygen2 conformance engine.
//!
//! [`project_to_rd`] walks arity's lossless CST and emits the **parser-owned**
//! Rd section subtrees in roxygen2's canonical S-expression shape --- the same
//! shape the R driver's `block-to-sections` op mints (`tests/oracle/
//! roxygen_oracle.R`). The projector-parity gate (`tests/roxygen_projector.rs`)
//! diffs this against a *pinned* `expected.rdtree` per corpus case, so it runs in
//! plain `cargo test` with no R, and **structural** divergences (a `\describe`
//! the CST never modeled as a block, a markdown list still flat prose) surface as
//! a mismatch. That is the signal that drives parser growth.
//!
//! ## What it projects, and what it deliberately does not
//!
//! It is a **faithful encoding translation**, never a roxygen2 roclet
//! reimplementation (RECAP's first invariant). It projects what the parser
//! models: the title/description derived from the intro paragraphs, and the
//! body of the prose section tags (`@details`, `@return` → `\value`,
//! `@seealso`, `@source`, `@format`, `@section`, …). It excludes everything
//! roxygen2 *generates* rather than parses --- `\name`/`\alias` (the object),
//! `\usage` (the formals), and the `\arguments` wrapper that groups `@param`
//! (the `block-to-sections` op drops the same set, so the two stay aligned).
//!
//! ## Current reach
//!
//! A section body is projected as a *sequence* of inline atoms: prose runs
//! coalesce into whitespace-normalized `(TEXT …)`, and inline Rd macros
//! (`\code`/`\link`/`\emph`/`\url`/…, including nesting, a dropped `[pkg]`
//! option, and verbatim `(VERB …)` bodies) surface as nested subtrees from the
//! CST's `ROXYGEN_RD_MACRO` nodes. A section the CST does not yet model
//! structurally --- a multi-line `\describe`/`\itemize`/`\tabular`, or markdown
//! that roxygen2 translates into nodes under a resolved `@md` mode (`*x*` →
//! `\emph{x}`) --- still projects as flat text and therefore **diverges**. Those
//! divergences are the backlog: each is closed by teaching the *parser* the
//! structure, then the projector grows a faithful arm for the new nodes. Never
//! patch the projector to make a case pass.

use rowan::NodeOrToken;

use crate::ast::{AstNode, RoxygenBlock, RoxygenParagraph, RoxygenSection, RoxygenTag};
use crate::parser::parse;
use crate::parser::roxygen::is_two_arg_rd_macro;
use crate::syntax::{SyntaxKind, SyntaxNode};

/// Project `text` to the parser-owned Rd section subtrees, one canonical
/// S-expression per line, sorted --- byte-identical to the R driver's
/// `block-to-sections` output for the cases the projector models.
///
/// Sections are sorted (not in document order) because roxygen2's Rd emission
/// order is not the document order, and the projector does not replicate it; the
/// gate compares a *set* of section subtrees. Sections from every
/// `ROXYGEN_BLOCK` in `text` are merged into one sorted set.
pub fn project_to_rd(text: &str) -> String {
    let cst = parse(text).cst;
    let mut sections: Vec<String> = Vec::new();
    for block in cst.descendants().filter_map(RoxygenBlock::cast) {
        project_block(&block, &mut sections);
    }
    sections.sort();
    sections.join("\n")
}

/// One inline element of a section body: a run of prose text (coalesced and
/// whitespace-normalized at serialization) or an Rd macro node (projected as a
/// nested subtree). Modeling the body as a *sequence* — rather than one flat
/// string — is what lets inline `\code`/`\link`/… surface as structure.
#[derive(Clone)]
enum Inline {
    Text(String),
    Macro(SyntaxNode),
    /// A markdown inline leaf resolved under `@md` mode — emphasis, strong, or a
    /// code span — carrying its delimiter-stripped inner content. Emphasis/strong
    /// project to `\emph`/`\strong` over `(TEXT …)`; a code span projects to
    /// `\code` or `\verb` per roxygen2's R-parseability rule (see [`md_code_atom`]).
    Md(MdInline, String),
    /// A markdown block list resolved under `@md` mode (a `ROXYGEN_MD_LIST` node).
    /// Projects to `\itemize`/`\enumerate` with a name-only `\item` per item ahead
    /// of its content (see [`serialize_md_list`]).
    MdList(SyntaxNode),
    /// A markdown link resolved under `@md` mode, carrying the raw leaf text. The
    /// inline `[text](url)` form projects to `\href{url}{text}`; the reference
    /// (`[text][ref]`) and shortcut (`[dest]`) forms resolve to an `\link`/
    /// `\linkS4class` (optionally `\code`-wrapped) per roxygen2's `parse_link`
    /// (see [`resolve_md_link`]).
    MdLink(String),
    /// A markdown image resolved under `@md` mode, carrying the raw leaf text
    /// `![alt](url "title")`. Projects to `\figure{url}{title}` — wrapped in
    /// `\if{html}{…}`/`\if{pdf}{…}` per roxygen2's extension-keyed image-format
    /// rule (see [`resolve_md_image`]).
    MdImage(String),
}

/// The kind of a resolved markdown inline leaf.
#[derive(Clone, Copy)]
enum MdInline {
    Emph,
    Strong,
    Code,
}

/// One topic's worth of sections from a single roxygen block.
///
/// The block already owns logical structure: its children are `ROXYGEN_SECTION`s
/// (the intro, then one per `@tag`), each holding a `ROXYGEN_TAG` heading and/or
/// `ROXYGEN_PARAGRAPH`s. So the projector is a direct walk — the line-reassembly
/// state machine the line-flat CST forced is gone. A tag section's body is the
/// tag's own inline prose followed by its paragraphs (continuation and
/// paragraph-break both collapse to a single space under `norm_ws`).
fn project_block(block: &RoxygenBlock, out: &mut Vec<String>) {
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

    for section in block.sections() {
        if let Some(tag) = section.tag() {
            let name = tag.name().map(|n| n.to_string()).unwrap_or_default();
            let mut body = tag_inlines(&tag);
            for part in section_body_parts(&section) {
                if !body.is_empty() {
                    body.push(Inline::Text(" ".to_string()));
                }
                body.extend(part);
            }
            match name.as_str() {
                "slot" | "field" => {
                    let arg = tag.arg().map(|t| t.text().to_string()).unwrap_or_default();
                    if name == "slot" {
                        slots.push((arg, body));
                    } else {
                        fields.push((arg, body));
                    }
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
        .any(|(n, b)| n == "title" && !is_null_section(b));
    let has_explicit_desc = tag_sections
        .iter()
        .any(|(n, b)| n == "description" && !is_null_section(b));
    let explicit_title_body = tag_sections
        .iter()
        .find(|(n, b)| n == "title" && !is_null_section(b))
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
    // Everything remaining = details, merged with any explicit @details — but
    // roxygen2 only folds @details in when there *are* leftover intro paragraphs;
    // otherwise @details stands alone (emitted by the tag loop below).
    let intro_details = &intro_paras[cursor..];
    let merge_details = !intro_details.is_empty();

    if let Some(title) = &intro_title {
        push_section(out, "title", title);
    }

    // Description: the intro's 2nd paragraph, else roxygen2's
    // title-as-description fallback — when no description exists anywhere, the
    // title value (intro title, else explicit @title) is reused.
    let description = match intro_desc {
        Some(d) => Some(d),
        None if has_explicit_desc => None, // emitted by the tag loop below
        None => intro_title.clone().or(explicit_title_body),
    };
    if let Some(description) = description {
        push_section(out, "description", &description);
    }

    // The intro-derived details (and any folded-in @details).
    if merge_details {
        let mut body = join_paras(intro_details);
        for (_, ed) in tag_sections.iter().filter(|(n, _)| n == "details") {
            body.push(Inline::Text(" ".to_string()));
            body.extend(join_paras(std::slice::from_ref(ed)));
        }
        push_section(out, "details", &body);
    }

    for (name, body) in &tag_sections {
        // A folded-in @details was emitted above; skip the standalone section.
        if merge_details && name == "details" {
            continue;
        }
        project_tag_section(name, body, out);
    }

    // The aggregated `@slot`/`@field` sections (roxygen2's Slots/Fields).
    if !slots.is_empty() {
        out.push(describe_section("Slots", &slots));
    }
    if !fields.is_empty() {
        out.push(describe_section("Fields", &fields));
    }

    // The single aggregated `\examples` section (body reformatted R → placeholder).
    if has_examples {
        out.push("(\\examples ...)".to_string());
    }
}

/// Project the aggregated `@slot`/`@field` tags of a topic into a single
/// `\section{<title>}{\describe{\item{\code{name}}{def}…}}`. roxygen2 collects
/// every `@slot` (S4) and `@field` (reference class) into one Slots/Fields
/// section; each tag becomes a `\describe` item whose term is the verbatim
/// `\code{name}` (the name is R-code, tagged `RCODE` like a `\code` body) and
/// whose definition is the tag's prose.
fn describe_section(title: &str, items: &[(String, Vec<Inline>)]) -> String {
    let mut item_atoms: Vec<String> = Vec::new();
    for (name, def) in items {
        let code_atoms = rcode_atoms(name);
        let term = if code_atoms.is_empty() {
            "(\\code)".to_string()
        } else {
            format!("(\\code {})", code_atoms.join(" "))
        };
        // The definition is `\item`'s second (structural) argument: a multi-atom
        // prose+macro run is `(GRP …)`-wrapped, a single atom stays bare.
        let mut parts = vec![term];
        let def_arg = grp_arg(&serialize_inlines(def));
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

/// Flatten paragraphs into a single inline run, with a space between each (the
/// canonical serializer collapses the paragraph break to one space anyway).
fn join_paras(paras: &[Vec<Inline>]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        if i > 0 {
            out.push(Inline::Text(" ".to_string()));
        }
        for inl in p {
            out.push(match inl {
                Inline::Text(s) => Inline::Text(s.clone()),
                Inline::Macro(n) => Inline::Macro(n.clone()),
                Inline::Md(k, s) => Inline::Md(*k, s.clone()),
                Inline::MdList(n) => Inline::MdList(n.clone()),
                Inline::MdLink(s) => Inline::MdLink(s.clone()),
                Inline::MdImage(s) => Inline::MdImage(s.clone()),
            });
        }
    }
    out
}

/// Map a tag to its Rd section macro and push the projected subtree. Tags that
/// roxygen2 does not turn into a parser-owned section (`@param` feeds the excluded
/// `\arguments`; `@export`/`@md`/`@name`/… are directives) are skipped. The
/// aggregating `@slot`/`@field` tags are handled by [`describe_section`], not here.
fn project_tag_section(name: &str, body: &[Inline], out: &mut Vec<String>) {
    // roxygen2's `rd_section()` drops any section whose value is the literal
    // string "NULL" (`R/field.R`), a sentinel to suppress that field (e.g.
    // `@format NULL` to override an auto-generated data `\format`). This applies
    // to every prose tag that maps to a plain-string `rd_section`; `@section`
    // (a two-part value) and the excluded `@param`/… are unaffected.
    if NULL_SUPPRESSIBLE.contains(&name) && is_null_section(body) {
        return;
    }
    match name {
        // Direct prose → section-macro mappings.
        "description" => push_section(out, "description", body),
        "details" => push_section(out, "details", body),
        "return" => push_section(out, "value", body),
        "seealso" => push_section(out, "seealso", body),
        "source" => push_section(out, "source", body),
        "format" => push_section(out, "format", body),
        "references" => push_section(out, "references", body),
        "note" => push_section(out, "note", body),
        "author" => push_section(out, "author", body),
        "title" => push_section(out, "title", body),
        // `@section Title: body` → \section{Title}{body}. roxygen2 splits the
        // field value on its first `:`; parse_Rd then models `\section` as a
        // two-arg structural macro, so each side sub-parses inline macros/markdown
        // and a multi-atom argument is `(GRP …)`-wrapped while a single-atom one
        // stays bare (the same rule `serialize_macro` applies to `\item`/`\tabular`).
        "section" => {
            let (heading, content) = split_section_title(body);
            let title = serialize_inlines(&heading);
            let body = serialize_inlines(&content);
            let mut inner = grp_arg(&title);
            if !body.is_empty() {
                if !inner.is_empty() {
                    inner.push(' ');
                }
                inner.push_str(&grp_arg(&body));
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
fn is_null_section(body: &[Inline]) -> bool {
    let atoms = serialize_inlines(body);
    atoms.len() == 1 && atoms[0] == "(TEXT \"NULL\")"
}

/// Push `(\<macro> <atoms…>)` for a prose section, or `(\<macro>)` when the body
/// has no content (after coalescing).
fn push_section(out: &mut Vec<String>, macro_name: &str, body: &[Inline]) {
    let atoms = serialize_inlines(body);
    if atoms.is_empty() {
        out.push(format!("(\\{macro_name})"));
    } else {
        out.push(format!("(\\{macro_name} {})", atoms.join(" ")));
    }
}

/// Serialize an inline run into the canonical atom sequence: maximal prose runs
/// coalesce into one whitespace-normalized `(TEXT …)`, and each macro becomes a
/// nested subtree — mirroring the R driver's `serialize_children`.
fn serialize_inlines(body: &[Inline]) -> Vec<String> {
    let mut atoms: Vec<String> = Vec::new();
    let mut run = String::new();
    for inl in body {
        match inl {
            Inline::Text(s) => run.push_str(s),
            Inline::Macro(node) => {
                if let Some(atom) = text_atom(&run) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_macro(node));
            }
            Inline::Md(kind, content) => {
                if let Some(atom) = text_atom(&run) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_md_inline(*kind, content));
            }
            Inline::MdList(node) => {
                if let Some(atom) = text_atom(&run) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(serialize_md_list(node));
            }
            Inline::MdLink(raw) => {
                if let Some(atom) = text_atom(&run) {
                    atoms.push(atom);
                }
                run.clear();
                atoms.push(resolve_md_link(raw).unwrap_or_default());
            }
            Inline::MdImage(raw) => {
                if let Some(atom) = text_atom(&run) {
                    atoms.push(atom);
                }
                run.clear();
                if let Some(atom) = resolve_md_image(raw) {
                    atoms.push(atom);
                }
            }
        }
    }
    if let Some(atom) = text_atom(&run) {
        atoms.push(atom);
    }
    atoms
}

/// Project one `ROXYGEN_RD_MACRO` node into `(\name <children…>)`: the `[opt]` and
/// `{`/`}` delimiters are dropped, prose text coalesces into `(TEXT …)`, verbatim
/// content becomes `(VERB …)` (no whitespace collapse), and nested macros recurse.
///
/// A *structural* macro (`\item`, `\tabular` --- [`is_two_arg_rd_macro`]) models
/// each `{…}` argument as a list, so a multi-atom argument projects to a
/// `(GRP …)` wrapper (`\tabular{rl}{a \tab b}` → `(\tabular (TEXT "rl") (GRP …))`)
/// while a single-atom argument unwraps (`\item{a}{first}` → `(\item (TEXT "a")
/// (TEXT "first"))`). A latexlike macro (`\code`, `\emph`, …) inlines its single
/// argument's atoms directly, never wrapping.
fn serialize_macro(node: &SyntaxNode) -> String {
    let mut head = String::new();
    let mut structural = false;
    let mut out_atoms: Vec<String> = Vec::new();
    let mut group: Vec<String> = Vec::new();
    let mut run = String::new();
    // Flush the pending text run into the current argument group. A `\code` macro
    // tags its textual content as verbatim `(RCODE …)` (parse_Rd treats `\code`
    // bodies as R code, preserving whitespace and splitting at newlines); every
    // other macro coalesces prose into one whitespace-normalized `(TEXT …)`.
    let flush = |run: &mut String, group: &mut Vec<String>, code: bool| {
        if code {
            group.extend(rcode_atoms(run));
        } else if let Some(atom) = text_atom(run) {
            group.push(atom);
        }
        run.clear();
    };
    // Finalize a `{…}` argument group at its closing `}`: a structural macro's
    // multi-atom argument becomes a `(GRP …)` (parse_Rd models it as a list);
    // everything else (a single-atom argument, or a latexlike macro's inlined
    // content) splices its atoms in directly.
    let finalize = |group: &mut Vec<String>, out: &mut Vec<String>, structural: bool| {
        if structural && group.len() > 1 {
            out.push(format!("(GRP {})", group.join(" ")));
            group.clear();
        } else {
            out.append(group);
        }
    };
    for el in node.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_RD_MACRO_NAME => {
                head = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                structural = is_two_arg_rd_macro(head.trim_start_matches('\\'));
            }
            SyntaxKind::ROXYGEN_RD_MACRO_VERB => {
                flush(&mut run, &mut group, head == "\\code");
                let raw = el
                    .as_token()
                    .map(|t| t.text().to_string())
                    .unwrap_or_default();
                group.push(format!("(VERB {})", encode_text(&raw)));
            }
            SyntaxKind::ROXYGEN_RD_MACRO => {
                flush(&mut run, &mut group, head == "\\code");
                if let Some(n) = el.as_node() {
                    group.push(serialize_macro(n));
                }
            }
            // A closing `}` ends an argument group: flush the run, then finalize
            // the group (GRP-wrapping a structural macro's multi-atom argument).
            // The opening `{` carries no content.
            SyntaxKind::ROXYGEN_RD_MACRO_DELIM => {
                if el.as_token().is_some_and(|t| t.text() == "}") {
                    flush(&mut run, &mut group, head == "\\code");
                    finalize(&mut group, &mut out_atoms, structural);
                }
            }
            // The dropped option and the `#'` markers threaded into a multi-line
            // block macro carry no projected content; any other leaf (text, and
            // the collapsed newline/whitespace trivia) is prose.
            SyntaxKind::ROXYGEN_RD_MACRO_OPT | SyntaxKind::ROXYGEN_MARKER => {}
            _ => {
                if let Some(t) = el.as_token() {
                    run.push_str(t.text());
                }
            }
        }
    }
    // Defensive: trailing content with no closing brace (a malformed macro).
    flush(&mut run, &mut group, head == "\\code");
    finalize(&mut group, &mut out_atoms, structural);
    if out_atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", out_atoms.join(" "))
    }
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

/// Render a structural macro argument from its serialized atoms: a multi-atom
/// argument is `(GRP …)`-wrapped (parse_Rd models it as a list), a single-atom one
/// stays bare, and an empty one yields nothing. Mirrors `serialize_macro`'s
/// `finalize`, used for the `\section` title/body arguments.
fn grp_arg(atoms: &[String]) -> String {
    match atoms {
        [] => String::new(),
        [one] => one.clone(),
        many => format!("(GRP {})", many.join(" ")),
    }
}

fn prefix_space(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else {
        format!(" {s}")
    }
}

/// A `(TEXT "…")` atom with the body whitespace-normalized (matching the R
/// driver's `norm_ws`), or `None` if the body is blank.
fn text_atom(body: &str) -> Option<String> {
    let t = norm_ws(body);
    (!t.is_empty()).then(|| format!("(TEXT {})", encode_text(&t)))
}

/// The verbatim `(RCODE …)` atoms for a `\code` body. parse_Rd keeps `\code`
/// content verbatim (no whitespace collapse) but splits it at newlines, attaching
/// each `\n` to the atom it ends (`\code{a\nb}` → `(RCODE "a\n") (RCODE "b")`). An
/// empty body yields no atom.
fn rcode_atoms(body: &str) -> Vec<String> {
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

/// Collapse every whitespace run to a single space and trim (the R `norm_ws`).
fn norm_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Escape a string the way the R driver's `encode_text` does (`\`, `"`, `\n`).
fn encode_text(s: &str) -> String {
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

/// The body parts of a section, grouped into roxygen2 *paragraphs* (its blank-
/// line-delimited prose blocks), excluding its `@tag` heading. roxygen2 splits
/// the section text on `\n\n`, so a block macro or markdown list that directly
/// follows a prose line — with no blank `#'` line between — belongs to the same
/// paragraph as that prose; a blank `#'` line (a *section-level* `ROXYGEN_MARKER`,
/// as opposed to the per-line markers nested inside each node) starts a new
/// paragraph. Each returned `Vec<Inline>` is one such paragraph: a prose
/// `ROXYGEN_PARAGRAPH` contributes its inline run, a block `ROXYGEN_RD_MACRO`
/// (a multi-line `\itemize`/`\describe`/…) an `Inline::Macro`, and a
/// `ROXYGEN_MD_LIST` an `Inline::MdList`, with adjacent nodes joined by a space.
fn section_body_parts(section: &RoxygenSection) -> Vec<Vec<Inline>> {
    let mut groups: Vec<Vec<Inline>> = Vec::new();
    let mut cur: Vec<Inline> = Vec::new();
    for el in section.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_PARAGRAPH
            | SyntaxKind::ROXYGEN_RD_MACRO
            | SyntaxKind::ROXYGEN_MD_LIST => {
                let Some(node) = el.into_node() else { continue };
                let inlines = match node.kind() {
                    SyntaxKind::ROXYGEN_PARAGRAPH => RoxygenParagraph::cast(node)
                        .map(|p| paragraph_inlines(&p))
                        .unwrap_or_default(),
                    SyntaxKind::ROXYGEN_MD_LIST => vec![Inline::MdList(node)],
                    _ => vec![Inline::Macro(node)],
                };
                if !cur.is_empty() {
                    cur.push(Inline::Text(" ".to_string()));
                }
                cur.extend(inlines);
            }
            // A section-level `#'` marker is a blank doc-comment line: it ends the
            // current paragraph (per-line markers live *inside* the nodes above).
            SyntaxKind::ROXYGEN_MARKER if !cur.is_empty() => {
                groups.push(std::mem::take(&mut cur));
            }
            _ => {}
        }
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    groups
}

/// The inline elements of a prose paragraph: its text and inline Rd-macro
/// content, with the threaded `#'` markers dropped and inter-line newlines turned
/// into a joining space (continuation lines fold into one run). An Rd macro
/// becomes an `Inline::Macro`; all other content (plain text and — in the absence
/// of resolved markdown — inline code and link spans, which are literal Rd prose)
/// becomes `Inline::Text`. Whitespace is collapsed downstream by `norm_ws`.
fn paragraph_inlines(para: &RoxygenParagraph) -> Vec<Inline> {
    let mut out = Vec::new();
    for el in para.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MARKER => {} // trivia: never prose
            SyntaxKind::NEWLINE => out.push(Inline::Text(" ".to_string())), // line join
            _ => push_inline(&mut out, el),
        }
    }
    out
}

/// The inline elements of a tag line: everything after the `@`, the tag name, and
/// an arg-bearing tag's argument (and the leading whitespace before the prose).
fn tag_inlines(tag: &RoxygenTag) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut seen_prose = false;
    for el in tag.syntax().children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_AT | SyntaxKind::ROXYGEN_TAG_NAME | SyntaxKind::ROXYGEN_TAG_ARG => {
                continue;
            }
            SyntaxKind::WHITESPACE => {
                if seen_prose {
                    push_inline(&mut out, el);
                }
            }
            _ => {
                seen_prose = true;
                push_inline(&mut out, el);
            }
        }
    }
    out
}

/// Append `el` to an inline run: a macro node as `Inline::Macro`, anything else
/// as `Inline::Text` of its source text.
fn push_inline(out: &mut Vec<Inline>, el: NodeOrToken<SyntaxNode, crate::syntax::SyntaxToken>) {
    match el {
        NodeOrToken::Node(n) if n.kind() == SyntaxKind::ROXYGEN_RD_MACRO => {
            out.push(Inline::Macro(n));
        }
        NodeOrToken::Node(n) => out.push(Inline::Text(n.text().to_string())),
        // Markdown inline leaves (emitted only under `@md`): carve off their
        // delimiters and carry the inner content; the kind chooses the Rd macro.
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_EMPH => {
            out.push(Inline::Md(MdInline::Emph, strip_delim(t.text(), 1)));
        }
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_STRONG => {
            out.push(Inline::Md(MdInline::Strong, strip_delim(t.text(), 2)));
        }
        NodeOrToken::Token(t) if t.kind() == SyntaxKind::ROXYGEN_MD_CODE => {
            out.push(Inline::Md(MdInline::Code, strip_code_span(t.text())));
        }
        // A markdown link leaf: the inline `[text](url)` form projects to `\href`;
        // the reference (`[text][ref]`) and shortcut (`[dest]`) forms resolve to an
        // `\link`/`\linkS4class` (optionally `\code`-wrapped) per roxygen2's
        // `parse_link` (see [`resolve_md_link`]). A leaf that resolves to nothing
        // (an unrecognized shape) falls through to literal prose.
        NodeOrToken::Token(t)
            if t.kind() == SyntaxKind::ROXYGEN_MD_LINK && resolve_md_link(t.text()).is_some() =>
        {
            out.push(Inline::MdLink(t.text().to_string()));
        }
        // A markdown image leaf `![alt](url "title")` → `\figure` (see
        // [`resolve_md_image`]). A leaf that resolves to nothing falls through to
        // literal prose.
        NodeOrToken::Token(t)
            if t.kind() == SyntaxKind::ROXYGEN_MD_IMAGE && resolve_md_image(t.text()).is_some() =>
        {
            out.push(Inline::MdImage(t.text().to_string()));
        }
        // A `ROXYGEN_MD_LIST_MARKER` that reached an inline run (rather than a
        // `ROXYGEN_MD_LIST`) is a marker that did not form a list — the CommonMark
        // interrupt rule kept it inline. roxygen2 renders it as literal text.
        NodeOrToken::Token(t) => out.push(Inline::Text(t.text().to_string())),
    }
}

/// Strip `n` leading and `n` trailing delimiter bytes from an emphasis/strong
/// span (`*x*` → `x`, `**x**` → `x`). The span always has at least `2*n`
/// delimiter bytes around non-empty content (the lexer guarantees it).
fn strip_delim(text: &str, n: usize) -> String {
    text.get(n..text.len() - n).unwrap_or("").to_string()
}

/// The content of a markdown code span: drop the matched backtick runs, then
/// apply CommonMark's single-space trim (if the inner text both starts and ends
/// with a space but is not all spaces, one space is removed from each end).
fn strip_code_span(text: &str) -> String {
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

/// Project a resolved markdown inline leaf into its Rd atom: `\emph`/`\strong`
/// wrap whitespace-normalized `(TEXT …)`, while a code span becomes `\code` or
/// `\verb` per [`md_code_atom`].
fn serialize_md_inline(kind: MdInline, content: &str) -> String {
    match kind {
        MdInline::Emph => format!("(\\emph {})", text_atom(content).unwrap_or_default()),
        MdInline::Strong => format!("(\\strong {})", text_atom(content).unwrap_or_default()),
        MdInline::Code => md_code_atom(content),
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
fn resolve_md_link(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let text_end = scan_delimited(bytes, 0, b'[', b']')?;
    let text = &raw[1..text_end - 1];
    match bytes.get(text_end) {
        Some(&b'(') => {
            let url_end = scan_delimited(bytes, text_end, b'(', b')')?;
            (url_end == bytes.len()).then(|| href_atom(text, &raw[text_end + 1..url_end - 1]))
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

/// An inline `[text](url)` link → `(\href (VERB url) (TEXT text))`: the URL is
/// verbatim (no whitespace collapse), the display text whitespace-normalized
/// prose (an empty display contributes no atom).
fn href_atom(text: &str, url: &str) -> String {
    let mut atoms = vec![format!("(VERB {})", encode_text(url))];
    if let Some(atom) = text_atom(text) {
        atoms.push(atom);
    }
    format!("(\\href {})", atoms.join(" "))
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
fn shortcut_link_atom(dest: &str) -> String {
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
fn resolve_md_image(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    // The leaf always begins `![`; the alt span is `[…]` starting at index 1.
    let alt_end = scan_delimited(bytes, 1, b'[', b']')?;
    if bytes.get(alt_end) != Some(&b'(') {
        return None;
    }
    let dest_end = scan_delimited(bytes, alt_end, b'(', b')')?;
    if dest_end != bytes.len() {
        return None;
    }
    let (url, title) = split_image_dest(&raw[alt_end + 1..dest_end - 1]);
    Some(figure_atom(url, title))
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
fn strip_title_delims(s: &str) -> &str {
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
    let mut args = vec![format!("(VERB {})", encode_text(url))];
    if !title.is_empty() {
        args.push(format!("(VERB {})", encode_text(title)));
    }
    let figure = format!("(\\figure {})", args.join(" "));
    match image_format(url) {
        ImageFormat::Html => format!("(\\if (TEXT {}) {figure})", encode_text("html")),
        ImageFormat::Pdf => format!("(\\if (TEXT {}) {figure})", encode_text("pdf")),
        ImageFormat::All => figure,
    }
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

/// Wrap an atom in `(\code …)` when `is_code`, else return it unchanged.
fn code_wrap(inner: String, is_code: bool) -> String {
    if is_code {
        format!("(\\code {inner})")
    } else {
        inner
    }
}

/// If `s` is a single-backtick code span (`` `x` ``), return its inner text and
/// `true`; otherwise return `s` unchanged and `false`.
fn unwrap_code_span(s: &str) -> (&str, bool) {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'`' && b[b.len() - 1] == b'`' {
        (&s[1..s.len() - 1], true)
    } else {
        (s, false)
    }
}

/// Index just past the balanced `close` byte matching the `open` at `start`, or
/// `None` if `start` is not `open` or the group never closes. Brackets are ASCII,
/// so a byte scan is sufficient.
fn scan_delimited(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
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

/// Project a `ROXYGEN_MD_LIST` node into `(\itemize …)` or `(\enumerate …)`: each
/// `ROXYGEN_MD_LIST_ITEM` contributes a name-only `(\item)` followed by its
/// content atoms (the same inline serialization as prose), mirroring roxygen2's
/// translation of a markdown list into an Rd `\itemize`/`\enumerate`. The list is
/// ordered iff its first item's marker is a number.
fn serialize_md_list(node: &SyntaxNode) -> String {
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
        atoms.extend(serialize_inlines(&md_list_item_inlines(&item)));
    }
    if atoms.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", atoms.join(" "))
    }
}

/// Whether a `ROXYGEN_MD_LIST` is ordered (`\enumerate`): its first item's
/// `ROXYGEN_MD_LIST_MARKER` begins with a digit (`1.`/`1)`), as opposed to a
/// bullet (`-`/`*`/`+`).
fn md_list_is_ordered(node: &SyntaxNode) -> bool {
    node.descendants_with_tokens()
        .filter_map(|el| el.into_token())
        .find(|t| t.kind() == SyntaxKind::ROXYGEN_MD_LIST_MARKER)
        .is_some_and(|t| t.text().starts_with(|c: char| c.is_ascii_digit()))
}

/// The inline elements of a markdown list item: its content after the marker
/// leaf, with the threaded `#'` markers dropped and inter-line newlines turned
/// into joining spaces (the same treatment as a prose paragraph). The
/// `ROXYGEN_MD_LIST_MARKER` leaf itself is the item bullet, not content.
fn md_list_item_inlines(item: &SyntaxNode) -> Vec<Inline> {
    let mut out = Vec::new();
    for el in item.children_with_tokens() {
        match el.kind() {
            SyntaxKind::ROXYGEN_MD_LIST_MARKER | SyntaxKind::ROXYGEN_MARKER => {}
            SyntaxKind::NEWLINE => out.push(Inline::Text(" ".to_string())),
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
fn md_code_atom(content: &str) -> String {
    if code_span_is_r(content) {
        format!("(\\code (RCODE {}))", encode_text(content))
    } else {
        format!("(\\verb (VERB {}))", encode_text(content))
    }
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
fn code_span_is_r(code: &str) -> bool {
    if SPECIAL_CODE.contains(&code) {
        return true;
    }
    let out = crate::parser::parse(code);
    if !out.diagnostics.is_empty() {
        return false;
    }
    out.cst
        .children_with_tokens()
        .filter(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMENT
            )
        })
        .count()
        == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_plain_prose_sections() {
        let src = "#' Add two numbers\n\
                   #' @param x,y Numbers to add.\n\
                   #' @return Their sum.\n\
                   #' @export\n\
                   add <- function(x, y) x + y\n";
        // @param feeds the excluded \arguments; @export is a directive. Title and
        // description are derived from the single intro paragraph.
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Add two numbers\"))\n\
             (\\title (TEXT \"Add two numbers\"))\n\
             (\\value (TEXT \"Their sum.\"))"
        );
    }

    #[test]
    fn two_intro_paragraphs_split_title_and_description() {
        let src = "#' Example dataset\n\
                   #'\n\
                   #' A longer description.\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"A longer description.\"))\n\
             (\\title (TEXT \"Example dataset\"))"
        );
    }

    #[test]
    fn three_intro_paragraphs_split_title_description_details() {
        // roxygen2's `parse_description` (R/block.R): the 1st intro paragraph is
        // the title, the 2nd the description, and every remaining paragraph the
        // details — not all-the-rest folded into the description.
        let src = "#' title\n\
                   #'\n\
                   #' description\n\
                   #'\n\
                   #' details\n\
                   #' @name a\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"description\"))\n\
             (\\details (TEXT \"details\"))\n\
             (\\title (TEXT \"title\"))"
        );
    }

    #[test]
    fn section_body_serializes_inline_macros_with_grp_wrap() {
        // `@section Title: body` → \section{Title}{body}; parse_Rd models \section
        // as a two-arg structural macro, so the body sub-parses inline macros and
        // GRP-wraps its multi-atom argument while the single-atom title stays bare.
        let src = "#' Title\n\
                   #'\n\
                   #' Description.\n\
                   #' @section Foobar:\n\
                   #' With some \\strong{bold text}.\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Description.\"))\n\
             (\\section (TEXT \"Foobar\") (GRP (TEXT \"With some\") (\\strong (TEXT \"bold text\")) (TEXT \".\")))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn block_macro_joins_its_paragraph_then_splits_at_blank_line() {
        // A block macro that directly follows a prose line (no blank `#'` line)
        // belongs to that paragraph; a blank line starts the next paragraph. So
        // here the first `\itemize` rides with the description and the second with
        // the details — roxygen2 splits the intro on `\n\n`, not per CST node.
        let src = "#' Title\n\
                   #'\n\
                   #' Description with some\n\
                   #' \\itemize{\n\
                   #' \\item itemized\n\
                   #' \\item list\n\
                   #' }\n\
                   #'\n\
                   #' And then another one:\n\
                   #' \\itemize{\n\
                   #' \\item item 1\n\
                   #' \\item item 2\n\
                   #' }\n\
                   foo <- function() {}\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Description with some\") \
             (\\itemize (\\item) (TEXT \"itemized\") (\\item) (TEXT \"list\")))\n\
             (\\details (TEXT \"And then another one:\") \
             (\\itemize (\\item) (TEXT \"item 1\") (\\item) (TEXT \"item 2\")))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn trailing_intro_details_merge_with_explicit_details_tag() {
        // When the intro has leftover paragraphs *and* there is an explicit
        // @details tag, roxygen2 folds them into a single \details (intro
        // paragraphs first, then the tag body), rather than two sections.
        let src = "#' Title\n\
                   #'\n\
                   #' Description\n\
                   #'\n\
                   #' Details1\n\
                   #'\n\
                   #' Details2\n\
                   #'\n\
                   #' @details Details3\n\
                   #'\n\
                   #' Details4\n\
                   foo <- function(x) {}\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Description\"))\n\
             (\\details (TEXT \"Details1 Details2 Details3 Details4\"))\n\
             (\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn explicit_title_without_description_duplicates_into_description() {
        // roxygen2's title-as-description fallback: an explicit `@title` with no
        // intro prose and no `@description` reuses the title as the description.
        let src = "#' @title a\n#' @name a\nNULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"a\"))\n(\\title (TEXT \"a\"))"
        );
    }

    #[test]
    fn null_tag_value_suppresses_section() {
        // roxygen2's `rd_section()` treats a value of the literal string "NULL" as
        // a sentinel that suppresses the section (`R/field.R`). `@format NULL` and
        // `@details NULL` emit no section at all; `@description NULL` suppresses the
        // explicit description, which re-triggers the title-as-description fallback.
        let src = "#' Title\n\
                   #' @description NULL\n\
                   #' @details NULL\n\
                   #' @format NULL\n\
                   #' @name d\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"Title\"))\n(\\title (TEXT \"Title\"))"
        );
    }

    #[test]
    fn examples_body_is_a_placeholder() {
        let src = "#' T\n#' @examples\n#' f(1)\n#' @name d\nNULL\n";
        assert!(project_to_rd(src).contains("(\\examples ...)"));
    }

    #[test]
    fn multiple_examples_tags_merge_into_one_section() {
        // roxygen2's `@examples`/`@examplesIf` is an aggregating field: every
        // examples tag of a topic concatenates into a *single* `\examples`
        // section, so the projector emits exactly one `(\examples ...)` no matter
        // how many tags appear.
        let src = "#' @name a\n\
                   #' @title a\n\
                   #' @examples\n\
                   #' TRUE\n\
                   #' @examples\n\
                   #' FALSE\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\description (TEXT \"a\"))\n\
             (\\examples ...)\n\
             (\\title (TEXT \"a\"))"
        );
    }

    #[test]
    fn projects_inline_rd_macros() {
        // Nested latexlike macros, a dropped `[pkg]` option, and a verbatim
        // `\url` (VERB, not coalesced TEXT) — the faithful translation of the
        // CST macro nodes into roxygen2's Rd section shape.
        let src = "#' T\n\
                   #'\n\
                   #' See \\code{\\link{add}} and \\emph{e}, plus \\url{http://x}\n\
                   #' and \\link[stats]{lm} end.\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\description (TEXT \"See\") (\\code (\\link (TEXT \"add\"))) \
                 (TEXT \"and\") (\\emph (TEXT \"e\")) (TEXT \", plus\") \
                 (\\url (VERB \"http://x\")) (TEXT \"and\") (\\link (TEXT \"lm\")) \
                 (TEXT \"end.\"))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn code_macro_body_projects_as_rcode() {
        // parse_Rd tags a `\code` body as verbatim R code: its plain text becomes
        // `(RCODE …)`, not the whitespace-normalized `(TEXT …)` every other
        // latexlike macro produces (`\verb` stays VERB; a nested macro recurses).
        let src = "#' T\n\
                   #'\n\
                   #' Some \\code{code} and \\verb{More code.}\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\description (TEXT \"Some\") (\\code (RCODE \"code\")) (TEXT \"and\") \
                 (\\verb (VERB \"More code.\")))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn href_projects_verbatim_url_and_latexlike_text() {
        // `\href{url}{text}` is a two-arg *structural* macro with a per-argument
        // encoding: parse_Rd tags the first argument (the URL) as verbatim `VERB`
        // and sub-parses the second (the link text) like any latexlike body, so a
        // multi-atom link text wraps in `(GRP …)` and nested macros recurse.
        let src = "#' T\n\
                   #'\n\
                   #' See \\href{http://a.com/x y}{click \\emph{here} now}.\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\description (TEXT \"See\") (\\href (VERB \"http://a.com/x y\") \
                 (GRP (TEXT \"click\") (\\emph (TEXT \"here\")) (TEXT \"now\"))) (TEXT \".\"))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn multiline_itemize_projects_nested() {
        // A multi-line `\itemize` block macro: each `\item` is a name-only nested
        // macro, its trailing prose a sibling `(TEXT …)` --- the pinned shape, from
        // the kind-based `serialize_macro` walking the block-macro node.
        let src = "#' @details\n\
                   #' \\itemize{\n\
                   #'   \\item one\n\
                   #'   \\item two\n\
                   #' }\n\
                   #' @name x\n\
                   NULL\n";
        assert_eq!(
            project_to_rd(src),
            "(\\details (\\itemize (\\item) (TEXT \"one\") (\\item) (TEXT \"two\")))"
        );
    }

    #[test]
    fn multiline_describe_item_projects_two_args() {
        // A multi-line `\describe` whose `\item{term}{def}` takes *two* brace
        // groups (Stage 3): the lexer pulls both groups into one macro token, the
        // tree builder emits both as `\item` children, and the projector flushes
        // at each closing `}` so they stay separate atoms ---
        // `(\item (TEXT "a") (TEXT "first"))`, byte-identical to roxygen2.
        let src = "#' T\n\
                   #' @format A frame:\n\
                   #' \\describe{\n\
                   #'   \\item{a}{first}\n\
                   #'   \\item{b}{second}\n\
                   #' }\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\describe (\\item (TEXT \"a\") (TEXT \"first\")) \
                 (\\item (TEXT \"b\") (TEXT \"second\")))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn multiline_tabular_projects_format_and_grp_body() {
        // A multi-line `\tabular{format}{content}`: the format arg projects to a
        // single `(TEXT …)`, the multi-row body to a `(GRP …)` (parse_Rd models
        // each `\tabular` argument as a list, so a multi-atom one wraps), with
        // `\tab`/`\cr` as name-only macros --- byte-identical to roxygen2.
        let src = "#' T\n\
                   #' @details\n\
                   #' \\tabular{rl}{\n\
                   #'   a \\tab the first row \\cr\n\
                   #'   b \\tab the second row \\cr\n\
                   #' }\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\details (\\tabular (TEXT \"rl\") \
                 (GRP (TEXT \"a\") (\\tab) (TEXT \"the first row\") (\\cr) \
                 (TEXT \"b\") (\\tab) (TEXT \"the second row\") (\\cr))))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn md_inline_projects_emph_strong_and_code_vs_verb() {
        // Under a resolved `@md` mode the inline grammar gains emphasis/strong and
        // markdown code spans. A code span renders as `\code` when its content
        // parses as a single R expression (`a + b`) and `\verb` otherwise (`inline
        // code` is two symbols) --- roxygen2's `can_parse` rule, replicated with
        // arity's own parser.
        let src = "#' T\n\
                   #' @details\n\
                   #' Text with *emphasis*, **strong** words, `inline code`, and `a + b` code.\n\
                   #' @md\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\details (TEXT \"Text with\") (\\emph (TEXT \"emphasis\")) (TEXT \",\") \
                 (\\strong (TEXT \"strong\")) (TEXT \"words,\") (\\verb (VERB \"inline code\")) \
                 (TEXT \", and\") (\\code (RCODE \"a + b\")) (TEXT \"code.\"))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn md_block_lists_project_itemize_and_enumerate() {
        // Under a resolved `@md` mode, a `-`/`*`/`+` list projects to `\itemize`
        // and a `1.`/`1)` list to `\enumerate`, each item a name-only `\item`
        // ahead of its content --- roxygen2's translation of a markdown list into
        // Rd, replicated from the `ROXYGEN_MD_LIST` node.
        let src = "#' T\n\
                   #' @details\n\
                   #' Bullets:\n\
                   #'\n\
                   #' - first\n\
                   #' - second\n\
                   #'\n\
                   #' Numbered:\n\
                   #'\n\
                   #' 1. one\n\
                   #' 2. two\n\
                   #' @md\n\
                   #' @name d\n\
                   NULL\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\details (TEXT \"Bullets:\") \
                 (\\itemize (\\item) (TEXT \"first\") (\\item) (TEXT \"second\")) \
                 (TEXT \"Numbered:\") \
                 (\\enumerate (\\item) (TEXT \"one\") (\\item) (TEXT \"two\")))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn slot_tags_aggregate_into_slots_section() {
        // roxygen2 collects every `@slot` of an S4 class into a single
        // `\section{Slots}{\describe{…}}`, each slot a `\describe` item whose term
        // is the verbatim `\code{name}` and whose definition is the tag's prose.
        let src = "#' Important class.\n\
                   #'\n\
                   #' @slot a slot a\n\
                   #' @slot b slot b\n\
                   setClass('test')\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\section (TEXT \"Slots\") (\\describe \
                 (\\item (\\code (RCODE \"a\")) (TEXT \"slot a\")) \
                 (\\item (\\code (RCODE \"b\")) (TEXT \"slot b\"))))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn field_tags_aggregate_into_fields_section() {
        // The reference-class analog of `@slot`: every `@field` aggregates into a
        // single `\section{Fields}{\describe{…}}` with the same item shape.
        let src = "#' Important class.\n\
                   #'\n\
                   #' @field a field a\n\
                   #' @field b field b\n\
                   setRefClass('test')\n";
        let out = project_to_rd(src);
        assert!(
            out.contains(
                "(\\section (TEXT \"Fields\") (\\describe \
                 (\\item (\\code (RCODE \"a\")) (TEXT \"field a\")) \
                 (\\item (\\code (RCODE \"b\")) (TEXT \"field b\"))))"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn md_block_list_is_off_without_md_tag() {
        // No `@md`: the `-` lines stay literal Rd prose (no `\itemize`), one
        // coalesced `(TEXT …)` --- the CST, and thus the projection, is mode-keyed.
        let src = "#' T\n\
                   #' @details\n\
                   #' - first\n\
                   #' - second\n\
                   #' @name d\n\
                   NULL\n";
        assert!(
            project_to_rd(src).contains("(\\details (TEXT \"- first - second\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }

    #[test]
    fn md_inline_is_off_without_md_tag() {
        // No `@md`: markdown is not resolved, so `*emphasis*` and `` `code` `` stay
        // literal Rd prose (one coalesced `(TEXT …)`, delimiters included) --- the
        // CST, and thus the projection, is mode-keyed.
        let src = "#' T\n\
                   #' @details\n\
                   #' Text with *emphasis* and `code` here.\n\
                   #' @name d\n\
                   NULL\n";
        assert!(
            project_to_rd(src)
                .contains("(\\details (TEXT \"Text with *emphasis* and `code` here.\"))"),
            "got: {}",
            project_to_rd(src)
        );
    }
}
