\description{\link[pkg\%:topic]{label} \code{"a\"b"} \code{"a\\"b"} \code{"a\\\"b"} \code{"a\\\\" + x}}
\examples{
x <- "a\%b"
# A comment with % literal and \{escaped\} braces.
y <- r"({a} % \unknown{x})"
z <- "start
#ifdef unix
body
#endif
end"
}
