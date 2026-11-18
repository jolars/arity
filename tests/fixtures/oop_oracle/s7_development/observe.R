owner <- environment()
facts <- list(
  system = "s7_development",
  classes = list(
    list(name = Point@name, binding = "Point", parents = list(),
         members = list(id = "property")),
    list(name = Colored@name, binding = "Colored", parents = list("Plain"),
         members = list(seed = "property", color = "property"))
  ),
  constructors = list(
    list(binding = "Point", formals = formal_facts(Point)),
    list(binding = "Colored", formals = formal_facts(Colored))
  ),
  methods = list(method_facts("describe", "Point", method(describe, Point), owner)),
  dispatch = list(dispatch_facts("describe(Point())", point, owner)),
  s7_observations = list(
    s4_subclass_property = as.character(prop(wrapped, "id")),
    s4_parent_property = as.character(prop(colored, "seed"))
  )
)
