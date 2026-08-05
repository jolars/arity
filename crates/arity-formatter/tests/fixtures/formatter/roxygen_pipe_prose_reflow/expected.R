#' @param .id `r lifecycle::badge("deprecated")`: convert
#'   `df |> unnest(x, .id = "id")` to `df |> mutate(id = names(x)) |>
#'   unnest(x))`.
NULL
