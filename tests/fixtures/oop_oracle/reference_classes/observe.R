owner <- environment()
class_facts <- function(generator, binding) {
  definition <- methods::getClassDef(generator@className, where = owner)
  list(name = generator@className[[1L]], binding = binding,
       parents = as.list(setdiff(names(Filter(function(x) x@distance == 1L, definition@contains)), "envRefClass")),
       members = c(as.list(setNames(rep("field", length(generator$fields())), names(generator$fields()))),
                   list(read = if ("read" %in% generator$methods()) "method" else stop("missing read"))))
}
facts <- list(
  system = "reference_classes",
  classes = list(class_facts(Base, "Base"), class_facts(Child, "Child")),
  constructors = list(
    list(binding = "Base$new", formals = formal_facts(Base$new)),
    list(binding = "Child$new", formals = formal_facts(Child$new))
  ),
  methods = list(
    method_facts("read", "Base", base$read, owner),
    method_facts("read", "Child", child$read, owner)
  ),
  dispatch = list(
    dispatch_facts("base$read()", base, owner),
    dispatch_facts("child$read()", child, owner),
    dispatch_facts("child$read(scale = 3)", child, owner)
  )
)
