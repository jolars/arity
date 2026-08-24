a <- 1         # first
long_name <- 2 # second

short <- 1       #' first
longer_name <- 2 #' second

f(
  a,         # first
  long_name, # second
  z          # third
)

function(
  a,        # first
  long_name # second
) {}

function() {
  x <- 1           # first
  longer_name <- 2 # second

  y <- 3 # third
  # standalone
  longest_name <- 4 # fourth

  # arity-format skip: preserve
  unformatted<-5    # skipped
  q <- 6 # fifth
}

# comment
a <- f() # lone trailing

short_value <- 1 # first
middle <- 2
longer_value <- 3 # second

function(
  # parameter note
  x = 1,          # first
  longer_name = 2 # second
) {}

function(
  x = 1, # first
  middle = 2,
  longer_name = 3 # second
) {}

short_call <- f(
  this_argument_name_is_far_too_long_to_keep_the_entire_call_on_one_physical_output_line
)                # first
long_name <- g() # second
