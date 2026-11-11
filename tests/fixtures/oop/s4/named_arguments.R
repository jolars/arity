Point <- methods::setClass(Class = "Point", slots = c(value = "numeric"))
obj <- methods::new("Point", value = 1)
print(obj@value)
