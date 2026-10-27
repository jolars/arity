#ifdef unix
\title{Unix title}
#ifndef windows
\description{Unix body.}
#endif
#endif
\examples{
#ifdef unix
x <- { 1 }
#endif
}
\details{Before.
#ifdef html
\emph{HTML}
#endif
After.}
