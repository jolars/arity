# A comment has no cell of its own, so the call keeps the ordinary
# comment-relocating layout rather than being forced into a table.
tribble(
  ~x,
  ~y,
  # a note about the first row
  1,
  2,
  33,
  4
)

tribble(
  ~x,
  ~y,
  1,
  2, # about this row
  33,
  4
)

# A comment buried inside a cell is relocated by that cell's own construct, and
# the row it would have sat in cannot render on one line.
tribble(
  ~x,
  ~y,
  c(
    1, # first
    2
  ),
  2,
  33,
  4
)
