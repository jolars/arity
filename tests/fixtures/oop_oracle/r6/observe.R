owner <- environment()
class_facts <- function(generator, binding) {
  member_kinds <- function(values, kind) as.list(setNames(rep(kind, length(values)), names(values)))
  parent <- generator$get_inherit()
  list(name = generator$classname, binding = binding,
       parents = if (is.null(parent)) list() else list(parent$classname),
       members = c(member_kinds(generator$public_fields, "public-field"),
                   member_kinds(generator$public_methods, "public-method"),
                   member_kinds(generator$private_fields, "private-field"),
                   member_kinds(generator$active, "active-binding")))
}
facts <- list(
  system = "r6",
  classes = list(class_facts(Base, "Base"), class_facts(Child, "Child")),
  constructors = list(
    list(binding = "Base$new", formals = formal_facts(Base$new)),
    list(binding = "Base$initialize", formals = formal_facts(Base$public_methods$initialize)),
    list(binding = "Child$new", formals = formal_facts(Child$new)),
    list(binding = "Child$initialize", formals = formal_facts(Child$public_methods$initialize))
  ),
  methods = list(
    method_facts("read", "Base", base$read, owner),
    method_facts("read", "Child", child$read, owner)
  ),
  dispatch = list(
    dispatch_facts("base$read()", base, owner),
    dispatch_facts("child$read()", child, owner),
    dispatch_facts("child$read(scale = 3)", child, owner),
    dispatch_facts("child$doubled", child, owner)
  )
)
