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
fn same_line_nested_list_marker_projects_a_sublist() {
    // `- - foo`: an item whose content itself begins with a list marker
    // holds a nested list (cm-300); a tag-value form nests the same way.
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' - - foo\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (\\itemize (\\item) (\\itemize (\\item) (TEXT \"foo\"))))\n\
         (\\title (TEXT \"T\"))"
    );
    let value = "#' @md\n\
                 #' @title T\n\
                 #' @details - - foo\n\
                 #' @name x\n\
                 NULL\n";
    assert_eq!(
        project_to_rd(value),
        "(\\description (TEXT \"T\"))\n\
         (\\details (\\itemize (\\item) (\\itemize (\\item) (TEXT \"foo\"))))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn item_starting_with_indented_code_snaps_content_indent() {
    // `1.     code`: content five or more columns past the marker starts
    // with indented code, so the content indent snaps to marker + 1
    // (cm-275/276) — the line's remainder is a code block inside the item,
    // and a later line at the snapped column is item content.
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' 1.     code\n\
               #'\n\
               #'    para\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (\\enumerate (\\item) (\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\"))) (\\preformatted (VERB \"code\\n\")) (\\if (TEXT \"html\") (\\out (VERB \"</div>\"))) (TEXT \"para\")))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn empty_marker_folds_next_line_content_but_not_across_a_blank() {
    // `-` alone: the item's content starts on the immediately following
    // line at the content column (cm-280/281). An actual blank line in
    // between keeps the content out — "a list item can begin with at most
    // one blank line" (cm-282).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' -\n\
               #'   foo\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (\\itemize (\\item) (TEXT \"foo\")))\n\
         (\\title (TEXT \"T\"))"
    );
    let blank = "#' @md\n\
                 #' @title T\n\
                 #' @details\n\
                 #' -\n\
                 #'\n\
                 #'   foo\n\
                 #' @name x\n\
                 NULL\n";
    assert_eq!(
        project_to_rd(blank),
        "(\\description (TEXT \"T\"))\n\
         (\\details (\\itemize (\\item)) (TEXT \"foo\"))\n\
         (\\title (TEXT \"T\"))"
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
fn md_heading_hoists_section_and_nests_subsection() {
    // A level-1 heading in `@details` hoists to a top-level `\section` (out of
    // `\details`); a deeper heading nests as a `\subsection`. With no prose
    // before the first heading, `\details` is omitted entirely.
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' # First\n\
               #' a\n\
               #'\n\
               #' ## Nested\n\
               #' b\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\section (TEXT \"First\") (GRP (TEXT \"a\") (\\subsection (TEXT \"Nested\") (TEXT \"b\"))))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn md_subsection_without_level_one_stays_in_details() {
    // A level->=2 heading with no enclosing level-1 heading nests as a
    // `\subsection` inside the enclosing `\details`, which keeps its leading
    // prose (the section is not hoisted out).
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' Lead.\n\
               #'\n\
               #' ## Sub\n\
               #' body\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\details (TEXT \"Lead.\") (\\subsection (TEXT \"Sub\") (TEXT \"body\")))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn setext_title_sheds_leading_linkref_defs() {
    // cmark strips a paragraph's leading link-reference definitions *before*
    // deciding a setext promotion (`resolve_reference_link_definitions`), so
    // the definition line never joins the heading title — the title is only
    // `bar` — and the def resolves the reference in the section body, across
    // the heading split (cm-217).
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' [foo]: /url\n\
               #' bar\n\
               #' ===\n\
               #' [foo]\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\section (TEXT \"bar\") (\\href (VERB \"/url\") (TEXT \"foo\")))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn all_defs_setext_title_demotes_heading() {
    // A setext paragraph consisting only of link-reference definitions is
    // never promoted: the definitions strip, the `===` underline is ordinary
    // paragraph text, and the following line continues that paragraph
    // (cm-218). The stripped definition still resolves the reference.
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' [foo]: /url\n\
               #' ===\n\
               #' [foo]\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\details (TEXT \"===\") (\\href (VERB \"/url\") (TEXT \"foo\")))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn field_defs_resolve_heading_title_reference() {
    // roxygen2 markdown-processes the whole field as one document before
    // splitting sections, so a definition in a section *body* resolves a
    // reference in its heading *title* (cm-216; label match is case-folded).
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' # [Foo]\n\
               #' [foo]: /url\n\
               #' body\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\section (\\href (VERB \"/url\") (TEXT \"Foo\")) (TEXT \"body\"))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn md_setext_heading_hoists_section() {
    // A setext `===` underline promotes its preceding paragraph into a level-1
    // heading, hoisted to a top-level `\section` out of `\details` (same as an
    // ATX `#`). The prose after the underline is the section body.
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' Big\n\
               #' ===\n\
               #'\n\
               #' body\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\section (TEXT \"Big\") (TEXT \"body\"))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn md_setext_multiline_title_and_nesting() {
    // The underline promotes the *whole* preceding paragraph: `intro`+`Sub` are
    // one paragraph (no blank between), so `---` makes both the H2 title
    // ("intro Sub"). A `-` underline is level 2, nested under the `===` H1.
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' Top\n\
               #' ===\n\
               #' intro\n\
               #' Sub\n\
               #' ---\n\
               #' deep\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\section (TEXT \"Top\") (\\subsection (TEXT \"intro Sub\") (TEXT \"deep\")))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn md_setext_single_dash_underline_hoists_subsection() {
    // CommonMark resolves a lone `-` line after a paragraph as a level-2 setext
    // underline (an empty list item cannot interrupt a paragraph), so `Foo`
    // becomes an H2 `\subsection` and `bar` its body. A `- item` line with
    // content would instead interrupt as a list (not exercised here).
    let src = "#' Title\n\
               #'\n\
               #' @md\n\
               #' @details\n\
               #' Foo\n\
               #' -\n\
               #' bar\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\details (\\subsection (TEXT \"Foo\") (TEXT \"bar\")))\n\
         (\\title (TEXT \"Title\"))"
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
fn non_md_percent_is_an_rd_line_comment() {
    // In non-markdown prose (literal Rd), an unescaped `%` begins a comment to
    // end of line, so `@format %` projects to an empty `\format` and a mid-line
    // `%` keeps only the prose before it (roxygen2 passes the value as raw Rd).
    let src = "#' Title here\n\
               #'\n\
               #' Desc with a %% comment to end of line\n\
               #' @format %\n\
               x <- list(a = 1, b = 2)\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Desc with a\"))\n\
         (\\format)\n\
         (\\title (TEXT \"Title here\"))"
    );
}

#[test]
fn non_md_percent_comment_is_scoped_per_line() {
    // The `%` comment runs only to the end of *its* physical line: a multi-line
    // tag value drops the commented tail of the first line but keeps the next
    // line, then both coalesce under `norm_ws`.
    let src = "#' Title\n\
               #' @details First detail line %% trailing comment\n\
               #'   second detail line stays\n\
               #' @name x\n\
               NULL\n";
    // Sections sort in byte order: `\description` < `\details` < `\title`.
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\details (TEXT \"First detail line second detail line stays\"))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn md_mode_percent_survives() {
    // Under `@md` roxygen2 escapes `%` (`\%`), which `parse_Rd` decodes back to a
    // literal `%`, so the character survives in the projected text — the
    // projector must *not* treat it as a comment in markdown mode.
    let src = "#' Title\n\
               #' @md\n\
               #' @format % and more\n\
               x <- list(a = 1)\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\format (TEXT \"% and more\"))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn strip_rd_line_comment_honors_backslash_escape() {
    // A `\%` is an escaped percent, not a comment opener.
    assert_eq!(strip_rd_line_comment("a %% b"), "a ");
    assert_eq!(strip_rd_line_comment("%"), "");
    assert_eq!(strip_rd_line_comment("no comment here"), "no comment here");
    assert_eq!(
        strip_rd_line_comment("keep \\% literal"),
        "keep \\% literal"
    );
    assert_eq!(
        strip_rd_line_comment("keep \\% then % cut"),
        "keep \\% then "
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
fn sameline_tag_value_folds_plain_continuation() {
    // A tag with a same-line prose value folds its contiguous plain-prose
    // continuation into the `ROXYGEN_TAG` node (see `emit_tag_line`), so the
    // whole field value projects as one run. This exercises `tag_inlines`'
    // handling of the folded threaded `#'` markers (dropped) and inter-line
    // newlines (a soft break `norm_ws` collapses) — no markdown span involved.
    let src = "#' Title\n\
               #' @details First line\n\
               #' second line.\n\
               #' @name d\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Title\"))\n\
         (\\details (TEXT \"First line second line.\"))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn md_table_projects_to_tabular() {
    // A GFM table under `@md` projects to `\tabular`: the delimiter row supplies
    // the per-column alignment (`l`/`c`/`r`), the header and body rows fill one
    // `GRP` with `\tab` between cells and `\cr` ending each row, a short row is
    // padded (an empty trailing cell) and a long row truncated to the column
    // count, and each cell's content resolves as markdown.
    let src = "#' T\n\
               #' @md\n\
               #' @details\n\
               #' | a | b |\n\
               #' | :-- | --: |\n\
               #' | *x* | y |\n\
               #' | solo |\n\
               #' @name d\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (\\tabular (TEXT \"lr\") (GRP \
         (TEXT \"a\") (\\tab) (TEXT \"b\") (\\cr) \
         (\\emph (TEXT \"x\")) (\\tab) (TEXT \"y\") (\\cr) \
         (TEXT \"solo\") (\\tab) (\\cr))))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn md_block_quote_flattens_to_plain_text() {
    // roxygen2 does not support block quotes: it warns and renders the node's
    // *flattened plain text* (`escape_comment(xml_text)`) — the `>` markers and
    // inner markdown (emphasis, code, link) dropped, and the two lines concatenated
    // with **no separator** (`code` + `and` glue to `codeand`).
    let src = "#' T\n\
               #' @md\n\
               #' @details\n\
               #' > a *quote* with `code`\n\
               #' > and [text](https://x.org)\n\
               #' @name d\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"a quote with codeand text\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn md_block_quote_glues_onto_adjacent_prose() {
    // A block quote emits no paragraph separator, so its flattened text glues
    // onto the surrounding prose with no space: a preceding paragraph on the
    // *same* line (`before` + `> q` → `beforeq`), a preceding paragraph across a
    // *blank* line (still glued), and a following paragraph that keeps its own
    // separating space (`beforeq after`). Two adjacent quotes also glue (`q1q2`).
    let same_part = "#' T\n\
                     #' @md\n\
                     #' @details\n\
                     #' before\n\
                     #' > quoted line\n\
                     #' @name d\n\
                     NULL\n";
    assert_eq!(
        project_to_rd(same_part),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"beforequoted line\"))\n\
         (\\title (TEXT \"T\"))"
    );

    let around = "#' T\n\
                  #' @md\n\
                  #' @details\n\
                  #' before\n\
                  #'\n\
                  #' > quoted\n\
                  #'\n\
                  #' after\n\
                  #' @name d\n\
                  NULL\n";
    assert_eq!(
        project_to_rd(around),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"beforequoted after\"))\n\
         (\\title (TEXT \"T\"))"
    );

    let two_quotes = "#' T\n\
                      #' @md\n\
                      #' @details\n\
                      #' > q1\n\
                      #'\n\
                      #' > q2\n\
                      #' @name d\n\
                      NULL\n";
    assert_eq!(
        project_to_rd(two_quotes),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"q1q2\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn md_block_quote_lazy_continuation_folds_into_quote() {
    // CommonMark lazy continuation: a non-`>` paragraph line immediately after a
    // quote line (no blank) belongs to the quote's open paragraph, so it flattens
    // into the quote with **no** separator (`quoted line one` + `lazy continuation`
    // → `quoted line onelazy continuation`). A blank line ends the quote; the
    // following paragraph is separate and keeps its own separating space.
    let src = "#' T\n\
               #' @md\n\
               #' @details\n\
               #' > quoted line one\n\
               #' lazy continuation\n\
               #'\n\
               #' Separate paragraph.\n\
               #' @name d\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"quoted line onelazy continuation Separate paragraph.\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn md_thematic_break_renders_empty_and_coalesces() {
    // roxygen2 has no thematic-break support: it warns and renders the empty
    // `escape_comment(xml_text)` (a break has no text), so the break contributes
    // nothing and the surrounding paragraphs coalesce into one `\details` atom.
    // A `---` after a blank (setext heads nothing), a `***` interrupting a
    // paragraph, and an `___` all render identically.
    let src = "#' T\n\
               #' @md\n\
               #' @details\n\
               #' Before.\n\
               #'\n\
               #' ---\n\
               #'\n\
               #' Foo\n\
               #' ***\n\
               #' bar\n\
               #'\n\
               #' ___\n\
               #'\n\
               #' After.\n\
               #' @name d\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"Before. Foo bar After.\"))\n\
         (\\title (TEXT \"T\"))"
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
fn md_non_fragile_macro_arg_is_markdown_processed() {
    // Under `@md`, a non-fragile inline text macro (`\emph`) has its argument
    // markdown-processed, so `*x*` becomes a nested `\emph` — matching
    // roxygen2's `\emph{\emph{x}}` (`escaped_for_md` protects only the fragile
    // set). A fragile macro (`\code`) keeps its argument literal.
    let emph = "#' @md\n#' @title T\n#' @details A \\emph{*x*} b.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(emph)
            .contains("(\\details (TEXT \"A\") (\\emph (\\emph (TEXT \"x\"))) (TEXT \"b.\"))"),
        "{}",
        project_to_rd(emph)
    );

    let multi = "#' @md\n#' @title T\n#' @details A \\emph{a *b* c} d.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(multi).contains(
            "(\\details (TEXT \"A\") (\\emph (TEXT \"a\") (\\emph (TEXT \"b\")) (TEXT \"c\")) (TEXT \"d.\"))"
        ),
        "{}",
        project_to_rd(multi)
    );

    let strong = "#' @md\n#' @title T\n#' @details A \\strong{*x*} b.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(strong).contains("(\\strong (\\emph (TEXT \"x\")))"),
        "{}",
        project_to_rd(strong)
    );

    // `\code` is fragile — its body stays literal `*x*` (RCODE), not `\emph`.
    let code = "#' @md\n#' @title T\n#' @details A \\code{*x*} b.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(code).contains("(\\code (RCODE \"*x*\"))"),
        "{}",
        project_to_rd(code)
    );
}

#[test]
fn md_structural_macro_args_are_markdown_processed() {
    // Under `@md`, a structural two-arg macro (`\item`, `\tabular`, `\href`)
    // has *each* of its non-verbatim arguments markdown-processed, then a
    // multi-atom argument GRP-wraps (parse_Rd models it as a list). roxygen2
    // protects only its `escaped_for_md` set, so `\item`/`\tabular`/`\href`'s
    // text args are markdown while a nested fragile macro (`\code`) stays
    // literal and a verbatim argument (the `\href` URL) is untouched.
    let item = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
                #'   \\item{*term*}{a \\strong{bold} def}\n#' }\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(item).contains(
            "(\\describe (\\item (\\emph (TEXT \"term\")) \
             (GRP (TEXT \"a\") (\\strong (TEXT \"bold\")) (TEXT \"def\"))))"
        ),
        "{}",
        project_to_rd(item)
    );

    // Both arguments single-atom markdown unwrap (no GRP).
    let two = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
               #'   \\item{*term*}{*def*}\n#' }\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(two)
            .contains("(\\describe (\\item (\\emph (TEXT \"term\")) (\\emph (TEXT \"def\"))))"),
        "{}",
        project_to_rd(two)
    );

    // A nested fragile macro keeps its argument literal even inside an md arg.
    let frag = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
                #'   \\item{x}{a \\code{*y*} b}\n#' }\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(frag).contains(
            "(\\item (TEXT \"x\") (GRP (TEXT \"a\") (\\code (RCODE \"*y*\")) (TEXT \"b\")))"
        ),
        "{}",
        project_to_rd(frag)
    );

    // `\href`: verbatim URL untouched, display markdown-processed and wrapped.
    let href = "#' @md\n#' @title T\n#' @details See \\href{http://x.org}{*the* site}.\n\
                #' @name x\nNULL\n";
    assert!(
        project_to_rd(href).contains(
            "(\\href (VERB \"http://x.org\") (GRP (\\emph (TEXT \"the\")) (TEXT \"site\")))"
        ),
        "{}",
        project_to_rd(href)
    );

    // `\tabular`: the format string and each cell are markdown, `\tab`/`\cr`
    // preserved; the multi-atom body wraps in `(GRP …)`.
    let tab = "#' @md\n#' @title T\n#' @details\n#' \\tabular{ll}{\n\
               #'   *a* \\tab **b** \\cr\n#' }\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(tab).contains(
            "(\\tabular (TEXT \"ll\") (GRP (\\emph (TEXT \"a\")) (\\tab) (\\strong (TEXT \"b\")) (\\cr)))"
        ),
        "{}",
        project_to_rd(tab)
    );
}

#[test]
fn md_structural_macro_arg_emphasis_spans_nested_macro() {
    // roxygen2 resolves a structural argument as **one** cmark run, so an
    // emphasis span crosses a nested Rd macro (the macro is opaque text to
    // cmark, reconstituted afterward). arity must do the same rather than
    // splitting the run at the macro and leaving the `*` delimiters literal.
    //
    // `\item{x}{*a \strong{y} b*}` → the `\emph` wraps the whole second
    // argument *including* the `\strong`, so the argument is a single atom
    // (no `(GRP …)`).
    let item = "#' @md\n#' @title T\n#' @details\n#' \\describe{\n\
                #'   \\item{x}{*a \\strong{y} b*}\n#' }\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(item).contains(
            "(\\item (TEXT \"x\") (\\emph (TEXT \"a\") (\\strong (TEXT \"y\")) (TEXT \"b\")))"
        ),
        "{}",
        project_to_rd(item)
    );

    // `\tabular`: an emphasis span even crosses a brace-less `\tab` separator
    // (cmark treats `\tab` as literal text). The `\emph` owns `a \tab b`, so
    // the body is `(GRP (\emph a \tab b) \cr)`.
    let tab = "#' @md\n#' @title T\n#' @details\n#' \\tabular{ll}{\n\
               #'   *a \\tab b* \\cr\n#' }\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(tab).contains(
            "(\\tabular (TEXT \"ll\") (GRP (\\emph (TEXT \"a\") (\\tab) (TEXT \"b\")) (\\cr)))"
        ),
        "{}",
        project_to_rd(tab)
    );
}

