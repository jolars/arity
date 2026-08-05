fn()
fn(a)

# ------------------------------------------------------------------------
# Holes

# Leading holes should hug the `(` token
fn(,)
fn(,,)

# Non-leading holes retain spaces because they are considered "weird"
# and we want them to stand out
fn(, a, )
fn(, a, , )
fn(a, , b, , )

fn(
  a_really_long_argument_here,
  ,
  another_really_really_long_argument_to_test_this_feature,
  ,
)
