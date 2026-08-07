# Within-block same-head section collapse: roxygen2 merges repeated same-type
# sections inside ONE block exactly as across blocks (`RoxyTopic$add` appends
# each `rd_section`'s value; the per-type `format` then renders) — repeated
# COLLAPSE_HEADS tags join into one macro (`format_collapse`), repeated
# `@title`s keep the first (`format_first`).

# Two @details values collapse into one \details, space-joined.
#' Title one
#'
#' @details first detail
#' @details second detail
#' @name collapse_details
NULL

# Repeated @seealso, @note, and @source each collapse; multi-paragraph values.
#' Title two
#'
#' @seealso one
#' @seealso two
#' @note n1
#'
#' second para of n1
#' @note n2
#' @source s1
#' @source s2
#' @name collapse_many
NULL

# Repeated @title: format_first keeps the first explicit value; the intro
# paragraph shifts to \description (the explicit tag claims the title role).
#' Intro paragraph
#'
#' @title kept title
#' @title dropped title
#' @name title_first
NULL

# Repeated @title with no description anywhere: the title-as-description
# fallback reuses the WHOLE title value vector, collapsed.
#' @title fallback one
#' @title fallback two
#' @name title_fallback
NULL

# md-on: markdown runs per tag value BEFORE the join — the heading in the
# first @details hoists its own \section without swallowing the second value.
#' Title five
#'
#' @md
#' @details d1 body
#'
#' # Head
#'
#' under heading
#' @details d2 body
#' @name collapse_md_heading
NULL

# The title-as-description fallback is per-topic: this block's fallback fires
# even though an earlier topic in the file already has a \description.
#' @title lone title
#' @name fallback_scoped
NULL

# Leftover intro paragraphs raw-join with EVERY explicit @details into one tag
# (`parse_description`), a different regime from the per-tag collapse above;
# the collapse pass must not disturb the already-merged single \details.
#' Title seven
#'
#' Desc seven
#'
#' intro tail
#' @details tag a
#' @details tag b
#' @name intro_raw_join
NULL
