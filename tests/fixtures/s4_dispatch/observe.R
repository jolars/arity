args <- commandArgs(trailingOnly = TRUE)
root <- if (length(args)) args[[1L]] else "."
env <- new.env(parent = globalenv())
env$.packageName <- ".GlobalEnv"
emit <- function(...) cat(paste(..., sep = "\t"), "\n", sep = "")
packed <- function(x) if (length(x)) paste(x, collapse = ",") else "-"
expressions <- parse(file.path(root, "input.R"))
for (expression in expressions) {
    invisible(suppressMessages(eval(expression, env)))
    if (!is.call(expression)) next
    callee <- paste(deparse(expression[[1L]]), collapse = "")
    if (callee == "methods::setGeneric") {
        name <- eval(expression[[2L]], env)
        generic <- methods::getGeneric(name, where = env)
        default <- generic@default
        emit("generic", name, generic@package, packed(generic@signature),
             packed(generic@group), if (is.null(default)) "-" else packed(names(formals(default))))
    } else if (callee %in% c("methods::setMethod", "methods::setReplaceMethod")) {
        call <- match.call(methods::setMethod, expression)
        name <- eval(call$f, env)
        if (callee == "methods::setReplaceMethod") name <- paste0(name, "<-")
        signature <- if (is.null(call$signature)) character() else eval(call$signature, env)
        generic <- methods::getGeneric(name, where = env)
        method <- methods::getMethod(name, signature, where = env)
        defined <- setNames(rep("ANY", length(generic@signature)), generic@signature)
        defined[names(method@defined)] <- as.character(method@defined)
        emit("method", name, generic@package, packed(paste(names(defined), defined, sep = ":")))
    }
}
for (name in c("Ops", "Arith", "Compare", "Logic", "Math", "Math2", "Summary", "Complex", "matrixOps")) {
    generic <- methods::getGeneric(name)
    emit("group", name, generic@package, packed(generic@signature), packed(generic@group))
}
# The forwarded value has class character, but selection still uses the current
# method's declared numeric/numeric signature when finding the next method.
stopifnot(identical(env$s4Next(1, 2), "numeric"))
# selectMethod ignores names, whereas setMethod uses partial argument matching.
selected <- methods::selectMethod("s4Partial", methods::signature(other = "numeric", object = "character"),
                                 useInherited = FALSE,
                                 fdef = methods::getGeneric("s4Partial", where = env))
stopifnot(identical(as.character(selected@defined), c("numeric", "character")))
stopifnot(!is.null(methods::getGeneric("s4Qualified", where = env)@default))
generic <- methods::getGeneric("s4Promoted", where = env)
emit("generic", "s4Promoted", generic@package, packed(generic@signature),
     packed(generic@group), packed(names(formals(generic@default))))

owner <- new.env(parent = globalenv())
owner$.packageName <- "one"
invisible(methods::setGeneric("s4Owned", function(x, y) standardGeneric("s4Owned"),
                              where = owner))
installer <- new.env(parent = owner)
installer$.packageName <- "two"
invisible(eval(quote(methods::setMethod("s4Owned", c(x = "numeric", y = "missing"),
                                      function(x, y) x, where = installer)), installer))
generic <- methods::getGeneric("s4Owned", where = installer)
method <- methods::getMethod("s4Owned", c(x = "numeric", y = "missing"), where = installer)
emit("owner", generic@package, environment(method)$.packageName)

