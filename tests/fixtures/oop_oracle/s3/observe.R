owner <- environment()
facts <- list(
  system = "s3",
  classes = list(
    list(name = class(base)[[1L]], binding = "point", parents = list(),
         members = as.list(setNames(rep("field", length(base)), names(base)))),
    list(name = class(child)[[1L]], binding = "colored_point",
         parents = as.list(class(child)[-1L]),
         members = as.list(setNames(rep("field", length(child)), names(child))))
  ),
  constructors = list(
    list(binding = "point", formals = formal_facts(point)),
    list(binding = "colored_point", formals = formal_facts(colored_point))
  ),
  methods = list(
    method_facts("describe", "point", getS3method("describe", "point", envir = owner), owner),
    method_facts("describe", "colored_point", getS3method("describe", "colored_point", envir = owner), owner),
    method_facts("+", "point", getS3method("+", "point", envir = owner), owner)
  ),
  dispatch = list(
    dispatch_facts("describe(base)", base, owner),
    dispatch_facts("describe(child)", child, owner),
    dispatch_facts("describe(0)", 0, owner),
    dispatch_facts("(base + 3)$value", base, owner)
  )
)
