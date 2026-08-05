fn[]
fn[a]

fn[a = {
  1 + 1
}]
fn["description", {
  1 + 1
}]

DT[,
  {
    # write each group to a different file
    fwrite(.SD, "name")
  },
  by = x
]

DT[, by = x, {
  # write each group to a different file
  fwrite(.SD, "name")
}]

fn[,]
fn[,,]

df[a, ]

fn[a, , b, , ]
fn[
  a_really_long_argument_here,
  ,
  another_really_really_long_argument_to_test_this_feature,
  ,
]

fn[, x = 1]
fn[, x = 1]
fn[, x = 1]
fn[,, x = 1]
