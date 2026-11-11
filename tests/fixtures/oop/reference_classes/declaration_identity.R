Formal <- methods::setClass(Class = "Formal")
make <- methods::setRefClass
Reference <- make(Class = "Reference", contains = "Formal")
methods::setClass(Class = "Child", contains = "Reference")
print(Formal)
print(Reference)
