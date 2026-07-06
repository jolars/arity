#' @param data The data to be displayed in this layer. There are three options:
#' * `NULL` (default): the data is inherited from the plot data as specified
#' in the call to [ggplot()].
#' * A `data.frame`, or other object, will override the plot data.
#'
#' @param geom The geometric object to use to display the data for this layer.
#'   When using a `stat_*()` function to construct a layer, the `geom` argument
#'   can be used.
#' * A `Geom` ggproto subclass, for example `GeomPoint`.
#' * A string naming the geom, stripped of the `geom_` prefix.
f <- function(data, geom) data
