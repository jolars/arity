MakeBase <- methods::setClass("S4Base", slots = c(value = "numeric"),
  prototype = list(value = 2), sealed = TRUE,
  validity = function(object) TRUE)
MakeLabel <- methods::setClass("S4Label", slots = c(label = "character"))
MakeChild <- methods::setClass("S4Child", contains = c("S4Base", "S4Label"),
  slots = c(flag = "logical"), prototype = methods::prototype(flag = TRUE))
MakeLegacy <- methods::setClass("S4Legacy",
  methods::representation("S4Child", note = "character"))
MakeAny <- methods::setClass("S4Any", slots = c("first", "second"))
MakeWide <- methods::setClass("S4Wide", slots = c(value = "ANY"))
MakeNarrow <- methods::setClass("S4Narrow", contains = "S4Wide",
  slots = c(value = "numeric"))
MakeCombined <- methods::setClass("S4Combined", contains = c("S4Base", "S4Wide"))
MakeVirtual <- methods::setClass("S4Virtual", contains = "VIRTUAL",
  slots = c(common = "ANY"))
MakeConcrete <- methods::setClass("S4Concrete", contains = "S4Virtual")
MakeEmpty <- methods::setClass("S4Empty")
MakePrototype <- methods::setClass("S4Prototype", prototype = list())
MakeUnion <- methods::setClassUnion("S4Union", c("S4Base", "S4Label"))
methods::setIs("S4Any", "S4Union")
methods::setClass("S4Converted", slots = c(converted = "character"))
methods::setIs("S4Any", "S4Converted",
  coerce = function(from) methods::new("S4Converted", converted = "converted"),
  replace = function(from, value) from)
methods::setClass("S4Conditional", contains = "VIRTUAL")
methods::setIs("S4Any", "S4Conditional", test = function(object) TRUE)
methods::setAs("S4Base", "S4Label",
  def = function(from) methods::new("S4Label", label = "coerced"))
OldResult <- methods::setOldClass(c("S4OldChild", "S4OldParent"))
MakeOldShape <- methods::setClass("S4OldShape", slots = c(known = "numeric"))
methods::setOldClass("S4OldShape", S4Class = "S4OldShape")
methods::setOldClass("S4OldCopy", S4Class = "S4OldShape")
MakeNumber <- methods::setClass("S4Number", contains = "numeric")
MakeEnvironment <- methods::setClass("S4Environment", contains = "environment")
methods::setOldClass("S4RegisteredAncestor")
methods::setOldClass(c("S4RegisteredChild", "S4RegisteredAncestor"))
methods::setOldClass("S4OldPrototype",
  prototype = structure(list(), class = "S4OldPrototype"))
methods::setClass("S4FormalBridge", slots = c(x = "numeric"))
methods::setOldClass(c("S4FormalBridge", "S4BridgeAncestor"), S4Class = "S4FormalBridge")
methods::setClass("S4RelationShape", slots = c(shape = "numeric"))
methods::setClass("S4RelationTarget", slots = c(converted = "character"))
methods::setIs("S4RelationShape", "S4RelationTarget",
  coerce = function(from) methods::new("S4RelationTarget"),
  replace = function(from, value) from)
methods::setClassUnion("S4RelationUnion", "S4RelationShape")
methods::setOldClass("S4RelationCopy", S4Class = "S4RelationShape")
methods::setClass("S4DataAny", contains = "ANY")
methods::setClass("S4DataS4", contains = "S4")
methods::setClass("S4DataNull", contains = "NULL")
methods::setClass("S4ImplicitFirst", slots = c(x = "numeric"))
methods::setClass("S4ExplicitSecond", slots = c(x = "numeric"), prototype = list(x = 2))
methods::setClass("S4ImplicitCombined", contains = c("S4ImplicitFirst", "S4ExplicitSecond"))
obj <- MakeChild(value = 3, label = "three")
