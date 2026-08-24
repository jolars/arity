# Nested calls render flat inside their cell.
standardized <- tribble(
~from, ~to,
c("UNC", "Chapel Hill"), "UNC",
c("Duke", "Duke University"), "Duke",
c("NC State"), "NC State",
NA, NA
)

# Column widths count characters, not bytes.
tribble(
~col1, ~col2,
"A (Mio. €)", 10,
"B (T €)", 20,
"C §", 10,
"ascii", 10,
)

# A cell too wide for the line still holds its row: breaking it would break the
# table.
tribble(
~a, ~b,
foooooooooo(baaaaaaaaar, foooooooooo, baaaaaaaaar, quuuuuuuuux, zooooooooop), 1,
c(1, 2), 100
)

# A one-statement function body flattens, as it does anywhere else.
tribble(
~name, ~fn,
"identity", function(z) { z },
"first", function(z) z[[1]]
)

# A cell is laid out flat whatever its width, so a value-position `if` that
# would brace elsewhere stays on its row here.
tribble(
~x, ~y,
1, if (a) 1 else 2,
3, if (condition) alternative_one_value else alternative_two_value_that_is_long
)
