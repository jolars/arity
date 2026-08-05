# Motivating hugging cases
abort(glue::glue(
  "Length implied by `dim`, {n_elements}, must match the length of `x`, {n_x}."
))
abort(paste0(
  "This is a section",
  and,
  "this is another section",
  "and this is a final section"
))

# Single line
c(list(1))

# Persistent newline
c(list(1))

# Symbol: Line length expansion
c(list(
  foobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbazzzzzzzzzfoobarbaz
))

# Call: Recursive hugging case, no breaks
c(list(
  foobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbazzzzzzzzzfoobarbaz()
))

# Call: Recursive hugging case, inner arguments break
c(list(foobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbazzzzzzzzzfoobarbaz(
  1,
  2
)))

# Call: Recursive hugging case, persistent newlines
c(list(foobar(1, 2)))

# Named arguments prevent hugging
fn(
  name = foobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbafoobarbazzzzzzzzzfoobarbaz(
    1,
    2
  )
)