#[test]
fn md_emphasis_span_abuts_an_inline_macro() {
    // roxygen2 protects a fragile Rd tag as an alphanumeric placeholder before
    // cmark (`escape_rd_for_md`), so the macro flanks like a letter at its
    // leading edge — a `*` opener abutting the macro can open and the span
    // crosses it. `a*\code{x} y*` → `a` then `\emph{\code{x} y}`.
    let opens = "#' @md\n#' @title T\n#' @details a*\\code{x} y*\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(opens)
            .contains("(\\details (TEXT \"a\") (\\emph (\\code (RCODE \"x\")) (TEXT \"y\")))"),
        "{}",
        project_to_rd(opens)
    );

    // The placeholder ends in `-` (the `-<i>-` suffix), so a `*` closer abutting
    // the macro's trailing edge stays blocked — `a*\code{z}*b` keeps both `*`
    // literal (no emphasis), exactly as roxygen2 leaves it.
    let blocked = "#' @md\n#' @title T\n#' @details a*\\code{z}*b\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(blocked)
            .contains("(\\details (TEXT \"a*\") (\\code (RCODE \"z\")) (TEXT \"*b\"))"),
        "{}",
        project_to_rd(blocked)
    );
}

#[test]
fn md_macro_arg_resolution_is_off_without_md() {
    // Without `@md`, `*x*` is literal Rd prose inside the macro (no emphasis).
    let src = "#' @title T\n#' @details A \\emph{*x*} b.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\emph (TEXT \"*x*\"))"),
        "{}",
        project_to_rd(src)
    );
}

#[test]
fn md_link_display_with_active_markdown_macro_drops() {
    // A shortcut link whose display carries a macro with cmark-active markdown
    // (`\emph{*x*}`) is dropped ("markdown links must contain plain text"); the
    // surrounding prose coalesces. A macro with a literal arg (`\emph{x}`) keeps
    // the link, and a fragile `\code{*x*}` keeps it too (its body is protected).
    let drop = "#' @md\n#' @title T\n#' @details See [a\\emph{*x*}] here.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(drop).contains("(\\details (TEXT \"See here.\"))"),
        "{}",
        project_to_rd(drop)
    );

    let keep_plain = "#' @md\n#' @title T\n#' @details See [a\\emph{x}] here.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(keep_plain).contains("(\\link (TEXT \"a\") (\\emph (TEXT \"x\")))"),
        "{}",
        project_to_rd(keep_plain)
    );

    let keep_code = "#' @md\n#' @title T\n#' @details See [a\\code{*x*}] here.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(keep_code).contains("(\\link (TEXT \"a\") (\\code (RCODE \"*x*\")))"),
        "{}",
        project_to_rd(keep_code)
    );

    // Recursive: a nested non-fragile `\strong{*y*}` makes the display active.
    // (The display carries leading text `x`, so its truncated link-reference
    // label stays self-consistent — a *macro-only* display like `[\emph{…}]`
    // hits the empty-label demotion edge and is deferred to backlog.)
    let drop_nested = "#' @md\n#' @title T\n#' @details See [x \\emph{a \\strong{*y*}}] here.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(drop_nested).contains("(\\details (TEXT \"See here.\"))"),
        "{}",
        project_to_rd(drop_nested)
    );
}

#[test]
fn md_nested_fragile_macro_stays_literal() {
    // A fragile `\code` nested inside a non-fragile `\emph`: the outer arg is
    // markdown-processed, but `\code`'s own body stays literal (recursive
    // fragility check) — `(\emph (TEXT "a") (\code (RCODE "*x*")) (TEXT "b"))`.
    let src = "#' @md\n#' @title T\n#' @details A \\emph{a \\code{*x*} b} c.\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\emph (TEXT \"a\") (\\code (RCODE \"*x*\")) (TEXT \"b\"))"),
        "{}",
        project_to_rd(src)
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
fn inline_link_code_span_text_subrenders() {
    // roxygen2 renders the markdown *children* of a link, so a code-span link
    // text becomes `\verb`/`\code` (via `mdxml_code`) rather than literal
    // prose. An **inline** `[text](url)` carries that rendered span as its
    // `\href` text argument; a **reference** `[text][ref]` keeps the always-
    // `\code` wrap around the whole `\link` (the has-link-text branch).
    let src = "#' Title\n\
               #'\n\
               #' Description, see [`code link text`][func].\n\
               #' And also [`code as well`](https://external.com).\n\
               #' @md\n\
               foo <- function() {}\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"Description, see\") \
         (\\code (\\link (TEXT \"code link text\"))) (TEXT \". And also\") \
         (\\href (VERB \"https://external.com\") (\\verb (VERB \"code as well\"))) \
         (TEXT \".\"))\n\
         (\\title (TEXT \"Title\"))"
    );
}

#[test]
fn non_plain_shortcut_links_are_dropped() {
    // roxygen2's `parse_link` rejects a shortcut/reference link whose display is
    // not plain text ("markdown links must contain plain text") and renders it as
    // empty, leaving the surrounding prose contiguous: `[*foo*]` (emphasis) and
    // `` [`x` `y`] `` (two code spans) drop, while `[a_b]` (intraword `_` is not
    // emphasis) and `` [`code`] `` (a sole code span) survive.
    let src = "#' @details\n\
               #' A shortcut [*foo*] is dropped, but [a_b] and [`code`] survive \
               while [`x` `y`] drops too.\n\
               #' @md\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"A shortcut is dropped, but\") (\\link (TEXT \"a_b\")) \
         (TEXT \"and\") (\\code (\\link (TEXT \"code\"))) (TEXT \"survive while drops too.\"))"
    );
}

#[test]
fn non_plain_reference_links_are_dropped() {
    // The reference (`[text][ref]`) analog of the shortcut drop: a reference
    // whose synthesized `R:` destination links as `\link` requires plain-text
    // display, so `[*foo*][r1]` (emphasis) and `` [`x` `y`][r4] `` (two code
    // spans) drop, while `[plain][r2]` (plain) and `` [`code`][r3] `` (a sole
    // code span) survive. All reference displays are now carved onto the arena
    // (`same_line_bracket_opener`) as `ROXYGEN_MD_LINK` nodes, reaching the same
    // projection the opaque leaf used to.
    let src = "#' @details\n\
               #' A reference [*foo*][r1] is dropped, but [plain][r2] and \
               [`code`][r3] survive while [`x` `y`][r4] drops too.\n\
               #' @md\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"A reference is dropped, but\") (\\link (TEXT \"plain\")) \
         (TEXT \"and\") (\\code (\\link (TEXT \"code\"))) (TEXT \"survive while drops too.\"))"
    );
}

#[test]
fn link_display_droppable_boundary() {
    // A sole code span is unwrapped and allowed; pure text is allowed; anything
    // richer (emphasis, a second code span, an autolink) drops the link.
    assert!(!link_display_is_droppable(&[Inline::MdCode("x".into())]));
    assert!(!link_display_is_droppable(&[Inline::Text("a_b".into())]));
    assert!(link_display_is_droppable(&[Inline::MdEmphasis {
        strong: false,
        children: vec![Inline::Text("foo".into())],
    }]));
    assert!(link_display_is_droppable(&[
        Inline::MdCode("x".into()),
        Inline::Text(" ".into()),
        Inline::MdCode("y".into()),
    ]));
    assert!(link_display_is_droppable(&[Inline::MdLink(
        "<https://e.org>".into()
    )]));
}

