saved <- lapply(fields, function(name) {
  list(
    exists = exists(name, envir = state, inherits = FALSE),
    value = if (exists(name, envir = state, inherits = FALSE)) {
      get(name, state)
    } else {
      NULL
    }
  )
})
