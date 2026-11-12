owner <- environment()
generators <- list(Base = Base, Child = Child, Grandchild = Grandchild,
                   Other = Other, Multiple = Multiple, Mixed = Mixed, Builtin = Builtin,
                   CustomInit = CustomInit)
objects <- list(Base = base, Child = child, Grandchild = grandchild,
                Other = other, Multiple = multiple, Mixed = mixed, Builtin = builtin,
                CustomInit = custom_init)
generator_definition <- methods::getClassDef("refGeneratorSlot")
object_members <- function(generator) {
  fields <- generator$fields()
  result <- as.list(setNames(ifelse(fields == "activeBindingFunction",
                                    "active-binding", paste0("field:", fields)), names(fields)))
  # Super implementation aliases and backing bindings are implementation details.
  method_names <- grep("#", generator$methods(), invert = TRUE, value = TRUE)
  result <- c(result, as.list(setNames(ifelse(method_names %in% c(".objectParent", ".objectPackage"),
                                             "field", "method"), method_names)))
  c(result, list(.self = "field", .refClassDef = "field"))
}
class_facts <- function(binding) {
  generator <- generators[[binding]]
  definition <- methods::getClassDef(generator@className, where = owner)
  slot_names <- names(methods::getSlots(definition))
  slots <- as.list(setNames(rep("slot", length(slot_names)), paste0("slot/", slot_names)))
  instance <- object_members(generator)
  names(instance) <- paste0("instance/", names(instance))
  methods <- names(generator_definition@refMethods)
  generator_members <- c(as.list(setNames(ifelse(methods %in% c(".objectParent", ".objectPackage"),
                                               "field", "method"), methods)),
                         list(.self = "field", .refClassDef = "field", def = "field", className = "field"))
  names(generator_members) <- paste0("generator/", names(generator_members))
  list(name = generator@className[[1L]], binding = binding,
       parents = as.list(setdiff(names(Filter(function(x) x@distance == 1L, definition@contains)), "envRefClass")),
       members = c(instance, generator_members, slots))
}
get_member <- function(object, name) do.call("$", list(object, name))
facts <- list(
  system = "reference_members",
  classes = lapply(names(generators), class_facts),
  constructors = c(unlist(lapply(names(generators), function(binding) {
    generator <- generators[[binding]]
    object <- objects[[binding]]
    list(list(binding = binding, formals = formal_facts(generator)),
         list(binding = paste0(binding, "$new"), formals = formal_facts(generator$new)),
         list(binding = paste0(binding, "$initialize"), formals = if ("initialize" %in% generator$methods()) formal_facts(object$initialize) else formal_facts(function(...) NULL)))
  }), recursive = FALSE), lapply(names(generator_definition@refMethods), function(name) {
    value <- get_member(Base, name)
    list(binding = paste0("generator/", name), formals = if (is.function(value)) formal_facts(value) else list())
  }), lapply(grep("#", Base$methods(), invert = TRUE, value = TRUE), function(name) {
    value <- get_member(base, name)
    list(binding = paste0("instance/", name), formals = if (is.function(value)) formal_facts(value) else list())
  }), list(list(binding = "external-source/Base$external", formals = formal_facts(base$external@actual)))),
  methods = list(
    method_facts("read", "Base", base$read, owner),
    method_facts("read", "Child", child$read, owner),
    method_facts("read", "Grandchild", grandchild$read, owner),
    method_facts("callSuper/read", "Child", environment(child$read)$callSuper, owner),
    method_facts("callSuper/read", "Grandchild", environment(grandchild$read)$callSuper, owner),
    method_facts("callSuper/read", "Mixed", environment(mixed$read)$callSuper, owner),
    method_facts("callSuper/initialize", "Base", environment(base$initialize)$callSuper, owner),
    method_facts("callSuper/copy", "Builtin", environment(builtin$copy)$callSuper, owner),
    method_facts("callSuper/show", "Builtin", environment(builtin$show)$callSuper, owner),
    method_facts("callSuper/initialize", "CustomInit", environment(custom_init$initialize)$callSuper, owner),
    method_facts("external", "Base", base$external, owner)
  ),
  dispatch = list(
    dispatch_facts("base$read()", base, owner),
    dispatch_facts("child$read()", child, owner),
    dispatch_facts("grandchild$read()", grandchild, owner),
    dispatch_facts("multiple$read()", multiple, owner),
    dispatch_facts("mixed$read()", mixed, owner),
    dispatch_facts("base$external()", base, owner),
    dispatch_facts("base$doubled", base, owner),
    dispatch_facts("builtin$copy()$getRefClass()@className", builtin, owner),
    dispatch_facts("custom_init$getRefClass()@className", custom_init, owner),
    dispatch_facts("getRefClass('Child')@className", child, owner)
  )
)
