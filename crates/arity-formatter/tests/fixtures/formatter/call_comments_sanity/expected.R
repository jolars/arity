c(
  #foo
  list(1)
)

c(
  list(1)
  #foo
)

c(list(
  #foo
  1
))

c(list(
  #foo
  x = 1
))

c(list(
  #foo
  x = 1
))

c(list(
  #foo
))

# Trailing comment of inner paren
c(
  list(1) #foo
)

# Leading comment of outer paren
c(
  list(1)
  #foo
)

c(
  list(1) #foo
)
