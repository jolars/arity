owner <- environment()
class_facts <- function(generator, binding) {
  member_kinds <- function(values, kind) as.list(setNames(rep(kind, length(values)), names(values)))
  parent <- generator$get_inherit()
  list(name = generator$classname, binding = binding,
       parents = if (is.null(parent)) list() else list(parent$classname),
       members = c(member_kinds(generator$public_fields, "public-field"),
                   member_kinds(generator$public_methods, "public-method"),
                   member_kinds(generator$private_fields, "private-field"),
                   member_kinds(generator$private_methods, "private-method"),
                   member_kinds(generator$active, "active-binding")))
}
facts <- list(
  system = "r6_environments",
  classes = list(class_facts(Base, "Base"), class_facts(Child, "Child"),
                 class_facts(Empty, "Empty"), class_facts(NonPortable, "NonPortable"),
                 class_facts(Aliased, "Aliased")),
  constructors = list(list(binding = "Base$new", formals = formal_facts(Base$new))),
  methods = list(
    method_facts("read", "Base", base$read, owner),
    method_facts("nested", "Base", base$nested, owner),
    method_facts("nested", "Base", child$nested, owner),
    method_facts("read", "Child", child$read, owner),
    method_facts("read", "Empty", empty$read, owner),
    method_facts("read", "NonPortable", nonportable$read, owner),
    method_facts("read", "Aliased", aliased$read, owner),
    method_facts("private_read", "Child", child$.__enclos_env__$private$private_read, owner),
    method_facts("doubled", "Child", child$.__enclos_env__$.__active__$doubled, owner)
  ),
  dispatch = list(
    dispatch_facts("base$read()", base, owner),
    dispatch_facts("identical(base$nested()(), base)", base, owner),
    dispatch_facts("base$boxed$fn()", base, owner),
    dispatch_facts("child$read()", child, owner),
    dispatch_facts("child$shadow('parameter self')", child, owner),
    dispatch_facts("child$assign()", child, owner),
    dispatch_facts("child$doubled", child, owner),
    dispatch_facts("empty$read()", empty, owner),
    dispatch_facts("nonportable$read()", nonportable, owner),
    dispatch_facts("identical(aliased$read(), aliased)", aliased, owner),
    dispatch_facts("helper()", aliased, owner)
  )
)
