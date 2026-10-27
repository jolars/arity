\usage{foo(x = "}", y = '\\n', z = `a}b`)
\method{foo}{bar}(x)
\S3method{foo}{bar}(x)
\S4method{foo}{bar}(x)}
\examples{
x <- function() { 1 }
y <- r"---({\notmacro}%raw)---"
z <- R'{raw } " \foo{x}}'
# R comment with balanced {braces} and \emph{literal}
\donttest{foo(1)}
\dontshow{foo(2)}
\dontdiff{foo(3)}
\testonly{foo(4)}
\dontrun{foo("unbalanced) \emph{literal}}
}
\details{\code{f(\var{x}, "\link{foo}", "\emph{literal}")} \special{...}}
\RdOpts{stage=render, echo=FALSE}
\Sexpr[results=rd,stage=render]{stop("not evaluated")}