#[test]
fn autolink_wins_over_bracket_carve() {
    // cmark scans inlines left-to-right at equal precedence: an autolink
    // whose span covers the `]` consumes it, so the bracket never closes and
    // the `[` stays literal (cm-528). The inline pass's whole-paragraph
    // rescan carves the autolink across the already-carved `](uri)` token.
    let src = "#' @details\n\
               #' [foo<https://example.com/?search=](uri)>\n\
               #' @md\n\
               #' @name x\n\
               NULL\n";
    let expected = "(\\details (TEXT \"[foo\") \
                    (\\url (VERB \"https://example.com/?search=](uri)\")))";
    assert_eq!(project_to_rd(src), expected);
    // The formatter folds the value onto the tag line; the whole-run rescan
    // must resolve the autolink identically in a same-line tag value (the
    // pure-Rust fixed-point analog for the curated corpus case).
    let folded = "#' @details [foo<https://example.com/?search=](uri)>\n\
                  #' @md\n\
                  #' @name x\n\
                  NULL\n";
    assert_eq!(project_to_rd(folded), expected);
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
fn underscore_leading_code_span_is_verb_not_code() {
    // R's lexer rejects any name beginning with `_` (rlang's `parse_expr`
    // errors), so roxygen2's `can_parse` is false and a `` `_` `` code span
    // renders `\verb`. arity's lexer is more lenient (it lexes `_` as an
    // ordinary identifier), so `code_span_is_r` must screen these out.
    assert!(!code_span_is_r("_"));
    assert!(!code_span_is_r("_x"));
    assert!(!code_span_is_r("_foo_"));
    // A lone `_` stays valid as the native-pipe placeholder.
    assert!(code_span_is_r("x |> _$col"));
    // Ordinary names with a non-leading underscore are unaffected.
    assert!(code_span_is_r("a_b"));
}

#[test]
fn empty_backquoted_name_code_span_is_verb_not_code() {
    // R's parser rejects a zero-length backquoted name (`parse(text = "``")`
    // errors "attempt to use zero-length variable name"), so roxygen2's
    // `can_parse` is false and a `` ` `` ` `` code span renders `\verb`
    // (cm-332/333). arity lexes `` `` `` as an ordinary IDENT, so
    // `code_span_is_r` must screen it out.
    assert!(!code_span_is_r("``"));
    assert!(!code_span_is_r(" `` "));
    // A non-empty backquoted name is a valid R name.
    assert!(code_span_is_r("`x`"));
    assert!(code_span_is_r("` `"));
}

#[test]
fn thematic_break_line_never_opens_a_setext_title() {
    // `***` then `---`: block structure wins over paragraph text in
    // CommonMark, so a thematic-break line never becomes a setext title —
    // the `---` heads nothing and all three lines are breaks, which
    // roxygen2 renders empty (cm-043).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' ***\n\
               #' ---\n\
               #' ___\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details)\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn over_indented_thematic_break_folds_into_the_open_paragraph() {
    // `Foo` then `    ***`: content at column five is indented-code
    // territory, not a thematic break — and indented code cannot interrupt
    // a paragraph, so the line lazily folds as prose (cm-049).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' Foo\n\
               #'     ***\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"Foo ***\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn over_indented_setext_underline_folds_into_the_open_paragraph() {
    // `Foo` then `    ---`: a setext underline is subject to CommonMark's
    // three-space indent allowance — at column five it is indented-code
    // territory, which cannot interrupt a paragraph, so the line lazily
    // folds as prose and no heading forms (cm-087).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' Foo\n\
               #'     ---\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"Foo ---\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn trailing_backslash_heading_title_drops_the_piece() {
    // `Foo\` promoted by `----`: the title's trailing backslash escapes
    // the rendered `\subsection{Foo\}` closing brace, so `rdComplete`
    // fails and roxygen2 empties the whole enclosing piece (cm-090).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' Foo\\\n\
               #' ----\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details)\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn incomplete_section_piece_keeps_its_title() {
    // The `rdComplete` drop is per level-1 piece: the intro piece is
    // complete and survives; the `# Good` section's piece holds the
    // brace-incomplete `\subsection{Bad\}`, so its body empties while its
    // title (part of roxygen2's split marker) survives (engine-probed).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' Intro\n\
               #'\n\
               #' # Good\n\
               #' good body\n\
               #'\n\
               #' Bad\\\n\
               #' ----\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"Intro\"))\n\
         (\\section (TEXT \"Good\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn same_line_item_thematic_break_drops_but_keeps_the_item() {
    // `- * * *`: a thematic break at the item's content start (cm-061)
    // renders empty in roxygen2, leaving the `\item` bare.
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' - Foo\n\
               #' - * * *\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (\\itemize (\\item) (TEXT \"Foo\") (\\item)))\n\
         (\\title (TEXT \"T\"))"
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
fn slot_with_unbalanced_brace_is_dropped() {
    // roxygen2 parses `@slot` with `tag_two_part`, which runs
    // `rdComplete(x$raw, is_code = FALSE)` on the *raw* tag value and drops the
    // whole tag on a brace imbalance (mode-independent). Only the balanced slot
    // survives the aggregated Slots section.
    let src = "#' Important class.\n\
               #'\n\
               #' @slot a sl{ot a\n\
               #' @slot b slot b\n\
               setClass('test')\n";
    let out = project_to_rd(src);
    assert!(
        out.contains(
            "(\\section (TEXT \"Slots\") (\\describe \
             (\\item (\\code (RCODE \"b\")) (TEXT \"slot b\"))))"
        ),
        "got: {out}"
    );
    assert!(!out.contains("slot a"), "dropped slot leaked: {out}");
}

#[test]
fn all_fields_unbalanced_drops_fields_section() {
    // When every `@field` is brace-incomplete, all drop and roxygen2 emits no
    // Fields section at all (the aggregating field is empty).
    let src = "#' Important class.\n\
               #'\n\
               #' @field a fi{eld a\n\
               setRefClass('test')\n";
    let out = project_to_rd(src);
    assert!(
        !out.contains("Fields"),
        "Fields section should be absent: {out}"
    );
}

#[test]
fn slot_with_percent_commented_brace_survives() {
    // `rdComplete` runs on the *raw* value where `%` is a line comment, so an
    // unbalanced `{` after a `%` is commented out and the slot survives.
    let src = "#' Important class.\n\
               #'\n\
               #' @slot a desc %{\n\
               setClass('test')\n";
    let out = project_to_rd(src);
    assert!(out.contains("Slots"), "Slots section should survive: {out}");
}

#[test]
fn section_with_unbalanced_brace_drops_to_na_md_off() {
    // markdown-OFF: `markdown_if_active`'s else-branch runs `rdComplete(x$raw)`
    // unconditionally on the whole `@section` value; a brace imbalance replaces
    // it with "". `roxy_tag_rd` then splits "" on ":" → title="", content=NA →
    // `\section{}{NA}` → `(\section (TEXT "NA"))`.
    let src = "#' @title T\n\
               #' @section Heading:\n\
               #'   body with brace {\n\
               #' @name x\n\
               NULL\n";
    let out = project_to_rd(src);
    assert!(out.contains("(\\section (TEXT \"NA\"))"), "got: {out}");
    assert!(!out.contains("Heading"), "dropped title leaked: {out}");
}

#[test]
fn section_with_percent_commented_brace_survives_md_off() {
    // The raw `rdComplete` treats `%` as a line comment, so a `{` after a `%` is
    // commented out and the section renders normally (not dropped to NA).
    let src = "#' @title T\n\
               #' @section Heading:\n\
               #'   body %{\n\
               #' @name x\n\
               NULL\n";
    let out = project_to_rd(src);
    assert!(
        out.contains("(\\section (TEXT \"Heading\") (TEXT \"body\"))"),
        "got: {out}"
    );
}

#[test]
fn section_unbalanced_brace_not_dropped_md_on() {
    // markdown-ON: `@section` uses `tag_markdown` with `sections = FALSE`, so the
    // per-section `rdComplete` drop never fires — the body is not replaced by NA
    // (roxygen2 emits the imbalanced content as-is).
    let src = "#' @md\n\
               #' @title T\n\
               #' @section Heading:\n\
               #'   body with brace {\n\
               #' @name x\n\
               NULL\n";
    let out = project_to_rd(src);
    assert!(
        !out.contains("(\\section (TEXT \"NA\"))"),
        "md-on @section must not drop to NA: {out}"
    );
    assert!(out.contains("Heading"), "title should survive: {out}");
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
        project_to_rd(src).contains("(\\details (TEXT \"Text with *emphasis* and `code` here.\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn norm_ws_collapses_ascii_but_preserves_unicode_whitespace() {
    // ASCII whitespace runs collapse to a single space and the ends trim.
    assert_eq!(norm_ws("  a \t\n b  "), "a b");
    // Non-ASCII Unicode whitespace (NBSP, NEL) is preserved verbatim --- the
    // R driver's `[[:space:]]` is ASCII-only even in a UTF-8 locale.
    assert_eq!(norm_ws("*\u{a0}a\u{a0}*"), "*\u{a0}a\u{a0}*");
    assert_eq!(norm_ws("x\u{85}y"), "x\u{85}y");
}

#[test]
fn nbsp_cannot_flank_emphasis_stays_literal() {
    // A NBSP is Unicode whitespace, so the `*`s around `\u{a0}a\u{a0}` cannot
    // flank --- no `\emph`, the literal text (NBSP intact) survives. (cm-355)
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' *\u{a0}a\u{a0}*\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"*\u{a0}a\u{a0}*\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn field_edge_unicode_whitespace_trims() {
    // roxygen2 trims the rendered field with stringr's `str_trim` (the
    // Unicode White_Space set --- `mdxml_children_to_rd_top`, R/markdown.R),
    // so an entity-decoded NBSP at either field edge vanishes while an
    // interior one survives. (cm-025)
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' &nbsp; a&nbsp;b &nbsp;\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"a\u{a0}b\"))"),
        "got: {}",
        project_to_rd(src)
    );
    // Non-md: `tag_value` runs the same `str_trim` on the raw value, so a
    // literal NBSP at the field edge trims there too.
    let src = "#' @title T\n\
               #' @details\n\
               #' \u{a0} lead\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"lead\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn section_piece_edges_trim_but_subsection_interior_survives() {
    // A level-1 heading emits only the split marker (no braces), so the
    // `\section` body is a whole piece and `str_trim(secs)` trims both its
    // edges; a `\subsection` body sits inside literal `{`...`}` in the
    // rendered string --- interior, never trimmed. (engine-probed)
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' intro &nbsp;\n\
               #'\n\
               #' # Head\n\
               #' &nbsp; secbody &nbsp;\n\
               #' @name spec\n\
               NULL\n";
    let out = project_to_rd(src);
    assert!(out.contains("(\\details (TEXT \"intro\"))"), "got: {out}");
    assert!(
        out.contains("(\\section (TEXT \"Head\") (TEXT \"secbody\"))"),
        "got: {out}"
    );
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' ## Sub\n\
               #' &nbsp; subbody &nbsp;\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\details (\\subsection (TEXT \"Sub\") (TEXT \"\u{a0} subbody \u{a0}\")))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn section_tag_value_trims_as_one_string() {
    // `@section Title: content` markdown-processes and `str_trim`s the
    // *whole* value before the `:` split, so the title carries the field's
    // leading edge and the content its trailing edge; the edges at the
    // split are interior and keep their whitespace. (engine-probed)
    let src = "#' @md\n\
               #' @title T\n\
               #' @section &nbsp;Head&nbsp;:\n\
               #' &nbsp; content &nbsp;\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains("(\\section (TEXT \"Head\u{a0}\") (TEXT \"\u{a0} content\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn fence_info_string_decodes_entities() {
    // cmark entity-decodes a fence's info string, so the `sourceCode` div
    // class carries the decoded text. (cm-034)
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' ``` f&ouml;&ouml;\n\
               #' foo\n\
               #' ```\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains("sourceCode f\u{f6}\u{f6}"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn raw_fence_info_percent_drops_the_section() {
    // roxygen2 pastes the fence info string RAW into the `sourceCode` div class
    // (`mdxml_code_block` — only the body is `escape_verb`-escaped), so a `%` in
    // the info comments out the rest of the rendered line (`">}}\preformatted{`)
    // and `rdComplete` fails → the whole section drops. (cm-143)
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' ~~~~    ruby startline=3 $%@#$\n\
               #' def foo(x)\n\
               #'   return 3\n\
               #' end\n\
               #' ~~~~~~~\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains("(\\details)"),
        "got: {}",
        project_to_rd(src)
    );
    // An unbalanced brace in the raw info drops the same way; a balanced pair
    // and a plain info string keep the section.
    let with_info = |info: &str| {
        format!(
            "#' @md\n#' @title T\n#' @details\n#' ``` {info}\n#' foo\n#' ```\n#' @name spec\nNULL\n"
        )
    };
    assert!(project_to_rd(&with_info("a{b")).contains("(\\details)"));
    assert!(project_to_rd(&with_info("a{b}c")).contains("\\preformatted"));
    assert!(project_to_rd(&with_info("ruby")).contains("\\preformatted"));
}

#[test]
fn unescape_md_brackets_consumes_one_backslash_before_a_bracket() {
    // `\[`/`\]` lose exactly one backslash; a deeper run keeps the rest.
    assert_eq!(unescape_md_brackets(r"\[x\]"), "[x]");
    assert_eq!(unescape_md_brackets(r"\\[x"), r"\[x");
    // Other escapes are untouched (only brackets are special in roxygen2).
    assert_eq!(
        unescape_md_brackets(r"foo \* \` \% bar"),
        r"foo \* \` \% bar"
    );
    // A backslash not adjacent to a bracket (e.g. at a line break) is kept.
    assert_eq!(unescape_md_brackets("a\\\n[b"), "a\\\n[b");
}

#[test]
fn collapse_md_backslash_runs_halves_a_run() {
    // A run of `k` source backslashes renders as `ceil(k/2)` (double_escape
    // doubles, cmark and parse_Rd each collapse pairs): `\\` → `\`,
    // `\\\\` → `\\`, but a lone `\` (`\*`, `\_`, …) is unchanged.
    assert_eq!(collapse_md_backslash_runs(r"a \ b"), r"a \ b");
    assert_eq!(collapse_md_backslash_runs(r"a \\ b"), r"a \ b");
    assert_eq!(collapse_md_backslash_runs(r"a \\\\ b"), r"a \\ b");
    assert_eq!(collapse_md_backslash_runs(r"a \\\\\\ b"), r"a \\\ b");
    assert_eq!(collapse_md_backslash_runs(r"\* \_ \%"), r"\* \_ \%");
    // A run abutting a bracket is left verbatim for `unescape_md_brackets`.
    assert_eq!(collapse_md_backslash_runs(r"\\[x"), r"\\[x");
    assert_eq!(collapse_md_backslash_runs(r"a\\]b"), r"a\\]b");
}

#[test]
fn odd_backslash_run_demotes_a_following_md_macro() {
    // `\*x*` under `@md` (cm-014): `double_escape_md` doubles the `\`, so cmark
    // sees `\\*` — a literal `\` and an *active* star. The rendered field is then
    // `\` + `\emph{x}`, whose `\\` parse_Rd pairs left-to-right: the macro's own
    // backslash is consumed, the name is absorbed into the TEXT, and the braced
    // argument re-parses as a bare LIST group.
    let emph = "#' @md\n\
                #' @title T\n\
                #' @details\n\
                #' \\*not emphasized*\n\
                #' @name x\n\
                NULL\n";
    assert_eq!(
        project_to_rd(emph),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"\\\\emph\") (LIST (TEXT \"not emphasized\")))\n\
         (\\title (TEXT \"T\"))"
    );
    // Same collision for a code span's `\verb` (its verbatim-ness is lost — the
    // group re-parses as plain text) ...
    let code = "#' @md\n\
                #' @title T\n\
                #' @details\n\
                #' \\`not code`\n\
                #' @name x\n\
                NULL\n";
    assert_eq!(
        project_to_rd(code),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"\\\\verb\") (LIST (TEXT \"not code\")))\n\
         (\\title (TEXT \"T\"))"
    );
    // ... and for inline HTML's `\if{html}{\out{…}}`: both args become LISTs;
    // the `\out` inside the second still parses (parse_Rd knows it anywhere).
    let html = "#' @md\n\
                #' @title T\n\
                #' @details\n\
                #' \\<br/> not a tag\n\
                #' @name x\n\
                NULL\n";
    assert_eq!(
        project_to_rd(html),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"\\\\if\") (LIST (TEXT \"html\")) (LIST (\\out (VERB \"<br/>\"))) \
         (TEXT \"not a tag\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn even_backslash_run_keeps_the_following_md_macro() {
    // An even source run pairs away entirely before the macro's backslash, so
    // the macro survives (`x\\*y*` → text `x\` + `\emph{y}`); an odd run of 3
    // demotes with two literal backslashes left in the text (engine-probed).
    let even = "#' @md\n\
                #' @title T\n\
                #' @details\n\
                #' x\\\\*y* z\n\
                #' @name x\n\
                NULL\n";
    assert_eq!(
        project_to_rd(even),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"x\\\\\") (\\emph (TEXT \"y\")) (TEXT \"z\"))\n\
         (\\title (TEXT \"T\"))"
    );
    let triple = "#' @md\n\
                  #' @title T\n\
                  #' @details\n\
                  #' x\\\\\\*y* z\n\
                  #' @name x\n\
                  NULL\n";
    assert_eq!(
        project_to_rd(triple),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"x\\\\\\\\emph\") (LIST (TEXT \"y\")) (TEXT \"z\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn braceless_item_projects_as_unknown_node() {
    // A brace-less `\item` outside a list is parse_Rd's out-of-list recovery:
    // an `(UNKNOWN "\item")` node splitting the surrounding text (mode-
    // independent). It can start, sit mid-prose, or end a line; an escaped
    // `\\item` stays literal; two items on a line split twice.
    let src = "#' T\n\
               #' @details a \\item b. c \\item. d \\\\item e. f \\item g \\item h.\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"a\") (UNKNOWN \"\\\\item\") (TEXT \"b. c\") (UNKNOWN \"\\\\item\") \
         (TEXT \". d \\\\item e. f\") (UNKNOWN \"\\\\item\") (TEXT \"g\") (UNKNOWN \"\\\\item\") \
         (TEXT \"h.\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn braceless_sticky_swallows_tail_per_line() {
    // A brace-less `\code`/`\verb` leaves parse_Rd in R-code/verbatim mode: the
    // dropped `\name` and everything after it, to section end, becomes one
    // `RCODE`/`VERB` atom per physical source line. The prose *before* the
    // trigger stays ordinary text. Non-`@md`: a continuation line keeps all but
    // the one `#'`-marker space.
    let src = "#' T\n\
               #' @details a \\code z here\n\
               #'   cont line.\n\
               #' @seealso b \\verb c d.\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"a\") (RCODE \" z here\\n\") (RCODE \"  cont line.\\n\"))\n\
         (\\seealso (TEXT \"b\") (VERB \" c d.\\n\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn braceless_sticky_md_strips_continuation_indent() {
    // Under `@md`, cmark strips a continuation line's remaining leading
    // whitespace before the swallow captures it (`#'   cont` → flush `cont`),
    // unlike non-`@md` (two spaces survive above).
    let src = "#' T\n\
               #' @md\n\
               #' @details a \\code z here\n\
               #'   cont line.\n\
               #' @name x\n\
               NULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"a\") (RCODE \" z here\\n\") (RCODE \"cont line.\\n\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn braceless_sticky_withholds_impure_tail() {
    // A tail carrying a real macro node still parses inside the swallow (it
    // splits the RCODE), and a bare `{`/`}` or `%` breaks the section or acts
    // as a comment — none are modeled at section granularity, so the swallow is
    // withheld and the `\code` stays literal prose (its prior projection).
    for tail in [
        "\\emph{x} after", // a macro node in the tail
        "{grp} after",     // a bare brace group
        "50% done",        // an Rd `%` comment
    ] {
        let src = format!("#' T\n#' @details a \\code z {tail}\n#' @name x\nNULL\n",);
        let out = project_to_rd(&src);
        assert!(
            !out.contains("(RCODE"),
            "expected withhold (no swallow) for tail {tail:?}, got: {out}"
        );
    }
}

#[test]
fn braceless_drop_macro_vanishes_from_text() {
    // parse_Rd's drop-recovery: an unpaired `\` re-forming a brace-required
    // known macro without its `{` drops the `\name`; the text continues.
    assert_eq!(
        resolve_rd_text_escapes(r"before \emph z after"),
        "before  z after"
    );
    // The drop applies at end of input (the `{` never arrives) …
    assert_eq!(
        resolve_rd_text_escapes(r"end of line \strong"),
        "end of line "
    );
    // … and to section-header names misused mid-prose.
    assert_eq!(resolve_rd_text_escapes(r"a \title z"), "a  z");
    // An even run is a literal backslash, not a macro: no drop.
    assert_eq!(resolve_rd_text_escapes(r"a \\emph z"), r"a \emph z");
    // Sticky names (code/verbatim mode-flip, `\item`) stay literal (backlog).
    assert_eq!(resolve_rd_text_escapes(r"a \code z"), r"a \code z");
    assert_eq!(resolve_rd_text_escapes(r"a \item z"), r"a \item z");
    // The `@md` collapse mirrors the drop, keyed on the original run parity:
    // odd runs drop the name and keep the paired `k/2` backslashes.
    assert_eq!(collapse_md_backslash_runs(r"a \emph z"), "a  z");
    assert_eq!(collapse_md_backslash_runs(r"a \\\link q"), r"a \ q");
    assert_eq!(collapse_md_backslash_runs(r"a \\emph z"), r"a \emph z");
    assert_eq!(collapse_md_backslash_runs(r"a \code z"), r"a \code z");
}

#[test]
fn md_percent_swallow_is_parity_keyed() {
    // A bare `%` (even run, k=0) stays literal.
    assert_eq!(md_percent_swallow("a % b"), "a % b");
    // A lone `\%` (odd) comments to end of line; the escaping `\` is kept
    // (later halved to `ceil(1/2) == 1` by collapse_md_backslash_runs).
    assert_eq!(md_percent_swallow(r"a \% b"), "a \\");
    // `\\%` (even) survives literal; `\\\%` (odd) swallows, keeping 3 `\`.
    assert_eq!(md_percent_swallow(r"a \\% b"), r"a \\% b");
    assert_eq!(md_percent_swallow(r"a \\\% b"), "a \\\\\\");
    // The first odd `%` wins even when a bare `%` precedes it on the line.
    assert_eq!(md_percent_swallow(r"a % b \% c"), "a % b \\");
    // Line-scoped: a continuation on the next physical line survives.
    assert_eq!(md_percent_swallow("a \\% b\nc"), "a \\\nc");
    // The physical line ends at a soft-wrap (SOFT_BREAK) too, not just a
    // paragraph break: the continuation on the next `#'` line survives.
    assert_eq!(
        md_percent_swallow(&format!("a \\% b{SOFT_BREAK}c")),
        "a \\\nc"
    );
}

#[test]
fn strip_rd_comments_stops_at_soft_wrap() {
    // A non-`@md` `%` comment ends at the physical source line. Both a
    // paragraph break (`\n`) and a soft-wrap (SOFT_BREAK) end the line, so a
    // continuation on the next `#'` line survives the comment either way.
    assert_eq!(strip_rd_comments("a % swallowed\nc"), "a \nc");
    assert_eq!(
        strip_rd_comments(&format!("a % swallowed{SOFT_BREAK}c")),
        "a \nc"
    );
}

#[test]
fn md_escaped_bracket_is_literal_with_the_backslash_consumed() {
    // Under `@md`, an escaped `\[` neither opens a link nor keeps its
    // backslash: roxygen2 renders `\[text](url)` as the literal `[text](url)`
    // (the `double_escape_md` bracket revert + cmark escape). The lexer
    // suppresses the link; the projector drops the backslash.
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' A \\[bracket](x) and \\[shortcut] stay literal.\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\details (TEXT \"A [bracket](x) and [shortcut] stay literal.\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn shortcut_link_node_atom_resolves_text_and_code() {
    // A plain-text display is the destination: `\link{text}` (text coalesced).
    assert_eq!(
        shortcut_link_node_atom(&[Inline::Text("cross-line shortcut".to_string())]),
        "(\\link (TEXT \"cross-line shortcut\"))"
    );
    // A single code-span display is `\code`-wrapped, mirroring `shortcut_link_atom`.
    assert_eq!(
        shortcut_link_node_atom(&[Inline::MdCode("f".to_string())]),
        "(\\code (\\link (TEXT \"f\")))"
    );
}

#[test]
fn md_cross_line_shortcut_link_joins_into_one_link() {
    // Under `@md`, a shortcut link `[text]` whose `[` opens on an earlier `#'`
    // line resolves into one `\link{text}` over the coalesced text; a stray `]`
    // with no opener stays literal (matching roxygen2).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' A [broken\n\
               #' across lines] joins, but a stray a] stays.\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"A\") (\\link (TEXT \"broken across lines\")) \
             (TEXT \"joins, but a stray a] stays.\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn double_escape_md_reverts_only_bracket_escapes() {
    // Every backslash is doubled, then the two bracket escapes are reverted —
    // so a bracket escape survives unchanged, every other escape is neutralized.
    assert_eq!(double_escape_md("[text\\]"), "[text\\]");
    assert_eq!(double_escape_md("a\\*b"), "a\\\\*b");
    assert_eq!(double_escape_md("\\[x\\]"), "\\[x\\]");
    // Two source backslashes before `]` become three (2*2 then revert one pair).
    assert_eq!(double_escape_md("[text\\\\]"), "[text\\\\\\]");
}

#[test]
fn url_encode_matches_r_urlencode() {
    // Alphanumerics and the unreserved/sub-delim set pass through; everything
    // else is percent-encoded uppercase (`\`→%5C, space→%20, `%`→%25).
    assert_eq!(url_encode("text\\"), "text%5C");
    assert_eq!(url_encode("a b"), "a%20b");
    assert_eq!(url_encode("a/b:c"), "a/b:c");
    assert_eq!(url_encode("100%"), "100%25");
}

#[test]
fn md_linkref_labels_ports_get_md_linkrefs() {
    // A bare shortcut; the second `[ref]` group wins as the label.
    assert_eq!(md_linkref_labels("see [foo] now"), vec!["foo".to_string()]);
    assert_eq!(md_linkref_labels("[text][ref]"), vec!["ref".to_string()]);
    // Lookbehind: a `[` preceded by `\` (an escaped-open bracket) is no match.
    assert!(md_linkref_labels("\\[foo]").is_empty());
    // Lookahead: a `[…]` immediately followed by `[` or `{` is no match.
    assert!(md_linkref_labels("[a]{x}").is_empty());
    // The escaped-close shortcut still matches (its `]` closes the content).
    assert_eq!(md_linkref_labels("[text\\]"), vec!["text\\".to_string()]);
}

#[test]
fn linkref_label_closes_on_even_trailing_backslashes() {
    assert!(linkref_label_closes("text")); // 0 trailing — valid definition
    assert!(!linkref_label_closes("text\\")); // 1 trailing — `]` escaped, leaks
    assert!(!linkref_label_closes("text\\\\\\")); // 3 trailing — leaks
    assert!(linkref_label_closes("text\\\\")); // 2 trailing — `]` not escaped
}

#[test]
fn leaked_linkref_text_leaks_from_first_invalid_definition() {
    // An escaped-close shortcut leaks its synthesized definition; a valid
    // shortcut before any invalid one does not (roxygen2 links it). Lines
    // come back at the cmark stage (`\]` still escaped) — the final
    // unescape happens in `append_leaked_defs`' fragment resolution.
    assert_eq!(
        leaked_linkref_text("see [text\\] here"),
        vec!["[text\\]: R:text%5C".to_string()]
    );
    assert!(leaked_linkref_text("see [foo] here").is_empty());
    // Multiple escaped-close candidates each leak (all-invalid block).
    assert_eq!(
        leaked_linkref_text("a [one\\] b [two\\] c"),
        vec![
            "[one\\]: R:one%5C".to_string(),
            "[two\\]: R:two%5C".to_string()
        ]
    );
    // An escaped-open `\[…]` is excluded by the lookbehind — no leak.
    assert!(leaked_linkref_text("an escaped \\[x\\] stays").is_empty());
    // Poisoning: the first invalid definition swallows the rest of the block, so
    // a *valid* candidate after it leaks too (and is de-linked elsewhere).
    assert_eq!(
        leaked_linkref_text("a [one] b [two\\] c [three] d"),
        vec![
            "[two\\]: R:two%5C".to_string(),
            "[three]: R:three".to_string()
        ]
    );
}

#[test]
fn blank_line_label_never_defines_and_leaks() {
    // A label spanning a blank line cannot close its definition — a link
    // reference definition is a paragraph-level construct, and the paragraph
    // ends at the blank line — so the whole synthesized def leaks (cm-184).
    // Line endings arrive as real `\n`s or as the skeleton's soft-break
    // sentinel, depending on the stage; both count.
    assert_eq!(
        leaked_linkref_text("see [a\n\nb] here"),
        vec!["[a\n\nb]: R:a%0A%0Ab".to_string()]
    );
    assert_eq!(
        leaked_linkref_text(&format!("see [a{SOFT_BREAK}{SOFT_BREAK}b] here")),
        vec!["[a\n\nb]: R:a%0A%0Ab".to_string()]
    );
    // A soft-wrapped (single line ending) label still defines — no leak.
    assert!(leaked_linkref_text("see [a\nb] here").is_empty());
}

#[test]
fn html_block_lines_are_field_text_to_the_candidate_scan() {
    // roxygen2's `get_md_linkrefs` scans the whole raw field text, markup
    // included, so a bracket-free `[…]` inside an HTML block is a candidate —
    // the skeleton must surface the block's lines (a single space would hide
    // them, and no leak would ever fire from inside a block).
    let src = "#' @md\n#' @details\n#' <!-- [text\\] -->\n#' @name a\nNULL\n";
    let cst = crate::parser::parse(src).cst;
    let node = cst
        .descendants()
        .find(|n| n.kind() == SyntaxKind::ROXYGEN_MD_HTML_BLOCK)
        .expect("html block node");
    let body = vec![Inline::MdHtmlBlock(node)];
    assert_eq!(
        leaked_linkref_text(&leak_source_skeleton(&body)),
        vec!["[text\\]: R:text%5C".to_string()]
    );
}

#[test]
fn blank_line_leak_reparses_as_markdown_blocks() {
    // cm-184: the CDATA body's second bracket opens a candidate whose label
    // spans blank lines, so its synthesized definition leaks whole — and cmark
    // parses the leaked lines as *blocks* in the document. The leak's leading
    // lines lazily gather into the trailing `okay` paragraph, the post-blank
    // 4-column `return 0;` is indented code, and parse_Rd nests the rendered
    // bare braces as `LIST` groups spanning those blocks.
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' <![CDATA[\n\
               #' function matchwo(a,b)\n\
               #' {\n\
               #'   if (a < b && a < 0) then {\n\
               #'     return 1;\n\
               #'\n\
               #'   } else {\n\
               #'\n\
               #'     return 0;\n\
               #'   }\n\
               #' }\n\
               #' ]]>\n\
               #' okay\n\
               #' @name spec\n\
               NULL\n";
    let out = project_to_rd(src);
    // The lazy paragraph glues onto `okay`; the brace groups nest as LISTs.
    assert!(
        out.contains(
            "(TEXT \"okay [ function matchwo(a,b)\") \
             (LIST (TEXT \"if (a < b && a < 0) then\") (LIST (TEXT \"return 1;\"))"
        ),
        "got: {out}"
    );
    // The post-blank indented code renders `\preformatted` inside its group.
    assert!(
        out.contains("(TEXT \"else\") (LIST (\\if (TEXT \"html\")"),
        "got: {out}"
    );
    assert!(
        out.contains("(\\preformatted (VERB \"return 0;\\n\"))"),
        "got: {out}"
    );
    // The def tail's URL encodes the label's newlines and blank lines.
    assert!(
        out.contains("(TEXT \"]: R:%0Afunction%20matchwo(a,b)%0A"),
        "got: {out}"
    );
}

#[test]
fn first_invalid_linkref_offset_finds_the_poison_bracket() {
    // The opening `[` of the first escaped-close candidate (`[two\]` at index 10).
    assert_eq!(
        first_invalid_linkref_offset("a [one] b [two\\] c"),
        Some(10)
    );
    // All candidates close → no poisoning.
    assert_eq!(first_invalid_linkref_offset("[foo] [bar]"), None);
    // A leading escaped-close candidate poisons from the start.
    assert_eq!(first_invalid_linkref_offset("[bad\\] tail"), Some(0));
}

#[test]
fn demoted_link_source_targets_only_definition_backed_links() {
    // Shortcut/reference links lose their (now-leaked) definition → literal text.
    assert_eq!(
        demoted_link_source(&Inline::MdShortcutLink {
            display: vec![Inline::Text("foo".to_string())]
        }),
        Some("[foo]".to_string())
    );
    assert_eq!(
        demoted_link_source(&Inline::MdRefLink {
            dest: "ref".to_string(),
            display: vec![Inline::Text("disp".to_string())]
        }),
        Some("[disp][ref]".to_string())
    );
    assert_eq!(
        demoted_link_source(&Inline::MdLink("[foo]".to_string())),
        Some("[foo]".to_string())
    );
    assert_eq!(
        demoted_link_source(&Inline::MdLink("[t][r]".to_string())),
        Some("[t][r]".to_string())
    );
    // Inline links and autolinks carry their own destination → survive.
    assert_eq!(
        demoted_link_source(&Inline::MdLink("[t](u)".to_string())),
        None
    );
    assert_eq!(
        demoted_link_source(&Inline::MdLink("<http://x>".to_string())),
        None
    );
    assert_eq!(
        demoted_link_source(&Inline::Text("plain".to_string())),
        None
    );
}

#[test]
fn skeleton_exposes_inline_link_brackets_for_leaked_defs() {
    // roxygen2's `get_md_linkrefs` synthesizes a `[text]: R:text` definition for
    // an inline `[text](url)` link too, so the skeleton must surface its `[text]`
    // as a candidate (a single space would hide it). The link itself survives.
    let link = Inline::MdInlineLink {
        url: "https://example.org".to_string(),
        display: vec![Inline::Text("after".to_string())],
    };
    assert_eq!(inline_skeleton_fragment(&link), "[after] ");
    // `skeleton_len` must agree with the fragment, or the boundary offset mapping
    // in `demote_poisoned_links` drifts.
    assert_eq!(skeleton_len(&link), "[after] ".len());
    // An escaped-close candidate poisons the tail; the surviving inline link's
    // definition leaks alongside it.
    let body = vec![Inline::Text("see [stop\\] then ".to_string()), link];
    assert_eq!(
        leaked_linkref_text(&inline_source_skeleton(&body)),
        vec![
            "[stop\\]: R:stop%5C".to_string(),
            "[after]: R:after".to_string(),
        ]
    );
    // Without a poison boundary nothing leaks (the def is consumed, not leaked).
    let clean = vec![Inline::MdInlineLink {
        url: "u".to_string(),
        display: vec![Inline::Text("x".to_string())],
    }];
    assert!(leaked_linkref_text(&inline_source_skeleton(&clean)).is_empty());
}

#[test]
fn skeleton_exposes_image_alt_for_leaked_defs() {
    // An image `![alt](url)`'s `[alt]` is a bracket-free candidate too, so the
    // skeleton must surface it (a single space would hide it). The `\figure`
    // survives; only its synthesized `[alt]: R:alt` definition leaks.
    let image = Inline::MdImage("![alt](https://example.org/x.png)".to_string());
    assert_eq!(image_alt_text("![alt](u)"), Some("alt"));
    assert_eq!(inline_skeleton_fragment(&image), "[alt] ");
    assert_eq!(skeleton_len(&image), "[alt] ".len());
    // The image survives poisoning (carries its own destination), never demoted.
    assert_eq!(demoted_link_source(&image), None);
    // An escaped-close candidate poisons the tail; the surviving image's
    // definition leaks alongside it.
    let body = vec![Inline::Text("see [stop\\] then ".to_string()), image];
    assert_eq!(
        leaked_linkref_text(&inline_source_skeleton(&body)),
        vec![
            "[stop\\]: R:stop%5C".to_string(),
            "[alt]: R:alt".to_string()
        ]
    );
}

#[test]
fn skeleton_exposes_opaque_inline_link_inner_bracket_for_leaked_defs() {
    // A nested-bracket display keeps the inline link opaque (the lexer only
    // nodes a bracket-free display), yet `get_md_linkrefs` still finds the
    // *inner* `[b]` candidate (the outer `[a [b] c]` is not a candidate — its
    // content has brackets). The skeleton must surface the display verbatim.
    let link = Inline::MdLink("[a [b] c](https://example.org)".to_string());
    assert_eq!(
        opaque_inline_link_display("[a [b] c](https://example.org)"),
        Some("a [b] c")
    );
    // A shortcut/reference leaf has no `(` after the display; an autolink opens
    // with `<` — neither is an inline-link display.
    assert_eq!(opaque_inline_link_display("[shortcut]"), None);
    assert_eq!(opaque_inline_link_display("[text][ref]"), None);
    assert_eq!(opaque_inline_link_display("<https://example.org>"), None);
    assert_eq!(inline_skeleton_fragment(&link), "[a [b] c] ");
    assert_eq!(skeleton_len(&link), "[a [b] c] ".len());
    // The inline link survives poisoning (carries its own destination).
    assert_eq!(demoted_link_source(&link), None);
    // An escaped-close candidate poisons the tail; the surviving link's inner
    // `[b]` definition leaks alongside it.
    let body = vec![Inline::Text("see [stop\\] then ".to_string()), link];
    assert_eq!(
        leaked_linkref_text(&inline_source_skeleton(&body)),
        vec!["[stop\\]: R:stop%5C".to_string(), "[b]: R:b".to_string()]
    );
}

#[test]
fn projects_mixed_linkref_poisoning() {
    // The end-to-end mixed case: a valid shortcut before the escaped-close
    // candidate links; the escaped-close poisons the appended definition block,
    // so a later shortcut is de-linked into literal text and *both* trailing
    // definitions leak.
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' See [before] then [stop\\] and [after].\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"See\") (\\link (TEXT \"before\")) \
             (TEXT \"then [stop] and [after]. [stop]: R:stop%5C [after]: R:after\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn linkref_keys_skips_a_label_after_a_closing_bracket() {
    // The `(?<!\])` lookbehind: a `[` right after `]` defines nothing, so a
    // standalone `a][b]` produces an empty link-reference map; a label defined
    // elsewhere (a normal shortcut, or a second `[ref]` group) is present.
    let keys = |s: &str| linkref_keys(&[Inline::Text(s.to_string())]);
    assert!(keys("a][b]").is_empty());
    assert!(keys("a][b] and [b] here").contains("b"));
    assert!(keys("[text][ref]").contains("ref"));
    // Lookahead: a `[…]` followed by `{` defines nothing.
    assert!(keys("[a]{x}").is_empty());
}

#[test]
fn projects_undefined_shortcut_after_bracket_as_literal() {
    // `a][b]` standalone: `b` is never a link-reference candidate (the `[` is
    // preceded by `]`), so roxygen2 leaves it literal — arity must demote its
    // optimistically-resolved `\link{b}` back to text.
    let src = "#' @md\n#' @details\n#' A stray a][b] here.\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"A stray a][b] here.\"))"
    );
}

#[test]
fn projects_undefined_ref_links_only_the_defined_inner_shortcut() {
    // `[a [b] c][ref]`: the inner `[b]` is a defined candidate (links), the
    // outer `[ref]` after a `]` is not (stays literal with its brackets).
    let src = "#' @md\n#' @details\n#' A [a [b] c][ref] link.\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"A [a\") (\\link (TEXT \"b\")) (TEXT \"c][ref] link.\"))"
    );
}

#[test]
fn undefined_shortcut_links_when_defined_elsewhere() {
    // The same `a][b]` resolves when a later standalone `[b]` defines `b` —
    // the full-field refmap, not a position rule (cf. md_ref_link_multiline).
    let src = "#' @md\n#' @details\n#' A stray a][b], later [b].\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"A stray a]\") (\\link (TEXT \"b\")) \
         (TEXT \", later\") (\\link (TEXT \"b\")) (TEXT \".\"))"
    );
}

#[test]
fn projects_undefined_shortcut_inside_a_list_item_as_literal() {
    // The whole-field refmap + undefined-label demotion descend into list
    // items: an `a][b]` inside a list item is undefined (the `[` is preceded
    // by `]`), so roxygen2 keeps it literal — arity must demote its
    // optimistic `\link{b}` inside the `\itemize`.
    let src = "#' @md\n#' @details\n#' Top.\n#'\n\
               #' - a stray a][b] keeps it\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"Top.\") \
         (\\itemize (\\item) (TEXT \"a stray a][b] keeps it\")))"
    );
}

#[test]
fn projects_self_defined_shortcut_inside_a_list_item_as_link() {
    // A plain `[foo]` shortcut inside a list item self-defines (roxygen2
    // synthesizes `[foo]: R:foo`), so the whole-field refmap keeps it in
    // `keys` and it stays a `\link` — the refmap recursion must not demote a
    // self-defined in-list shortcut.
    let src = "#' @md\n#' @details\n#' Top.\n#'\n\
               #' - see [foo] here\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"Top.\") \
         (\\itemize (\\item) (TEXT \"see\") (\\link (TEXT \"foo\")) (TEXT \"here\")))"
    );
}

#[test]
fn projects_in_list_poisoning_demotes_a_later_in_list_shortcut() {
    // Whole-field poisoning descends into list items: an escaped-close
    // candidate inside a list item poisons the appended definition block, so a
    // *later* in-list shortcut is de-linked into literal text and both leaked
    // definitions surface as trailing prose.
    let src = "#' @md\n#' @details\n#' Pre [before] links.\n#'\n\
               #' - an escaped close [stop\\] here\n\
               #' - a shortcut [foo] after\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"Pre\") (\\link (TEXT \"before\")) (TEXT \"links.\") \
         (\\itemize (\\item) (TEXT \"an escaped close [stop] here\") \
         (\\item) (TEXT \"a shortcut [foo] after\")) \
         (TEXT \"[stop]: R:stop%5C [foo]: R:foo\"))"
    );
}

#[test]
fn projects_in_list_candidate_before_the_boundary_survives() {
    // The boundary maps back through the list's per-item space-guard offsets:
    // a shortcut in an *earlier* item (before the escaped-close candidate)
    // still resolves, while one in a later item is demoted.
    let src = "#' @md\n#' @details\n#' Top.\n#'\n\
               #' - early [foo] survives\n\
               #' - an escaped close [stop\\] here\n\
               #' - [bar] dead\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"Top.\") \
         (\\itemize (\\item) (TEXT \"early\") (\\link (TEXT \"foo\")) (TEXT \"survives\") \
         (\\item) (TEXT \"an escaped close [stop] here\") \
         (\\item) (TEXT \"[bar] dead\")) \
         (TEXT \"[stop]: R:stop%5C [bar]: R:bar\"))"
    );
}

#[test]
fn decode_html_entities_resolves_named_and_numeric_refs() {
    assert_eq!(decode_html_entities("a&amp;b"), "a&b");
    assert_eq!(decode_html_entities("&lt;&gt;&quot;&apos;"), "<>\"'");
    assert_eq!(decode_html_entities("&#65;&#x42;"), "AB");
    // No `&`: byte-identical fast path. Unrecognized name or a bare `&`: verbatim.
    assert_eq!(decode_html_entities("plain"), "plain");
    assert_eq!(decode_html_entities("a&b=1"), "a&b=1");
    assert_eq!(decode_html_entities("&unknown;"), "&unknown;");
}

#[test]
fn parses_a_multiline_linkref_definition() {
    // `[ref]:` then a continuation line carrying the URL resolve to one
    // `\href`; the definition lines are consumed.
    let src = "#' @md\n#' @details\n#' See [ref].\n#'\n\
               #' [ref]:\n#'   https://example.com\n#' @name x\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\details (TEXT \"See\") \
         (\\href (VERB \"https://example.com\") (TEXT \"ref\")) (TEXT \".\"))"
    );
}

#[test]
fn append_rendered_text_coalesces_into_trailing_text() {
    // Merges into a trailing `(TEXT …)`, round-tripping the escape encoding.
    let mut atoms = vec!["(TEXT \"prose.\")".to_string()];
    append_rendered_text(&mut atoms, "[t]: R:t%5C");
    assert_eq!(atoms, vec!["(TEXT \"prose. [t]: R:t%5C\")".to_string()]);
    // With no trailing prose atom, a fresh `(TEXT …)` is pushed.
    let mut atoms = vec!["(\\link (TEXT \"x\"))".to_string()];
    append_rendered_text(&mut atoms, "[t]: R:t%5C");
    assert_eq!(
        atoms,
        vec![
            "(\\link (TEXT \"x\"))".to_string(),
            "(TEXT \"[t]: R:t%5C\")".to_string()
        ]
    );
}

#[test]
fn projects_escaped_close_bracket_leaked_linkref() {
    // The end-to-end case: a `@md` shortcut whose closing bracket is escaped is
    // not a link, but roxygen2 leaks its synthesized reference definition into
    // the rendered prose (coalesced with the section text).
    let src = "#' @md\n\
               #' @title T\n\
               #' @details\n\
               #' A link like [text\\] leaks.\n\
               #' @name spec\n\
               NULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\details (TEXT \"A link like [text] leaks. [text]: R:text%5C\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn rd_complete_ports_the_brace_balance_check() {
    // Balanced braces, escaped braces, and `%` line comments are complete.
    assert!(rd_complete("a{b}"));
    assert!(rd_complete("a\\{b")); // escaped `{` not counted
    assert!(rd_complete("\\emph{x}"));
    assert!(rd_complete("a%{")); // `%` comments the unmatched `{`
    assert!(rd_complete("{%}\n}")); // comment ends at newline; `}` then closes
    // Unbalanced or escaped-away closers are incomplete.
    assert!(!rd_complete("a{b"));
    assert!(!rd_complete("a}b"));
    assert!(!rd_complete("\\emph{\\}")); // trailing `\` escapes the closing `}`
    assert!(!rd_complete("a\\")); // a dangling escape is incomplete
    assert!(!rd_complete("{%}")); // comment swallows the `}`; `{` stays open
}

#[test]
fn section_atoms_rd_complete_reconstructs_braces() {
    // A balanced inline macro projects complete; a `%` in prose is re-escaped
    // (no comment), so a following structural `}` still closes.
    assert!(section_atoms_rd_complete(
        &["(TEXT \"foo\")".into(), "(\\emph (TEXT \"x\"))".into()],
        true,
    ));
    assert!(section_atoms_rd_complete(
        &["(\\emph (TEXT \"a % b\"))".into()],
        true,
    ));
    // A `%` inside a verbatim URL is escaped too (roxygen2 renders `\%`), so an
    // `\href{…%20…}{…}` stays complete rather than the URL commenting out the
    // closing braces.
    assert!(section_atoms_rd_complete(
        &["(\\href (VERB \"https://x/a%20b\") (TEXT \"link % text\"))".into()],
        true,
    ));
    // An emphasis whose content is a lone backslash renders `\emph{\}`, whose
    // trailing `\` escapes the closing brace --- exactly roxygen2's `*\**` bug.
    assert!(!section_atoms_rd_complete(
        &["(TEXT \"foo\")".into(), "(\\emph (TEXT \"\\\\\"))".into()],
        true,
    ));
}

#[test]
fn trailing_backslash_inline_dest_drops_the_section() {
    // `[t](foo\)bar)`: `double_escape_md` turns the `\)` into a literal `\` + a
    // closing `)`, so cmark's destination is `foo\` — a trailing backslash that
    // escapes the `\href{…}` brace → roxygen2 drops the whole section.
    let drop = Inline::MdInlineLink {
        url: "foo\\)bar".into(),
        display: vec![Inline::Text("t".into())],
    };
    assert!(md_href_dest_drops("foo\\)bar"));
    assert!(!section_rd_complete(std::slice::from_ref(&drop), true));
    // An **even** backslash run before the `)` pairs off (`foo\\)bar` → `foo\\`),
    // so the brace closes and the section is kept.
    assert!(!md_href_dest_drops("foo\\\\)bar"));
    assert!(section_rd_complete(
        &[Inline::MdInlineLink {
            url: "foo\\\\)bar".into(),
            display: vec![Inline::Text("t".into())],
        }],
        true,
    ));
    // A destination with no closer, ending in a lone backslash, drops too; an
    // ordinary destination is unaffected.
    assert!(md_href_dest_drops("foo\\"));
    assert!(!md_href_dest_drops("foo/bar"));
    // A dropping href nested inside emphasis still drops the section.
    assert!(!section_rd_complete(
        &[Inline::MdEmphasis {
            strong: false,
            children: vec![drop],
        }],
        true,
    ));
}

#[test]
fn trailing_percent_swallow_does_not_false_drop() {
    // An odd-run `\%` swallow at a section's end keeps a dangling `\` in the
    // output atom (`y \% {z} end.` renders `y \`), but roxygen2 scans
    // `markdown(text)` = `y \\% {z} end.` (even run pairs, bare `%` comments) and
    // keeps the section. The drop scan must strip the whole region so no trailing
    // escape survives. A soft-wrap continuation already resolved the escape; a
    // physical line end did not (the bug).
    assert!(section_rd_complete(
        &[Inline::Text("y \\% {z} end.".into())],
        true,
    ));
    // A longer odd run behaves the same (comments to end of line).
    assert!(section_rd_complete(
        &[Inline::Text("a \\\\\\% b {c} d.".into())],
        true,
    ));
    // An even-run `%` is a genuine literal percent (escaped, not a comment): a
    // following unbalanced `{` still drops.
    assert!(!section_rd_complete(
        &[Inline::Text("a % b {c".into())],
        true
    ));
    // A real brace imbalance before the `%` still drops (`{` opens, the comment
    // eats the closer).
    assert!(!section_rd_complete(
        &[Inline::Text("{a \\% b}".into())],
        true,
    ));
    // The stripper drops the run + `%` + line tail, but keeps later physical
    // lines and an even-run `%`.
    assert_eq!(strip_scan_percent_comment("y \\% {z} end."), "y ");
    assert_eq!(
        strip_scan_percent_comment("y \\% gone\nkept % here"),
        "y \nkept % here"
    );
}

#[test]
fn fragile_macro_arg_never_unbalances_the_scan() {
    // A fragile macro's argument is raw + brace-balanced in markdown() output,
    // so its interior braces (resolved to bare `{` in the atom, or kept as `\{`
    // in a code span) must not count against `rd_complete`. Both the resolved
    // and escaped forms stay complete; only the enclosing `\name{…}` pair counts.
    assert!(section_atoms_rd_complete(
        &["(\\verb (VERB \"d { e\"))".into()], // literal `\verb{d \{ e}`, resolved
        true,
    ));
    assert!(section_atoms_rd_complete(
        &["(\\verb (VERB \"x \\\\{ y\"))".into()], // code span, kept as `\{`
        true,
    ));
    assert!(section_atoms_rd_complete(
        &["(\\code (RCODE \"a { b\"))".into()],
        true,
    ));
    // The fragile-macro neutralization is confined to fragile heads: a
    // cmark-derived `\emph{\}` still counts its escaping backslash and drops.
    assert!(!section_atoms_rd_complete(
        &["(\\emph (TEXT \"\\\\\"))".into()],
        true,
    ));
}

#[test]
fn projects_rdcomplete_failure_drops_the_section() {
    // roxygen2 runs `rdComplete` on the rendered Rd of an `@description`/
    // `@details` section (`markdown_if_active`, `sections = TRUE`); when the
    // braces are unbalanced it warns and drops the body to empty. An escaped
    // emphasis delimiter `*\**` renders `\emph{\}*`, which is incomplete, so the
    // section projects empty --- matching the `cm-439`/`442`/`451`/`454` pins.
    for delim in ["*\\**", "**\\***", "_\\__", "__\\___"] {
        let src =
            format!("#' @md\n#' @title T\n#' @details\n#' foo {delim}\n#' @name spec\nNULL\n");
        let out = project_to_rd(&src);
        assert!(
            out.contains("(\\details)") && !out.contains("(\\details "),
            "delim {delim:?} got: {out}"
        );
    }
}

#[test]
fn rdcomplete_drop_is_scoped_to_with_sections_tags() {
    // `@return` (`tag_markdown`, `sections = FALSE`) is *not* dropped on an
    // imbalance --- only `@description`/`@details` carry the per-section check.
    let src = "#' @md\n#' @title T\n#' @return foo *\\**\n#' @name spec\nNULL\n";
    let out = project_to_rd(src);
    assert!(out.contains("(\\value"), "got: {out}");
}

#[test]
fn markdown_off_escaped_brace_does_not_drop_the_section() {
    // Markdown-off, roxygen2 runs `rdComplete` on the *raw* tag value, where a
    // `\{`/`\}` is still escaped and therefore not counted --- so an unbalanced
    // *escaped* brace keeps the section (`\{` renders a bare `{`). The projector
    // must scan the pre-resolution raw text, not the escape-resolved atoms
    // (where `\{` has collapsed to a bare `{` that would false-drop the section).
    let src = "#' @title A title with an escaped a \\{ brace\n\
               #' @description Desc with an escaped a \\} brace\n\
               #' @name x\nNULL\n";
    let out = project_to_rd(src);
    assert!(
        out.contains("(\\title (TEXT \"A title with an escaped a { brace\"))"),
        "title got: {out}"
    );
    assert!(
        out.contains("(\\description (TEXT \"Desc with an escaped a } brace\"))"),
        "description got: {out}"
    );
    // A genuinely unbalanced *bare* brace still drops (no escape to protect it).
    let bare = "#' @title T\n#' @description bare a { brace\n#' @name x\nNULL\n";
    let out = project_to_rd(bare);
    assert!(
        out.contains("(\\description)") && !out.contains("(\\description "),
        "bare got: {out}"
    );
}

#[test]
fn md_bare_brace_groups_project_as_lists() {
    // Under `@md` a balanced bare `{…}` is an Rd `LIST` too. The brace parity
    // is shared with non-md (odd run escapes, even run opens), but the
    // `%`-comment trigger is inverted (`group_brace_lists` mirrors
    // `md_percent_swallow`): a bare/even-preceded `%` stays literal and does
    // *not* hide a following group, while an odd-preceded `\%` renders bare and
    // swallows to the physical line end.
    let case = |body: &str| {
        let src = format!("#' @md\n#' @title T\n#' @details {body}\n#' @name x\nNULL\n");
        let out = project_to_rd(&src);
        out.lines()
            .find(|l| l.starts_with("(\\details"))
            .unwrap_or("")
            .to_string()
    };
    // Simple / nested / empty groups.
    assert_eq!(
        case("a {b c} d"),
        "(\\details (TEXT \"a\") (LIST (TEXT \"b c\")) (TEXT \"d\"))"
    );
    assert_eq!(
        case("a {b {c} d} e"),
        "(\\details (TEXT \"a\") (LIST (TEXT \"b\") (LIST (TEXT \"c\")) (TEXT \"d\")) (TEXT \"e\"))"
    );
    assert_eq!(
        case("a {} b"),
        "(\\details (TEXT \"a\") (LIST) (TEXT \"b\"))"
    );
    // An even backslash run opens the group (one literal backslash kept).
    assert_eq!(
        case(r"s \\{t} u"),
        "(\\details (TEXT \"s \\\\\") (LIST (TEXT \"t\")) (TEXT \"u\"))"
    );
    // An odd run escapes the braces: literal, no group.
    assert_eq!(case(r"p \{q\} r"), "(\\details (TEXT \"p {q} r\"))");
    // A bare/even `%` is literal (roxygen2 escapes it) and does not hide braces.
    assert_eq!(
        case("v % {w} x"),
        "(\\details (TEXT \"v %\") (LIST (TEXT \"w\")) (TEXT \"x\"))"
    );
    assert_eq!(
        case(r"v \\% {w} x"),
        "(\\details (TEXT \"v \\\\%\") (LIST (TEXT \"w\")) (TEXT \"x\"))"
    );
}

#[test]
fn macro_arg_bare_groups_project_as_lists() {
    // A bare `{…}` inside a *prose* macro argument is an Rd `LIST` too
    // (parse_Rd lexes the argument with the same bare-group rule). Verbatim
    // arguments (`\code`) never group; structural macros (`\href`) GRP-wrap a
    // multi-atom display with the group counted as one atom.
    let case = |md: bool, body: &str| {
        let md_line = if md { "#' @md\n" } else { "" };
        let src = format!("{md_line}#' @title T\n#' @details {body}\n#' @name x\nNULL\n");
        project_to_rd(&src)
            .lines()
            .find(|l| l.starts_with("(\\details"))
            .unwrap_or("")
            .to_string()
    };
    for md in [false, true] {
        // A latexlike single-arg macro folds a bare group.
        assert_eq!(
            case(md, r"\emph{a {b} c}"),
            "(\\details (\\emph (TEXT \"a\") (LIST (TEXT \"b\")) (TEXT \"c\")))"
        );
        // Groups nest and span a nested macro.
        assert_eq!(
            case(md, r"\emph{i {j \strong{k} l} m}"),
            "(\\details (\\emph (TEXT \"i\") (LIST (TEXT \"j\") (\\strong (TEXT \"k\")) (TEXT \"l\")) (TEXT \"m\")))"
        );
        // An empty group is a bare `(LIST)`.
        assert_eq!(
            case(md, r"\emph{n {} o}"),
            "(\\details (\\emph (TEXT \"n\") (LIST) (TEXT \"o\")))"
        );
        // A structural display GRP-wraps; the group counts as one atom.
        assert_eq!(
            case(md, r"\href{http://x.org}{s {t} u}"),
            "(\\details (\\href (VERB \"http://x.org\") (GRP (TEXT \"s\") (LIST (TEXT \"t\")) (TEXT \"u\"))))"
        );
        // A verbatim `\code` argument is R code: braces stay literal, no group.
        assert_eq!(
            case(md, r"\code{v {w} x}"),
            "(\\details (\\code (RCODE \"v {w} x\")))"
        );
    }
    // Non-md only: escaped braces stay literal (an odd backslash run escapes).
    assert_eq!(
        case(false, r"\emph{p \{q\} r}"),
        "(\\details (\\emph (TEXT \"p {q} r\")))"
    );
}

#[test]
fn even_run_braced_macro_projects_as_literal_plus_list() {
    // An *even* backslash run before a brace-required macro name defeats the
    // macro carve: parse_Rd pairs the backslashes to a literal `\`, leaves the
    // name as plain text (`\\emph` -> `\emph`), and the following `{…}` is an
    // ordinary bare-brace `LIST` -- never the macro's argument. This is the
    // even-run twin of `md_bare_brace_groups_project_as_lists`; it falls out of
    // the backslash-parity gate (no macro carve) plus `group_brace_lists`, and
    // holds identically in both modes.
    let case = |md: bool, body: &str| {
        let md_line = if md { "#' @md\n" } else { "" };
        let src = format!("{md_line}#' @title T\n#' @details {body}\n#' @name x\nNULL\n");
        project_to_rd(&src)
            .lines()
            .find(|l| l.starts_with("(\\details"))
            .unwrap_or("")
            .to_string()
    };
    for md in [false, true] {
        // Even run (k=2): one literal backslash kept, `emph` literal, `{x}` a LIST.
        assert_eq!(
            case(md, r"a \\emph{x} b"),
            "(\\details (TEXT \"a \\\\emph\") (LIST (TEXT \"x\")) (TEXT \"b\"))"
        );
        // A longer even run (k=4) halves to two literal backslashes.
        assert_eq!(
            case(md, r"c \\\\emph{y} d"),
            "(\\details (TEXT \"c \\\\\\\\emph\") (LIST (TEXT \"y\")) (TEXT \"d\"))"
        );
        // Even runs also spare a brace-required section macro from its
        // brace-less drop (`\link` normally drops when brace-less).
        assert_eq!(
            case(md, r"e \\link{z} f"),
            "(\\details (TEXT \"e \\\\link\") (LIST (TEXT \"z\")) (TEXT \"f\"))"
        );
        // The trailing group still nests.
        assert_eq!(
            case(md, r"g \\emph{h {i} j} k"),
            "(\\details (TEXT \"g \\\\emph\") (LIST (TEXT \"h\") (LIST (TEXT \"i\")) (TEXT \"j\")) (TEXT \"k\"))"
        );
    }
}

#[test]
fn heading_title_bare_groups_project_as_lists() {
    // A bare `{…}` in a markdown heading title is an Rd `LIST`, exactly as in
    // prose and macro args; the multi-atom title GRP-wraps. ATX and setext
    // share the path (both flow through `render_heading_frame`); groups nest
    // and span emphasis.
    let section = |body: &str| {
        let src = format!(
            "#' @md\n#' @title T\n#' @details Intro.\n#'\n#' {body}\n#' body prose.\n#' @name x\nNULL\n"
        );
        project_to_rd(&src)
            .lines()
            .find(|l| l.starts_with("(\\section"))
            .unwrap_or("")
            .to_string()
    };
    // A bare group in an ATX title.
    assert_eq!(
        section("# H {a b}"),
        "(\\section (GRP (TEXT \"H\") (LIST (TEXT \"a b\"))) (TEXT \"body prose.\"))"
    );
    // Groups nest.
    assert_eq!(
        section("# H {a {b} c}"),
        "(\\section (GRP (TEXT \"H\") (LIST (TEXT \"a\") (LIST (TEXT \"b\")) (TEXT \"c\"))) (TEXT \"body prose.\"))"
    );
    // A group spans emphasis.
    assert_eq!(
        section("# H {k *x* l}"),
        "(\\section (GRP (TEXT \"H\") (LIST (TEXT \"k\") (\\emph (TEXT \"x\")) (TEXT \"l\"))) (TEXT \"body prose.\"))"
    );
}

#[test]
fn resolve_md_brace_runs_pairs_a_backslash_run_before_a_brace() {
    // parse_Rd pairs the cmark-stage run into `floor(k/2)` backslashes; an odd
    // trailing `\` escapes the brace bare. Matches roxygen2 for odd `k`.
    assert_eq!(resolve_md_brace_runs(r"a \{ b \} c"), "a { b } c"); // k=1
    assert_eq!(resolve_md_brace_runs(r"a \{ b"), "a { b");
    assert_eq!(resolve_md_brace_runs(r"a \\\{ b \\\} c"), r"a \{ b \} c"); // k=3
    assert_eq!(resolve_md_brace_runs(r"a \\\\\{ c"), r"a \\{ c"); // k=5
    // An even run halves and leaves the brace bare (the `(LIST …)` backlog).
    assert_eq!(resolve_md_brace_runs(r"a \\{ b"), r"a \{ b"); // k=2
    // A bare brace and non-brace escapes are left alone.
    assert_eq!(resolve_md_brace_runs("a { b } c"), "a { b } c");
    assert_eq!(resolve_md_brace_runs(r"a \* \b c"), r"a \* \b c");
}

#[test]
fn resolve_md_text_braces_only_touches_text_leaves() {
    // A `TEXT` leaf resolves its escaped braces; a `VERB` leaf (a verbatim code
    // span / URL) keeps them, and structure is copied verbatim.
    let sexpr = "(\\details (TEXT \"a \\\\{ b\") (\\verb (VERB \"c \\\\{ d\")))";
    assert_eq!(
        resolve_md_text_braces(sexpr),
        "(\\details (TEXT \"a { b\") (\\verb (VERB \"c \\\\{ d\")))"
    );
    // A literal `(TEXT "` inside a non-`TEXT` string is data, not a leaf opener.
    let tricky = "(\\verb (VERB \"see (TEXT \\\"x \\\\{ y\\\")\"))";
    assert_eq!(resolve_md_text_braces(tricky), tricky);
}

#[test]
fn markdown_on_escaped_brace_renders_bare_but_does_not_drop() {
    // Under `@md` the source escape survives the double->cmark round trip, so an
    // unbalanced *escaped* brace is kept (not dropped) and the rendered TEXT is
    // bare; a `\code`-fragile arg and a verbatim code span are unaffected here.
    let src = "#' @title T\n#' @md\n#' @details a \\{ b\n#' @name x\nNULL\n";
    let out = project_to_rd(src);
    assert!(
        out.contains("(\\details (TEXT \"a { b\"))"),
        "details got: {out}"
    );
    // A genuinely unbalanced *bare* brace still drops under `@md` too.
    let bare = "#' @title T\n#' @md\n#' @details a { b\n#' @name x\nNULL\n";
    let out = project_to_rd(bare);
    assert!(
        out.contains("(\\details)") && !out.contains("(\\details "),
        "bare got: {out}"
    );
}

#[test]
fn resolve_rd_arg_escapes_resolves_the_rd_metacharacter_escapes() {
    // The four braced-argument escapes render bare; backslashes pair
    // left-to-right; any other lone backslash stays literal.
    assert_eq!(resolve_rd_arg_escapes(r"a \{ b \} c"), "a { b } c");
    assert_eq!(resolve_rd_arg_escapes(r"a \% b"), "a % b");
    assert_eq!(resolve_rd_arg_escapes(r"a \\ b"), r"a \ b");
    assert_eq!(resolve_rd_arg_escapes(r"a \* \b c"), r"a \* \b c");
    // A paired `\\` before a metacharacter is a literal backslash, not an escape.
    assert_eq!(resolve_rd_arg_escapes(r"a \\{ b"), r"a \{ b");
}

#[test]
fn literal_macro_args_resolve_rd_escapes_both_modes() {
    // A literal `\code`/`\emph`/`\url` argument resolves parse_Rd's Rd-string
    // escapes (`\{`/`\}`/`\%`/`\\`), verbatim `RCODE`/`VERB` and prose `TEXT`
    // alike, with markdown off.
    let src = "#' @title T\n\
               #' @details c \\code{a \\{ b \\% d} e \\emph{f \\} g} u \\url{h/\\%20}\n\
               #' @name x\nNULL\n";
    let out = project_to_rd(src);
    assert!(
        out.contains("(\\code (RCODE \"a { b % d\"))")
            && out.contains("(\\emph (TEXT \"f } g\"))")
            && out.contains("(\\url (VERB \"h/%20\"))"),
        "non-md got: {out}"
    );
    // A fragile macro's argument resolves the same braces under `@md` (the
    // TEXT-only post-pass never reaches its verbatim `RCODE`/`VERB`).
    let md = "#' @title T\n#' @md\n\
              #' @details span \\code{a \\{ b \\} c} verb \\verb{d \\{ e \\} f}\n\
              #' @name x\nNULL\n";
    let out = project_to_rd(md);
    assert!(
        out.contains("(\\code (RCODE \"a { b } c\"))")
            && out.contains("(\\verb (VERB \"d { e } f\"))"),
        "md got: {out}"
    );
}

#[test]
fn markdown_code_span_keeps_its_backslash_brace() {
    // A markdown code span projects through a *different* path than a literal
    // `\verb`, so it must NOT resolve `\{` (roxygen2 renders the span verbatim).
    let src = "#' @title T\n#' @md\n#' @details a `x \\{ y` z\n#' @name w\nNULL\n";
    let out = project_to_rd(src);
    assert!(
        out.contains("(\\verb (VERB \"x \\\\{ y\"))"),
        "code span got: {out}"
    );
}

#[test]
fn url_defined_reference_links_render_href() {
    // A user link-reference definition `[ref]: url` defines a destination, so
    // roxygen2 renders the referencing link as `\href{url}{display}` with the
    // display *kept* (the "must contain plain text" drop is `\link`-only). The
    // definition lines themselves are consumed (cmark removes them).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' See [*foo*][r1] and [plain][r2] and [`code`][r3].\n\
               #'\n\
               #' [r1]: https://example.com\n\
               #' [r2]: https://example.org\n\
               #' [r3]: https://example.net\n\
               #' @name spec\nNULL\n";
    assert_eq!(
        project_to_rd(src),
        "(\\description (TEXT \"T\"))\n\
         (\\details (TEXT \"See\") \
         (\\href (VERB \"https://example.com\") (\\emph (TEXT \"foo\"))) (TEXT \"and\") \
         (\\href (VERB \"https://example.org\") (TEXT \"plain\")) (TEXT \"and\") \
         (\\href (VERB \"https://example.net\") (\\code (RCODE \"code\"))) (TEXT \".\"))\n\
         (\\title (TEXT \"T\"))"
    );
}

#[test]
fn url_defined_shortcut_link_renders_href() {
    // A bare shortcut `[r1]` whose label has a user URL definition → `\href`;
    // the definition line is consumed.
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' See [r1] here.\n#'\n#' [r1]: https://example.com\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"See\") (\\href (VERB \"https://example.com\") (TEXT \"r1\")) (TEXT \"here.\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn linkref_labels_match_by_unicode_case_fold() {
    // cmark's `normalize_reference` uses full Unicode case folding (CaseFolding
    // C+F), not lowercasing: `ẞ` folds to `ss` (matching `SS`), micro sign `µ`
    // to Greek `μ` (matching `Μ`), and the `ﬁ` ligature expands to `fi`
    // (cm-542; engine-probed 2026-07-14).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [\u{1e9e}] and [\u{b5}w] and [\u{fb01}n]\n#'\n\
               #' [SS]: /a\n#' [\u{39c}W]: /b\n#' [FIN]: /c\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\href (VERB \"/a\") (TEXT \"\u{1e9e}\")) (TEXT \"and\") \
             (\\href (VERB \"/b\") (TEXT \"\u{b5}w\")) (TEXT \"and\") \
             (\\href (VERB \"/c\") (TEXT \"\u{fb01}n\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn nbsp_in_linkref_label_is_content_not_whitespace() {
    // cmark's `normalize_reference` collapses only ASCII whitespace; a NBSP is
    // label content, so `[a\u{a0}b]` does NOT match `[a b]: /d` — the shortcut
    // resolves against its synthesized `R:` definition instead (engine-probed
    // 2026-07-14).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [a\u{a0}b]\n#'\n#' [a b]: /d\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (\\link (TEXT \"a\u{a0}b\")))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn trailing_backslash_label_never_defines_or_links() {
    // A label with a trailing backslash run never closes after roxygen2's
    // double-escape (`\\]` reverts to `\]`, leaving an odd escaping run): the
    // def line stays literal prose, the shortcut de-links, and both candidate
    // definitions leak with the post-escape three-backslash label (cm-552).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [bar\\\\]: /uri\n#'\n#' [bar\\\\]\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"[bar\\\\]: /uri [bar\\\\] \
             [bar\\\\]: R:bar%5C%5C%5C [bar\\\\]: R:bar%5C%5C%5C\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn blank_label_never_defines_or_links() {
    // A whitespace-only label (`[` + newline + space + `]`) has no
    // non-whitespace character, so it neither defines nor resolves
    // (CommonMark); both candidates leak, URL-encoding the real newline as
    // `%0A` and the continuation line's content space as `%20` (cm-554).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [\n#'  ]\n#'\n#' [\n#'  ]: /uri\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\details (TEXT \"[ ] [ ]: /uri [ ]: R:%0A%20 [ ]: R:%0A%20\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn escaped_open_bracket_is_label_content() {
    // A backslash-escaped `[` inside a link label is content (cmark): a
    // reference link `[foo][ref\[]` matches its user definition `[ref\[]: /uri`
    // source-exactly (cmark's `normalize_reference` does not unescape), the
    // def line is consumed, and the rendered `\href` display is `foo`. A
    // defined *shortcut* `[ref\[]` links too, with the escape resolved in its
    // display text (`ref[`) (cm-551).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [foo][ref\\[] and [ref\\[]\n#'\n#' [ref\\[]: /uri\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\href (VERB \"/uri\") (TEXT \"foo\")) (TEXT \"and\") \
             (\\href (VERB \"/uri\") (TEXT \"ref[\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn undefined_escaped_bracket_label_demotes_to_literal() {
    // With no matching definition, an escaped-`[` label is not a
    // `get_md_linkrefs` candidate (its content is not bracket-free), so no
    // reference is synthesized and the link demotes to literal text with the
    // bracket escape resolved (`[ref[]`), not an `R:`-dest link.
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' a [ref\\[] b\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"a [ref[] b\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn ref_link_chain_pairs_left_to_right() {
    // `[foo][bar][baz]` pairs left-to-right (cmark): the first `]` consumes
    // the following `[bar]` as its reference label — regardless of the
    // `[baz]` that follows it — and `[baz]` is a separate shortcut. With
    // user definitions for both labels, that is `\href{/url2}{foo}` +
    // `\href{/url1}{baz}` (cm-572).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [foo][bar][baz]\n#'\n#' [baz]: /url1\n#' [bar]: /url2\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\href (VERB \"/url2\") (TEXT \"foo\")) \
             (\\href (VERB \"/url1\") (TEXT \"baz\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn user_def_resolves_ref_link_inside_emphasis() {
    // A user `[ref]: /uri` definition resolves a reference link nested
    // inside emphasis: the inner `[baz][ref]` forms (deactivating the outer
    // opener, which stays literal) and rewrites to `\href{/uri}{baz}`, just
    // like the top-level trailing `[ref]` shortcut (cm-535).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [foo *bar [baz][ref]*][ref]\n#'\n#' [ref]: /uri\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"[foo\") \
             (\\emph (TEXT \"bar\") (\\href (VERB \"/uri\") (TEXT \"baz\"))) \
             (TEXT \"]\") (\\href (VERB \"/uri\") (TEXT \"ref\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn resolved_inner_link_is_not_a_blank_label() {
    // A literal `[` + resolved inner shortcut + literal `]` (`[[x]](url)`)
    // must not read as a blank `[ ]` candidate in the poisoning skeleton —
    // the stand-in for resolved structure is non-whitespace, so the inner
    // link survives and the outer brackets stay literal (cm-550, cm-592).
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' a [[x]](https://example.com) b\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"a [\") (\\link (TEXT \"x\")) \
             (TEXT \"](https://example.com) b\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn linkref_definition_cannot_interrupt_a_paragraph() {
    // A `[r1]: url` line *without* a preceding blank line is part of the
    // paragraph, not a definition (CommonMark): the label stays an R-topic
    // `\link` and the line renders literally. (Regression guard: the user-def
    // transform must only fire at a real block start.)
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' Some prose with [r1] here.\n#' [r1]: https://example.com\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"Some prose with\") (\\link (TEXT \"r1\")) (TEXT \"here.\") (\\link (TEXT \"r1\")) (TEXT \": https://example.com\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn linkref_definition_with_trailing_macro_is_not_a_definition() {
    // A `[foo]: url \emph{bar}` line has trailing inline content after the
    // destination (the `\emph{bar}` macro), which CommonMark forbids in a link
    // reference definition, so it is *not* a definition: the label stays an
    // R-topic `\link` (synthesized `R:foo`) and the line renders literally, with
    // the macro surfacing as its own subtree. (Regression guard: the user-def
    // scan only sees the trailing `Text` run, so it must also reject a trailing
    // non-`Text` inline.)
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' See [foo].\n#'\n#' [foo]: https://x.org \\emph{bar}\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"See\") (\\link (TEXT \"foo\")) (TEXT \".\") (\\link (TEXT \"foo\")) (TEXT \": https://x.org\") (\\emph (TEXT \"bar\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn backslash_word_in_link_display_renders_as_rd_macro() {
    // A markdown link display carrying a backslash word (`\b`, an Rd macro to
    // parse_Rd) keeps the link — at the markdown level the backslash is literal,
    // so roxygen2 does not drop it — and the macro surfaces as a nested subtree
    // inside the `\link` rather than collapsing into the topic text. The
    // reference form `[a\b][lbl]` drops its topic and renders identically.
    let src = "#' @md\n#' @title T\n#' @details See [a\\b] and [a\\b][lbl] now.\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"See\") (\\link (TEXT \"a\") (UNKNOWN \"\\\\b\")) (TEXT \"and\") (\\link (TEXT \"a\") (UNKNOWN \"\\\\b\")) (TEXT \"now.\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn escaped_emphasis_in_link_display_drops_the_link() {
    // `[a\*b\*]` resolves an emphasis node in its display (a non-text child), so
    // roxygen2's `parse_link` drops the whole link ("must contain plain text")
    // and the surrounding prose coalesces — unlike a backslash *word*, which is
    // markdown-level plain text and is kept.
    let src = "#' @md\n#' @title T\n#' @details A [a\\*b\\*] gap.\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"A gap.\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn pure_macro_active_link_display_drops() {
    // A shortcut whose display is a *pure* macro (no surrounding text) carrying
    // cmark-active markdown (`[\emph{*x*}]`) drops to empty like any non-plain
    // display — the link must reach the drop site, not be spuriously demoted to a
    // literal `[]` by an empty link-reference label. Regression guard for the
    // pure-macro label fix (`link_label_text` includes the macro source).
    let src = "#' @md\n#' @title T\n#' @details A [\\emph{*x*}] gap.\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"A gap.\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn pure_macro_inert_link_display_keeps() {
    // A pure-macro display with an *inert* argument (`[\emph{y}]`) or a *fragile*
    // macro (`[\code{f}]`) keeps the link, rendering `\link` over the macro
    // subtree — not a literal `[]`. The self-consistent macro-source label lets
    // the link survive the undefined-label demotion and reach the keep path.
    let src = "#' @md\n#' @title T\n#' @details Keep [\\emph{y}] and [\\code{f}].\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"Keep\") (\\link (\\emph (TEXT \"y\"))) (TEXT \"and\") (\\link (\\code (RCODE \"f\"))) (TEXT \".\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn blank_separated_content_column_prose_folds_into_item() {
    // A blank line closes a list item's paragraph, but a following prose line
    // indented to the item's content column opens a new paragraph *inside the
    // same item* (a loose item), which Rd rendering flattens into the item
    // text. Both blank-separated paragraphs fold — `- a` / blank / `  more` /
    // blank / `  even more` → one item, `a more even more`.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#'   more\n#'\n#'   even more\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (\\itemize (\\item) (TEXT \"a more even more\")))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn blank_then_underindented_prose_ends_the_list() {
    // A blank-separated continuation below the content column does *not* fold:
    // it ends the list and becomes sibling section prose (`- a` / blank /
    // `more` at column 1 → item `a`, then a separate `more`).
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#' more\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\details (\\itemize (\\item) (TEXT \"a\")) (TEXT \"more\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn fence_at_content_column_folds_into_item() {
    // A fenced code block indented to the item's content column folds into the
    // item as a child block (the three-atom `\if…\preformatted…\if` sequence
    // inside the `\itemize`), with a below content-column marker after it a
    // sibling item — a fenced block interrupts the item's paragraph.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'   ```\n#'   code\n#'   ```\n\
               #' - b\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\") \
             (\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\"))) \
             (\\preformatted (VERB \"code\\n\")) \
             (\\if (TEXT \"html\") (\\out (VERB \"</div>\"))) (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn block_quote_at_content_column_folds_into_item() {
    // A block quote indented to the item's content column folds into the
    // item — a blank only makes the item loose — and roxygen2 flattens the
    // quote to plain text glued onto the item's prose (`- a` / blank /
    // `  > q` → item text `aq`, engine-probed). A following below-column
    // list marker is a sibling item.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#'   > q\n#' - b\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\details (\\itemize (\\item) (TEXT \"aq\") (\\item) (TEXT \"b\")))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn block_quote_without_blank_folds_into_item() {
    // A block quote interrupts the item's open paragraph, so it folds with
    // no intervening blank too (`- a` / `  > q` → item text `aq`,
    // engine-probed).
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'   > q\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (\\itemize (\\item) (TEXT \"aq\")))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn same_line_block_quote_opens_inside_item() {
    // A block quote opening on the marker line itself (`- > quoted here`)
    // is the item's content — roxygen2 flattens it to the item text with
    // the `>` dropped, and a following marker line is a sibling item
    // (engine-probed; cm-294/295's inner shape).
    let src = "#' @md\n#' @title T\n#' @details\n#' - > quoted here\n#' - b\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"quoted here\") (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn same_line_item_quote_takes_lazy_continuation() {
    // A plain line after the same-line quote is the quote paragraph's lazy
    // continuation, glued with no separator (`1. > Blockquote` /
    // `continued here.` → item text `Blockquotecontinued here.`) — and the
    // same shape one quote level deeper flattens identically via the outer
    // quote's reparse (cm-294/295).
    let src = "#' @md\n#' @title T\n#' @details\n#' 1. > Blockquote\n\
               #' continued here.\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\details (\\enumerate (\\item) (TEXT \"Blockquotecontinued here.\")))"),
        "got: {}",
        project_to_rd(src)
    );
    let outer = "#' @md\n#' @title T\n#' @details\n#' > 1. > Blockquote\n\
                 #' continued here.\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(outer).contains("(\\details (TEXT \"Blockquotecontinued here.\"))"),
        "got: {}",
        project_to_rd(outer)
    );
}

#[test]
fn same_line_fence_opens_inside_item() {
    // A fenced code block opening on the marker line itself (`- ```` ``` ````,
    // cm-320/326): the block is the item's content, its code lines strip the
    // item's *content column* (the marker-less opener carries no indent for
    // the cancellation `md_code_block_parts` normally relies on), the
    // content-column closer closes it, and a following marker line is a
    // sibling item.
    let src = "#' @md\n#' @title T\n#' @details\n#' - ```\n#'   b\n#'   ```\n\
               #' - c\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\preformatted (VERB \"b\\n\"))"),
        "got: {}",
        project_to_rd(src)
    );
    assert!(
        project_to_rd(src).contains("(\\item) (TEXT \"c\")"),
        "got: {}",
        project_to_rd(src)
    );
    // The ordered-marker form keeps its info string in the `<div>` class and
    // takes a blank-separated second paragraph after the closer (cm-326).
    let ordered = "#' @md\n#' @title T\n#' @details\n#' 1. ```r\n#'    foo\n\
                   #'    ```\n#'\n#'    bar\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(ordered).contains("sourceCode r"),
        "got: {}",
        project_to_rd(ordered)
    );
    assert!(
        project_to_rd(ordered).contains("(\\preformatted (VERB \"foo\\n\"))"),
        "got: {}",
        project_to_rd(ordered)
    );
}

#[test]
fn same_line_html_block_opens_inside_item() {
    // An HTML block (condition 6) opening on the marker line itself
    // (`- <div>`, cm-177): the block is the item's content and renders
    // roxygen2's `\if{html}{\out{…}}` — a leading `(VERB "\n")` and the
    // line with its trailing newline, the same as a section-level block —
    // and a following under-indented marker line is a sibling item, not a
    // blank-terminated continuation.
    let src = "#' @md\n#' @title T\n#' @details\n#' - <div>\n#' - foo\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains(
            "(\\itemize (\\item) (\\if (TEXT \"html\") \
             (\\out (VERB \"\\n\") (VERB \"<div>\\n\"))) (\\item) (TEXT \"foo\"))"
        ),
        "got: {rd}"
    );
}

#[test]
fn escaped_close_label_defines_links_and_leaks_with_emphasis() {
    // cm-196: `\]` is link-label content (`double_escape_md`'s bracket
    // de-dup keeps the escape live through cmark), so the def matches and
    // the shortcut resolves to it. The regex candidate scan is
    // escape-blind, so both lines' `[Foo*bar\]` candidates are invalid
    // (trailing backslash) and leak — and cmark parses the leaked block
    // as markdown, pairing each line's `*`s into `\emph`.
    let src = "#' @md\n#' @title T\n#' @details\n\
               #' [Foo*bar\\]]:my_(url) 'title (with parens)'\n#'\n\
               #' [Foo*bar\\]]\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains(
            "(\\details (\\href (VERB \"my_(url)\") (TEXT \"Foo*bar]\")) \
             (TEXT \"[Foo\") (\\emph (TEXT \"bar]: R:Foo\")) (TEXT \"bar%5C [Foo\") \
             (\\emph (TEXT \"bar]: R:Foo\")) (TEXT \"bar%5C\"))"
        ),
        "got: {rd}"
    );
}

#[test]
fn escaped_md_to_source_inverts_double_escape() {
    // Before a bracket the de-dup leaves `2k - 1` backslashes → `(k+1)/2`;
    // elsewhere plain doubling → `k/2`.
    assert_eq!(
        escaped_md_to_source(r"[stop\]: R:stop%5C"),
        r"[stop\]: R:stop%5C"
    );
    assert_eq!(escaped_md_to_source(r"[bad\\\]: x"), r"[bad\\]: x");
    assert_eq!(escaped_md_to_source(r"a\\b"), r"a\b");
    assert_eq!(escaped_md_to_source(r"a\\\\b"), r"a\\b");
}

#[test]
fn leak_skeleton_reconstructs_emphasis_display_source() {
    // The flatten turns `[a\*b\*]` into `[a\b\]` (trailing backslash — a
    // spurious invalid candidate); the leak skeleton re-emits the emphasis
    // delimiters so the candidate stays valid and nothing leaks.
    let display = vec![
        Inline::Text("a\\".to_string()),
        Inline::MdEmphasis {
            strong: false,
            children: vec![Inline::Text("b\\".to_string())],
        },
    ];
    let body = vec![Inline::MdShortcutLink { display }];
    assert_eq!(leak_source_skeleton(&body), "[a\\*b\\*]");
    assert!(leaked_linkref_text(&leak_source_skeleton(&body)).is_empty());
}

#[test]
fn linkref_def_inside_block_quote_defines_and_consumes() {
    // A definition inside a block quote is document-global and consumed in
    // the quote's own block context (cm-220): the outer shortcut resolves
    // to the user `\href` (beating the synthesized `R:` def), and the
    // quote flattens to only its remaining prose (engine-probed).
    let src = "#' @md\n#' @title T\n#' @details\n#' [foo]\n#'\n\
               #' > [foo]: /url\n#' > rest\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\details (\\href (VERB \"/url\") (TEXT \"foo\")) (TEXT \"rest\"))"),
        "got: {rd}"
    );
    // A def-shaped line that does NOT open a quote block stays paragraph
    // prose — a definition cannot interrupt a paragraph (engine-probed:
    // the quote flattens with the def text kept, links resolve to the
    // synthesized `R:` defs).
    let mid = "#' @md\n#' @title T\n#' @details\n#' [foo] and [bar]\n#'\n\
               #' > pre [bar]: /b\n#' > [foo]: /url\n#' > post\n#' @name x\nNULL\n";
    let rd = project_to_rd(mid);
    assert!(
        rd.contains(
            "(\\details (\\link (TEXT \"foo\")) (TEXT \"and\") (\\link (TEXT \"bar\")) \
             (TEXT \"pre bar: /bfoo: /urlpost\"))"
        ),
        "got: {rd}"
    );
}

#[test]
fn in_list_level1_heading_hoists_and_drops() {
    // A level-1 heading inside a list item slices roxygen2's flat Rd string
    // mid-`\itemize{`: the tag piece and the section's own piece both fail
    // rdComplete and empty, so only the hoisted `\section` title survives
    // (cm-302; engine-probed for the same-line, continuation, and setext
    // `===` forms alike).
    for details in [
        "#' - # Foo\n",                             // same-line ATX (cm-302's first item)
        "#' - foo\n#'   # Foo\n",                   // content-column continuation ATX
        "#' - Foo\n#'   ===\n#'   baz\n",           // promoted setext H1
        "#' intro\n#'\n#' - # Foo\n#'\n#' outro\n", // surrounding prose drops too
    ] {
        let src = format!("#' @md\n#' @title T\n#' @details\n{details}#' @name spec\nNULL\n");
        let out = project_to_rd(&src);
        assert!(
            out.contains("(\\section (TEXT \"Foo\"))") && !out.contains("\\details"),
            "for {details:?} got: {out}"
        );
    }
    // Two headings in the *same* list: the piece between them is balanced,
    // so the first section keeps its stranded brace-less `\item`
    // (parse_Rd's unknown macro) while the trailing piece still drops
    // (engine-probed p4).
    let src = "#' @md\n#' @title T\n#' @details\n#' - # A\n#' - # B\n\
               #' @name spec\nNULL\n";
    let out = project_to_rd(src);
    assert!(
        out.contains("(\\section (TEXT \"A\") (UNKNOWN \"\\\\item\"))")
            && out.contains("(\\section (TEXT \"B\"))")
            && !out.contains("\\details"),
        "got: {out}"
    );
}

#[test]
fn in_item_deeper_heading_renders_subsection() {
    // A level >= 2 heading inside a list item becomes a `\subsection` atom
    // after the item's own text, its body the item's following content —
    // `\item bar \subsection{Sub}{baz}` (engine-probed p10/p12); a promoted
    // setext `---` gives the same shape with the paragraph as the title
    // (cm-302's second item, probe p5).
    let src = "#' @md\n#' @title T\n#' @details\n#' - bar\n#'   ## Sub\n#'   baz\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"bar\") \
             (\\subsection (TEXT \"Sub\") (TEXT \"baz\"))))"
        ),
        "got: {}",
        project_to_rd(src)
    );
    let setext = "#' @md\n#' @title T\n#' @details\n#' - Bar\n#'   ---\n#'   baz\n\
                  #' @name spec\nNULL\n";
    assert!(
        project_to_rd(setext).contains(
            "(\\details (\\itemize (\\item) (\\subsection (TEXT \"Bar\") (TEXT \"baz\"))))"
        ),
        "got: {}",
        project_to_rd(setext)
    );
}

#[test]
fn trailing_empty_heading_section_falls_back_to_raw_text() {
    // A trailing level-1 heading whose section body renders empty crashes
    // roxygen2's section splicer (`strsplit` drops the trailing empty
    // piece, `structure(names = )` errors) and `markdown()` returns the
    // **raw** value — the whole field renders unprocessed (cm-010,
    // engine-probed): the heading stays literal `# Foo` prose and earlier
    // markdown (emphasis) stays raw too.
    for (details, want) in [
        ("#' # Foo\n", "(\\details (TEXT \"# Foo\"))"),
        (
            "#' body *raw*\n#'\n#' # Foo\n",
            "(\\details (TEXT \"body *raw* # Foo\"))",
        ),
    ] {
        let src = format!("#' @md\n#' @title T\n#' @details\n{details}#' @name spec\nNULL\n");
        let out = project_to_rd(&src);
        assert!(
            out.contains(want) && !out.contains("\\section"),
            "for {details:?} got: {out}"
        );
    }
    // A trailing `\subsection` (or any non-empty tail) rescues the split:
    // the last piece is non-empty, so the outline renders normally. An
    // *interior* empty level-1 section is kept by `strsplit` and survives
    // too (engine-probed).
    let rescued = "#' @md\n#' @title T\n#' @details\n#' # Foo\n#'\n#' ## Sub\n\
                   #' @name spec\nNULL\n";
    assert!(
        project_to_rd(rescued).contains("(\\section (TEXT \"Foo\") (\\subsection (TEXT \"Sub\")))"),
        "got: {}",
        project_to_rd(rescued)
    );
    let interior = "#' @md\n#' @title T\n#' @details\n#' # Foo\n#'\n#' # Bar\n#' body\n\
                    #' @name spec\nNULL\n";
    let out = project_to_rd(interior);
    assert!(
        out.contains("(\\section (TEXT \"Foo\"))")
            && out.contains("(\\section (TEXT \"Bar\") (TEXT \"body\"))"),
        "got: {out}"
    );
}

#[test]
fn quote_interior_structure_flattens_via_reparse() {
    // roxygen2 renders a quote as `xml_text` over cmark's *parsed* body, so
    // interior markup vanishes: nested `>` markers, a heading's `#`s, and a
    // list bullet all contribute only their text, glued with no separator
    // (`> # Foo` / `> > bar` / `> - baz` → `Foobarbaz`, engine-probed;
    // cm-230/253/237 are the spec pins).
    let src = "#' @md\n#' @title T\n#' @details\n#' > # Foo\n#' > > bar\n#' > - baz\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"Foobarbaz\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn blank_quote_line_ends_lazy_continuation() {
    // A blank `>` line closes the quote's paragraph, so a following unmarked
    // line is not a lazy continuation: the quote ends and the line is a
    // sibling paragraph, joined by roxygen2's `\n\n` paragraph separator
    // (`> bar` / `>` / `baz` → `bar baz`, cm-251).
    let src = "#' @md\n#' @title T\n#' @details\n#' > bar\n#' >\n#' baz\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"bar baz\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn indented_code_in_quote_blocks_laziness() {
    // Laziness continues only a paragraph: after `>     foo` (indented code
    // inside the quote) an unmarked `    bar` does not fold — the quote ends
    // and the line is section-level indented code (cm-238).
    let src = "#' @md\n#' @title T\n#' @details\n#' >     foo\n#'     bar\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (TEXT \"foo\") \
             (\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\"))) \
             (\\preformatted (VERB \"bar\\n\")) \
             (\\if (TEXT \"html\") (\\out (VERB \"</div>\"))))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn overindented_prose_is_lazy_when_quote_paragraph_open() {
    // A >= 4-column line cannot interrupt a paragraph (would-be indented
    // code), so with the quote's paragraph open it folds as lazy paragraph
    // text — the would-be list marker stays literal (`> foo` / `    - bar` →
    // `foo- bar`, cm-240).
    let src = "#' @md\n#' @title T\n#' @details\n#' > foo\n#'     - bar\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"foo- bar\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn indented_list_marker_does_not_interrupt_paragraph() {
    // The same interrupt gate at section level, no quote: a >= 4-column
    // marker line after an open paragraph is lazy prose, not a list
    // (`foo` / `    - bar` → `foo - bar`, engine-probed).
    let src = "#' @md\n#' @title T\n#' @details\n#' foo\n#'     - bar\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"foo - bar\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn empty_item_does_not_fold_block_quote() {
    // An empty item folds no blank-separated block quote: the list ends and
    // the quote is section-level content instead (engine-probed, the same
    // gate as indented code).
    let src = "#' @md\n#' @title T\n#' @details\n#' -\n#'\n#'   > q\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (\\itemize (\\item)) (TEXT \"q\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn fenced_code_block_body_is_one_verb_per_line() {
    // parse_Rd splits a `\preformatted` body into one `VERB` leaf per source
    // line (each carrying its trailing `\n`), never one glued atom.
    let src = "#' @md\n#' @title T\n#' @details\n#' ```\n#' aaa\n#' bbb\n#' ```\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\preformatted (VERB \"aaa\\n\") (VERB \"bbb\\n\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn empty_fenced_code_block_has_no_verb_child() {
    // An empty fenced code block yields a childless `\preformatted` (parse_Rd
    // emits no `VERB` leaf for an empty body).
    let src = "#' @md\n#' @title T\n#' @details\n#' ```\n#' ```\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\"))) \
             (\\preformatted) \
             (\\if (TEXT \"html\") (\\out (VERB \"</div>\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn tilde_fence_opens_a_code_block() {
    // A run of three-plus tildes is a code fence too, and a tilde fence's
    // info string may contain backticks and tildes (CommonMark 4.5); the
    // info lands in the `<div>` class after `sourceCode`.
    let src = "#' @md\n#' @title T\n#' @details\n#' ~~~ aa ``` ~~~\n#' foo\n#' ~~~\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode aa ``` ~~~\\\">\"))) \
             (\\preformatted (VERB \"foo\\n\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn non_matching_fence_lines_are_content() {
    // Only a matching fence closes a block: a shorter run, a different fence
    // character, an info-string-bearing fence, or one indented four-plus
    // columns is verbatim content (CommonMark 4.5).
    let src = "#' @md\n#' @title T\n#' @details\n#' ````\n#' ~~~~\n#' ```\n#' ``` bbb\n\
               #'     ````\n#' ``````\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\preformatted (VERB \"~~~~\\n\") (VERB \"```\\n\") \
             (VERB \"``` bbb\\n\") (VERB \"    ````\\n\"))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn unterminated_fenced_block_keeps_its_last_line() {
    // An unterminated block runs to the section end; its last line is code,
    // not a dropped closer (the closer test decides, not position).
    let src = "#' @md\n#' @title T\n#' @details\n#' `````\n#'\n#' ```\n#' aaa\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src)
            .contains("(\\preformatted (VERB \"\\n\") (VERB \"```\\n\") (VERB \"aaa\\n\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn fence_below_content_column_ends_the_list() {
    // A fenced code block *below* the item's content column is a section-level
    // block, not part of the item: the list ends at the `\item` and the code
    // block is a sibling of the `\itemize`.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#' ```\n#' code\n#' ```\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\")) \
             (\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn folded_fence_strips_only_its_own_indentation() {
    // CommonMark removes up to the opener fence's indentation from each content
    // line, so a body line indented *past* the fence keeps the surplus: fence at
    // column 3, body at column 5 → `  code` (two leading spaces survive).
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'    ```\n#'      code\n#'    ```\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\preformatted (VERB \"  code\\n\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn indented_code_at_content_column_folds_into_item() {
    // A blank-separated line indented four columns past the item's content
    // column (2 + 4 = 6) is an indented code block inside the item, projecting
    // to the same three-atom sequence as a fenced block, with `- b` a sibling.
    // The item's content column is stripped on top of CommonMark's four, so the
    // code renders flush (`code`, no leading spaces).
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#'       code\n\
               #' - b\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\") \
             (\\if (TEXT \"html\") (\\out (VERB \"<div class=\\\"sourceCode\\\">\"))) \
             (\\preformatted (VERB \"code\\n\")) \
             (\\if (TEXT \"html\") (\\out (VERB \"</div>\"))) (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn indented_code_without_blank_is_lazy_continuation() {
    // A CommonMark indented code block cannot interrupt a paragraph, so an
    // over-indented line *immediately* after the item text (no blank) is a lazy
    // paragraph continuation folded into the item (`a code`), not a code block.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'       code\n\
               #' @name spec\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\details (\\itemize (\\item) (TEXT \"a code\")))"),
        "got: {rd}"
    );
    assert!(!rd.contains("\\preformatted"), "got: {rd}");
}

#[test]
fn folded_indented_code_keeps_surplus_indentation() {
    // Only the item's content column plus CommonMark's four are stripped, so a
    // code line indented *past* that threshold keeps the surplus: content column
    // 2, strip 6, a line at markdown column 8 → `  code` (two spaces survive).
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#'         code\n\
               #' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\preformatted (VERB \"  code\\n\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn table_at_content_column_folds_into_item() {
    // A GFM table indented to the item's content column folds into the item as
    // a `\tabular` child (between the two `\item`s), with `- b` a sibling. A
    // blank line before the table only makes the item loose.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#'   | x | y |\n\
               #'   | --- | --- |\n#'   | 1 | 2 |\n#' - b\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\") \
             (\\tabular (TEXT \"ll\") (GRP (TEXT \"x\") (\\tab) (TEXT \"y\") (\\cr) \
             (TEXT \"1\") (\\tab) (TEXT \"2\") (\\cr))) (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn table_without_blank_folds_into_item() {
    // A GFM table interrupts the item's paragraph at the content column, so it
    // folds in with *no* intervening blank line too (same `\tabular` shape).
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'   | x | y |\n\
               #'   | --- | --- |\n#'   | 1 | 2 |\n#' - b\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\") \
             (\\tabular (TEXT \"ll\") (GRP (TEXT \"x\") (\\tab) (TEXT \"y\") (\\cr) \
             (TEXT \"1\") (\\tab) (TEXT \"2\") (\\cr))) (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn unindented_table_is_lazy_continuation() {
    // A table header *below* the item's content column cannot interrupt the
    // item's paragraph across the container boundary, so the whole table folds
    // in as lazy paragraph-continuation prose (`a | x | y | ...`), not a
    // `\tabular` — engine-probed.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#' | x | y |\n\
               #' | --- | --- |\n#' | 1 | 2 |\n#' @name spec\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains(
            "(\\details (\\itemize (\\item) (TEXT \"a | x | y | | --- | --- | | 1 | 2 |\")))"
        ),
        "got: {rd}"
    );
    assert!(!rd.contains("\\tabular"), "got: {rd}");
}

#[test]
fn block_macro_at_content_column_folds_into_item() {
    // A block Rd macro (`\itemize{…}`) at the item's content column is not a
    // markdown block — cmark passes the raw Rd through as the item's paragraph
    // text — so it folds into the item as a nested `\itemize` child (between the
    // two `\item`s), with `- b` a sibling.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'   \\itemize{\n\
               #'     \\item x\n#'   }\n#' - b\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\") \
             (\\itemize (\\item) (TEXT \"x\")) (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn block_macro_below_content_column_folds_lazily() {
    // With no intervening blank, a block macro folds as a *lazy* paragraph
    // continuation regardless of indent (CommonMark paragraph continuation does
    // not require the content column), so a macro indented *below* the content
    // column still nests — same shape as the content-column case.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#' \\itemize{\n\
               #'   \\item x\n#' }\n#' - b\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\") \
             (\\itemize (\\item) (TEXT \"x\")) (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn blank_separated_below_column_block_macro_ends_list() {
    // A blank line closes the item's paragraph; a following block macro *below*
    // the content column then cannot belong to the item, so it is a
    // section-level block that ends the list — `- a`, the `\itemize`, and `- b`
    // become three separate `\itemize`s (engine-probed).
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#'\n#' \\itemize{\n\
               #'   \\item x\n#' }\n#' - b\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\")) \
             (\\itemize (\\item) (TEXT \"x\")) (\\itemize (\\item) (TEXT \"b\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn setext_underline_folds_into_block_quote_lazily() {
    // A setext underline (`===`) cannot be a lazy continuation *underline* in a
    // block quote (CommonMark), so it never promotes the quote's paragraph into
    // a heading; it folds in as ordinary paragraph-continuation text. roxygen2
    // has no block-quote support, so the whole quote flattens to one `(TEXT …)`
    // with no separators: `> foo` + `===` -> `foo===` (engine-probed).
    let src = "#' @md\n#' @title T\n#' @details\n#' > foo\n#' ===\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"foo===\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn inline_link_title_is_dropped_from_href() {
    // cmark parses an inline link's `(dest title)` and roxygen2 renders only
    // the destination into `\href`; the title is discarded, whatever its quote
    // form (all engine-probed).
    let cases = [
        (
            "[t](https://ex.org \"the title\")",
            "(VERB \"https://ex.org\")",
        ),
        (
            "[t](https://ex.org 'the title')",
            "(VERB \"https://ex.org\")",
        ),
        (
            "[t](https://ex.org (the title))",
            "(VERB \"https://ex.org\")",
        ),
        // No whitespace before the quote: it stays part of the destination.
        ("[t](url\"x\")", "(VERB \"url\\\"x\\\"\")"),
        // Angle-bracketed destination (may contain spaces), title dropped.
        (
            "[t](<https://ex.org/a b> \"x\")",
            "(VERB \"https://ex.org/a b\")",
        ),
        // The destination is entity-decoded like a reference definition's.
        (
            "[t](https://ex.org/a?x&amp;y)",
            "(VERB \"https://ex.org/a?x&y\")",
        ),
    ];
    for (link, want) in cases {
        let src = format!("#' T\n#'\n#' @md\n#' @details\n#' {link}\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(
            rd.contains(&format!("(\\href {want} (TEXT \"t\"))")),
            "link {link:?}: want href with {want}, got: {rd}"
        );
    }
}

#[test]
fn invalid_inline_dest_falls_back_to_shortcut() {
    // A bare inline destination may not contain ASCII whitespace (a backslash
    // never escapes whitespace), and a stray space before non-title text
    // invalidates the link — cmark then leaves the `[…]` a shortcut reference
    // (`\link`) and the `(…)` literal prose. A valid destination (percent-
    // encoded, angle-bracketed, or titled) still links.
    let invalid = [
        ("[t](a\\ b)", "(\\link (TEXT \"t\")) (TEXT \"(a\\\\ b) z\")"),
        ("[t](a b c)", "(\\link (TEXT \"t\")) (TEXT \"(a b c) z\")"),
        ("[t](url ok)", "(\\link (TEXT \"t\")) (TEXT \"(url ok) z\")"),
    ];
    for (link, want) in invalid {
        let src = format!("#' T\n#'\n#' @md\n#' @details\n#' {link} z\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(
            rd.contains(want),
            "link {link:?}: want shortcut fallback {want}, got: {rd}"
        );
    }
    // A valid destination still resolves to an inline link.
    for (link, want) in [
        ("[t](a%20b)", "(\\href (VERB \"a%20b\") (TEXT \"t\"))"),
        ("[t](<a b>)", "(\\href (VERB \"a b\") (TEXT \"t\"))"),
        ("[t](url \"x\")", "(\\href (VERB \"url\") (TEXT \"t\"))"),
    ] {
        let src = format!("#' T\n#'\n#' @md\n#' @details\n#' {link} z\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(rd.contains(want), "link {link:?}: want {want}, got: {rd}");
    }
}

#[test]
fn inline_dest_parity_mirrors_cmark_after_double_escape() {
    // roxygen2 runs `double_escape_md` before cmark, so every source backslash
    // reaches cmark self-paired (literal) and never escapes a paren, an angle
    // bracket, or a title quote; and cmark's destination whitespace is ASCII
    // only (a U+00A0 is destination content). All engine-probed (CommonMark
    // spec examples 489/494/495/500/501/509).
    let cases = [
        // An angle-bracketed destination may contain parens.
        ("[t](<b)c>)", "(\\href (VERB \"b)c\") (TEXT \"t\"))"),
        (
            "[t](<foo(and(bar)>)",
            "(\\href (VERB \"foo(and(bar)\") (TEXT \"t\"))",
        ),
        // A bare destination counts every paren raw (`\(` is literal `\` then
        // an active paren), so this never balances: not a link.
        (
            "[t](foo\\(and\\(bar\\))",
            "(\\link (TEXT \"t\")) (TEXT \"(foo\\\\(and\\\\(bar\\\\))\")",
        ),
        // A U+00A0 is not ASCII whitespace: the whole run is the destination
        // (no title separation).
        (
            "[t](/url\u{a0}\"title\")",
            "(\\href (VERB \"/url\u{a0}\\\"title\\\"\") (TEXT \"t\"))",
        ),
        // An empty destination with empty text renders roxygen2's `\url{}`.
        ("[]()", "(\\url)"),
    ];
    for (link, want) in cases {
        let src = format!("#' T\n#'\n#' @md\n#' @details\n#' {link}\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(rd.contains(want), "link {link:?}: want {want}, got: {rd}");
    }
    // `<foo\>` closes at the `>` (the backslash is literal), so cmark's
    // destination is `foo\` — a trailing backslash that escapes the `\href`
    // brace, dropping the whole section.
    let src = "#' T\n#'\n#' @md\n#' @details\n#' [t](<foo\\>)\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\details)"),
        "angle dest with trailing backslash must drop the section, got: {rd}"
    );
}

#[test]
fn cross_line_inline_destination_links() {
    // A CommonMark inline link's `(…)` may span a soft line break: each gap
    // between the destination, the title, and the closing `)` admits "spaces,
    // tabs, and up to one line ending" (spec example 512), and a title itself
    // may contain line endings. roxygen2 renders the usual `\href` (title
    // dropped); anything after the closing `)` stays literal prose. All
    // engine-probed.
    let cases = [
        // cm-512: destination on the opening line, title on the next.
        (
            "#' [link](   /uri\n#'   \"title\"  )",
            "(\\details (\\href (VERB \"/uri\") (TEXT \"link\")))",
        ),
        // Leftover text after the cross-line closer stays literal prose.
        (
            "#' [link](   /uri\n#'   \"title\"  ) tail text",
            "(\\details (\\href (VERB \"/uri\") (TEXT \"link\")) (TEXT \"tail text\"))",
        ),
        // A title may span the soft break.
        (
            "#' [a](/u \"t1\n#' t2\") z",
            "(\\details (\\href (VERB \"/u\") (TEXT \"a\")) (TEXT \"z\"))",
        ),
        // The whole `(…)` on the continuation gap: destination on line two.
        (
            "#' [link](\n#' /uri)",
            "(\\details (\\href (VERB \"/uri\") (TEXT \"link\")))",
        ),
        // Non-title junk after the destination invalidates the link: the
        // bracket falls back to a shortcut and the `(…)` stays literal.
        (
            "#' [link](   /uri\n#' junk more)",
            "(\\details (\\link (TEXT \"link\")) (TEXT \"( /uri junk more)\"))",
        ),
    ];
    for (body, want) in cases {
        let src = format!("#' T\n#'\n#' @md\n#' @details\n{body}\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(rd.contains(want), "body {body:?}: want {want}, got: {rd}");
    }
}

#[test]
fn collapsed_ref_link_resolves_label_from_display() {
    // A collapsed reference `[text][]` (CommonMark) takes its label from the
    // display. A user definition resolves it to `\href` with the display kept
    // (label match is case-insensitive); a label that is a shortcut candidate
    // elsewhere in the field resolves through the synthesized `R:label`
    // definition exactly like a shortcut (`\link`); an undefined label stays
    // literal source text.
    let defined =
        "#' T\n#'\n#' @md\n#' @details\n#' a [Foo][] b\n#'\n#' [foo]: /url\n#' @name x\nNULL\n";
    let rd = project_to_rd(defined);
    assert!(
        rd.contains("(\\href (VERB \"/url\") (TEXT \"Foo\"))"),
        "user-defined collapsed link: got {rd}"
    );
    let synthesized =
        "#' T\n#'\n#' @md\n#' @details\n#' a [foo][] b and [foo] c\n#' @name x\nNULL\n";
    let rd = project_to_rd(synthesized);
    assert!(
        rd.contains("(\\link (TEXT \"foo\")) (TEXT \"b and\") (\\link (TEXT \"foo\"))"),
        "candidate-synthesized collapsed link: got {rd}"
    );
    let undefined = "#' T\n#'\n#' @md\n#' @details\n#' a [nope][] b\n#' @name x\nNULL\n";
    let rd = project_to_rd(undefined);
    assert!(
        rd.contains("(TEXT \"a [nope][] b\")"),
        "undefined collapsed link stays literal: got {rd}"
    );
}

#[test]
fn shortcut_and_reference_images_resolve_to_synthesized_figures() {
    // A shortcut image `![alt]` and a reference image `![alt][ref]` (with no
    // user-defined destination) resolve against roxygen2's synthesized
    // `[label]: R:label` reference definition, so both become `\figure{R:label}`
    // — the shortcut keyed on its alt, the reference on its label.
    for (image, want) in [
        ("![x]", "(\\figure (VERB \"R:x\"))"),
        ("![y][ref]", "(\\figure (VERB \"R:ref\"))"),
    ] {
        let src = format!("#' T\n#'\n#' @md\n#' @details\n#' a {image} b\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(rd.contains(want), "image {image:?}: want {want}, got: {rd}");
    }
    // An invalid inline destination `(a\ b)` is not consumed: the `![z]` stays a
    // shortcut image and the `(a\ b)` is left literal prose.
    let src = "#' T\n#'\n#' @md\n#' @details\n#' see ![z](a\\ b) end\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\figure (VERB \"R:z\")) (TEXT \"(a\\\\ b) end\")"),
        "got: {rd}"
    );
    // A collapsed `![alt][]` and an empty `![]` are not images (no synthesized
    // definition) — they stay literal prose.
    for image in ["![alt][]", "![]"] {
        let src = format!("#' T\n#'\n#' @md\n#' @details\n#' {image} z\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(
            !rd.contains("\\figure"),
            "image {image:?} should stay literal, got: {rd}"
        );
    }
}

#[test]
fn user_defined_image_refs_override_synthesized_destination() {
    // A reference/shortcut image whose label has a user-written `[label]: url`
    // definition resolves to that URL (not the synthesized `R:label`), and the
    // image-format wrapping still applies (an `svg` destination -> `\if{html}`).
    for (image, def, want) in [
        (
            "![a][ref]",
            "[ref]: https://example.com/img.png",
            "(\\figure (VERB \"https://example.com/img.png\"))",
        ),
        (
            "![pic]",
            "[pic]: https://example.com/pic.gif",
            "(\\figure (VERB \"https://example.com/pic.gif\"))",
        ),
        (
            "![d][s]",
            "[s]: diagram.svg",
            "(\\if (TEXT \"html\") (\\figure (VERB \"diagram.svg\")))",
        ),
    ] {
        let src =
            format!("#' T\n#'\n#' @md\n#' @details x {image} y\n#'\n#' {def}\n#' @name x\nNULL\n");
        let rd = project_to_rd(&src);
        assert!(rd.contains(want), "image {image:?}: want {want}, got: {rd}");
    }
    // An undefined reference label still falls back to the synthesized `R:label`.
    let src = "#' T\n#'\n#' @md\n#' @details a ![y][ref] b\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\figure (VERB \"R:ref\"))"),
        "got: {}",
        project_to_rd(src)
    );
}

#[test]
fn user_def_title_reaches_figure() {
    // roxygen2's `mdxml_image` keeps a definition's title as `\figure`'s
    // second argument (cm-590: `![foo]` + `[foo]: /url "title"` →
    // `\figure{/url}{title}`); `mdxml_link` ignores it, so a *link* through
    // the same definition renders a plain `\href{/url}{…}`.
    let src = "#' T\n#'\n#' @md\n#' @details a ![foo] and [foo] b\n#'\n\
               #' [foo]: /url \"title\"\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\figure (VERB \"/url\") (VERB \"title\"))"),
        "image keeps the def title, got: {rd}"
    );
    assert!(
        rd.contains("(\\href (VERB \"/url\") (TEXT \"foo\"))"),
        "link drops the def title, got: {rd}"
    );
}

#[test]
fn collapsed_image_resolves_only_via_user_def() {
    // A collapsed reference image `![alt][]` resolves by its alt-as-label —
    // but only through a *user* definition (cm-586): the collapsed
    // occurrence's own `[alt]` candidate is blocked by `get_md_linkrefs`'
    // `(?=[^\[{])` lookahead, so no `R:alt` definition is synthesized and an
    // undefined one stays literal cmark text, glued into the prose run.
    let src = "#' T\n#'\n#' @md\n#' @details a ![pic][] and ![nope][] b\n#'\n\
               #' [pic]: img.png \"A picture\"\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\figure (VERB \"img.png\") (VERB \"A picture\"))"),
        "defined collapsed image resolves with title, got: {rd}"
    );
    assert!(
        rd.contains("(TEXT \"and ![nope][] b\")"),
        "undefined collapsed image stays literal in the run, got: {rd}"
    );
}

#[test]
fn emphasis_label_image_matches_flattened_def() {
    // An emphasis-bearing label's definition arrives as a resolved display
    // and is keyed by its flatten; the image's raw label must flatten the
    // same way to find it (cm-575 shortcut, cm-578 collapsed). Undefined,
    // the shortcut still synthesizes from the *raw* label (`R:foo%20*bar*`).
    let src = "#' T\n#'\n#' @md\n#' @details x ![foo *bar*] and ![*baz* qux][] y\n#'\n\
               #' [foo *bar*]: train.jpg \"train & tracks\"\n\
               #' [*baz* qux]: pic.png\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\figure (VERB \"train.jpg\") (VERB \"train & tracks\"))"),
        "shortcut image with emphasis label resolves, got: {rd}"
    );
    assert!(
        rd.contains("(\\figure (VERB \"pic.png\"))"),
        "collapsed image with emphasis label resolves, got: {rd}"
    );
    let undefined = "#' T\n#'\n#' @md\n#' @details x ![foo *bar*] y\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(undefined).contains("(\\figure (VERB \"R:foo%20*bar*\"))"),
        "got: {}",
        project_to_rd(undefined)
    );
}

#[test]
fn multiline_linkref_def_consumes_label_dest_and_title_lines() {
    // cmark parses a link-reference definition at the block level, before
    // inline resolution: the label, destination, and title may each sit on
    // their own line (cm-197 — the next-line `<my url>` destination reads as
    // raw HTML to the inline pass and must be regathered as raw source), and
    // the label itself may span lines (cm-210 — the def consumes through the
    // destination's line, later prose stays).
    let src = "#' @md\n#' @title T\n#' @details\n#' [Foo bar]:\n#' <my url>\n#' 'title'\n\
               #'\n#' [Foo bar]\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(\\details (\\href (VERB \"my url\") (TEXT \"Foo bar\")))"),
        "multi-line def resolves the shortcut, got: {rd}"
    );
    let cross = "#' @md\n#' @title T\n#' @details\n#' [\n#' foo\n#' ]: /url\n#' bar\n\
                 #' @name x\nNULL\n";
    assert!(
        project_to_rd(cross).contains("(\\details (TEXT \"bar\"))"),
        "cross-line label def is consumed, prose after it stays, got: {}",
        project_to_rd(cross)
    );
}

#[test]
fn invalid_next_line_title_falls_back_to_dest_only_def() {
    // A title on the line after the destination that is followed by junk (or
    // never closes) fails the *title*, not the definition: cmark backtracks
    // to the destination-only form ending at the destination's line, and the
    // title line stays prose (cm-212). Junk after a title on the
    // *destination's* line fails the whole definition (cm-209's tail rule).
    let src = "#' @md\n#' @title T\n#' @details\n#' [foo]: /url\n#' \"title\" ok\n\
               #'\n#' [foo]\n#' @name x\nNULL\n";
    let rd = project_to_rd(src);
    assert!(
        rd.contains("(TEXT \"\\\"title\\\" ok\") (\\href (VERB \"/url\") (TEXT \"foo\"))"),
        "dest-only fallback, title line stays prose, got: {rd}"
    );
    let same_line = "#' @md\n#' @title T\n#' @details\n#' [bar]: /b 'title' junk\n\
                     #'\n#' [bar]\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(same_line).contains("(\\link (TEXT \"bar\")) (TEXT \": /b 'title' junk\")"),
        "same-line junk fails the whole def, got: {}",
        project_to_rd(same_line)
    );
}

#[test]
fn linkref_def_dest_parity_mirrors_cmark_after_double_escape() {
    // Destination edges after `double_escape_md`: an empty angle destination
    // `<>` is a valid definition (its empty-URL link renders roxygen2's
    // `\url{display}`, cm-202); a backslash-bearing bare destination is
    // verbatim — no source backslash escapes anything — and its same-line
    // title closes longest-match at the last escapable quote (cm-204); an
    // unmatched `)` in a bare destination fails the definition.
    let empty = "#' @md\n#' @title T\n#' @details\n#' [foo]: <>\n#'\n#' [foo]\n\
                 #' @name x\nNULL\n";
    assert!(
        project_to_rd(empty).contains("(\\details (\\url (VERB \"foo\")))"),
        "empty angle dest defines, got: {}",
        project_to_rd(empty)
    );
    let escapes = "#' @md\n#' @title T\n#' @details\n\
                   #' [foo]: /url\\bar\\*baz \"foo\\\"bar\\baz\"\n#'\n#' [foo]\n\
                   #' @name x\nNULL\n";
    assert!(
        project_to_rd(escapes)
            .contains("(\\details (\\href (VERB \"/url\\\\bar\\\\*baz\") (TEXT \"foo\")))"),
        "backslash dest is verbatim, longest-match title, got: {}",
        project_to_rd(escapes)
    );
    let unbalanced = "#' @md\n#' @title T\n#' @details\n#' [foo]: /url)x\n#'\n#' [foo]\n\
                      #' @name x\nNULL\n";
    assert!(
        project_to_rd(unbalanced).contains("(\\link (TEXT \"foo\")) (TEXT \": /url)x\")"),
        "unmatched `)` fails the def, got: {}",
        project_to_rd(unbalanced)
    );
}

#[test]
fn linkref_def_in_list_item_is_consumed() {
    // A link-reference definition as a blank-separated second block of a
    // list item is a definition, not prose: cmark consumes it and the item
    // keeps only its first paragraph (cm-319). Inside an item the paragraph
    // break is a blank `#'` line — two adjacent SOFT_BREAK inlines — not a
    // `\n`-bearing `Text`, so the block-start scan must read that shape too.
    let src = "#' @md\n#' @title T\n#' @details\n#' - a\n#' - b\n#'\n\
               #'   [ref]: /url\n#' - d\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(src).contains(
            "(\\details (\\itemize (\\item) (TEXT \"a\") (\\item) (TEXT \"b\") \
             (\\item) (TEXT \"d\")))"
        ),
        "got: {}",
        project_to_rd(src)
    );
    // The definition it collects resolves a reference elsewhere in the
    // field: `[x][ref]` renders `\href{/url}{x}`, not the synthesized
    // `R:ref` topic link.
    let referencing = "#' @md\n#' @title T\n#' @details [x][ref]\n#'\n#' - b\n#'\n\
                       #'   [ref]: /url\n#' @name spec\nNULL\n";
    assert!(
        project_to_rd(referencing).contains("(\\href (VERB \"/url\") (TEXT \"x\"))"),
        "got: {}",
        project_to_rd(referencing)
    );
}

#[test]
fn thematic_break_ends_block_quote() {
    // A `---` (three-plus dash run) is a thematic break, which *interrupts* a
    // paragraph, so it is not a lazy continuation: it ends the quote (and
    // renders empty). Only `foo` remains in the flattened quote.
    let src = "#' @md\n#' @title T\n#' @details\n#' > foo\n#' ---\n#' @name x\nNULL\n";
    assert!(
        project_to_rd(src).contains("(\\details (TEXT \"foo\"))"),
        "got: {}",
        project_to_rd(src)
    );
}
