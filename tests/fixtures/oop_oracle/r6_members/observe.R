owner <- environment()
generators <- list(Original = Original, Child = Child, Grandchild = Grandchild,
                   NoClone = NoClone, ChildClone = ChildClone,
                   RestoredClone = RestoredClone, DisabledClone = DisabledClone)
objects <- list(Original = base, Child = child, Grandchild = grandchild,
                NoClone = no_clone, ChildClone = child_clone,
                RestoredClone = restored_clone, DisabledClone = disabled_clone)
member_facts <- function(env, prefix) {
  if (is.null(env)) return(list())
  names <- setdiff(ls(env, all.names = TRUE), ".__enclos_env__")
  if (length(names) == 0L) return(list())
  values <- lapply(names, function(name) {
    if (bindingIsActive(name, env)) "active-binding"
    else if (is.function(env[[name]])) "method" else "field"
  })
  setNames(values, paste0(prefix, names))
}
class_facts <- function(binding) {
  generator <- generators[[binding]]
  object <- objects[[binding]]
  parent <- generator$get_inherit()
  list(name = if (is.null(generator$classname)) "<unnamed>" else generator$classname,
       binding = binding,
       parents = if (is.null(parent)) list() else list(
         if (is.null(parent$classname)) "<unnamed>" else parent$classname),
       members = c(member_facts(object, "public/"),
                   member_facts(object$.__enclos_env__$private, "private/"),
                   member_facts(object$.__enclos_env__$super, "super/"),
                   member_facts(generator, "generator/")))
}
facts <- list(
  system = "r6_members",
  classes = lapply(names(generators), class_facts),
  constructors = c(unlist(lapply(names(generators), function(binding) {
    initialize <- objects[[binding]]$initialize
    list(list(binding = paste0(binding, "$new"),
              formals = formal_facts(generators[[binding]]$new)),
         list(binding = paste0(binding, "$initialize"),
              formals = if (is.null(initialize)) list() else formal_facts(initialize)))
  }), recursive = FALSE), lapply(
    c("set", "debug", "undebug", "lock", "unlock", "is_locked", "get_inherit", "has_private", "clone_method"),
    function(name) list(binding = paste0("Original$", name),
                        formals = formal_facts(Original[[name]]))
  )),
  methods = list(
    method_facts("read", "Original", base$read, owner),
    method_facts("read", "Child", child$read, owner),
    method_facts("read", "Grandchild", grandchild$read, owner),
    method_facts("hidden", "Original", child$.__enclos_env__$super$hidden, owner)
  ),
  dispatch = list(
    dispatch_facts("base$read()", base, owner),
    dispatch_facts("child$read()", child, owner),
    dispatch_facts("grandchild$read()", grandchild, owner),
    dispatch_facts("child$parent_private()", child, owner),
    dispatch_facts("child$doubled", child, owner),
    dispatch_facts("is.function(no_clone$clone)", no_clone, owner),
    dispatch_facts("is.function(child_clone$clone)", child_clone, owner),
    dispatch_facts("is.function(restored_clone$clone)", restored_clone, owner),
    dispatch_facts("is.function(disabled_clone$clone)", disabled_clone, owner)
  )
)
