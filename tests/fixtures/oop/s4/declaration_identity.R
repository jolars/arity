make <- methods::setClass
First <- make(Class = "Base")
Second <- methods::setClass(Class = "Base")
methods::setClass(Class = "Child", contains = "Base")
print(First)
print(Second)
