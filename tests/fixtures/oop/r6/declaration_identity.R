make <- R6::R6Class
`Generator<-` <- make(classname = "Display", public = list())
Alias <- `Generator<-`
Child <- R6::R6Class(inherit = Alias)
Other <- R6::R6Class("Display")
print(Child$new())
print(Other$new())
