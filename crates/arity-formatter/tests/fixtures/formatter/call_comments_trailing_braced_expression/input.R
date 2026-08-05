with(
  xs, # end-of-line
  expr = {
    x + 1
  }
)

with(
  xs,
  # own-line
  expr = {
    x + 1
  }
)

with(
  xs,
  expr # end-of-line
  = {
    x + 1
  }
)

with(
  xs,
  expr
  # own-line
  = {
    x + 1
  }
)

with(
  xs,
  expr = # end-of-line
  {
    x + 1
  }
)

with(
  xs,
  expr =
  # own-line
  {
    x + 1
  }
)

with(
  xs,
  expr =
  {
    x + 1
  } # end-of-line
)

with(
  xs,
  expr =
  {
    x + 1
  }
  # own-line
)
