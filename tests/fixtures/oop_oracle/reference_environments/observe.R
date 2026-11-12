owner <- environment()
class_facts <- function(generator, binding) {
  definition <- methods::getClassDef(generator@className, where = owner)
  list(name = generator@className[[1L]], binding = binding,
       parents = as.list(setdiff(names(Filter(function(x) x@distance == 1L, definition@contains)), "envRefClass")),
       members = as.list(setNames(rep("field", length(generator$fields())), names(generator$fields()))))
}
facts <- list(
  system = "reference_environments",
  classes = list(class_facts(Base, "Base"), class_facts(Child, "Child")),
  constructors = list(list(binding = "Child$new", formals = formal_facts(Child$new))),
  methods = list(
    method_facts("read", "Base", base$read, owner),
    method_facts("read", "Child", child$read, owner),
    method_facts("external", "Child", child$external, owner)
  ),
  dispatch = list(
    dispatch_facts("child$read()", child, owner),
    dispatch_facts("child$nested()", child, owner),
    dispatch_facts("child$definition()", child, owner),
    dispatch_facts("child$external()", child, owner),
    dispatch_facts("child$probe()", child, owner),
    dispatch_facts("child$update()", child, owner),
    dispatch_facts("value", child, owner)
  )
)
