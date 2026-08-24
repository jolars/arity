# Cell widths drive the column widths, not the input layout.
tribble(
  ~quarter , ~region     , ~product , ~price  , ~units_sold ,
  "Q1"     , "NorthWest" , "Laptop" , 1499.99 ,         250 ,
  "Q2"     , "South"     , "Laptop" ,  489.5  ,         196 ,
  "Q1"     , "South"     , "Tablet" ,  249.99 ,         304
)

# A tribble written on one line still lays out as a table.
tribble(
  ~x , ~y ,
   1 ,  2 ,
  33 ,  4
)

# The namespace-qualified call is recognized too.
tibble::tribble(
  ~x , ~y ,
   1 ,  2 ,
  33 ,  4
)

# A trailing comma is the author's, and is kept.
tribble(
  ~x , ~y ,
   1 ,  2 ,
  33 ,  4 ,
)

# Headers with no rows yet.
tribble(
  ~alpha , ~beta
)

# The assignment's right-hand side is a table like any other.
counts <- tribble(
  ~name  , ~n ,
  "a"    ,  1 ,
  "bbbb" , 22
)
