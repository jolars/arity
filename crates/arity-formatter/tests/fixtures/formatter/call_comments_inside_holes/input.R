# Comments "inside" holes
fn(# comment
  ,
)

fn(, # comment
)
fn(,
  # comment
)
fn(
  , # comment
)
fn(
  ,
  # comment
)

fn(, # comment
  ,
)
fn(,
  # comment
  ,
)
fn(
  , # comment
  ,
)
fn(
  ,
  # comment
  ,
)

fn(
  ,
  , # comment1
  # comment2
  ,
  x
)

# Trails `a`
fn(
  a, # comment
  ,
  b
)
# Trails `a` technically, but should stay on own line
fn(
  a,
  # comment
  ,
  b
)
# Trails `a`
fn(
  a, # comment
  # comment2
  ,
  b
)

# Special test - ensure this leads `b` rather than trails `a`
fn(
  ,
  a,
  , # comment
  b
)

# Both comments lead the hole
fn(# comment1
  # comment2
  ,
  x
)

# Comment leads hole
# Following token is `,`, preceding before hole is another hole
fn(
  a,
  , # comment
  ,
  b
)
fn(
  , # comment
  ,
  x
)

# Comment leads `{` but doesn't move inside it
fn(
  ,
  , # comment
  { 1 +  1 }
)

# A particular motivating case. Want trailing `,` commentB to stay on `b`.
list2(
  a, # commentA
  b, # commentB
)
