# The cells do not divide into complete rows.
tribble(~x, ~y, 1, 2, 3)

# No leading header formulas at all.
tribble(1, 2, 3, 4)
list(~x, ~y, 1, 2)

# A named argument is not a cell.
tribble(~x, ~y, 1, 2, .rows = 1)

# A hole leaves a cell with no content to align.
tribble(~x, ~y, , 2, 3, 4)

# Forwarded and unquoted arguments stand in for an unknown number of cells.
tribble(~x, ~y, 1, 2, !!!rows)
tribble(~x, ~y, 1, 2, !!row)
tribble(~x, ~y, 1, 2, ...)

# A cell that cannot render on one line cannot sit in a row.
tribble(~x, ~y, 1, 2, { a }, 4)

# Only a bare or `::`-qualified name identifies the function statically.
foo$tribble(~x, ~y, 1, 2)
tribbles(~x, ~y, 1, 2)