# Keep the regression observations executable without changing the emitted table.
local({
    scope <- function(package = ".GlobalEnv", parent = globalenv()) {
        env <- new.env(parent = parent)
        env$.packageName <- package
        env
    }
    promoted <- scope()
    invisible(evalq(methods::setGeneric("mean"), promoted))
    stopifnot(identical(promoted$mean@package, "base"))

    for (definition in list(quote(function(x, ...) standardGeneric("mean")),
                            quote(function(x, y) standardGeneric("mean")))) {
        for (package in list(NULL, "base")) {
            external <- scope()
            invisible(suppressMessages(eval(substitute(methods::setGeneric("mean", DEF, package = PKG),
                                                       list(DEF = definition, PKG = package)), external)))
            expected <- if (identical(names(formals(external$mean)), c("x", "..."))) "base" else ".GlobalEnv"
            stopifnot(identical(external$mean@package, expected))
        }
    }

    for (original in list(quote(methods::setGeneric("g", function(x) x + 10)),
                          quote(methods::setGeneric("g", function(x) standardGeneric("g"),
                                                    useAsDefault = function(x) x + 10)))) {
        for (replacement in list(quote(function(x) standardGeneric("g")),
                                 quote(function(x = 1) standardGeneric("g")),
                                 quote(function(y) standardGeneric("g")))) {
            for (disabled in c(FALSE, TRUE)) {
                redefined <- scope()
                invisible(eval(original, redefined))
                invisible(suppressMessages(eval(substitute(methods::setGeneric("g", DEF, useAsDefault = DEFAULT),
                                                           list(DEF = replacement, DEFAULT = if (disabled) FALSE else NULL)), redefined)))
                retains <- !disabled && identical(names(formals(eval(replacement))), "x")
                stopifnot(identical(!is.null(redefined$g@default), retains))
                if (retains) stopifnot(identical(redefined$g(1), 11))
            }
        }
    }

    options <- scope()
    invisible(evalq({
        methods::setGeneric("s4Options", function(x, y) standardGeneric("s4Options"),
                            signature = "y", useAsDefault = function(x, y) y)
        methods::setGeneric("s4Options", signature = "x", useAsDefault = FALSE)
    }, options))
    stopifnot(identical(options$s4Options@signature, "x"), is.null(options$s4Options@default))

    implicit <- scope()
    invisible(evalq({
        s4Implicit <- function(x) x
        methods::setMethod("s4Implicit", "numeric", function(x) x)
        captured <- s4Implicit
        s4Implicit <- function(a, b) a
    }, implicit))
    stopifnot(identical(implicit$captured@signature, "x"),
              identical(names(formals(implicit$captured@default)), "x"))

    rebound <- scope()
    invisible(evalq({
        g <- function(x) x
        methods::setMethod("g", "numeric", function(x) x)
        first <- g
        g <- function(y) y
        methods::setMethod("g", "character", function(y) y)
    }, rebound))
    stopifnot(identical(rebound$first@signature, "x"),
              identical(rebound$g@signature, "y"))

    dynamic <- scope()
    invisible(evalq({
        g <- function(x) x
        classes <- "numeric"
        methods::setMethod("g", classes, function(x) x)
        methods::setMethod("g", "character", function(x) x)
    }, dynamic))
    stopifnot(identical(dynamic$g@signature, "x"),
              !is.null(methods::selectMethod(dynamic$g, "character", useInherited = FALSE)))

    conformed <- scope()
    invisible(evalq({
        methods::setGeneric("s4Conformed", function(x, y) standardGeneric("s4Conformed"))
        methods::setMethod("s4Conformed", c("ANY", "numeric"), function(y) y)
    }, conformed))
    stopifnot(is.null(methods::selectMethod(conformed$s4Conformed, c("ANY", "numeric"),
                                          optional = TRUE, useInherited = FALSE)))
    selected <- methods::selectMethod(conformed$s4Conformed, c("missing", "numeric"),
                                      useInherited = FALSE)
    stopifnot(identical(as.character(selected@defined), c("missing", "numeric")))

    owner <- scope("one")
    evalq(g <- function(x) x, owner)
    shadow <- scope("two", owner)
    invisible(evalq({ g <- function(x) x; methods::setGeneric("g") }, shadow))
    stopifnot(identical(shadow$g@package, "two"))

    # Function-valued references consult the generic's namespace. Use an actual
    # temporary package to check imported copies and exported promoted bindings.
    local({
        root <- tempfile("arity-s4-package-")
        package <- file.path(root, "arityS4PromotedOracle")
        library <- file.path(root, "library")
        dir.create(file.path(package, "R"), recursive = TRUE)
        dir.create(library)
        on.exit(unlink(root, recursive = TRUE), add = TRUE)
        writeLines(c("Package: arityS4PromotedOracle", "Version: 0.0.0.9000",
                     "Title: S4 promoted binding oracle", "Description: Tests promoted generic bindings.",
                     "Authors@R: person('Arity', 'Oracle', email = 'oracle@example.com', role = c('aut', 'cre'))",
                     "License: MIT", "Imports: methods"), file.path(package, "DESCRIPTION"))
        writeLines("export(g, h, implicit, rebound, nonstandard, quoted, dots, plainQuoted, plainDots, foreign)",
                   file.path(package, "NAMESPACE"))
        writeLines(c("g <- function(x) x", "methods::setGeneric('g')",
                     "methods::setGeneric('foreign', function(x) x + 10, package = 'base')",
                     "internalMean <- function(x, ...) x",
                     "methods::setGeneric('h', function(x, y) standardGeneric('h'), signature = 'y', group = 'Math', useAsDefault = FALSE)",
                     "methods::setGeneric('rebound', function(x) standardGeneric('rebound'))",
                     "rebound <- function(x) x",
                     "methods::setGeneric('nonstandard', function(x) { message('old'); standardGeneric('nonstandard') })",
                     "quoted <- function(x = \"a\") x",
                     "methods::setGeneric('quoted')",
                     "dots <- function(x = 1, ...) x",
                     "methods::setGeneric('dots')",
                     "plainQuoted <- function(x = \"a\") x",
                     "plainDots <- function(x = 1, ...) x",
                     "methods::setGeneric('sibling', function(x) standardGeneric('sibling'))",
                     "methods::setGeneric('aliased', function(x) standardGeneric('aliased'), useAsDefault = function(x) 'fallback')",
                     "impl <- function(x) methods::callNextMethod()",
                     "alias <- impl",
                     "impl <- function(x) 'replacement'",
                     "implicit <- function(x) x",
                     "methods::setMethod('implicit', 'numeric', function(x) x)"),
                   file.path(package, "R", "g.R"))
        # A later package file sees the implicit installation in the namespace.
        writeLines("sibling <- function(y) y", file.path(package, "R", "zy.R"))
        writeLines(c("methods::setGeneric('h')",
                     "methods::setMethod('sibling', 'numeric', function(y) y)",
                     "methods::setMethod('aliased', 'numeric', alias)"),
                   file.path(package, "R", "zz.R"))
        output <- system2(file.path(R.home("bin"), "R"),
                          c("CMD", "INSTALL", "--no-byte-compile", "--no-help",
                            paste0("--library=", shQuote(library)), shQuote(package)),
                          stdout = TRUE, stderr = TRUE)
        if (!is.null(attr(output, "status"))) stop(paste(output, collapse = "\n"))
        previous <- .libPaths()
        on.exit(.libPaths(previous), add = TRUE)
        .libPaths(c(library, previous))
        namespace <- asNamespace("arityS4PromotedOracle")
        foreign <- scope("two", namespace)
        invisible(suppressMessages(evalq(methods::setGeneric("foreign", function(x) standardGeneric("foreign"),
                                                              package = "arityS4PromotedOracle"), foreign)))
        stopifnot(identical(namespace$foreign@package, "base"),
                  identical(foreign$foreign@package, "two"))
        for (definition in list(quote(function(x) standardGeneric("internalMean")),
                                quote(function(x, y) standardGeneric("internalMean")))) {
            private <- scope("two")
            invisible(suppressMessages(eval(substitute(
                methods::setGeneric("internalMean", DEF, package = "arityS4PromotedOracle"),
                list(DEF = definition)), private)))
            expected <- if (identical(names(formals(eval(definition))), "x")) "arityS4PromotedOracle" else "two"
            stopifnot(identical(private$internalMean@package, expected))
        }
        sibling <- methods::getMethod("sibling", "numeric", where = namespace)
        stopifnot(identical(names(sibling@defined), "y"),
                  identical(namespace$aliased(1), "fallback"))
        for (name in c("quoted", "plainQuoted", "dots", "plainDots")) {
            replacement <- scope("two", namespace)
            # Parse the different quote spellings, rather than deparsing them.
            definition <- if (name %in% c("quoted", "plainQuoted")) {
                parse(text = sprintf("function(x = 'a') standardGeneric('%s')", name))[[1L]]
            } else {
                # The trailing-dots exception also accepts changed defaults.
                parse(text = sprintf("function(x = 2) standardGeneric('%s')", name))[[1L]]
            }
            invisible(eval(substitute(methods::setGeneric(NAME, DEF),
                                      list(NAME = name, DEF = definition)), replacement))
            original <- methods:::implicitGeneric(name, namespace)
            stopifnot(identical(replacement[[name]], original))
        }
        target <- scope("two", asNamespace("arityS4PromotedOracle"))
        original <- arityS4PromotedOracle::h
        for (name in c("g", "h")) {
            for (options in list(list(def = NULL), list(signature = NULL),
                                 list(useAsDefault = FALSE), list(group = "Math2"))) {
                redeclared <- scope("two", asNamespace("arityS4PromotedOracle"))
                call <- as.call(c(list(quote(methods::setGeneric), name), options))
                invisible(suppressMessages(eval(call, redeclared)))
                stopifnot(identical(redeclared[[name]], get(name, asNamespace("arityS4PromotedOracle"))))
            }
        }
        invisible(evalq(methods::setGeneric("h"), target))
        stopifnot(identical(target$h, original), is.null(target$h@default))
        changed <- scope("two", asNamespace("arityS4PromotedOracle"))
        invisible(suppressMessages(evalq(methods::setGeneric("h", signature = "x", useAsDefault = FALSE), changed)))
        stopifnot(identical(changed$h@package, "two"), identical(changed$h@signature, "x"),
                  is.null(changed$h@default))
        invisible(methods::setMethod("h", "numeric", function(x, y) y, where = target))
        selected <- methods::getMethod("h", "numeric", where = target)
        stopifnot(identical(names(selected@defined), "y"))
        invisible(methods::setMethod(arityS4PromotedOracle::g, "numeric", function(x) x, where = target))
        selected <- methods::selectMethod(arityS4PromotedOracle::g, "numeric", useInherited = FALSE)
        stopifnot(identical(arityS4PromotedOracle::g@package, "arityS4PromotedOracle"),
                  identical(as.character(selected@defined), "numeric"))
        invisible(methods::setMethod(arityS4PromotedOracle::implicit, "character", function(x) x, where = target))
        selected <- methods::selectMethod(arityS4PromotedOracle::implicit, "character", useInherited = FALSE)
        stopifnot(identical(arityS4PromotedOracle::implicit@package, "arityS4PromotedOracle"),
                  identical(as.character(selected@defined), "character"))

        for (definition in list(quote(function(x, y) standardGeneric("g")),
                                quote(function(x = 1) standardGeneric("g")))) {
            replacement <- scope("two", asNamespace("arityS4PromotedOracle"))
            invisible(suppressMessages(eval(substitute(methods::setGeneric("g", DEF),
                                                       list(DEF = definition)), replacement)))
            stopifnot(identical(replacement$g@package, "two"),
                      identical(arityS4PromotedOracle::g@package, "arityS4PromotedOracle"))
        }
        compatible <- scope("two", asNamespace("arityS4PromotedOracle"))
        invisible(evalq(methods::setGeneric("g", function(x) standardGeneric("g")), compatible))
        stopifnot(identical(compatible$g@package, "arityS4PromotedOracle"))

        for (definition in list(quote(function(x, y) standardGeneric("h")),
                                quote(function(x, y) { message("new"); standardGeneric("h") }))) {
            replacement <- scope("two", asNamespace("arityS4PromotedOracle"))
            invisible(suppressMessages(eval(substitute(
                methods::setGeneric("h", DEF, signature = "y", group = "Math",
                                    useAsDefault = function(x, y) "new"),
                list(DEF = definition)), replacement)))
            expected <- if (identical(definition[[3L]][[1L]], as.name("{"))) "two" else "arityS4PromotedOracle"
            stopifnot(identical(replacement$h@package, expected))
            if (expected != "two") stopifnot(identical(replacement$h, original), is.null(replacement$h@default))
        }

        replacement <- scope("two", asNamespace("arityS4PromotedOracle"))
        invisible(suppressMessages(evalq(methods::setGeneric("nonstandard", function(x) {
            message("new")
            standardGeneric("nonstandard")
        }), replacement)))
        stopifnot(identical(replacement$nonstandard@package, "two"))

        failure <- tryCatch(methods::setMethod(arityS4PromotedOracle::rebound, "numeric",
                                              function(x) x, where = target), error = identity)
        stopifnot(inherits(failure, "error"))

        shadow <- scope("two", asNamespace("arityS4PromotedOracle"))
        invisible(evalq({
            rebound <- function(x) x
            methods::setMethod("rebound", "numeric", function(x) x)
        }, shadow))
        stopifnot(identical(shadow$rebound@package, "two"))
    })

    captured <- scope()
    invisible(evalq({
        methods::setGeneric("s4Captured", function(x) standardGeneric("s4Captured"))
        impl <- function(x) 1
        methods::setMethod("s4Captured", "numeric", impl)
        impl <- function(x, y) methods::callNextMethod()
    }, captured))
    method <- methods::getMethod("s4Captured", "numeric", where = captured)
    stopifnot(identical(names(formals(method)), "x"), identical(body(method), quote(1)),
              identical(captured$s4Captured(1), 1))

    for (signature in list(c("x", "..."), c("...", "x"), "...")) {
        explicit <- scope()
        invisible(eval(substitute(methods::setGeneric("s4Dots", function(x, ...) standardGeneric("s4Dots"),
                                                      signature = SIGNATURE),
                                  list(SIGNATURE = signature)), explicit))
        stopifnot(identical(explicit$s4Dots@signature, if (length(signature) == 1L) "..." else "x"))
        invisible(evalq(methods::setMethod("s4Dots", "numeric", function(x, ...) x), explicit))
        stopifnot(!is.null(methods::selectMethod(explicit$s4Dots, "numeric", useInherited = FALSE)))
    }

    null <- scope()
    invisible(evalq({
        methods::setGeneric("s4NullDefault", def = NULL, useAsDefault = function(x, y) x)
        s4NullExisting <- function(x, y) x
        methods::setGeneric("s4NullExisting", def = NULL)
    }, null))
    stopifnot(identical(null$s4NullDefault@signature, c("x", "y")),
              identical(null$s4NullExisting@signature, c("x", "y")))

    for (generic in c(FALSE, TRUE)) {
        skeleton <- scope()
        evalq(g <- function(x, y = 1) x, skeleton)
        if (generic) invisible(evalq(methods::setGeneric("g"), skeleton))
        invisible(evalq(methods::setGeneric("g", useAsDefault = function(x, y = 2) 0), skeleton))
        stopifnot(identical(formals(skeleton$g)$y, 1),
                  identical(formals(skeleton$g@default)$y, 2))
    }

    for (promote in c("generic", "method", "none")) {
        dots <- scope()
        if (promote != "none") {
            evalq(g <- function(...) NULL, dots)
            if (promote == "generic") invisible(evalq(methods::setGeneric("g"), dots))
        } else {
            invisible(evalq(methods::setGeneric("g", function(...) standardGeneric("g")), dots))
        }
        invisible(evalq(methods::setMethod("g", "numeric", function(...) 1), dots))
        stopifnot(identical(dots$g@signature, "..."))
        stopifnot(!is.null(methods::selectMethod(dots$g, "numeric", useInherited = FALSE)))
    }

    ordered <- scope()
    invisible(evalq(methods::setGeneric("g", function(x) standardGeneric("g")), ordered))
    stopifnot(is.null(evalq(methods::selectMethod("g", "ANY", optional = TRUE,
                                               useInherited = FALSE), ordered)))
    evalq(g <- function(x) x, ordered)

    skeleton <- scope()
    invisible(evalq(methods::setGeneric("g", function(x) {
        methods::callNextMethod()
        standardGeneric("g")
    }, useAsDefault = function(x) 0), skeleton))
    failure <- tryCatch(skeleton$g(1), error = identity)
    stopifnot(inherits(failure, "error"))
})
