owner <- environment()
r6_members <- c(as.list(setNames(rep("field", length(R6Generator$public_fields)),
                                names(R6Generator$public_fields))),
                as.list(setNames(rep("method", length(R6Generator$public_methods)),
                                names(R6Generator$public_methods))),
                as.list(setNames(rep("active-binding", length(R6Generator$active)),
                                names(R6Generator$active))))
reference_methods <- grep("#", ReferenceGenerator$methods(), invert = TRUE, value = TRUE)
facts <- list(
  system = "member_mutations",
  classes = list(
    list(name = "MutationR6", binding = "R6Generator", parents = list(), members = r6_members),
    list(name = "MutationReference", binding = "ReferenceGenerator", parents = list(),
         members = c(list(value = "field", doubled = "active-binding"),
                     as.list(setNames(rep("method", length(reference_methods)), reference_methods))))
  ),
  constructors = list(list(binding = "R6Generator$new", formals = formal_facts(R6Generator$new)),
                      list(binding = "ReferenceGenerator", formals = formal_facts(ReferenceGenerator))),
  methods = list(method_facts("read", "old", old$read, owner),
                 method_facts("read", "fresh", fresh$read, owner),
                 method_facts("read", "reference_old", reference_old$read, owner),
                 method_facts("read", "reference_cached", reference_cached$read, owner),
                 method_facts("clone", "fresh", fresh$clone, owner),
                 method_facts("read", "reference_fresh", reference_fresh$read, owner)),
  dispatch = list(dispatch_facts("old$read()", old, owner),
                  dispatch_facts("fresh$read()", fresh, owner),
                  dispatch_facts("fresh$doubled", fresh, owner),
                  dispatch_facts("reference_old$read()", reference_old, owner),
                  dispatch_facts("reference_cached$read()", reference_cached, owner),
                  dispatch_facts("reference_fresh$read()", reference_fresh, owner),
                  dispatch_facts("locked_error", fresh, owner),
                  dispatch_facts("field_lock_error", reference_fresh, owner),
                  dispatch_facts("class_lock_error", reference_fresh, owner))
)
