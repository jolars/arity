# Decimal points align within a column; an integer literal's `L` sits where the
# point would.
tribble(
  ~a   , ~b     ,
    1L ,   0.   ,
   12  ,   1.5  ,
  100  ,  12.34 ,
    3  , 100.0  ,
)

# A single `+`/`-` counts as one more integer digit.
tribble(
  ~a       , ~b   , ~c     ,
    -1.200 , +2L  , -100.  ,
  1000     ,  2.5 ,   50L  ,
   123.456 ,  9   ,    0.1
)

# Repeated unary operators are not numeric, so they align as text.
tribble(
  ~a      , ~b ,
  --1.200 ,  0 ,
  -100.   ,  1
)

# Numbers right-align, everything else left-aligns.
tribble(
  ~kind , ~count ,
  "a"   ,      1 ,
  foo() ,     22 ,
  "ccc" ,    333
)

# A non-numeric cell wider than the numeric sub-column widens the whole column.
tribble(
  ~value     ,
   1.5       ,
  "a string" ,
  22.25
)
