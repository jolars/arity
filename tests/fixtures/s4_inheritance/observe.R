args <- commandArgs(trailingOnly = TRUE)
root <- if (length(args)) args[[1L]] else "."
env <- new.env(parent = globalenv())
env$.packageName <- ".GlobalEnv"
invisible(capture.output(sys.source(file.path(root, "input.R"), env)))
emit <- function(...) cat(paste(..., sep = "\t"), "\n", sep = "")
observe <- function(name, generic, classes) {
    fdef <- methods::getGeneric(generic, where = env)
    ambiguous <- FALSE
    method <- withCallingHandlers(
        methods::selectMethod(fdef, classes),
        ambiguousMethodSelection = function(condition) ambiguous <<- TRUE,
        message = function(message) invokeRestart("muffleMessage")
    )
    defined <- setNames(rep("ANY", length(fdef@signature)), fdef@signature)
    defined[names(method@defined)] <- as.character(method@defined)
    emit("selection", name, generic, paste(classes, collapse = ","),
         paste(defined, collapse = ","), method@generic, ambiguous)
}
observe("nearest", "s4Distance", c("DistanceChild", "numeric"))
observe("missing", "s4Distance", c("DistanceChild", "missing"))
observe("crossed", "s4Crossed", c("Both", "Both"))
observe("equal", "s4Equal", "Both")
observe("union_class", "s4Union", "DistanceChild")
observe("union_builtin", "s4Union", "NULL")
observe("old_class", "s4Old", "OldChild")
observe("reference_storage", "s4Reference", "RefChild")
observe("reference_base", "s4Environment", "RefChild")
observe("primitive", "length", "DistanceChild")
observe("member", "+", c("DistanceRoot", "DistanceRoot"))
observe("group", "-", c("DistanceRoot", "DistanceRoot"))
observe("default_override", "s4Default", "numeric")
emit("next", env$s4Next(methods::new("DistanceChild")))
emit("storage", paste(sort(names(methods::getSlots(methods::getClassDef("RefChild", where = env)))), collapse = ","),
     paste(sort(names(env$RefChild$fields())), collapse = ","))
