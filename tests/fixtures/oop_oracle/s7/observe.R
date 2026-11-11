owner <- environment()
class_facts <- function(class, binding, object) {
  list(name = class@name, binding = binding,
       parents = if (identical(class@parent, S7::S7_object)) list() else list(class@parent@name),
       members = as.list(setNames(rep("property", length(S7::prop_names(object))), S7::prop_names(object))))
}
facts <- list(
  system = "s7",
  classes = list(class_facts(Point, "Point", base), class_facts(ColoredPoint, "ColoredPoint", child)),
  constructors = list(
    list(binding = "Point", formals = formal_facts(Point)),
    list(binding = "ColoredPoint", formals = formal_facts(ColoredPoint))
  ),
  methods = list(
    method_facts("describe", "Point", S7::method(describe, Point), owner),
    method_facts("combine", c("Point", "ColoredPoint"), S7::method(combine, list(Point, ColoredPoint)), owner)
  ),
  dispatch = list(
    dispatch_facts("describe(base)", base, owner),
    dispatch_facts("describe(child)", child, owner),
    dispatch_facts("combine(base, child)", base, owner)
  )
)
