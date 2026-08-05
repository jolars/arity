switch(
  name,
  one = , # Trailing, stays beside `one`
  two = , # Trailing, stays beside `two`
  three = 1,
  stop("oh no")
)

fn(
  x,
  one # Moves above `one`
  = ,
  two = 2
)

fn(
  x,
  one = # Trailing, stays beside `one`
  ,
  two = 2
)

fn(
  x,
  one = # Trailing, stays beside `one`
)
