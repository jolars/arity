methods::setGeneric("audit_speak", function(x) standardGeneric("audit_speak"))
methods::setMethod("audit_speak", "numeric", function(x) {
  list(.Generic, .Methods, .defined, .target)
})
print(audit_speak(1))
