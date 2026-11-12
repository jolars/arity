owner <- environment()
facts <- list(
  system = "dispatch_environments",
  classes = list(list(name = "DispatchFormal", binding = "Formal", parents = list(),
                      members = list(value = "numeric"))),
  constructors = list(list(binding = "Dispatch", formals = formal_facts(Dispatch))),
  methods = list(method_facts("g", "Dispatch", g.Dispatch, owner)),
  dispatch = list(
    dispatch_facts("g(Dispatch())", Dispatch(), owner),
    dispatch_facts("Dispatch() + 1", Dispatch(), owner),
    dispatch_facts("Dispatch()[1]", Dispatch(), owner),
    dispatch_facts("g(structure(1, class = 'Registered'))", Dispatch(), owner),
    dispatch_facts("shadow(Dispatch())", Dispatch(), owner),
    dispatch_facts("readDispatch(Formal())", Formal(), owner),
    dispatch_facts("length(Formal())", Formal(), owner),
    dispatch_facts("abs(Formal())", Formal(), owner),
    dispatch_facts("`replaceDispatch<-`(Formal(), 1)", Formal(), owner),
    dispatch_facts("exists('readDispatch', inherits = FALSE)", Formal(), owner),
    dispatch_facts("exists('HiddenDispatchClass', inherits = FALSE)", Formal(), owner)
  )
)
