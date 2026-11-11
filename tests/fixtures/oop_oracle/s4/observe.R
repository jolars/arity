owner <- environment()
class_facts <- function(name, binding) {
  definition <- methods::getClassDef(name, where = owner)
  list(name = definition@className[[1L]], binding = binding,
       parents = as.list(names(Filter(function(x) x@distance == 1L, definition@contains))),
       members = as.list(setNames(rep("slot", length(definition@slots)), names(definition@slots))))
}
facts <- list(
  system = "s4",
  classes = list(class_facts("Point", "Point"), class_facts("ColoredPoint", "ColoredPoint")),
  constructors = list(
    list(binding = "Point", formals = formal_facts(Point)),
    list(binding = "ColoredPoint", formals = formal_facts(ColoredPoint))
  ),
  methods = list(
    method_facts("describe", "Point", methods::getMethod("describe", "Point", where = owner), owner),
    method_facts("describe", "ColoredPoint", methods::getMethod("describe", "ColoredPoint", where = owner), owner),
    method_facts("combine", c("Point", "ColoredPoint"),
                 methods::getMethod("combine", c("Point", "ColoredPoint"), where = owner), owner)
  ),
  dispatch = list(
    dispatch_facts("describe(base)", base, owner),
    dispatch_facts("describe(child)", child, owner),
    dispatch_facts("combine(base, child)", base, owner)
  )
)
