our_apply <- function(x, fun, ...) { # <1>
  val <- numeric(length(x))
  for (i in seq_along(x)) {
    val[i] <- fun(x[[i]], ...) # <2>
  }
  val
}

penguins |> # <1>
  mutate( # <2>
    bill_ratio = bill_depth_mm / bill_length_mm, # <2>
    bill_area = bill_depth_mm * bill_length_mm   # <2>
  ) # <2>

f(
  # ordinary comment
  x
)

function() {
  # <not-an-annotation>
  x
}

f( # <12>
)

function() { # <13>
}
