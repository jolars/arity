args <- commandArgs(trailingOnly = TRUE)
stopifnot(getRversion() == "4.6.1")
source(file.path(args[[1L]], "input.R"), local = .GlobalEnv)
emit <- function(...) cat(paste(..., sep = "\t"), "\n", sep = "")
for (name in c("S4Base", "S4Child", "S4Legacy", "S4Any", "S4Virtual",
               "S4Wide", "S4Narrow", "S4Combined", "S4Concrete", "S4Empty", "S4Prototype", "S4Union",
               "S4OldChild", "S4OldParent", "S4OldShape", "S4OldCopy", "S4Number", "S4Environment",
               "S4RegisteredAncestor", "S4RegisteredChild", "S4OldPrototype",
               "S4FormalBridge", "S4BridgeAncestor", "S4RelationCopy",
               "S4DataAny", "S4DataS4", "S4DataNull", "S4ImplicitCombined")) {
  definition <- methods::getClassDef(name)
  emit("class", name, definition@virtual, definition@sealed)
  slots <- methods::getSlots(definition)
  for (slot in sort(names(slots))) emit("slot", name, slot, slots[[slot]])
}
emit("generator", "MakeBase", is(MakeBase, "classGeneratorFunction"))
emit("generator", "MakeUnion", is(MakeUnion, "classGeneratorFunction"))
emit("generator", "OldResult", is(OldResult, "classGeneratorFunction"))
emit("extends", "S4Base", "S4Union", extends("S4Base", "S4Union"))
emit("extends", "S4Any", "S4Union", extends("S4Any", "S4Union"))
emit("extends", "S4Any", "S4Converted", extends("S4Any", "S4Converted"))
emit("conditional", "S4Any", "S4Conditional", is(methods::getClassDef("S4Any")@contains$S4Conditional@test, "function"))
emit("coercion", "S4Base", "S4Label", methods::as(MakeBase(), "S4Label")@label)
emit("prototype", "S4Base", "value", methods::getClassDef("S4Base")@prototype@value)
emit("prototype", "S4Child", "value", methods::getClassDef("S4Child")@prototype@value)
emit("prototype", "S4Child", "flag", methods::getClassDef("S4Child")@prototype@flag)
emit("extends", "S4RegisteredChild", "S4RegisteredAncestor", extends("S4RegisteredChild", "S4RegisteredAncestor"))
emit("extends", "S4FormalBridge", "S4BridgeAncestor", extends("S4FormalBridge", "S4BridgeAncestor"))
emit("extends", "S4RelationCopy", "S4RelationTarget", extends("S4RelationCopy", "S4RelationTarget"))
emit("extends", "S4RelationCopy", "S4RelationUnion", extends("S4RelationCopy", "S4RelationUnion"))
emit("constructible", "S4OldPrototype", is(methods::new("S4OldPrototype"), "S4OldPrototype"))
emit("implicit", "S4ImplicitCombined", "x", identical(methods::new("S4ImplicitCombined")@x, numeric()))
emit("access", "at", obj@value)
emit("access", "slot", methods::slot(obj, "value"))
obj@value <- 4
emit("replace", "at", obj@value)
methods::slot(obj, "value") <- 5
emit("replace", "slot", obj@value)
